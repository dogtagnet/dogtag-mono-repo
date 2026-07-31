//! The microchip cross-check, end to end over the real router: does the credential being attached
//! describe THIS animal?
//!
//! A tag↔pet link is otherwise unchecked — a mistyped digit or two similar dogs in one afternoon
//! produce a link that is structurally perfect and about the wrong animal. A credential carries the
//! animal's microchip as a Merkle leaf, so recording the same code on the shop's own pet gives the
//! two sides something to compare.
//!
//! What these tests protect, in order of how badly each would hurt:
//!
//!  1. **The check is not INERT.** It shipped once reading a single key path no real issuer emits,
//!     passed every test, and protected nothing — because the fixtures were written to the shape the
//!     reader expected rather than the shape the emitters produce. Section 6 drives the emitters'
//!     own shapes, and everything else here rests on that.
//!  2. **A mismatch is REFUSED**, at every route that writes the binding — not just the link route.
//!     The binding is a pair and either half can move, so a guard on one route reads as an enforced
//!     invariant while the way in stays open.
//!  3. **An absent microchip NEVER blocks a link.** Many animals are not chipped; getting this
//!     backwards makes the field unusable and teaches operators to route around it. This is the
//!     commonest state in the product, so it gets the most cases here.
//!  4. **"Could not compare" is reported as itself** — never as a pass, never as a refusal — with a
//!     reason that says which side was missing or which read did not resolve. And a microchip we
//!     cannot READ is kept apart from one that is not there, because those look identical from the
//!     operator's chair and that is what hid (1).
//!  5. **A mismatch is not an accusation against the credential.** It stays valid; the LINK is wrong.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use std::sync::Arc;
use vet_api::chain::MemChain;
use vet_api::store::{MemStore, Store};

const CHIP: &str = "985141006580319";
const OTHER_CHIP: &str = "900000000000001";

/// The concrete `MemStore` is handed back alongside the router so the tests can seed the per-tag
/// document cache directly — a groomer/vet test has no live customer wallet to run
/// `POST /import/pull` against — and so one test can inject a store read failure.
async fn app_with_state() -> (axum::Router, String, Arc<MemStore>) {
    let store = Arc::new(MemStore::new());
    let mut state = state_with(
        Arc::new(MemChain::new()),
        "memchain".to_string(),
        "0x00000000000000000000000000000000000000aa".to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
        "vet.example".to_string(),
        1,
    );
    state.store = store.clone();
    let op = mint_operator(&state).await;
    (vet_api::router(state), op, store)
}

/// Wrap a `data` block into the envelope `POST /import/pull` files, with each leaf packed as
/// `"<saltHex>:<typeTag>:<value>"`.
///
/// Only `data` varies between the fixtures below, because only `data` is what `check_integrity`
/// folds and only `data` is where a key path can differ between issuers.
fn wrapped(data: Value) -> Value {
    json!({
        "version": "dogtag/1.0",
        "data": data,
        "signature": {
            "type": "MerkleRoot",
            "targetHash": "0x00",
            "proof": [],
            "merkleRoot": "0x00",
        },
        "privacy": { "obfuscated": [] },
        "issuer": {
            "name": "Seaport Vet",
            "domain": "vet.example",
            "documentStore": "0x00000000000000000000000000000000000000aa",
            "recordType": "VACCINATION",
        },
    })
}

fn packed(value: &str) -> String {
    format!("b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2:5:{value}")
}

const DOG_TAG_LEAF: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1:2:4";

/// A held document in the SCHEMA-CONFORMANT nested shape, `credentialSubject.microchip.code` — the
/// one `dogtag_standard::schema::validate_schema` describes.
///
/// `chip: None` is a credential for an unchipped animal — the leaf is simply absent, exactly as it is
/// on a real document for a pet that has no microchip.
fn held_doc(chip: Option<&str>) -> Value {
    let mut subject = json!({ "dogTagId": DOG_TAG_LEAF });
    if let Some(c) = chip {
        subject["microchip"] = json!({ "code": packed(c) });
    }
    wrapped(json!({ "credentialSubject": subject }))
}

// ---- fixtures CAPTURED FROM THE EMITTERS, not written to match the reader --------------------
//
// This check shipped reading ONE key path that no real issuer emits, and every test passed, because
// the fixtures were hand-written to the shape the implementation expected. They agreed with the code
// and disagreed with production. The three below are derived from the emitters instead, and each
// names where it came from so a reader can re-check it against the source rather than against this
// file's own habits.

/// The vet portal's REAL VACCINATION `fields` payload, derived from the emitter.
///
/// `packages/ui/src/schema/recordTypes.ts` declares `path: "microchip.code"` with NO
/// `credentialSubject.` prefix; `buildFieldsObject` in the same file nests from the ROOT;
/// `stacks/vet/web/src/pages/Issue.tsx` keys its `values` map by `f.path` and posts the result as
/// `fields`; and `app::build_vc` clones that object verbatim, injecting only
/// `credentialSubject.dogTagId`. So the microchip lands at the data TOP LEVEL, a SIBLING of
/// `credentialSubject` — never inside it.
///
/// The shared `common::vaccination_fields()` hand-nests it under `credentialSubject`, a shape the
/// portal never produces. It is left alone (eleven call sites across six other suites read it) and
/// this fixture is what the microchip cases drive instead.
fn vet_portal_fields(chip: &str) -> Value {
    json!({
        "microchip": {
            "code": { "tag": 2, "value": chip },
            "standard": { "tag": 2, "value": "ISO_11784_11785" },
            "implantDate": { "tag": 2, "value": "2023-10-01" },
        },
        "vaccineProductName": { "tag": 2, "value": "Rabvac 3" },
        "vaccinationDate": { "tag": 2, "value": "2026-01-11" },
    })
}

