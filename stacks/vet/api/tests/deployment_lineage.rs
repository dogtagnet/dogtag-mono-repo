//! A dog tag's identity is (deployment, id) — the issuance surfaces must refuse a clone that
//! belongs to a different deployment, and the id allocator must never hand out an id the SBT did
//! not vouch for.
//!
//! Both faults were measured live on 2026-08-09 after a redeploy:
//!
//! * `contracts/.env` still named the previous generation's `PROFILE_ISSUER_ADDR` (the env sync
//!   deliberately preserves per-provider clone variables), the vet `issue(R)`'d into the old
//!   clone and `mintCustodial`'d into the NEW SBT, and the result was a credential no verifier
//!   can attribute — `rootIssuer(R)` is zero on the new factory, forever. Nothing refused it.
//! * `dog_tag_seq` consulted the chain but FAILED OPEN: an unreadable read `break`'d out of the
//!   loop and handed out the very id it could not vouch for, and 256 consecutive taken ids
//!   proceeded with a TAKEN id.
//!
//! Verdict discipline throughout: only a DEFINITE mismatch refuses; could-not-check warns (the
//! `deploymentLineage` response field) — except the allocator, where could-not-check REFUSES,
//! because a guessed id burns a write-once root and a one-time QR while a refused start costs one
//! retry.

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;

use ark_bn254::Fr;
use dogtag_standard::field::to_hex32;
use dogtag_standard::leaf::field_of_value;
use dogtag_standard::profile_tree::{build_profile_tree, AttributeLeaf, SALT_LEN};
use dogtag_standard::types::TypedScalar;
use vet_api::chain::MemChain;

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const VACCINATION_ISSUER: &str = "0x00000000000000000000000000000000000000bb";
/// A factory from a different (superseded or hand-typed) deployment.
const OTHER_FACTORY: &str = "0x00000000000000000000000000000000000000f1";

const DEVICE_SEED: &[u8] = b"DogTag deployment-lineage test seed - TEST MATERIAL ONLY";

fn start_body() -> serde_json::Value {
    serde_json::json!({
        "ownerIdentity": { "countryOfIdentification": "", "identification": "", "name": "" },
        "pet": { "name": "Rex" }
    })
}

fn mem_state(mem: &MemChain) -> vet_api::app::AppState {
    state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        REGISTRY.to_string(),
        VACCINATION_ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    )
}

/// Seed the chain so the configured set reads as ONE deployment: the verification registry pins
/// the configured factory + SBT, and `clone` is a member of that factory.
fn seed_matched(mem: &MemChain, clone: &str) {
    mem.set_registry_pins(VREG_CONSENT_ADDR, FACTORY_ADDR, SBT_CONSENT_ADDR);
    mem.set_factory_clone(FACTORY_ADDR, clone, true);
}

async fn start(app: &axum::Router, op: &str) -> (StatusCode, serde_json::Value) {
    call(
        app,
        "POST",
        "/profiles/issue/session/start",
        Some(op),
        Some(start_body()),
    )
    .await
}

/// The canonical on-chain id for a decimal handle — what the SBT keys `profileRoot` by.
fn onchain_id(handle: &str) -> String {
    to_hex32(&field_of_value(&TypedScalar::Integer(handle.to_string())).expect("field"))
}

/// A real device-built root + bind body for `handle`, exactly as the phone folds it.
fn device_bind_body(handle: &str, token: &str) -> serde_json::Value {
    let id_field: Fr = field_of_value(&TypedScalar::Integer(handle.to_string())).expect("field");
    let mut owner_address = [0u8; 20];
    owner_address[16..].copy_from_slice(&0xdead_beefu32.to_be_bytes());
    let attributes = vec![AttributeLeaf {
        key_path: "credentialSubject.name".to_string(),
        salt: [7u8; SALT_LEN],
        value: TypedScalar::Str("Rex".to_string()),
    }];
    let tree = build_profile_tree(DEVICE_SEED, id_field, &owner_address, &attributes)
        .expect("device-side build_profile_tree");
    serde_json::json!({
        "token": token,
        "root": to_hex32(&tree.root),
        "leaves": [{
            "keyPath": "credentialSubject.name",
            "saltHex": format!("0x{}", hex::encode([7u8; SALT_LEN])),
            "tag": 2,
            "value": "Rex",
        }],
        "reservedLeafHashes": [
            to_hex32(&tree.owner_address_leaf),
            to_hex32(&tree.consent_key_leaf),
            to_hex32(&tree.owner_secret_leaf),
        ],
    })
}

/// Poll the operator status route to a terminal state.
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
        if b["status"] != "pending" && b["status"] != "minting" {
            return b;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("session {session_id} never left pending");
}

