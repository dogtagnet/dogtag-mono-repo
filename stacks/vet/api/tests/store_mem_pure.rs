//! Pure-logic unit coverage for `store.rs`'s `MemStore` — the in-memory `Store` impl that every
//! vet-api E2E test runs against. The security-sensitive contracts here (one-time tokens, jti/
//! idempotency single-use, expiry-on-read, monotonic dogTagId, the appointment-window query) were
//! exercised only indirectly through HTTP handlers; this file pins them directly.
//!
//! Behavior-preserving: every assertion captures the existing contract. Expiry is made deterministic
//! by passing a far-future `exp` for the live case and a past `exp` for the expired case, so the
//! `now > exp` comparison against the wall clock is stable regardless of when the test runs.

use std::time::{SystemTime, UNIX_EPOCH};

use vet_api::store::{ApptReplica, GcalEventMap, GcalSyncState, MemStore, Store};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// `exp` far enough in the future that `now > exp` is false for the whole test run.
fn future() -> u64 {
    now_secs() + 3600
}

/// `exp` in the past so `now > exp` is true (token treated as expired-on-read).
fn past() -> u64 {
    now_secs().saturating_sub(10)
}

fn appt(id: &str, updated_at: u64) -> ApptReplica {
    ApptReplica {
        appointment_id: id.to_string(),
        business_id: "biz-1".to_string(),
        dog_tag_id: "dog-1".to_string(),
        slot: "2026-01-01T10:00:00Z".to_string(),
        rev: 1,
        state: "CONFIRMED".to_string(),
        updated_at,
    }
}

// --------------------------------------------------------------------------------------------
// jti — one-time consume (replay protection for share/verify JWTs).
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn consume_jti_is_single_use_and_per_jti() {
    let s = MemStore::new();
    // first consume of a jti succeeds, the second is rejected as already-used.
    assert!(s.consume_jti("jti-a").await);
    assert!(!s.consume_jti("jti-a").await);
    // a different jti is independent.
    assert!(s.consume_jti("jti-b").await);
    assert!(!s.consume_jti("jti-b").await);
}

// --------------------------------------------------------------------------------------------
// share tokens — one-time consume + expiry-on-read.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn share_token_take_consumes_once_then_gone() {
    let s = MemStore::new();
    s.put_share_token("tok", "rec-7", future()).await;
    assert_eq!(s.take_share_token("tok").await, Some("rec-7".to_string()));
    // one-time: a second take finds nothing.
    assert_eq!(s.take_share_token("tok").await, None);
}

#[tokio::test]
async fn share_token_missing_and_expired_are_none() {
    let s = MemStore::new();
    assert_eq!(s.take_share_token("absent").await, None);
    s.put_share_token("old", "rec-7", past()).await;
    assert_eq!(s.take_share_token("old").await, None);
}

// --------------------------------------------------------------------------------------------
// export tokens — peek is NON-consuming, take consumes once; both honor expiry.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn export_token_peek_does_not_consume_take_does() {
    let s = MemStore::new();
    s.put_export_token("x", "sess-9", future()).await;
    // peek twice: still resolvable both times (not consumed).
    assert_eq!(s.peek_export_token("x").await, Some("sess-9".to_string()));
    assert_eq!(s.peek_export_token("x").await, Some("sess-9".to_string()));
    // take consumes; a second take is empty.
    assert_eq!(s.take_export_token("x").await, Some("sess-9".to_string()));
    assert_eq!(s.take_export_token("x").await, None);
}

#[tokio::test]
async fn export_token_expired_peek_and_take_are_none() {
    let s = MemStore::new();
    s.put_export_token("x", "sess-9", past()).await;
    assert_eq!(s.peek_export_token("x").await, None);
    assert_eq!(s.take_export_token("x").await, None);
}

// --------------------------------------------------------------------------------------------
// bind tokens — same peek/take split as export tokens.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn bind_token_peek_non_consuming_take_one_time_and_expiry() {
    let s = MemStore::new();
    s.put_bind_token("b", "sess-2", future()).await;
    assert_eq!(s.peek_bind_token("b").await, Some("sess-2".to_string()));
    // still present after peek; take consumes it.
    assert_eq!(s.take_bind_token("b").await, Some("sess-2".to_string()));
    assert_eq!(s.take_bind_token("b").await, None);
    // expired bind token never resolves.
    s.put_bind_token("c", "sess-3", past()).await;
    assert_eq!(s.peek_bind_token("c").await, None);
    assert_eq!(s.take_bind_token("c").await, None);
}

// --------------------------------------------------------------------------------------------
// dogTagId counter — monotonic, starts at 1, never a hash.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn next_dog_tag_id_is_monotonic_from_one() {
    let s = MemStore::new();
    assert_eq!(s.next_dog_tag_id().await, 1);
    assert_eq!(s.next_dog_tag_id().await, 2);
    assert_eq!(s.next_dog_tag_id().await, 3);
}

