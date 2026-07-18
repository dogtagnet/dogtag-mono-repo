//! Traceability portal (govarch PR-5) — hermetic coverage over the real vet-api router.
//!
//! The oversight indexer is a `MemFeed` seeded with a `/v1/events` payload shaped exactly like the
//! live indexer's (scoped, newest-first, with `txUrl`). We prove the load-bearing properties:
//!   - **server-side scoping** — a foreign operator's event is dropped by the LOCAL scope gate even
//!     when the feed leaks it, so a vet can never fetch another vet's activity;
//!   - **the DB-record join** — each in-scope on-chain event is joined to this operator's own record;
//!   - **auth** — every `/trace/*` surface requires an operator session;
//!   - **fail-closed** — an unconfigured indexer (the default `DisabledFeed`) yields 503.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{json, Value};

use vet_api::app::AppState;
use vet_api::oversight::MemFeed;
use vet_api::store::{Record, RecordStatus};

use common::{call, mint_operator, state_with};

// This vet's own on-chain identity.
const CLONE_A: &str = "0x000000000000000000000000000000000c10e0a1"; // VACCINATION clone (configured)
const SIGNER_A: &str = "0x0000000000000000000000000000000000516ea01"; // vet's signer
const ROOT_A: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const TX_A: &str = "0xaaaa1111";
// A DIFFERENT operator (another vet) — this backend must NEVER surface its events.
const CLONE_B: &str = "0x000000000000000000000000000000000c10e0b2";
const SIGNER_B: &str = "0x0000000000000000000000000000000000516eb02";
const ROOT_B: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";

/// A minimal issued `Record` anchored at `root` on `CLONE_A` by `SIGNER_A`.
fn record(root: &str, tx: &str, dog_tag_id: &str) -> Record {
    Record {
        record_id: format!("rec-{dog_tag_id}"),
        record_type: "VACCINATION".to_string(),
        dog_tag_id: dog_tag_id.to_string(),
        wrapped_doc: json!({}),
        root: root.to_string(),
        prepared_calldata: "0x".to_string(),
        issuer_addr: CLONE_A.to_string(),
        chain_id: Some(135),
        protocol_version: Some("dogtag-levela/1".to_string()),
        verification_registry: Some("0x4E2f0996e1CB4E24F1053346f3da2186906835E8".to_string()),
        issuer_signer: Some(SIGNER_A.to_string()),
        status: RecordStatus::Issued,
        tx_hash: Some(tx.to_string()),
        confirmed_tx_hash: Some(tx.to_string()),
        signer_address: Some(SIGNER_A.to_string()),
        signing_mode: Some("backend".to_string()),
        block_number: Some(10),
        explorer_url: None,
        created_at: 1000,
        updated_at: 1000,
        label: Some("Rex annual".to_string()),
        notes: None,
        revoked_tx_hash: None,
        revoked_block_number: None,
        revoke_explorer_url: None,
        invalidated_at: None,
        invalidation_reason: None,
    }
}

/// A `/v1/events` payload shaped like the indexer's, carrying BOTH this vet's event (on `CLONE_A`) and
/// a foreign vet's event (on `CLONE_B`) — the latter simulates a leak the local gate must catch.
fn feed_events() -> Value {
    json!({
        "events": [
            { "id": "0xaaaa1111:0", "type": "rootIssued", "actor": SIGNER_A, "clone": CLONE_A,
              "root": ROOT_A, "txHash": TX_A, "blockNumber": 10, "blockTimestamp": 1000,
              "finality": "finalized", "txUrl": "https://explorer.roax.net/tx/0xaaaa1111" },
            { "id": "0xbbbb2222:0", "type": "rootIssued", "actor": SIGNER_B, "clone": CLONE_B,
              "root": ROOT_B, "txHash": "0xbbbb2222", "blockNumber": 11, "blockTimestamp": 1001,
              "finality": "finalized", "txUrl": "https://explorer.roax.net/tx/0xbbbb2222" }
        ],
        "total": 2, "limit": 100, "offset": 0,
        "scope": { "label": "Seaport Vet", "unscoped": false }
    })
}

/// Build a vet AppState whose VACCINATION clone is `CLONE_A`, with `feed` seeded to `events`.
fn vet_state(events: Value) -> AppState {
    let chain: Arc<dyn vet_api::chain::ChainClient> = Arc::new(vet_api::chain::MemChain::new());
    let mut state = state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        CLONE_A.to_string(),
        "vet.example".to_string(),
        1,
    );
    state.feed = Arc::new(MemFeed::new().with_events(events).with_stats(json!({
        "rootIssued": 1, "rootRevoked": 0, "activeCredentials": 1, "verifications": 0,
        "scope": { "label": "Seaport Vet", "unscoped": false }
    })));
    state
}

