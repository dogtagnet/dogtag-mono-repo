//! D1 - the bind-time ATTESTATION-INTEGRITY GATE (`POST /profiles/issue/custodial-bind`).
//!
//! The vet collects the owner's identity at session-start, salts one `owner.identity.*` attribute
//! leaf per field, and hands `{keyPath, salt, value}` to the device via `GET /p/<token>`. The
//! device folds them into `R` and posts `identityProofs` (Merkle paths only) at bind; the vet
//! recomputes each leaf from its OWN stored triple and refuses the bind unless every one folds to
//! the posted `R` - BEFORE anything reaches the chain.
//!
//! The forged-attestation test is the load-bearing one: without the gate, a device could fold a
//! DIFFERENT identity value and later disclose it against `rootIssuer[R] = trusted vet` - a forged
//! vet attestation. The gate is exactly what makes "identity in `R`" mean "identity VET-ATTESTED
//! in `R`".

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;

use ark_bn254::Fr;
use dogtag_standard::field::to_hex32;
use dogtag_standard::leaf::field_of_value;
use dogtag_standard::profile_tree::{build_profile_tree, AttributeLeaf, SALT_LEN};
use dogtag_standard::types::TypedScalar;
use vet_api::chain::{ChainClient, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";

/// Test-material wallet seed. Committed in the clear; never holds value.
const DEVICE_SEED: &[u8] = b"DogTag identity-gate test seed - TEST MATERIAL ONLY";

fn start_body(with_identity: bool) -> serde_json::Value {
    let identity = if with_identity {
        serde_json::json!({
            "countryOfIdentification": "GB",
            "identification": "PASSPORT-123",
            "name": "Alice Owner"
        })
    } else {
        // Operator collected no identity: the session must degrade to the pet-only contract.
        serde_json::json!({ "countryOfIdentification": "", "identification": "", "name": "" })
    };
    serde_json::json!({
        "ownerIdentity": identity,
        "pet": { "name": "Rex", "breedLabel": "Labrador Retriever" }
    })
}

fn owner_address() -> [u8; 20] {
    let mut a = [0u8; 20];
    a[16..].copy_from_slice(&0xdead_beefu32.to_be_bytes());
    a
}

fn pet_attrs() -> Vec<AttributeLeaf> {
    vec![AttributeLeaf {
        key_path: "credentialSubject.name".to_string(),
        salt: [7u8; SALT_LEN],
        value: TypedScalar::Str("Rex".to_string()),
    }]
}

fn canonical_field(handle: &str) -> Fr {
    field_of_value(&TypedScalar::Integer(handle.to_string())).expect("canonical field")
}

fn mem_state(chain: Arc<dyn ChainClient>) -> vet_api::app::AppState {
    state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    )
}

