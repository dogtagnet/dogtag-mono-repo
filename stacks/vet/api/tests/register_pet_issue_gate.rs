//! The two-layer ISSUE gate on Register pet: asked BEFORE a QR exists.
//!
//! What went wrong (measured live 2026-08-07, same screen and same class as the anchor gate): the
//! portal drew a QR and a second human scanned, built and POSTed for an issuance the backend was
//! always going to refuse — BOTH on-chain permissions were absent (the registrar's address-keyed
//! ISSUE grant, and the clone's own `issuanceAllowed` for the backend signer), and the refusal
//! surfaced as a 403 on the owner's phone, where nobody can act on it.
//!
//! `DogTagIssuer.issue` requires BOTH layers (`onlyIssuanceCapable` = the authority's `canIssue`
//! AND the clone's own list), the backend knows its issuer address and its signer, so
//! `/profiles/issue/session/start` now asks both halves and refuses IN PLACE — naming which half
//! is missing and which portal fixes it — with NOTHING allocated and no QR drawn.
//!
//! The one rule that cuts the other way: could-not-check is NOT a refusal. An unreadable chain (or
//! a generation-1 clone, which has neither getter) proceeds and the response carries a
//! `signerIssuance` warning instead.

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;

use vet_api::chain::MemChain;

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";
/// The generation-2 authority the profile clone answers as its own `registry()`.
const PROFILE_AUTHORITY: &str = "0x00000000000000000000000000000000000000a2";

fn start_body() -> serde_json::Value {
    serde_json::json!({
        "ownerIdentity": { "countryOfIdentification": "", "identification": "", "name": "" },
        "pet": { "name": "Rex" }
    })
}

/// Boot a router over a shared MemChain + store handle so a test can seed the two layers for the
/// custody signer boot_custody just created.
async fn harness(mem: &MemChain) -> (axum::Router, Arc<dyn vet_api::store::Store>, String, String) {
    let state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let store = state.store.clone();
    let app = vet_api::router(state);
    let (_admin, op, backend) = boot_custody(&app).await;
    (app, store, op, backend)
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

/// The captain's exact state: rightsOf carried no ISSUE bit AND the clone had not admitted the
/// signer. The refusal must name BOTH halves, each with its fixing portal, and allocate nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_layers_missing_refuses_in_place_naming_both_halves_and_allocates_nothing() {
    let mem = MemChain::new();
    let (app, store, op, backend) = harness(&mem).await;
    // The authority is known and answers a definite NO (no capability seeded); the clone's own
    // list is known and has a definite NO for our signer.
    mem.set_governing_registry(PROFILE_ISSUER_ADDR, PROFILE_AUTHORITY);
    mem.set_issuance_allowed(PROFILE_ISSUER_ADDR, &backend, false);

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    let msg = b["error"].as_str().unwrap();
    assert!(msg.contains(&backend), "names the signing key: {msg}");
    assert!(
        msg.contains("ISSUE right") && msg.contains("Providers page"),
        "names the registrar half and its portal: {msg}"
    );
    assert!(
        msg.contains("has not admitted it") && msg.contains("Signing keys"),
        "names the clone half and its portal: {msg}"
    );
    assert!(
        msg.contains("Nothing was allocated"),
        "says no tag or QR exists: {msg}"
    );
    assert!(
        store.list_profile_sessions().await.is_empty(),
        "no session row was created"
    );
}

/// Only the registrar grant missing: the message points at DogTag's admin portal and NOT at this
/// portal's Signing keys page — sending the operator to a page that cannot fix it is its own harm.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_registrar_grant_names_the_admin_portal_only() {
    let mem = MemChain::new();
    let (app, _store, op, backend) = harness(&mem).await;
    mem.set_governing_registry(PROFILE_ISSUER_ADDR, PROFILE_AUTHORITY);
    mem.set_issuance_allowed(PROFILE_ISSUER_ADDR, &backend, true);

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    let msg = b["error"].as_str().unwrap();
    assert!(msg.contains("ISSUE right"), "{msg}");
    assert!(!msg.contains("Signing keys"), "the admitted half is not accused: {msg}");
}

/// Only the clone's own list missing: the message points at THIS portal's Signing keys page and
/// not at the registrar.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_clone_admission_names_the_signing_keys_page_only() {
    let mem = MemChain::new();
    let (app, _store, op, backend) = harness(&mem).await;
    mem.set_governing_registry(PROFILE_ISSUER_ADDR, PROFILE_AUTHORITY);
    mem.set_issuance_capability(PROFILE_AUTHORITY, PROFILE_ISSUER_ADDR, &backend, true);
    mem.set_issuance_allowed(PROFILE_ISSUER_ADDR, &backend, false);

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    let msg = b["error"].as_str().unwrap();
    assert!(msg.contains("Signing keys"), "{msg}");
    assert!(!msg.contains("ISSUE right"), "the granted half is not accused: {msg}");
}

/// Both halves read true: the start proceeds and says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_layers_held_starts_and_reports_authorized() {
    let mem = MemChain::new();
    let (app, _store, op, backend) = harness(&mem).await;
    mem.set_governing_registry(PROFILE_ISSUER_ADDR, PROFILE_AUTHORITY);
    mem.set_issuance_capability(PROFILE_AUTHORITY, PROFILE_ISSUER_ADDR, &backend, true);
    mem.set_issuance_allowed(PROFILE_ISSUER_ADDR, &backend, true);

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["signerIssuance"]["state"], "authorized", "{b}");
    assert!(b["token"].as_str().is_some());
}

/// COULD-NOT-CHECK IS NOT A REFUSAL. An unseeded chain (no governing registry, no list — the same
/// shape as a generation-1 clone or an unreachable RPC) must WARN and still mint the QR: refusing
/// on a read that never happened would shut a healthy clinic out over an RPC blip.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undetermined_warns_and_never_refuses() {
    let mem = MemChain::new();
    let (app, _store, op, _backend) = harness(&mem).await;

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::OK, "an unknown answer must not refuse: {b}");
    assert_eq!(b["signerIssuance"]["state"], "unknown", "{b}");
    let detail = b["signerIssuance"]["detail"].as_str().unwrap();
    assert!(
        detail.contains("could not be checked"),
        "the warning states inability, not a verdict: {detail}"
    );
    assert!(
        detail.contains("Providers") && detail.contains("Signing keys"),
        "the warning still names where a later failure gets fixed: {detail}"
    );
    assert!(b["token"].as_str().is_some(), "the QR is still minted");
}

/// One half definite-NO while the other is UNREADABLE still refuses — a definite refusal on either
/// layer dooms the write regardless of what the other would have said — and accuses only the half
/// that actually answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_definite_no_on_one_layer_refuses_even_when_the_other_is_unreadable() {
    let mem = MemChain::new();
    let (app, _store, op, backend) = harness(&mem).await;
    // Clone list: definite NO. Registry: never taught -> the capability read cannot resolve.
    mem.set_issuance_allowed(PROFILE_ISSUER_ADDR, &backend, false);

    let (s, b) = start(&app, &op).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    let msg = b["error"].as_str().unwrap();
    assert!(msg.contains("Signing keys"), "{msg}");
    assert!(
        !msg.contains("ISSUE right"),
        "the unreadable half is not accused: {msg}"
    );
}
