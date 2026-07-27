//! GATE: the deployment ROLE decides which surfaces exist.
//!
//! The groomer stack runs the SAME `vet-api` binary as the vet, distinguished only by
//! `BUSINESS_TYPE=groomer`. A groomer VERIFIES and does not ISSUE, so the issuance routes are not
//! mounted for that role at all. These tests pin BOTH directions:
//!
//!   1. groomer role -> every issuance route is absent (404, i.e. no such route)
//!   2. vet role     -> every one of those routes is still there and behaves exactly as before
//!
//! Direction (2) is the regression guard that matters most: the gate must not have taken anything
//! away from the vet, which is a live deployment.

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;
use vet_api::app::{AppState, Config};
use vet_api::auth::JwtKeys;
use vet_api::calendar::{MockCalendar, MockCentralClient};
use vet_api::chain::MemChain;
use vet_api::custody::Custody;
use vet_api::oversight::DisabledFeed;
use vet_api::store::MemStore;

/// Every route that only an ISSUER should expose, as (method, path). A groomer must 404 on all of
/// them; a vet must not. Keep this in step with the `issuance` sub-router in `routes::public_router`.
const ISSUANCE_ROUTES: &[(&str, &str)] = &[
    ("POST", "/credentials/prepare"),
    ("POST", "/credentials/confirm"),
    ("GET", "/records"),
    ("POST", "/records/some-id/revoke"),
    ("POST", "/records/some-id/share"),
    ("GET", "/records/some-id"),
    ("PATCH", "/records/some-id"),
    ("GET", "/r/sometoken"),
    ("POST", "/profiles/issue/session/start"),
    ("GET", "/profiles/issue/session/some-id"),
    ("POST", "/profiles/issue/custodial-bind"),
    ("GET", "/p/sometoken"),
];

/// Routes a groomer legitimately needs and that the role gate must leave alone.
const VERIFIER_ROUTES: &[(&str, &str)] = &[
    ("GET", "/health"),
    ("POST", "/login"),
    ("GET", "/x/sometoken"),
    ("GET", "/clients"),
    ("GET", "/appointments"),
    ("GET", "/verifications"),
    // Pets are a first-class collection for every role: a client/pet book is business data, not an
    // issuance surface. This pairing is also what keeps the groomer pet page honest — the pet's tag
    // can be LINKED and UNLINKED here while `/records/{id}/revoke` above stays absent, so a local
    // disassociation is the only tag-removing act this role has.
    ("GET", "/pets"),
    ("GET", "/pets/some-id/credentials"),
];

/// Build a Config for a given `BUSINESS_TYPE`, otherwise identical to the shared test config.
fn cfg_for_role(business_type: &str) -> Config {
    let mut issuer_addrs = std::collections::HashMap::new();
    issuer_addrs.insert(
        "VACCINATION".to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
    );
    Config {
        deployment_url: "http://localhost:43618".to_string(),
        rpc_url: "memchain".to_string(),
        issuer_registry_addr: "0x00000000000000000000000000000000000000aa".to_string(),
        issuer_addrs,
        issuer_name: "Pampered Paws".to_string(),
        issuer_domain: "groomer.local".to_string(),
        verification_registry_consent_addr: VREG_CONSENT_ADDR.to_string(),
        sbt_consent_addr: SBT_CONSENT_ADDR.to_string(),
        profile_issuer_addr: PROFILE_ISSUER_ADDR.to_string(),
        vet_signer_index: 0,
        operator_password: OPERATOR_PW.to_string(),
        admin_password: ADMIN_PW.to_string(),
        confirmations: 1,
        business_id: "biz-groomer".to_string(),
        business_type: business_type.to_string(),
        central_hmac_secret: CENTRAL_HMAC_SECRET.to_string(),
        custody_seal_path: None,
    }
}