/// A government `EU_HEALTH_CERT` document: `credentialSubject.microchipNumber`.
///
/// TRANSCRIBED from `stacks/government/api/src/app.rs::build_gov_vc`, not driven through it —
/// `build_gov_vc` lives in the `government-api` crate, which vet-api's tests cannot call, so this is
/// the one gap that could not be closed by execution. The subject's leaf NAMES are copied verbatim
/// from that match arm; re-check them there if the two ever disagree.
fn government_eu_health_cert_doc(chip: &str) -> Value {
    wrapped(json!({
        "credentialSubject": {
            "dogTagId": DOG_TAG_LEAF,
            "receiptId": packed("A1B2C3D4E5F6"),
            "species": packed("dog"),
            "microchipNumber": packed(chip),
            "rabiesVaccinationDate": packed("2026-01-15"),
            "rabiesValidUntil": packed("2029-01-14"),
        }
    }))
}

/// A government `TRAVEL_CLEARANCE` document: `credentialSubject.animal.microchipNumber`, nested
/// under the CDC Section B `animal` block.
///
/// TRANSCRIBED from the same function's `_` arm, with the same caveat as above. This is the shape
/// that most needs a suffix match: neither the leaf name nor its parent matches the vet portal's.
fn government_travel_clearance_doc(chip: &str) -> Value {
    wrapped(json!({
        "credentialSubject": {
            "dogTagId": DOG_TAG_LEAF,
            "receiptId": packed("A1B2C3D4E5F6"),
            "validity": { "validFrom": packed("2026-07-01"), "validUntil": packed("2027-01-01") },
            "importer": { "lastName": packed("Zagara") },
            "animal": {
                "name": packed("Blaze"),
                "breed": packed("Poodle - Standard"),
                "microchipNumber": packed(chip),
            },
            "travel": { "portOfEntry": packed("JFK") },
        }
    }))
}

async fn hold(store: &Arc<MemStore>, tag: &str, chip: Option<&str>) {
    store.upsert_client_cache(tag.to_string(), held_doc(chip)).await;
}

async fn hold_doc(store: &Arc<MemStore>, tag: &str, doc: Value) {
    store.upsert_client_cache(tag.to_string(), doc).await;
}

/// Create a client with pets verbatim; returns the minted petIds in order.
async fn make_pets(app: &axum::Router, op: &str, name: &str, pets: Value) -> Vec<String> {
    let (s, b) = call(
        app,
        "POST",
        "/clients",
        Some(op),
        Some(json!({ "name": name, "pets": pets })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "create client: {b}");
    b["pets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["petId"].as_str().unwrap().to_string())
        .collect()
}

async fn link(app: &axum::Router, op: &str, pet: &str, tag: &str) -> (StatusCode, Value) {
    call(
        app,
        "POST",
        &format!("/pets/{pet}/dogtag"),
        Some(op),
        Some(json!({ "dogTagId": tag })),
    )
    .await
}

/// Every `notComparable` body must carry all four keys. Asserted from ONE place so no case can quietly
/// report a state without the reason that makes it readable.
fn assert_not_comparable(check: &Value, reason: &str, is_failure: bool) {
    assert_eq!(check["state"], "notComparable", "{check}");
    assert_eq!(check["reason"], reason, "{check}");
    assert_eq!(
        check["isFailure"], is_failure,
        "the renderer's tone comes from this flag: {check}"
    );
    let detail = check["detail"].as_str().unwrap_or_default();
    assert!(
        !detail.is_empty(),
        "a state that could not be established must say why: {check}"
    );
}

// ============================================================================================
// 1. the check fires and refuses — at every route that writes the binding
// ============================================================================================

#[tokio::test]
async fn a_matching_microchip_links_and_says_so() {
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "microchipCode": CHIP }])).await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "a matching chip must link: {b}");
    assert_eq!(b["microchipCheck"]["state"], "matched", "{b}");
    assert_eq!(b["microchipCheck"]["microchip"], CHIP, "{b}");
    assert_eq!(b["dogTagId"], "4", "the link must actually have been written: {b}");
}

#[tokio::test]
async fn a_mismatched_microchip_is_refused_and_the_message_names_both_codes() {
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let pets = make_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex", "microchipCode": OTHER_CHIP }]),
    )
    .await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::CONFLICT, "a mismatch must be refused: {b}");
    assert_eq!(b["microchipCheck"]["state"], "mismatch", "{b}");
    assert_eq!(b["microchipCheck"]["petMicrochip"], OTHER_CHIP, "{b}");
    assert_eq!(b["microchipCheck"]["credentialMicrochip"], CHIP, "{b}");
    // Naming BOTH is what tells a typo from a genuinely wrong animal.
    let msg = b["error"].as_str().unwrap();
    assert!(msg.contains(CHIP) && msg.contains(OTHER_CHIP), "{msg}");

    // ...and NOTHING was written. A refused link that half-applied would be worse than no guard.
    let (_, pet) = call(&app, "GET", &format!("/pets/{}", pets[0]), Some(&op), None).await;
    assert!(pet["dogTagId"].is_null(), "the refused link must not have been written: {pet}");
}

