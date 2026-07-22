//! Phase-4 hermetic acceptance (MemChain/MemStore/MemVault — always on, no node/forge required):
//!   (a) appointment ownership + rev allocation (businessB cannot touch businessA's appt; rev never collides)
//!   (b) one-time share JWT (reuse -> 401)
//!   (c) microchip.code uniqueness (duplicate rejected)
//!   (d) erasure (crypto-shred): delete-request + fulfill destroys DEKs incl. historical records

mod common;

use axum::http::StatusCode;
use common::*;

use admin_api::auth::{self, hmac_sign};
use admin_api::crypto::{seal_json, KeyVault};
use admin_api::store::{ConsentReceipt, VerificationRecord};

// --------------------------------------------------------------------------------------------
// helper: register a business (admin) -> (businessId, hmacSecret).
// --------------------------------------------------------------------------------------------
async fn register_business(app: &axum::Router, admin: &str, name: &str) -> (String, String) {
    let (s, b) = call(
        app,
        "POST",
        "/v1/businesses",
        Some(admin),
        Some(serde_json::json!({
            "type": "vet", "name": name, "lat": 37.0, "lng": -122.0,
            "services": ["exam"], "apiBaseUrl": "http://biz.example", "domain": "biz.example",
            "documentStores": ["0x00000000000000000000000000000000000000cc"]
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "register business: {b}");
    (
        b["businessId"].as_str().unwrap().to_string(),
        b["hmacSecret"].as_str().unwrap().to_string(),
    )
}

// ============================================================================================
// (a) appointment ownership + sole-rev-allocator
// ============================================================================================

#[tokio::test]
async fn appointment_ownership_and_rev_allocation() {
    let (state, _chain, _vault, business) = hermetic_state();
    let app = admin_api::router(state);
    let admin = admin_token(&app).await;
    let (_oid, sess) = signup(&app, "a@x.io", "0x00000000000000000000000000000000000000e1").await;

    let (biz_a, secret_a) = register_business(&app, &admin, "Biz A").await;
    let (biz_b, secret_b) = register_business(&app, &admin, "Biz B").await;

    // owner creates an appointment with biz A (rev:1 REQUESTED) -> PUT to biz A.
    let (s, appt) = call(
        &app,
        "POST",
        "/v1/appointments",
        Some(&sess),
        Some(
            serde_json::json!({ "businessId": biz_a, "dogTagId": "7", "slot": "2026-07-01T10:00" }),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "create appt: {appt}");
    assert_eq!(appt["rev"], 1);
    assert_eq!(appt["state"], "REQUESTED");
    let appt_id = appt["id"].as_str().unwrap().to_string();
    // the PUT-to-business was issued.
    assert!(
        business.calls().iter().any(|c| c.method == "PUT"),
        "PUT to business A expected"
    );

    // biz B's HMAC key CANNOT post an event for biz A's appointment (ownership C-2).
    let path_b = format!("/v1/businesses/{biz_b}/appointment-events");
    let body_b = serde_json::to_vec(&serde_json::json!({
        "appointmentId": appt_id, "event": "CONFIRMED", "occurredAt": 1
    }))
    .unwrap();
    let sig_b = hmac_sign(&secret_b, "POST", &path_b, &body_b);
    let (s, b) = call_raw(&app, "POST", &path_b, &[("X-DogTag-HMAC", &sig_b)], &body_b).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "biz B must NOT act on biz A's appt: {b}"
    );

    // a VALID event from biz A bumps rev 1 -> 2 and applies CONFIRMED.
    let path_a = format!("/v1/businesses/{biz_a}/appointment-events");
    let body_a = serde_json::to_vec(&serde_json::json!({
        "appointmentId": appt_id, "event": "CONFIRMED", "occurredAt": 2
    }))
    .unwrap();
    let sig_a = hmac_sign(&secret_a, "POST", &path_a, &body_a);
    let (s, b) = call_raw(&app, "POST", &path_a, &[("X-DogTag-HMAC", &sig_a)], &body_a).await;
    assert_eq!(s, StatusCode::OK, "valid event: {b}");
    assert_eq!(b["rev"], 2, "central bumped rev");
    assert_eq!(b["state"], "CONFIRMED");

    // a tampered/bad HMAC is rejected.
    let (s, _b) = call_raw(
        &app,
        "POST",
        &path_a,
        &[("X-DogTag-HMAC", "deadbeef")],
        &body_a,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "bad HMAC must be 401");

    // rev never collides under concurrent-ish creates: many appts each start at distinct revs and
    // bump monotonically. Drive a burst of events at the SAME appt and assert strictly-increasing revs.
    let mut last = 2u64;
    for i in 0..10 {
        let body = serde_json::to_vec(&serde_json::json!({
            "appointmentId": appt_id, "event": "CONFIRMED", "occurredAt": 10 + i
        }))
        .unwrap();
        let sig = hmac_sign(&secret_a, "POST", &path_a, &body);
        let (s, b) = call_raw(&app, "POST", &path_a, &[("X-DogTag-HMAC", &sig)], &body).await;
        assert_eq!(s, StatusCode::OK);
        let rev = b["rev"].as_u64().unwrap();
        assert!(rev > last, "rev must strictly increase: {rev} !> {last}");
        last = rev;
    }
}

// ============================================================================================
// (b) one-time share JWT
// ============================================================================================

#[tokio::test]
async fn one_time_share_jwt() {
    let (state, _chain, _vault, _biz) = hermetic_state();
    let app = admin_api::router(state);
    let (_oid, sess) = signup(&app, "b@x.io", "0x00000000000000000000000000000000000000e2").await;

    // import a credential so there is something to share.
    let cred_id = import_a_credential(&app, &sess).await;

    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/share/{cred_id}"),
        Some(&sess),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "share: {b}");
    let ref_id = b["ref"].as_str().unwrap().to_string();
    let token = b["token"].as_str().unwrap().to_string();

    // first GET /share/{ref} succeeds (business pulls the doc).
    let (s, doc) = call(&app, "GET", &format!("/share/{ref_id}"), Some(&token), None).await;
    assert_eq!(s, StatusCode::OK, "first share fetch: {doc}");
    assert!(doc.get("signature").is_some(), "returned a wrapped doc");

    // reuse the SAME token -> 401 (one-time jti consumed).
    let (s, _b) = call(&app, "GET", &format!("/share/{ref_id}"), Some(&token), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "reused share JWT must be 401");
}

// ============================================================================================
// (c) microchip uniqueness
// ============================================================================================

#[tokio::test]
async fn microchip_uniqueness() {
    let (state, _chain, _vault, _biz) = hermetic_state();
    let app = admin_api::router(state);
    let (_oid, sess) = signup(&app, "c@x.io", "0x00000000000000000000000000000000000000e3").await;

    let pet = |code: &str| {
        serde_json::json!({
            "name": "Rex",
            "microchip": { "code": code, "standard": "ISO_11784_11785", "implantDate": "2024-01-01", "bodyLocation": "neck" }
        })
    };
    let (s, _b) = call(
        &app,
        "POST",
        "/v1/pets",
        Some(&sess),
        Some(pet("985141006580319")),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "first pet");
    // second pet with the SAME microchip code -> 409.
    let (s, b) = call(
        &app,
        "POST",
        "/v1/pets",
        Some(&sess),
        Some(pet("985141006580319")),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "duplicate microchip must be rejected: {b}"
    );
    // a different code is fine.
    let (s, _b) = call(
        &app,
        "POST",
        "/v1/pets",
        Some(&sess),
        Some(pet("985141006580320")),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "distinct microchip ok");
}

#[tokio::test]
async fn retired_mint_and_central_consent_routes_are_absent() {
    let (state, _chain, _vault, _business) = hermetic_state();
    let app = admin_api::router(state);
    let (_owner_id, session) = signup(
        &app,
        "routes@x.io",
        "0x00000000000000000000000000000000000000e6",
    )
    .await;

    let (status, _) = call(
        &app,
        "POST",
        "/v1/pets/retired/mint",
        Some(&session),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = call(
        &app,
        "POST",
        "/v1/verify/consent",
        Some(&session),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ============================================================================================
// (d) erasure — crypto-shred incl. verification_records
// ============================================================================================

#[tokio::test]
async fn erasure_crypto_shreds_records_and_deks() {
    // keep a clone of `state` (AppState is Clone, sharing the same Arc store+vault) so we can drive the
    // erasure module against the EXACT collections the router mutates.
    let (state, _chain, vault, _business) = hermetic_state();
    let store = state.store.clone();
    let app = admin_api::router(state.clone());
    let wallet = "0x00000000000000000000000000000000000000e4";
    let (owner_id, sess) = signup(&app, "d@x.io", wallet).await;

    // a credential (sealed under a DEK).
    let cred_id = import_a_credential(&app, &sess).await;
    let cred = store.get_credential(&cred_id).await.unwrap();
    let cred_dek = cred.sealed_doc.dek_id.clone();
    assert!(
        vault.has_dek(&cred_dek).await,
        "credential DEK exists pre-erasure"
    );

    // Seed historical verification/receipt rows directly. The retired central relay no longer creates
    // these, but erasure must still destroy rows written before the owner-hidden cutover.
    let relayer = "0x00000000000000000000000000000000000000cc";
    let n = auth::now();
    let vr_sealed = seal_json(
        &vault,
        &serde_json::json!({ "dogTagId": "7", "purpose": "BOARDING" }),
    )
    .await
    .unwrap();
    let vr_dek = vr_sealed.dek_id.clone();
    store
        .put_verification_record(VerificationRecord {
            record_id: "historical-verification".into(),
            owner_id: owner_id.clone(),
            dog_tag_id: "7".into(),
            purpose: "BOARDING".into(),
            relayer: relayer.into(),
            status: "recorded".into(),
            sealed: vr_sealed,
        })
        .await;
    let receipt_sealed = seal_json(&vault, &serde_json::json!({ "dogTagId": "7" }))
        .await
        .unwrap();
    let receipt_dek = receipt_sealed.dek_id.clone();
    store
        .put_consent_receipt(ConsentReceipt {
            receipt_id: "historical-receipt".into(),
            owner_id: owner_id.clone(),
            hash: "0x01".into(),
            issued_at: n,
            sealed: receipt_sealed,
        })
        .await;
    let vrs = store.verification_records_of_owner(&owner_id).await;
    assert_eq!(vrs.len(), 1, "one verification_record");
    let receipts = store.receipts_of_owner(&owner_id).await;
    assert_eq!(receipts.len(), 1, "one consent receipt");
    assert!(
        vault.has_dek(&vr_dek).await && vault.has_dek(&receipt_dek).await,
        "DEKs exist pre-erasure"
    );

    // delete-request -> deletion{ dueBy: now+45d }.
    let (s, b) = call(
        &app,
        "POST",
        "/v1/privacy/delete-request",
        Some(&sess),
        Some(serde_json::json!({ "scope": "all" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "delete-request: {b}");
    let due_by = b["dueBy"].as_u64().unwrap();
    assert!(due_by >= auth::now() + 44 * 24 * 3600, "dueBy ~ now+45d");

    // not yet due -> fulfill does nothing (cron at `now`).
    let fulfilled = admin_api::erasure::fulfill_due_deletions(&state, auth::now()).await;
    assert_eq!(fulfilled, 0, "nothing due yet");
    assert!(
        store.get_credential(&cred_id).await.is_some(),
        "credential present pre-due"
    );
    assert!(vault.has_dek(&cred_dek).await, "DEK intact pre-due");

    // at/after dueBy -> fulfill runs erase: crypto-shred everything in scope incl. verification_records.
    let fulfilled = admin_api::erasure::fulfill_due_deletions(&state, due_by + 1).await;
    assert_eq!(fulfilled, 1, "one deletion fulfilled");

    // DEKs destroyed -> ciphertext permanently undecryptable.
    assert!(!vault.has_dek(&cred_dek).await, "credential DEK DESTROYED");
    assert!(
        !vault.has_dek(&vr_dek).await,
        "verification_records DEK DESTROYED"
    );
    assert!(
        !vault.has_dek(&receipt_dek).await,
        "consent receipt DEK DESTROYED"
    );
    // rows deleted.
    assert!(
        store.get_credential(&cred_id).await.is_none(),
        "credential row deleted"
    );
    assert!(
        store
            .verification_records_of_owner(&owner_id)
            .await
            .is_empty(),
        "verification_records deleted"
    );
    assert!(
        store.receipts_of_owner(&owner_id).await.is_empty(),
        "consent receipts deleted"
    );
}

#[tokio::test]
async fn import_projects_unified_default_provenance() {
    // Importing an unstamped document projects the single owner-hidden protocol metadata into the
    // queryable columns.
    let (state, _chain, _vault, _biz) = hermetic_state();
    let store = state.store.clone();
    let app = admin_api::router(state);
    let (_oid, sess) = signup(
        &app,
        "prov@x.io",
        "0x00000000000000000000000000000000000000e9",
    )
    .await;
    let cred_id = import_a_credential(&app, &sess).await; // build_sample_wrapped_doc has no protocol block

    let cred = store
        .get_credential(&cred_id)
        .await
        .expect("credential persisted");
    assert_eq!(cred.chain_id, Some(135));
    assert_eq!(cred.protocol_version.as_deref(), Some("dogtag-levelb/1"));
    assert_eq!(
        cred.verification_registry.as_deref(),
        Some("0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87")
    );
    // issuerClone defaults to the imported doc's own documentStore.
    assert_eq!(
        cred.issuer_addr.as_deref(),
        Some("0x0000000000000000000000000000000000000001")
    );
    // An unstamped import cannot resolve the on-chain issuer signer, so it remains absent.
    assert_eq!(cred.issuer_signer, None);
}

// ============================================================================================
// (e) approve → grant DogTagSBT.ISSUER_ROLE for dog-tag issuers (DOG_PROFILE), NOT for groomers.
// ============================================================================================

/// Submit an issuer-application and return its applicationId.
async fn submit_application(app: &axum::Router, body: serde_json::Value) -> String {
    let (s, b) = call(app, "POST", "/v1/issuer-applications", None, Some(body)).await;
    assert_eq!(s, StatusCode::OK, "submit application: {b}");
    b["applicationId"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn approve_dog_profile_grants_issuer_role() {
    use admin_api::chain::ChainClient;

    let (state, chain, _vault, _biz) = hermetic_state();
    let sbt = state.cfg.sbt_addr.clone();
    let app = admin_api::router(state);
    let admin = admin_token(&app).await;

    let vet_addr = "0x70997970c51812dc3a010c7d01b50e0d17dc79c8";
    let groomer_addr = "0x3c44cdddb6a900fa2b585dd299e03d12fa4293bc";

    // (1) a dog-tag issuer (recordTypes include DOG_PROFILE) — approve must grant ISSUER_ROLE.
    let vet_app = submit_application(
        &app,
        serde_json::json!({
            "issuerEntityId": "bayview-vet",
            "addresses": [vet_addr],
            "recordTypes": ["VACCINATION", "DOG_PROFILE"],
            "domain": "vet.example",
            "documentStore": "0x00000000000000000000000000000000000000cc",
            "usdaNan": "123456",
        }),
    )
    .await;
    assert!(
        !chain.has_issuer_role(&sbt, vet_addr).await.unwrap(),
        "no role before approve"
    );
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/issuer-applications/{vet_app}/approve"),
        Some(&admin),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "approve vet: {b}");
    assert_eq!(
        b["issuerRoleGranted"], true,
        "dog-tag issuer must be granted ISSUER_ROLE"
    );
    assert!(
        b["issuerRoleTxHash"].as_str().is_some(),
        "issuerRoleTxHash present"
    );
    assert!(
        chain.has_issuer_role(&sbt, vet_addr).await.unwrap(),
        "ISSUER_ROLE granted on the SBT"
    );

    // (2) a groomer (VERIFY-only, no DOG_PROFILE) — approve must NOT grant ISSUER_ROLE.
    let groomer_app = submit_application(
        &app,
        serde_json::json!({
            "issuerEntityId": "pawsh-groomer",
            "addresses": [groomer_addr],
            "recordTypes": ["VACCINATION"],
            "verifyPurposes": ["grooming_intake"],
            "domain": "groomer.example",
            "documentStore": "0x00000000000000000000000000000000000000cc",
        }),
    )
    .await;
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/issuer-applications/{groomer_app}/approve"),
        Some(&admin),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "approve groomer: {b}");
    assert_eq!(
        b["issuerRoleGranted"], false,
        "groomer (no DOG_PROFILE) must NOT get ISSUER_ROLE"
    );
    assert!(
        b["issuerRoleTxHash"].is_null(),
        "no issuer-role tx for a groomer"
    );
    assert!(
        !chain.has_issuer_role(&sbt, groomer_addr).await.unwrap(),
        "groomer holds no ISSUER_ROLE"
    );
}

// --------------------------------------------------------------------------------------------
// test helpers
// --------------------------------------------------------------------------------------------

/// Import a fully-disclosed wrapped doc (built via the SDK) and return its credentialId.
async fn import_a_credential(app: &axum::Router, sess: &str) -> String {
    let doc = build_sample_wrapped_doc();
    let (s, b) = call(
        app,
        "POST",
        "/v1/credentials/import",
        Some(sess),
        Some(serde_json::json!({ "wrappedDoc": doc })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "import: {b}");
    b["credentialId"].as_str().unwrap().to_string()
}

/// Build a valid DOG_PROFILE-style wrapped doc using the SDK's `wrap_document` (so structural verify passes).
fn build_sample_wrapped_doc() -> serde_json::Value {
    use dogtag_standard::wrap::{wrap_document, IssuerMeta};
    let vc = serde_json::json!({
        "credentialSubject": {
            "dogTagId": { "tag": 3, "value": "7" },
            "name": { "tag": 2, "value": "Rex" }
        }
    });
    let meta = IssuerMeta {
        name: "Vet".into(),
        domain: "vet.example".into(),
        document_store: "0x0000000000000000000000000000000000000001".into(),
        record_type: "VACCINATION".into(),
    };
    let mut n: u8 = 1;
    let mut salt = move || {
        let s = [n; 16];
        n = n.wrapping_add(1);
        s
    };
    let doc = wrap_document(&vc, meta, &mut salt).unwrap();
    serde_json::to_value(&doc).unwrap()
}
