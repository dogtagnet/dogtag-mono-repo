//! Phase-7 acceptance: Google Calendar two-way sync + the business-side appointment replica.
//!
//! Hermetic — no live Google, no live central. Drives the real Axum router with a `MockCalendar`
//! (programmable events.list incl. a 410), a `MemStore`, and a `MockCentralClient` (the SOLE rev
//! allocator). Covers: echo-loop avoidance (etag-primary §13.3), human-edit detection, 410 full
//! resync, reschedule/cancel consistency (terminal wins / stale_rev), and appointment-events
//! ownership + rev allocation.

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use common::*;
use vet_api::calendar::{CalEvent, MockCalendar, MockCentralClient};
use vet_api::store::ApptReplica;

// --------------------------------------------------------------------------------------------
// helpers
// --------------------------------------------------------------------------------------------

/// Issue an HMAC-signed cross-backend request (as central would), with an Idempotency-Key.
async fn signed_call(
    app: &axum::Router,
    method: &str,
    path: &str,
    idempotency_key: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let body_bytes = serde_json::to_vec(body).unwrap();
    let sig = vet_api::auth::hmac_sign(CENTRAL_HMAC_SECRET, method, path, &body_bytes);
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("X-DogTag-HMAC", sig)
        .header("Idempotency-Key", idempotency_key)
        .body(Body::from(body_bytes))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) };
    (status, v)
}

/// The central appointment JSON shape `{id, businessId, dogTagId, slot, rev, state, updatedAt}`.
fn appt_body(id: &str, rev: u64, state: &str, slot: &str) -> Value {
    json!({
        "id": id, "businessId": BUSINESS_ID, "dogTagId": "42",
        "slot": slot, "rev": rev, "state": state, "updatedAt": 1_700_000_000u64
    })
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

// --------------------------------------------------------------------------------------------
// 1. Echo-loop avoidance + 2. human edit not dropped (etag-PRIMARY §13.3 / §8.1).
// --------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn echo_loop_avoided_and_human_edit_not_dropped() {
    let cal = Arc::new(MockCalendar::new());
    let central = Arc::new(MockCentralClient::new());
    let state = state_for_calendar(cal.clone(), central.clone());
    let store = state.store.clone();
    let op = mint_operator(&state).await;
    let app = vet_api::router(state);

    // central PUTs an appointment -> the business upserts the replica AND mirrors it to (mock) Google
    // with a dogtag.owned tag + a stored etag.
    let (s, _b) = signed_call(&app, "PUT", "/v1/appointments/appt-1", "idem-1", &appt_body("appt-1", 1, "REQUESTED", "2026-07-01T10:00:00Z")).await;
    assert_eq!(s, StatusCode::OK);

    // the mirror created exactly one Google event, mapping stored with an etag.
    let map = store.get_gcal_map_by_appt("appt-1").await.expect("mapping created");
    let gevent_id = map.google_event_id.clone();
    let echo_etag = map.etag.clone();
    assert_eq!(cal.upsert_count(), 1, "exactly one mirror write");

    // appointment count before the echo sync.
    let before: usize = store.appts_updated_since(0).await.len();
    assert_eq!(before, 1);

    // ---- ECHO: the next /calendar/sync returns OUR event back (owned tag + MATCHING etag) ----
    cal.queue_page(
        vec![CalEvent {
            id: gevent_id.clone(),
            etag: echo_etag.clone(),
            owned: true,
            appt_id: Some("appt-1".to_string()),
            rev: Some(1),
            cancelled: false,
            summary: "DogTag appt 42 (REQUESTED)".to_string(),
            start: "2026-07-01T10:00:00Z".to_string(),
            end: "2026-07-01T10:30:00Z".to_string(),
        }],
        "sync-token-after-echo",
    );
    let (s, b) = call(&app, "POST", "/calendar/sync", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "sync: {b}");
    assert_eq!(b["echoesSkipped"], 1, "our own echo must be SKIPPED");
    assert_eq!(b["humanEdits"], 0);
    // no duplicate appointment created.
    assert_eq!(store.appts_updated_since(0).await.len(), before, "echo must not create a duplicate");
    let mirror_writes_after_echo = cal.upsert_count();

    // ---- HUMAN EDIT: same event comes back with a CHANGED etag (a human edited it in Google) ----
    cal.queue_page(
        vec![CalEvent {
            id: gevent_id.clone(),
            etag: "\"etag-HUMAN-EDITED\"".to_string(), // etag changed -> NOT our echo
            owned: true,
            appt_id: Some("appt-1".to_string()),
            rev: Some(1),
            cancelled: false,
            summary: "moved by a human".to_string(),
            start: "2026-07-01T14:00:00Z".to_string(),
            end: "2026-07-01T14:30:00Z".to_string(),
        }],
        "sync-token-after-human-edit",
    );
    let (s, b) = call(&app, "POST", "/calendar/sync", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "sync: {b}");
    assert_eq!(b["echoesSkipped"], 0, "a changed etag is NOT an echo");
    assert_eq!(b["humanEdits"], 1, "the human edit must be DETECTED, not silently dropped");
    assert_eq!(b["reconciled"], 1, "platform-wins: the edit is reconciled (re-mirrored)");
    // platform-wins => we re-mirrored over the human edit (one more upsert).
    assert!(cal.upsert_count() > mirror_writes_after_echo, "human edit triggers a re-mirror (platform wins)");
}

