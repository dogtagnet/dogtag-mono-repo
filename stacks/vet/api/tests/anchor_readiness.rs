//! The dog-tag ANCHOR readiness gate: a backend that cannot anchor refuses where the OPERATOR can
//! act, never only on the owner's phone.
//!
//! The defect this pins (measured on a live walk, 2026-08-07): with `PROFILE_ISSUER_ADDR` unset the
//! vet portal's Register pet screen allocated a tag, drew a QR, and had a second human scan it —
//! and only then did the owner's phone see the custodial-bind 503. The refusal was correct; where
//! it surfaced was not. So:
//!
//! 1. `POST /profiles/issue/session/start` refuses 503 BEFORE allocating a tag or minting a QR
//!    token, with a message that names the missing contract in the operator's vocabulary and points
//!    at the fixing step.
//! 2. `GET /health` reports the same config facts (`dogTagIssuance`) so the portal can refuse the
//!    screen in place before the operator fills a form. Config facts only — never a chain read, so
//!    `ready` claims configuration, not function.
//! 3. `POST /credentials/prepare` names a blank/malformed issuer clone address instead of passing
//!    it into a chain read as the zero address — which used to surface as
//!    `502 preflight: rpc: ABI decoding failed: buffer overrun while deserializing`, a config hole
//!    reading as a chain fault.

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;
use vet_api::chain::MemChain;

const ZERO_ADDR: &str = "0x0000000000000000000000000000000000000000";

fn issuing_state() -> vet_api::app::AppState {
    state_with(
        Arc::new(MemChain::new()),
        "memchain".to_string(),
        "0x00000000000000000000000000000000000000aa".to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
        "vet.example".to_string(),
        1,
    )
}

fn with_cfg(
    mut state: vet_api::app::AppState,
    f: impl FnOnce(&mut vet_api::app::Config),
) -> vet_api::app::AppState {
    let mut cfg = (*state.cfg).clone();
    f(&mut cfg);
    state.cfg = Arc::new(cfg);
    state
}

fn start_body() -> serde_json::Value {
    serde_json::json!({
        "ownerIdentity": { "countryOfIdentification": "", "identification": "", "name": "" },
        "pet": { "name": "Rex" }
    })
}

/// THE GATE: an unanchorable backend refuses Register pet up front — 503, no session row persisted,
/// no dogTagId/QR in the response — and the message names the DOG_PROFILE contract and the step
/// that fixes it, not a bare variable name.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn register_pet_refuses_before_allocating_when_the_profile_issuer_is_unset() {
    let state = with_cfg(issuing_state(), |cfg| {
        cfg.profile_issuer_addr = ZERO_ADDR.to_string();
    });
    let store = state.store.clone();
    let app = vet_api::router(state);
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/session/start",
        Some(&op),
        Some(start_body()),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "must refuse up front: {b}");

    let msg = b["error"].as_str().expect("refusal carries a message");
    assert!(
        msg.contains("DOG_PROFILE issuer contract"),
        "the missing thing is named in the operator's vocabulary: {msg}"
    );
    assert!(
        msg.contains("Provider page"),
        "the refusal points at the fixing step: {msg}"
    );
    assert!(
        msg.contains("PROFILE_ISSUER_ADDR"),
        "the config detail rides along for whoever edits the env: {msg}"
    );

    // Nothing was allocated and nothing can be scanned: no session row (which is what carries the
    // allocated dogTagId), and the refusal body offers no token/QR to render.
    assert!(
        store.list_profile_sessions().await.is_empty(),
        "a refused start must not have allocated a tag"
    );
    assert!(b.get("qr").is_none() && b.get("token").is_none() && b.get("dogTagId").is_none());
}

/// The SBT half names ITS OWN remedy — the deployment ledger, not the Provider page — because the
/// two addresses are fixed by different people in different places, and pointing the operator at
/// the wrong screen is the defect this gate exists to remove.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_sbt_names_the_ledger_not_the_provider_page() {
    let state = with_cfg(issuing_state(), |cfg| {
        cfg.sbt_consent_addr = ZERO_ADDR.to_string();
    });
    let app = vet_api::router(state);
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/session/start",
        Some(&op),
        Some(start_body()),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    let msg = b["error"].as_str().unwrap();
    assert!(
        msg.contains("DogTagSBTConsent") && msg.contains("deployment ledger"),
        "the SBT remedy is the ledger: {msg}"
    );
    assert!(
        !msg.contains("DOG_PROFILE issuer contract is not set"),
        "a configured profile issuer must not be accused: {msg}"
    );
}