/// The captain's exact state, refused where the operator can act: the configured profile clone is
/// NOT a clone of the deployment's factory, so session start 503s naming the foreign contract,
/// the factory asked, the env var, and the fix — and allocates nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_foreign_profile_clone_refuses_session_start_before_anything_is_allocated() {
    let mem = MemChain::new();
    mem.set_registry_pins(VREG_CONSENT_ADDR, FACTORY_ADDR, SBT_CONSENT_ADDR);
    mem.set_factory_clone(FACTORY_ADDR, PROFILE_ISSUER_ADDR, false);
    let state = mem_state(&mem);
    let store = state.store.clone();
    let app = vet_api::router(state);
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    let msg = b["error"].as_str().unwrap();
    assert!(msg.contains(PROFILE_ISSUER_ADDR), "names the foreign clone: {msg}");
    assert!(msg.contains(FACTORY_ADDR), "names the factory asked: {msg}");
    assert!(msg.contains("PROFILE_ISSUER_ADDR"), "names the env var: {msg}");
    assert!(
        msg.contains("preserves per-provider clone addresses"),
        "names why the variable went stale and that it is the operator's to move: {msg}"
    );
    assert!(msg.contains("Nothing was allocated"), "{msg}");
    // The refusal fired before the allocator ran: the counter was never touched.
    assert_eq!(
        store.next_dog_tag_id().await,
        1,
        "a refused start must not have consumed a dog tag id"
    );
}

/// A configuration that itself mixes deployments — a factory the verification registry does not
/// resolve roots through — refuses with the config-level message, whatever the clone answers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_configuration_mixing_deployments_refuses_at_session_start() {
    let mem = MemChain::new();
    mem.set_registry_pins(VREG_CONSENT_ADDR, OTHER_FACTORY, SBT_CONSENT_ADDR);
    mem.set_factory_clone(FACTORY_ADDR, PROFILE_ISSUER_ADDR, true);
    let app = vet_api::router(mem_state(&mem));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    let msg = b["error"].as_str().unwrap();
    assert!(msg.contains("mixes deployments"), "{msg}");
    assert!(msg.contains(VREG_CONSENT_ADDR), "names the registry whose pin disagrees: {msg}");
}

/// A matched set still issues, end to end: start reports the lineage CONFIRMED, and a real
/// device-built root binds, anchors and mints exactly as before this gate existed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_matched_set_still_issues_end_to_end() {
    let mem = MemChain::new();
    seed_matched(&mem, PROFILE_ISSUER_ADDR);
    let app = vet_api::router(mem_state(&mem));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["deploymentLineage"]["state"], "confirmed", "{b}");
    let token = b["token"].as_str().unwrap().to_string();
    let dog_tag_id = b["dogTagId"].as_str().unwrap().to_string();
    let session_id = b["sessionId"].as_str().unwrap().to_string();

    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(device_bind_body(&dog_tag_id, &token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "bind: {b}");
    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(settled["status"], "bound", "{settled}");
}

/// Could-not-check is never a refusal and never a silent pass: an unseeded chain proceeds, and
/// the response says the lineage is UNKNOWN with the reason.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unestablished_lineage_warns_in_the_response_and_never_refuses() {
    let mem = MemChain::new(); // no pins, no membership: both reads fail
    let app = vet_api::router(mem_state(&mem));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["deploymentLineage"]["state"], "unknown", "{b}");
    assert!(
        b["deploymentLineage"]["detail"]
            .as_str()
            .unwrap()
            .contains("could not be established"),
        "{b}"
    );
}

/// The bind's own gate, for the restart case session-start cannot cover: the session and token
/// outlive the process, and the environment the process restarts with can name a foreign clone.
/// The refusal lands BEFORE the one-time token is consumed, so fixing the config rescues the same
/// QR — proven by completing the bind after the chain confirms membership.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_foreign_clone_refuses_the_bind_before_the_token_is_consumed() {
    let mem = MemChain::new(); // unseeded at start: lineage unknown, start proceeds
    let app = vet_api::router(mem_state(&mem));
    let (_admin, op, _backend) = boot_custody(&app).await;
    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let token = b["token"].as_str().unwrap().to_string();
    let dog_tag_id = b["dogTagId"].as_str().unwrap().to_string();
    let session_id = b["sessionId"].as_str().unwrap().to_string();

    // The chain now answers definitively: the configured clone is another deployment's.
    mem.set_registry_pins(VREG_CONSENT_ADDR, FACTORY_ADDR, SBT_CONSENT_ADDR);
    mem.set_factory_clone(FACTORY_ADDR, PROFILE_ISSUER_ADDR, false);

    let body = device_bind_body(&dog_tag_id, &token);
    let (s, b) = call(&app, "POST", "/profiles/issue/custodial-bind", None, Some(body.clone())).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    assert!(
        b["error"].as_str().unwrap().contains("not a clone of this deployment's factory"),
        "{b}"
    );

    // Not 410: the one-time token survived the refusal.
    let (s, b) = call(&app, "POST", "/profiles/issue/custodial-bind", None, Some(body.clone())).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "the refusal must not consume the token: {b}");

    // With membership established, the SAME token completes the issuance.
    mem.set_factory_clone(FACTORY_ADDR, PROFILE_ISSUER_ADDR, true);
    let (s, b) = call(&app, "POST", "/profiles/issue/custodial-bind", None, Some(body)).await;
    assert_eq!(s, StatusCode::OK, "bind after fix: {b}");
    let settled = await_settled(&app, &op, &session_id).await;
    assert_eq!(settled["status"], "bound", "{settled}");
}

