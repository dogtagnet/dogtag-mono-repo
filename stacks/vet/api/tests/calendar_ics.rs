//! The `.ics` calendar-interop surface end to end: publishing a subscribable feed, protecting it,
//! revoking it, and importing an uploaded calendar into real appointments.
//!
//! These drive the REAL router through `oneshot`, so what is asserted is what a calendar client (or
//! the portal) actually receives — headers and raw iCalendar text included, not just a JSON shape.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use vet_api::chain::MemChain;

/// A router with an operator session already minted.
async fn app() -> (axum::Router, String) {
    let state = state_with(
        Arc::new(MemChain::new()),
        "memchain".to_string(),
        "0x00000000000000000000000000000000000000aa".to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
        "vet.example".to_string(),
        1,
    );
    let op = mint_operator(&state).await;
    (vet_api::router(state), op)
}

/// Issue an UNAUTHENTICATED GET and return (status, content-type, raw body text).
///
/// The feed is served as `text/calendar`, not JSON, so the shared `call` helper (which parses a JSON
/// body) cannot see what a subscriber sees.
async fn get_raw(app: &axum::Router, path: &str) -> (StatusCode, String, String) {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let cache = resp
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    // cache-control is folded into the content-type slot's sibling return for the one test that
    // asserts it; keeping the tuple at three keeps every other call site readable.
    (status, format!("{ctype}|{cache}"), body)
}

/// Publish a feed and return its secret.
async fn publish_feed(app: &axum::Router, op: &str) -> String {
    let (s, b) = call(app, "POST", "/calendar/feed/rotate", Some(op), None).await;
    assert_eq!(s, StatusCode::OK, "rotate: {b}");
    assert_eq!(b["enabled"], true);
    b["token"].as_str().unwrap().to_string()
}

