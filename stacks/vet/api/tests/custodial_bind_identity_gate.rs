//! D1 - the bind-time ATTESTATION-INTEGRITY GATE (`POST /profiles/issue/custodial-bind`), the
//! FULL-LEAF-LIST commitment check.
//!
//! The vet collects the owner's identity at session-start, salts one `owner.identity.*` attribute
//! leaf per field, and hands `{keyPath, salt, value}` to the device via `GET /p/<token>`. The
//! device folds them into `R` and, at bind, posts the opening of EVERY attribute leaf of its tree
//! (`leaves`, pet and identity alike) plus the three opaque reserved owner-control leaf hashes
//! (`reservedLeafHashes`). The vet recomputes every opened leaf, requires the identity openings to
//! EXACTLY equal its own stored set, and rebuilds the posted `R` from the full list - BEFORE
//! anything reaches the chain.
//!
//! The two refusal tests are the load-bearing ones: without the gate, a device could fold a
//! DIFFERENT identity value (replacement) or an EXTRA identity leaf the vet never attested
//! (injection) and later disclose it against `rootIssuer[R] = trusted vet` - a forged vet
//! attestation. The full-list commitment closes BOTH: a forged `owner.identity.*` leaf must either
//! be opened (refused by exact-set equality) or hide among the 3 opaque hashes (displacing a
//! reserved leaf, and a tree missing a reserved leaf can never produce a consent proof).

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

/// The device's bind payload pieces: `R`, the opening of every attribute leaf, and the three
/// opaque reserved owner-control leaf hashes - exactly what both mobile apps compute off the
/// SAME `build_profile_tree` the FFI wraps.
struct DeviceWire {
    root: String,
    leaves: serde_json::Value,
    reserved: serde_json::Value,
}

fn opening_of(a: &AttributeLeaf) -> serde_json::Value {
    let value = match &a.value {
        TypedScalar::Str(v) => v.clone(),
        other => panic!("v1 attrs are strings, got {other:?}"),
    };
    serde_json::json!({
        "keyPath": a.key_path,
        "saltHex": format!("0x{}", hex::encode(a.salt)),
        "tag": 2,
        "value": value,
    })
}

fn device_wire(dog_tag_id_field: Fr, attrs: &[AttributeLeaf]) -> DeviceWire {
    let tree = build_profile_tree(DEVICE_SEED, dog_tag_id_field, &owner_address(), attrs)
        .expect("device build_profile_tree");
    DeviceWire {
        root: to_hex32(&tree.root),
        leaves: serde_json::Value::Array(attrs.iter().map(opening_of).collect()),
        reserved: serde_json::json!([
            to_hex32(&tree.owner_address_leaf),
            to_hex32(&tree.consent_key_leaf),
            to_hex32(&tree.owner_secret_leaf),
        ]),
    }
}

fn bind_body(token: &str, wire: &DeviceWire) -> serde_json::Value {
    serde_json::json!({
        "token": token,
        "root": wire.root,
        "leaves": wire.leaves,
        "reservedLeafHashes": wire.reserved,
    })
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
        // "minting" is the accepted-but-anchoring intermediate state; poll through it to terminal.
        if b["status"] != "pending" && b["status"] != "minting" {
            return b;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("session {session_id} never left pending");
}

async fn assert_nothing_on_chain(chain: &MemChain, dog_tag_id: &str, root: &str) {
    let onchain_id = vet_api::routes::onchain_dog_tag_id(dog_tag_id).unwrap();
    assert!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .is_err(),
        "the refused bind must not have minted"
    );
    assert!(
        chain
            .issued_at(PROFILE_ISSUER_ADDR, root)
            .await
            .unwrap()
            .is_zero(),
        "the refused bind must not have anchored"
    );
}

/// HAPPY PATH: the device folds the vet-salted identity leaves into `R`, opens every attribute
/// leaf, names the reserved triple's hashes, the gate verifies the identity openings against its
/// OWN stored set and rebuilds `R` from the full list, and the tag mints + anchors.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identity_committed_r_with_full_openings_binds_and_mints() {
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

    let mut attrs = pet_attrs();
    attrs.extend(identity);
    let wire = device_wire(canonical_field(&dog_tag_id), &attrs);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(bind_body(&token, &wire)),
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
        wire.root.to_lowercase()
    );
    assert!(chain.is_valid(PROFILE_ISSUER_ADDR, &wire.root).await.unwrap());
}

