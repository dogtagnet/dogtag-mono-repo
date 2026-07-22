//! Shared test helpers: build an AppState, drive the router via `oneshot`, JSON request/response.

#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use admin_api::app::{AppState, Config};
use admin_api::auth::JwtKeys;
use admin_api::business::{BusinessClient, MockBusinessClient};
use admin_api::chain::{ChainClient, MemChain};
use admin_api::crypto::{KeyVault, MemVault};
use admin_api::dns::{DnsChecker, MockDnsChecker};
use admin_api::indexer::{MemFeed, OversightFeed};
use admin_api::store::MemStore;

pub const ADMIN_PW: &str = "admin-pw";
pub const REGISTRY: &str = "0x00000000000000000000000000000000000000a1";
pub const SBT: &str = "0x00000000000000000000000000000000000000b2";
pub const FACTORY: &str = "0x00000000000000000000000000000000000000fa";

/// Build a fully hermetic AppState (MemChain/MemStore/MemVault + mock DNS/business). Returns the state
/// plus typed handles to the chain/business/dns mocks for assertions.
pub fn hermetic_state() -> (AppState, MemChain, MemVault, MockBusinessClient) {
    let chain = MemChain::new();
    // Register the admin signer at index 0 for whitelisting and governance actions.
    chain.set_signer(0, "0x00000000000000000000000000000000000000ad");
    let vault = MemVault::new();
    let business = MockBusinessClient::new(true);
    let cfg = Config {
        deployment_url: "http://localhost:39742".to_string(),
        rpc_url: "http://localhost:0".to_string(),
        issuer_registry_addr: REGISTRY.to_string(),
        chain_id: 135,
        verification_registry_addr: "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87".to_string(),
        sbt_addr: SBT.to_string(),
        factory_addr: FACTORY.to_string(),
        admin_password_hash: admin_api::auth::hash_password(ADMIN_PW),
        admin_signer_index: 0,
    };
    let state = AppState {
        store: Arc::new(MemStore::new()),
        chain: Arc::new(chain.clone()) as Arc<dyn ChainClient>,
        dns: Arc::new(MockDnsChecker::ok()) as Arc<dyn DnsChecker>,
        business: Arc::new(business.clone()) as Arc<dyn BusinessClient>,
        vault: Arc::new(vault.clone()) as Arc<dyn KeyVault>,
        // Default hermetic feed: empty MemFeed. Tests that exercise the activity/count surfaces build
        // their own state via `hermetic_state_with_feed` with a seeded MemFeed.
        feed: Arc::new(MemFeed::new()) as Arc<dyn OversightFeed>,
        jwt: JwtKeys::generate(),
        cfg: Arc::new(cfg),
        ratelimit: Arc::new(admin_api::auth::RateLimiter::new()),
    };
    (state, chain, vault, business)
}

/// Like `hermetic_state`, but with a caller-supplied oversight feed (PR-B). Used by the
/// indexer-consumption tests to inject a `MemFeed` seeded with canned `/v1/events` / `/v1/stats` /
/// `/v1/issuers` payloads, so the activity + directory-enrichment routes run end-to-end with no live
/// indexer. Returns just the state (the chain/vault/business mocks are rarely needed there).
pub fn hermetic_state_with_feed(feed: Arc<dyn OversightFeed>) -> AppState {
    let (mut state, _chain, _vault, _business) = hermetic_state();
    state.feed = feed;
    state
}

/// Issue a request and return (status, json body).
pub async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(b) = bearer {
        req = req.header("authorization", format!("Bearer {b}"));
    }
    let req = if let Some(json) = body {
        req.header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&json).unwrap()))
            .unwrap()
    } else {
        req.body(Body::empty()).unwrap()
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

/// Issue a request with arbitrary headers (for HMAC-signed appointment-events).
pub async fn call_raw(
    app: &axum::Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::from(body.to_vec())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

/// Admin-login -> token.
pub async fn admin_token(app: &axum::Router) -> String {
    let (s, b) = call(
        app,
        "POST",
        "/v1/admin/login",
        None,
        Some(serde_json::json!({"password": ADMIN_PW})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "admin login: {b}");
    b["token"].as_str().unwrap().to_string()
}

/// Signup an owner -> (ownerId, session token, walletAddress).
pub async fn signup(app: &axum::Router, email: &str, wallet: &str) -> (String, String) {
    let (s, b) = call(
        app,
        "POST",
        "/v1/auth/signup",
        None,
        Some(serde_json::json!({
            "email": email, "password": "pw123", "walletAddress": wallet, "name": "Owner"
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "signup: {b}");
    (
        b["ownerId"].as_str().unwrap().to_string(),
        b["token"].as_str().unwrap().to_string(),
    )
}