async fn make_client(app: &axum::Router, op: &str, name: &str, pet: &str) -> (String, String) {
    let (s, b) = call(
        app,
        "POST",
        "/clients",
        Some(op),
        Some(json!({ "name": name, "pets": [{ "name": pet }] })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "create client: {b}");
    (
        b["clientId"].as_str().unwrap().to_string(),
        b["pets"][0]["petId"].as_str().unwrap().to_string(),
    )
}

/// The exact unfolding rule from RFC 5545 §3.1, so assertions can look at logical content lines
/// rather than however the writer happened to wrap them.
fn unfold(ics: &str) -> Vec<String> {
    ics.replace("\r\n ", "")
        .split("\r\n")
        .map(|s| s.to_string())
        .collect()
}

// ================================================================================================
// feed: publish / protect / revoke
// ================================================================================================

#[tokio::test]
async fn feed_is_absent_until_the_operator_publishes_one() {
    let (app, op) = app().await;

    let (s, b) = call(&app, "GET", "/calendar/feed", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["enabled"], false, "no feed exists before one is published");
    assert!(b["token"].is_null());
    assert!(b["path"].is_null());

    // ...and nothing is served, even at a well-formed-looking URL.
    let (s, _, _) = get_raw(&app, &format!("/calendar/feed/{}.ics", "a".repeat(64))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn feed_administration_requires_an_operator_session() {
    let (app, _op) = app().await;
    for (method, path) in [
        ("GET", "/calendar/feed"),
        ("POST", "/calendar/feed/rotate"),
        ("DELETE", "/calendar/feed"),
        ("POST", "/calendar/import"),
    ] {
        let (s, _) = call(&app, method, path, None, Some(json!({}))).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {path} must be gated");
    }
}

#[tokio::test]
async fn publishing_mints_a_high_entropy_secret_and_serves_the_feed_unauthenticated() {
    let (app, op) = app().await;
    let token = publish_feed(&app, &op).await;
    assert_eq!(token.len(), 64, "32 CSPRNG bytes, hex");
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));

    // A calendar client presents no bearer token: the secret in the path IS the authorization.
    let (s, meta, body) = get_raw(&app, &format!("/calendar/feed/{token}.ics")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(meta.starts_with("text/calendar"), "content-type: {meta}");
    assert!(meta.contains("no-store"), "the URL is a credential: {meta}");
    assert!(body.starts_with("BEGIN:VCALENDAR\r\n"));
    assert!(body.trim_end().ends_with("END:VCALENDAR"));

    // Both URL shapes reach the same feed — clients differ on whether they want the extension.
    let (s2, _, body2) = get_raw(&app, &format!("/calendar/feed/{token}")).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(body.lines().next(), body2.lines().next());
}

#[tokio::test]
async fn a_wrong_secret_is_indistinguishable_from_no_feed_at_all() {
    let (app, op) = app().await;
    let token = publish_feed(&app, &op).await;
    let wrong = format!("{}{}", &token[..63], if token.ends_with('a') { 'b' } else { 'a' });

    // 404, not 401/403: there is nothing to tell someone who does not hold the secret, and a
    // distinguishable refusal would confirm a feed exists to probe for.
    let (s, _, _) = get_raw(&app, &format!("/calendar/feed/{wrong}.ics")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // a prefix of the real token is not enough either
    let (s, _, _) = get_raw(&app, &format!("/calendar/feed/{}.ics", &token[..32])).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rotating_kills_the_previous_url_immediately() {
    let (app, op) = app().await;
    let first = publish_feed(&app, &op).await;
    let (s, _, _) = get_raw(&app, &format!("/calendar/feed/{first}.ics")).await;
    assert_eq!(s, StatusCode::OK);

    let second = publish_feed(&app, &op).await;
    assert_ne!(first, second);
    let (s, _, _) = get_raw(&app, &format!("/calendar/feed/{first}.ics")).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "the old link must stop working");
    let (s, _, _) = get_raw(&app, &format!("/calendar/feed/{second}.ics")).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn revoking_takes_the_feed_offline() {
    let (app, op) = app().await;
    let token = publish_feed(&app, &op).await;

    let (s, b) = call(&app, "DELETE", "/calendar/feed", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["enabled"], false);
    assert!(b["token"].is_null());

    let (s, _, _) = get_raw(&app, &format!("/calendar/feed/{token}.ics")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn switching_signing_mode_does_not_revoke_the_published_feed() {
    // Settings is one document; a careless whole-document write would silently break every
    // subscription the shop had handed out.
    let (app, op) = app().await;
    let token = publish_feed(&app, &op).await;

    let (s, _) = call(
        &app,
        "PUT",
        "/settings/signing-mode",
        Some(&op),
        Some(json!({ "mode": "wallet" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, b) = call(&app, "GET", "/calendar/feed", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["token"], token, "the feed secret must survive an unrelated settings write");
    let (s, _, _) = get_raw(&app, &format!("/calendar/feed/{token}.ics")).await;
    assert_eq!(s, StatusCode::OK);
}

// ================================================================================================
// feed: content
// ================================================================================================

#[tokio::test]
async fn an_empty_shop_publishes_an_empty_calendar_not_a_sample_one() {
    let (app, op) = app().await;
    let token = publish_feed(&app, &op).await;
    let (_, _, body) = get_raw(&app, &format!("/calendar/feed/{token}.ics")).await;
    assert!(!body.contains("BEGIN:VEVENT"), "no fabricated bookings: {body}");
    assert!(body.contains("VERSION:2.0"));
    assert!(body.contains("METHOD:PUBLISH"));
}

#[tokio::test]
async fn the_feed_publishes_real_bookings_with_stable_uids() {
    let (app, op) = app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let start = vet_api::auth::now() + 3 * 86_400;
    let (s, appt) = call(
        &app,
        "POST",
        "/appointments",
        Some(&op),
        Some(json!({
            "clientId": client_id,
            "petId": pet_id,
            "service": "Full groom",
            "startAt": start,
            "endAt": start + 5400,
            "groomer": "Sam",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{appt}");
    let appt_id = appt["appointmentId"].as_str().unwrap();

    let token = publish_feed(&app, &op).await;
    let (_, _, body) = get_raw(&app, &format!("/calendar/feed/{token}.ics")).await;
    let lines = unfold(&body);

    assert!(lines.iter().any(|l| l == "BEGIN:VEVENT"));
    assert!(
        lines.contains(&format!("UID:{appt_id}@vet.example")),
        "UID must be stable + globally unique: {body}"
    );
    assert!(lines.iter().any(|l| l == "SUMMARY:Full groom - Rex / Alice Tan"));
    assert!(lines.iter().any(|l| l.starts_with("DTSTART:") && l.ends_with('Z')));
    assert!(lines.iter().any(|l| l.starts_with("DTEND:")));
    assert!(lines.iter().any(|l| l == "STATUS:CONFIRMED"));

    // Regenerating must not mint a new identity for the same booking.
    let (_, _, again) = get_raw(&app, &format!("/calendar/feed/{token}.ics")).await;
    assert!(unfold(&again).contains(&format!("UID:{appt_id}@vet.example")));
}

#[tokio::test]
async fn a_cancelled_booking_is_published_as_cancelled_so_subscribers_drop_it() {
    let (app, op) = app().await;
    let (client_id, _pet) = make_client(&app, &op, "Bo Lim", "Momo").await;
    let start = vet_api::auth::now() + 86_400;
    let (_, appt) = call(
        &app,
        "POST",
        "/appointments",
        Some(&op),
        Some(json!({ "clientId": client_id, "service": "Bath", "startAt": start })),
    )
    .await;
    let id = appt["appointmentId"].as_str().unwrap().to_string();
    let (s, _) = call(
        &app,
        "PUT",
        &format!("/appointments/{id}"),
        Some(&op),
        Some(json!({ "clientId": client_id, "service": "Bath", "startAt": start, "status": "cancelled" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let token = publish_feed(&app, &op).await;
    let (_, _, body) = get_raw(&app, &format!("/calendar/feed/{token}.ics")).await;
    assert!(unfold(&body).iter().any(|l| l == "STATUS:CANCELLED"), "{body}");
}

#[tokio::test]
async fn feed_text_is_escaped_and_folded_so_the_grammar_survives_operator_input() {
    let (app, op) = app().await;
    let (client_id, _pet) = make_client(&app, &op, "Chen; Wei, Ltd", "Mr. Bigglesworth").await;
    let start = vet_api::auth::now() + 86_400;
    call(
        &app,
        "POST",
        "/appointments",
        Some(&op),
        Some(json!({
            "clientId": client_id,
            "service": "Full groom, de-shed; nails",
            "startAt": start,
            "notes": "line one\nline two",
        })),
    )
    .await;

    let token = publish_feed(&app, &op).await;
    let (_, _, body) = get_raw(&app, &format!("/calendar/feed/{token}.ics")).await;

    // Every PHYSICAL line stays within the 75-octet limit...
    for line in body.split("\r\n") {
        assert!(line.len() <= 75, "unfolded line ({} octets): {line}", line.len());
    }
    // ...and the LOGICAL lines carry escaped separators, not raw ones that would split a value.
    let lines = unfold(&body);
    let summary = lines.iter().find(|l| l.starts_with("SUMMARY:")).unwrap();
    assert!(summary.contains("\\;"), "semicolon must be escaped: {summary}");
    assert!(summary.contains("\\,"), "comma must be escaped: {summary}");
    let desc = lines.iter().find(|l| l.starts_with("DESCRIPTION:")).unwrap();
    assert!(desc.contains("line one\\nline two"), "{desc}");
}

// ================================================================================================
// import
// ================================================================================================

/// The normalized payload the browser parser produces from an uploaded `.ics`.
fn ics_event(uid: &str, start: u64) -> Value {
    json!({
        "uid": uid,
        "summary": "Full groom",
        "description": "from the old calendar",
        "location": "Shop",
        "startAt": start,
        "endAt": start + 3600,
    })
}

async fn import(app: &axum::Router, op: &str, events: Value) -> Value {
    let (s, b) = call(
        app,
        "POST",
        "/calendar/import",
        Some(op),
        Some(json!({ "events": events })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "import: {b}");
    b
}

#[tokio::test]
async fn import_creates_real_appointments_that_show_up_on_the_calendar() {
    let (app, op) = app().await;
    let start = vet_api::auth::now() + 2 * 86_400;
    let r = import(&app, &op, json!([ics_event("evt-1@old.example", start)])).await;
    assert_eq!(r["created"], 1);
    assert_eq!(r["updated"], 0);
    assert_eq!(r["skipped"], 0);

    let (s, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(page["total"], 1);
    let row = &page["rows"][0];
    assert_eq!(row["service"], "Full groom");
    assert_eq!(row["startAt"], start);
    assert_eq!(row["endAt"], start + 3600);
    assert_eq!(row["status"], "scheduled");
    assert_eq!(row["source"], "ics");
    assert_eq!(row["externalUid"], "evt-1@old.example");
    // An import never invents a customer to fill the client column.
    assert_eq!(row["clientId"], "");
    assert_eq!(row["clientName"], "");
    assert!(row["notes"].as_str().unwrap().contains("Location: Shop"));
}

#[tokio::test]
async fn importing_the_same_file_twice_updates_rather_than_duplicating() {
    let (app, op) = app().await;
    let start = vet_api::auth::now() + 2 * 86_400;
    let first = import(&app, &op, json!([ics_event("evt-1@old.example", start)])).await;
    assert_eq!(first["created"], 1);

    // Same file, same UIDs — the classic "did it work? let me upload it again" case.
    let second = import(&app, &op, json!([ics_event("evt-1@old.example", start)])).await;
    assert_eq!(second["created"], 0);
    assert_eq!(second["updated"], 1);

    let (_, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    assert_eq!(page["total"], 1, "one event must remain one booking");
}

#[tokio::test]
async fn a_duplicate_uid_within_one_upload_is_imported_once() {
    let (app, op) = app().await;
    let start = vet_api::auth::now() + 86_400;
    let r = import(
        &app,
        &op,
        json!([
            ics_event("evt-dup@old.example", start),
            ics_event("evt-dup@old.example", start + 3600),
        ]),
    )
    .await;
    assert_eq!(r["created"], 1);
    assert_eq!(r["skipped"], 1);
    let (_, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    assert_eq!(page["total"], 1);
}

#[tokio::test]
async fn a_reimport_moves_the_slot_but_keeps_the_client_the_operator_linked() {
    let (app, op) = app().await;
    let start = vet_api::auth::now() + 2 * 86_400;
    import(&app, &op, json!([ics_event("evt-1@old.example", start)])).await;

    // The operator does the human half: assign the booking to a real client + pet.
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let (_, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    let appt_id = page["rows"][0]["appointmentId"].as_str().unwrap().to_string();
    let (s, _) = call(
        &app,
        "PUT",
        &format!("/appointments/{appt_id}"),
        Some(&op),
        Some(json!({
            "clientId": client_id,
            "petId": pet_id,
            "service": "Full groom",
            "startAt": start,
            "status": "confirmed",
            "groomer": "Sam",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // The calendar file then moves the appointment an hour later.
    let moved = start + 3600;
    let r = import(&app, &op, json!([ics_event("evt-1@old.example", moved)])).await;
    assert_eq!(r["updated"], 1);
    assert_eq!(r["created"], 0, "editing must not orphan the booking from its source event");

    let (_, a) = call(&app, "GET", &format!("/appointments/{appt_id}"), Some(&op), None).await;
    assert_eq!(a["startAt"], moved, "the file owns WHEN");
    assert_eq!(a["clientId"], client_id, "the portal owns WHO");
    assert_eq!(a["clientName"], "Alice Tan");
    assert_eq!(a["groomer"], "Sam");
    assert_eq!(a["status"], "confirmed", "the operator's workflow state survives");
}

#[tokio::test]
async fn a_cancellation_in_the_source_file_cancels_the_booking() {
    let (app, op) = app().await;
    let start = vet_api::auth::now() + 86_400;
    import(&app, &op, json!([ics_event("evt-1@old.example", start)])).await;

    let mut cancelled = ics_event("evt-1@old.example", start);
    cancelled["status"] = json!("CANCELLED");
    let r = import(&app, &op, json!([cancelled])).await;
    assert_eq!(r["cancelled"], 1);

    let (_, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    assert_eq!(page["rows"][0]["status"], "cancelled");
}

#[tokio::test]
async fn a_cancellation_for_an_unknown_event_creates_nothing() {
    let (app, op) = app().await;
    let mut ev = ics_event("never-seen@old.example", vet_api::auth::now() + 86_400);
    ev["status"] = json!("CANCELLED");
    let r = import(&app, &op, json!([ev])).await;
    assert_eq!(r["created"], 0);
    assert_eq!(r["skipped"], 1);
    let (_, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    assert_eq!(page["total"], 0);
}

#[tokio::test]
async fn recurrence_is_reported_as_unexpanded_rather_than_silently_collapsed() {
    let (app, op) = app().await;
    let start = vet_api::auth::now() + 86_400;
    let mut weekly = ics_event("weekly@old.example", start);
    weekly["recurring"] = json!(true);
    let r = import(&app, &op, json!([weekly])).await;

    assert_eq!(r["created"], 1);
    assert_eq!(
        r["recurringNotExpanded"], 1,
        "the count is what lets the portal say it out loud"
    );
    let (_, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    let notes = page["rows"][0]["notes"].as_str().unwrap();
    assert!(
        notes.contains("does NOT expand recurrence"),
        "the caveat must live on the booking too: {notes}"
    );
}

#[tokio::test]
async fn an_all_day_event_is_marked_as_one_on_the_booking() {
    let (app, op) = app().await;
    let start = vet_api::auth::now() + 86_400;
    let mut all_day = ics_event("allday@old.example", start);
    all_day["allDay"] = json!(true);
    all_day["endAt"] = json!(start + 86_400);
    let r = import(&app, &op, json!([all_day])).await;
    assert_eq!(r["allDay"], 1);

    let (_, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    assert!(page["rows"][0]["notes"].as_str().unwrap().contains("all-day"));
}

#[tokio::test]
async fn events_without_a_uid_or_a_start_are_skipped_not_guessed_at() {
    let (app, op) = app().await;
    let r = import(
        &app,
        &op,
        json!([
            { "uid": "", "summary": "no id", "startAt": 1_772_020_800u64 },
            { "uid": "no-start@old.example", "summary": "no start", "startAt": 0 },
        ]),
    )
    .await;
    assert_eq!(r["created"], 0);
    assert_eq!(r["skipped"], 2);
    let (_, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    assert_eq!(page["total"], 0);
}

#[tokio::test]
async fn an_event_with_no_end_gets_the_portal_s_own_one_hour_default() {
    let (app, op) = app().await;
    let start = vet_api::auth::now() + 86_400;
    import(
        &app,
        &op,
        json!([{ "uid": "no-end@old.example", "summary": "Nail trim", "startAt": start }]),
    )
    .await;
    let (_, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    assert_eq!(page["rows"][0]["endAt"], start + 3600);
}

#[tokio::test]
async fn a_dry_run_reports_exactly_what_would_happen_and_writes_nothing() {
    let (app, op) = app().await;
    let start = vet_api::auth::now() + 86_400;
    let (s, b) = call(
        &app,
        "POST",
        "/calendar/import?dryRun=1",
        Some(&op),
        Some(json!({ "events": [ics_event("evt-1@old.example", start)] })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["dryRun"], true);
    assert_eq!(b["created"], 1, "the preview must report the real outcome");

    let (_, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    assert_eq!(page["total"], 0, "a preview must not touch the shop's book");
}

#[tokio::test]
async fn an_unrecognized_dry_run_value_is_refused_rather_than_guessed_at() {
    // Guessing either way is a trap: guessing "write" turns a client typo into an unwanted write,
    // guessing "preview" makes a real import quietly do nothing.
    let (app, op) = app().await;
    let (s, b) = call(
        &app,
        "POST",
        "/calendar/import?dryRun=yes",
        Some(&op),
        Some(json!({ "events": [ics_event("evt-1@old.example", vet_api::auth::now() + 86_400)] })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (_, page) = call(&app, "GET", "/appointments", Some(&op), None).await;
    assert_eq!(page["total"], 0);
}

#[tokio::test]
async fn an_empty_upload_reports_zeroes_rather_than_failing() {
    let (app, op) = app().await;
    let r = import(&app, &op, json!([])).await;
    assert_eq!(r["total"], 0);
    assert_eq!(r["created"], 0);
    assert_eq!(r["skipped"], 0);
}

// ================================================================================================
// round trip
// ================================================================================================

#[tokio::test]
async fn an_imported_booking_republishes_under_its_original_uid() {
    // Import a calendar, then subscribe to the feed from the SAME calendar app: the shop must see
    // one event per booking, not the original plus a DogTag copy of it.
    let (app, op) = app().await;
    let start = vet_api::auth::now() + 86_400;
    import(&app, &op, json!([ics_event("evt-77@google.com", start)])).await;

    let token = publish_feed(&app, &op).await;
    let (_, _, body) = get_raw(&app, &format!("/calendar/feed/{token}.ics")).await;
    let lines = unfold(&body);
    assert!(lines.contains(&"UID:evt-77@google.com".to_string()), "{body}");
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("UID:")).count(),
        1,
        "one booking, one identity"
    );
    // And an unassigned booking says so rather than publishing a blank client.
    let desc = lines.iter().find(|l| l.starts_with("DESCRIPTION:")).unwrap();
    assert!(desc.contains("Unassigned"), "{desc}");
}
