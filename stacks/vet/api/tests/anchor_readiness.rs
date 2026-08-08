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
//!    screen in place before the operator fills a form. `ready` claims configuration, not
//!    function; the ONE chain fact beside it is `mintRole` (bounded, three-state), because a fully
//!    configured stack whose signer was never granted the SBT's ISSUER_ROLE cannot mint a single
//!    tag — measured live 2026-08-07, surfacing as a silent estimation revert.
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
    assert_eq!(
        s,
        StatusCode::SERVICE_UNAVAILABLE,
        "must refuse up front: {b}"
    );

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

// ============================================================================================
// The SBT mint role on /health: held / missing / unknown, and could-not-check WARNS, never
// refuses (the gate half is pinned in custodial_issuance_bridge.rs).
// ============================================================================================

/// Custody LOCKED (nobody has unlocked since boot): the signer's address cannot be resolved, so
/// the answer is "unknown" — never either verdict — while the config half still reports ready.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_reports_mint_role_unknown_while_custody_is_locked() {
    let app = vet_api::router(issuing_state());
    let (s, b) = call(&app, "GET", "/health", None, None).await;
    assert_eq!(s, StatusCode::OK);
    let block = &b["dogTagIssuance"];
    assert_eq!(block["ready"], true, "config half unaffected: {b}");
    assert_eq!(block["mintRole"], "unknown", "{b}");
    assert!(
        block["mintRoleDetail"]
            .as_str()
            .unwrap_or_default()
            .contains("custody is locked"),
        "the unknown names its cause: {b}"
    );
}

/// Unlocked + the fake's default (a provisioned SBT): "held", no detail.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_reports_mint_role_held_once_unlocked_on_a_provisioned_sbt() {
    let app = vet_api::router(issuing_state());
    let _ = boot_custody(&app).await;
    let (s, b) = call(&app, "GET", "/health", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["dogTagIssuance"]["mintRole"], "held", "{b}");
    assert!(b["dogTagIssuance"]["mintRoleDetail"].is_null(), "{b}");
}

/// Unlocked + the role NOT held: "missing", and the detail is the operator remedy — it names the
/// signer and the ADMIN portal's "Dog-tag mint role" card, never a cast command.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_reports_mint_role_missing_with_the_admin_portal_remedy() {
    let mem = MemChain::new();
    let state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        "0x00000000000000000000000000000000000000aa".to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let (_admin, _op, signer) = boot_custody(&app).await;
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, false);

    let (s, b) = call(&app, "GET", "/health", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["dogTagIssuance"]["mintRole"], "missing", "{b}");
    let detail = b["dogTagIssuance"]["mintRoleDetail"]
        .as_str()
        .unwrap_or_default();
    assert!(
        detail.contains("Dog-tag mint role")
            && detail.contains(&signer.to_lowercase())
            && !detail.contains("cast "),
        "the remedy is a portal card naming the signer, never a command: {detail}"
    );
}

/// A FAILED role read is "unknown" with the failure named — could-not-check is not a verdict, and
/// /health must keep answering (the read is bounded, the probe never hangs the liveness check).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_reports_mint_role_unknown_when_the_read_fails() {
    let mem = MemChain::new();
    let state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        "0x00000000000000000000000000000000000000aa".to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let _ = boot_custody(&app).await;
    mem.set_sbt_role_reads_failing(true);

    let (s, b) = call(&app, "GET", "/health", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["dogTagIssuance"]["mintRole"], "unknown", "{b}");
    assert!(
        b["dogTagIssuance"]["mintRoleDetail"]
            .as_str()
            .unwrap_or_default()
            .contains("could not check"),
        "the unknown says it could not check: {b}"
    );
}
