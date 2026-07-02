//! Per-role persistence + CRUD over the government authority's OWN records (MemChain + MemStore).
//!
//! Proves the management layer the DB task adds around the intact issuance/revocation crypto flow:
//!   1. issue → the credential is persisted with its on-chain proof (tx hash, block number, contract
//!      address, and a ready-to-click `https://explorer.roax.net/tx/<hash>` link);
//!   2. GET /v1/records lists it back from the authority's own DB;
//!   3. PATCH updates OFF-CHAIN metadata but REJECTS any on-chain-derived field (immutability);
//!   4. revoke is a SOFT-invalidation: the credential stays listed as `revoked`, keeps its issuance
//!      proof intact, gains a revoke proof, and is still verifiable on-chain (`isValid` reads false);
//!   5. the off-chain `expired` transition is likewise non-destructive.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use government_api::app::{AppState, Config, TRAVEL_CLEARANCE};
use government_api::chain::{ChainClient, MemChain};
use government_api::store::{MemStore, Store};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

const ISSUER_ADDR: &str = "0x1111111111111111111111111111111111111111";
const REGISTRY_ADDR: &str = "0x5d86e4cf98a34ae0576f190f8d209c2943a9c79c";
const API_TOKEN: &str = "test-gov-token";

fn demo_state() -> (AppState, MemChain) {
    let cfg = Config {
        deployment_url: "http://localhost:44832".into(),
        rpc_url: "https://devrpc.roax.net".into(),
        chain_id: 135,
        issuer_registry_addr: REGISTRY_ADDR.into(),
        travel_clearance_issuer_addr: ISSUER_ADDR.into(),
        eu_health_cert_issuer_addr: "0x0000000000000000000000000000000000000000".into(),
        issuer_name: "DogTag Government Authority".into(),
        issuer_domain: "gov.example".into(),
        demo: true,
        api_token: Some(API_TOKEN.into()),
    };
    let chain = MemChain::new();
    if let Some(signer) = chain.signer_address() {
        chain.whitelist(
            REGISTRY_ADDR,
            &government_api::app::record_type_key(TRAVEL_CLEARANCE),
            &signer,
        );
    }
    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    let state = AppState {
        store,
        chain: Arc::new(chain.clone()),
        cfg: Arc::new(cfg),
    };
    (state, chain)
}

async fn call(state: &AppState, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    call_with_token(state, method, uri, body, None).await
}

/// Same as `call` but with an explicit `Authorization: Bearer <token>` (the mutation-endpoint gate).
async fn call_auth(state: &AppState, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    call_with_token(state, method, uri, body, Some(API_TOKEN)).await
}

