//! The Level-B CUSTODIAL ISSUANCE BRIDGE (M-2 / unifyplan §P-1, closing datamodel D-1).
//!
//! Before this milestone no backend called `mintCustodial`, so no owner-hidden tag could exist
//! on-chain: the vet/admin stacks only ever minted through the Level-A, owner-revealing
//! `mint(to, id, root)`. These tests drive the new `POST /profiles/issue/custodial-bind` end to end
//! against `MemChain` and pin the ways it must fail closed.
//!
//! What is asserted, and why each matters:
//!
//! 1. **A REAL device-computed `R` mints.** The root is not a literal - it is folded here by the same
//!    `dogtag_standard::profile_tree::build_profile_tree` the iOS `ProfileTreeStore` calls, so a
//!    change to the device builder moves this test rather than silently desyncing the server.
//! 2. **BOTH on-chain conditions land in one flow** (datamodel §3.5): `R` is sealed as
//!    `profileRoot[dogTagId]` AND anchored so `rootIssuer[R]` resolves and `isValid(R)` holds. These
//!    are the exact two reads a Level-B verify performs (`VerificationRegistryConsent.sol:163` and
//!    `:188-192`), so satisfying both is what makes the minted tag verifiable. Skipping the anchor is
//!    the tested-in-Solidity failure mode at `contracts/test/CustodialIssuance.t.sol:367`
//!    ("unknown root" on EVERY verify); here we prove the SERVER path never produces it.
//! 3. **Canonical-field discipline** (§P-1.3): the tag is sealed under
//!    `field_of_value(Integer(handle))`, never the raw operator handle. The raw-handle test proves
//!    this is load-bearing rather than cosmetic - the two bind DIFFERENT trees, so a raw-bound `R`
//!    can never equal `profileRoot(canonical id)` and every verify fails closed.
//! 4. **Owner-blindness.** No wallet is sent, none is stored, and the mint calldata cannot express
//!    one.
//!
//! The Level-A `/profiles/issue/bind` route is untouched by this milestone and keeps its own coverage
//! in `profile_issue.rs`; `level_a_issuance_path_still_works` here guards the two paths coexisting.

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;