#[tokio::test]
async fn the_refusal_does_not_accuse_the_credential() {
    // The credential is genuine and stays valid for everyone; it just describes a different animal.
    // Same rule that keeps `issuer_mismatch` apart from `issuer_not_whitelisted`.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let pets = make_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex", "microchipCode": OTHER_CHIP }]),
    )
    .await;

    let (_, b) = link(&app, &op, &pets[0], "4").await;
    let msg = b["error"].as_str().unwrap().to_lowercase();
    assert!(
        msg.contains("stays valid") || msg.contains("not refused"),
        "the refusal must say the credential itself is not being rejected: {msg}"
    );
    assert!(
        !msg.contains("invalid") && !msg.contains("forged"),
        "a wrong LINK is not an invalid credential: {msg}"
    );
    // And the held document is untouched — refusing a link never unfiles a credential.
    assert!(store.get_client_cache("4").await.is_some());
}

#[tokio::test]
async fn creating_a_pet_with_a_tag_runs_the_same_check_as_linking() {
    // `POST /pets` carries both fields, so guarding only the link route would leave the mis-link
    // reachable through the adjacent route — exactly why the one-pet-per-tag rule is duplicated here.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let (s, b) = call(
        &app,
        "POST",
        "/clients",
        Some(&op),
        Some(json!({ "name": "Bob Lim", "pets": [] })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let client = b["clientId"].as_str().unwrap().to_string();

    let (s, b) = call(
        &app,
        "POST",
        "/pets",
        Some(&op),
        Some(json!({
            "clientId": client, "name": "Milo",
            "dogTagId": "4", "microchipCode": OTHER_CHIP,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["microchipCheck"]["state"], "mismatch", "{b}");
}

#[tokio::test]
async fn the_whole_document_client_routes_run_the_same_check() {
    // `POST /clients` and `PUT /clients/{id}` carry pets inline, and the groomer's client form is the
    // likelier place an operator types a tag than the pet page is.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;

    let (s, b) = call(
        &app,
        "POST",
        "/clients",
        Some(&op),
        Some(json!({
            "name": "Alice Tan",
            "pets": [{ "name": "Rex", "dogTagId": "4", "microchipCode": OTHER_CHIP }],
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "create client must refuse a mismatch: {b}");
    assert_eq!(b["microchipCheck"]["state"], "mismatch", "{b}");

    // The edit route too — its payload REPLACES the pet list, so it is a second way in.
    let (s, b) = call(
        &app,
        "POST",
        "/clients",
        Some(&op),
        Some(json!({ "name": "Alice Tan", "pets": [{ "name": "Rex" }] })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let client = b["clientId"].as_str().unwrap().to_string();
    let pet_id = b["pets"][0]["petId"].as_str().unwrap().to_string();

    let (s, b) = call(
        &app,
        "PUT",
        &format!("/clients/{client}"),
        Some(&op),
        Some(json!({
            "name": "Alice Tan",
            "pets": [{ "petId": pet_id, "name": "Rex", "dogTagId": "4", "microchipCode": OTHER_CHIP }],
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "update client must refuse a mismatch: {b}");
}

#[tokio::test]
async fn setting_a_wrong_microchip_after_linking_is_refused() {
    // THE TWO-STEP HOLE. The binding is a pair and this route moves the other half: link while the
    // pet has no chip (correctly allowed), then set a chip belonging to a different animal. A guard
    // that only watched the routes writing a TAG would let the exact defect through in two ordinary
    // steps, through the adjacent route with the same field.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "an unchipped pet must link freely: {b}");
    assert_not_comparable(&b["microchipCheck"], "petHasNoMicrochip", false);

    let (s, b) = call(
        &app,
        "PUT",
        &format!("/pets/{}", pets[0]),
        Some(&op),
        Some(json!({ "microchipCode": OTHER_CHIP })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "step two must be refused too: {b}");
    assert_eq!(b["microchipCheck"]["state"], "mismatch", "{b}");

    // The MATCHING code still saves, and clearing stays possible — clearing a wrongly-typed code is
    // how an operator gets out of a mismatch they cannot otherwise correct.
    let (s, b) = call(
        &app,
        "PUT",
        &format!("/pets/{}", pets[0]),
        Some(&op),
        Some(json!({ "microchipCode": CHIP })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["microchipCode"], CHIP, "{b}");

    let (s, b) = call(
        &app,
        "PUT",
        &format!("/pets/{}", pets[0]),
        Some(&op),
        Some(json!({ "microchipCode": "" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "an explicit blank must clear it: {b}");
    assert!(b["microchipCode"].is_null(), "{b}");
}

// ============================================================================================
// 2. absent is NORMAL — the commonest state in the product, and it must never block anything
// ============================================================================================

#[tokio::test]
async fn a_pet_with_no_microchip_links_freely_and_the_reason_is_reported() {
    // Cats routinely are not chipped. Refusing here would make the field unusable.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Mittens" }])).await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "an unchipped pet must never be blocked: {b}");
    assert_not_comparable(&b["microchipCheck"], "petHasNoMicrochip", false);
    assert_eq!(b["dogTagId"], "4", "{b}");
}

#[tokio::test]
async fn a_credential_with_no_microchip_links_freely_and_the_reason_is_reported() {
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", None).await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "microchipCode": CHIP }])).await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_not_comparable(&b["microchipCheck"], "credentialHasNoMicrochip", false);
    assert_eq!(b["dogTagId"], "4", "{b}");
}

#[tokio::test]
async fn neither_side_having_one_is_an_ordinary_link() {
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", None).await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Mittens" }])).await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    // The immovable side is named: no entry on the pet record could make this comparison possible,
    // so telling the operator to go find a chip number would be an errand that cannot succeed.
    assert_not_comparable(&b["microchipCheck"], "credentialHasNoMicrochip", false);
}

#[tokio::test]
async fn a_blank_microchip_is_absent_rather_than_a_code_that_cannot_match() {
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let pets = make_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex", "microchipCode": "   " }]),
    )
    .await;
    let (s, b) = call(&app, "GET", &format!("/pets/{}", pets[0]), Some(&op), None).await;
    assert!(b["microchipCode"].is_null(), "whitespace must normalize to absent: {b}");

    let (s2, b2) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s2, StatusCode::OK, "{s} {b2}");
    assert_not_comparable(&b2["microchipCheck"], "petHasNoMicrochip", false);
}

#[tokio::test]
async fn padding_around_a_code_is_not_a_mismatch() {
    // A pasted code must compare equal to itself. Untrimmed, this reports a mismatch between a code
    // and itself — a refusal whose cause the operator cannot see and cannot clear.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let pets = make_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex", "microchipCode": format!("  {CHIP} ") }]),
    )
    .await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["microchipCheck"]["state"], "matched", "{b}");
}

// ============================================================================================
// 3. "could not compare" is its own answer, and its reasons do not collapse into each other
// ============================================================================================

#[tokio::test]
async fn holding_no_credential_is_a_fact_not_a_pass_and_not_a_refusal() {
    let (app, op, _store) = app_with_state().await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "microchipCode": CHIP }])).await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "nothing to compare must not block a link: {b}");
    assert_not_comparable(&b["microchipCheck"], "noCredentialHeld", false);
}

#[tokio::test]
async fn a_field_element_link_reports_a_lookup_it_cannot_perform_not_an_absence() {
    // The held-document cache is keyed by the HANDLE, and handle -> field element is a Poseidon hash
    // that cannot be inverted. Folding this into `noCredentialHeld` would state "this shop holds
    // nothing" when the truth is "this shop cannot ask" — the same understatement
    // `PetTagCredentials` already refuses to make about the same cache.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let field = "1195241908933892557940129631300775214454584041594363078565480038450625444405";
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "microchipCode": CHIP }])).await;

    let (s, b) = link(&app, &op, &pets[0], field).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_not_comparable(&b["microchipCheck"], "cannotLookUpByFieldElement", true);
}

