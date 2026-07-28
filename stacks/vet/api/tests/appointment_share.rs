//! The CLIENT half of the calendar, end to end: a shop shares ONE booking, and the person it
//! belongs to walks away with it in their own calendar.
//!
//! These drive the REAL router through `oneshot`, so what is asserted is what a phone actually
//! receives — status codes, headers and raw bytes, not a JSON shape a handler might not have used.
//!
//! The properties that matter here, and why each is pinned:
//!
//!   * the QR names a host a CLIENT'S PHONE can reach, or there is NO QR and a stated reason. A QR
//!     built from a name rather than a host shipped here once already;
//!   * the handoff carries the client's own booking and nothing of the shop's — not the operator's
//!     notes, not the client's name, and never another client's booking;
//!   * "the store could not be read" renders as ITSELF, never as a cancellation and never as a
//!     confirmation;
//!   * a booking the shop has cancelled or deleted is still expressible to a calendar that already
//!     holds it, which is the only way a stale entry is ever corrected.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;
use vet_api::app::{AppState, Config};
use vet_api::auth::JwtKeys;
use vet_api::calendar::{MockCalendar, MockCentralClient};
use vet_api::chain::MemChain;
use vet_api::custody::Custody;
use vet_api::oversight::DisabledFeed;
use vet_api::store::MemStore;
use http_body_util::BodyExt;

/// A LAN-shaped base: the repo's convention for "an address a phone on the shop's network can
/// actually reach", as opposed to the `localhost` default, which it cannot.
const REACHABLE_BASE: &str = "http://192.168.1.20:43618";

/// Build state with an explicit `DEPLOYMENT_URL` and a handle on the store, so a test can choose the
/// base under test and inject a read failure. The shared `state_with` fixes both.
fn state_with_base(deployment_url: &str, business_type: &str) -> (AppState, Arc<MemStore>) {
    let mut issuer_addrs = HashMap::new();
    issuer_addrs.insert(
        "VACCINATION".to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
    );
    let cfg = Config {
        deployment_url: deployment_url.to_string(),
        rpc_url: "memchain".to_string(),
        issuer_registry_addr: "0x00000000000000000000000000000000000000aa".to_string(),
        factory_addr: FACTORY_ADDR.to_string(),
        issuer_addrs,
        issuer_name: "Pampered Paws".to_string(),
        issuer_domain: "groomer.example".to_string(),
        verification_registry_consent_addr: VREG_CONSENT_ADDR.to_string(),
        sbt_consent_addr: SBT_CONSENT_ADDR.to_string(),
        profile_issuer_addr: PROFILE_ISSUER_ADDR.to_string(),
        vet_signer_index: 0,
        operator_password: OPERATOR_PW.to_string(),
        admin_password: ADMIN_PW.to_string(),
        confirmations: 1,
        business_id: BUSINESS_ID.to_string(),
        business_type: business_type.to_string(),
        central_hmac_secret: CENTRAL_HMAC_SECRET.to_string(),
        custody_seal_path: None,
    };
    let store = Arc::new(MemStore::new());
    let state = AppState {
        store: store.clone(),
        chain: Arc::new(MemChain::new()),
        consent_prover: Arc::new(vet_api::prover::ConsentProver::disabled()),
        calendar: Arc::new(MockCalendar::new()),
        central: Arc::new(MockCentralClient::new()),
        custody: Custody::new(),
        jwt: JwtKeys::generate(),
        cfg: Arc::new(cfg),
        ratelimit: Arc::new(vet_api::auth::RateLimiter::new()),
        feed: Arc::new(DisabledFeed),
    };
    (state, store)
}

async fn app_with_base(base: &str) -> (axum::Router, String, Arc<MemStore>) {
    let (state, store) = state_with_base(base, "groomer");
    let op = mint_operator(&state).await;
    (vet_api::router(state), op, store)
}

/// The ordinary harness: a reachable base, groomer role.
async fn app() -> (axum::Router, String, Arc<MemStore>) {
    app_with_base(REACHABLE_BASE).await
}