use ark_bn254::Fr;
use dogtag_standard::field::{field_from_uint, to_hex32};
use dogtag_standard::leaf::field_of_value;
use dogtag_standard::profile_tree::{build_profile_tree, AttributeLeaf, SALT_LEN};
use dogtag_standard::types::TypedScalar;
use vet_api::chain::{ChainClient, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";

/// Test-material wallet seed. Committed in the clear; never holds value.
const DEVICE_SEED: &[u8] = b"DogTag custodial-bridge test seed - TEST MATERIAL ONLY";

fn start_body() -> serde_json::Value {
    serde_json::json!({
        "ownerIdentity": {
            "countryOfIdentification": "GB",
            "identification": "PASSPORT-123",
            "name": "Alice Owner"
        },
        "pet": {
            "name": "Rex",
            "species": "Canis lupus familiaris",
            "breedVbo": "VBO:0200798",
            "breedLabel": "Labrador Retriever",
            "sex": "male",
            "neuterStatus": "neutered",
            "dateOfBirth": "2021-05-01",
            "weightHistory": [{ "unit": "kg", "value": "22.7", "measuredOn": "2026-01-10" }],
            "microchip": { "code": "985141006580319", "standard": "ISO_11784_11785", "implantDate": "2021-06-01" }
        }
    })
}

/// Fold a profile root `R` exactly as the OWNER'S DEVICE does: the real `build_profile_tree` over a
/// wallet seed the server never sees. Returns `R` as `0x`-hex.
///
/// `dog_tag_id_field` is the KDF binding input - the whole point of the canonical-field discipline is
/// that this parameter must be `field_of_value(Integer(handle))` and not the raw handle, so it is
/// exposed here rather than derived, letting `raw_handle_binding_breaks_the_R_binding_fail_closed`
/// pass the WRONG one deliberately.
fn device_root(dog_tag_id_field: Fr) -> String {
    let mut owner_address = [0u8; 20];
    owner_address[16..].copy_from_slice(&0xdead_beefu32.to_be_bytes());
    let attributes = vec![AttributeLeaf {
        key_path: "credentialSubject.name".to_string(),
        salt: [7u8; SALT_LEN],
        value: TypedScalar::Str("Rex".to_string()),
    }];
    let tree = build_profile_tree(DEVICE_SEED, dog_tag_id_field, &owner_address, &attributes)
        .expect("device-side build_profile_tree");
    to_hex32(&tree.root)
}

/// The CANONICAL dogTagId field — what the device binds into `R` and what the SBT is minted under.
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

/// Operator-start a session; returns `(token, dogTagId, sessionId)`.
async fn start_session(app: &axum::Router, op: &str) -> (String, String, String) {
    let (s, b) = call(
        app,
        "POST",
        "/profiles/issue/session/start",
        Some(op),
        Some(start_body()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    (
        b["token"].as_str().unwrap().to_string(),
        b["dogTagId"].as_str().unwrap().to_string(),
        b["sessionId"].as_str().unwrap().to_string(),
    )
}

/// Poll the operator status route until it leaves "pending". Returns the terminal row.
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

/// THE MILESTONE TEST: a device-computed `R` is accepted by the server, minted owner-blind via
/// `mintCustodial(id, R)`, and anchored via `issue(R)` in the SAME flow — leaving on-chain state that
/// satisfies BOTH conditions a Level-B consent verify checks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_computed_root_mints_custodially_and_is_anchored_for_verification() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (token, dog_tag_id, session_id) = start_session(&app, &op).await;

    // The device folds R locally from its wallet seed, binding the CANONICAL dogTagId field. The
    // server never sees the seed, the owner secret, or a wallet address.
    let root = device_root(canonical_field(&dog_tag_id));

    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({ "token": token, "root": root })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "custodial bind: {b}");
    assert_eq!(
        b["status"], "minting",
        "responds before the chain writes land"
    );
    assert_eq!(b["root"].as_str().unwrap(), root);
    assert_eq!(b["dogTagId"].as_str().unwrap(), dog_tag_id);
    // Stamped on the on-chain ContractSet axis of the two-axis registry (R-5) — the axis that names
    // the deployed trio this tag is bound to. NOT the artifact axis, which a zkey rotation moves.
    assert_eq!(
        b["protocolVersion"], "dogtag-levelb/1",
        "a Level-B tag must never claim the Level-A version"
    );
    // Owner-blind on the wire: the response cannot hand back a wallet because none was ever sent.
    assert!(
        b.get("walletAddress").is_none() || b["walletAddress"].is_null(),
        "custodial issuance must not surface an owner wallet: {b}"
    );

    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(
        settled["status"], "bound",
        "custodial issuance must complete: {settled}"
    );
    assert!(settled["txHash"].as_str().is_some(), "mint txHash recorded");
    assert!(
        settled["walletAddress"].is_null(),
        "the stored session must hold NO owner wallet: {settled}"
    );

    // ---- the two independent on-chain conditions (datamodel §3.5) ----
    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();

    // (1) sealed: R == profileRoot[dogTagId], under the CANONICAL id, on the LEVEL-B SBT.
    let sealed = chain
        .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
        .await
        .expect("profileRoot set on the Level-B SBT");
    assert_eq!(
        sealed.to_lowercase(),
        root.to_lowercase(),
        "the device-built R must be what mintCustodial sealed"
    );

    // (2) anchored: issue(R) landed on the issuer clone, so rootIssuer[R] resolves and isValid(R)
    //     holds. Without this every verify reverts "unknown root".
    assert!(
        chain.is_valid(PROFILE_ISSUER_ADDR, &root).await.unwrap(),
        "R must be anchored in the DogTagIssuer clone, not merely sealed"
    );

    // Owner-blind on-chain: `mintCustodial` has no recipient parameter, so unlike the Level-A path
    // there is no per-tag owner to read back at all.
    assert!(
        chain.owner_of(SBT_CONSENT_ADDR, &onchain_id).await.is_err(),
        "custodial issuance must not write a per-tag owner"
    );

    // The one-time token is consumed: a replay cannot mint a second tag.
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({ "token": token, "root": root })),
    )
    .await;
    assert!(
        s == StatusCode::GONE || s == StatusCode::NOT_FOUND,
        "replayed bind token must be rejected, got {s}"
    );
}