async fn call_with_token(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let req = builder.body(Body::from(body.to_string())).unwrap();
    let resp = government_api::router(state.clone())
        .oneshot(req)
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

async fn issue_one(state: &AppState) -> String {
    let (s, b) = call(
        state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({
            "record_type": TRAVEL_CLEARANCE,
            "dog_tag_id": "7",
            "fields": { "destinationCountry": "FR" }
        }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "issue: {b}");
    assert_eq!(b["anchored"], true);
    b["root"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn issue_persists_onchain_proof_and_lists() {
    let (state, mem) = demo_state();
    let root = issue_one(&state).await;

    let (s, b) = call(&state, "GET", "/v1/records", Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    let records = b["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    let rec = &records[0];
    assert_eq!(rec["status"], "issued");
    assert_eq!(
        rec["issuerAddr"].as_str().unwrap().to_lowercase(),
        ISSUER_ADDR
    );
    let tx = rec["txHash"].as_str().expect("tx hash persisted");
    assert!(
        rec["blockNumber"].as_u64().is_some(),
        "block number persisted"
    );
    assert_eq!(
        rec["explorerUrl"].as_str().unwrap(),
        format!("https://explorer.roax.net/tx/{tx}"),
        "explorer link built as https://explorer.roax.net/tx/<hash>"
    );
    assert!(mem.is_valid(ISSUER_ADDR, &root).await.unwrap());
}

#[tokio::test]
async fn patch_updates_offchain_but_rejects_onchain_fields() {
    let (state, _mem) = demo_state();
    let root = issue_one(&state).await;
    let path = format!("/v1/records/{root}");

    let (s, b) = call_auth(
        &state,
        "PATCH",
        &path,
        json!({ "label": "case-42", "notes": "priority" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "patch: {b}");
    assert_eq!(b["label"], "case-42");
    assert_eq!(b["notes"], "priority");
    assert_eq!(b["root"], root, "on-chain root untouched");

    for (k, v) in [
        ("txHash", json!("0xdead")),
        ("blockNumber", json!(1)),
        ("issuerAddr", json!("0xother")),
        ("contractAddress", json!("0xother")),
        ("root", json!("0xother")),
        ("wrappedDoc", json!({})),
        ("explorerUrl", json!("https://evil/tx/x")),
        ("anchored", json!(false)),
    ] {
        let (s, b) = call_auth(&state, "PATCH", &path, json!({ k: v })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "on-chain '{k}' rejected: {b}");
        assert!(b["error"].as_str().unwrap_or("").contains("immutable"));
    }
}

#[tokio::test]
async fn mutations_require_the_bearer_token() {
    let (state, mem) = demo_state();
    let root = issue_one(&state).await;
    let path = format!("/v1/records/{root}");

    // missing token -> 401, wrong token -> 401 - on BOTH mutation endpoints.
    let (s, b) = call(&state, "PATCH", &path, json!({ "label": "hax" })).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "patch without token: {b}");
    let (s, b) = call_with_token(
        &state,
        "PATCH",
        &path,
        json!({ "label": "hax" }),
        Some("wrong"),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "patch with wrong token: {b}");
    let (s, b) = call(&state, "POST", &format!("{path}/revoke"), Value::Null).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "revoke without token: {b}");
    let (s, b) = call_with_token(
        &state,
        "POST",
        &format!("{path}/revoke"),
        Value::Null,
        Some("wrong"),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "revoke with wrong token: {b}");

    // the stored record is unchanged: still issued, no label, anchor still valid on-chain.
    let (_s, b) = call(&state, "GET", &path, Value::Null).await;
    assert_eq!(b["status"], "issued");
    assert!(b["label"].is_null(), "rejected patch must not write: {b}");
    assert!(
        b["revokedTxHash"].is_null(),
        "rejected revoke must not write: {b}"
    );
    assert!(mem.is_valid(ISSUER_ADDR, &root).await.unwrap());

    // reads stay open (no token needed).
    let (s, _b) = call(&state, "GET", "/v1/records", Value::Null).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn revoke_is_soft_invalidation_keeping_history_and_proof() {
    let (state, mem) = demo_state();
    let root = issue_one(&state).await;
    assert!(mem.is_valid(ISSUER_ADDR, &root).await.unwrap());

    let (s, b) = call_auth(
        &state,
        "POST",
        &format!("/v1/records/{root}/revoke"),
        json!({ "reason": "credential withdrawn" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "revoke: {b}");
    assert_eq!(b["status"], "revoked");

    // on-chain: isValid flips false, historical anchor intact.
    assert!(!mem.is_valid(ISSUER_ADDR, &root).await.unwrap());
    assert!(!mem.issued_at(ISSUER_ADDR, &root).await.unwrap().is_zero());

    // record retained + still shows the issuance AND revoke proofs.
    let (_s, b) = call(&state, "GET", "/v1/records", Value::Null).await;
    let records = b["records"].as_array().unwrap();
    assert_eq!(records.len(), 1, "revoked record retained (never deleted)");
    let rec = &records[0];
    assert_eq!(rec["status"], "revoked");
    let orig_tx = rec["txHash"].as_str().unwrap();
    assert_eq!(
        rec["explorerUrl"].as_str().unwrap(),
        format!("https://explorer.roax.net/tx/{orig_tx}")
    );
    let revoke_tx = rec["revokedTxHash"].as_str().unwrap();
    assert_eq!(
        rec["revokeExplorerUrl"].as_str().unwrap(),
        format!("https://explorer.roax.net/tx/{revoke_tx}")
    );
    assert_eq!(rec["invalidationReason"], "credential withdrawn");
    assert!(rec["invalidatedAt"].as_u64().is_some());

    // double-revoke -> 409.
    let (s, _b) = call_auth(
        &state,
        "POST",
        &format!("/v1/records/{root}/revoke"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // revoked is terminal: expiring a revoked credential -> 409, status stays `revoked`.
    let (s, b) = call_auth(
        &state,
        "PATCH",
        &format!("/v1/records/{root}"),
        json!({ "status": "expired" }),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "expire-after-revoke: {b}");
    let (_s, b) = call(&state, "GET", &format!("/v1/records/{root}"), Value::Null).await;
    assert_eq!(
        b["status"], "revoked",
        "revoked never downgraded to expired"
    );
}

#[tokio::test]
async fn expire_is_offchain_soft_state_that_keeps_the_record() {
    let (state, mem) = demo_state();
    let root = issue_one(&state).await;

    let (s, b) = call_auth(
        &state,
        "PATCH",
        &format!("/v1/records/{root}"),
        json!({ "status": "expired", "reason": "validUntil lapsed" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "expire: {b}");
    assert_eq!(b["status"], "expired");

    // off-chain expiry doesn't touch the anchor; record retained + verifiable.
    assert!(mem.is_valid(ISSUER_ADDR, &root).await.unwrap());
    let (_s, b) = call(&state, "GET", "/v1/records", Value::Null).await;
    assert_eq!(b["records"][0]["status"], "expired");

    // expired -> revoked with an empty body keeps the recorded expiry reason.
    let (s, b) = call_auth(
        &state,
        "POST",
        &format!("/v1/records/{root}/revoke"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "revoke after expire: {b}");
    assert_eq!(b["status"], "revoked");
    assert_eq!(
        b["invalidationReason"], "validUntil lapsed",
        "revoke without a reason preserves the prior expiry reason"
    );
    assert!(!mem.is_valid(ISSUER_ADDR, &root).await.unwrap());
}