#[tokio::test]
async fn an_unreadable_credential_store_is_a_failure_not_an_absence() {
    // A collapsed driver fault would arrive as "this shop holds no credential" — a FACT about the
    // shop's records, rendered neutrally, on the strength of a read that never happened.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "microchipCode": CHIP }])).await;

    store.set_fail_client_cache_reads(true);

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    // Still LINKED: this check is evidence, not a safety invariant, so failing closed here would
    // refuse ordinary work. But it is reported as a failure, with its own reason and tone.
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_not_comparable(&b["microchipCheck"], "couldNotRead", true);
}

#[tokio::test]
async fn a_held_document_that_will_not_parse_is_a_failure_not_an_absence() {
    let (app, op, store) = app_with_state().await;
    store
        .upsert_client_cache("4".to_string(), json!({ "not": "a wrapped doc" }))
        .await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "microchipCode": CHIP }])).await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_not_comparable(&b["microchipCheck"], "credentialUnreadable", true);
}

#[tokio::test]
async fn the_link_response_always_carries_the_verdict() {
    // Absence must not be representable as "checked and fine" on the surface that writes the binding.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    hold(&store, "5", None).await;

    for (tag, chip) in [("4", Some(CHIP)), ("5", Some(CHIP)), ("6", None), ("7", Some(CHIP))] {
        let mut pet = json!({ "name": "Rex" });
        if let Some(c) = chip {
            pet["microchipCode"] = json!(c);
        }
        let pets = make_pets(&app, &op, &format!("Owner {tag}"), json!([pet])).await;
        let (s, b) = link(&app, &op, &pets[0], tag).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert!(
            b["microchipCheck"].is_object(),
            "tag {tag} produced a link response with no verdict: {b}"
        );
    }
}

#[tokio::test]
async fn a_list_row_carries_the_code_but_never_a_verdict() {
    // A verdict on a list row would be a claim about a comparison nobody ran for that row — and an
    // absent key there would then be ambiguous with "checked and fine".
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "microchipCode": CHIP }])).await;
    link(&app, &op, &pets[0], "4").await;

    let (s, b) = call(&app, "GET", &format!("/pets/{}", pets[0]), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["microchipCode"], CHIP, "{b}");
    assert!(b["microchipCheck"].is_null(), "a read must carry no verdict: {b}");

    let (s, b) = call(&app, "GET", "/pets", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(b["rows"][0]["microchipCheck"].is_null(), "{b}");
}

// ============================================================================================
// 4. persistence
// ============================================================================================

#[tokio::test]
async fn the_microchip_survives_a_write_to_a_sibling_and_a_dogtag_write() {
    // Pets are stored embedded, so every pet write is a client write. A field that vanished on an
    // unrelated save would make the check quietly stop firing.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let pets = make_pets(
        &app,
        &op,
        "Alice Tan",
        json!([
            { "name": "Rex", "microchipCode": CHIP },
            { "name": "Mittens" },
        ]),
    )
    .await;

    link(&app, &op, &pets[0], "4").await;
    call(
        &app,
        "PUT",
        &format!("/pets/{}", pets[1]),
        Some(&op),
        Some(json!({ "breed": "Ragdoll" })),
    )
    .await;
    call(&app, "DELETE", &format!("/pets/{}/dogtag", pets[0]), Some(&op), None).await;

    let (_, b) = call(&app, "GET", &format!("/pets/{}", pets[0]), Some(&op), None).await;
    assert_eq!(b["microchipCode"], CHIP, "the chip must survive an unlink: {b}");
    let (_, b) = call(&app, "GET", &format!("/pets/{}", pets[1]), Some(&op), None).await;
    assert!(b["microchipCode"].is_null(), "a sibling gains nothing: {b}");
}