// --------------------------------------------------------------------------------------------
// 3. 410 -> full resync wipes the mirror.
// --------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_410_triggers_full_resync() {
    let cal = Arc::new(MockCalendar::new());
    let central = Arc::new(MockCentralClient::new());
    let state = state_for_calendar(cal.clone(), central.clone());
    let store = state.store.clone();
    let op = mint_operator(&state).await;
    let app = vet_api::router(state);

    // seed a mirror entry + an external busy block so we can prove the wipe.
    signed_call(&app, "PUT", "/v1/appointments/appt-x", "idem-x", &appt_body("appt-x", 1, "REQUESTED", "2026-07-02T09:00:00Z")).await;
    assert!(!store.all_gcal_maps().await.is_empty(), "mirror seeded");

    // queue a 410, then the full-list response that follows the wipe (one external busy block).
    cal.queue_gone();
    cal.queue_page(
        vec![CalEvent {
            id: "ext-evt-1".to_string(),
            etag: "\"ext-etag\"".to_string(),
            owned: false,
            appt_id: None,
            rev: None,
            cancelled: false,
            summary: "external meeting".to_string(),
            start: "2026-07-03T11:00:00Z".to_string(),
            end: "2026-07-03T12:00:00Z".to_string(),
        }],
        "fresh-token-after-resync",
    );

    let (s, b) = call(&app, "POST", "/calendar/sync", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "sync must not crash on 410: {b}");
    assert_eq!(b["fullResync"], true, "410 must trigger a full resync");
    assert_eq!(b["busyBlocks"], 1, "the external event becomes a read-only busy block");

    // the old mirror entry was WIPED; only the post-resync busy block remains.
    let maps = store.all_gcal_maps().await;
    assert_eq!(maps.len(), 1, "mirror wiped + rebuilt");
    assert_eq!(maps[0].direction, "in", "the rebuilt entry is the external busy block");
    // token advanced to the fresh one.
    assert_eq!(store.get_sync_state().await.sync_token.as_deref(), Some("fresh-token-after-resync"));
}

