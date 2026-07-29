//! A provider with no location is first class, and `0,0` is never manufactured.
//!
//! The defect these pin: `Business.lat`/`lng` were non-optional `f64`, so a provider that published
//! no location was stored as the literal `0.0, 0.0` - a legal coordinate in the Gulf of Guinea -
//! and every consumer rendered it as a pin there. Absence had no representation at all.
//!
//! The rule, the same one this codebase applies to verification verdicts: a value that cannot be
//! established must not be rendered as one that can.

mod common;

use axum::http::StatusCode;
use common::*;

use admin_api::store::Business;

/// Register a business, returning the raw (status, body) so the failure cases can assert on it.
async fn register(
    app: &axum::Router,
    admin: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    call(app, "POST", "/v1/businesses", Some(admin), Some(body)).await
}

fn base(name: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "vet", "name": name,
        "services": ["exam"], "apiBaseUrl": "http://biz.example", "domain": "biz.example",
        "documentStores": ["0x00000000000000000000000000000000000000cc"]
    })
}

fn with(name: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut v = base(name);
    let obj = v.as_object_mut().unwrap();
    for (k, val) in extra.as_object().unwrap() {
        obj.insert(k.clone(), val.clone());
    }
    v
}

/// Find one business by name in a `GET /v1/businesses` body.
fn row<'a>(body: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    body["businesses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no business named {name} in {body}"))
}

// ============================================================================================
// A contact-only provider
// ============================================================================================

