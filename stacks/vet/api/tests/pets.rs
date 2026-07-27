//! PETS as a collection of their own: `/pets` list/search/paging, `/pets/{id}` read + patch, and the
//! DogTag LINK/UNLINK that is deliberately NOT a revocation.
//!
//! What these tests protect, in order of how badly each would hurt:
//!
//!  1. A pet write must not disturb its SIBLINGS. Pets are stored embedded in the client document and
//!     `PUT /clients/{id}` replaces that array wholesale, so a pet route implemented the same way
//!     would silently delete every pet the caller did not mention.
//!  2. Paging must be over PETS, not clients — `total` counts pets, and a page boundary falls between
//!     pets — with a total order stable enough that paging neither repeats nor skips a row.
//!  3. Search must narrow to ONE pet. The client's own `searchKey` concatenates all of its pets, so
//!     reusing it would return a pet's siblings alongside it.
//!  4. Unlinking a DogTag must leave everything except this shop's own note of it untouched.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use std::sync::Arc;
use vet_api::chain::MemChain;

async fn pets_app() -> (axum::Router, String) {
    let (app, op, _) = pets_app_with_state().await;
    (app, op)
}

/// Same app, plus the `AppState` - needed by the tests that have to seed the per-tag document cache
/// directly, since a groomer/vet test has no live customer wallet to run `POST /import/pull` against.
async fn pets_app_with_state() -> (axum::Router, String, vet_api::app::AppState) {
    let state = state_with(
        Arc::new(MemChain::new()),
        "memchain".to_string(),
        "0x00000000000000000000000000000000000000aa".to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
        "vet.example".to_string(),
        1,
    );
    let op = mint_operator(&state).await;
    (vet_api::router(state.clone()), op, state)
}

