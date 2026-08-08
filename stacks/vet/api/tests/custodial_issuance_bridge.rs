//! The owner-hidden custodial issuance bridge.
//!
//! These tests drive `POST /profiles/issue/custodial-bind` end to end against `MemChain` and pin the
//! ways it must fail closed.
//!
//! What is asserted, and why each matters:
//!
//! 1. **A REAL device-computed `R` mints.** The root is not a literal - it is folded here by the same
//!    `dogtag_standard::profile_tree::build_profile_tree` the iOS `ProfileTreeStore` calls, so a
//!    change to the device builder moves this test rather than silently desyncing the server.
//! 2. **BOTH on-chain conditions land in one flow** (datamodel §3.5): `R` is sealed as
//!    `profileRoot[dogTagId]` AND anchored so `rootIssuer[R]` resolves and `isValid(R)` holds. These
//!    are the exact two reads owner-hidden verification performs (`VerificationRegistryConsent.sol:163` and
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
        // BLANK identity on purpose: a session with identity carries D1 identity leaves, which the
        // full-leaf-list attestation-integrity gate then requires the bind to open. This file tests
        // the ANCHORING mechanics on the identity-less degrade path; the identity-full path (fold,
        // openings, gate refusals) lives in `custodial_bind_identity_gate.rs`.
        "ownerIdentity": {
            "countryOfIdentification": "",
            "identification": "",
            "name": ""
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

/// The device's bind payload pieces: `R`, the opening of every attribute leaf, and the three
/// opaque reserved leaf hashes the D1 full-leaf-list gate rebuilds `R` from.
struct DeviceWire {
    root: String,
    leaves: serde_json::Value,
    reserved: serde_json::Value,
}

impl DeviceWire {
    fn bind_body(&self, token: &str) -> serde_json::Value {
        serde_json::json!({
            "token": token,
            "root": self.root,
            "leaves": self.leaves,
            "reservedLeafHashes": self.reserved,
        })
    }
}

/// Fold a profile tree exactly as the OWNER'S DEVICE does: the real `build_profile_tree` over a
/// wallet seed the server never sees.
///
/// `dog_tag_id_field` is the KDF binding input - the whole point of the canonical-field discipline is
/// that this parameter must be `field_of_value(Integer(handle))` and not the raw handle, so it is
/// exposed here rather than derived, letting `raw_handle_binding_breaks_the_R_binding_fail_closed`
/// pass the WRONG one deliberately.
fn device_wire(dog_tag_id_field: Fr) -> DeviceWire {
    let mut owner_address = [0u8; 20];
    owner_address[16..].copy_from_slice(&0xdead_beefu32.to_be_bytes());
    let attributes = vec![AttributeLeaf {
        key_path: "credentialSubject.name".to_string(),
        salt: [7u8; SALT_LEN],
        value: TypedScalar::Str("Rex".to_string()),
    }];
    let tree = build_profile_tree(DEVICE_SEED, dog_tag_id_field, &owner_address, &attributes)
        .expect("device-side build_profile_tree");
    DeviceWire {
        root: to_hex32(&tree.root),
        leaves: serde_json::json!([{
            "keyPath": "credentialSubject.name",
            "saltHex": format!("0x{}", hex::encode([7u8; SALT_LEN])),
            "tag": 2,
            "value": "Rex",
        }]),
        reserved: serde_json::json!([
            to_hex32(&tree.owner_address_leaf),
            to_hex32(&tree.consent_key_leaf),
            to_hex32(&tree.owner_secret_leaf),
        ]),
    }
}

/// Just `R`, for the call sites that never reach the bind route's leaf-commitment gate.
fn device_root(dog_tag_id_field: Fr) -> String {
    device_wire(dog_tag_id_field).root
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
        // "minting" is the accepted-but-anchoring intermediate state; poll through it to terminal.
        if b["status"] != "pending" && b["status"] != "minting" {
            return b;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("session {session_id} never left pending");
}

/// THE MILESTONE TEST: a device-computed `R` is accepted by the server, minted owner-blind via
/// `mintCustodial(id, R)`, and anchored via `issue(R)` in the SAME flow — leaving on-chain state that
/// satisfies BOTH conditions an owner-hidden consent verify checks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_computed_root_mints_custodially_and_is_anchored_for_verification() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (token, dog_tag_id, session_id) = start_session(&app, &op).await;

    // The device folds R locally from its wallet seed, binding the CANONICAL dogTagId field. The
    // server never sees the seed, the owner secret, or a wallet address - the bind carries the
    // attribute openings + the opaque reserved leaf hashes, never their preimages.
    let wire = device_wire(canonical_field(&dog_tag_id));
    let root = wire.root.clone();

    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&token)),
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
        "owner-hidden issuance must stamp the unified version"
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

    // (1) sealed: R == profileRoot[dogTagId], under the CANONICAL id.
    let sealed = chain
        .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
        .await
        .expect("profileRoot set on the owner-hidden SBT");
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

    let bad_wire = device_wire(field_from_uint(dog_tag_id.parse::<u64>().unwrap()));
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(bad_wire.bind_body(&token)),
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

