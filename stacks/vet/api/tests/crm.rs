//! The shop CRM surfaces: client + appointment CRUD, search/filter/paging, and the linkage that
//! makes a verification searchable by the appointment and client it was performed for.
//!
//! The verification-linkage tests cover the row's OPEN side (session start -> a linked, searchable
//! `pending` row). The owner-hidden consent submission settles asynchronously in a spawned task and
//! is covered by `submit_consent_levelb_route.rs`; the row it writes goes through the same
//! `crm::attach_evidence` / `crm::finish_log` seam.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use std::sync::Arc;
use vet_api::chain::MemChain;

/// A state + router with the CRM routes mounted and an operator session already minted.
async fn crm_app() -> (axum::Router, String) {
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

/// Create a client with one pet; returns (clientId, petId).
async fn make_client(app: &axum::Router, op: &str, name: &str, pet: &str) -> (String, String) {
    let (s, b) = call(
        app,
        "POST",
        "/clients",
        Some(op),
        Some(json!({
            "name": name,
            "email": format!("{}@example.com", name.to_lowercase().replace(' ', ".")),
            "phone": "+65 9123 4567",
            "pets": [{ "name": pet, "species": "dog", "breed": "Poodle" }],
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "create client: {b}");
    let pet_id = b["pets"][0]["petId"].as_str().unwrap().to_string();
    (b["clientId"].as_str().unwrap().to_string(), pet_id)
}

async fn make_appointment(
    app: &axum::Router,
    op: &str,
    client_id: &str,
    pet_id: &str,
    start_at: u64,
    service: &str,
) -> String {
    let (s, b) = call(
        app,
        "POST",
        "/appointments",
        Some(op),
        Some(json!({
            "clientId": client_id,
            "petId": pet_id,
            "service": service,
            "startAt": start_at,
            "notes": "nervous around clippers",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "create appointment: {b}");
    b["appointmentId"].as_str().unwrap().to_string()
}

// ============================================================================================
// auth
// ============================================================================================

#[tokio::test]
async fn crm_routes_require_an_operator_session() {
    let (app, _op) = crm_app().await;
    // Bodies are WELL-FORMED on purpose: like every other route in this stack, the `Json<T>`
    // extractor runs before the handler's auth check, so a malformed body would surface as a 422
    // parse rejection and prove nothing about the auth gate.
    let cases: Vec<(&str, &str, Option<Value>)> = vec![
        ("GET", "/clients", None),
        ("POST", "/clients", Some(json!({ "name": "Alice Tan" }))),
        ("GET", "/clients/some-id", None),
        ("PUT", "/clients/some-id", Some(json!({ "name": "Alice Tan" }))),
        ("DELETE", "/clients/some-id", None),
        ("GET", "/appointments", None),
        (
            "POST",
            "/appointments",
            Some(json!({ "clientId": "c", "startAt": 1_800_000_000u64 })),
        ),
        ("GET", "/appointments/some-id", None),
        ("DELETE", "/appointments/some-id", None),
        ("GET", "/verifications", None),
        ("GET", "/verifications/some-id", None),
    ];
    for (method, path, body) in cases {
        let (s, b) = call(&app, method, path, None, body).await;
        assert_eq!(
            s,
            StatusCode::UNAUTHORIZED,
            "{method} {path} must require an operator session, got {s}: {b}"
        );
    }
}

// ============================================================================================
// clients
// ============================================================================================

#[tokio::test]
async fn client_crud_roundtrip() {
    let (app, op) = crm_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;

    // read back
    let (s, b) = call(&app, "GET", &format!("/clients/{client_id}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "get client: {b}");
    assert_eq!(b["name"], "Alice Tan");
    assert_eq!(b["pets"][0]["name"], "Rex");
    assert_eq!(b["pets"][0]["petId"], pet_id.as_str());

    // update — echoing petId must PRESERVE the pet's identity so its appointment links survive.
    let (s, b) = call(
        &app,
        "PUT",
        &format!("/clients/{client_id}"),
        Some(&op),
        Some(json!({
            "name": "Alice Tan-Lim",
            "phone": "+65 8000 0000",
            "pets": [{ "petId": pet_id, "name": "Rex", "breed": "Standard Poodle" }],
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "update client: {b}");
    assert_eq!(b["name"], "Alice Tan-Lim");
    assert_eq!(b["pets"][0]["petId"], pet_id.as_str(), "petId must be stable across an edit");
    assert_eq!(b["pets"][0]["breed"], "Standard Poodle");

    // delete
    let (s, _) = call(&app, "DELETE", &format!("/clients/{client_id}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&app, "GET", &format!("/clients/{client_id}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a deleted client must be gone");
}

#[tokio::test]
async fn client_create_rejects_a_blank_name() {
    let (app, op) = crm_app().await;
    let (s, b) = call(&app, "POST", "/clients", Some(&op), Some(json!({ "name": "   " }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "blank name must be rejected: {b}");
}

#[tokio::test]
async fn client_search_matches_name_phone_and_pet_and_narrows_on_multiple_terms() {
    let (app, op) = crm_app().await;
    make_client(&app, &op, "Alice Tan", "Rex").await;
    make_client(&app, &op, "Bob Lim", "Biscuit").await;
    make_client(&app, &op, "Carol Ong", "Rexie").await;

    // by owner name
    let (s, b) = call(&app, "GET", "/clients?q=alice", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["total"], 1, "one owner matches 'alice'");
    assert_eq!(b["rows"][0]["name"], "Alice Tan");

    // by PET name — the pet is part of the client's search key
    let (s, b) = call(&app, "GET", "/clients?q=biscuit", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["total"], 1);
    assert_eq!(b["rows"][0]["name"], "Bob Lim");

    // a substring hits both Rex and Rexie...
    let (_, b) = call(&app, "GET", "/clients?q=rex", Some(&op), None).await;
    assert_eq!(b["total"], 2, "'rex' is a substring of both Rex and Rexie");

    // ...and a second term NARROWS rather than widens
    let (_, b) = call(&app, "GET", "/clients?q=rex%20carol", Some(&op), None).await;
    assert_eq!(b["total"], 1, "every term must match");
    assert_eq!(b["rows"][0]["name"], "Carol Ong");

    // an empty needle is not a filter
    let (_, b) = call(&app, "GET", "/clients?q=", Some(&op), None).await;
    assert_eq!(b["total"], 3, "a blank q must not filter anything out");
}

#[tokio::test]
async fn client_list_pages_and_reports_the_full_total() {
    let (app, op) = crm_app().await;
    for i in 0..7 {
        make_client(&app, &op, &format!("Owner {i}"), "Pet").await;
    }
    let (s, b) = call(&app, "GET", "/clients?limit=3", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["rows"].as_array().unwrap().len(), 3, "the page is bounded by limit");
    assert_eq!(b["total"], 7, "total is the full match count, not the page size");
    assert_eq!(b["limit"], 3);

    let (_, b2) = call(&app, "GET", "/clients?limit=3&offset=6", Some(&op), None).await;
    assert_eq!(b2["rows"].as_array().unwrap().len(), 1, "the last page is partial");
    assert_eq!(b2["total"], 7);
}

#[tokio::test]
async fn a_page_limit_is_clamped_so_a_caller_cannot_pull_the_whole_collection() {
    let (app, op) = crm_app().await;
    make_client(&app, &op, "Alice Tan", "Rex").await;
    let (s, b) = call(&app, "GET", "/clients?limit=100000", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["limit"], 200, "limit must be clamped to MAX_PAGE");
}

#[tokio::test]
async fn deleting_a_client_with_appointments_is_refused() {
    let (app, op) = crm_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    make_appointment(&app, &op, &client_id, &pet_id, 1_800_000_000, "Full groom").await;

    let (s, b) = call(&app, "DELETE", &format!("/clients/{client_id}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "must not orphan appointments: {b}");
    let (s, _) = call(&app, "GET", &format!("/clients/{client_id}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "the client must still exist after the refused delete");
}

#[tokio::test]
async fn renaming_a_client_refreshes_the_denormalized_name_on_their_appointments() {
    let (app, op) = crm_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let appt = make_appointment(&app, &op, &client_id, &pet_id, 1_800_000_000, "Full groom").await;

    let (s, _) = call(
        &app,
        "PUT",
        &format!("/clients/{client_id}"),
        Some(&op),
        Some(json!({ "name": "Alice Lim", "pets": [{ "petId": pet_id, "name": "Rexy" }] })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (_, b) = call(&app, "GET", &format!("/appointments/{appt}"), Some(&op), None).await;
    assert_eq!(b["clientName"], "Alice Lim", "the appointment must not keep the stale name");
    assert_eq!(b["petName"], "Rexy");
    // and the refreshed name is searchable
    let (_, b) = call(&app, "GET", "/appointments?q=alice%20lim", Some(&op), None).await;
    assert_eq!(b["total"], 1, "the appointment search key must be rebuilt too");
}

// ============================================================================================
// appointments
// ============================================================================================

#[tokio::test]
async fn appointment_crud_roundtrip_and_denormalized_names() {
    let (app, op) = crm_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let appt = make_appointment(&app, &op, &client_id, &pet_id, 1_800_000_000, "Full groom").await;

    let (s, b) = call(&app, "GET", &format!("/appointments/{appt}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["clientId"], client_id.as_str());
    assert_eq!(b["clientName"], "Alice Tan", "the list view renders without a client join");
    assert_eq!(b["petName"], "Rex");
    assert_eq!(b["status"], "scheduled", "a new appointment defaults to scheduled");
    assert_eq!(
        b["endAt"].as_u64().unwrap(),
        1_800_000_000 + 3600,
        "a missing endAt defaults to a one-hour slot"
    );
    assert!(b["verifications"].as_array().unwrap().is_empty());

    // update: move it and mark it confirmed
    let (s, b) = call(
        &app,
        "PUT",
        &format!("/appointments/{appt}"),
        Some(&op),
        Some(json!({
            "clientId": client_id,
            "petId": pet_id,
            "service": "Bath & brush",
            "startAt": 1_800_003_600u64,
            "endAt": 1_800_009_000u64,
            "status": "confirmed",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "update appointment: {b}");
    assert_eq!(b["status"], "confirmed");
    assert_eq!(b["startAt"], 1_800_003_600u64);
    assert_eq!(b["endAt"], 1_800_009_000u64);

    let (s, _) = call(&app, "DELETE", &format!("/appointments/{appt}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&app, "GET", &format!("/appointments/{appt}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn appointment_create_validates_its_client_pet_slot_and_status() {
    let (app, op) = crm_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let (other_client, other_pet) = make_client(&app, &op, "Bob Lim", "Biscuit").await;

    let cases: Vec<(Value, &str)> = vec![
        (
            json!({ "clientId": "no-such-client", "startAt": 1_800_000_000u64 }),
            "an unknown clientId",
        ),
        (
            json!({ "clientId": client_id, "petId": other_pet, "startAt": 1_800_000_000u64 }),
            "a pet belonging to a DIFFERENT client",
        ),
        (json!({ "clientId": client_id, "startAt": 0u64 }), "a missing startAt"),
        (
            json!({ "clientId": client_id, "startAt": 1_800_000_000u64, "endAt": 1_700_000_000u64 }),
            "an endAt before startAt",
        ),
        (
            json!({ "clientId": client_id, "startAt": 1_800_000_000u64, "status": "sparkling" }),
            "an unknown status",
        ),
    ];
    for (body, what) in cases {
        let (s, b) = call(&app, "POST", "/appointments", Some(&op), Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{what} must be rejected: {b}");
    }
    // sanity: the valid combination still works
    let _ = make_appointment(&app, &op, &other_client, &other_pet, 1_800_000_000, "Full groom").await;
    let _ = pet_id;
}

#[tokio::test]
async fn appointment_list_filters_by_client_status_and_calendar_window() {
    let (app, op) = crm_app().await;
    let (alice, alice_pet) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let (bob, bob_pet) = make_client(&app, &op, "Bob Lim", "Biscuit").await;

    // three days of bookings (86400s apart)
    let day0 = 1_800_000_000u64;
    let a0 = make_appointment(&app, &op, &alice, &alice_pet, day0, "Full groom").await;
    let a1 = make_appointment(&app, &op, &alice, &alice_pet, day0 + 86_400, "Nail trim").await;
    let b0 = make_appointment(&app, &op, &bob, &bob_pet, day0 + 172_800, "Full groom").await;

    // by client
    let (s, b) = call(&app, "GET", &format!("/appointments?clientId={alice}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["total"], 2);

    // calendar order is earliest-first
    assert_eq!(b["rows"][0]["appointmentId"], a0.as_str());
    assert_eq!(b["rows"][1]["appointmentId"], a1.as_str());

    // day window [day0, day0+86400) — half-open, so the next day's booking is excluded
    let (_, b) = call(
        &app,
        "GET",
        &format!("/appointments?from={}&to={}", day0, day0 + 86_400),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(b["total"], 1, "a half-open day window holds exactly one booking");
    assert_eq!(b["rows"][0]["appointmentId"], a0.as_str());

    // week window covers all three
    let (_, b) = call(
        &app,
        "GET",
        &format!("/appointments?from={}&to={}", day0, day0 + 604_800),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(b["total"], 3);

    // by status
    let (s, _) = call(
        &app,
        "PUT",
        &format!("/appointments/{b0}"),
        Some(&op),
        Some(json!({ "clientId": bob, "petId": bob_pet, "startAt": day0 + 172_800, "status": "completed" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, b) = call(&app, "GET", "/appointments?status=completed", Some(&op), None).await;
    assert_eq!(b["total"], 1);
    assert_eq!(b["rows"][0]["appointmentId"], b0.as_str());

    // by free text over service / owner / pet
    let (_, b) = call(&app, "GET", "/appointments?q=nail", Some(&op), None).await;
    assert_eq!(b["total"], 1);
    assert_eq!(b["rows"][0]["appointmentId"], a1.as_str());

    // an unknown status filter is a request error, not a silently empty list
    let (s, _) = call(&app, "GET", "/appointments?status=sparkling", Some(&op), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// ============================================================================================
// verification <-> appointment linkage
// ============================================================================================

/// Start a verification session, optionally FROM an appointment.
async fn start_verify_session(
    app: &axum::Router,
    op: &str,
    appointment_id: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({ "purpose": "grooming_intake", "recordType": "VACCINATION" });
    if let Some(id) = appointment_id {
        body.as_object_mut().unwrap().insert("appointmentId".into(), json!(id));
    }
    call(app, "POST", "/verify/session/start", Some(op), Some(body)).await
}

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";

/// A booted app whose custody is unlocked and whose relayer is whitelisted for the purposes these
/// tests verify under, so `/verify/session/start` passes its `VERIFY:<purpose>` preflight.
async fn verify_app() -> (axum::Router, String) {
    let chain = Arc::new(MemChain::new());
    let state = state_with(
        chain.clone(),
        "memchain".to_string(),
        REGISTRY.to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let (_admin, op, relayer) = boot_custody(&app).await;
    for purpose in ["grooming_intake", "boarding_intake"] {
        chain.whitelist(REGISTRY, &vet_api::verify::verify_key(purpose), &relayer);
    }
    (app, op)
}

#[tokio::test]
async fn starting_a_verification_from_an_appointment_opens_a_linked_history_row() {
    let (app, op) = verify_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let appt = make_appointment(&app, &op, &client_id, &pet_id, 1_800_000_000, "Full groom").await;

    let (s, b) = start_verify_session(&app, &op, Some(&appt)).await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    let session_id = b["sessionId"].as_str().unwrap().to_string();

    // The row exists BEFORE the owner consents, so an in-flight verification is visible.
    let (s, v) = call(&app, "GET", &format!("/verifications/{session_id}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "verification row must exist at session start: {v}");
    assert_eq!(v["status"], "pending");
    assert_eq!(v["appointmentId"], appt.as_str());
    assert_eq!(v["clientId"], client_id.as_str());
    assert_eq!(v["petId"], pet_id.as_str());
    assert_eq!(v["clientName"], "Alice Tan", "the row carries the business context");
    assert_eq!(v["petName"], "Rex");
    assert_eq!(v["purpose"], "grooming_intake");
    assert!(
        v["disclosedKeyPaths"].as_array().unwrap().is_empty(),
        "nothing disclosed yet"
    );

    // and it is reachable from the appointment detail
    let (_, a) = call(&app, "GET", &format!("/appointments/{appt}"), Some(&op), None).await;
    assert_eq!(a["verifications"].as_array().unwrap().len(), 1);
    assert_eq!(a["verifications"][0]["verificationId"], session_id.as_str());
}

#[tokio::test]
async fn an_ad_hoc_verification_still_works_and_is_recorded_unlinked() {
    let (app, op) = verify_app().await;
    let (s, b) = start_verify_session(&app, &op, None).await;
    assert_eq!(s, StatusCode::OK, "the ad-hoc path must keep working: {b}");
    let session_id = b["sessionId"].as_str().unwrap();

    let (s, v) = call(&app, "GET", &format!("/verifications/{session_id}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["appointmentId"].is_null(), "an ad-hoc verification has no appointment");
    assert!(v["clientId"].is_null());
    assert_eq!(v["status"], "pending");
}

#[tokio::test]
async fn starting_a_verification_from_an_unknown_appointment_is_rejected() {
    let (app, op) = verify_app().await;
    let (s, b) = start_verify_session(&app, &op, Some("no-such-appointment")).await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "an unresolvable appointmentId must fail loudly, not silently unlink: {b}"
    );
}

#[tokio::test]
async fn verification_history_filters_by_client_appointment_purpose_and_status() {
    let (app, op) = verify_app().await;

    let (alice, alice_pet) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let (bob, bob_pet) = make_client(&app, &op, "Bob Lim", "Biscuit").await;
    let a_appt = make_appointment(&app, &op, &alice, &alice_pet, 1_800_000_000, "Full groom").await;
    let b_appt = make_appointment(&app, &op, &bob, &bob_pet, 1_800_086_400, "Nail trim").await;

    let (_, v1) = start_verify_session(&app, &op, Some(&a_appt)).await;
    let (_, v2) = start_verify_session(&app, &op, Some(&a_appt)).await;
    let (_, v3) = start_verify_session(&app, &op, Some(&b_appt)).await;
    let v1_id = v1["sessionId"].as_str().unwrap();
    let v3_id = v3["sessionId"].as_str().unwrap();
    let _ = v2;

    // by client — Alice has two, Bob one
    let (s, b) = call(&app, "GET", &format!("/verifications?clientId={alice}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["total"], 2, "filtering by client must find both of Alice's verifications");

    let (_, b) = call(&app, "GET", &format!("/verifications?clientId={bob}"), Some(&op), None).await;
    assert_eq!(b["total"], 1);
    assert_eq!(b["rows"][0]["verificationId"], v3_id);

    // by appointment
    let (_, b) = call(
        &app,
        "GET",
        &format!("/verifications?appointmentId={b_appt}"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(b["total"], 1);
    assert_eq!(b["rows"][0]["appointmentId"], b_appt.as_str());

    // by purpose + status
    let (_, b) = call(
        &app,
        "GET",
        "/verifications?purpose=grooming_intake&status=pending",
        Some(&op),
        None,
    )
    .await;
    assert!(b["total"].as_u64().unwrap() >= 3);

    // by free text — the owner's name is part of the row's search key
    let (_, b) = call(&app, "GET", "/verifications?q=alice", Some(&op), None).await;
    assert_eq!(b["total"], 2);

    // an unknown status filter is a request error, not a silently empty history — the operator
    // cannot tell a typo'd filter apart from a shop that has verified nothing.
    let (s, _) = call(&app, "GET", "/verifications?status=recordedd", Some(&op), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // unfiltered still lists everything
    let (_, b) = call(&app, "GET", "/verifications", Some(&op), None).await;
    assert_eq!(b["total"], 3, "unfiltered lists every verification this shop performed");
    let _ = v1_id;
}

#[tokio::test]
async fn verification_history_filters_by_pet_not_just_by_owner() {
    // `?petId=` was parsed off the query string and then never applied, so it returned the UNFILTERED
    // history. That is worse than an ignored filter: a caller asking for one pet's checks got every
    // pet's, and a pet page rendering them would state that THIS pet was verified about verifications
    // belonging to its sibling. One owner with two pets is the case that catches it — a `clientId`
    // filter cannot tell the two apart.
    let (app, op) = verify_app().await;
    let (s, c) = call(
        &app,
        "POST",
        "/clients",
        Some(&op),
        Some(json!({
            "name": "Alice Tan",
            "pets": [{ "name": "Rex" }, { "name": "Milo" }],
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let client_id = c["clientId"].as_str().unwrap().to_string();
    let rex = c["pets"][0]["petId"].as_str().unwrap().to_string();
    let milo = c["pets"][1]["petId"].as_str().unwrap().to_string();

    let rex_appt = make_appointment(&app, &op, &client_id, &rex, 1_800_000_000, "Full groom").await;
    let milo_appt = make_appointment(&app, &op, &client_id, &milo, 1_800_086_400, "Bath").await;
    let (_, v_rex) = start_verify_session(&app, &op, Some(&rex_appt)).await;
    let (_, v_milo) = start_verify_session(&app, &op, Some(&milo_appt)).await;

    // Both belong to the same owner, so the owner filter cannot separate them.
    let (_, b) = call(&app, "GET", &format!("/verifications?clientId={client_id}"), Some(&op), None).await;
    assert_eq!(b["total"], 2, "both checks are this owner's: {b}");

    let (s, b) = call(&app, "GET", &format!("/verifications?petId={rex}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["total"], 1, "?petId= must narrow to ONE pet's checks: {b}");
    assert_eq!(b["rows"][0]["verificationId"], v_rex["sessionId"].as_str().unwrap());
    assert_eq!(b["rows"][0]["petId"], rex.as_str());

    let (_, b) = call(&app, "GET", &format!("/verifications?petId={milo}"), Some(&op), None).await;
    assert_eq!(b["total"], 1, "{b}");
    assert_eq!(b["rows"][0]["verificationId"], v_milo["sessionId"].as_str().unwrap());

    // A pet with no checks must come back EMPTY rather than falling back to the whole history.
    let (_, b) = call(&app, "GET", "/verifications?petId=no-such-pet", Some(&op), None).await;
    assert_eq!(b["total"], 0, "an unmatched petId must not silently list everything: {b}");

    // A blank filter is still no filter, as everywhere else.
    let (_, b) = call(&app, "GET", "/verifications?petId=", Some(&op), None).await;
    assert_eq!(b["total"], 2, "?petId= (blank) must not filter: {b}");
}

/// Rename a client, echoing their one pet under a new name.
async fn rename_to_alice_lim(app: &axum::Router, op: &str, client_id: &str, pet_id: &str) {
    let (s, b) = call(
        app,
        "PUT",
        &format!("/clients/{client_id}"),
        Some(op),
        Some(json!({ "name": "Alice Lim", "pets": [{ "petId": pet_id, "name": "Rexy" }] })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "rename client: {b}");
}

/// A whitelisted MemChain app that can actually carry a consent submit through to `recorded`, so the
/// terminal-settle seam (`crm::finish_log`) is exercised end to end and not just at session start.
///
/// `MemChain` does not verify Groth16, so `a/b/c` are placeholders and the public signals do the
/// talking — exactly the surface these tests are about (see `submit_consent_levelb_route.rs`).
async fn recordable_app() -> (axum::Router, String, String) {
    let chain = Arc::new(MemChain::new());
    let state = state_with(
        chain.clone(),
        "memchain".to_string(),
        REGISTRY.to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let (_admin, op, relayer) = boot_custody(&app).await;
    chain.whitelist(
        REGISTRY,
        &vet_api::verify::verify_key("grooming_intake"),
        &relayer,
    );
    (app, op, relayer)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn u256_dec_of_addr(a: &str) -> String {
    use alloy::primitives::U256;
    U256::from_str_radix(a.trim_start_matches("0x"), 16)
        .expect("addr hex")
        .to_string()
}

/// The 0x-hex32 word a decimal public signal is recorded as in the history row.
fn word_of_dec(dec: &str) -> String {
    use alloy::primitives::U256;
    let u = U256::from_str_radix(dec, 10).expect("dec");
    format!("0x{}", hex::encode(u.to_be_bytes::<32>()))
}

const PUB_DOG_TAG_ID: &str = "424242";
const PUB_NULLIFIER: &str = "1111111111111111111111";

/// A well-formed owner-hidden consent vector for the session `start_verify_session` opens, built
/// through the NAMED indices so a reordering of the circuit's outputs moves this fixture rather than
/// silently producing a vector that means something else.
fn consent_pubs(relayer: &str) -> [String; 7] {
    use dogtag_standard::public_signals::level_b as PB;
    let mut p: [String; 7] = std::array::from_fn(|_| "0".to_string());
    p[PB::DOG_TAG_ID] = PUB_DOG_TAG_ID.to_string();
    p[PB::PURPOSE] = dec_field(&vet_api::verify::purpose_key("grooming_intake"));
    p[PB::RELAYER] = u256_dec_of_addr(relayer);
    p[PB::NULLIFIER] = PUB_NULLIFIER.to_string();
    p[PB::ROOT] = "2222222222222222222222".to_string();
    p[PB::RECORD_TYPE] = dec_field(&vet_api::verify::purpose_key("VACCINATION"));
    // comfortably past the handler's MIN_DEADLINE_MARGIN_SECS preflight floor
    p[PB::DEADLINE] = (now_secs() + 900).to_string();
    p
}

/// `purpose_key` returns a 0x-hex word; the proof carries decimal field elements.
fn dec_field(word_hex: &str) -> String {
    use alloy::primitives::U256;
    U256::from_str_radix(word_hex.trim_start_matches("0x"), 16)
        .expect("field word")
        .to_string()
}

/// Submit an owner-hidden proof for `session_id` and poll the audit row to its TERMINAL state.
///
/// The handler acks `"recording"` and broadcasts from a detached `tokio::spawn`, so the ack says
/// nothing about the outcome — every assertion must come from the settled row, or it passes
/// vacuously.
async fn submit_and_settle(app: &axum::Router, op: &str, session_id: &str, relayer: &str) -> Value {
    let (s, ack) = call(
        app,
        "POST",
        "/v1/verify/consent",
        Some(op),
        Some(json!({
            "sessionId": session_id,
            "proof": {
                "a": ["1", "2"],
                "b": [["3", "4"], ["5", "6"]],
                "c": ["7", "8"],
                "pubSignals": consent_pubs(relayer),
            }
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "consent submit: {ack}");
    assert_eq!(ack["status"], "recording", "the ack is not terminal: {ack}");

    for _ in 0..200 {
        let (ss, row) = call(
            app,
            "GET",
            &format!("/verify/session/{session_id}"),
            Some(op),
            None,
        )
        .await;
        assert_eq!(ss, StatusCode::OK, "session lookup: {row}");
        if row["status"] != "recording" {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("session {session_id} never left recording");
}

/// Poll the SHOP's history row until the verify leg has settled it.
///
/// The session settles one write before the history row does (`crm::finish_log` runs after
/// `update_session` in the same detached task), so a test that means to act on a TERMINAL row has to
/// wait for the row itself — waiting on the session would leave it racing that last write, and would
/// silently exercise the in-flight path instead.
async fn settled_history_row(app: &axum::Router, op: &str, verification_id: &str) -> Value {
    for _ in 0..200 {
        let (s, v) = call(
            app,
            "GET",
            &format!("/verifications/{verification_id}"),
            Some(op),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "verification lookup: {v}");
        if v["status"] != "pending" && v["status"] != "recording" {
            return v;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("verification {verification_id} never settled");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn renaming_a_client_refreshes_the_denormalized_name_on_their_verification_history() {
    let (app, op, relayer) = recordable_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let appt = make_appointment(&app, &op, &client_id, &pet_id, 1_800_000_000, "Full groom").await;
    let (s, b) = start_verify_session(&app, &op, Some(&appt)).await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    let verification_id = b["sessionId"].as_str().unwrap().to_string();

    // Settle FIRST: a terminal row is the one a rename is allowed to rewrite at all.
    submit_and_settle(&app, &op, &verification_id, &relayer).await;
    let settled = settled_history_row(&app, &op, &verification_id).await;
    assert_eq!(settled["status"], "recorded", "the row must settle: {settled}");
    let tx = settled["txHash"].as_str().unwrap().to_string();

    rename_to_alice_lim(&app, &op, &client_id, &pet_id).await;

    let (_, v) = call(
        &app,
        "GET",
        &format!("/verifications/{verification_id}"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(
        v["clientName"], "Alice Lim",
        "the history must not keep showing the pre-rename label"
    );
    assert_eq!(v["petName"], "Rexy");
    // …and the rename must cost the row none of its permanent evidence
    assert_eq!(v["status"], "recorded", "a rename must not disturb the settled outcome");
    assert_eq!(v["txHash"].as_str().unwrap(), tx, "the tx evidence must survive a rename");
    assert_eq!(v["nullifier"], word_of_dec(PUB_NULLIFIER));

    // and the row is findable under the new name — the search key was rebuilt too
    let (_, b) = call(&app, "GET", "/verifications?q=alice%20lim", Some(&op), None).await;
    assert_eq!(b["total"], 1, "the verification search key must be rebuilt on rename");
    assert_eq!(b["rows"][0]["verificationId"], verification_id.as_str());

    let (_, b) = call(&app, "GET", "/verifications?q=alice%20tan", Some(&op), None).await;
    assert_eq!(b["total"], 0, "the stale name must no longer match");
}

/// The other half of the writer split: an IN-FLIGHT row is the verify leg's alone, so a concurrent
/// rename must not touch it — and the settle must then adopt the new name itself, or the rename
/// would simply be lost for that verification.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rename_during_an_in_flight_verification_lands_when_the_row_settles() {
    let (app, op, relayer) = recordable_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let appt = make_appointment(&app, &op, &client_id, &pet_id, 1_800_000_000, "Full groom").await;
    let (s, b) = start_verify_session(&app, &op, Some(&appt)).await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    let verification_id = b["sessionId"].as_str().unwrap().to_string();

    rename_to_alice_lim(&app, &op, &client_id, &pet_id).await;

    let (_, v) = call(
        &app,
        "GET",
        &format!("/verifications/{verification_id}"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(v["status"], "pending", "the row must still be in flight: {v}");
    assert_eq!(
        v["clientName"], "Alice Tan",
        "an in-flight row belongs to the verify leg — a rename must not write it"
    );

    submit_and_settle(&app, &op, &verification_id, &relayer).await;
    let settled = settled_history_row(&app, &op, &verification_id).await;
    assert_eq!(settled["status"], "recorded", "the row must settle: {settled}");
    assert_eq!(
        settled["clientName"], "Alice Lim",
        "the settle must adopt the rename that landed mid-flight"
    );
    assert_eq!(settled["petName"], "Rexy");
    // the handoff must not have cost the settle its evidence
    let tx = settled["txHash"].as_str().unwrap_or_default();
    assert!(tx.starts_with("0x"), "the settled row keeps its tx: {settled}");
    assert_eq!(settled["nullifier"], word_of_dec(PUB_NULLIFIER));

    let (_, b) = call(&app, "GET", "/verifications?q=alice%20lim", Some(&op), None).await;
    assert_eq!(b["total"], 1, "the settled row is findable under the new name");
    assert_eq!(b["rows"][0]["verificationId"], verification_id.as_str());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_completed_verification_settles_the_history_row_with_its_evidence() {
    let (app, op, relayer) = recordable_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let appt = make_appointment(&app, &op, &client_id, &pet_id, 1_800_000_000, "Full groom").await;

    let (s, b) = start_verify_session(&app, &op, Some(&appt)).await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    let session_id = b["sessionId"].as_str().unwrap().to_string();

    let session = submit_and_settle(&app, &op, &session_id, &relayer).await;
    assert_eq!(session["status"], "recorded", "the session must settle: {session}");
    let tx = session["txHash"].as_str().unwrap().to_string();

    // the HISTORY row must now agree with the session, and carry the on-chain evidence
    let (s, v) = call(
        &app,
        "GET",
        &format!("/verifications/{session_id}"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "recorded", "the row must settle when the session does");
    assert_eq!(v["txHash"].as_str().unwrap(), tx, "the tx is recorded as evidence");
    assert_eq!(
        v["nullifier"],
        word_of_dec(PUB_NULLIFIER),
        "the consumed nullifier is recorded as evidence"
    );
    assert_eq!(
        v["dogTagId"],
        word_of_dec(PUB_DOG_TAG_ID),
        "the opaque dogTagId is recorded as evidence"
    );
    // and the linkage survived the settle
    assert_eq!(v["appointmentId"], appt.as_str());
    assert_eq!(v["clientId"], client_id.as_str());

    // it is now findable by status, and still by client + appointment
    let (_, b) = call(&app, "GET", "/verifications?status=recorded", Some(&op), None).await;
    assert_eq!(b["total"], 1, "a recorded verification is findable by status");
    let (_, b) = call(
        &app,
        "GET",
        &format!("/verifications?clientId={client_id}&status=recorded"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(b["total"], 1, "…and by client + status together");
    let (_, b) = call(
        &app,
        "GET",
        &format!("/verifications?appointmentId={appt}&status=recorded"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(b["total"], 1, "…and by appointment + status together");

    // the tx hash is searchable free-text, so an operator can go from a chain explorer back to the visit
    let (_, b) = call(&app, "GET", &format!("/verifications?q={tx}"), Some(&op), None).await;
    assert_eq!(b["total"], 1, "the tx hash must be searchable");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_owner_hidden_verification_records_no_disclosed_leaves() {
    // The privacy invariant, pinned: the owner reveals nothing, so the shop's row must hold an EMPTY
    // disclosed list — never backfilled from the proof or anywhere else.
    let (app, op, relayer) = recordable_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let appt = make_appointment(&app, &op, &client_id, &pet_id, 1_800_000_000, "Full groom").await;
    let (_, b) = start_verify_session(&app, &op, Some(&appt)).await;
    let session_id = b["sessionId"].as_str().unwrap().to_string();

    let session = submit_and_settle(&app, &op, &session_id, &relayer).await;
    assert_eq!(session["status"], "recorded", "{session}");

    let (_, v) = call(
        &app,
        "GET",
        &format!("/verifications/{session_id}"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(v["status"], "recorded");
    assert!(
        v["disclosedKeyPaths"].as_array().unwrap().is_empty(),
        "an owner-hidden verification must record NO disclosed leaves — that emptiness is the guarantee"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verification_row_never_stores_an_owner_wallet() {
    // The owner-hidden path hands the verifier no `subject` at all, and the shop's row must not
    // acquire one by any other route: persisting a wallet here would create a client -> wallet
    // linkage the protocol goes out of its way to withhold from a verifier.
    let (app, op, relayer) = recordable_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let appt = make_appointment(&app, &op, &client_id, &pet_id, 1_800_000_000, "Full groom").await;
    let (_, b) = start_verify_session(&app, &op, Some(&appt)).await;
    let session_id = b["sessionId"].as_str().unwrap().to_string();

    let session = submit_and_settle(&app, &op, &session_id, &relayer).await;
    assert_eq!(session["status"], "recorded", "{session}");

    let (_, v) = call(
        &app,
        "GET",
        &format!("/verifications/{session_id}"),
        Some(&op),
        None,
    )
    .await;
    assert!(
        v.get("subject").is_none(),
        "the row must have no owner-wallet field at all: {v}"
    );
    let serialized = serde_json::to_string(&v).unwrap().to_lowercase();
    assert!(
        !serialized.contains("0x00000000000000000000000000000000000000cc"),
        "no owner wallet may appear in the verification row: {v}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_qr_resolver_exposes_no_client_or_appointment_context() {
    // GET /x/{token} is UNAUTHENTICATED — anyone who scans (or guesses) the QR can read it. Linking a
    // verification to a client must not push the shop's customer records onto that surface.
    let (app, op, _relayer) = recordable_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let appt = make_appointment(&app, &op, &client_id, &pet_id, 1_800_000_000, "Full groom").await;
    let (s, b) = start_verify_session(&app, &op, Some(&appt)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let token = b["qrUrl"]
        .as_str()
        .unwrap()
        .split("/x/")
        .nth(1)
        .unwrap()
        .split('?')
        .next()
        .unwrap()
        .to_string();

    let (s, resolved) = call(&app, "GET", &format!("/x/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "{resolved}");
    // exactly the pre-existing key set — no more
    let mut keys: Vec<&str> = resolved.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "challenge",
            "purpose",
            "recordType",
            "relayer",
            "sessionId",
            "unverifiedClaims"
        ],
        "the QR resolver's response shape must be unchanged by the appointment linkage"
    );
    let body = serde_json::to_string(&resolved).unwrap().to_lowercase();
    for leaked in [client_id.as_str(), appt.as_str(), pet_id.as_str(), "alice", "rex"] {
        assert!(
            !body.contains(&leaked.to_lowercase()),
            "the QR resolver must not expose {leaked:?}: {resolved}"
        );
    }
}

#[tokio::test]
async fn deleting_an_appointment_that_has_verifications_is_refused() {
    let (app, op) = verify_app().await;
    let (client_id, pet_id) = make_client(&app, &op, "Alice Tan", "Rex").await;
    let appt = make_appointment(&app, &op, &client_id, &pet_id, 1_800_000_000, "Full groom").await;
    let (s, _) = start_verify_session(&app, &op, Some(&appt)).await;
    assert_eq!(s, StatusCode::OK);

    let (s, b) = call(&app, "DELETE", &format!("/appointments/{appt}"), Some(&op), None).await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "verification evidence must not be orphaned by a delete: {b}"
    );
}