/// THE REPLACEMENT-FORGERY GUARD. The device folds a DIFFERENT identity value (country US, vet
/// stored GB) using the vet's own salt, and opens its forged leaves. The exact-set check refuses
/// the opening (it matches no vet-attested entry) and the bind is REFUSED with NOTHING written
/// on-chain - so a forged "vet-attested" identity can never anchor.
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

    let mut attrs = pet_attrs();
    attrs.extend(forged);
    let wire = device_wire(canonical_field(&dog_tag_id), &attrs);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(bind_body(&token, &wire)),
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
    assert_nothing_on_chain(&mem, &dog_tag_id, &wire.root).await;
}

/// THE INJECTION-FORGERY GUARD, opened variant. The device commits an EXTRA `owner.identity.*`
/// leaf the vet never attested (its own salt, its own value) beside the honest set, and opens
/// everything. The exact-set equality refuses the extra opening - injection is closed, not just
/// replacement.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_extra_opened_identity_leaf_the_vet_never_attested_is_refused() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (token, dog_tag_id, session_id) = start_session(&app, &op, true).await;

    let mut attrs = pet_attrs();
    attrs.extend(resolve_identity_leaves(&app, &token).await);
    attrs.push(AttributeLeaf {
        key_path: "owner.identity.email".to_string(),
        salt: [42u8; SALT_LEN],
        value: TypedScalar::Str("alice@example.com".to_string()),
    });

    let wire = device_wire(canonical_field(&dog_tag_id), &attrs);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(bind_body(&token, &wire)),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "injected identity must refuse: {b}");
    let msg = b["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("owner.identity.email"),
        "the refusal must name the injected leaf: {msg}"
    );

    let row = await_settled(&app, &op, &session_id).await;
    assert_eq!(row["status"], "error");
    assert_nothing_on_chain(&mem, &dog_tag_id, &wire.root).await;
}

