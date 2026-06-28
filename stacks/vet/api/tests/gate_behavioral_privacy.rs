//! PHASE-8 GATE — Behavioral-privacy (BUILD_PROMPT §8 acceptance; impl §3.9/§11.8; CHANGESPEC §5).
//!
//! On-chain verification-event linkage (`subject`+`dogTagId`+`relayer`+`ts`) is pseudonymous personal
//! data. Two mitigations are normative and tested/documented here:
//!
//!   1. ZK is the DATA-MINIMIZATION DEFAULT for sensitive purposes — `/verify/session/start` defaults
//!      `mode` to "zk" when the caller does NOT specify it (no raw credential data, no `recordType`/
//!      `credentialRoot` on chain). This test drives the REAL endpoint with mode UNSPECIFIED and
//!      asserts the persisted session mode is "zk".
//!
//!   2. Fresh-per-pet `subject` address bounds linkage to ONE pet, not the owner's whole portfolio
//!      (mobile mints each pet's SBT to a fresh derived address; the ZK `subject` IS that per-pet
//!      address). That mitigation is implemented mobile-side; it is documented in docs/DPIA.md and
//!      asserted structurally here (the session carries a per-purpose challenge, not an owner id).
//!
//! Hermetic: MemChain + MemStore, no anvil/ZK proving.

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;
use vet_api::chain::MemChain;
use vet_api::verify::verify_key;

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";

async fn booted_app() -> (axum::Router, MemChain, String, String) {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let state = state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let (_admin, op, relayer) = boot_custody(&app).await;
    (app, mem, op, relayer)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zk_is_the_default_mode_when_unspecified() {
    let (app, mem, op, relayer) = booted_app().await;

    // a "sensitive" purpose — relayer must be whitelisted for keccak256("VERIFY:"||purpose).
    let purpose = "VET_INTAKE";
    mem.whitelist(REGISTRY, &verify_key(purpose), &relayer);

    // start a session WITHOUT specifying `mode` — the server must DEFAULT to "zk".
    let (s, b) = call(
        &app,
        "POST",
        "/verify/session/start",
        Some(&op),
        Some(serde_json::json!({ "purpose": purpose, "recordType": "VACCINATION" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "session start (mode unspecified): {b}");
    let session_id = b["sessionId"].as_str().expect("sessionId").to_string();

    // The persisted export session carries mode == "zk" (the data-minimization default). The QR is a
    // low-density one-time TOKEN (no JWT); we resolve the session metadata via GET /x/<token> and read
    // its `mode`.
    let qr = b["qrUrl"].as_str().expect("qrUrl");
    assert!(
        !qr.contains("t="),
        "export QR must not carry a JWT query string: {qr}"
    );
    let token = extract_token(qr);
    let (s, meta) = call(&app, "GET", &format!("/x/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "GET /x/<token> resolve: {meta}");
    assert_eq!(
        meta["mode"].as_str(),
        Some("zk"),
        "default export mode MUST be zk (sensitive default)"
    );
    assert_eq!(
        meta["sessionId"].as_str(),
        Some(session_id.as_str()),
        "resolve binds the session"
    );

    // sanity: the session id is opaque and the request carried NO owner identifier (only a purpose),
    // so the session itself does not bind the owner's portfolio — linkage is per-pet via `subject`.
    assert!(!session_id.is_empty());
    assert!(
        !qr.contains("owner") && !qr.contains("ownerId"),
        "the export QR must not embed an owner identifier"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_normal_mode_is_still_honoured() {
    // ZK is the DEFAULT, not the only option: an explicit "normal" request is still accepted (the
    // fallback when an on-chain credentialRoot commitment is genuinely required).
    let (app, mem, op, relayer) = booted_app().await;
    let purpose = "TRAVEL_PRESENTATION";
    mem.whitelist(REGISTRY, &verify_key(purpose), &relayer);

    let (s, b) = call(
        &app,
        "POST",
        "/verify/session/start",
        Some(&op),
        Some(serde_json::json!({ "purpose": purpose, "recordType": "VACCINATION", "mode": "normal" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "explicit normal mode: {b}");
    let token = extract_token(b["qrUrl"].as_str().unwrap());
    let (s, meta) = call(&app, "GET", &format!("/x/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "GET /x/<token> resolve: {meta}");
    assert_eq!(
        meta["mode"].as_str(),
        Some("normal"),
        "explicit normal honoured"
    );
}

/// Pull the one-time export TOKEN out of the export QR URL (`.../x/<token>?a=<relayer>`).
fn extract_token(qr: &str) -> String {
    qr.rsplit('/')
        .next()
        .unwrap()
        .split('?')
        .next()
        .unwrap()
        .to_string()
}