/// `/health` carries the same config facts, so the portal can refuse the SCREEN in place. Both
/// booleans are reported apart — the two contracts have different remedies, and one folded `ready`
/// would tell the operator something is wrong while withholding which thing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_reports_anchor_readiness_for_an_issuing_role() {
    // Fully configured: ready.
    let app = vet_api::router(issuing_state());
    let (s, b) = call(&app, "GET", "/health", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["dogTagIssuance"]["ready"], true, "{b}");
    assert_eq!(b["dogTagIssuance"]["profileIssuerConfigured"], true);
    assert_eq!(b["dogTagIssuance"]["sbtConsentConfigured"], true);

    // Profile issuer unset: not ready, and WHICH half is missing is stated.
    let state = with_cfg(issuing_state(), |cfg| {
        cfg.profile_issuer_addr = ZERO_ADDR.to_string();
    });
    let app = vet_api::router(state);
    let (s, b) = call(&app, "GET", "/health", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["dogTagIssuance"]["ready"], false, "{b}");
    assert_eq!(b["dogTagIssuance"]["profileIssuerConfigured"], false);
    assert_eq!(b["dogTagIssuance"]["sbtConsentConfigured"], true);
}

/// A groomer mounts no issuance routes, so its health carries NO `dogTagIssuance` block at all. A
/// consumer must read an ABSENT block as could-not-check, never as either verdict — pinned here so
/// the absence stays deliberate rather than becoming a groomer falsely reporting itself
/// unanchorable (or worse, ready).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_carries_no_issuance_block_for_a_groomer() {
    let state = with_cfg(issuing_state(), |cfg| {
        cfg.business_type = "groomer".to_string();
    });
    let app = vet_api::router(state);
    let (s, b) = call(&app, "GET", "/health", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        b.get("dogTagIssuance").is_none(),
        "a non-issuing role makes no issuance claim: {b}"
    );
}

/// The 502's source, closed: a BLANK configured issuer clone address (what `demo-up.sh` exports
/// when no clone is configured) is refused naming the record type and the fixing step — it must
/// never reach a chain read as the zero address, whose empty returndata used to surface as
/// `preflight: rpc: ABI decoding failed: buffer overrun while deserializing`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_blank_vaccination_issuer_address_is_named_not_surfaced_as_a_decode_error() {
    let state = with_cfg(issuing_state(), |cfg| {
        cfg.issuer_addrs
            .insert("VACCINATION".to_string(), "".to_string());
    });
    let app = vet_api::router(state);
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (s, b) = call(
        &app,
        "POST",
        "/credentials/prepare",
        Some(&op),
        Some(serde_json::json!({
            "recordType": "VACCINATION",
            "dogTagId": "7",
            "fields": vaccination_fields(),
        })),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    let msg = b["error"].as_str().unwrap();
    assert!(
        msg.contains("VACCINATION") && msg.contains("VACCINATION_ISSUER_ADDR"),
        "the unset address is named: {msg}"
    );
    assert!(
        !msg.contains("buffer overrun") && !msg.contains("ABI decoding"),
        "a config hole must not read as a chain fault: {msg}"
    );
}

/// A record type with NO configured entry stays a 400 and now says what that means, instead of the
/// old two-causes-in-one "unknown recordType / no issuer address".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_record_type_names_the_missing_issuer_configuration() {
    let app = vet_api::router(issuing_state());
    let (_admin, op, _backend) = boot_custody(&app).await;

    let (s, b) = call(
        &app,
        "POST",
        "/credentials/prepare",
        Some(&op),
        Some(serde_json::json!({
            "recordType": "BOARDING",
            "dogTagId": "7",
            "fields": vaccination_fields(),
        })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let msg = b["error"].as_str().unwrap();
    assert!(
        msg.contains("no issuer contract is configured for recordType BOARDING"),
        "{msg}"
    );
}
