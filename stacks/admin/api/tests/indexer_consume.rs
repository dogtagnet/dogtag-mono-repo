//! Integration coverage for the PR-B oversight-indexer consumption + signer→business directory:
//! the UNSCOPED cross-issuer activity feed, the cross-issuer counts, and the directory join - all over
//! the real Axum admin router. The indexer is a hermetic `MemFeed` seeded with responses shaped exactly
//! like the live indexer's `/v1/events` / `/v1/stats` / `/v1/issuers`; the directory is built from
//! businesses + issuer applications inserted into the store, proving the admin re-enriches every event
//! with its OWN authoritative names (never a role's PII Mongo).

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;

use admin_api::indexer::{DisabledFeed, MemFeed, OversightFeed};
use admin_api::store::{Business, IssuerApplication};

use common::{admin_token, call, hermetic_state, hermetic_state_with_feed};

const SIGNER: &str = "0x00000000000000000000000000000000516e0001";
const CLONE: &str = "0x0000000000000000000000000000000000c10e01";

fn business_row(id: &str, name: &str, domain: &str) -> Business {
    Business {
        business_id: id.into(),
        kind: "vet".into(),
        name: name.into(),
        lat: 0.0,
        lng: 0.0,
        services: vec![],
        api_base_url: "http://x".into(),
        domain: domain.into(),
        document_stores: vec![],
        hmac_key_id: "k".into(),
        hmac_secret: "s".into(),
    }
}

fn application_row(app_id: &str, entity: &str, domain: &str, addrs: &[&str]) -> IssuerApplication {
    IssuerApplication {
        application_id: app_id.into(),
        issuer_entity_id: entity.into(),
        addresses: addrs.iter().map(|s| s.to_string()).collect(),
        record_types: vec!["TRAVEL_CLEARANCE".into()],
        verify_purposes: vec!["boarding_intake".into()],
        domain: domain.into(),
        usda_nan: None,
        license: None,
        document_store: "0xstore".into(),
        status: "approved".into(),
        dns_state: "verified".into(),
        dns_checked_at: 1,
        dns_state_at_approval: "verified".into(),
        dns_proceeded_unverified: false,
        whitelist_txs: vec![],
    }
}

/// A MemFeed whose `/v1/events` payload mirrors the real indexer: a cross-issuer feed with a signer
/// (`actor`) + clone that the admin directory can name, and a `total`/`scope.unscoped` envelope.
fn seeded_feed() -> Arc<dyn OversightFeed> {
    let events = json!({
        "events": [
            { "id": "0xtx1:0", "type": "rootIssued", "actor": SIGNER, "clone": CLONE,
              "txHash": "0xtx1", "blockNumber": 10, "blockTimestamp": 1000, "finality": "finalized",
              "txUrl": "https://explorer.roax.net/tx/0xtx1" },
            { "id": "0xtx2:0", "type": "issuerCreated", "clone": CLONE, "name": "Gov Travel",
              "txHash": "0xtx2", "blockNumber": 9, "blockTimestamp": 990, "finality": "finalized" }
        ],
        "total": 2, "limit": 100, "offset": 0,
        "scope": { "label": "government-oversight", "unscoped": true }
    });
    let stats = json!({
        "totalEvents": 2, "finalized": 2, "pending": 0,
        "rootIssued": 2, "rootRevoked": 1, "activeCredentials": 1,
        "verifications": 1, "whitelisted": 1, "delisted": 0, "clones": 1, "signers": 1,
        "scope": { "label": "government-oversight", "unscoped": true }
    });
    let issuers = json!({
        "issuers": [
            { "clone": CLONE, "name": "Gov Travel", "recordType": "0xrt", "issued": 2, "revoked": 1, "active": 1 }
        ],
        "scope": { "label": "government-oversight" }
    });
    // `/v1/status`: the chain-health watermark PR-D's dashboard renders (head vs last-indexed, finality).
    let status = json!({
        "chainId": 135, "headBlock": 100, "finalizedBlock": 95, "finalitySource": "finalized-tag",
        "lastFinalizedIndexed": 95, "lag": 5, "confirmations": 12,
        "scope": { "label": "government-oversight", "unscoped": true }
    });
    Arc::new(
        MemFeed::new()
            .with_events(events)
            .with_stats(stats)
            .with_issuers(issuers)
            .with_status(status),
    ) as Arc<dyn OversightFeed>
}

/// Insert the business + issuer application so the directory can name SIGNER/CLONE.
async fn seed_directory(state: &admin_api::app::AppState) {
    state
        .store
        .put_business(business_row(
            "biz-gov",
            "DogTag Government Authority",
            "gov.example",
        ))
        .await;
    state
        .store
        .put_application(application_row(
            "app-gov",
            "biz-gov",
            "gov.example",
            &[SIGNER, CLONE],
        ))
        .await;
}

