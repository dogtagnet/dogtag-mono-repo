//! Government oversight console (govarch PR-5) — hermetic coverage over the real government-api router.
//!
//! The oversight indexer is a `MemFeed` seeded with a `/v1/events` payload shaped like the live
//! indexer's UNSCOPED feed (multiple issuers). We prove:
//!   - **unscoped** — every issuer's event is served (the government sees all), not just its own;
//!   - **the DB-record join** — the government's OWN issuance is highlighted with its record summary
//!     (non-PII: no importer/subject block), while other issuers' events carry `local: null`;
//!   - **auth** — the oversight surfaces require the government API token;
//!   - **fail-closed** — an unconfigured indexer (`DisabledFeed`) yields 503.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use government_api::app::{AppState, Config, TRAVEL_CLEARANCE};
use government_api::chain::{ChainClient, MemChain};
use government_api::oversight::{DisabledFeed, MemFeed, OversightFeed};
use government_api::store::{MemStore, Store};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

const ISSUER_ADDR: &str = "0x1111111111111111111111111111111111111111"; // gov TRAVEL_CLEARANCE clone
const REGISTRY_ADDR: &str = "0x5d86e4cf98a34ae0576f190f8d209c2943a9c79c";
const API_TOKEN: &str = "test-gov-token";
const OTHER_CLONE: &str = "0x2222222222222222222222222222222222222222"; // a DIFFERENT issuer's clone
const OTHER_SIGNER: &str = "0x00000000000000000000000000000000516e0002";

fn gov_state(feed: Arc<dyn OversightFeed>) -> (AppState, MemChain) {
    let cfg = Config {
        deployment_url: "http://localhost:44832".into(),
        rpc_url: "https://devrpc.roax.net".into(),
        chain_id: 135,
        issuer_registry_addr: REGISTRY_ADDR.into(),
        issuer_domain_registry_addr: "0x00000000000000000000000000000000000000dd".into(),
        dns_doh_endpoint: String::new(),
        verification_registry_addr: "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87".into(),
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
        dns: std::sync::Arc::new(dogtag_dns_rs::BindingResolver::production(String::new())),
        feed,
    };
    (state, chain)
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

/// Issue a real TRAVEL_CLEARANCE credential and return its anchored root (persisted in the gov store).
async fn issue_one(state: &AppState) -> String {
    let (s, b) = call_with_token(
        state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "7", "fields": { "animalName": "Rex" } }),
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "issue: {b}");
    assert_eq!(b["anchored"], true, "issue: {b}");
    b["root"].as_str().unwrap().to_string()
}

/// A `/v1/events` payload with the government's own issuance (on `own_root`/`own_clone`/`own_signer`)
/// plus a DIFFERENT issuer's issuance — the unscoped feed sees both.
fn feed_events(own_root: &str, own_clone: &str, own_signer: &str) -> Value {
    json!({
        "events": [
            { "id": "0xgov:0", "type": "rootIssued", "actor": own_signer, "clone": own_clone,
              "root": own_root, "txHash": "0xgovtx", "blockNumber": 20, "blockTimestamp": 2000,
              "finality": "finalized", "txUrl": "https://explorer.roax.net/tx/0xgovtx" },
            { "id": "0xother:0", "type": "rootIssued", "actor": OTHER_SIGNER, "clone": OTHER_CLONE,
              "root": "0x9999999999999999999999999999999999999999999999999999999999999999",
              "txHash": "0xothertx", "blockNumber": 21, "blockTimestamp": 2001, "finality": "finalized" }
        ],
        "total": 2, "limit": 100, "offset": 0,
        "scope": { "label": "government-oversight", "unscoped": true }
    })
}