async fn start_session(
    app: &axum::Router,
    op: &str,
    with_identity: bool,
) -> (String, String, String) {
    let (s, b) = call(
        app,
        "POST",
        "/profiles/issue/session/start",
        Some(op),
        Some(start_body(with_identity)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    (
        b["token"].as_str().unwrap().to_string(),
        b["dogTagId"].as_str().unwrap().to_string(),
        b["sessionId"].as_str().unwrap().to_string(),
    )
}

/// Resolve `/p/<token>` (unauthenticated, non-consuming) and parse the D1 `identityLeaves` into
/// device-side attribute leaves - EXACTLY what both mobile apps do before folding `R`.
async fn resolve_identity_leaves(app: &axum::Router, token: &str) -> Vec<AttributeLeaf> {
    let (s, b) = call(app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "resolve: {b}");
    b["identityLeaves"]
        .as_array()
        .expect("identityLeaves array")
        .iter()
        .map(|leaf| {
            let salt_hex = leaf["saltHex"].as_str().expect("saltHex");
            let bytes = hex::decode(salt_hex.trim_start_matches("0x")).expect("salt hex");
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&bytes);
            assert_eq!(leaf["tag"].as_u64(), Some(2), "v1 identity attrs are strings");
            AttributeLeaf {
                key_path: leaf["keyPath"].as_str().expect("keyPath").to_string(),
                salt,
                value: TypedScalar::Str(leaf["value"].as_str().expect("value").to_string()),
            }
        })
        .collect()
}

/// Fold the device tree over pet + identity attrs and emit `(R, identityProofs)` - the exact bind
/// payload pieces the phone computes. Proofs come from the SAME `build_profile_disclosure` the
/// mobile FFI calls, so a change to the envelope moves this test.
fn device_root_and_proofs(
    dog_tag_id_field: Fr,
    identity: &[AttributeLeaf],
) -> (String, serde_json::Value) {
    let mut attrs = pet_attrs();
    attrs.extend(identity.iter().cloned());
    let tree = build_profile_tree(DEVICE_SEED, dog_tag_id_field, &owner_address(), &attrs)
        .expect("device build_profile_tree");
    let reveal: Vec<String> = identity.iter().map(|a| a.key_path.clone()).collect();
    let proofs = if reveal.is_empty() {
        serde_json::json!([])
    } else {
        let disclosure = dogtag_standard::disclosure::build_profile_disclosure(
            DEVICE_SEED,
            dog_tag_id_field,
            &owner_address(),
            &attrs,
            &reveal,
        )
        .expect("build_profile_disclosure");
        serde_json::Value::Array(
            disclosure
                .disclosures
                .iter()
                .map(|entry| {
                    serde_json::json!({ "keyPath": entry.key_path, "proof": entry.proof })
                })
                .collect(),
        )
    };
    (to_hex32(&tree.root), proofs)
}

async fn await_settled(app: &axum::Router, op: &str, session_id: &str) -> serde_json::Value {
    for _ in 0..200 {
        let (s, b) = call(
            app,
            "GET",
            &format!("/profiles/issue/session/{session_id}"),
            Some(op),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        if b["status"] != "pending" {
            return b;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("session {session_id} never left pending");
}

/// HAPPY PATH: the device folds the vet-salted identity leaves into `R`, posts the paths, the gate
/// verifies every leaf against its OWN stored openings, and the tag mints + anchors.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identity_committed_r_with_valid_proofs_binds_and_mints() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (token, dog_tag_id, session_id) = start_session(&app, &op, true).await;

    let identity = resolve_identity_leaves(&app, &token).await;
    assert_eq!(
        identity.iter().map(|a| a.key_path.as_str()).collect::<Vec<_>>(),
        vec![
            "owner.identity.fullName",
            "owner.identity.country",
            "owner.identity.docNumber"
        ],
        "one leaf per non-blank identity field, in the sanctioned namespace"
    );

    let (root, proofs) = device_root_and_proofs(canonical_field(&dog_tag_id), &identity);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({ "token": token, "root": root, "identityProofs": proofs })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "custodial bind: {b}");

    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(settled["status"], "bound", "must mint: {settled}");

    // Both on-chain conditions hold for the identity-committed R.
    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    assert_eq!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .unwrap()
            .to_lowercase(),
        root.to_lowercase()
    );
    assert!(chain.is_valid(PROFILE_ISSUER_ADDR, &root).await.unwrap());
}

/// THE FORGED-ATTESTATION GUARD - the single most important new test. The device folds a DIFFERENT
/// identity value (country US, vet stored GB) using the vet's own salt, and posts proofs for its
/// forged leaves. The gate recomputes each leaf from the VET's stored `{keyPath, salt, value}`,
/// the forged path cannot fold the honest leaf to the forged `R`, and the bind is REFUSED with
/// NOTHING written on-chain - so a forged "vet-attested" identity can never anchor.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_root_committing_a_forged_identity_value_is_refused_before_any_chain_write() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (token, dog_tag_id, session_id) = start_session(&app, &op, true).await;

    // The device tampers ONE value: country GB -> US. Same keyPaths, same vet salts.
    let mut forged = resolve_identity_leaves(&app, &token).await;
    let country = forged
        .iter_mut()
        .find(|a| a.key_path == "owner.identity.country")
        .expect("country leaf");
    country.value = TypedScalar::Str("US".to_string());

    let (forged_root, forged_proofs) =
        device_root_and_proofs(canonical_field(&dog_tag_id), &forged);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({
            "token": token, "root": forged_root, "identityProofs": forged_proofs
        })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "forged identity must refuse: {b}");
    let msg = b["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("owner.identity.country") && msg.contains("FRESH session"),
        "the refusal must name the failing leaf and the remedy: {msg}"
    );

    // The session row is settled with the reason, so the operator sees WHY.
    let row = await_settled(&app, &op, &session_id).await;
    assert_eq!(row["status"], "error");

    // NOTHING reached the chain: not the seal, not the anchor. `issue(R)` is globally write-once,
    // so firing it before the gate would have burned the R forever.
    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    assert!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .is_err(),
        "the forged bind must not have minted"
    );
    assert!(
        chain
            .issued_at(PROFILE_ISSUER_ADDR, &forged_root)
            .await
            .unwrap()
            .is_zero(),
        "the forged bind must not have anchored"
    );
}

/// A session WITH identity leaves refuses a bind that posts NO proofs: skipping the fold entirely
/// is the lazier forgery, and it must fail exactly like the active one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_identity_proofs_refuse_when_the_session_carries_identity() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (token, dog_tag_id, _session_id) = start_session(&app, &op, true).await;

    // Device folds ONLY pet attrs (ignoring the identity leaves) and posts no proofs.
    let (root, _) = device_root_and_proofs(canonical_field(&dog_tag_id), &[]);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({ "token": token, "root": root })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "proof-less bind must refuse: {b}");

    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    assert!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .is_err(),
        "nothing may have minted"
    );
}

/// DEGRADE PATH (the existing #74 contract): a session whose operator collected no identity has no
/// identity leaves; the resolve carries an empty `identityLeaves` and the bind mints without
/// proofs - while a bind that posts UNEXPECTED proofs against it is refused rather than ignored.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_without_identity_degrades_to_the_pet_only_contract() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    // (a) no identity -> empty leaves -> proof-less bind mints.
    let (token, dog_tag_id, session_id) = start_session(&app, &op, false).await;
    let identity = resolve_identity_leaves(&app, &token).await;
    assert!(identity.is_empty(), "blank identity fields must yield no leaves");

    let (root, _) = device_root_and_proofs(canonical_field(&dog_tag_id), &[]);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({ "token": token, "root": root })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "identity-less session must still bind: {b}");
    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(settled["status"], "bound", "{settled}");

    // (b) unexpected proofs against an identity-less session are refused, not silently dropped.
    let (token2, dog_tag_id2, _sid2) = start_session(&app, &op, false).await;
    let fake_identity = vec![AttributeLeaf {
        key_path: "owner.identity.country".to_string(),
        salt: [9u8; SALT_LEN],
        value: TypedScalar::Str("GB".to_string()),
    }];
    let (root2, proofs2) = device_root_and_proofs(canonical_field(&dog_tag_id2), &fake_identity);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({
            "token": token2, "root": root2, "identityProofs": proofs2
        })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "proofs against an identity-less session must refuse: {b}"
    );
}