#[tokio::test]
async fn a_pet_row_written_before_this_field_existed_still_reads() {
    // `#[serde(default)]`: live rows predate the field, and a store that cannot deserialize them
    // would take the whole clients collection down rather than merely losing a check.
    let legacy = json!({
        "petId": "p1", "name": "Rex", "species": "dog", "breed": "",
        "sex": "", "dateOfBirth": "", "notes": "", "dogTagId": "4",
    });
    let p: vet_api::store::ClientPet =
        serde_json::from_value(legacy).expect("a pre-field row must still deserialize");
    assert!(p.microchip_code.is_none());
    assert_eq!(p.dog_tag_id.as_deref(), Some("4"));
}

// ============================================================================================
// 5. the IMPORT direction
//
// The link routes cover "the tag is attached after the credential is held". The other order is just
// as ordinary: the tag is linked first and the credential arrives later, through `POST /import/pull`.
// Without the check here the identical wrong pairing simply lands the other way round.
//
// These drive the REAL route — real outbound fetch, real `third_party_verify`, real credential — so
// they also pin that the microchip check runs AFTER verification and never disturbs the verdict.
// ============================================================================================

/// A vet whose MemChain registers issuance under `FACTORY_ADDR`, so a genuine credential resolves
/// through the factory exactly as on a real deployment and `third_party_verify` passes.
struct Import {
    app: axum::Router,
    op: String,
    store: Arc<MemStore>,
}

async fn boot_import() -> Import {
    let mem = vet_api::chain::MemChain::new().with_factory(FACTORY_ADDR);
    let store = Arc::new(MemStore::new());
    let mut state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        "0x00000000000000000000000000000000000000aa".to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
        "vet.example".to_string(),
        1,
    );
    state.store = store.clone();
    let app = vet_api::router(state);
    let (_admin, op, backend) = boot_custody(&app).await;
    let rt = vet_api::chain::record_type_key("VACCINATION");
    mem.whitelist("0x00000000000000000000000000000000000000aa", &rt, &backend);
    mem.set_record_type("0x00000000000000000000000000000000000000bb", &rt);
    Import { app, op, store }
}

/// Issue a real credential through the ordinary flow and read the shared document back.
///
/// Drives [`vet_portal_fields`], the shape the VET PORTAL actually posts, rather than the shared
/// `vaccination_fields()` — so the end-to-end path exercises the emitter's key paths and not the
/// reader's. Swapping this back to the shared fixture is what would let this suite go green over a
/// check that is inert in production.
async fn issue_doc(app: &axum::Router, op: &str, dog_tag_id: &str, chip: &str) -> Value {
    issue_doc_with_fields(app, op, dog_tag_id, vet_portal_fields(chip)).await
}