// --------------------------------------------------------------------------------------------
// 4. reschedule/cancel consistency: newer rev re-mirrors; terminal (cancel) wins over a later
//    CONFIRMED with an older rev; a strictly older rev is rejected stale_rev.
// --------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reschedule_and_cancel_consistency() {
    let cal = Arc::new(MockCalendar::new());
    let central = Arc::new(MockCentralClient::new());
    let state = state_for_calendar(cal.clone(), central.clone());
    let store = state.store.clone();
    let app = vet_api::router(state);

    // rev 1: create.
    let (s, _b) = signed_call(&app, "PUT", "/v1/appointments/appt-2", "i1", &appt_body("appt-2", 1, "REQUESTED", "2026-07-04T09:00:00Z")).await;
    assert_eq!(s, StatusCode::OK);
    let writes_after_create = cal.upsert_count();

    // central reschedule at rev 2 (newer) -> updates the replica + re-mirrors to Google.
    let resched = json!({ "rev": 2u64, "slot": "2026-07-04T15:00:00Z", "state": "REQUESTED" });
    let (s, b) = signed_call(&app, "POST", "/v1/appointments/appt-2/reschedule", "i2", &resched).await;
    assert_eq!(s, StatusCode::OK, "reschedule: {b}");
    assert_eq!(b["rev"], 2);
    assert_eq!(b["slot"], "2026-07-04T15:00:00Z", "replica reflects the new slot");
    assert!(cal.upsert_count() > writes_after_create, "reschedule re-mirrors to Google");
    // the Google mirror reflects the updated slot.
    let map = store.get_gcal_map_by_appt("appt-2").await.unwrap();
    assert_eq!(cal.get_event(&map.google_event_id).unwrap().start, "2026-07-04T15:00:00Z");

    // central cancel at rev 3 (terminal).
    let cancel = json!({ "rev": 3u64, "state": "CANCELLED" });
    let (s, b) = signed_call(&app, "POST", "/v1/appointments/appt-2/cancel", "i3", &cancel).await;
    assert_eq!(s, StatusCode::OK, "cancel: {b}");
    assert_eq!(b["state"], "CANCELLED");

    // a LATER CONFIRMED arrives with an OLDER rev (2) -> stale_rev (rejected); terminal preserved.
    let stale = appt_body("appt-2", 2, "CONFIRMED", "2026-07-04T15:00:00Z");
    let (s, b) = signed_call(&app, "PUT", "/v1/appointments/appt-2", "i4", &stale).await;
    assert_eq!(s, StatusCode::CONFLICT, "older rev must be stale_rev: {b}");
    assert_eq!(b["error"], "stale_rev");

    // even a NEWER rev CONFIRMED cannot move OUT of the terminal CANCELLED state (terminal wins).
    let newer_confirmed = appt_body("appt-2", 4, "CONFIRMED", "2026-07-04T15:00:00Z");
    let (s, _b) = signed_call(&app, "PUT", "/v1/appointments/appt-2", "i5", &newer_confirmed).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(store.get_appt("appt-2").await.unwrap().state, "CANCELLED", "terminal wins over later CONFIRMED");
}