/// Create a client carrying `pets` verbatim; returns the clientId and the minted petIds in order.
async fn make_client_with_pets(
    app: &axum::Router,
    op: &str,
    name: &str,
    pets: Value,
) -> (String, Vec<String>) {
    let (s, b) = call(
        app,
        "POST",
        "/clients",
        Some(op),
        Some(json!({ "name": name, "phone": "+65 9123 4567", "pets": pets })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "create client: {b}");
    let ids = b["pets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["petId"].as_str().unwrap().to_string())
        .collect();
    (b["clientId"].as_str().unwrap().to_string(), ids)
}

async fn get_pet(app: &axum::Router, op: &str, pet_id: &str) -> Value {
    let (s, b) = call(app, "GET", &format!("/pets/{pet_id}"), Some(op), None).await;
    assert_eq!(s, StatusCode::OK, "get pet: {b}");
    b
}

// ============================================================================================
// auth
// ============================================================================================

#[tokio::test]
async fn every_pet_route_requires_an_operator_session() {
    let (app, op) = pets_app().await;
    let (_, pets) = make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;
    let pet = &pets[0];

    for (method, path, body) in [
        ("GET", "/pets".to_string(), None),
        ("POST", "/pets".to_string(), Some(json!({ "clientId": "x", "name": "Rex" }))),
        ("GET", format!("/pets/{pet}"), None),
        ("PUT", format!("/pets/{pet}"), Some(json!({ "name": "Rex" }))),
        ("DELETE", format!("/pets/{pet}"), None),
        ("POST", format!("/pets/{pet}/dogtag"), Some(json!({ "dogTagId": "4" }))),
        ("DELETE", format!("/pets/{pet}/dogtag"), None),
        ("GET", format!("/pets/{pet}/credentials"), None),
    ] {
        let (s, b) = call(&app, method, &path, None, body).await;
        assert_eq!(
            s,
            StatusCode::UNAUTHORIZED,
            "{method} {path} must require an operator session: {b}"
        );
    }
}

// ============================================================================================
// the pet <-> owner round trip
// ============================================================================================

#[tokio::test]
async fn a_pet_is_addressable_on_its_own_and_names_its_owner() {
    let (app, op) = pets_app().await;
    let (client_id, pets) = make_client_with_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex", "species": "dog", "breed": "Standard Poodle" }]),
    )
    .await;

    // Reachable by petId ALONE — no knowledge of the owner required, which is the whole point.
    let pet = get_pet(&app, &op, &pets[0]).await;
    assert_eq!(pet["name"], "Rex");
    assert_eq!(pet["breed"], "Standard Poodle");
    // ...and it carries the owner, so the pet -> owner half of the round trip is a link, not a search.
    assert_eq!(pet["clientId"], client_id.as_str());
    assert_eq!(pet["clientName"], "Alice Tan");
}

#[tokio::test]
async fn listing_by_client_id_returns_that_owners_pets_only() {
    let (app, op) = pets_app().await;
    let (alice, _) = make_client_with_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex" }, { "name": "Milo" }]),
    )
    .await;
    make_client_with_pets(&app, &op, "Bob Lee", json!([{ "name": "Coco" }])).await;

    let (s, b) = call(&app, "GET", &format!("/pets?clientId={alice}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["total"], 2, "owner filter must count only this owner's pets: {b}");
    let names: Vec<&str> = b["rows"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Rex") && names.contains(&"Milo"), "{names:?}");
    assert!(!names.contains(&"Coco"), "another owner's pet leaked in: {names:?}");
}

#[tokio::test]
async fn a_missing_pet_is_a_404_not_an_empty_pet() {
    let (app, op) = pets_app().await;
    for (method, body) in [("GET", None), ("PUT", Some(json!({ "name": "Rex" }))), ("DELETE", None)] {
        let (s, b) = call(&app, method, "/pets/no-such-pet", Some(&op), body).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{method} /pets/no-such-pet: {b}");
    }
}

// ============================================================================================
// search + paging over PETS
// ============================================================================================

#[tokio::test]
async fn search_narrows_to_one_pet_and_does_not_drag_in_its_siblings() {
    let (app, op) = pets_app().await;
    make_client_with_pets(
        &app,
        &op,
        "Alice Tan",
        json!([
            { "name": "Rex", "breed": "Standard Poodle" },
            { "name": "Milo", "breed": "Beagle" },
        ]),
    )
    .await;

    // The CLIENT's search key concatenates both pets, so matching on it would return both. A pet query
    // must match the pet's OWN fields.
    let (s, b) = call(&app, "GET", "/pets?q=beagle", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["total"], 1, "\"beagle\" must match Milo alone: {b}");
    assert_eq!(b["rows"][0]["name"], "Milo");
}

#[tokio::test]
async fn a_pet_is_findable_by_its_own_fields_by_its_tag_and_by_its_owners_name() {
    let (app, op) = pets_app().await;
    make_client_with_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex", "species": "dog", "breed": "Standard Poodle", "dogTagId": "4" }]),
    )
    .await;

    for needle in ["rex", "poodle", "dog", "4", "alice"] {
        let (s, b) = call(&app, "GET", &format!("/pets?q={needle}"), Some(&op), None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert_eq!(b["total"], 1, "\"{needle}\" must find the pet: {b}");
        assert_eq!(b["rows"][0]["name"], "Rex", "for needle {needle}");
    }
}

#[tokio::test]
async fn every_term_must_match_so_multiple_terms_narrow() {
    let (app, op) = pets_app().await;
    make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "breed": "Poodle" }])).await;
    make_client_with_pets(&app, &op, "Bob Lee", json!([{ "name": "Rex", "breed": "Beagle" }])).await;

    let (_, b) = call(&app, "GET", "/pets?q=rex", Some(&op), None).await;
    assert_eq!(b["total"], 2, "both pets are called Rex: {b}");
    let (_, b) = call(&app, "GET", "/pets?q=rex%20beagle", Some(&op), None).await;
    assert_eq!(b["total"], 1, "adding a term must narrow, not widen: {b}");
    assert_eq!(b["rows"][0]["clientName"], "Bob Lee");
}

