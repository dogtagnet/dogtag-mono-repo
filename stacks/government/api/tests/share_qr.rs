//! Record-share QR handoff: the owner walks up with a phone, scans a one-time QR the government
//! portal displays, and imports the credential.
//!
//! The contract under test is deliberately IDENTICAL to vet-api's (`stacks/vet/api/tests/flow*.rs`):
//! a 32-hex token, a `<DEPLOYMENT_URL>/r/<token>` URL with no query string, and consumption on first
//! read — so `RecordImporter` on both phones drives the government with no special-casing. The
//! government-specific part is that `/r/:id` ALSO serves the public receipt page, and the two id
//! shapes must not shadow each other.

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
const API_TOKEN: &str = "dogtag-gov-demo-token";
const DEPLOYMENT_URL: &str = "http://192.168.1.20:44832";
const FACTORY: &str = "0xed20269e3ebf0119739aab5258741f3aeb49f140";

fn demo_state() -> AppState {
    let cfg = Config {
        // A LAN URL, not localhost: the QR must name a host the OWNER'S PHONE can reach.
        deployment_url: DEPLOYMENT_URL.into(),
        rpc_url: "https://devrpc.roax.net".into(),
        chain_id: 135,
        issuer_registry_addr: REGISTRY_ADDR.into(),
        factory_addr: "0x00000000000000000000000000000000000000fa".into(),
        issuer_domain_registry_addr: "0x00000000000000000000000000000000000000dd".into(),
        dns_doh_endpoint: String::new(),
        verification_registry_addr: "0xaBFd6f6E31780EBcB7ABd28A2a9bCfc9C8e6A77B".into(),
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
    AppState {
        store: Arc::new(MemStore::new()) as Arc<dyn Store>,
        chain: Arc::new(chain),
        cfg: Arc::new(cfg),
        dns: std::sync::Arc::new(dogtag_dns_rs::BindingResolver::production(String::new())),
        feed: Arc::new(government_api::oversight::DisabledFeed),
    }
}

async fn call(
    state: &AppState,
    method: &str,
    uri: &str,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let (status, bytes) = raw(state, method, uri, token).await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Raw call — the `/r/` surface serves BOTH JSON and HTML, so some assertions need the bytes.
async fn raw(
    state: &AppState,
    method: &str,
    uri: &str,
    token: Option<&str>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let req = builder.body(Body::from("{}")).unwrap();
    let resp = government_api::router(state.clone())
        .oneshot(req)
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, bytes)
}

/// Issue a credential and return `(root, receiptId)`.
async fn issue(state: &AppState) -> (String, String) {
    let (status, v) = {
        let req = Request::builder()
            .method("POST")
            .uri("/v1/travel-clearance/issue")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {API_TOKEN}"))
            .body(Body::from(
                json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "7", "fields": {} })
                    .to_string(),
            ))
            .unwrap();
        let resp = government_api::router(state.clone())
            .oneshot(req)
            .await
            .unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice::<Value>(&bytes).unwrap())
    };
    assert_eq!(status, StatusCode::OK, "issue: {v}");
    (
        v["root"].as_str().unwrap().to_string(),
        v["receiptId"].as_str().unwrap().to_string(),
    )
}

fn token_of(qr_url: &str) -> String {
    qr_url.rsplit('/').next().unwrap().to_string()
}