/// Build a router for a given `BUSINESS_TYPE`, with an operator session minted.
async fn app_for_role(business_type: &str) -> (axum::Router, String) {
    let state = AppState {
        store: Arc::new(MemStore::new()),
        chain: Arc::new(MemChain::new()),
        consent_prover: Arc::new(vet_api::prover::ConsentProver::disabled()),
        calendar: Arc::new(MockCalendar::new()),
        central: Arc::new(MockCentralClient::new()),
        custody: Custody::new(),
        jwt: JwtKeys::generate(),
        cfg: Arc::new(cfg_for_role(business_type)),
        ratelimit: Arc::new(vet_api::auth::RateLimiter::new()),
        feed: Arc::new(DisabledFeed),
    };
    let op = mint_operator(&state).await;
    (vet_api::router(state), op)
}

/// Is this route ABSENT from the router?
///
/// A mounted handler can legitimately answer 404 too (an unknown record id, an expired token), so
/// the status alone cannot tell "no such route" from "no such thing". Axum's own route-miss returns
/// 404 with an EMPTY body, whereas every handler here returns `{"error": ...}` — so an empty-bodied
/// 404 is exactly "this route is not mounted".
async fn route_absent(app: &axum::Router, method: &str, path: &str, op: &str) -> bool {
    let body = if method == "GET" {
        None
    } else {
        Some(serde_json::json!({}))
    };
    let (status, json) = call(app, method, path, Some(op), body).await;
    status == StatusCode::NOT_FOUND && json.is_null()
}

#[tokio::test]
async fn groomer_role_does_not_mount_the_issuance_routes() {
    let (app, op) = app_for_role("groomer").await;
    for (method, path) in ISSUANCE_ROUTES {
        assert!(
            route_absent(&app, method, path, &op).await,
            "a groomer must not expose {method} {path} — a groomer verifies, it does not issue"
        );
    }
}

#[tokio::test]
async fn groomer_role_keeps_every_verifier_route() {
    let (app, op) = app_for_role("groomer").await;
    for (method, path) in VERIFIER_ROUTES {
        assert!(
            !route_absent(&app, method, path, &op).await,
            "the role gate must not have removed {method} {path} from the groomer"
        );
    }
}

#[tokio::test]
async fn vet_role_still_mounts_every_issuance_route() {
    let (app, op) = app_for_role("vet").await;
    for (method, path) in ISSUANCE_ROUTES {
        assert!(
            !route_absent(&app, method, path, &op).await,
            "the vet must keep {method} {path} — the role gate must take nothing away from the vet"
        );
    }
}

#[tokio::test]
async fn an_unset_or_unknown_business_type_defaults_to_the_full_issuing_surface() {
    // Fail-OPEN on the role, deliberately: a typo'd or absent BUSINESS_TYPE must never silently
    // strip a real vet deployment of its issuance routes. Only the explicit "groomer" narrows.
    for role in ["", "vet", "VET", "clinic", "kennel"] {
        let (app, op) = app_for_role(role).await;
        assert!(
            !route_absent(&app, "POST", "/credentials/prepare", &op).await,
            "BUSINESS_TYPE={role:?} must keep issuance mounted"
        );
    }
}

#[tokio::test]
async fn the_groomer_role_is_matched_case_insensitively() {
    for role in ["groomer", "Groomer", "GROOMER"] {
        let (app, op) = app_for_role(role).await;
        assert!(
            route_absent(&app, "POST", "/credentials/prepare", &op).await,
            "BUSINESS_TYPE={role:?} must be recognized as the groomer role"
        );
    }
}

#[tokio::test]
async fn issuance_enabled_reflects_the_role() {
    // the predicate itself, so the intent is pinned independently of the router wiring
    assert!(
        !cfg_for_role("groomer").issuance_enabled(),
        "a groomer does not issue"
    );
    assert!(cfg_for_role("vet").issuance_enabled(), "a vet issues");
    assert!(cfg_for_role("").issuance_enabled(), "an unset role issues");
}