/// FAIL-CLOSED: skipping `issue(R)` leaves a tag that reverts `unknown root` on every verify.
///
/// This is the trap the spec's step list invites (it names only `profileRoot`) and the one the
/// contract warns about in-code (`VerificationRegistryConsent.sol:188-189`,
/// `DogTagSBTConsent.sol:139-143`). Solidity pins the on-chain consequence at
/// `CustodialIssuance.t.sol:367`; this pins the SERVER's obligation — we assert the anchor condition
/// is FALSE when only the mint happened, i.e. that condition (2) is genuinely independent of (1) and
/// a route doing only half its job would be caught.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn minting_without_issue_leaves_the_root_unanchored_and_unverifiable() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, _op, _backend) = boot_custody(&app).await;

    let handle = "424242";
    let onchain_id = vet_api::routes::onchain_dog_tag_id(handle).unwrap();
    let root = device_root(canonical_field(handle));

    // Do ONLY the seal — the half a naive bridge would implement.
    chain
        .mint_custodial(0, SBT_CONSENT_ADDR, &onchain_id, &root)
        .await
        .expect("mintCustodial");

    // Condition (1) holds...
    assert_eq!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .unwrap()
            .to_lowercase(),
        root.to_lowercase()
    );
    // ...but condition (2) does not, which is what makes every verify revert "unknown root".
    assert!(
        !chain.is_valid(PROFILE_ISSUER_ADDR, &root).await.unwrap(),
        "a root that was minted but never issued must NOT resolve as anchored"
    );

    // And the damage is permanent: `profileRoot[id]` is write-once and survives a burn, so the
    // dogTagId cannot be re-minted to repair the tag. This is precisely why the route anchors FIRST.
    let err = chain
        .mint_custodial(0, SBT_CONSENT_ADDR, &onchain_id, &root)
        .await
        .expect_err("a re-mint of the same dogTagId must revert");
    assert!(
        format!("{err}").contains("already minted"),
        "re-mint must fail write-once, got: {err}"
    );
}

/// FAIL-CLOSED: binding the RAW handle instead of `field_of_value(Integer(handle))` breaks the
/// `R` ↔ `dogTagId` binding.
///
/// The SDK fixtures take this shortcut and the code warns against copying it into the issuance path
/// (`profile_tree.rs` `DeviceRootFixtureWitness`); this is the server-side proof of why. The mismatch
/// fails CLOSED (a liveness bug, never a forgery) — but because the mint is write-once it also
/// retires the dogTagId, so it must be impossible to reach by construction, not merely detected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_handle_binding_breaks_the_r_binding_fail_closed() {
    let handle = "424242";

    // Two trees over identical seed/owner/attributes, differing ONLY in the dogTagId field bound as
    // the KDF input: the canonical field vs the fixture's raw-handle shortcut.
    let canonical_root = device_root(canonical_field(handle));
    let raw_root = device_root(field_from_uint(424_242));
    assert_ne!(
        canonical_root, raw_root,
        "the canonical field and the raw handle must fold DIFFERENT roots - \
         if these ever collide the discipline is unenforceable"
    );

    // The chain is keyed by the canonical field, so a raw-bound R can never satisfy condition (1).
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;
    let (token, dog_tag_id, session_id) = start_session(&app, &op).await;

    let bad_root = device_root(field_from_uint(dog_tag_id.parse::<u64>().unwrap()));
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({ "token": token, "root": bad_root })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "the server cannot detect this at the edge"
    );
    await_settled(&app, &op, &session_id).await;

    // The server seals whatever R it was handed - it CANNOT recompute R (no seed). So the on-chain
    // state is self-consistent, and the failure surfaces at VERIFY: the owner's proof re-derives R
    // from the canonical field, which is not what was sealed. Fail-closed, exactly as designed.
    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    let sealed = chain
        .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
        .await
        .unwrap();
    assert_ne!(
        sealed.to_lowercase(),
        device_root(canonical_field(&dog_tag_id)).to_lowercase(),
        "a raw-handle-bound R must NOT match the canonical R the circuit will prove against"
    );
}