// --------------------------------------------------------------------------------------------
// Idempotency-Key dedupe — first true (proceed), replays false.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn record_idempotency_key_dedupes() {
    let s = MemStore::new();
    assert!(s.record_idempotency_key("k1").await);
    assert!(!s.record_idempotency_key("k1").await);
    assert!(s.record_idempotency_key("k2").await);
}

// --------------------------------------------------------------------------------------------
// operator sessions — membership set.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn op_session_membership() {
    let s = MemStore::new();
    assert!(!s.has_op_session("op_tok").await);
    s.put_op_session("op_tok".to_string()).await;
    assert!(s.has_op_session("op_tok").await);
    assert!(!s.has_op_session("other").await);
}

// --------------------------------------------------------------------------------------------
// imported client cache — upsert overwrites by dogTagId.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn client_cache_upsert_overwrites() {
    let s = MemStore::new();
    assert_eq!(s.get_client_cache("dog-1").await, None);
    s.upsert_client_cache("dog-1".to_string(), serde_json::json!({"v": 1}))
        .await;
    assert_eq!(
        s.get_client_cache("dog-1").await,
        Some(serde_json::json!({"v": 1}))
    );
    // upsert replaces the prior value for the same key.
    s.upsert_client_cache("dog-1".to_string(), serde_json::json!({"v": 2}))
        .await;
    assert_eq!(
        s.get_client_cache("dog-1").await,
        Some(serde_json::json!({"v": 2}))
    );
}

// --------------------------------------------------------------------------------------------
// issuer settings — default before any put, then round-trips.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn settings_default_then_roundtrip() {
    let s = MemStore::new();
    // unset settings yield the IssuerSettings::default() (backend signing).
    assert_eq!(s.get_settings().await.signing_mode, "backend");
    let mut new = s.get_settings().await;
    new.signing_mode = "wallet".to_string();
    s.put_settings(new).await;
    assert_eq!(s.get_settings().await.signing_mode, "wallet");
}

// --------------------------------------------------------------------------------------------
// appointment replica — put keys by appointment_id (overwrite), updated-since is an inclusive
// window sorted ascending by updated_at.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn appt_put_get_overwrites_by_id() {
    let s = MemStore::new();
    assert!(s.get_appt("a1").await.is_none());
    s.put_appt(appt("a1", 100)).await;
    assert_eq!(s.get_appt("a1").await.unwrap().updated_at, 100);
    // same id replaces.
    s.put_appt(appt("a1", 200)).await;
    assert_eq!(s.get_appt("a1").await.unwrap().updated_at, 200);
}

#[tokio::test]
async fn appts_updated_since_is_inclusive_and_sorted() {
    let s = MemStore::new();
    s.put_appt(appt("a1", 100)).await;
    s.put_appt(appt("a2", 200)).await;
    s.put_appt(appt("a3", 300)).await;
    // `>= since` is inclusive at the boundary, and results are ascending by updated_at.
    let got: Vec<u64> = s
        .appts_updated_since(200)
        .await
        .iter()
        .map(|a| a.updated_at)
        .collect();
    assert_eq!(got, vec![200, 300]);
    // a since past every row yields nothing.
    assert!(s.appts_updated_since(1000).await.is_empty());
}

// --------------------------------------------------------------------------------------------
// gcal mapping table — lookups by appt and by event, list, delete-one, and full wipe.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn gcal_map_lookup_delete_and_wipe() {
    let s = MemStore::new();
    let m = GcalEventMap {
        appointment_id: "a1".to_string(),
        google_event_id: "g1".to_string(),
        etag: "etag-1".to_string(),
        rev: 1,
        direction: "out".to_string(),
    };
    s.put_gcal_map(m).await;
    assert_eq!(
        s.get_gcal_map_by_appt("a1").await.unwrap().google_event_id,
        "g1"
    );
    assert_eq!(
        s.get_gcal_map_by_event("g1").await.unwrap().appointment_id,
        "a1"
    );
    assert_eq!(s.all_gcal_maps().await.len(), 1);
    // delete by event removes the single row.
    s.delete_gcal_map_by_event("g1").await;
    assert!(s.get_gcal_map_by_event("g1").await.is_none());
    // wipe clears everything (HTTP-410 full resync path).
    s.put_gcal_map(GcalEventMap {
        google_event_id: "g2".to_string(),
        ..Default::default()
    })
    .await;
    s.wipe_gcal_mirror().await;
    assert!(s.all_gcal_maps().await.is_empty());
}

// --------------------------------------------------------------------------------------------
// gcal sync state — default before any put, then round-trips.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn sync_state_default_then_roundtrip() {
    let s = MemStore::new();
    let init = s.get_sync_state().await;
    assert!(init.sync_token.is_none());
    assert_eq!(init.channel_created_at, 0);
    let next = GcalSyncState {
        sync_token: Some("tok-123".to_string()),
        channel_created_at: 42,
        ..Default::default()
    };
    s.put_sync_state(next).await;
    let got = s.get_sync_state().await;
    assert_eq!(got.sync_token.as_deref(), Some("tok-123"));
    assert_eq!(got.channel_created_at, 42);
}