/// The unscoped activity feed is proxied AND re-enriched with the admin's authoritative signer names.
#[tokio::test]
async fn activity_feed_is_unscoped_and_named_by_admin_directory() {
    let state = hermetic_state_with_feed(seeded_feed());
    seed_directory(&state).await;
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    let (s, b) = call(&app, "GET", "/v1/admin/activity", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["total"], 2, "unscoped: every cross-issuer event");
    assert_eq!(b["scope"]["unscoped"], true);
    // the signer event is NAMED from the admin business directory (authoritative enrichment).
    let ev0 = &b["events"][0];
    assert_eq!(ev0["actor"].as_str(), Some(SIGNER));
    assert_eq!(ev0["actorName"], "DogTag Government Authority");
    assert_eq!(ev0["cloneName"], "DogTag Government Authority");
    // explorer link from the indexer is preserved.
    assert!(ev0["txUrl"]
        .as_str()
        .unwrap()
        .starts_with("https://explorer.roax.net/tx/"));
}

/// Query filters pass through to the indexer (the route accepts + forwards them without error).
#[tokio::test]
async fn activity_feed_accepts_narrowing_filters() {
    let state = hermetic_state_with_feed(seeded_feed());
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    let (s, b) = call(
        &app,
        "GET",
        "/v1/admin/activity?type=rootIssued&recordType=TRAVEL_CLEARANCE&finality=finalized&limit=50",
        Some(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(b["events"].is_array());
}

/// Cross-issuer counts are the aggregates PR-D renders (active vs revoked, verifications, …).
#[tokio::test]
async fn cross_issuer_counts_are_served() {
    let state = hermetic_state_with_feed(seeded_feed());
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    let (s, b) = call(&app, "GET", "/v1/admin/activity/stats", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["rootIssued"], 2);
    assert_eq!(b["rootRevoked"], 1);
    assert_eq!(b["activeCredentials"], 1);
    assert_eq!(b["clones"], 1);
}

/// The per-clone issuers list is served + enriched with the admin directory name.
#[tokio::test]
async fn issuers_list_is_named() {
    let state = hermetic_state_with_feed(seeded_feed());
    seed_directory(&state).await;
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    let (s, b) = call(&app, "GET", "/v1/admin/activity/issuers", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["issuers"][0]["clone"].as_str(), Some(CLONE));
    assert_eq!(b["issuers"][0]["active"], 1);
    assert_eq!(b["issuers"][0]["cloneName"], "DogTag Government Authority");
}

/// The chain-health status watermark is served unscoped (head vs last-indexed, finality source) - the
/// fuel for PR-D's chain-health card.
#[tokio::test]
async fn chain_health_status_is_served() {
    let state = hermetic_state_with_feed(seeded_feed());
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    let (s, b) = call(&app, "GET", "/v1/admin/activity/status", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["headBlock"], 100);
    assert_eq!(b["finalizedBlock"], 95);
    assert_eq!(b["lastFinalizedIndexed"], 95);
    assert_eq!(b["lag"], 5);
    assert_eq!(b["finalitySource"], "finalized-tag");
}

/// The signer→business directory resolves an on-chain signer to its business identity, and 404s on an
/// unknown address.
#[tokio::test]
async fn directory_resolves_signer_and_404s_unknown() {
    let state = hermetic_state_with_feed(seeded_feed());
    seed_directory(&state).await;
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    let (s, b) = call(
        &app,
        "GET",
        &format!("/v1/admin/directory/signer/{SIGNER}"),
        Some(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["business"], "DogTag Government Authority");
    assert_eq!(b["entity"], "biz-gov");
    assert_eq!(b["recordTypes"][0], "TRAVEL_CLEARANCE");

    // full directory listing.
    let (s, b) = call(&app, "GET", "/v1/admin/directory", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["total"], 2, "both SIGNER and CLONE addresses are indexed");

    // unknown address → 404.
    let (s, _b) = call(
        &app,
        "GET",
        "/v1/admin/directory/signer/0x00000000000000000000000000000000deadbeef",
        Some(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

/// Every PR-B surface requires an admin session.
#[tokio::test]
async fn activity_surfaces_require_admin() {
    let state = hermetic_state_with_feed(seeded_feed());
    let app = admin_api::router(state);
    for path in [
        "/v1/admin/activity",
        "/v1/admin/activity/stats",
        "/v1/admin/activity/issuers",
        "/v1/admin/directory",
        "/v1/admin/directory/signer/0xabc",
    ] {
        let (s, _b) = call(&app, "GET", path, None, None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{path} must require admin");
    }
}

/// When no indexer is configured (DisabledFeed), the activity/count surfaces fail closed with 503 -
/// but the store-derived directory still works (it needs no indexer).
#[tokio::test]
async fn unconfigured_indexer_returns_503_but_directory_still_works() {
    let (mut state, _c, _v, _b) = hermetic_state();
    state.feed = Arc::new(DisabledFeed) as Arc<dyn OversightFeed>;
    seed_directory(&state).await;
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    let (s, b) = call(&app, "GET", "/v1/admin/activity", Some(&tok), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    assert_eq!(b["indexer"], "not-configured");

    let (s, _b) = call(&app, "GET", "/v1/admin/activity/stats", Some(&tok), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);

    // the directory does NOT depend on the indexer, so it still resolves.
    let (s, b) = call(
        &app,
        "GET",
        &format!("/v1/admin/directory/signer/{SIGNER}"),
        Some(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["business"], "DogTag Government Authority");
}