async fn issue_doc_with_fields(
    app: &axum::Router,
    op: &str,
    dog_tag_id: &str,
    fields: Value,
) -> Value {
    let (s, b) = call(
        app,
        "POST",
        "/credentials/prepare",
        Some(op),
        Some(json!({ "recordType": "VACCINATION", "dogTagId": dog_tag_id, "fields": fields })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "prepare: {b}");
    let record_id = b["recordId"].as_str().unwrap().to_string();

    let (s, b) = call(
        app,
        "POST",
        &format!("/records/{record_id}/share"),
        Some(op),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "share: {b}");
    let token = b["qrUrl"].as_str().unwrap().rsplit('/').next().unwrap().to_string();

    let (s, doc) = call(app, "GET", &format!("/r/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "get shared: {doc}");
    doc
}

/// Serve `doc` at `GET /share/{ref}` on an ephemeral loopback port, as an owner's wallet API does,
/// so the route performs its real outbound fetch.
async fn serve_doc(doc: Value) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/share/:record_ref",
        axum::routing::get(move || {
            let doc = doc.clone();
            async move { axum::Json(doc) }
        }),
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn import_pull(app: &axum::Router, op: &str, doc: Value) -> (StatusCode, Value) {
    let base = serve_doc(doc).await;
    call(
        app,
        "POST",
        "/import/pull",
        Some(op),
        Some(json!({ "userApiBase": base, "recordRef": "rec-1", "userJwt": "owner-token" })),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn importing_a_credential_whose_microchip_matches_the_linked_pet_succeeds() {
    let d = boot_import().await;
    let doc = issue_doc(&d.app, &d.op, "1001", CHIP).await;
    make_pets(
        &d.app,
        &d.op,
        "Alice Tan",
        json!([{ "name": "Rex", "dogTagId": "1001", "microchipCode": CHIP }]),
    )
    .await;

    let (s, b) = import_pull(&d.app, &d.op, doc).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["imported"], true, "{b}");
    assert_eq!(b["microchipCheck"]["state"], "matched", "{b}");
    assert!(d.store.get_client_cache("1001").await.is_some(), "the document must be filed");
}

#[tokio::test(flavor = "multi_thread")]
async fn importing_a_credential_for_a_different_animal_is_refused_without_condemning_it() {
    let d = boot_import().await;
    let doc = issue_doc(&d.app, &d.op, "1001", CHIP).await;
    make_pets(
        &d.app,
        &d.op,
        "Alice Tan",
        json!([{ "name": "Rex", "dogTagId": "1001", "microchipCode": OTHER_CHIP }]),
    )
    .await;

    let (s, b) = import_pull(&d.app, &d.op, doc).await;
    assert_eq!(s, StatusCode::CONFLICT, "a wrong pairing must be refused: {b}");
    assert_eq!(b["microchipCheck"]["state"], "mismatch", "{b}");
    // The VERDICT is untouched: the credential verified against every pillar and is genuine. A
    // wrong LINK is a different accusation with a different remedy, and folding it into `valid`
    // would report a real credential as forged.
    assert_eq!(b["verdict"]["valid"], true, "the credential itself still verifies: {b}");
    // The refusal names WHERE the shop has that tag, which is what turns it into an instruction.
    let msg = b["error"].as_str().unwrap();
    assert!(msg.contains("Rex") && msg.contains("Alice Tan"), "{msg}");
    // ...and nothing was filed. A refused import that still cached would leave the wrong document
    // reachable from the pet page.
    assert!(
        d.store.get_client_cache("1001").await.is_none(),
        "a refused import must not file the document"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_withheld_microchip_leaf_imports_and_reports_the_withheld_count() {
    // THE REAL not-comparable case on this path: the holder redacted the microchip under selective
    // disclosure. `obfuscate` moves the leaf's hash into `privacy.obfuscated` and drops the
    // cleartext, leaving the Merkle root UNCHANGED — so the credential still verifies, and this
    // shop genuinely cannot compare. Which leaf was withheld is uninvertible (the Poseidon image
    // needs both salt and value), so the count is offered as context and never as an attribution.
    let d = boot_import().await;
    let doc = issue_doc(&d.app, &d.op, "1001", CHIP).await;
    let parsed: dogtag_standard::wrap::WrappedDoc = serde_json::from_value(doc).unwrap();
    let root_before = parsed.signature.merkle_root.clone();
    // The vet portal's own key path — `microchip.code` at the data TOP LEVEL, not under
    // `credentialSubject`. Naming the nested path here fails outright with "cannot obfuscate missing
    // field", which is the emitter shape asserting itself.
    let redacted = dogtag_standard::wrap::obfuscate(&parsed, &["microchip.code".to_string()])
        .expect("the microchip leaf is present and obfuscatable");
    assert_eq!(
        redacted.signature.merkle_root, root_before,
        "obfuscation must not move R, or this would be testing a different document"
    );

    make_pets(
        &d.app,
        &d.op,
        "Alice Tan",
        json!([{ "name": "Rex", "dogTagId": "1001", "microchipCode": CHIP }]),
    )
    .await;

    let (s, b) = import_pull(&d.app, &d.op, serde_json::to_value(&redacted).unwrap()).await;
    assert_eq!(s, StatusCode::OK, "a redacted credential still imports: {b}");
    assert_not_comparable(&b["microchipCheck"], "credentialHasNoMicrochip", false);
    let detail = b["microchipCheck"]["detail"].as_str().unwrap();
    assert!(detail.contains('1'), "the withheld count is context: {detail}");
    assert!(
        detail.contains("cannot be determined"),
        "it must not claim to know WHICH field was withheld: {detail}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn importing_for_a_tag_no_pet_holds_reports_that_rather_than_a_pass() {
    // "No pet is linked to this tag" is not "this pet has no microchip" — saying the latter would
    // name an animal that is not there.
    let d = boot_import().await;
    let doc = issue_doc(&d.app, &d.op, "1001", CHIP).await;

    let (s, b) = import_pull(&d.app, &d.op, doc).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_not_comparable(&b["microchipCheck"], "noLinkedPet", false);
}

#[tokio::test(flavor = "multi_thread")]
async fn importing_against_an_unchipped_pet_reports_that_and_still_imports() {
    let d = boot_import().await;
    let doc = issue_doc(&d.app, &d.op, "1001", CHIP).await;
    make_pets(
        &d.app,
        &d.op,
        "Alice Tan",
        json!([{ "name": "Mittens", "dogTagId": "1001" }]),
    )
    .await;

    let (s, b) = import_pull(&d.app, &d.op, doc).await;
    assert_eq!(s, StatusCode::OK, "an unchipped pet must never block an import: {b}");
    assert_not_comparable(&b["microchipCheck"], "petHasNoMicrochip", false);
    assert!(d.store.get_client_cache("1001").await.is_some());
}

#[tokio::test]
async fn a_client_edit_that_echoes_the_microchip_preserves_it() {
    // `PUT /clients/{id}` REPLACES the owner's whole pet list, so the microchip behaves exactly like
    // `dogTagId`: echoed it survives, omitted it is cleared. Pinned because the failure is silent —
    // an erased code does not error, it just quietly stops the cross-check firing on every future
    // link. The groomer's `ClientForm` seeds and re-sends the field for this reason.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let (s, b) = call(
        &app,
        "POST",
        "/clients",
        Some(&op),
        Some(json!({
            "name": "Alice Tan",
            "pets": [{ "name": "Rex", "microchipCode": CHIP }],
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let client = b["clientId"].as_str().unwrap().to_string();
    let pet_id = b["pets"][0]["petId"].as_str().unwrap().to_string();
    assert_eq!(b["pets"][0]["microchipCode"], CHIP, "{b}");

    let (s, b) = call(
        &app,
        "PUT",
        &format!("/clients/{client}"),
        Some(&op),
        Some(json!({
            "name": "Alice Tan-Lim",
            "pets": [{ "petId": pet_id, "name": "Rex", "microchipCode": CHIP }],
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["pets"][0]["microchipCode"], CHIP, "an echoed code must survive: {b}");

    // ...and the surviving code still gates a later link, which is the whole reason it must survive.
    let (s, b) = link(&app, &op, &pet_id, "4").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["microchipCheck"]["state"], "matched", "{b}");
}

#[tokio::test]
async fn an_unreadable_pet_store_refuses_a_microchip_edit_rather_than_skipping_the_check() {
    // The guard's OWN fail-open. `update_pet` sources the tag it checks a new microchip against from
    // a pet read; the collapsing `get_pet` turns an unreadable store into `None`, which would skip
    // the whole guard and let the write land with no check and no report — the exact defect this
    // feature exists to close, reintroduced through the guard itself.
    //
    // It REFUSES rather than reporting `couldNotRead` and proceeding, unlike the credential-cache
    // read: this is the read that decides WHICH tag is being checked at all, so it is the same
    // uniqueness-class failure `create_pet` and `link_pet_dogtag` already answer with 503.
    let (app, op, store) = app_with_state().await;
    hold(&store, "4", Some(CHIP)).await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;
    let (s, _) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK);

    store.set_fail_pet_reads(true);
    let (s, b) = call(
        &app,
        "PUT",
        &format!("/pets/{}", pets[0]),
        Some(&op),
        Some(json!({ "microchipCode": OTHER_CHIP })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::SERVICE_UNAVAILABLE,
        "a check that could not run must not admit the write: {b}"
    );

    // ...and nothing was written, so the wrong code did not land unchecked.
    store.set_fail_pet_reads(false);
    let (_, pet) = call(&app, "GET", &format!("/pets/{}", pets[0]), Some(&op), None).await;
    assert!(pet["microchipCode"].is_null(), "{pet}");
}

#[tokio::test]
async fn an_edit_that_does_not_touch_the_microchip_is_not_refused_by_a_stored_mismatch() {
    // Only a SUPPLIED code is a claim. If the guard checked the STORED code on every edit instead,
    // a pet whose record already disagrees with its credential — which pre-rule data can contain,
    // since the guard postdates the field — could never be edited again at all: an operator fixing a
    // breed would be refused over a microchip they never touched and cannot clear from that form.
    // The same grandfathering `reject_dog_tag_conflicts` already applies to tags.
    let (app, op, store) = app_with_state().await;
    let pets = make_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex", "microchipCode": OTHER_CHIP }]),
    )
    .await;
    // Linked while the shop held nothing, then the conflicting credential arrives afterwards.
    let (s, _) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK);
    hold(&store, "4", Some(CHIP)).await;

    let (s, b) = call(
        &app,
        "PUT",
        &format!("/pets/{}", pets[0]),
        Some(&op),
        Some(json!({ "breed": "Standard Poodle" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "an unrelated edit must not be blocked: {b}");
    assert_eq!(b["breed"], "Standard Poodle", "{b}");
    assert_eq!(b["microchipCode"], OTHER_CHIP, "and the stored code is left alone: {b}");
}

// ============================================================================================
// 6. the four REAL emitter shapes, and the reader that could not read one
//
// This check shipped reading exactly one key path — `credentialSubject.microchip.code` — that no
// issuer in the fleet emits, so it was INERT on every real credential: it passed its whole suite
// and protected nothing. What let that happen is not the key-path list, it is that the fixtures were
// written to the shape the READER expected. So these cases drive the emitters' own shapes, and the
// last group pins the state that makes a future inertness loud instead of invisible.
// ============================================================================================

#[tokio::test(flavor = "multi_thread")]
async fn the_vet_portals_own_shape_is_read_end_to_end() {
    // THE MUTATION CATCHER for the original defect. `issue_doc` posts the fields the vet portal
    // really posts, so the microchip leaf reaches `data` at the TOP LEVEL as `microchip.code`.
    // Narrowing the matcher back to exact `credentialSubject.microchip.code` makes this credential
    // read as carrying no microchip at all, and this case goes red on `state` — where the previous
    // fixtures, which nested it under `credentialSubject` themselves, stayed green.
    let d = boot_import().await;
    let doc = issue_doc(&d.app, &d.op, "1001", CHIP).await;
    assert!(
        doc["data"]["microchip"]["code"].is_string(),
        "the vet portal emits this leaf at the data top level, NOT under credentialSubject: {doc}"
    );
    make_pets(
        &d.app,
        &d.op,
        "Alice Tan",
        json!([{ "name": "Rex", "dogTagId": "1001", "microchipCode": CHIP }]),
    )
    .await;

    let (s, b) = import_pull(&d.app, &d.op, doc).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        b["microchipCheck"]["state"], "matched",
        "the vet portal's own shape must be READ, not reported as an absent microchip: {b}"
    );
}

#[tokio::test]
async fn the_governments_eu_health_cert_shape_is_read() {
    // `credentialSubject.microchipNumber` — a different leaf NAME under a different parent. Half
    // covering the fleet reproduces the identical silent hole for the other half.
    let (app, op, store) = app_with_state().await;
    hold_doc(&store, "4", government_eu_health_cert_doc(CHIP)).await;
    let pets = make_pets(
        &app,
        &op,
        "Alice Tan",
        json!([{ "name": "Rex", "microchipCode": OTHER_CHIP }]),
    )
    .await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::CONFLICT, "a wrong pairing must be refused here too: {b}");
    assert_eq!(b["microchipCheck"]["state"], "mismatch", "{b}");
}

#[tokio::test]
async fn the_governments_travel_clearance_shape_is_read_from_its_section_b_block() {
    // `credentialSubject.animal.microchipNumber` — nested one level deeper again. This is the shape
    // an exact-path match cannot reach under any single spelling.
    let (app, op, store) = app_with_state().await;
    hold_doc(&store, "4", government_travel_clearance_doc(CHIP)).await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "microchipCode": CHIP }])).await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["microchipCheck"]["state"], "matched", "{b}");
    assert_eq!(b["microchipCheck"]["microchip"], CHIP, "{b}");
}

#[tokio::test]
async fn a_microchip_at_a_key_path_we_cannot_read_is_loud_and_never_reads_as_nothing_to_compare() {
    // THE REMEDY FOR THE MECHANISM, not for the key-path list. "The credential has no microchip" is
    // an ordinary, quiet, benign state — and it is exactly what a broken reader produced. So a
    // microchip-shaped leaf we do not recognise gets its OWN state, names the path, and says the
    // check could not RUN. Folding it back into `notComparable` under any reason re-creates the
    // camouflage that let this ship inert.
    let (app, op, store) = app_with_state().await;
    hold_doc(
        &store,
        "4",
        wrapped(json!({
            "credentialSubject": {
                "dogTagId": DOG_TAG_LEAF,
                "chipDetails": { "microchipIdentifier": packed(CHIP) },
            }
        })),
    )
    .await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "microchipCode": CHIP }])).await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "a reader defect is not evidence about the animal: {b}");
    let check = &b["microchipCheck"];
    assert_eq!(check["state"], "unrecognisedCredentialLeaf", "{check}");
    assert_ne!(check["state"], "notComparable", "{check}");
    assert!(check["reason"].is_null(), "it must not borrow notComparable's shape: {check}");
    assert_eq!(
        check["keyPaths"],
        json!(["credentialSubject.chipDetails.microchipIdentifier"]),
        "the unreadable path must be NAMED, so the remedy is obvious from the message: {check}"
    );
    let detail = check["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("credentialSubject.chipDetails.microchipIdentifier"),
        "{detail}"
    );
    assert!(
        !detail.to_lowercase().contains("nothing to compare"),
        "the one sentence it must never say — that is the unchipped animal's, and it reads as \
         success: {detail}"
    );
}

#[tokio::test]
async fn an_unreadable_key_path_outranks_the_commonest_pet_side_fact() {
    // The camouflage would come straight back if any pet-side fact could stand in front of it, and
    // "this pet has no microchip on file" is the commonest state in the product.
    let (app, op, store) = app_with_state().await;
    hold_doc(
        &store,
        "4",
        wrapped(json!({
            "credentialSubject": { "dogTagId": DOG_TAG_LEAF, "microchipRef": packed(CHIP) }
        })),
    )
    .await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex" }])).await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["microchipCheck"]["state"], "unrecognisedCredentialLeaf", "{b}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_with_no_linked_pet_still_reports_an_unreadable_key_path() {
    // The import direction's OWN pet-side fact is `noLinkedPet`, and it is the ordinary order for
    // this route — the credential arrives before the tag is linked. If it stood in front of the loud
    // state, the reader defect would be invisible on precisely the commonest import.
    let d = boot_import().await;
    // A REAL issued document, so it passes `third_party_verify` — only its microchip key path is one
    // this build does not recognise.
    let doc = issue_doc_with_fields(
        &d.app,
        &d.op,
        "1001",
        json!({
            "chipDetails": { "microchipIdentifier": { "tag": 2, "value": CHIP } },
            "vaccinationDate": { "tag": 2, "value": "2026-01-11" },
        }),
    )
    .await;

    let (s, b) = import_pull(&d.app, &d.op, doc).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["imported"], true, "{b}");
    assert_eq!(b["microchipCheck"]["state"], "unrecognisedCredentialLeaf", "{b}");
}

#[tokio::test]
async fn a_chip_container_with_no_code_stays_the_ordinary_absent_case() {
    // The detector's false-positive guard. The vet schema's `microchip.standard` and
    // `microchip.implantDate` are real, common leaves that are NOT codes, so a credential carrying a
    // chip container and no code is an ordinary unchipped animal — firing the loud state here would
    // nag the commonest case in the product with a bug report.
    let (app, op, store) = app_with_state().await;
    hold_doc(
        &store,
        "4",
        wrapped(json!({
            "credentialSubject": {
                "dogTagId": DOG_TAG_LEAF,
                "microchip": {
                    "standard": packed("ISO_11784_11785"),
                    "implantDate": packed("2023-10-01"),
                },
            }
        })),
    )
    .await;
    let pets = make_pets(&app, &op, "Alice Tan", json!([{ "name": "Rex", "microchipCode": CHIP }])).await;

    let (s, b) = link(&app, &op, &pets[0], "4").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_not_comparable(&b["microchipCheck"], "credentialHasNoMicrochip", false);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_pet_lookup_files_the_credential_and_reports_the_skipped_check() {
    // The import's fallible pet read fails OPEN, and this is the ONLY case that reaches that arm.
    // It is the deliberate OPPOSITE of `update_pet`'s pet read, which refuses with 503: this route
    // makes no claim about any pet — it files a credential that already verified — so refusing here
    // would block ordinary work over a lookup the operator never asked for. What it must NOT do is
    // omit the check silently, which would be indistinguishable from a check that passed.
    let d = boot_import().await;
    let doc = issue_doc(&d.app, &d.op, "1001", CHIP).await;
    make_pets(
        &d.app,
        &d.op,
        "Alice Tan",
        json!([{ "name": "Rex", "dogTagId": "1001", "microchipCode": OTHER_CHIP }]),
    )
    .await;
    d.store.set_fail_find_pets_by_dog_tag_reads(true);

    let (s, b) = import_pull(&d.app, &d.op, doc).await;
    assert_eq!(s, StatusCode::OK, "an unreadable pet lookup must not refuse the import: {b}");
    assert_eq!(b["imported"], true, "{b}");
    assert_not_comparable(&b["microchipCheck"], "couldNotRead", true);

    d.store.set_fail_find_pets_by_dog_tag_reads(false);
    assert!(
        d.store.get_client_cache("1001").await.is_some(),
        "the verified credential is still filed"
    );
}
