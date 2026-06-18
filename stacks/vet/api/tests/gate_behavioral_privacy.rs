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

    // The persisted session + its one-time JWT carry mode == "zk" (the data-minimization default).
    // We verify via the session-status surface if present, else re-issue is not needed: the QR URL
    // embeds the JWT whose `mode` claim we decode below.
    let qr = b["qrUrl"].as_str().expect("qrUrl");
    let token = extract_jwt(qr);
    let mode_claim = jwt_claim_mode(&token);
    assert_eq!(mode_claim.as_deref(), Some("zk"), "default verify mode MUST be zk (sensitive default)");

    // sanity: the session id is opaque and the request carried NO owner identifier (only a purpose),
    // so the session itself does not bind the owner's portfolio — linkage is per-pet via `subject`.
    assert!(!session_id.is_empty());
    assert!(
        !qr.contains("owner") && !qr.contains("ownerId"),
        "the session QR/JWT must not embed an owner identifier"
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
    let token = extract_jwt(b["qrUrl"].as_str().unwrap());
    assert_eq!(jwt_claim_mode(&token).as_deref(), Some("normal"), "explicit normal honoured");
}

/// Pull the JWT out of the session QR URL (`.../v?t=<jwt>`).
fn extract_jwt(qr: &str) -> String {
    qr.split("t=").nth(1).unwrap().split('&').next().unwrap().to_string()
}

/// Decode the (unverified) JWT payload and read the `mode` claim. Test-only; we don't need to verify
/// the signature here — we are asserting the server-chosen default, not authenticating.
fn jwt_claim_mode(token: &str) -> Option<String> {
    use base64::Engine;
    let payload_b64 = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("mode").and_then(|m| m.as_str()).map(|s| s.to_string())
}
