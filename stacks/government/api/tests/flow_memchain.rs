//! End-to-end government flow over the in-memory MemChain + MemStore (no live node, no gas):
//! issue a TRAVEL_CLEARANCE credential (anchored on the emulated chain) → verify it → confirm the
//! verdict, the persisted credential, and the audit log. This is the demoable "one real E2E action".

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
        api_token: Some("dogtag-gov-demo-token".into()),
    };
    let chain = MemChain::new();
    // whitelist the demo signer for TRAVEL_CLEARANCE so the issuer-identity pillar can be exercised.
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
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = government_api::router(state.clone())
        .oneshot(req)
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

#[tokio::test]
async fn health_reports_ready() {
    let (state, _) = demo_state();
    let (status, v) = call(&state, "GET", "/health", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "ok");
    assert_eq!(v["service"], "government-api");
    assert_eq!(v["canSign"], true);
}

#[tokio::test]
async fn issue_then_verify_end_to_end() {
    let (state, _) = demo_state();

    // ISSUE — build + anchor the TRAVEL_CLEARANCE credential on the emulated chain.
    let (status, issued) = call(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({
            "record_type": TRAVEL_CLEARANCE,
            "dog_tag_id": "7",
            "fields": { "destinationCountry": "FR", "originCountry": "US" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");
    assert_eq!(issued["anchored"], true);
    assert!(issued["txHash"].is_string());
    let root = issued["root"].as_str().unwrap().to_string();
    assert!(root.starts_with("0x") && root.len() == 66);
    let wrapped = issued["wrappedDoc"].clone();

    // VERIFY — integrity (offline) + on-chain isValid (MemChain) + issuer whitelist.
    let signer = state.chain.signer_address().unwrap();
    let (status, verdict) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": wrapped, "signer_addr": signer }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "verify: {verdict}");
    assert_eq!(verdict["verdict"], true);
    assert_eq!(verdict["fragments"]["integrity"], true);
    assert_eq!(verdict["fragments"]["onchain"], true);
    assert_eq!(verdict["fragments"]["issuerWhitelisted"], true);
    assert_eq!(verdict["recomputedRoot"], root);

    // audit + records surfaces reflect the flow.
    let (_, records) = call(&state, "GET", "/v1/records", Value::Null).await;
    assert_eq!(records["records"].as_array().unwrap().len(), 1);
    let (_, audit) = call(&state, "GET", "/v1/verifications", Value::Null).await;
    assert_eq!(audit["verifications"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn verify_unanchored_root_is_invalid() {
    let (state, _) = demo_state();
    // Build (dry_run) but DON'T anchor — on-chain isValid must be false → verdict false.
    let (status, issued) = call(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "dog_tag_id": "9", "dry_run": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issued["anchored"], false);

    let (status, verdict) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": issued["wrappedDoc"].clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(verdict["fragments"]["integrity"], true);
    assert_eq!(verdict["fragments"]["onchain"], false);
    assert_eq!(verdict["verdict"], false);
}