/// FAIL-CLOSED: an unconfigured owner-hidden deployment refuses the BIND, and does so WITHOUT
/// burning the operator's one-time token — a half-wired stack must never consume a QR it cannot
/// fulfil.
///
/// The session is started on a CONFIGURED router and the bind arrives on one that lost the anchor
/// address, sharing the same store and custody. That is not test contortion: session start now
/// refuses on an unanchorable backend (see `tests/anchor_readiness.rs`), so within one process this
/// arm is unreachable through the normal flow — but sessions OUTLIVE a process under the Mongo
/// store, so a bind really can arrive on a restart whose env no longer carries the addresses the
/// starting process had. This is the state that keeps the bind-time gate load-bearing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unconfigured_owner_hidden_refuses_without_consuming_the_token() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let state = mem_state(chain.clone());
    let app = vet_api::router(state.clone());
    let (_admin, op, _backend) = boot_custody(&app).await;
    let (token, dog_tag_id, _session_id) = start_session(&app, &op).await;

    // "Restart" with the anchor gone: same store (the session and its token survive), same custody,
    // a config whose SBT address is back at the unset zero default.
    let mut restarted = state.clone();
    let mut cfg = (*restarted.cfg).clone();
    cfg.sbt_consent_addr = "0x0000000000000000000000000000000000000000".to_string();
    restarted.cfg = Arc::new(cfg);
    let app = vet_api::router(restarted);

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
        "unconfigured owner-hidden issuance must fail closed: {b}"
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

/// FAIL-CLOSED: a MALFORMED (non-zero but unparseable) owner-hidden address is refused, and — like the
/// unconfigured case — WITHOUT burning the operator's one-time token.
///
/// A bare non-zero test is not enough: `chain::parse_addr` coerces an unparseable address to
/// `Address::ZERO`, so a typo'd `SBT_CONSENT_ADDR` would consume the token and dispatch BOTH
/// `issue(R)` and `mintCustodial` at the zero address — txs that succeed against a codeless address,
/// surfacing only at the read-back, after gas is spent and the QR is burned.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_owner_hidden_address_refuses_without_consuming_the_token() {
    for bad in [
        "0xnotanaddress",                              // non-zero, non-hex
        "0x00000000000000000000000000000000000000d",   // 39 digits — one short
        "0x00000000000000000000000000000000000000ddd", // 41 digits — one long
        "00000000000000000000000000000000000000dd",    // no 0x prefix
    ] {
        // Started configured, bound after a "restart" whose env mangled the address — the same
        // two-router model as the unconfigured case above, and for the same reason: session start
        // itself now refuses on a misconfigured backend.
        let mem = MemChain::new();
        let chain = Arc::new(mem.clone());
        let state = mem_state(chain.clone());
        let app = vet_api::router(state.clone());
        let (_admin, op, _backend) = boot_custody(&app).await;
        let (token, dog_tag_id, _session_id) = start_session(&app, &op).await;

        let mut restarted = state.clone();
        let mut cfg = (*restarted.cfg).clone();
        cfg.sbt_consent_addr = bad.to_string();
        restarted.cfg = Arc::new(cfg);
        let app = vet_api::router(restarted);

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

/// A `dogTagId` already retired by custodial issuance must never be handed out again.
///
/// `mintCustodial` writes no owner the allocator can see (the holder is the neutral custodian) and
/// retires the id through the write-once `profileRoot[id]`, a marker that survives a burn. That is
/// expensive here specifically: `issue(R)` runs BEFORE the mint and `registerRoot` is globally
/// write-once, so the collision would burn both the operator's QR and the device-computed `R`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retired_dog_tag_id_is_not_reallocated() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    // Retire the first two ids the counter would hand out, exactly as prior custodial issuances do.
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
            .expect("seed a retired id");
    }

    let (_token, dog_tag_id, _session_id) = start_session(&app, &op).await;
    assert!(
        dog_tag_id != "1" && dog_tag_id != "2",
        "an id retired by custodial issuance must not be re-allocated, got {dog_tag_id}"
    );
    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    assert!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .is_err(),
        "the allocated id must be free on the owner-hidden SBT"
    );
}