/// `credentials/prepare` refuses a record-type clone from another deployment, naming ITS env var
/// — and proceeds for a member clone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepare_refuses_a_foreign_record_type_clone_and_passes_a_member() {
    let mem = MemChain::new();
    mem.set_registry_pins(VREG_CONSENT_ADDR, FACTORY_ADDR, SBT_CONSENT_ADDR);
    mem.set_factory_clone(FACTORY_ADDR, VACCINATION_ISSUER, false);
    let app = vet_api::router(mem_state(&mem));
    let (_admin, op, backend) = boot_custody(&app).await;

    let req = serde_json::json!({
        "recordType": "VACCINATION",
        "dogTagId": "7",
        "fields": vaccination_fields(),
    });
    let (s, b) = call(&app, "POST", "/credentials/prepare", Some(&op), Some(req.clone())).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    let msg = b["error"].as_str().unwrap();
    assert!(msg.contains(VACCINATION_ISSUER), "names the foreign clone: {msg}");
    assert!(msg.contains("VACCINATION_ISSUER_ADDR"), "names the env var: {msg}");

    // The same request anchors once the clone is a member (the two-layer authority seeded so the
    // pre-existing issuance preflight — untouched by this gate — passes as before).
    mem.set_factory_clone(FACTORY_ADDR, VACCINATION_ISSUER, true);
    mem.set_governing_registry(VACCINATION_ISSUER, REGISTRY);
    mem.set_issuance_capability(REGISTRY, VACCINATION_ISSUER, &backend, true);
    let (s, b) = call(&app, "POST", "/credentials/prepare", Some(&op), Some(req)).await;
    assert_eq!(s, StatusCode::OK, "member clone must still prepare and anchor: {b}");
}

/// THE ALLOCATOR IS FAIL-CLOSED: an unreadable SBT refuses the start — it never hands out an id
/// it could not vouch for — and the same start succeeds once the chain answers again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreadable_sbt_refuses_allocation_rather_than_guessing() {
    let mem = MemChain::new();
    seed_matched(&mem, PROFILE_ISSUER_ADDR);
    mem.set_failing_profile_root_reads(true);
    let app = vet_api::router(mem_state(&mem));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    let msg = b["error"].as_str().unwrap();
    assert!(msg.contains("cannot vouch for"), "{msg}");
    assert!(msg.contains("no QR was drawn"), "{msg}");

    mem.set_failing_profile_root_reads(false);
    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::OK, "recovers once the chain answers: {b}");
}

/// A restart that lost the journal does not collide with already-minted ids: the allocator skips
/// every id the SBT says is taken. 256 consecutive taken ids REFUSE rather than proceeding with a
/// taken id — and the refusal itself advances the counter, so a retry makes progress.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn minted_ids_are_skipped_and_exhaustion_refuses_rather_than_reusing() {
    let mem = MemChain::new();
    seed_matched(&mem, PROFILE_ISSUER_ADDR);
    // A chain holding tags 1..=2 minted by a previous journal's life.
    for handle in 1..=2u64 {
        mem.set_profile_root(
            SBT_CONSENT_ADDR,
            &onchain_id(&handle.to_string()),
            "0x1111111111111111111111111111111111111111111111111111111111111111",
        );
    }
    let app = vet_api::router(mem_state(&mem));
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["dogTagId"], "3", "a fresh counter must skip past minted ids: {b}");

    // Now a chain 300 tags deep: the fresh counter (at 3 after the skip + start above... the next
    // probe starts at 4) meets >256 consecutive minted ids and must refuse, never hand one out.
    for handle in 3..=300u64 {
        mem.set_profile_root(
            SBT_CONSENT_ADDR,
            &onchain_id(&handle.to_string()),
            "0x1111111111111111111111111111111111111111111111111111111111111111",
        );
    }
    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    assert!(
        b["error"].as_str().unwrap().contains("behind the chain"),
        "{b}"
    );

    // The refused probes advanced the counter past what they probed; a retry reaches free ids.
    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::OK, "the retry continues from the advanced counter: {b}");
    let allocated: u64 = b["dogTagId"].as_str().unwrap().parse().unwrap();
    assert!(allocated > 300, "must land past the minted range: {b}");
}
