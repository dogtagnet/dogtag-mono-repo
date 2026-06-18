//! Phase-4 hermetic acceptance (MemChain/MemStore/MemVault — always on, no node/forge required):
//!   (a) appointment ownership + rev allocation (businessB cannot touch businessA's appt; rev never collides)
//!   (b) one-time share JWT (reuse -> 401)
//!   (c) microchip.code uniqueness (duplicate rejected)
//!   (d) erasure (crypto-shred): delete-request + fulfill destroys DEKs incl. verification_records
//!   (e) verify/consent relay (relays to mock verifier /verify/consent/submit + stores a receipt)

mod common;

use axum::http::StatusCode;
use common::*;

use admin_api::auth::{self, hmac_sign, keccak256_hex, sign_jwt};
use admin_api::crypto::KeyVault;

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
    (b["businessId"].as_str().unwrap().to_string(), b["hmacSecret"].as_str().unwrap().to_string())
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
        Some(serde_json::json!({ "businessId": biz_a, "dogTagId": "7", "slot": "2026-07-01T10:00" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "create appt: {appt}");
    assert_eq!(appt["rev"], 1);
    assert_eq!(appt["state"], "REQUESTED");
    let appt_id = appt["id"].as_str().unwrap().to_string();
    // the PUT-to-business was issued.
    assert!(business.calls().iter().any(|c| c.method == "PUT"), "PUT to business A expected");

    // biz B's HMAC key CANNOT post an event for biz A's appointment (ownership C-2).
    let path_b = format!("/v1/businesses/{biz_b}/appointment-events");
    let body_b = serde_json::to_vec(&serde_json::json!({
        "appointmentId": appt_id, "event": "CONFIRMED", "occurredAt": 1
    }))
    .unwrap();
    let sig_b = hmac_sign(&secret_b, "POST", &path_b, &body_b);
    let (s, b) = call_raw(&app, "POST", &path_b, &[("X-DogTag-HMAC", &sig_b)], &body_b).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "biz B must NOT act on biz A's appt: {b}");

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
    let (s, _b) = call_raw(&app, "POST", &path_a, &[("X-DogTag-HMAC", "deadbeef")], &body_a).await;
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

    let (s, b) = call(&app, "POST", &format!("/v1/share/{cred_id}"), Some(&sess), Some(serde_json::json!({}))).await;
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
    let (s, _b) = call(&app, "POST", "/v1/pets", Some(&sess), Some(pet("985141006580319"))).await;
    assert_eq!(s, StatusCode::OK, "first pet");
    // second pet with the SAME microchip code -> 409.
    let (s, b) = call(&app, "POST", "/v1/pets", Some(&sess), Some(pet("985141006580319"))).await;
    assert_eq!(s, StatusCode::CONFLICT, "duplicate microchip must be rejected: {b}");
    // a different code is fine.
    let (s, _b) = call(&app, "POST", "/v1/pets", Some(&sess), Some(pet("985141006580320"))).await;
    assert_eq!(s, StatusCode::OK, "distinct microchip ok");
}

// ============================================================================================
// (d) erasure — crypto-shred incl. verification_records
// ============================================================================================