/// A `dogTagId` sealed on the owner-hidden SBT AFTER its session started must be refused at bind time —
/// and refused having written NOTHING to the chain.
///
/// The allocation check in `/profiles/issue/session/start` cannot cover this: it runs up to 180s
/// earlier, and the SBT is shared across vet instances that each keep their own `next_dog_tag_id`
/// counter, so a collision can open between start and bind. This test seats the collision in exactly
/// that order (start first, so the id IS free when allocated; seal it second) — seeding it before
/// `start_session` would instead exercise the allocator, which already skips sealed ids, and pass
/// vacuously.
///
/// The load-bearing assertion is the LAST one. Refusing with a 409 is not enough on its own: the
/// whole point of moving the check ahead of the spawn is that `issue(R)` — which globally and
/// permanently consumes `R` via write-once `registerRoot` — must never have run. So the device root
/// must still be unanchored afterwards, leaving it reusable in the fresh session the operator is
/// told to start.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_collision_opened_after_session_start_is_refused_before_anything_reaches_the_chain() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _backend) = boot_custody(&app).await;

    // 1. Start the session while the id is genuinely free — the allocator hands it out legitimately.
    let (token, dog_tag_id, session_id) = start_session(&app, &op).await;
    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    assert!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .is_err(),
        "precondition: the id must be free at session start, or this tests the allocator instead"
    );

    // 2. A CONCURRENT issuance (another vet instance) seals the same id before the device binds.
    let other_root = device_root(field_from_uint(999_u64));
    chain
        .mint_custodial(0, SBT_CONSENT_ADDR, &onchain_id, &other_root)
        .await
        .expect("a concurrent instance seals the id");

    // 3. The device binds its own root against the now-colliding id.
    let wire = device_wire(canonical_field(&dog_tag_id));
    let root = wire.root.clone();
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&token)),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "a sealed dogTagId must be refused with a specific conflict, not a generic mint error: {b}"
    );
    let msg = b["error"].as_str().unwrap_or_default().to_string();
    assert!(
        msg.contains(&dog_tag_id) && msg.contains("FRESH session"),
        "the refusal must name the colliding dogTagId and say a fresh session is required: {msg}"
    );

    // The operator poll must surface that same specific reason, not a stalled `pending` row.
    let (s, row) = call(
        &app,
        "GET",
        &format!("/profiles/issue/session/{session_id}"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(row["status"], "error", "the session row must be settled");
    assert!(
        row["error"]
            .as_str()
            .unwrap_or_default()
            .contains(&dog_tag_id),
        "the operator must see WHY it failed, in the reason's own field: {row}"
    );
    assert!(
        row["txHash"].is_null(),
        "a failed bind's txHash is never prose — the reason has its own field: {row}"
    );

    // THE LOAD-BEARING ASSERTION: `issue(R)` never ran, so the device's `R` was not burned. Assert
    // against PROFILE_ISSUER_ADDR — the clone this route actually anchors to. Using any other clone
    // (e.g. the VACCINATION `ISSUER`) reads as unanchored no matter what the handler did, which makes
    // the assertion pass vacuously and stops it testing anything.
    assert!(
        !chain.is_valid(PROFILE_ISSUER_ADDR, &root).await.unwrap(),
        "the refusal must precede issue(R) — a burned R can never be re-anchored (registerRoot is \
         globally write-once), so the device would lose it to a check that fired too late"
    );
    assert!(
        chain
            .issued_at(PROFILE_ISSUER_ADDR, &root)
            .await
            .unwrap()
            .is_zero(),
        "no anchoring tx may have been sent for the device root"
    );

    // The pre-existing seal is untouched — the refusal wrote nothing at all.
    assert_eq!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .unwrap(),
        other_root.to_lowercase(),
        "the refused bind must not have disturbed the concurrent issuance's seal"
    );
}