/// The `cache-control` a path answered with.
async fn cache_control(app: &axum::Router, path: &str) -> String {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    resp.headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

/// An UNAUTHENTICATED GET returning (status, content-type, content-disposition, raw body).
async fn get_raw(app: &axum::Router, path: &str) -> (StatusCode, String, String, String) {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let hdr = |n: &str| {
        resp.headers()
            .get(n)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    let (ctype, disp) = (hdr("content-type"), hdr("content-disposition"));
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, ctype, disp, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Create a client with one pet, then a booking for them. Returns the appointment id.
///
/// `notes` and the client's name carry PLANTED, distinctive strings: the leakage tests assert on
/// those exact values, so they cannot pass by accident on a term that also appears in a path or a
/// fixture name.
async fn make_appointment(app: &axum::Router, op: &str, service: &str) -> String {
    made(app, op, service).await.0
}

/// As [`make_appointment`], also returning the ids an edit needs — `PUT /appointments/{id}` takes a
/// FULL replacement body, so a test that cancels or moves a booking has to resend them.
async fn made(app: &axum::Router, op: &str, service: &str) -> (String, String, String) {
    let (s, b) = call(
        app,
        "POST",
        "/clients",
        Some(op),
        Some(json!({ "name": PLANTED_CLIENT_NAME, "pets": [{ "name": "Rex" }] })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "create client: {b}");
    let client_id = b["clientId"].as_str().unwrap().to_string();
    let pet_id = b["pets"][0]["petId"].as_str().unwrap().to_string();

    let (s, b) = call(
        app,
        "POST",
        "/appointments",
        Some(op),
        Some(json!({
            "clientId": client_id,
            "petId": pet_id,
            "service": service,
            "startAt": 1_772_020_800u64,
            "endAt": 1_772_024_400u64,
            "groomer": "Sam",
            "notes": PLANTED_NOTES,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "create appointment: {b}");
    (
        b["appointmentId"].as_str().unwrap().to_string(),
        client_id,
        pet_id,
    )
}

/// Re-send a booking with one field changed. The route replaces the whole row, so every field the
/// shop still wants has to be present — including the planted notes, which the leakage tests rely on
/// surviving the edit.
async fn edit(
    app: &axum::Router,
    op: &str,
    appt_id: &str,
    client_id: &str,
    pet_id: &str,
    start_at: u64,
    end_at: u64,
    status: &str,
) {
    let (s, b) = call(
        app,
        "PUT",
        &format!("/appointments/{appt_id}"),
        Some(op),
        Some(json!({
            "clientId": client_id,
            "petId": pet_id,
            "service": "Full groom",
            "startAt": start_at,
            "endAt": end_at,
            "groomer": "Sam",
            "notes": PLANTED_NOTES,
            "status": status,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "edit appointment: {b}");
}

/// Distinctive enough that finding it anywhere in a client-facing byte stream is unambiguous.
const PLANTED_NOTES: &str = "INTERNAL-ONLY-ZZQX dog bites, muzzle on arrival";
const PLANTED_CLIENT_NAME: &str = "Alice Wintermute-Kowalczyk";

/// Mint a handoff for `appt_id` and return the whole JSON.
async fn share(app: &axum::Router, op: &str, appt_id: &str) -> Value {
    let (s, b) = call(
        app,
        "POST",
        &format!("/appointments/{appt_id}/share"),
        Some(op),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "share: {b}");
    b
}

// ================================================================================================
// the URL must be one a client's phone can reach
// ================================================================================================

#[tokio::test]
async fn the_qr_is_built_from_the_deployment_url_and_nothing_else() {
    let (app, op, _) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let b = share(&app, &op, &id).await;

    let qr = b["qrUrl"].as_str().expect("a reachable base must yield a QR");
    assert!(
        qr.starts_with(&format!("{REACHABLE_BASE}/a/")),
        "qrUrl must be <DEPLOYMENT_URL>/a/<token>: {qr}"
    );
    assert!(b["qrUnavailableReason"].is_null());

    // NOT the issuer identity. `issuer_name` is a display label and `issuer_domain` is a did:web
    // name that need not resolve — building a scannable link from either is the defect that shipped
    // in the receipt QR.
    assert!(!qr.contains("Pampered"), "the shop's NAME is not a host: {qr}");
    assert!(
        !qr.contains("groomer.example"),
        "the did:web issuer domain is not a deployment: {qr}"
    );

    // ...and the token is the low-density 32-hex shape the other scan QRs use.
    let token = b["token"].as_str().unwrap();
    assert_eq!(token.len(), 32, "16 CSPRNG bytes, hex");
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(qr.ends_with(token));
}

#[tokio::test]
async fn a_request_host_header_cannot_choose_where_the_qr_points() {
    // Otherwise anyone able to reach the mint could aim a client's scan wherever they liked.
    let (app, op, _) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;

    let req = Request::builder()
        .method("POST")
        .uri(format!("/appointments/{id}/share"))
        .header("authorization", format!("Bearer {op}"))
        .header("host", "attacker.example")
        .header("x-forwarded-host", "attacker.example")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let b: Value = serde_json::from_slice(&bytes).unwrap();

    let qr = b["qrUrl"].as_str().unwrap();
    assert!(qr.starts_with(REACHABLE_BASE), "{qr}");
    assert!(!qr.contains("attacker.example"), "{qr}");
}

#[tokio::test]
async fn a_loopback_deployment_url_yields_no_qr_and_says_why() {
    // The SHIPPED DEFAULT (`main.rs` -> http://localhost:{port}`). It resolves on the operator's own
    // laptop, so a QR built from it looks proven while being unscannable by any client.
    let (app, op, _) = app_with_base("http://localhost:43618").await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let b = share(&app, &op, &id).await;

    assert!(
        b["qrUrl"].is_null(),
        "a loopback base must render NO QR, not a dead one: {b}"
    );
    let reason = b["qrUnavailableReason"]
        .as_str()
        .expect("withholding a QR must be explained");
    assert!(reason.contains("loopback"), "{reason}");
    assert!(reason.contains("DEPLOYMENT_URL"), "the fix must be named: {reason}");
    // The link itself is still offered — it is the working one ON THIS MACHINE, which is where a
    // dev run needs it. Withholding the QR is the claim being made, not hiding the surface.
    assert!(
        b["url"].as_str().unwrap().starts_with("http://localhost:43618/a/"),
        "{b}"
    );
}

#[tokio::test]
async fn an_unset_deployment_url_yields_neither_a_qr_nor_a_link() {
    let (app, op, _) = app_with_base("").await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let b = share(&app, &op, &id).await;

    assert!(b["qrUrl"].is_null(), "{b}");
    assert!(
        b["url"].is_null(),
        "with no base there is no absolute link to offer at all: {b}"
    );
    let reason = b["qrUnavailableReason"].as_str().unwrap();
    assert!(reason.contains("DEPLOYMENT_URL"), "{reason}");
    // The relative paths still exist, so the portal can show what the route WOULD be.
    assert!(b["path"].as_str().unwrap().starts_with("/a/"));
}

#[tokio::test]
async fn a_deployment_with_no_reachable_base_publishes_no_url_property_in_the_ics() {
    // A calendar entry pointing at `localhost` would dead-end on the client's phone. Better to carry
    // no back-link than a broken one.
    let (app, op, _) = app_with_base("http://localhost:43618").await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let b = share(&app, &op, &id).await;
    let token = b["token"].as_str().unwrap();

    let (s, _, _, body) = get_raw(&app, &format!("/a/{token}.ics")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!body.contains("URL:"), "no fabricated back-link: {body}");
    assert!(!body.contains("localhost"), "{body}");
}

// ================================================================================================
// minting is the shop's, resolving is the client's
// ================================================================================================

#[tokio::test]
async fn minting_a_handoff_requires_an_operator_session() {
    let (app, op, _) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let (s, _) = call(
        &app,
        "POST",
        &format!("/appointments/{id}/share"),
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sharing_an_appointment_that_does_not_exist_is_a_404() {
    let (app, op, _) = app().await;
    let (s, _) = call(
        &app,
        "POST",
        "/appointments/no-such-booking/share",
        Some(&op),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_handoff_token_is_not_consumed_by_reading_it() {
    // The difference from the one-time `/r/` record share, and it is load-bearing: the client opens
    // the page, THEN downloads the `.ics` it links to, and may re-open it a week later to check the
    // booking still stands. A consuming token breaks at the second step.
    let (app, op, _) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let token = share(&app, &op, &id).await["token"].as_str().unwrap().to_string();

    for attempt in 1..=3 {
        let (s, _, _, _) = get_raw(&app, &format!("/a/{token}")).await;
        assert_eq!(s, StatusCode::OK, "page read {attempt} must still resolve");
        let (s, _, _, _) = get_raw(&app, &format!("/a/{token}.ics")).await;
        assert_eq!(s, StatusCode::OK, "ics read {attempt} must still resolve");
    }
}

#[tokio::test]
async fn an_unknown_token_resolves_to_nothing_on_both_shapes() {
    let (app, _op, _) = app().await;
    let (s, _, _, body) = get_raw(&app, &format!("/a/{}", "f".repeat(32))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(body.contains("not valid"), "{body}");

    let (s, _, _, _) = get_raw(&app, &format!("/a/{}.ics", "f".repeat(32))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// ================================================================================================
// no leakage: this client's booking, and nothing else of the shop's
// ================================================================================================

#[tokio::test]
async fn the_handoff_carries_neither_the_shop_s_notes_nor_the_client_s_name() {
    let (app, op, _) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let token = share(&app, &op, &id).await["token"].as_str().unwrap().to_string();

    let (_, _, _, page) = get_raw(&app, &format!("/a/{token}")).await;
    let (_, _, _, ics) = get_raw(&app, &format!("/a/{token}.ics")).await;

    for planted in [PLANTED_NOTES, PLANTED_CLIENT_NAME, "INTERNAL-ONLY-ZZQX"] {
        assert!(!page.contains(planted), "the PAGE leaked {planted:?}:\n{page}");
        assert!(!ics.contains(planted), "the ICS leaked {planted:?}:\n{ics}");
    }

    // The counter-assertion, so the test above cannot pass merely because the surface is empty:
    // what the client legitimately needs IS there.
    assert!(ics.contains("SUMMARY:Full groom - Rex"), "{ics}");
    assert!(page.contains("Full groom - Rex"), "{page}");
    assert!(page.contains("Pampered Paws"), "{page}");
}

#[tokio::test]
async fn a_token_resolves_to_its_own_appointment_and_never_another() {
    let (app, op, _) = app().await;
    let first = make_appointment(&app, &op, "Full groom").await;
    let second = make_appointment(&app, &op, "Nail clipping ONLY-FOR-SECOND").await;

    let t1 = share(&app, &op, &first).await["token"].as_str().unwrap().to_string();
    let (_, _, _, ics) = get_raw(&app, &format!("/a/{t1}.ics")).await;

    assert!(ics.contains("Full groom"), "{ics}");
    assert!(
        !ics.contains("ONLY-FOR-SECOND"),
        "one token must never expose another booking:\n{ics}"
    );
    assert!(!ics.contains(&second), "{ics}");
    assert_eq!(
        ics.matches("BEGIN:VEVENT").count(),
        1,
        "a handoff is ONE appointment:\n{ics}"
    );
}

// ================================================================================================
// what a phone receives
// ================================================================================================

#[tokio::test]
async fn the_ics_is_served_as_a_calendar_attachment_a_phone_opens_natively() {
    let (app, op, _) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let token = share(&app, &op, &id).await["token"].as_str().unwrap().to_string();

    let (s, ctype, disp, body) = get_raw(&app, &format!("/a/{token}.ics")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(ctype.starts_with("text/calendar"), "{ctype}");
    assert!(disp.contains("attachment"), "iOS/Android hand it to the calendar app: {disp}");
    assert!(disp.contains(".ics"), "{disp}");

    assert!(body.starts_with("BEGIN:VCALENDAR\r\n"), "{body}");
    assert!(body.ends_with("END:VCALENDAR\r\n"), "{body}");
    assert!(body.contains("DTSTART:20260225T120000Z\r\n"), "{body}");
    assert!(body.contains("DTEND:20260225T130000Z\r\n"), "{body}");
    // every physical line CRLF-terminated — Apple Calendar rejects bare LF
    assert_eq!(body.matches('\n').count(), body.matches("\r\n").count());
}

#[tokio::test]
async fn the_page_offers_the_ics_and_an_add_to_google_link() {
    let (app, op, _) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let token = share(&app, &op, &id).await["token"].as_str().unwrap().to_string();

    let (s, ctype, _, page) = get_raw(&app, &format!("/a/{token}")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(ctype.starts_with("text/html"), "{ctype}");

    assert!(page.contains(&format!("/a/{token}.ics")), "the .ics link: {page}");
    assert!(
        page.contains("calendar.google.com/calendar/render"),
        "the add-to-Google path: {page}"
    );
    // UTC instants in the Google link — no zone to re-resolve, so no DST class.
    assert!(page.contains("dates=20260225T120000Z%2F20260225T130000Z"), "{page}");
    // The snapshot limitation is stated where the client acts on it, not left to be discovered.
    assert!(page.contains("will not update on its own"), "{page}");
}

#[tokio::test]
async fn the_page_states_the_time_in_utc_for_a_reader_with_no_javascript() {
    // An unlabelled wall clock in a zone the reader does not know is a wrong time. The markup
    // carries a LABELLED UTC rendering; script localises it for everyone else.
    let (app, op, _) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let token = share(&app, &op, &id).await["token"].as_str().unwrap().to_string();

    let (_, _, _, page) = get_raw(&app, &format!("/a/{token}")).await;
    assert!(page.contains("2026-02-25 12:00 – 2026-02-25 13:00 UTC"), "{page}");
    assert!(page.contains("data-start=\"1772020800\""), "{page}");
}

// ================================================================================================
// a cancelled or deleted booking — a stale calendar entry is a wrong claim about the world
// ================================================================================================

#[tokio::test]
async fn a_cancelled_booking_publishes_as_cancelled_and_the_page_does_not_offer_a_bare_add() {
    let (app, op, _) = app().await;
    let (id, client_id, pet_id) = made(&app, &op, "Full groom").await;
    let token = share(&app, &op, &id).await["token"].as_str().unwrap().to_string();

    edit(
        &app,
        &op,
        &id,
        &client_id,
        &pet_id,
        1_772_020_800,
        1_772_024_400,
        "cancelled",
    )
    .await;

    let (s, _, _, ics) = get_raw(&app, &format!("/a/{token}.ics")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        ics.contains("STATUS:CANCELLED\r\n"),
        "re-adding must REMOVE it from the client's calendar:\n{ics}"
    );

    let (s, _, _, page) = get_raw(&app, &format!("/a/{token}")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(page.contains("Cancelled"), "{page}");
    assert!(
        !page.contains(">Add to calendar<"),
        "a cancelled booking must not offer an unqualified add:\n{page}"
    );
    assert!(
        !page.contains("calendar.google.com"),
        "nor an add-to-Google for a booking that is off:\n{page}"
    );
    assert!(page.contains("removes it"), "the honest action is named: {page}");
}

#[tokio::test]
async fn the_page_is_never_cached_so_it_cannot_show_a_state_the_booking_has_left() {
    // The client is expected to RE-OPEN this link to re-check the booking, so a stored copy is the
    // one way this surface can confidently state a booking it has already left — rendering
    // "Scheduled" beside a working "Add to calendar" for something the shop has cancelled. Nothing
    // else about a 200 text/html response stops a browser or an intermediary caching it heuristically.
    let (app, op, _) = app().await;
    let (id, client_id, pet_id) = made(&app, &op, "Full groom").await;
    let token = share(&app, &op, &id).await["token"].as_str().unwrap().to_string();

    for path in [format!("/a/{token}"), format!("/a/{token}.ics")] {
        let cc = cache_control(&app, &path).await;
        assert!(
            cc.contains("no-store") || cc.contains("no-cache"),
            "{path} must not be servable from cache, got {cc:?}"
        );
        assert!(
            cc.contains("private"),
            "{path} carries a token in the URL and must stay out of shared caches, got {cc:?}"
        );
    }

    // ...and the state really does change under the same URL, which is what makes caching harmful.
    edit(&app, &op, &id, &client_id, &pet_id, 1_772_020_800, 1_772_024_400, "cancelled").await;
    let (_, _, _, page) = get_raw(&app, &format!("/a/{token}")).await;
    assert_eq!(state_pill(&page), Some("Cancelled".to_string()), "{page}");
}

#[tokio::test]
async fn a_deleted_booking_publishes_a_tombstone_rather_than_a_bare_404() {
    // A 404 leaves the client's calendar entry standing forever with only a sync error behind it.
    // A CANCELLED event under the same UID is what actually clears it.
    let (app, op, _) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let token = share(&app, &op, &id).await["token"].as_str().unwrap().to_string();

    // capture the UID the client's calendar now holds
    let (_, _, _, before) = get_raw(&app, &format!("/a/{token}.ics")).await;
    let uid_line = before
        .lines()
        .find(|l| l.starts_with("UID:"))
        .expect("the live event must carry a UID")
        .trim_end()
        .to_string();

    let (s, _) = call(&app, "DELETE", &format!("/appointments/{id}"), Some(&op), None).await;
    assert!(s.is_success(), "delete: {s}");

    let (s, ctype, _, after) = get_raw(&app, &format!("/a/{token}.ics")).await;
    assert_eq!(s, StatusCode::OK, "a tombstone is served, not a 404");
    assert!(ctype.starts_with("text/calendar"), "{ctype}");
    assert!(after.contains("STATUS:CANCELLED\r\n"), "{after}");
    assert!(
        after.contains(&uid_line),
        "a tombstone under a DIFFERENT UID cancels nothing.\nheld: {uid_line}\ngot:\n{after}"
    );

    // ...and the page says so plainly, without claiming the booking merely could not be checked.
    let (s, _, _, page) = get_raw(&app, &format!("/a/{token}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(page.contains("no longer booked"), "{page}");
    assert!(!page.to_lowercase().contains("could not check"), "{page}");
}

// ================================================================================================
// "could not check" is its own state — never a pass, never a failure
// ================================================================================================

#[tokio::test]
async fn an_unreadable_store_says_it_could_not_check_and_claims_nothing_else() {
    let (app, op, store) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let token = share(&app, &op, &id).await["token"].as_str().unwrap().to_string();

    store.set_fail_appointment_reads(true);

    let (s, _, _, page) = get_raw(&app, &format!("/a/{token}")).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "not a 200, not a 404");
    let lower = page.to_lowercase();

    // The STATE the page renders is the pill, so that is what must be asserted on — not the mere
    // presence of a word. The copy below legitimately contains "cancelled", in the sentence telling
    // the reader this is NOT one.
    assert_eq!(
        state_pill(&page),
        Some("Could not check".to_string()),
        "the rendered state must be its own, not either neighbour:\n{page}"
    );
    // Neither neighbour's state is rendered anywhere on the page.
    for neighbour in ["Cancelled", "Removed", "Confirmed", "Scheduled", "In progress"] {
        assert!(
            !page.contains(&format!(">{neighbour}</span>")),
            "an unreadable store rendered the {neighbour:?} state:\n{page}"
        );
    }
    assert!(!lower.contains("no longer booked"), "{page}");
    // No add-to-calendar affordance: this process does not currently know what it would be adding.
    assert!(!page.contains(".ics"), "{page}");
    assert!(!page.contains("calendar.google.com"), "{page}");
    // ...and it says so in as many words, because a client who sees a scary page will otherwise
    // assume the worst.
    assert!(lower.contains("does not mean it was cancelled"), "{page}");
}

#[tokio::test]
async fn an_unreadable_store_never_publishes_a_calendar_body_at_all() {
    // A tombstone here would cancel a booking that may be perfectly live; the last-known event would
    // state a slot this process did not read. 503 is the only truthful answer.
    let (app, op, store) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;
    let token = share(&app, &op, &id).await["token"].as_str().unwrap().to_string();

    store.set_fail_appointment_reads(true);

    let (s, ctype, _, body) = get_raw(&app, &format!("/a/{token}.ics")).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!ctype.starts_with("text/calendar"), "{ctype}");
    assert!(!body.contains("BEGIN:VCALENDAR"), "{body}");
    assert!(!body.contains("STATUS:CANCELLED"), "{body}");
    assert!(body.contains("does NOT mean it was cancelled"), "{body}");
}

#[tokio::test]
async fn an_unreadable_store_refuses_to_mint_rather_than_minting_a_link_to_nothing() {
    let (app, op, store) = app().await;
    let id = make_appointment(&app, &op, "Full groom").await;
    store.set_fail_appointment_reads(true);

    let (s, b) = call(
        &app,
        "POST",
        &format!("/appointments/{id}/share"),
        Some(&op),
        Some(json!({})),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::SERVICE_UNAVAILABLE,
        "a failed read must not become a 404 'no such appointment': {b}"
    );
}

// ================================================================================================
// supersede semantics — what makes a re-download correct a moved booking
// ================================================================================================

#[tokio::test]
async fn moving_a_booking_republishes_the_same_uid_with_a_higher_sequence() {
    // This is the whole mechanism by which a client's existing calendar entry is CORRECTED rather
    // than duplicated when they re-open the link.
    //
    // The booking is seeded as having been made an hour ago rather than by POSTing it: `SEQUENCE` is
    // derived from `updated_at - created_at`, both unix SECONDS, so a booking created and edited
    // inside the same second cannot advance it (see `appointment_share::client_sequence`). Backdating
    // `created_at` reproduces the real case — a shop moves a booking well after taking it — instead
    // of racing the clock. The EDIT itself still goes through the real route.
    let (app, op, store) = app().await;
    let (id, client_id, pet_id) = made(&app, &op, "Full groom").await;
    {
        use vet_api::store::Store as _;
        let mut a = store.get_appointment(&id).await.expect("seeded booking");
        // Both stamps move back together, so the booking reads as "made an hour ago and untouched
        // since": revision 0 now, and whatever the edit below makes it.
        a.created_at -= 3600;
        a.updated_at -= 3600;
        store.put_appointment(a).await;
    }
    let token = share(&app, &op, &id).await["token"].as_str().unwrap().to_string();

    let (_, _, _, before) = get_raw(&app, &format!("/a/{token}.ics")).await;
    let uid = before.lines().find(|l| l.starts_with("UID:")).unwrap().to_string();
    let seq_before = sequence_of(&before);

    let moved = 1_772_020_800u64 + 86_400;
    edit(&app, &op, &id, &client_id, &pet_id, moved, moved + 3600, "scheduled").await;

    let (_, _, _, after) = get_raw(&app, &format!("/a/{token}.ics")).await;
    assert!(after.contains("DTSTART:20260226T120000Z\r\n"), "the new slot: {after}");
    assert!(
        after.lines().any(|l| l.to_string() == uid),
        "the UID must be STABLE or the client gets a second event.\nbefore: {uid}\nafter:\n{after}"
    );
    assert!(
        sequence_of(&after) > seq_before,
        "SEQUENCE must advance so the client's calendar supersedes its copy: {seq_before} -> {}",
        sequence_of(&after)
    );
}

/// The STATE the page renders, read off its status pill.
///
/// Asserting on the pill rather than on whether a word appears anywhere is what makes the
/// "could not check" tests meaningful: that page's copy legitimately contains the word "cancelled",
/// in the sentence saying this is not one.
fn state_pill(page: &str) -> Option<String> {
    let after = page.split("class=\"pill ").nth(1)?;
    let inner = after.split_once('>')?.1;
    Some(inner.split_once("</span>")?.0.trim().to_string())
}

/// The `SEQUENCE` an `.ics` publishes, or 0 when it publishes none.
fn sequence_of(ics: &str) -> u32 {
    ics.lines()
        .find_map(|l| l.strip_prefix("SEQUENCE:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

// ================================================================================================
// role: a groomer books appointments, so a groomer must be able to hand one over
// ================================================================================================

#[tokio::test]
async fn the_handoff_exists_for_a_groomer_which_issues_nothing() {
    // The issuance routes (`/r/`, `/p/`, `/records/...`) are mounted only for issuing roles. This
    // surface is business data, not issuance, and the groomer is precisely the role that needs it.
    let (app, op, _) = app_with_base(REACHABLE_BASE).await; // business_type = "groomer"
    let id = make_appointment(&app, &op, "Full groom").await;
    let b = share(&app, &op, &id).await;
    assert!(b["qrUrl"].as_str().is_some());

    let token = b["token"].as_str().unwrap();
    let (s, _, _, _) = get_raw(&app, &format!("/a/{token}")).await;
    assert_eq!(s, StatusCode::OK);

    // ...and the issuance surface really is absent for this role, so the assertion above is not
    // quietly running against a vet.
    let (s, _) = call(&app, "GET", "/records", Some(&op), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "this harness must be a non-issuer");
}