/// THE INJECTION-FORGERY GUARD, unopened variant. The device commits the extra forged identity
/// leaf into `R` but does NOT open it. Its only two hiding places are both refused: withholding
/// the opening breaks the root rebuild, and smuggling the leaf hash through `reservedLeafHashes`
/// either exceeds the count of 3 or displaces a real reserved hash (breaking the rebuild again -
/// and a tree missing a reserved leaf could never produce a consent proof anyway).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unopened_forged_identity_leaf_breaks_the_commitment_and_is_refused() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let forged_leaf = AttributeLeaf {
        key_path: "owner.identity.email".to_string(),
        salt: [42u8; SALT_LEN],
        value: TypedScalar::Str("alice@example.com".to_string()),
    };
    let forged_hash = to_hex32(
        &dogtag_standard::leaf::hash_leaf(
            &forged_leaf.key_path,
            &forged_leaf.salt,
            &forged_leaf.value,
        )
        .unwrap(),
    );

    // (a) withhold the forged leaf's opening: the posted list cannot rebuild the forged R.
    let (token, dog_tag_id, session_id) = start_session(&app, &op, true).await;
    let honest = {
        let mut attrs = pet_attrs();
        attrs.extend(resolve_identity_leaves(&app, &token).await);
        attrs
    };
    let mut with_forged = honest.clone();
    with_forged.push(forged_leaf.clone());
    let forged_tree = device_wire(canonical_field(&dog_tag_id), &with_forged);
    let honest_openings = device_wire(canonical_field(&dog_tag_id), &honest);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({
            "token": token,
            "root": forged_tree.root,
            "leaves": honest_openings.leaves,
            "reservedLeafHashes": forged_tree.reserved,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "withheld forged leaf must refuse: {b}");
    let msg = b["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("rebuild") && msg.contains("FRESH session"),
        "the refusal must name the broken commitment: {msg}"
    );
    let row = await_settled(&app, &op, &session_id).await;
    assert_eq!(row["status"], "error");
    assert_nothing_on_chain(&mem, &dog_tag_id, &forged_tree.root).await;

    // (b) smuggle the forged leaf hash as a FOURTH reserved hash: refused by the count of 3.
    let (token, dog_tag_id, _sid) = start_session(&app, &op, true).await;
    let mut with_forged = pet_attrs();
    with_forged.extend(resolve_identity_leaves(&app, &token).await);
    let honest_identity = with_forged.clone();
    with_forged.push(forged_leaf.clone());
    let forged_tree = device_wire(canonical_field(&dog_tag_id), &with_forged);
    let honest_openings = device_wire(canonical_field(&dog_tag_id), &honest_identity);
    let mut four_hashes = forged_tree.reserved.as_array().unwrap().clone();
    four_hashes.push(serde_json::json!(forged_hash));
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({
            "token": token,
            "root": forged_tree.root,
            "leaves": honest_openings.leaves,
            "reservedLeafHashes": four_hashes,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "4 reserved hashes must refuse: {b}");
    assert!(
        b["error"].as_str().unwrap_or_default().contains("exactly 3"),
        "the refusal must pin the reserved-triple count: {b}"
    );
    assert_nothing_on_chain(&mem, &dog_tag_id, &forged_tree.root).await;

    // (c) displace a real reserved hash with the forged leaf hash: the rebuild is missing a
    // reserved leaf, so it cannot reproduce R.
    let (token, dog_tag_id, _sid) = start_session(&app, &op, true).await;
    let mut with_forged = pet_attrs();
    with_forged.extend(resolve_identity_leaves(&app, &token).await);
    let honest_identity = with_forged.clone();
    with_forged.push(forged_leaf.clone());
    let forged_tree = device_wire(canonical_field(&dog_tag_id), &with_forged);
    let honest_openings = device_wire(canonical_field(&dog_tag_id), &honest_identity);
    let mut displaced = forged_tree.reserved.as_array().unwrap().clone();
    displaced[2] = serde_json::json!(forged_hash);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({
            "token": token,
            "root": forged_tree.root,
            "leaves": honest_openings.leaves,
            "reservedLeafHashes": displaced,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "displaced reserved hash must refuse: {b}");
    assert!(
        b["error"].as_str().unwrap_or_default().contains("rebuild"),
        "the refusal must name the broken commitment: {b}"
    );
    assert_nothing_on_chain(&mem, &dog_tag_id, &forged_tree.root).await;
}

/// A session WITH identity leaves refuses a bind whose openings omit them: skipping the identity
/// fold entirely is the lazier forgery, and it must fail exactly like the active one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_identity_openings_refuse_when_the_session_carries_identity() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (token, dog_tag_id, _session_id) = start_session(&app, &op, true).await;

    // Device folds ONLY pet attrs (ignoring the identity leaves) and opens only those.
    let wire = device_wire(canonical_field(&dog_tag_id), &pet_attrs());
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(bind_body(&token, &wire)),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "identity-less openings must refuse: {b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap_or_default()
            .contains("missing identity opening"),
        "the refusal must name the missing openings: {b}"
    );

    assert_nothing_on_chain(&mem, &dog_tag_id, &wire.root).await;
}

/// DEGRADE PATH (the existing #74 contract): a session whose operator collected no identity has no
/// identity leaves; the resolve carries an empty `identityLeaves` and the bind mints with an EMPTY
/// identity subset - while a bind that opens an UNEXPECTED `owner.identity.*` leaf against it is
/// refused rather than ignored.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_without_identity_degrades_to_the_pet_only_contract() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    // (a) no identity -> empty leaves -> a pet-only full-list bind mints.
    let (token, dog_tag_id, session_id) = start_session(&app, &op, false).await;
    let identity = resolve_identity_leaves(&app, &token).await;
    assert!(identity.is_empty(), "blank identity fields must yield no leaves");

    let wire = device_wire(canonical_field(&dog_tag_id), &pet_attrs());
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(bind_body(&token, &wire)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "identity-less session must still bind: {b}");
    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(settled["status"], "bound", "{settled}");

    // (b) an opened owner.identity.* leaf against an identity-less session is refused, not
    // silently dropped.
    let (token2, dog_tag_id2, _sid2) = start_session(&app, &op, false).await;
    let mut attrs2 = pet_attrs();
    attrs2.push(AttributeLeaf {
        key_path: "owner.identity.country".to_string(),
        salt: [9u8; SALT_LEN],
        value: TypedScalar::Str("GB".to_string()),
    });
    let wire2 = device_wire(canonical_field(&dog_tag_id2), &attrs2);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(bind_body(&token2, &wire2)),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "identity openings against an identity-less session must refuse: {b}"
    );
    assert!(
        b["error"]
            .as_str()
            .unwrap_or_default()
            .contains("owner.identity.country"),
        "the refusal must name the unexpected leaf: {b}"
    );
}