#[tokio::test]
async fn scoped_activity_joins_own_record_and_excludes_foreign_vet() {
    let state = vet_state(feed_events());
    // Seed this vet's own DB record (root ROOT_A) so the on-chain event has something to join to.
    state.store.put_record(record(ROOT_A, TX_A, "7")).await;
    let op = mint_operator(&state).await;
    let app = vet_api::router(state);

    let (s, b) = call(&app, "GET", "/trace/activity", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let events = b["events"].as_array().unwrap();
    // The foreign vet's event (CLONE_B / SIGNER_B) is dropped by the local scope gate.
    assert_eq!(events.len(), 1, "only THIS vet's event survives: {b}");
    assert_eq!(b["inScope"], 1);
    assert_eq!(
        b["droppedOutOfScope"], 1,
        "the other vet's event was rejected server-side"
    );
    for e in events {
        assert_ne!(
            e["clone"].as_str(),
            Some(CLONE_B),
            "another vet's clone must never appear"
        );
        assert_ne!(
            e["actor"].as_str(),
            Some(SIGNER_B),
            "another vet's signer must never appear"
        );
    }
    // The surviving event is JOINED to this vet's own record.
    assert_eq!(b["matched"], 1);
    let ev = &events[0];
    assert_eq!(ev["clone"].as_str(), Some(CLONE_A));
    assert_eq!(ev["local"]["recordId"], "rec-7");
    assert_eq!(ev["local"]["kind"], "issuance");
    assert_eq!(ev["local"]["label"], "Rex annual");
    // the indexer's explorer link is preserved through the join.
    assert!(ev["txUrl"]
        .as_str()
        .unwrap()
        .starts_with("https://explorer.roax.net/tx/"));
}

#[tokio::test]
async fn on_chain_event_without_local_record_is_shown_unmatched() {
    // Same feed, but this vet has NO local record for ROOT_A — the in-scope event still shows (it is
    // on our own clone) but carries `local: null`, surfacing on-chain activity with no local record.
    let state = vet_state(feed_events());
    let op = mint_operator(&state).await;
    let app = vet_api::router(state);

    let (s, b) = call(&app, "GET", "/trace/activity", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["inScope"], 1, "our clone's event is in scope");
    assert_eq!(b["matched"], 0, "but nothing joined — no local record");
    assert_eq!(b["events"][0]["local"], Value::Null);
}

#[tokio::test]
async fn a_different_vet_sees_none_of_this_feed() {
    // A vet whose configured clone is CLONE_B-adjacent but does NOT own CLONE_A/SIGNER_A. Given the
    // exact same feed events, its local scope admits neither → an empty traceability view. This is the
    // "a vet cannot fetch another vet's activity" guarantee, from the other side.
    let chain: Arc<dyn vet_api::chain::ChainClient> = Arc::new(vet_api::chain::MemChain::new());
    let mut other = state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        "0x000000000000000000000000000000000c10e0cc".to_string(), // a third, unrelated clone
        "othervet.example".to_string(),
        1,
    );
    other.feed = Arc::new(MemFeed::new().with_events(feed_events()));
    let op = mint_operator(&other).await;
    let app = vet_api::router(other);

    let (s, b) = call(&app, "GET", "/trace/activity", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["inScope"], 0, "neither event belongs to this vet: {b}");
    assert_eq!(b["droppedOutOfScope"], 2);
    assert!(b["events"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn trace_requires_operator_session() {
    let state = vet_state(feed_events());
    let app = vet_api::router(state);
    for path in ["/trace/activity", "/trace/stats"] {
        let (s, _b) = call(&app, "GET", path, None, None).await;
        assert_eq!(
            s,
            StatusCode::UNAUTHORIZED,
            "{path} must require an operator session"
        );
    }
}

#[tokio::test]
async fn unconfigured_indexer_returns_503() {
    // The default feed (from `state_with`) is `DisabledFeed`.
    let chain: Arc<dyn vet_api::chain::ChainClient> = Arc::new(vet_api::chain::MemChain::new());
    let state = state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        CLONE_A.to_string(),
        "vet.example".to_string(),
        1,
    );
    let op = mint_operator(&state).await;
    let app = vet_api::router(state);

    let (s, b) = call(&app, "GET", "/trace/activity", Some(&op), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    assert_eq!(b["indexer"], "not-configured");

    let (s, _b) = call(&app, "GET", "/trace/stats", Some(&op), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn trace_stats_adds_local_counts() {
    let state = vet_state(feed_events());
    state.store.put_record(record(ROOT_A, TX_A, "7")).await;
    let op = mint_operator(&state).await;
    let app = vet_api::router(state);

    let (s, b) = call(&app, "GET", "/trace/stats", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    // proxied scoped counters from the indexer
    assert_eq!(b["rootIssued"], 1);
    assert_eq!(b["scope"]["unscoped"], false);
    // plus this operator's own off-chain record count
    assert_eq!(b["local"]["records"], 1);
    assert_eq!(b["local"]["verifications"], 0);
}