/// FAIL-CLOSED: an unconfigured Level-B deployment refuses, and does so WITHOUT burning the
/// operator's one-time token — a half-wired stack must never consume a QR it cannot fulfil.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unconfigured_level_b_refuses_without_consuming_the_token() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let mut state = mem_state(chain.clone());
    let mut cfg = (*state.cfg).clone();
    cfg.sbt_consent_addr = "0x0000000000000000000000000000000000000000".to_string();
    state.cfg = Arc::new(cfg);
    let app = vet_api::router(state);
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (token, dog_tag_id, _session_id) = start_session(&app, &op).await;
    let root = device_root(canonical_field(&dog_tag_id));

    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({ "token": token, "root": root })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::SERVICE_UNAVAILABLE,
        "unconfigured Level-B must fail closed: {b}"
    );

    // The token survives, so the operator can retry once the deployment is wired.
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(serde_json::json!({ "token": token, "root": root })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::SERVICE_UNAVAILABLE,
        "the refused request must not have consumed the one-time token"
    );
}

/// A malformed or zero `R` is rejected at the edge — mirroring `mintCustodial`'s own
/// `root == 0 -> BadRoot` so an obviously-bad request never reaches the chain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_or_zero_root_is_rejected() {
    let mem = MemChain::new();
    let app = vet_api::router(mem_state(Arc::new(mem)));
    let (_admin, op, _backend) = boot_custody(&app).await;

    for bad in [
        "",
        "0x00",
        "not-hex",
        "0x0000000000000000000000000000000000000000000000000000000000000000",
    ] {
        let (token, _id, _sid) = start_session(&app, &op).await;
        let (s, b) = call(
            &app,
            "POST",
            "/profiles/issue/custodial-bind",
            None,
            Some(serde_json::json!({ "token": token, "root": bad })),
        )
        .await;
        assert_eq!(
            s,
            StatusCode::BAD_REQUEST,
            "root {bad:?} must be rejected: {b}"
        );
    }
}

/// NON-REGRESSION: adding the Level-B path must not disturb the Level-A one. Both routes are live on
/// the same router, and a Level-A bind still mints owner-revealingly to the device wallet on the
/// Level-A SBT, stamping the Level-A version. The cutover is a later milestone; until then the two
/// coexist.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn level_a_issuance_path_still_works_alongside() {
    use alloy::signers::local::PrivateKeySigner;
    use alloy::signers::SignerSync;

    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;
    let (token, dog_tag_id, session_id) = start_session(&app, &op).await;

    let signer = PrivateKeySigner::random();
    let wallet = format!("{:#x}", signer.address()).to_lowercase();
    let sig = signer
        .sign_message_sync(vet_api::auth::register_message(&wallet).as_bytes())
        .unwrap();
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/bind",
        None,
        Some(serde_json::json!({
            "token": token,
            "walletAddress": wallet,
            "signature": format!("0x{}", hex::encode(sig.as_bytes())),
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "Level-A bind must still work: {b}");
    let root = b["root"].as_str().unwrap().to_string();

    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(settled["status"], "bound", "Level-A mint: {settled}");
    assert_eq!(
        settled["protocolVersion"], "dogtag-levela/1",
        "the Level-A path keeps stamping the Level-A version - M-2 flips nothing"
    );

    // It minted on the LEVEL-A SBT, to the wallet, leaving the Level-B SBT untouched.
    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    assert_eq!(
        chain.owner_of(SBT_ADDR, &onchain_id).await.unwrap(),
        wallet,
        "Level-A still mints to the device wallet"
    );
    assert_eq!(
        chain
            .profile_root_of(SBT_ADDR, &onchain_id)
            .await
            .unwrap()
            .to_lowercase(),
        root.to_lowercase()
    );
    assert!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .is_err(),
        "a Level-A issuance must not touch the Level-B SBT"
    );
}