// --------------------------------------------------------------------------------------------
// 5. appointment-events ownership + rev: the business never assigns rev; central allocates the next
//    rev; a mismatched businessId is rejected.
// --------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn appointment_events_ownership_and_rev_allocation() {
    let cal = Arc::new(MockCalendar::new());
    let central = Arc::new(MockCentralClient::new());
    let state = state_for_calendar(cal.clone(), central.clone());
    let store = state.store.clone();
    let op = mint_operator(&state).await;

    // seed a replica at rev 5 owned by THIS business; central also knows it (rev 5, REQUESTED).
    store
        .put_appt(ApptReplica {
            appointment_id: "appt-3".to_string(),
            business_id: BUSINESS_ID.to_string(),
            dog_tag_id: "42".to_string(),
            slot: "2026-07-05T09:00:00Z".to_string(),
            rev: 5,
            state: "REQUESTED".to_string(),
            updated_at: now(),
        })
        .await;
    central.seed("appt-3", BUSINESS_ID, 5, "REQUESTED");
    let app = vet_api::router(state);

    // staff confirms -> the business POSTs {appointmentId, lastRev, event} to central (NO rev field).
    let (s, b) = call(
        &app,
        "POST",
        "/v1/appointments/appt-3/staff-action",
        Some(&op),
        Some(json!({ "event": "CONFIRMED" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "staff-action: {b}");

    // central allocated the NEXT rev (5 -> 6); the business applied it (never allocated itself).
    let calls = central.calls();
    assert_eq!(calls.len(), 1);
    let (cb_biz, cb_appt, cb_last_rev, cb_event, _ts) = &calls[0];
    assert_eq!(cb_biz, BUSINESS_ID);
    assert_eq!(cb_appt, "appt-3");
    assert_eq!(*cb_last_rev, 5, "business sends lastRev, never a new rev");
    assert_eq!(cb_event, "CONFIRMED");
    assert_eq!(central.rev_of("appt-3"), Some(6), "central is the SOLE rev allocator (5 -> 6)");
    assert_eq!(b["rev"], 6, "replica reflects the central-allocated rev");
    assert_eq!(store.get_appt("appt-3").await.unwrap().rev, 6);

    // ---- ownership mismatch: an appointment owned by ANOTHER business is rejected by central ----
    store
        .put_appt(ApptReplica {
            appointment_id: "appt-other".to_string(),
            business_id: "biz-OTHER".to_string(), // not this business
            dog_tag_id: "99".to_string(),
            slot: "2026-07-06T09:00:00Z".to_string(),
            rev: 1,
            state: "REQUESTED".to_string(),
            updated_at: now(),
        })
        .await;
    central.seed("appt-other", "biz-OTHER", 1, "REQUESTED"); // central knows the true owner

    let (s, _b) = call(
        &app,
        "POST",
        "/v1/appointments/appt-other/staff-action",
        Some(&op),
        Some(json!({ "event": "CONFIRMED" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a mismatched businessId must be rejected by central");
}

// --------------------------------------------------------------------------------------------
// 6. OAuth connect URL + callback + watch renewal cron (no live scheduler).
// --------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oauth_connect_callback_and_watch_renewal() {
    let cal = Arc::new(MockCalendar::new());
    let central = Arc::new(MockCentralClient::new());
    let state = state_for_calendar(cal.clone(), central.clone());
    let store = state.store.clone();
    let op = mint_operator(&state).await;
    let app = vet_api::router(state.clone());

    // connect -> consent URL with offline + consent + the events scope.
    let (s, b) = call(&app, "GET", "/calendar/google/connect", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    let url = b["consentUrl"].as_str().unwrap();
    assert!(url.contains("access_type=offline"));
    assert!(url.contains("prompt=consent"));
    assert!(url.contains("calendar.events"));

    // callback -> token exchange stores the refresh token + stands up a watch channel.
    let (s, b) = call(&app, "GET", "/calendar/google/callback?code=test-code", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "callback: {b}");
    assert_eq!(b["connected"], true);
    assert!(store.get_sync_state().await.refresh_token.is_some(), "refresh token stored");
    assert_eq!(cal.watch_count(), 1, "watch channel created on connect");

    // the renewal cron is a no-op when not yet due (channel just created)...
    assert!(!vet_api::sync::renew_watch_if_due(&state, now()).await, "not due yet");
    // ...but re-creates the channel once ~6 days have elapsed.
    let future = now() + vet_api::sync::WATCH_RENEW_SECS + 1;
    assert!(vet_api::sync::renew_watch_if_due(&state, future).await, "due after 6 days");
    assert_eq!(cal.watch_count(), 2, "watch channel renewed");
}

// --------------------------------------------------------------------------------------------
// 7. inbound HMAC + Idempotency-Key are enforced (security asserts).
// --------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inbound_hmac_and_idempotency_enforced() {
    let cal = Arc::new(MockCalendar::new());
    let central = Arc::new(MockCentralClient::new());
    let state = state_for_calendar(cal.clone(), central.clone());
    let store = state.store.clone();
    let app = vet_api::router(state);

    // bad HMAC -> 401.
    let body = appt_body("appt-9", 1, "REQUESTED", "2026-07-07T09:00:00Z");
    let req = Request::builder()
        .method("PUT")
        .uri("/v1/appointments/appt-9")
        .header("content-type", "application/json")
        .header("X-DogTag-HMAC", "deadbeef")
        .header("Idempotency-Key", "k1")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "bad HMAC must 401");

    // valid HMAC -> 200, replica written.
    let (s, _b) = signed_call(&app, "PUT", "/v1/appointments/appt-9", "k2", &body).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(store.get_appt("appt-9").await.unwrap().rev, 1);

    // replaying the SAME Idempotency-Key is an idempotent noop (200, no error), even if the body
    // would otherwise advance the rev.
    let newer = appt_body("appt-9", 2, "CONFIRMED", "2026-07-07T09:00:00Z");
    let (s, b) = signed_call(&app, "PUT", "/v1/appointments/appt-9", "k2", &newer).await;
    assert_eq!(s, StatusCode::OK, "idempotent replay: {b}");
    assert_eq!(store.get_appt("appt-9").await.unwrap().rev, 1, "replayed key must NOT re-apply");
}