#[tokio::test]
async fn oversight_activity_unscoped_joins_own_credential() {
    // Build state (feed set later, once we know the anchored root).
    let (mut state, chain) = gov_state(Arc::new(DisabledFeed));
    let own_signer = chain.signer_address().unwrap();
    let root = issue_one(&state).await;
    state.feed = Arc::new(MemFeed::new().with_events(feed_events(&root, ISSUER_ADDR, &own_signer)));

    let (s, b) = call_with_token(
        &state,
        "GET",
        "/v1/oversight/activity",
        Value::Null,
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let events = b["events"].as_array().unwrap();
    // UNSCOPED: both the government's own event AND the other issuer's event are served.
    assert_eq!(events.len(), 2, "unscoped oversight sees every issuer: {b}");
    assert_eq!(
        b["matched"], 1,
        "only the government's own issuance joins a local record"
    );

    // The government's own event carries its non-PII record summary...
    let own = events
        .iter()
        .find(|e| e["clone"] == json!(ISSUER_ADDR))
        .unwrap();
    assert_eq!(own["local"]["kind"], "issuance");
    assert_eq!(own["local"]["recordType"], "TRAVEL_CLEARANCE");
    assert_eq!(own["local"]["dogTagId"], "7");
    // ...and NEVER leaks the obfuscatable importer/subject PII into the oversight feed.
    assert!(
        own["local"].get("subject").is_none(),
        "oversight join must be non-PII"
    );
    assert!(own["local"].get("importer").is_none());

    // The other issuer's event is shown (unscoped) but has no local join.
    let other = events
        .iter()
        .find(|e| e["clone"] == json!(OTHER_CLONE))
        .unwrap();
    assert_eq!(other["local"], Value::Null);
}

#[tokio::test]
async fn verified_event_joins_credentialed_tag_by_onchain_dog_tag_id() {
    // Travel-clearance oversight: an owner-blind `Verified(dogTagId, relayer, purpose, nullifier,
    // deadline, ts)` event carries NO root and NO subject - only the FIELD-HASHED dogTagId. It must
    // still join the authority's own issued credential for that tag ("a tag we credentialed was
    // verified"), and a raw-handle key must never match (the transform is load-bearing).
    let (mut state, _chain) = gov_state(Arc::new(DisabledFeed));
    issue_one(&state).await; // dog_tag_id "7", receiptId committed + stored

    let onchain_id = government_api::trace::onchain_dog_tag_id_decimal("7").unwrap();
    assert_ne!(onchain_id, "7", "the event id is the field-hash, not the handle");
    state.feed = Arc::new(MemFeed::new().with_events(json!({
        "events": [
            // The tag the authority credentialed, verified by SOME relayer (another business).
            { "id": "0xv:0", "type": "verified", "actor": OTHER_SIGNER,
              "dogTagId": onchain_id, "purpose": "0x70757270", "nullifier": "0x6e756c6c",
              "deadline": 4102444800u64, "blockNumber": 30, "blockTimestamp": 3000,
              "finality": "finalized", "txHash": "0xvtx",
              "txUrl": "https://explorer.roax.net/tx/0xvtx" },
            // A verification of some OTHER tag (its field-hash matches nothing we issued).
            { "id": "0xw:0", "type": "verified", "actor": OTHER_SIGNER,
              "dogTagId": "999999999", "purpose": "0x70757270", "nullifier": "0x6e756c6d",
              "deadline": 4102444800u64, "blockNumber": 31, "blockTimestamp": 3001,
              "finality": "finalized", "txHash": "0xwtx" }
        ],
        "total": 2, "limit": 100, "offset": 0,
        "scope": { "label": "government-oversight", "unscoped": true }
    })));

    let (s, b) = call_with_token(
        &state,
        "GET",
        "/v1/oversight/activity",
        Value::Null,
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let events = b["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(b["matched"], 1, "only the credentialed tag's verification joins");

    let own = events.iter().find(|e| e["id"] == "0xv:0").unwrap();
    assert_eq!(own["local"]["kind"], "issuance");
    assert_eq!(own["local"]["recordType"], "TRAVEL_CLEARANCE");
    assert_eq!(own["local"]["dogTagId"], "7");
    assert!(
        own["local"]["receiptId"].as_str().is_some(),
        "the receipt handle names the credential: {b}"
    );
    assert_eq!(
        own["local"]["joinedBy"], "dogTagId",
        "stamped tag-granular - the event proves the TAG was verified, not which credential"
    );
    // Subject-less always: the join must never leak the Section-A importer block.
    assert!(own["local"].get("subject").is_none());
    assert!(own["local"].get("importer").is_none());

    // The unrelated tag's verification is served (unscoped) but joins nothing.
    let other = events.iter().find(|e| e["id"] == "0xw:0").unwrap();
    assert_eq!(other["local"], Value::Null);
}

#[tokio::test]
async fn oversight_requires_api_token() {
    let (state, _chain) = gov_state(Arc::new(
        MemFeed::new().with_events(json!({ "events": [] })),
    ));
    for path in [
        "/v1/oversight/activity",
        "/v1/oversight/stats",
        "/v1/oversight/issuers",
    ] {
        let (s, _b) = call_with_token(&state, "GET", path, Value::Null, None).await;
        assert_eq!(
            s,
            StatusCode::UNAUTHORIZED,
            "{path} must require the government API token"
        );
    }
}

#[tokio::test]
async fn unconfigured_indexer_returns_503() {
    let (state, _chain) = gov_state(Arc::new(DisabledFeed));
    let (s, b) = call_with_token(
        &state,
        "GET",
        "/v1/oversight/activity",
        Value::Null,
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    assert_eq!(b["indexer"], "not-configured");
}

#[tokio::test]
async fn oversight_stats_adds_local_counts() {
    let (mut state, _chain) = gov_state(Arc::new(DisabledFeed));
    issue_one(&state).await;
    state.feed = Arc::new(MemFeed::new().with_stats(json!({
        "rootIssued": 5, "clones": 3, "scope": { "unscoped": true }
    })));

    let (s, b) = call_with_token(
        &state,
        "GET",
        "/v1/oversight/stats",
        Value::Null,
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["rootIssued"], 5, "cross-issuer count proxied");
    assert_eq!(
        b["local"]["credentials"], 1,
        "plus the government's own record count"
    );
}

#[tokio::test]
async fn oversight_issuers_served() {
    let issuers = json!({
        "issuers": [ { "clone": ISSUER_ADDR, "issued": 2, "revoked": 0, "active": 2 } ],
        "scope": { "label": "government-oversight" }
    });
    let (state, _chain) = gov_state(Arc::new(MemFeed::new().with_issuers(issuers)));
    let (s, b) = call_with_token(
        &state,
        "GET",
        "/v1/oversight/issuers",
        Value::Null,
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["issuers"][0]["clone"], ISSUER_ADDR);
    assert_eq!(b["issuers"][0]["active"], 2);
}