/// FAIL-CLOSED: a MALFORMED (non-zero but unparseable) Level-B address is refused, and — like the
/// unconfigured case — WITHOUT burning the operator's one-time token.
///
/// A bare non-zero test is not enough: `chain::parse_addr` coerces an unparseable address to
/// `Address::ZERO`, so a typo'd `SBT_CONSENT_ADDR` would consume the token and dispatch BOTH
/// `issue(R)` and `mintCustodial` at the zero address — txs that succeed against a codeless address,
/// surfacing only at the read-back, after gas is spent and the QR is burned.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_level_b_address_refuses_without_consuming_the_token() {
    for bad in [
        "0xnotanaddress",                              // non-zero, non-hex
        "0x00000000000000000000000000000000000000d",   // 39 digits — one short
        "0x00000000000000000000000000000000000000ddd", // 41 digits — one long
        "00000000000000000000000000000000000000dd",    // no 0x prefix
    ] {
        let mem = MemChain::new();
        let chain = Arc::new(mem.clone());
        let mut state = mem_state(chain.clone());
        let mut cfg = (*state.cfg).clone();
        cfg.sbt_consent_addr = bad.to_string();
        state.cfg = Arc::new(cfg);
        let app = vet_api::router(state);
        let (_admin, op, _backend) = boot_custody(&app).await;

        let (token, dog_tag_id, _session_id) = start_session(&app, &op).await;
        let root = device_root(canonical_field(&dog_tag_id));
        let body = serde_json::json!({ "token": token, "root": root });

        let (s, b) = call(
            &app,
            "POST",
            "/profiles/issue/custodial-bind",
            None,
            Some(body.clone()),
        )
        .await;
        assert_eq!(
            s,
            StatusCode::SERVICE_UNAVAILABLE,
            "malformed SBT_CONSENT_ADDR {bad:?} must fail closed: {b}"
        );

        // The token survives, so the operator can retry once the deployment is wired correctly.
        let (s, _b) = call(
            &app,
            "POST",
            "/profiles/issue/custodial-bind",
            None,
            Some(body),
        )
        .await;
        assert_eq!(
            s,
            StatusCode::SERVICE_UNAVAILABLE,
            "the refused request must not have consumed the one-time token ({bad:?})"
        );

        // Nothing reached the chain.
        let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
        assert!(
            chain
                .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
                .await
                .is_err(),
            "a refused request must not have minted ({bad:?})"
        );
    }
}

/// A `dogTagId` already RETIRED by a Level-B custodial issuance must never be handed out again.
///
/// `mintCustodial` writes no owner the allocator can see (the holder is the neutral custodian) and
/// retires the id through the write-once `profileRoot[id]`, a marker that survives a burn — so an
/// allocator consulting only the Level-A `ownerOf` reads a Level-B-consumed id as FREE. That is
/// expensive here specifically: `issue(R)` runs BEFORE the mint and `registerRoot` is globally
/// write-once, so the collision would burn both the operator's QR and the device-computed `R`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_level_b_retired_dog_tag_id_is_not_reallocated() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    // Retire the first two ids the counter would hand out, sealed on the LEVEL-B SBT only — exactly
    // the state a prior custodial issuance leaves behind (no owner, non-zero profileRoot).
    for handle in ["1", "2"] {
        let onchain_id = vet_api::routes::onchain_dog_tag_id(handle).unwrap();
        chain
            .mint_custodial(
                0,
                SBT_CONSENT_ADDR,
                &onchain_id,
                &device_root(canonical_field(handle)),
            )
            .await
            .expect("seed a Level-B retired id");
        assert!(
            chain.owner_of(SBT_CONSENT_ADDR, &onchain_id).await.is_err(),
            "the custodial mint must leave the allocator NO owner to see — that is the whole trap"
        );
    }

    let (_token, dog_tag_id, _session_id) = start_session(&app, &op).await;
    assert!(
        dog_tag_id != "1" && dog_tag_id != "2",
        "an id retired by a Level-B custodial issuance must not be re-allocated, got {dog_tag_id}"
    );
    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    assert!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .is_err(),
        "the allocated id must be free on the Level-B SBT"
    );
}

/// The Level-B leg of the allocation check is SKIPPED on a Level-A-only deployment (unset/zero
/// `SBT_CONSENT_ADDR`), so the deployments that exist today keep allocating exactly as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn level_a_only_deployment_still_allocates_from_the_counter() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let mut state = mem_state(chain.clone());
    let mut cfg = (*state.cfg).clone();
    cfg.sbt_consent_addr = "0x0000000000000000000000000000000000000000".to_string();
    state.cfg = Arc::new(cfg);
    let app = vet_api::router(state);
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (_token, dog_tag_id, _session_id) = start_session(&app, &op).await;
    assert_eq!(
        dog_tag_id, "1",
        "with Level-B unconfigured the allocator must not consult it at all"
    );
}