// ============================================================================================
// A FAILED on-chain mint tells everyone (2026-08-07 live failure): the operator's log gets a
// line naming the stage, the session row carries the stage + reason in their own fields, and
// the DEVICE can learn it through the token status peek instead of chain-polling into a guess.
// ============================================================================================

/// Poll the device status peek (`GET /p/{token}/status`) until it leaves pending/minting.
async fn await_device_status(app: &axum::Router, token: &str) -> serde_json::Value {
    for _ in 0..200 {
        let (s, b) = call(app, "GET", &format!("/p/{token}/status"), None, None).await;
        assert_eq!(s, StatusCode::OK, "device peek must answer: {b}");
        if b["status"] != "pending" && b["status"] != "minting" {
            return b;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("device peek never settled");
}

/// The captain's exact failure, end to end: nobody holds the SBT's ISSUER_ROLE, `issue(R)` lands,
/// `mintCustodial` reverts — and NOW the session names the stage and reason in their own fields
/// (never `txHash`), and the device learns "error" through the consumed token's status peek.
///
/// The role READ is failed alongside (fault injection) so the bind's could-not-check gate lets the
/// attempt through — which also pins the gate's license: could-not-check never refuses. A READABLE
/// missing role is refused before the token is consumed (its own test below).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_mint_settles_the_session_and_the_device_learns_it() {
    let mem = MemChain::new();
    let chain: Arc<dyn ChainClient> = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, signer) = boot_custody(&app).await;

    // The live-walk state: the role is NOT held, and the gate cannot see that (read fails).
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, false);
    mem.set_sbt_role_reads_failing(true);

    let (token, dog_tag_id, session_id) = start_session(&app, &op).await;
    let wire = device_wire(canonical_field(&dog_tag_id));
    let root = wire.root.clone();
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&token)),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "bind accepted (gate could not check): {b}"
    );

    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(settled["status"], "error", "the row must settle: {settled}");
    assert_eq!(
        settled["errorStage"], "mint",
        "the stage is named in its own field: {settled}"
    );
    let reason = settled["error"].as_str().unwrap_or_default();
    assert!(
        reason.contains("mintCustodial") && reason.contains("Retry issuance"),
        "the reason names the failed call AND the way forward: {reason}"
    );
    assert!(
        settled["txHash"].is_null(),
        "txHash never carries prose: {settled}"
    );

    // The stranded-root state is real: issue(R) landed, the mint did not.
    assert!(
        !chain
            .issued_at(PROFILE_ISSUER_ADDR, &root)
            .await
            .unwrap()
            .is_zero(),
        "issue(R) landed before the mint failed"
    );
    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    assert!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .is_err(),
        "nothing was minted"
    );

    // THE DEVICE LEARNS IT — through the CONSUMED token, with a stage sentence and no operator
    // detail (no chain error text, no env vars).
    let peek = await_device_status(&app, &token).await;
    assert_eq!(peek["status"], "error");
    let device_reason = peek["reason"].as_str().unwrap_or_default();
    assert!(
        device_reason.contains("could not be minted")
            && device_reason.contains("retry this same issuance"),
        "the device gets the mint-stage sentence: {device_reason}"
    );
    assert!(
        !device_reason.contains("mintCustodial") && !device_reason.contains("sbt role read"),
        "the device sentence must not carry the operator's chain detail: {device_reason}"
    );

    // The pre-bind resolve is CONSUMED (404) even though the status peek still answers: retention
    // must not reopen the identity-bearing resolve.
    let (s, _b) = call(&app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "the resolve must 404 after the bind consumed the token"
    );
}