#[tokio::test]
async fn a_provider_with_no_location_is_listed_with_an_explicit_null_geo_and_its_contacts() {
    let (state, _chain, _vault, _business) = hermetic_state();
    let app = admin_api::router(state);
    let admin = admin_token(&app).await;

    let (s, b) = register(
        &app,
        &admin,
        with(
            "Contact Only Vet",
            serde_json::json!({
                "contact": {
                    "phone": "+65 6123 4567",
                    "whatsapp": "+65 9123 4567",
                    "telegram": "@contactonlyvet",
                    "email": "hello@biz.example",
                    "website": "https://biz.example"
                }
            }),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "register with no location: {b}");

    let (s, body) = call(&app, "GET", "/v1/businesses", None, None).await;
    assert_eq!(s, StatusCode::OK);
    let r = row(&body, "Contact Only Vet");

    // EXPLICIT null, not an omitted key and emphatically not `{lat: 0, lng: 0}`. The wire says
    // "this provider has no location" rather than leaving a consumer to guess.
    assert!(r["geo"].is_null(), "geo must be an explicit null, got {r}");
    assert_eq!(r["contact"]["phone"], "+65 6123 4567");
    assert_eq!(r["contact"]["whatsapp"], "+65 9123 4567");
    assert_eq!(r["contact"]["telegram"], "@contactonlyvet");
    assert_eq!(r["contact"]["email"], "hello@biz.example");
    assert_eq!(r["contact"]["website"], "https://biz.example");
}

#[tokio::test]
async fn a_blank_contact_string_is_absence_not_an_empty_channel() {
    let (state, _chain, _vault, _business) = hermetic_state();
    let app = admin_api::router(state);
    let admin = admin_token(&app).await;

    // A form that submits every field sends "" for the ones left empty. Storing that would make
    // "no phone" and "an empty phone" two states with one meaning.
    let (s, b) = register(
        &app,
        &admin,
        with(
            "Blank Channels",
            serde_json::json!({ "contact": { "phone": "  ", "email": "" , "website": " https://biz.example " } }),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (_s, body) = call(&app, "GET", "/v1/businesses", None, None).await;
    let r = row(&body, "Blank Channels");
    assert!(r["contact"].get("phone").is_none(), "blank phone must be absent: {r}");
    assert!(r["contact"].get("email").is_none(), "blank email must be absent: {r}");
    assert_eq!(r["contact"]["website"], "https://biz.example", "trimmed");
}

#[tokio::test]
async fn a_located_provider_still_carries_its_pair() {
    let (state, _chain, _vault, _business) = hermetic_state();
    let app = admin_api::router(state);
    let admin = admin_token(&app).await;

    let (s, b) = register(
        &app,
        &admin,
        with("Located Vet", serde_json::json!({ "lat": 1.28, "lng": 103.85 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (_s, body) = call(&app, "GET", "/v1/businesses", None, None).await;
    let r = row(&body, "Located Vet");
    assert_eq!(r["geo"]["lat"], 1.28);
    assert_eq!(r["geo"]["lng"], 103.85);
}

// ============================================================================================
// The write path refuses what the read path could not serve
// ============================================================================================

#[tokio::test]
async fn a_half_set_coordinate_pair_is_refused() {
    let (state, _chain, _vault, _business) = hermetic_state();
    let app = admin_api::router(state);
    let admin = admin_token(&app).await;

    // One coordinate is not a place. Refusing it here is what keeps `Business::location`'s
    // half-set arm unreachable through the API.
    let (s, b) = register(&app, &admin, with("Half A", serde_json::json!({ "lat": 1.28 }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, b) = register(&app, &admin, with("Half B", serde_json::json!({ "lng": 103.85 }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
}

#[tokio::test]
async fn an_out_of_range_coordinate_is_refused_at_the_write() {
    let (state, _chain, _vault, _business) = hermetic_state();
    let app = admin_api::router(state);
    let admin = admin_token(&app).await;

    // The read side cannot fix this: `packages/ui`'s directory row validator keeps its
    // all-or-nothing rule for a MALFORMED coordinate, so a single out-of-range row would take the
    // whole directory to `unavailable` for every consumer. Absence is first class; nonsense is not.
    for (name, lat, lng) in [
        ("Too North", 91.0, 0.0),
        ("Too South", -91.0, 0.0),
        ("Too East", 0.0, 181.0),
        ("Too West", 0.0, -181.0),
    ] {
        let (s, b) = register(
            &app,
            &admin,
            with(name, serde_json::json!({ "lat": lat, "lng": lng })),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{name}: {b}");
    }
}

// ============================================================================================
// The deprecated `near=` filter
// ============================================================================================

#[tokio::test]
async fn a_location_less_provider_is_not_within_any_radius() {
    let (state, _chain, _vault, _business) = hermetic_state();
    let app = admin_api::router(state);
    let admin = admin_token(&app).await;

    register(&app, &admin, base("Nowhere Vet")).await;
    register(
        &app,
        &admin,
        with("Nearby Vet", serde_json::json!({ "lat": 1.28, "lng": 103.85 })),
    )
    .await;

    // Same both-positions-must-be-usable rule the on-device `withinRadiusKm` applies: a provider we
    // cannot place is not distance zero, and it is not silently admitted either. Before location
    // was optional it sat at 0,0, so a caller near the Gulf of Guinea would have matched it.
    let (s, body) = call(&app, "GET", "/v1/businesses?near=1.28,103.85&radius=50", None, None).await;
    assert_eq!(s, StatusCode::OK);
    let names: Vec<&str> = body["businesses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Nearby Vet"], "location-less provider must not match a radius");

    // The near-0,0 caller that USED to match it.
    let (_s, body) = call(&app, "GET", "/v1/businesses?near=0,0&radius=50", None, None).await;
    assert!(
        body["businesses"].as_array().unwrap().is_empty(),
        "a location-less provider must not answer a query near 0,0: {body}"
    );

    // Unfiltered, it is listed like any other provider.
    let (_s, body) = call(&app, "GET", "/v1/businesses", None, None).await;
    assert_eq!(body["businesses"].as_array().unwrap().len(), 2);
}

// ============================================================================================
// 0,0 is surfaced for an operator, never reinterpreted
// ============================================================================================

#[tokio::test]
async fn a_row_at_exactly_zero_zero_is_surfaced_for_review_and_never_reinterpreted() {
    let (state, _chain, _vault, _business) = hermetic_state();
    let app = admin_api::router(state.clone());
    let admin = admin_token(&app).await;

    // A row genuinely at 0,0 is still a legal coordinate and must keep answering as one. This is
    // the honest part: code cannot tell it from a pre-optional blank, so it changes nothing.
    let (s, b) = register(
        &app,
        &admin,
        with("Gulf Of Guinea Vet", serde_json::json!({ "lat": 0.0, "lng": 0.0 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "0,0 is a legal coordinate: {b}");
    register(&app, &admin, base("Contact Only Vet")).await;
    register(
        &app,
        &admin,
        with("Real Pin Vet", serde_json::json!({ "lat": 1.28, "lng": 103.85 })),
    )
    .await;

    let (_s, body) = call(&app, "GET", "/v1/businesses", None, None).await;
    let r = row(&body, "Gulf Of Guinea Vet");
    assert_eq!(r["geo"]["lat"], 0.0, "0,0 must NOT be silently nulled out");
    assert_eq!(r["geo"]["lng"], 0.0);

    let (s, review) = call(
        &app,
        "GET",
        "/v1/admin/businesses/location-review",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{review}");
    assert_eq!(review["totalBusinesses"], 3);
    assert_eq!(review["needsReview"], 1, "only the 0,0 row: {review}");
    let flagged = review["businesses"].as_array().unwrap();
    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0]["name"], "Gulf Of Guinea Vet");
    // The location-less provider is NOT flagged - its absence is recorded, not ambiguous.
    assert!(
        !flagged.iter().any(|b| b["name"] == "Contact Only Vet"),
        "an explicitly location-less provider needs no review: {review}"
    );
}

#[tokio::test]
async fn the_location_review_is_admin_gated() {
    let (state, _chain, _vault, _business) = hermetic_state();
    let app = admin_api::router(state);

    let (s, _b) = call(&app, "GET", "/v1/admin/businesses/location-review", None, None).await;
    assert_ne!(s, StatusCode::OK, "review listing must require an admin session");
}

// ============================================================================================
// Persistence: a document written before the field existed must still load
// ============================================================================================

#[test]
fn a_business_document_written_before_these_fields_existed_still_deserializes() {
    // Exactly the shape Mongo holds for a row written by the pre-optional code path. `lat`/`lng`
    // were plain numbers and there was no `contact` key at all.
    let legacy = serde_json::json!({
        "business_id": "b1", "type": "vet", "name": "Legacy Vet",
        "lat": 0.0, "lng": 0.0,
        "services": [], "apiBaseUrl": "http://x", "domain": "x.example",
        "documentStores": [], "hmacKeyId": "k", "hmacSecret": "s"
    });
    let b: Business = serde_json::from_value(legacy).expect("legacy document must deserialize");
    assert_eq!(b.location(), Some((0.0, 0.0)), "0,0 stays 0,0");
    assert!(b.contact.is_empty());
    assert!(
        b.location_needs_review(),
        "a legacy 0,0 row is exactly the case an operator has to answer for"
    );

    // And one with the location key absent entirely.
    let absent = serde_json::json!({
        "business_id": "b2", "type": "vet", "name": "No Location Vet",
        "services": [], "apiBaseUrl": "http://x", "domain": "x.example",
        "documentStores": [], "hmacKeyId": "k", "hmacSecret": "s"
    });
    let b: Business = serde_json::from_value(absent).expect("location-less document must deserialize");
    assert_eq!(b.location(), None);
    assert!(!b.location_needs_review(), "absence is not ambiguous");
}

#[test]
fn a_half_set_document_is_no_location_rather_than_half_a_position() {
    let half = serde_json::json!({
        "business_id": "b3", "type": "vet", "name": "Half Vet", "lat": 1.28,
        "services": [], "apiBaseUrl": "http://x", "domain": "x.example",
        "documentStores": [], "hmacKeyId": "k", "hmacSecret": "s"
    });
    let b: Business = serde_json::from_value(half).unwrap();
    assert_eq!(b.location(), None, "one coordinate is not a place");
    assert!(!b.location_needs_review());
}