#[tokio::test]
async fn share_mints_a_low_density_one_time_qr_at_the_deployment_url() {
    let state = demo_state();
    let (root, _) = issue(&state).await;

    let (status, v) = call(
        &state,
        "POST",
        &format!("/v1/records/{root}/share"),
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "share: {v}");

    let qr = v["qrUrl"].as_str().unwrap();
    // The QR names the host the PHONE can reach (DEPLOYMENT_URL), never the API's bind address.
    assert!(
        qr.starts_with(&format!("{DEPLOYMENT_URL}/r/")),
        "qrUrl must be <DEPLOYMENT_URL>/r/<token>: {qr}"
    );
    // Low-density: a short token path, NO embedded JWT and NO query string (the phone's URL parser
    // treats any `/r/<seg>` WITH a query string as the legacy JWT shape).
    assert!(!qr.contains('?'), "qrUrl must carry no query string: {qr}");
    assert!(!qr.contains("t="), "qrUrl must carry no JWT: {qr}");

    let token = token_of(qr);
    assert_eq!(token.len(), 32, "token must be 32 hex chars: {token}");
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()), "hex: {token}");
    assert_eq!(v["root"], json!(root));
    // The countdown inputs the portal renders expiry from.
    assert_eq!(v["ttlSecs"], json!(180), "TTL must match the vet stack");
    assert!(v["expiresAt"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn share_token_resolves_to_the_wrapped_doc_then_is_consumed() {
    let state = demo_state();
    let (root, _) = issue(&state).await;
    let (_, v) = call(
        &state,
        "POST",
        &format!("/v1/records/{root}/share"),
        Some(API_TOKEN),
    )
    .await;
    let token = token_of(v["qrUrl"].as_str().unwrap());

    // The phone GETs the token UNAUTHENTICATED and receives the wrapped doc itself.
    let (status, doc) = call(&state, "GET", &format!("/r/{token}"), None).await;
    assert_eq!(status, StatusCode::OK, "resolve: {doc}");
    assert_eq!(doc["signature"]["merkleRoot"], json!(root));
    assert_eq!(doc["issuer"]["recordType"], json!(TRAVEL_CLEARANCE));

    // One-time: the second read is a 404, and it is JSON — an HTML body would reach the phone as
    // "bad wrapped doc" instead of an honest "expired".
    let (status, body) = call(&state, "GET", &format!("/r/{token}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "replay must 404");
    assert_eq!(body["error"], json!("share token missing or expired"));
}

#[tokio::test]
async fn a_second_share_mints_an_independent_token_so_the_portal_can_refresh_an_expired_qr() {
    let state = demo_state();
    let (root, _) = issue(&state).await;
    let mint = |uri: String| {
        let state = state.clone();
        async move { call(&state, "POST", &uri, Some(API_TOKEN)).await }
    };
    let (_, first) = mint(format!("/v1/records/{root}/share")).await;
    let (_, second) = mint(format!("/v1/records/{root}/share")).await;
    let (t1, t2) = (
        token_of(first["qrUrl"].as_str().unwrap()),
        token_of(second["qrUrl"].as_str().unwrap()),
    );
    assert_ne!(t1, t2, "each mint must be an independent token");
    // Both resolve — re-minting refreshes the QR without invalidating an in-flight scan.
    for t in [t1, t2] {
        let (status, _) = call(&state, "GET", &format!("/r/{t}"), None).await;
        assert_eq!(status, StatusCode::OK, "token {t} must resolve");
    }
}

#[tokio::test]
async fn minting_a_share_token_requires_the_operator_bearer() {
    let state = demo_state();
    let (root, _) = issue(&state).await;
    let (status, _) = call(&state, "POST", &format!("/v1/records/{root}/share"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // ...and an unknown root is a 404, not a silently-minted token.
    let (status, _) = call(
        &state,
        "POST",
        "/v1/records/0xdeadbeef/share",
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// `/r/:id` is overloaded. A 12-char Crockford receipt id must still render the PII-free HTML status
/// page, and an unknown 32-hex token must NOT fall through to it.
#[tokio::test]
async fn the_public_r_surface_still_serves_receipt_ids_as_html() {
    let state = demo_state();
    let (_, receipt_id) = issue(&state).await;
    assert_eq!(receipt_id.len(), 12, "receipt ids stay 12 Crockford chars");

    let (status, bytes) = raw(&state, "GET", &format!("/r/{receipt_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(bytes).unwrap();
    assert!(html.starts_with("<!doctype html>"), "receipt page is HTML");
    assert!(html.contains(&receipt_id));
    // The public page stays PII-free — no Section A applicant leaf leaks onto it.
    assert!(
        !html.contains("Zagara"),
        "no applicant PII on the status page"
    );

    // An unknown share-token-shaped id answers JSON, never the receipt page's HTML 404.
    let (status, bytes) = raw(&state, "GET", &format!("/r/{}", "ab".repeat(16)), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let v: Value = serde_json::from_slice(&bytes).expect("share-token 404 must be JSON");
    assert_eq!(v["error"], json!("share token missing or expired"));
}