#[tokio::test]
async fn paging_counts_pets_and_covers_every_row_exactly_once() {
    let (app, op) = pets_app().await;
    // Two owners with three pets each: six pets across two clients, so a client-paged implementation
    // could not produce these pages.
    let pets = json!([{ "name": "P1" }, { "name": "P2" }, { "name": "P3" }]);
    make_client_with_pets(&app, &op, "Alice Tan", pets.clone()).await;
    make_client_with_pets(&app, &op, "Bob Lee", pets).await;

    let mut seen: Vec<String> = Vec::new();
    for offset in [0, 2, 4] {
        let (s, b) = call(&app, "GET", &format!("/pets?limit=2&offset={offset}"), Some(&op), None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert_eq!(b["total"], 6, "total must count PETS: {b}");
        assert_eq!(b["limit"], 2);
        assert_eq!(b["offset"], offset);
        let rows = b["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "page at offset {offset}: {b}");
        seen.extend(rows.iter().map(|r| r["petId"].as_str().unwrap().to_string()));
    }
    // The ordering must be total, or paging would repeat some pets and skip others. Six distinct pets
    // over three pages of two proves it for pets that SHARE an owner (and so share its `updatedAt`).
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 6, "paging repeated or skipped a pet: {seen:?}");
}

// ============================================================================================
// create / edit — without disturbing siblings
// ============================================================================================

#[tokio::test]
async fn a_pet_is_created_under_an_owner_and_appears_in_both_collections() {
    let (app, op) = pets_app().await;
    let (client_id, _) = make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;

    let (s, b) = call(
        &app,
        "POST",
        "/pets",
        Some(&op),
        Some(json!({ "clientId": client_id, "name": "Milo", "species": "dog", "breed": "Beagle" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "create pet: {b}");
    assert_eq!(b["name"], "Milo");
    assert_eq!(b["clientId"], client_id.as_str());
    let milo = b["petId"].as_str().unwrap().to_string();

    // Visible as a pet in its own right...
    assert_eq!(get_pet(&app, &op, &milo).await["name"], "Milo");
    // ...and on its owner's record, which is the same stored document.
    let (_, c) = call(&app, "GET", &format!("/clients/{client_id}"), Some(&op), None).await;
    let names: Vec<&str> = c["pets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Rex", "Milo"], "the existing pet must survive the addition: {c}");
}

#[tokio::test]
async fn creating_a_pet_needs_a_real_owner_and_a_name() {
    let (app, op) = pets_app().await;
    let (client_id, _) = make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;

    let (s, b) = call(
        &app,
        "POST",
        "/pets",
        Some(&op),
        Some(json!({ "clientId": "no-such-client", "name": "Milo" })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "an ownerless pet must be refused: {b}");

    let (s, b) = call(
        &app,
        "POST",
        "/pets",
        Some(&op),
        Some(json!({ "clientId": client_id, "name": "   " })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a blank name must be refused: {b}");
}

#[tokio::test]
async fn creating_a_pet_mints_a_fresh_id_even_if_the_caller_supplies_one() {
    let (app, op) = pets_app().await;
    let (alice, alice_pets) = make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;
    let (bob, _) = make_client_with_pets(&app, &op, "Bob Lee", json!([{ "name": "Coco" }])).await;

    // Echoing an EXISTING pet's id must not graft a second pet onto it: pet ids are what appointments
    // and verification rows point at, so a collision would silently re-target another owner's history.
    let (s, b) = call(
        &app,
        "POST",
        "/pets",
        Some(&op),
        Some(json!({ "clientId": bob, "petId": alice_pets[0], "name": "Impostor" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_ne!(b["petId"].as_str().unwrap(), alice_pets[0].as_str());
    // Alice's Rex is untouched and still hers.
    let rex = get_pet(&app, &op, &alice_pets[0]).await;
    assert_eq!(rex["name"], "Rex");
    assert_eq!(rex["clientId"], alice.as_str());
}

#[tokio::test]
async fn editing_one_pet_leaves_its_siblings_and_their_links_intact() {
    let (app, op) = pets_app().await;
    let (client_id, pets) = make_client_with_pets(
        &app,
        &op,
        "Alice Tan",
        json!([
            { "name": "Rex", "breed": "Standard Poodle", "dogTagId": "4" },
            { "name": "Milo", "breed": "Beagle", "dogTagId": "3" },
        ]),
    )
    .await;
    let (rex, milo) = (&pets[0], &pets[1]);

    // Book Milo in, so a lost pet would take a real booking's link with it.
    let (s, appt) = call(
        &app,
        "POST",
        "/appointments",
        Some(&op),
        Some(json!({ "clientId": client_id, "petId": milo, "service": "Bath", "startAt": 1_800_000_000u64 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{appt}");

    let (s, b) = call(
        &app,
        "PUT",
        &format!("/pets/{rex}"),
        Some(&op),
        Some(json!({ "breed": "Toy Poodle" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "patch pet: {b}");
    assert_eq!(b["breed"], "Toy Poodle");
    // An ABSENT field is left alone — the defect a whole-document replace has by construction.
    assert_eq!(b["name"], "Rex", "an unmentioned field must not be blanked: {b}");
    assert_eq!(b["dogTagId"], "4", "the tag link must survive a details edit: {b}");

    // The sibling is entirely untouched, id and all...
    let after = get_pet(&app, &op, milo).await;
    assert_eq!(after["name"], "Milo");
    assert_eq!(after["breed"], "Beagle");
    assert_eq!(after["dogTagId"], "3");
    assert_eq!(after["petId"], milo.as_str());
    // ...and its booking still resolves to it.
    let (_, list) = call(&app, "GET", &format!("/appointments?petId={milo}"), Some(&op), None).await;
    assert_eq!(list["total"], 1, "the sibling's booking link must survive: {list}");
}

#[tokio::test]
async fn a_pet_edit_cannot_blank_the_name() {
    let (app, op) = pets_app().await;
    let (_, pets) = make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;
    let (s, b) = call(
        &app,
        "PUT",
        &format!("/pets/{}", pets[0]),
        Some(&op),
        Some(json!({ "name": "  " })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert_eq!(get_pet(&app, &op, &pets[0]).await["name"], "Rex");
}

#[tokio::test]
async fn renaming_a_pet_updates_the_labels_its_bookings_carry() {
    let (app, op) = pets_app().await;
    let (client_id, pets) = make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;
    let (s, _) = call(
        &app,
        "POST",
        "/appointments",
        Some(&op),
        Some(json!({ "clientId": client_id, "petId": pets[0], "service": "Bath", "startAt": 1_800_000_000u64 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = call(
        &app,
        "PUT",
        &format!("/pets/{}", pets[0]),
        Some(&op),
        Some(json!({ "name": "Rexy" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // The calendar denormalizes petName, so a rename that did not resync would keep showing the old
    // label indefinitely.
    let (_, list) = call(&app, "GET", &format!("/appointments?petId={}", pets[0]), Some(&op), None).await;
    assert_eq!(list["rows"][0]["petName"], "Rexy", "{list}");
}

// ============================================================================================
// DogTag: LINK and UNLINK — and what unlink is NOT
// ============================================================================================

#[tokio::test]
async fn linking_a_dogtag_records_it_and_makes_the_pet_findable_by_it() {
    let (app, op) = pets_app().await;
    let (_, pets) = make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;

    let (s, b) = call(
        &app,
        "POST",
        &format!("/pets/{}/dogtag", pets[0]),
        Some(&op),
        Some(json!({ "dogTagId": "  4  " })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "link tag: {b}");
    assert_eq!(b["dogTagId"], "4", "the id must be stored trimmed: {b}");

    let (_, found) = call(&app, "GET", "/pets?q=4", Some(&op), None).await;
    assert_eq!(found["total"], 1, "a linked tag must be searchable: {found}");
}

#[tokio::test]
async fn linking_requires_an_actual_tag_id() {
    let (app, op) = pets_app().await;
    let (_, pets) = make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;
    let (s, b) = call(
        &app,
        "POST",
        &format!("/pets/{}/dogtag", pets[0]),
        Some(&op),
        Some(json!({ "dogTagId": "   " })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
}

#[tokio::test]
async fn a_tag_already_held_by_another_pet_is_refused_and_the_holder_is_named() {
    let (app, op) = pets_app().await;
    let (_, alice) = make_client_with_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex", "dogTagId": "4" }]),
    )
    .await;
    let (_, bob) = make_client_with_pets(&app, &op, "Bob Lim", json!([{ "name": "Milo" }])).await;

    // A mistyped digit is the realistic way in, and letting it through would silently merge two
    // animals' histories: both pets would show the same held credential, the same on-chain history,
    // and both would answer `?q=4`.
    let (s, b) = call(
        &app,
        "POST",
        &format!("/pets/{}/dogtag", bob[0]),
        Some(&op),
        Some(json!({ "dogTagId": "4" })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    let msg = b["error"].as_str().unwrap_or_default();
    // Naming the holder is what tells a typo apart from a genuine conflict.
    assert!(msg.contains("Rex"), "the conflicting pet must be named: {b}");
    assert!(msg.contains("Alice Tan"), "its owner must be named too: {b}");

    // The refusal must have written NOTHING - not to the target, not to the holder.
    assert!(get_pet(&app, &op, &bob[0]).await["dogTagId"].is_null(), "{b}");
    assert_eq!(get_pet(&app, &op, &alice[0]).await["dogTagId"], "4");
}

#[tokio::test]
async fn creating_a_pet_cannot_seat_a_tag_another_pet_already_holds() {
    let (app, op) = pets_app().await;
    let (_, _) = make_client_with_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex", "dogTagId": "4" }]),
    )
    .await;
    let (bob, _) = make_client_with_pets(&app, &op, "Bob Lim", json!([])).await;

    // `POST /pets` carries `dogTagId` too, so it is a second way to attach a tag and has to enforce
    // the same rule - guarding only the link route would leave the merge reachable one route over.
    let (s, b) = call(
        &app,
        "POST",
        "/pets",
        Some(&op),
        Some(json!({ "clientId": bob, "name": "Milo", "dogTagId": "4" })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(b["error"].as_str().unwrap_or_default().contains("Rex"), "{b}");

    // ...and the pet must not have been created as a side effect of the refusal.
    let (_, found) = call(&app, "GET", &format!("/pets?clientId={bob}"), Some(&op), None).await;
    assert_eq!(found["total"], 0, "the refused pet must not exist: {found}");
}

#[tokio::test]
async fn creating_a_pet_with_a_free_tag_still_works() {
    let (app, op) = pets_app().await;
    let (alice, _) = make_client_with_pets(&app, &op, "Alice Tan", json!([])).await;

    let (s, b) = call(
        &app,
        "POST",
        "/pets",
        Some(&op),
        Some(json!({ "clientId": alice, "name": "Rex", "dogTagId": "4" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["dogTagId"], "4");
}

#[tokio::test]
async fn relinking_the_same_tag_to_the_same_pet_stays_idempotent() {
    let (app, op) = pets_app().await;
    let (_, pets) =
        make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "dogTagId": "4" }])).await;

    // Nothing is being merged, so this is not a conflict - re-sending the tag a pet already holds
    // must succeed rather than 409 an operator out of a no-op.
    let (s, b) = call(
        &app,
        "POST",
        &format!("/pets/{}/dogtag", pets[0]),
        Some(&op),
        Some(json!({ "dogTagId": "4" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["dogTagId"], "4");
}

#[tokio::test]
async fn a_conflicting_tag_on_an_unknown_pet_is_still_a_404() {
    let (app, op) = pets_app().await;
    make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "dogTagId": "4" }])).await;

    // The pet does not exist, so the honest answer is "no such pet" - not a conflict with a pet this
    // request was never about.
    let (s, b) = call(
        &app,
        "POST",
        "/pets/nope/dogtag",
        Some(&op),
        Some(json!({ "dogTagId": "4" })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{b}");
}

#[tokio::test]
async fn unlinking_a_dogtag_clears_only_this_shops_note_of_it() {
    let (app, op) = pets_app().await;
    let (client_id, pets) = make_client_with_pets(
        &app,
        &op,
        "Alice Tan",
        json!([
            { "name": "Rex", "breed": "Standard Poodle", "dogTagId": "4", "notes": "nervous" },
            { "name": "Milo", "dogTagId": "3" },
        ]),
    )
    .await;

    let (s, b) = call(&app, "DELETE", &format!("/pets/{}/dogtag", pets[0]), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "unlink: {b}");
    assert!(b["dogTagId"].is_null(), "the tag note must be cleared: {b}");

    // UNLINK IS NOT DELETE, and it is not revocation either. Everything else about the pet survives...
    assert_eq!(b["name"], "Rex");
    assert_eq!(b["breed"], "Standard Poodle");
    assert_eq!(b["notes"], "nervous");
    // ...the pet is still its owner's...
    assert_eq!(b["clientId"], client_id.as_str());
    // ...the SIBLING's tag is untouched...
    assert_eq!(get_pet(&app, &op, &pets[1]).await["dogTagId"], "3");
    // ...and re-linking restores it, which is what makes this a reversible local edit rather than a
    // permanent, public act.
    let (s, b) = call(
        &app,
        "POST",
        &format!("/pets/{}/dogtag", pets[0]),
        Some(&op),
        Some(json!({ "dogTagId": "4" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["dogTagId"], "4");
}

// The counterpart guard — that a groomer keeps `/pets` while having no `/records/{id}/revoke` to
// confuse unlink with — lives in `role_gating.rs`, next to the route lists it asserts against.

// ============================================================================================
// credentials held for the pet's tag
// ============================================================================================

#[tokio::test]
async fn a_pet_with_no_tag_reports_no_tag_rather_than_no_credentials() {
    let (app, op) = pets_app().await;
    let (_, pets) = make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;

    let (s, b) = call(&app, "GET", &format!("/pets/{}/credentials", pets[0]), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    // The distinction matters: "no tag to look up" is not evidence that the pet has no credentials.
    assert!(b["dogTagId"].is_null(), "{b}");
    assert_eq!(b["credentials"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_tagged_pet_with_nothing_imported_returns_an_empty_list_under_its_tag() {
    let (app, op) = pets_app().await;
    let (_, pets) =
        make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "dogTagId": "4" }])).await;

    let (s, b) = call(&app, "GET", &format!("/pets/{}/credentials", pets[0]), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["dogTagId"], "4", "the tag the lookup used must be named: {b}");
    assert_eq!(b["credentials"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_pet_linked_by_its_handle_finds_the_document_imported_under_that_handle() {
    let (app, op, state) = pets_app_with_state().await;
    let (_, pets) =
        make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "dogTagId": "4" }])).await;
    // What `POST /import/pull` writes: keyed by the doc's own `credentialSubject.dogTagId` leaf,
    // which is the short operator-facing HANDLE.
    state
        .store
        .upsert_client_cache("4".to_string(), json!({ "version": "dogtag/1.0" }))
        .await;

    let (s, b) = call(&app, "GET", &format!("/pets/{}/credentials", pets[0]), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["credentials"].as_array().unwrap().len(), 1, "{b}");
}

#[tokio::test]
async fn a_pet_linked_by_its_onchain_field_element_cannot_match_a_handle_keyed_document() {
    let (app, op, state) = pets_app_with_state().await;
    // The link route deliberately accepts BOTH forms, because linking from an explorer-visible
    // on-chain id is genuinely useful and chain discovery resolves either one.
    let field = "1195241908933892557940129631300775214454584041594363078565480038450625444405";
    let (_, pets) = make_client_with_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex", "dogTagId": field }]),
    )
    .await;
    state
        .store
        .upsert_client_cache("3".to_string(), json!({ "version": "dogtag/1.0" }))
        .await;

    let (s, b) = call(&app, "GET", &format!("/pets/{}/credentials", pets[0]), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    // The cache is keyed by the HANDLE and handle -> field element is a Poseidon hash, so there is no
    // way back: this lookup cannot be performed, which is NOT the same as holding nothing. The route
    // names the tag it looked under so the caller can tell the two apart and say so - the frontend
    // renders this case explicitly instead of "this shop holds no credential".
    assert_eq!(b["dogTagId"], field, "the tag the lookup used must be named: {b}");
    assert_eq!(b["credentials"].as_array().unwrap().len(), 0, "{b}");
}

// ============================================================================================
// delete
// ============================================================================================

#[tokio::test]
async fn deleting_a_pet_that_has_bookings_is_refused() {
    let (app, op) = pets_app().await;
    let (client_id, pets) = make_client_with_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;
    let (s, _) = call(
        &app,
        "POST",
        "/appointments",
        Some(&op),
        Some(json!({ "clientId": client_id, "petId": pets[0], "service": "Bath", "startAt": 1_800_000_000u64 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, b) = call(&app, "DELETE", &format!("/pets/{}", pets[0]), Some(&op), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "a booking must not be orphaned: {b}");
    // Still there, and still bookable.
    assert_eq!(get_pet(&app, &op, &pets[0]).await["name"], "Rex");
}

#[tokio::test]
async fn deleting_a_free_pet_removes_it_and_leaves_its_siblings() {
    let (app, op) = pets_app().await;
    let (client_id, pets) = make_client_with_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex" }, { "name": "Milo" }]),
    )
    .await;

    let (s, b) = call(&app, "DELETE", &format!("/pets/{}", pets[0]), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["deleted"], true);

    let (s, _) = call(&app, "GET", &format!("/pets/{}", pets[0]), Some(&op), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(get_pet(&app, &op, &pets[1]).await["name"], "Milo");

    let (_, c) = call(&app, "GET", &format!("/clients/{client_id}"), Some(&op), None).await;
    assert_eq!(c["pets"].as_array().unwrap().len(), 1, "{c}");
}
