//! The Register-pet QR deadline: sized to what it protects, started where the device's work starts.
//!
//! What went wrong (measured live 2026-08-07): the bind token died 180s after the OPERATOR drew the
//! QR, while the owner's device was built to work for longer than that — the deadline covered steps
//! the operator does not control (the owner walking over, opening the app, scanning) and then
//! expired underneath a device that had already picked the session up. The portal showed "expired"
//! for a run that was still legitimately in progress.
//!
//! The rule now under test: the token's clock is TWO windows. A SCAN window from mint (how long the
//! QR may sit on the operator's screen unclaimed) and, from the FIRST `GET /p/{token}` resolve, a
//! guaranteed BIND window (the device's remaining work: read, tap, biometrics, fold, POST). The
//! resolve may only ever EXTEND the deadline, only once, and the token stays strictly one-time.
//!
//! The headline case is the mismatch that shipped: a device that picks the session up near the scan
//! deadline must still be able to complete — a client's honest work must never outlive its own
//! server's deadline.

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
const DEVICE_SEED: &[u8] = b"DogTag bind-deadline test seed - TEST MATERIAL ONLY";

/// Mirrors the constants in `routes.rs`; the assertions below only need their ORDER of magnitude
/// (scan window minutes-long, bind window >= the device's realistic work), but pinning the values
/// makes a silent regression to the old 180s loud.
const SCAN_TTL: u64 = 600;
const BIND_TTL: u64 = 300;

fn start_body() -> serde_json::Value {
    serde_json::json!({
        // Blank identity: this file tests DEADLINE mechanics on the identity-less degrade path;
        // the identity-full gate lives in `custodial_bind_identity_gate.rs`.
        "ownerIdentity": { "countryOfIdentification": "", "identification": "", "name": "" },
        "pet": { "name": "Rex" }
    })
}

