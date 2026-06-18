//! Shared test helpers: build an AppState, drive the router via `oneshot`, JSON request/response.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use vet_api::app::{AppState, Config};
use vet_api::auth::JwtKeys;
use vet_api::chain::ChainClient;
use vet_api::custody::Custody;
use vet_api::prover::StubProver;
use vet_api::store::MemStore;

pub const OPERATOR_PW: &str = "op-pw";
pub const ADMIN_PW: &str = "admin-pw";

/// Build an AppState with the given chain client + issuer/registry addresses.
pub fn state_with(
    chain: Arc<dyn ChainClient>,
    rpc_url: String,
    issuer_registry_addr: String,
    vaccination_issuer_addr: String,
    issuer_domain: String,
    confirmations: u64,
) -> AppState {
    let mut issuer_addrs = HashMap::new();
    issuer_addrs.insert("VACCINATION".to_string(), vaccination_issuer_addr);
    let cfg = Config {
        deployment_url: "http://localhost:41874".to_string(),
        rpc_url,
        issuer_registry_addr,
        verification_registry_addr: "0x0000000000000000000000000000000000000000".to_string(),
        issuer_addrs,
        issuer_name: "DogTag Vet".to_string(),
        issuer_domain,
        operator_password: OPERATOR_PW.to_string(),
        admin_password: ADMIN_PW.to_string(),
        confirmations,
    };
    AppState {
        store: Arc::new(MemStore::new()),
        chain,
        prover: Arc::new(StubProver),
        custody: Custody::new(),
        jwt: JwtKeys::generate(),
        cfg: Arc::new(cfg),
    }
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

/// Full genesis -> confirm -> unlock, returning (admin_token, operator_token, backend_signer_addr).
pub async fn boot_custody(app: &axum::Router) -> (String, String, String) {
    // admin login
    let (s, b) = call(app, "POST", "/admin/login", None, Some(serde_json::json!({"password": ADMIN_PW}))).await;
    assert_eq!(s, StatusCode::OK, "admin login: {b}");
    let admin = b["token"].as_str().unwrap().to_string();

    // operator login
    let (s, b) = call(app, "POST", "/login", None, Some(serde_json::json!({"password": OPERATOR_PW}))).await;
    assert_eq!(s, StatusCode::OK, "op login: {b}");
    let operator = b["token"].as_str().unwrap().to_string();

    // genesis start
    let (s, b) = call(app, "POST", "/admin/genesis/start", Some(&admin), Some(serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::OK, "genesis start: {b}");
    let words: Vec<String> = b["words"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    let challenge: Vec<usize> = b["challengeIndices"].as_array().unwrap().iter().map(|w| w.as_u64().unwrap() as usize).collect();
    let typed: Vec<String> = challenge.iter().map(|&i| words[i].clone()).collect();

    // genesis confirm
    let (s, b) = call(
        app,
        "POST",
        "/admin/genesis/confirm",
        Some(&admin),
        Some(serde_json::json!({"words": typed, "passphrase": "seed-passphrase-123"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "genesis confirm: {b}");
    let backend_addr = b["address"].as_str().unwrap().to_string();

    // unlock
    let (s, b) = call(
        app,
        "POST",
        "/admin/unlock",
        Some(&admin),
        Some(serde_json::json!({"passphrase": "seed-passphrase-123"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "unlock: {b}");

    (admin, operator, backend_addr)
}

/// A minimal VACCINATION VC `fields` payload in the SDK typed-scalar shape.
pub fn vaccination_fields() -> Value {
    serde_json::json!({
        "credentialSubject": {
            "name": {"tag": 2, "value": "Rex"},
            "microchip": {"code": {"tag": 2, "value": "985141006580319"}}
        },
        "vaccineProductName": {"tag": 2, "value": "Rabvac 3"},
        "vaccinationDate": {"tag": 2, "value": "2026-01-11"}
    })
}