/// A READABLE missing role refuses at every operator surface BEFORE anything is spent: session
/// start refuses with nothing allocated, and a bind against an already-issued QR refuses WITHOUT
/// consuming the one-time token — the same QR completes once the role is granted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_mint_role_refuses_at_start_and_preserves_the_bind_token() {
    let mem = MemChain::new();
    let chain: Arc<dyn ChainClient> = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, signer) = boot_custody(&app).await;

    // Start a session while the role is held, so a QR exists…
    let (token, dog_tag_id, _session_id) = start_session(&app, &op).await;

    // …then the role turns out missing (readable this time).
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, false);

    // Session START refuses in the operator's vocabulary, pointing at the admin portal's card.
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/session/start",
        Some(&op),
        Some(start_body()),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "start must refuse: {b}");
    let msg = b["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("Dog-tag mint role") && msg.contains(&signer.to_lowercase()),
        "the refusal names the signer and the admin-portal card: {msg}"
    );

    // The BIND refuses too — before consuming the token.
    let wire = device_wire(canonical_field(&dog_tag_id));
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&token)),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "bind must refuse");

    // Grant the role: the SAME QR now completes — the refusal spent nothing.
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, true);
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "the preserved token binds: {b}");
}

/// A failing `issue(R)` (root already registered by a FOREIGN clone — globally write-once) settles
/// the session at stage "issue" and the device learns it. Nothing was minted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_issue_settles_the_session_and_the_device_learns_it() {
    let mem = MemChain::new().with_factory(FACTORY_ADDR);
    let chain: Arc<dyn ChainClient> = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _signer) = boot_custody(&app).await;

    let (token, dog_tag_id, session_id) = start_session(&app, &op).await;
    let wire = device_wire(canonical_field(&dog_tag_id));
    let root = wire.root.clone();

    // A foreign clone registers the same root first — `registerRoot` is globally write-once, so
    // OUR issue(R) can only revert "root taken".
    chain
        .issue(0, "0x00000000000000000000000000000000000000cc", &root)
        .await
        .expect("foreign clone takes the root");

    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(settled["status"], "error");
    assert_eq!(settled["errorStage"], "issue", "stage named: {settled}");
    let reason = settled["error"].as_str().unwrap_or_default();
    assert!(
        reason.contains("issue(R)") && reason.contains("root taken"),
        "the reason carries the underlying revert: {reason}"
    );

    let peek = await_device_status(&app, &token).await;
    assert_eq!(peek["status"], "error");
    assert!(
        peek["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("anchoring of this dog tag failed"),
        "the device gets the issue-stage sentence: {peek}"
    );
}

/// THE RECOVERY, end to end — the captain's stranded root completed. An earlier attempt anchored
/// R (issue(R) landed) and the mint failed; the operator fixes the cause (grants the role) and
/// RETRIES THE SAME SESSION. The device re-posts the same root off its persisted record, the
/// resume check skips the impossible re-issue, and the mint completes the tag.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_completes_a_stranded_root_after_the_cause_is_fixed() {
    let mem = MemChain::new();
    let chain: Arc<dyn ChainClient> = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, signer) = boot_custody(&app).await;

    // ---- the failure: role missing, gate blind, mint reverts, root stranded ----
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, false);
    mem.set_sbt_role_reads_failing(true);
    let (token, dog_tag_id, session_id) = start_session(&app, &op).await;
    let wire = device_wire(canonical_field(&dog_tag_id));
    let root = wire.root.clone();
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(settled["status"], "error");
    assert_eq!(settled["errorStage"], "mint");

    // ---- the fix: the admin grants ISSUER_ROLE (the admin portal's card does this on chain) ----
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, true);
    mem.set_sbt_role_reads_failing(false);

    // ---- the retry: same session, fresh token, same dogTagId + identity salts ----
    let (s, b) = call(
        &app,
        "POST",
        &format!("/profiles/issue/session/{session_id}/retry"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "retry must re-arm the session: {b}");
    assert_eq!(
        b["dogTagId"].as_str().unwrap(),
        dog_tag_id,
        "the SAME dogTagId — a fresh session would derive a DIFFERENT root and strand this one"
    );
    let retry_token = b["token"].as_str().unwrap().to_string();
    assert_ne!(retry_token, token, "a fresh one-time token is minted");

    // The device re-scans: same session metadata, same identity salts -> the persisted profile
    // record rebuilds the SAME root, and the bind resumes at the mint.
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&retry_token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "retry bind: {b}");
    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(
        settled["status"], "bound",
        "the stranded root completes: {settled}"
    );
    assert!(settled["txHash"].as_str().is_some(), "mint tx recorded");
    assert!(
        settled["error"].is_null() && settled["errorStage"].is_null(),
        "a completed retry clears the failure fields: {settled}"
    );

    // Both on-chain conditions hold — the tag is verifiable.
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

    // A retry of the now-BOUND session is refused: only failed issuances re-arm.
    let (s, b) = call(
        &app,
        "POST",
        &format!("/profiles/issue/session/{session_id}/retry"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "bound sessions cannot re-arm: {b}");
}