/// Fold a profile tree exactly as the OWNER'S DEVICE does (same shape as the bridge tests).
fn device_bind_body(dog_tag_id: &str, token: &str) -> serde_json::Value {
    let field: Fr =
        field_of_value(&TypedScalar::Integer(dog_tag_id.to_string())).expect("canonical field");
    let mut owner_address = [0u8; 20];
    owner_address[16..].copy_from_slice(&0xdead_beefu32.to_be_bytes());
    let attributes = vec![AttributeLeaf {
        key_path: "credentialSubject.name".to_string(),
        salt: [7u8; SALT_LEN],
        value: TypedScalar::Str("Rex".to_string()),
    }];
    let tree = build_profile_tree(DEVICE_SEED, field, &owner_address, &attributes)
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

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

struct Harness {
    app: axum::Router,
    store: Arc<dyn vet_api::store::Store>,
    op: String,
}

async fn harness() -> Harness {
    let chain = Arc::new(MemChain::new()) as Arc<dyn ChainClient>;
    let state = state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let store = state.store.clone();
    let app = vet_api::router(state);
    let (_admin, op, _backend) = boot_custody(&app).await;
    Harness { app, store, op }
}

async fn start_session(h: &Harness) -> (String, String, String) {
    let (s, b) = call(
        &h.app,
        "POST",
        "/profiles/issue/session/start",
        Some(&h.op),
        Some(start_body()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    assert_eq!(
        b["ttlSecs"].as_u64().unwrap(),
        SCAN_TTL,
        "the start response states the scan window so the portal never runs its own clock: {b}"
    );
    assert!(
        b["qrAddress"]["check"].as_str().is_some(),
        "the start response carries the QR-address self-check: {b}"
    );
    (
        b["token"].as_str().unwrap().to_string(),
        b["dogTagId"].as_str().unwrap().to_string(),
        b["sessionId"].as_str().unwrap().to_string(),
    )
}

async fn status(h: &Harness, session_id: &str) -> serde_json::Value {
    let (s, b) = call(
        &h.app,
        "GET",
        &format!("/profiles/issue/session/{session_id}"),
        Some(&h.op),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "status poll: {b}");
    b
}

/// Rewind the token's deadline (token store AND the session mirror) as if the QR had already sat on
/// screen for most of the scan window. Wall clocks cannot be advanced in a test; moving the
/// deadline toward `now` is the same instant, reached from the other side.
async fn age_token_to(h: &Harness, token: &str, session_id: &str, exp: u64) {
    h.store.put_bind_token(token, session_id, exp).await;
    let mut s = h
        .store
        .get_profile_session(session_id)
        .await
        .expect("session exists");
    s.token_exp = exp;
    h.store.update_profile_session(s).await;
}

/// THE MISMATCH TEST — the one that would have caught the shipped defect: a device that picks the
/// session up JUST before the scan deadline must still be granted the full bind window, so its
/// bind lands where the old mint-anchored clock would already have killed the token.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pickup_near_the_scan_deadline_still_gets_the_whole_bind_window() {
    let h = harness().await;
    let (token, dog_tag_id, session_id) = start_session(&h).await;

    // The QR has been sitting on the operator's screen: 2 seconds of scan window remain.
    age_token_to(&h, &token, &session_id, now() + 2).await;

    // The device picks the session up in time…
    let (s, b) = call(&h.app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "resolve: {b}");
    let granted = b["ttlSecs"].as_u64().unwrap();
    assert!(
        granted >= BIND_TTL - 5,
        "a resolve guarantees the device its whole bind window, got {granted}s: {b}"
    );

    // …and the operator's status poll now reports the pickup and the extended deadline.
    let st = status(&h, &session_id).await;
    assert!(
        st["resolvedAt"].as_u64().is_some(),
        "the portal can tell a picked-up session from an unclaimed one: {st}"
    );
    assert!(
        st["tokenSecondsLeft"].as_u64().unwrap() >= BIND_TTL - 5,
        "the server's deadline is what the portal reports: {st}"
    );

    // Wall-clock past the ORIGINAL deadline: under the old mint-anchored clock the token is dead
    // here and the device's bind would be refused 410 while it still shows "working".
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let (s, b) = call(
        &h.app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(device_bind_body(&dog_tag_id, &token)),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "the bind must outlive the scan deadline once a device has resolved: {b}"
    );
}

/// Expiry is not weakened: an untouched token still dies, and a dead token is refused on BOTH
/// routes — resolve (404) and bind (410) — with the status poll reporting zero seconds left and no
/// pickup, which is the portal's "no device ever arrived" state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unclaimed_token_still_dies_and_a_dead_token_still_refuses() {
    let h = harness().await;
    let (token, dog_tag_id, session_id) = start_session(&h).await;

    age_token_to(&h, &token, &session_id, now() - 1).await;

    let (s, _b) = call(&h.app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a dead token no longer resolves");

    let (s, b) = call(
        &h.app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(device_bind_body(&dog_tag_id, &token)),
    )
    .await;
    assert_eq!(s, StatusCode::GONE, "a dead token no longer binds: {b}");

    let st = status(&h, &session_id).await;
    assert_eq!(st["status"], "pending");
    assert_eq!(
        st["tokenSecondsLeft"].as_u64(),
        Some(0),
        "the poll reports the death rather than leaving the portal to guess: {st}"
    );
    assert!(
        st["resolvedAt"].is_null(),
        "no device ever picked this one up, and the poll says so: {st}"
    );
}

/// Only the FIRST resolve moves the deadline: repeated polls of `/p/` must not keep a leaked token
/// alive indefinitely. The sentinel makes the assertion deterministic — an extending second
/// resolve would overwrite it with `now + BIND_TTL`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_resolve_does_not_extend_the_deadline_again() {
    let h = harness().await;
    let (token, _dog_tag_id, session_id) = start_session(&h).await;

    let (s, _b) = call(&h.app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK);
    let after_first = h
        .store
        .get_profile_session(&session_id)
        .await
        .unwrap()
        .resolved_at
        .expect("first resolve recorded");

    // Sentinel: still in the future (so the token resolves), distinguishable from any max() result.
    let sentinel = now() + 7;
    age_token_to(&h, &token, &session_id, sentinel).await;

    let (s, _b) = call(&h.app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK);

    let session = h.store.get_profile_session(&session_id).await.unwrap();
    assert_eq!(
        session.token_exp, sentinel,
        "a second resolve must not move the deadline"
    );
    assert_eq!(
        session.resolved_at,
        Some(after_first),
        "the pickup timestamp is the FIRST resolve's"
    );
}

/// An early pickup never SHORTENS the scan window: with most of the window left, max() keeps it, so
/// an owner who scans immediately and then detours (wallet creation, a re-scan) is not worse off
/// than one who never scanned.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_early_pickup_keeps_the_full_scan_window() {
    let h = harness().await;
    let (token, _dog_tag_id, session_id) = start_session(&h).await;

    let (s, b) = call(&h.app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        b["ttlSecs"].as_u64().unwrap() >= SCAN_TTL - 5,
        "an immediate resolve keeps the scan window rather than clamping to the bind window: {b}"
    );
    let session = h.store.get_profile_session(&session_id).await.unwrap();
    assert!(
        session.token_exp >= now() + SCAN_TTL - 5,
        "session mirror agrees"
    );
}

/// The one-time property survives the new clock: a bound token is consumed atomically, and a
/// replay is refused however much deadline was left. (The bridge tests pin the same thing on the
/// happy path; this one pins it TOGETHER with the resolve-extended deadline.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_resolve_extension_does_not_weaken_one_time_use() {
    let h = harness().await;
    let (token, dog_tag_id, session_id) = start_session(&h).await;

    let (s, _b) = call(&h.app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK);

    let body = device_bind_body(&dog_tag_id, &token);
    let (s, b) = call(
        &h.app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "first bind: {b}");

    let (s, b) = call(
        &h.app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(body),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::GONE,
        "the SAME token twice is still refused: {b}"
    );

    // And the accepted bind is visible to the operator as progress, not silence: the row leaves
    // "pending" for "minting" and settles "bound" — the portal never declares a bound session dead.
    for _ in 0..200 {
        let st = status(&h, &session_id).await;
        match st["status"].as_str().unwrap() {
            "bound" => return,
            "pending" | "minting" => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await
            }
            other => panic!("unexpected terminal status {other}: {st}"),
        }
    }
    panic!("session never settled to bound");
}