#[tokio::test]
async fn erasure_crypto_shreds_records_and_deks() {
    // keep a clone of `state` (AppState is Clone, sharing the same Arc store+vault) so we can drive the
    // erasure module against the EXACT collections the router mutates.
    let (state, _chain, vault, business) = hermetic_state();
    let store = state.store.clone();
    let app = admin_api::router(state.clone());
    let wallet = "0x00000000000000000000000000000000000000e4";
    let (owner_id, sess) = signup(&app, "d@x.io", wallet).await;

    // a credential (sealed under a DEK).
    let cred_id = import_a_credential(&app, &sess).await;
    let cred = store.get_credential(&cred_id).await.unwrap();
    let cred_dek = cred.sealed_doc.dek_id.clone();
    assert!(vault.has_dek(&cred_dek).await, "credential DEK exists pre-erasure");

    // a verification_records row (sealed under a DEK) via the REAL relay path (consent receipt + record).
    let admin = admin_token(&app).await;
    let _ = register_business(&app, &admin, "Verifier").await; // documentStore == relayer below
    let relayer = "0x00000000000000000000000000000000000000cc";
    let n = auth::now();
    let claims = admin_api::verify_relay::VerifyClaims {
        iss: "verifier".into(), sub: "sess-x".into(), aud: "dogtag-mobile".into(),
        relayer: relayer.into(), purpose: "BOARDING".into(), record_type: "VACCINATION".into(),
        challenge: "0x00".into(), mode: "normal".into(), exp: n + 180, jti: "vjti-erase".into(),
        verifier_api_base: Some("http://biz.example".into()),
    };
    let session_jwt = sign_jwt(&state.jwt, &claims);
    let consent = serde_json::json!({
        "dogTagId": "7", "recordType": keccak256_hex("VACCINATION"), "purpose": "0x00",
        "relayer": relayer, "subject": wallet, "nonce": "1", "deadline": n + 3600,
    });
    let (s, _b) = call(
        &app, "POST", "/v1/verify/consent", Some(&sess),
        Some(serde_json::json!({ "sessionJwt": session_jwt, "consent": consent, "sig": "0xdead", "mode": "normal" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(business.calls().iter().any(|c| c.url.ends_with("/verify/consent/submit")));
    let vrs = store.verification_records_of_owner(&owner_id).await;
    assert_eq!(vrs.len(), 1, "one verification_record");
    let vr_dek = vrs[0].sealed.dek_id.clone();
    let receipts = store.receipts_of_owner(&owner_id).await;
    assert_eq!(receipts.len(), 1, "one consent receipt");
    let receipt_dek = receipts[0].sealed.dek_id.clone();
    assert!(vault.has_dek(&vr_dek).await && vault.has_dek(&receipt_dek).await, "DEKs exist pre-erasure");

    // delete-request -> deletion{ dueBy: now+45d }.
    let (s, b) = call(&app, "POST", "/v1/privacy/delete-request", Some(&sess), Some(serde_json::json!({ "scope": "all" }))).await;
    assert_eq!(s, StatusCode::OK, "delete-request: {b}");
    let due_by = b["dueBy"].as_u64().unwrap();
    assert!(due_by >= auth::now() + 44 * 24 * 3600, "dueBy ~ now+45d");

    // not yet due -> fulfill does nothing (cron at `now`).
    let fulfilled = admin_api::erasure::fulfill_due_deletions(&state, auth::now()).await;
    assert_eq!(fulfilled, 0, "nothing due yet");
    assert!(store.get_credential(&cred_id).await.is_some(), "credential present pre-due");
    assert!(vault.has_dek(&cred_dek).await, "DEK intact pre-due");

    // at/after dueBy -> fulfill runs erase: crypto-shred everything in scope incl. verification_records.
    let fulfilled = admin_api::erasure::fulfill_due_deletions(&state, due_by + 1).await;
    assert_eq!(fulfilled, 1, "one deletion fulfilled");

    // DEKs destroyed -> ciphertext permanently undecryptable.
    assert!(!vault.has_dek(&cred_dek).await, "credential DEK DESTROYED");
    assert!(!vault.has_dek(&vr_dek).await, "verification_records DEK DESTROYED");
    assert!(!vault.has_dek(&receipt_dek).await, "consent receipt DEK DESTROYED");
    // rows deleted.
    assert!(store.get_credential(&cred_id).await.is_none(), "credential row deleted");
    assert!(store.verification_records_of_owner(&owner_id).await.is_empty(), "verification_records deleted");
    assert!(store.receipts_of_owner(&owner_id).await.is_empty(), "consent receipts deleted");
}

// ============================================================================================
// (e) verify/consent relay
// ============================================================================================

#[tokio::test]
async fn verify_consent_relay_stores_receipt() {
    let (state, _chain, _vault, business) = hermetic_state();
    let store = state.store.clone();
    let app = admin_api::router(state.clone());
    let wallet = "0x00000000000000000000000000000000000000e5";
    let (owner_id, sess) = signup(&app, "e@x.io", wallet).await;

    // register the verifier business (relayer == its documentStore) so discovery resolves verifierApiBase.
    let admin = admin_token(&app).await;
    let relayer = "0x00000000000000000000000000000000000000cc"; // == the registered documentStore
    let (_bid, _secret) = register_business(&app, &admin, "Verifier").await;

    // mint a verifier session JWT (aud dogtag-mobile) signed with the deployment key.
    let n = auth::now();
    let claims = admin_api::verify_relay::VerifyClaims {
        iss: "verifier".into(),
        sub: "sess-1".into(),
        aud: "dogtag-mobile".into(),
        relayer: relayer.into(),
        purpose: "BOARDING".into(),
        record_type: "VACCINATION".into(),
        challenge: "0x00".into(),
        mode: "normal".into(),
        exp: n + 180,
        jti: "vjti-1".into(),
        verifier_api_base: Some("http://biz.example".into()),
    };
    let session_jwt = sign_jwt(&state.jwt, &claims);

    let consent = serde_json::json!({
        "dogTagId": "7",
        "recordType": keccak256_hex("VACCINATION"),
        "purpose": "0x00",
        "relayer": relayer,
        "subject": wallet,
        "nonce": "1",
        "deadline": n + 3600,
    });
    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent",
        Some(&sess),
        Some(serde_json::json!({ "sessionJwt": session_jwt, "consent": consent, "sig": "0xdead", "mode": "normal" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "relay: {b}");
    assert_eq!(b["relayed"], true);

    // relayed to the verifier's /verify/consent/submit.
    assert!(
        business.calls().iter().any(|c| c.url.ends_with("/verify/consent/submit")),
        "relayed to verifier submit endpoint"
    );
    // a receipt + verification_record were stored (off-chain, deletable).
    assert_eq!(store.receipts_of_owner(&owner_id).await.len(), 1, "consent receipt stored");
    assert_eq!(store.verification_records_of_owner(&owner_id).await.len(), 1, "verification_record stored");

    // reusing the SAME session JWT (same jti) -> 401 (one-time consume).
    let (s, _b) = call(
        &app,
        "POST",
        "/v1/verify/consent",
        Some(&sess),
        Some(serde_json::json!({ "sessionJwt": session_jwt, "consent": consent, "sig": "0xdead", "mode": "normal" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "reused session jti must be 401");
}

// ============================================================================================
// (f) central mint -> wrap: a created pet mints an SBT end-to-end and the wrapped DOG_PROFILE VC
//     passes schema validation + integrity verify.
// ============================================================================================

#[tokio::test]
async fn pet_mint_produces_valid_dog_profile_sbt() {
    let (state, _chain, vault, _biz) = hermetic_state();
    let store = state.store.clone();
    let app = admin_api::router(state.clone());
    let wallet = "0x00000000000000000000000000000000000000f1";
    let (_oid, sess) = signup(&app, "mint@x.io", wallet).await;

    // create a pet WITH the DOG_PROFILE fields the schema requires.
    let (s, pet) = call(
        &app,
        "POST",
        "/v1/pets",
        Some(&sess),
        Some(serde_json::json!({
            "name": "Rex",
            "microchip": {
                "code": "985141006580319", "standard": "ISO_11784_11785",
                "implantDate": "2024-01-01", "bodyLocation": "neck"
            },
            "profile": {
                "species": "Canis lupus familiaris",
                "breedVbo": "VBO:0200798",
                "breedLabel": "Labrador Retriever",
                "sex": "male",
                "neuterStatus": "neutered",
                "dateOfBirth": "2022-03-15",
                "weightHistory": [
                    { "unit": "kg", "value": "22.7", "measuredOn": "2024-05-01" }
                ]
            }
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "create pet: {pet}");
    let pet_id = pet["id"].as_str().unwrap().to_string();

    // mint SUCCEEDS and returns {dogTagId, root, txHash}.
    let (s, m) = call(&app, "POST", &format!("/v1/pets/{pet_id}/mint"), Some(&sess), Some(serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::OK, "mint must succeed: {m}");
    assert!(m["dogTagId"].as_str().is_some(), "dogTagId present");
    assert!(m["root"].as_str().is_some(), "root present");
    assert!(m["txHash"].as_str().is_some(), "txHash present");
    assert_eq!(m["recordType"], "DOG_PROFILE");

    // the stored wrapped doc opens, has the returned root, and passes integrity verify.
    let stored = store.get_pet(&pet_id).await.unwrap();
    assert_eq!(stored.dog_tag_id.as_deref(), m["dogTagId"].as_str());
    assert_eq!(stored.root.as_deref(), m["root"].as_str());
    let sealed = stored.sealed_doc.expect("sealed doc stored");
    let doc_val: serde_json::Value =
        admin_api::crypto::open_json(&vault, &sealed).await.expect("open sealed doc");
    let doc: dogtag_standard::wrap::WrappedDoc =
        serde_json::from_value(doc_val).expect("stored doc is a WrappedDoc");
    assert_eq!(doc.signature.merkle_root, m["root"].as_str().unwrap(), "stored root == returned root");
    assert!(admin_api::verify::structural_valid(&doc), "wrapped DOG_PROFILE VC integrity must verify");
    // the non-personal dogTagId is the disclosed reference identity.
    assert_eq!(
        admin_api::verify::dog_tag_id_of(&doc).as_deref(),
        m["dogTagId"].as_str(),
        "dogTagId disclosed in the wrapped doc",
    );
}

#[tokio::test]
async fn pet_mint_fills_defaults_when_profile_omitted() {
    // even with NO profile fields supplied, mint must still emit a schema-valid DOG_PROFILE VC.
    let (state, _chain, vault, _biz) = hermetic_state();
    let store = state.store.clone();
    let app = admin_api::router(state.clone());
    let (_oid, sess) = signup(&app, "mint2@x.io", "0x00000000000000000000000000000000000000f2").await;

    let (s, pet) = call(
        &app,
        "POST",
        "/v1/pets",
        Some(&sess),
        Some(serde_json::json!({
            "name": "Buddy",
            "microchip": { "code": "985141006580320", "standard": "ISO_11784_11785", "implantDate": "2024-01-01", "bodyLocation": "neck" }
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "create pet: {pet}");
    let pet_id = pet["id"].as_str().unwrap().to_string();

    let (s, m) = call(&app, "POST", &format!("/v1/pets/{pet_id}/mint"), Some(&sess), Some(serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::OK, "mint with default profile must succeed: {m}");

    let stored = store.get_pet(&pet_id).await.unwrap();
    let sealed = stored.sealed_doc.expect("sealed doc stored");
    let doc_val: serde_json::Value =
        admin_api::crypto::open_json(&vault, &sealed).await.expect("open sealed doc");
    let doc: dogtag_standard::wrap::WrappedDoc = serde_json::from_value(doc_val).expect("WrappedDoc");
    assert!(admin_api::verify::structural_valid(&doc), "defaulted DOG_PROFILE VC integrity must verify");
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