/// The device peek follows the whole HAPPY path too: pending before the bind, bound after — and
/// the peek keeps answering through the token the bind consumed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_device_peek_reports_pending_then_bound_across_the_consumed_token() {
    let mem = MemChain::new();
    let chain: Arc<dyn ChainClient> = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _signer) = boot_custody(&app).await;

    let (token, dog_tag_id, session_id) = start_session(&app, &op).await;
    let (s, b) = call(&app, "GET", &format!("/p/{token}/status"), None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["status"], "pending", "before the bind: {b}");
    assert!(b["reason"].is_null(), "no failure, no reason: {b}");

    let wire = device_wire(canonical_field(&dog_tag_id));
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let _ = await_settled(&app, &op, &session_id).await;

    let peek = await_device_status(&app, &token).await;
    assert_eq!(peek["status"], "bound", "after completion: {peek}");
    assert_eq!(peek["dogTagId"].as_str().unwrap(), dog_tag_id);

    // An unknown token stays a 404 — the peek is not an enumeration surface.
    let (s, _b) = call(
        &app,
        "GET",
        "/p/00ff00ff00ff00ff00ff00ff00ff00ff/status",
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

/// An interrupted issuance (anchored, unminted — e.g. a crash between the two transactions)
/// resumes at the mint on an ordinary bind: `issued_at != 0` on OUR clone means stage (1) is
/// already done, and re-running it could only revert "root taken".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_already_anchored_root_resumes_at_the_mint_instead_of_reissuing() {
    let mem = MemChain::new().with_factory(FACTORY_ADDR);
    let chain: Arc<dyn ChainClient> = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, _signer) = boot_custody(&app).await;

    let (token, dog_tag_id, session_id) = start_session(&app, &op).await;
    let wire = device_wire(canonical_field(&dog_tag_id));
    let root = wire.root.clone();

    // The earlier attempt's issue(R) already landed on OUR clone. With the factory modelled,
    // a second issue(R) would revert "root taken" — so completing proves the resume skipped it.
    chain
        .issue(0, PROFILE_ISSUER_ADDR, &root)
        .await
        .expect("the interrupted attempt anchored the root");

    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(
        settled["status"], "bound",
        "resumed and completed: {settled}"
    );

    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    assert_eq!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .unwrap()
            .to_lowercase(),
        root.to_lowercase()
    );
}

/// THE LOG LINE — the change that turns a two-hour hunt into one line. A failing background arm
/// must emit a real `tracing::error!` naming the stage, the dogTagId and the underlying error,
/// where an operator tailing the backend actually looks. Captured through a real subscriber; the
/// 2026-08-07 failure emitted NOTHING.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_background_arm_logs_where_the_operator_looks() {
    use std::io::Write;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    let sink = Capture(Arc::new(Mutex::new(Vec::new())));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(sink.clone())
        .with_ansi(false)
        .finish();
    // Global, not thread-scoped: the arm runs inside a spawned tokio task on another worker
    // thread, which a `set_default` guard would not cover.
    let _ = tracing::subscriber::set_global_default(subscriber);

    let mem = MemChain::new();
    let chain: Arc<dyn ChainClient> = Arc::new(mem.clone());
    let app = vet_api::router(mem_state(chain.clone()));
    let (_admin, op, signer) = boot_custody(&app).await;
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, false);
    mem.set_sbt_role_reads_failing(true);

    let (token, dog_tag_id, session_id) = start_session(&app, &op).await;
    let wire = device_wire(canonical_field(&dog_tag_id));
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let _ = await_settled(&app, &op, &session_id).await;

    let log = String::from_utf8_lossy(&sink.0.lock().unwrap()).to_string();
    assert!(
        log.contains("mintCustodial") && log.contains(&dog_tag_id) && log.contains("ERROR"),
        "the operator's log must name the stage and the dogTagId at ERROR level; got:\n{log}"
    );
}
