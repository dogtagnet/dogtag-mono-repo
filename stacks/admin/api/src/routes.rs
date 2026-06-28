//! Axum router + all central HTTP handlers (impl §4.1 mobile, §4.2 registry/discovery,
//! §4.3 whitelisting, §4.4 appointments, §4.5 consent/retention/erasure; §11.4 asserts).

use std::net::SocketAddr;

use axum::{
    body::Bytes,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{self, AppState, DOG_PROFILE};
use crate::auth::{self, keccak256_hex, ShareClaims};
use crate::chain::{record_type_key, verify_key};
use crate::crypto;
use crate::store::*;

type Resp = (StatusCode, Json<Value>);

fn ok(v: Value) -> Resp {
    (StatusCode::OK, Json(v))
}
fn err(code: StatusCode, msg: &str) -> Resp {
    (code, Json(json!({ "error": msg })))
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

/// Client IP for rate-limiting: prefer the first `X-Forwarded-For` hop (prod is behind Caddy),
/// else the raw socket peer (absent under in-process tests -> a stable fallback key).
fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| peer.map(|p| p.ip().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Liveness probe (no auth): used by the compose healthcheck.
async fn health() -> Resp {
    ok(json!({ "status": "ok" }))
}

/// Resolve the authenticated owner from a session bearer.
async fn require_owner(st: &AppState, headers: &HeaderMap) -> Result<String, Resp> {
    let token = bearer(headers).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing session"))?;
    st.store
        .session_owner(&token)
        .await
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "invalid session"))
}

/// Require a valid admin session bearer.
async fn require_admin(st: &AppState, headers: &HeaderMap) -> Result<(), Resp> {
    let token =
        bearer(headers).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing admin session"))?;
    if st.store.has_admin_session(&token).await {
        Ok(())
    } else {
        Err(err(StatusCode::UNAUTHORIZED, "invalid admin session"))
    }
}

// ============================================================================================
// §4.1 Mobile API — auth
// ============================================================================================

#[derive(Deserialize)]
struct SignupReq {
    email: String,
    password: String,
    #[serde(rename = "walletAddress")]
    wallet_address: String,
    #[serde(rename = "pushToken", default)]
    push_token: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

async fn signup(State(st): State<AppState>, Json(body): Json<SignupReq>) -> Resp {
    if st.store.get_owner_by_email(&body.email).await.is_some() {
        return err(StatusCode::CONFLICT, "email already registered");
    }
    let owner_id = uuid::Uuid::new_v4().to_string();
    let profile_pii = if let Some(name) = &body.name {
        crypto::seal_json(st.vault.as_ref(), &json!({ "name": name }))
            .await
            .ok()
    } else {
        None
    };
    let owner = Owner {
        owner_id: owner_id.clone(),
        email: Some(body.email.clone()),
        password_hash: Some(auth::hash_password(&body.password)),
        wallet_address: body.wallet_address.to_lowercase(),
        push_token: body.push_token,
        profile_pii,
    };
    st.store.put_owner(owner).await;
    let token = auth::new_session_token("sess");
    st.store.put_session(token.clone(), owner_id.clone()).await;
    ok(json!({ "ownerId": owner_id, "token": token }))
}

#[derive(Deserialize)]
struct LoginReq {
    email: String,
    password: String,
    #[serde(rename = "pushToken", default)]
    push_token: Option<String>,
}

async fn login(
    State(st): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<LoginReq>,
) -> Resp {
    let ip = client_ip(&headers, peer.map(|ConnectInfo(p)| p));
    if st.ratelimit.is_locked(&ip) {
        return err(
            StatusCode::TOO_MANY_REQUESTS,
            "too many attempts; try again later",
        );
    }
    let owner = match st.store.get_owner_by_email(&body.email).await {
        Some(o) => o,
        None => {
            st.ratelimit.record_failure(&ip);
            return err(StatusCode::UNAUTHORIZED, "bad credentials");
        }
    };
    // wallet-only owners have no password_hash -> password login is not available for them.
    let stored_hash = match &owner.password_hash {
        Some(h) => h,
        None => {
            st.ratelimit.record_failure(&ip);
            return err(StatusCode::UNAUTHORIZED, "bad credentials");
        }
    };
    if !auth::verify_password(&body.password, stored_hash) {
        st.ratelimit.record_failure(&ip);
        return err(StatusCode::UNAUTHORIZED, "bad credentials");
    }
    st.ratelimit.record_success(&ip);
    if let Some(pt) = body.push_token {
        let mut o = owner.clone();
        o.push_token = Some(pt);
        st.store.put_owner(o).await;
    }
    let token = auth::new_session_token("sess");
    st.store
        .put_session(token.clone(), owner.owner_id.clone())
        .await;
    ok(json!({ "ownerId": owner.owner_id, "token": token }))
}

#[derive(Deserialize)]
struct AdminLoginReq {
    password: String,
}

async fn admin_login(
    State(st): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<AdminLoginReq>,
) -> Resp {
    let ip = client_ip(&headers, peer.map(|ConnectInfo(p)| p));
    if st.ratelimit.is_locked(&ip) {
        return err(
            StatusCode::TOO_MANY_REQUESTS,
            "too many attempts; try again later",
        );
    }
    if !auth::verify_password(&body.password, &auth::hash_password(&st.cfg.admin_password))
        && body.password != st.cfg.admin_password
    {
        st.ratelimit.record_failure(&ip);
        return err(StatusCode::UNAUTHORIZED, "bad password");
    }
    st.ratelimit.record_success(&ip);
    let token = auth::new_session_token("admin");
    st.store.put_admin_session(token.clone()).await;
    ok(json!({ "token": token }))
}

// ============================================================================================
// §4.1 Pets + mint
// ============================================================================================

#[derive(Deserialize)]
struct CreatePetReq {
    name: String,
    microchip: Microchip,
    /// optional DOG_PROFILE identity fields (species/breed/sex/neuterStatus/dateOfBirth/weightHistory).
    #[serde(default)]
    profile: PetProfile,
}

async fn list_pets(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    let pets: Vec<Value> = st
        .store
        .pets_of_owner(&owner_id)
        .await
        .into_iter()
        .map(pet_json)
        .collect();
    ok(json!({ "pets": pets }))
}

fn pet_json(p: Pet) -> Value {
    json!({
        "id": p.pet_id,
        "name": p.name,
        "microchip": {
            "code": p.microchip.code,
            "standard": p.microchip.standard,
            "implantDate": p.microchip.implant_date,
            "bodyLocation": p.microchip.body_location,
        },
        "dogTagId": p.dog_tag_id,
        "root": p.root,
        "mintTx": p.mint_tx,
    })
}

async fn create_pet(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreatePetReq>,
) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    // enforce microchip.code uniqueness ATOMICALLY (reserve returns false if already taken).
    if !st.store.reserve_microchip(&body.microchip.code).await {
        return err(StatusCode::CONFLICT, "microchip.code already registered");
    }
    let pet = Pet {
        pet_id: uuid::Uuid::new_v4().to_string(),
        owner_id,
        name: body.name,
        microchip: body.microchip,
        profile: body.profile,
        dog_tag_id: None,
        root: None,
        mint_tx: None,
        sealed_doc: None,
    };
    st.store.put_pet(pet.clone()).await;
    ok(pet_json(pet))
}

async fn mint_pet(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    let mut pet = match st.store.get_pet(&id).await {
        Some(p) if p.owner_id == owner_id => p,
        Some(_) => return err(StatusCode::FORBIDDEN, "not your pet"),
        None => return err(StatusCode::NOT_FOUND, "pet not found"),
    };
    if pet.dog_tag_id.is_some() {
        return err(StatusCode::CONFLICT, "already minted");
    }
    let owner = match st.store.get_owner(&owner_id).await {
        Some(o) => o,
        None => return err(StatusCode::NOT_FOUND, "owner not found"),
    };
    // allocate the non-personal dogTagId, build + wrap the DOG_PROFILE VC -> root.
    let dog_tag_id = st.store.next_dog_tag_id().await.to_string();
    let meta = app::profile_issuer_meta(&st.cfg);
    // owner-session mint does not collect ownerIdentity; emit empty-string fields (schema only
    // requires the keys present as strings).
    let owner_identity = OwnerIdentity::default();
    let vc = app::build_profile_vc(
        &st.cfg,
        &pet.name,
        &pet.microchip,
        &pet.profile,
        &owner_identity,
        &dog_tag_id,
    );
    let doc = match app::wrap_vc(meta, &vc) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::BAD_REQUEST, &e),
    };
    let root = doc.signature.merkle_root.clone();
    // central protocol signer mints the SBT to the USER'S wallet.
    let sent = match st
        .chain
        .mint(
            st.cfg.admin_signer_index,
            &st.cfg.sbt_addr,
            &owner.wallet_address,
            &dog_tag_id,
            &root,
        )
        .await
    {
        Ok(s) => s,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("mint: {e}")),
    };
    // store the wrapped doc encrypted under a per-record DEK (erasure scope).
    let sealed = match crypto::seal_json(st.vault.as_ref(), &doc).await {
        Ok(s) => s,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "seal failed"),
    };
    pet.dog_tag_id = Some(dog_tag_id.clone());
    pet.root = Some(root.clone());
    pet.mint_tx = Some(sent.tx_hash.clone());
    pet.sealed_doc = Some(sealed);
    st.store.put_pet(pet).await;
    ok(json!({
        "dogTagId": dog_tag_id,
        "root": root,
        "txHash": sent.tx_hash,
        "ownerWallet": owner.wallet_address,
        "recordType": DOG_PROFILE,
    }))
}

// ============================================================================================
// §4.1 Credentials + import + share
// ============================================================================================

async fn list_credentials(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    let creds: Vec<Value> = st
        .store
        .credentials_of_owner(&owner_id)
        .await
        .into_iter()
        .map(|c| json!({ "id": c.credential_id, "dogTagId": c.dog_tag_id, "root": c.root }))
        .collect();
    ok(json!({ "credentials": creds }))
}

#[derive(Deserialize)]
struct ImportReq {
    #[serde(rename = "wrappedDoc")]
    wrapped_doc: Value,
}

async fn import_credential(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ImportReq>,
) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    // parse + minimally verify via the SDK (structural integrity check; full on-chain verify needs a
    // live RPC which the hermetic path lacks — we assert the doc is a well-formed WrappedDoc whose
    // recomputed root matches the embedded merkleRoot).
    let doc: dogtag_standard::wrap::WrappedDoc =
        match serde_json::from_value(body.wrapped_doc.clone()) {
            Ok(d) => d,
            Err(e) => return err(StatusCode::BAD_REQUEST, &format!("not a WrappedDoc: {e}")),
        };
    if !crate::verify::structural_valid(&doc) {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrapped doc integrity invalid",
        );
    }
    let dog_tag_id = crate::verify::dog_tag_id_of(&doc).unwrap_or_else(|| "unknown".to_string());
    let sealed = match crypto::seal_json(st.vault.as_ref(), &body.wrapped_doc).await {
        Ok(s) => s,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "seal failed"),
    };
    let credential_id = uuid::Uuid::new_v4().to_string();
    st.store
        .put_credential(Credential {
            credential_id: credential_id.clone(),
            owner_id,
            dog_tag_id,
            root: doc.signature.merkle_root.clone(),
            sealed_doc: sealed,
        })
        .await;
    ok(json!({ "credentialId": credential_id, "root": doc.signature.merkle_root }))
}

/// POST /v1/share/{credentialId} — mint a one-time JWT (aud dogtag-business) + a share ref.
async fn share_credential(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(cred_id): Path<String>,
) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    let cred = match st.store.get_credential(&cred_id).await {
        Some(c) if c.owner_id == owner_id => c,
        Some(_) => return err(StatusCode::FORBIDDEN, "not your credential"),
        None => return err(StatusCode::NOT_FOUND, "credential not found"),
    };
    let ref_id = uuid::Uuid::new_v4().to_string();
    st.store
        .put_share_ref(ShareRef {
            ref_id: ref_id.clone(),
            credential_id: cred.credential_id.clone(),
            owner_id,
        })
        .await;
    let n = auth::now();
    let claims = ShareClaims {
        iss: st.cfg.deployment_url.clone(),
        sub: ref_id.clone(),
        aud: "dogtag-business".to_string(),
        scope: "read:credential".to_string(),
        iat: n,
        nbf: n,
        exp: n + 180,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    let token = auth::sign_jwt(&st.jwt, &claims);
    ok(json!({ "ref": ref_id, "token": token }))
}

/// GET /share/{ref} Bearer<jwt> — mirrors the business-side asserts (impl §11.4 C-1):
/// sub==ref && aud=="dogtag-business" && scope check && atomic one-time jti consume (401 if reused).
async fn get_share(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(ref_id): Path<String>,
) -> Resp {
    let token = match bearer(&headers) {
        Some(t) => t,
        None => return err(StatusCode::UNAUTHORIZED, "missing share JWT"),
    };
    let claims: ShareClaims = match auth::verify_jwt(&st.jwt, &token, 30) {
        Ok(c) => c,
        Err(e) => return err(StatusCode::UNAUTHORIZED, &format!("jwt: {e}")),
    };
    if claims.sub != ref_id {
        return err(StatusCode::UNAUTHORIZED, "sub != ref");
    }
    if claims.aud != "dogtag-business" {
        return err(StatusCode::UNAUTHORIZED, "bad audience");
    }
    if claims.scope != "read:credential" {
        return err(StatusCode::UNAUTHORIZED, "bad scope");
    }
    // atomic one-time jti consume — 401 if reused.
    if !st.store.consume_jti(&claims.jti).await {
        return err(StatusCode::UNAUTHORIZED, "jti already used");
    }
    let share = match st.store.get_share_ref(&ref_id).await {
        Some(s) => s,
        None => return err(StatusCode::NOT_FOUND, "share ref not found"),
    };
    let cred = match st.store.get_credential(&share.credential_id).await {
        Some(c) => c,
        None => return err(StatusCode::NOT_FOUND, "credential not found"),
    };
    match crypto::open_json::<Value>(st.vault.as_ref(), &cred.sealed_doc).await {
        Ok(doc) => ok(doc),
        Err(_) => err(StatusCode::GONE, "credential erased"),
    }
}

// ============================================================================================
// §4.1 verify/consent relay + receipts
// ============================================================================================

#[derive(Deserialize)]
struct VerifyConsentReq {
    #[serde(rename = "sessionJwt")]
    session_jwt: String,
    consent: Value,
    sig: String,
    #[serde(default)]
    mode: Option<String>,
}

async fn verify_consent(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<VerifyConsentReq>,
) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    crate::verify_relay::relay(
        &st,
        &owner_id,
        body.session_jwt,
        body.consent,
        body.sig,
        body.mode,
    )
    .await
}

async fn verify_receipts(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    let recs: Vec<Value> = st
        .store
        .verification_records_of_owner(&owner_id)
        .await
        .into_iter()
        .map(|v| {
            json!({
                "id": v.record_id, "dogTagId": v.dog_tag_id, "purpose": v.purpose,
                "relayer": v.relayer, "mode": v.mode, "status": v.status,
            })
        })
        .collect();
    ok(json!({ "receipts": recs }))
}

// ============================================================================================
// §4.2 Registry / discovery
// ============================================================================================

#[derive(Deserialize)]
struct BusinessesQuery {
    #[serde(rename = "type")]
    kind: Option<String>,
    near: Option<String>, // "lat,lng"
    radius: Option<f64>,  // km
}

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0_f64;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dphi = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().asin()
}

async fn list_businesses(State(st): State<AppState>, Query(q): Query<BusinessesQuery>) -> Resp {
    let near = q.near.as_ref().and_then(|s| {
        let mut it = s.split(',');
        Some((
            it.next()?.trim().parse::<f64>().ok()?,
            it.next()?.trim().parse::<f64>().ok()?,
        ))
    });
    let radius = q.radius.unwrap_or(50.0);
    let out: Vec<Value> = st
        .store
        .all_businesses()
        .await
        .into_iter()
        .filter(|b| q.kind.as_ref().map(|k| &b.kind == k).unwrap_or(true))
        .filter(|b| match near {
            Some((lat, lng)) => haversine_km(lat, lng, b.lat, b.lng) <= radius,
            None => true,
        })
        .map(|b| {
            // non-personal fields only — NEVER the HMAC secret.
            json!({
                "businessId": b.business_id, "type": b.kind, "name": b.name,
                "geo": { "lat": b.lat, "lng": b.lng }, "services": b.services,
                "apiBaseUrl": b.api_base_url, "domain": b.domain,
                "documentStores": b.document_stores, "hmacKeyId": b.hmac_key_id,
            })
        })
        .collect();
    ok(json!({ "businesses": out }))
}

#[derive(Deserialize)]
struct RegisterBusinessReq {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    lat: f64,
    lng: f64,
    #[serde(default)]
    services: Vec<String>,
    #[serde(rename = "apiBaseUrl")]
    api_base_url: String,
    domain: String,
    #[serde(rename = "documentStores", default)]
    document_stores: Vec<String>,
}

async fn register_business(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RegisterBusinessReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let business_id = uuid::Uuid::new_v4().to_string();
    let hmac_key_id = format!("key_{}", uuid::Uuid::new_v4());
    let hmac_secret = auth::new_session_token("hsec");
    let biz = Business {
        business_id: business_id.clone(),
        kind: body.kind,
        name: body.name,
        lat: body.lat,
        lng: body.lng,
        services: body.services,
        api_base_url: body.api_base_url,
        domain: body.domain,
        document_stores: body.document_stores,
        hmac_key_id: hmac_key_id.clone(),
        hmac_secret: hmac_secret.clone(),
    };
    st.store.put_business(biz).await;
    // return the secret ONCE at registration (like an API key).
    ok(json!({ "businessId": business_id, "hmacKeyId": hmac_key_id, "hmacSecret": hmac_secret }))
}

// ============================================================================================
// §4.3 Issuer whitelisting
// ============================================================================================

#[derive(Deserialize)]
struct IssuerApplicationReq {
    #[serde(rename = "issuerEntityId")]
    issuer_entity_id: String,
    addresses: Vec<String>,
    #[serde(rename = "recordTypes")]
    record_types: Vec<String>,
    /// VERIFY:<purpose> labels (e.g. "boarding_intake") this verifier may relay verifications for.
    #[serde(rename = "verifyPurposes", default)]
    verify_purposes: Vec<String>,
    domain: String,
    #[serde(rename = "documentStore")]
    document_store: String,
    #[serde(rename = "usdaNan", default)]
    usda_nan: Option<String>,
    #[serde(default)]
    license: Option<License>,
}

async fn create_application(
    State(st): State<AppState>,
    Json(body): Json<IssuerApplicationReq>,
) -> Resp {
    if body.addresses.is_empty() || body.record_types.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "addresses[] and recordTypes[] required",
        );
    }
    let application_id = uuid::Uuid::new_v4().to_string();
    st.store
        .put_application(IssuerApplication {
            application_id: application_id.clone(),
            issuer_entity_id: body.issuer_entity_id,
            addresses: body.addresses.iter().map(|a| a.to_lowercase()).collect(),
            record_types: body.record_types,
            verify_purposes: body.verify_purposes,
            domain: body.domain,
            usda_nan: body.usda_nan,
            license: body.license,
            document_store: body.document_store.to_lowercase(),
            status: "pending".to_string(),
            whitelist_txs: Vec::new(),
        })
        .await;
    ok(json!({ "applicationId": application_id, "status": "pending" }))
}

async fn list_applications(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let apps: Vec<Value> = st
        .store
        .all_applications()
        .await
        .into_iter()
        .map(|a| {
            json!({
                "applicationId": a.application_id, "issuerEntityId": a.issuer_entity_id,
                "addresses": a.addresses, "recordTypes": a.record_types,
                "verifyPurposes": a.verify_purposes,
                "domain": a.domain, "status": a.status,
            })
        })
        .collect();
    ok(json!({ "applications": apps }))
}

/// USDA NAN is a 6-digit accreditation number.
fn usda_nan_valid(nan: &str) -> bool {
    nan.len() == 6 && nan.bytes().all(|b| b.is_ascii_digit())
}

async fn approve_application(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let mut app_rec = match st.store.get_application(&id).await {
        Some(a) => a,
        None => return err(StatusCode::NOT_FOUND, "application not found"),
    };
    if app_rec.status != "pending" {
        return err(StatusCode::CONFLICT, "application not pending");
    }
    // verify accreditation fields off-chain.
    if let Some(nan) = &app_rec.usda_nan {
        if !usda_nan_valid(nan) {
            return err(StatusCode::BAD_REQUEST, "usdaNan must be 6 digits");
        }
    }
    if let Some(lic) = &app_rec.license {
        if lic.number.is_empty() || lic.jurisdiction.is_empty() || lic.expiry.is_empty() {
            return err(
                StatusCode::BAD_REQUEST,
                "license{number,jurisdiction,expiry} required",
            );
        }
    }
    // verify the business's DNS TXT BEFORE whitelisting (architecture §13.3 H).
    let token = crate::dns::expected_txt(&app_rec.document_store);
    match st.dns.txt_contains(&app_rec.domain, &token).await {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::FORBIDDEN, "DNS TXT verification failed"),
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("dns: {e}")),
    }
    // for EACH (address, recordType): admin signer calls whitelistFor(keccak256(recordType), address).
    let mut txs = Vec::new();
    for addr in &app_rec.addresses {
        for rt in &app_rec.record_types {
            let rt_key = record_type_key(rt);
            match st
                .chain
                .whitelist_for(
                    st.cfg.admin_signer_index,
                    &st.cfg.issuer_registry_addr,
                    &rt_key,
                    addr,
                )
                .await
            {
                Ok(sent) => txs.push(sent.tx_hash),
                Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("whitelistFor: {e}")),
            }
        }
    }
    // for EACH (address, verifyPurpose): admin signer calls whitelistFor(verify_key(purpose), address)
    // so the verifier can relay VERIFY:<purpose> verifications (the on-chain VerificationRegistry checks
    // this exact key against the relayer). verify_key byte-matches the on-chain `_verifyKey`.
    for addr in &app_rec.addresses {
        for purpose in &app_rec.verify_purposes {
            let vk = verify_key(purpose);
            match st
                .chain
                .whitelist_for(
                    st.cfg.admin_signer_index,
                    &st.cfg.issuer_registry_addr,
                    &vk,
                    addr,
                )
                .await
            {
                Ok(sent) => txs.push(sent.tx_hash),
                Err(e) => {
                    return err(
                        StatusCode::BAD_GATEWAY,
                        &format!("whitelistFor(verify): {e}"),
                    )
                }
            }
        }
    }
    // dog-tag issuer onboarding: if this application is for the DOG_PROFILE record type, ALSO grant
    // DogTagSBT.ISSUER_ROLE to each signer address so it can mint dog tags (`DogTagSBT.mint`). The
    // admin signer holds the SBT's DEFAULT_ADMIN_ROLE, so it can grantRole. Idempotent: skipped if the
    // address already holds the role. (The DOG_PROFILE IssuerRegistry whitelist entry above stays —
    // harmless.) Groomers have no DOG_PROFILE record type, so this is a no-op for them.
    let is_dog_tag_issuer = app_rec
        .record_types
        .iter()
        .any(|rt| rt.eq_ignore_ascii_case(DOG_PROFILE));
    let mut issuer_role_granted = false;
    let mut issuer_role_txs = Vec::new();
    if is_dog_tag_issuer {
        for addr in &app_rec.addresses {
            match st.chain.has_issuer_role(&st.cfg.sbt_addr, addr).await {
                Ok(true) => issuer_role_granted = true, // already granted — idempotent skip
                Ok(false) => {
                    match st
                        .chain
                        .grant_issuer_role(st.cfg.admin_signer_index, &st.cfg.sbt_addr, addr)
                        .await
                    {
                        Ok(sent) => {
                            issuer_role_granted = true;
                            issuer_role_txs.push(sent.tx_hash);
                        }
                        Err(e) => {
                            return err(StatusCode::BAD_GATEWAY, &format!("grantRole(ISSUER): {e}"))
                        }
                    }
                }
                Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("hasRole(ISSUER): {e}")),
            }
        }
    }

    app_rec.status = "approved".to_string();
    app_rec.whitelist_txs = txs.clone();
    st.store.put_application(app_rec).await;
    ok(json!({
        "status": "approved",
        "whitelistTxs": txs,
        "issuerRoleGranted": issuer_role_granted,
        "issuerRoleTxHash": issuer_role_txs.first().cloned(),
    }))
}

async fn reject_application(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let mut app_rec = match st.store.get_application(&id).await {
        Some(a) => a,
        None => return err(StatusCode::NOT_FOUND, "application not found"),
    };
    app_rec.status = "rejected".to_string();
    st.store.put_application(app_rec).await;
    ok(json!({ "status": "rejected" }))
}

async fn delist_application(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let mut app_rec = match st.store.get_application(&id).await {
        Some(a) => a,
        None => return err(StatusCode::NOT_FOUND, "application not found"),
    };
    let mut txs = Vec::new();
    for addr in &app_rec.addresses {
        for rt in &app_rec.record_types {
            let rt_key = record_type_key(rt);
            match st
                .chain
                .delist_for(
                    st.cfg.admin_signer_index,
                    &st.cfg.issuer_registry_addr,
                    &rt_key,
                    addr,
                )
                .await
            {
                Ok(sent) => txs.push(sent.tx_hash),
                Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("delistFor: {e}")),
            }
        }
    }
    app_rec.status = "delisted".to_string();
    st.store.put_application(app_rec).await;
    ok(json!({ "status": "delisted", "delistTxs": txs }))
}

// ============================================================================================
// §4.4 Appointments — central is the SOLE rev allocator
// ============================================================================================

#[derive(Deserialize)]
struct CreateAppointmentReq {
    #[serde(rename = "businessId")]
    business_id: String,
    #[serde(rename = "dogTagId")]
    dog_tag_id: String,
    slot: String,
}

async fn create_appointment(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateAppointmentReq>,
) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    let biz = match st.store.get_business(&body.business_id).await {
        Some(b) => b,
        None => return err(StatusCode::NOT_FOUND, "business not found"),
    };
    let appointment_id = uuid::Uuid::new_v4().to_string();
    let appt_id_for_closure = appointment_id.clone();
    let dog_tag_id = body.dog_tag_id.clone();
    let slot = body.slot.clone();
    let business_id = body.business_id.clone();
    let oid = owner_id.clone();
    let now = auth::now();
    // central allocates rev:1 REQUESTED atomically.
    let appt = st
        .store
        .alloc_rev_and_apply(
            &appointment_id,
            Box::new(move |_cur, rev| {
                Some(Appointment {
                    appointment_id: appt_id_for_closure,
                    business_id,
                    dog_tag_id,
                    owner_id: oid,
                    slot,
                    rev,
                    state: "REQUESTED".to_string(),
                    updated_at: now,
                })
            }),
        )
        .await;
    let appt = match appt {
        Some(a) => a,
        None => return err(StatusCode::INTERNAL_SERVER_ERROR, "alloc failed"),
    };
    // PUT to business apiBaseUrl with Idempotency-Key + HMAC.
    let body_json = appointment_json(&appt);
    let _ = st
        .business
        .put_appointment(
            &biz.api_base_url,
            &biz.hmac_secret,
            &appt.appointment_id,
            &appt.appointment_id,
            &body_json,
        )
        .await;
    ok(appointment_json(&appt))
}

fn appointment_json(a: &Appointment) -> Value {
    json!({
        "id": a.appointment_id, "businessId": a.business_id, "dogTagId": a.dog_tag_id,
        "slot": a.slot, "rev": a.rev, "state": a.state, "updatedAt": a.updated_at,
    })
}

fn is_terminal(state: &str) -> bool {
    matches!(state, "DECLINED" | "CANCELLED" | "COMPLETED" | "NO_SHOW")
}

/// POST /v1/businesses/{bid}/appointment-events — HMAC verify (key resolved BY path bid); require
/// appointment.businessId == bid (ownership C-2); central allocates next rev; state machine
/// (terminal wins, apply-if-newer); push-notify owner.
async fn appointment_event(
    State(st): State<AppState>,
    Path(bid): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Resp {
    let biz = match st.store.get_business(&bid).await {
        Some(b) => b,
        None => return err(StatusCode::NOT_FOUND, "business not found"),
    };
    // HMAC verify with the key resolved BY path businessId.
    let sig = match headers.get("X-DogTag-HMAC").and_then(|h| h.to_str().ok()) {
        Some(s) => s,
        None => return err(StatusCode::UNAUTHORIZED, "missing HMAC"),
    };
    let path = format!("/v1/businesses/{bid}/appointment-events");
    if !auth::hmac_verify(&biz.hmac_secret, "POST", &path, &raw, sig) {
        return err(StatusCode::UNAUTHORIZED, "bad HMAC");
    }
    let body: Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_REQUEST, &format!("bad json: {e}")),
    };
    let appt_id = match body.get("appointmentId").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return err(StatusCode::BAD_REQUEST, "appointmentId required"),
    };
    let event = match body.get("event").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return err(StatusCode::BAD_REQUEST, "event required"),
    };
    // ownership binding C-2: appointment.businessId == path bid (checked under the alloc lock).
    let now = auth::now();
    let bid_owned = bid.clone();
    let event_apply = event.clone();
    let result = st
        .store
        .alloc_rev_and_apply(
            &appt_id,
            Box::new(move |cur, rev| {
                let mut a = cur?;
                if a.business_id != bid_owned {
                    return None; // ownership violation -> abort
                }
                // terminal wins: never move OUT of a terminal state.
                if is_terminal(&a.state) {
                    return Some(a); // no-op, but keep (rev not bumped meaningfully)
                }
                a.rev = rev;
                a.state = event_apply;
                a.updated_at = now;
                Some(a)
            }),
        )
        .await;
    match result {
        Some(a) if a.business_id != bid => {
            err(StatusCode::FORBIDDEN, "appointment not owned by business")
        }
        Some(a) => {
            // push-notify the owner (best-effort; we record intent).
            tracing::info!(owner = %a.owner_id, appt = %a.appointment_id, state = %a.state, "push notify");
            ok(appointment_json(&a))
        }
        None => {
            // Either the appointment is missing, or ownership failed. Distinguish for the caller.
            match st.store.get_appointment(&appt_id).await {
                Some(existing) if existing.business_id != bid => {
                    err(StatusCode::FORBIDDEN, "appointment not owned by business")
                }
                _ => err(StatusCode::NOT_FOUND, "appointment not found"),
            }
        }
    }
}

#[derive(Deserialize)]
struct ApptQuery {
    #[serde(rename = "updatedSince", default)]
    updated_since: Option<u64>,
}

async fn list_appointments(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ApptQuery>,
) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    let since = q.updated_since.unwrap_or(0);
    let appts: Vec<Value> = st
        .store
        .appointments_updated_since(&owner_id, since)
        .await
        .iter()
        .map(appointment_json)
        .collect();
    ok(json!({ "appointments": appts }))
}

// ============================================================================================
// §4.5 Consent / retention / erasure
// ============================================================================================

#[derive(Deserialize)]
struct ConsentReq {
    purpose: String,
    #[serde(rename = "lawfulBasis")]
    lawful_basis: String,
}

async fn create_consent(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ConsentReq>,
) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    let consent_id = uuid::Uuid::new_v4().to_string();
    let n = auth::now();
    st.store
        .put_consent(Consent {
            consent_id: consent_id.clone(),
            owner_id: owner_id.clone(),
            purpose: body.purpose.clone(),
            lawful_basis: body.lawful_basis.clone(),
            granted_at: n,
            withdrawn: false,
        })
        .await;
    // tamper-evident receipt (off-chain, deletable).
    let receipt_id = uuid::Uuid::new_v4().to_string();
    let hash = keccak256_hex(&format!("{consent_id}|{}|{n}", body.purpose));
    let sealed = match crypto::seal_json(
        st.vault.as_ref(),
        &json!({ "consentId": consent_id, "purpose": body.purpose, "lawfulBasis": body.lawful_basis }),
    )
    .await
    {
        Ok(s) => s,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "seal failed"),
    };
    st.store
        .put_consent_receipt(ConsentReceipt {
            receipt_id: receipt_id.clone(),
            owner_id,
            hash: hash.clone(),
            issued_at: n,
            sealed,
        })
        .await;
    ok(
        json!({ "consentId": consent_id, "receipt": { "receiptId": receipt_id, "hash": hash, "issuedAt": n } }),
    )
}

async fn list_consents(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    let out: Vec<Value> = st
        .store
        .consents_of_owner(&owner_id)
        .await
        .into_iter()
        .map(|c| {
            json!({
                "consentId": c.consent_id, "purpose": c.purpose, "lawfulBasis": c.lawful_basis,
                "grantedAt": c.granted_at, "withdrawn": c.withdrawn,
            })
        })
        .collect();
    ok(json!({ "consents": out }))
}

async fn withdraw_consent(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Resp {
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(e) => return e,
    };
    let mut c = match st.store.get_consent(&id).await {
        Some(c) if c.owner_id == owner_id => c,
        Some(_) => return err(StatusCode::FORBIDDEN, "not your consent"),
        None => return err(StatusCode::NOT_FOUND, "consent not found"),
    };
    c.withdrawn = true;
    st.store.put_consent(c).await;
    ok(json!({ "withdrawn": true }))
}

#[derive(Deserialize)]
struct DeleteReq {
    #[serde(rename = "ownerId", default)]
    owner_id: Option<String>,
    scope: String,
}

async fn delete_request(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<DeleteReq>,
) -> Resp {
    // owner self-service (session) OR admin-on-behalf.
    let owner_id = match require_owner(&st, &headers).await {
        Ok(o) => o,
        Err(_) => match require_admin(&st, &headers).await {
            Ok(()) => match body.owner_id.clone() {
                Some(o) => o,
                None => return err(StatusCode::BAD_REQUEST, "ownerId required (admin)"),
            },
            Err(e) => return e,
        },
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let due_by = auth::now() + 45 * 24 * 3600;
    st.store
        .put_deletion(Deletion {
            request_id: request_id.clone(),
            owner_id,
            scope: body.scope,
            due_by,
            status: "pending".to_string(),
        })
        .await;
    ok(json!({ "requestId": request_id, "dueBy": due_by, "status": "pending" }))
}

/// Admin/manual trigger of the erasure cron (fulfill all due deletions).
async fn fulfill_deletions(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let n = crate::erasure::fulfill_due_deletions(&st, auth::now()).await;
    ok(json!({ "fulfilled": n }))
}

// ============================================================================================
// router assembly
// ============================================================================================

/// Admin-console routes (admin-session gated). Mounted on the public listener by default; when
/// `ADMIN_LOOPBACK_ONLY` is set, served on a separate 127.0.0.1 listener instead. These are the
/// central operator's privileged actions (admin login + issuer whitelisting + erasure trigger).
pub fn admin_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/admin/login", post(admin_login))
        // issuer whitelisting (admin-session writes)
        .route(
            "/v1/issuer-applications/:id/approve",
            post(approve_application),
        )
        .route(
            "/v1/issuer-applications/:id/reject",
            post(reject_application),
        )
        .route(
            "/v1/issuer-applications/:id/delist",
            post(delist_application),
        )
        // erasure cron trigger (admin)
        .route("/v1/privacy/fulfill-deletions", post(fulfill_deletions))
        .with_state(state)
}

/// Public routes (mobile API, registry/discovery, applications submission, consent). Always mounted
/// on the public `0.0.0.0:PORT` listener.
pub fn public_router(state: AppState) -> Router {
    Router::new()
        // health (no auth) — used by compose healthchecks
        .route("/health", get(health))
        // auth
        .route("/v1/auth/signup", post(signup))
        .route("/v1/auth/login", post(login))
        // pets
        .route("/v1/pets", get(list_pets).post(create_pet))
        .route("/v1/pets/:id/mint", post(mint_pet))
        // credentials
        .route("/v1/credentials", get(list_credentials))
        .route("/v1/credentials/import", post(import_credential))
        .route("/v1/share/:id", post(share_credential))
        .route("/share/:ref", get(get_share))
        // verify relay
        .route("/v1/verify/consent", post(verify_consent))
        .route("/v1/verify/receipts", get(verify_receipts))
        // registry / discovery
        .route(
            "/v1/businesses",
            get(list_businesses).post(register_business),
        )
        // issuer applications (list + business submission)
        .route(
            "/v1/issuer-applications",
            get(list_applications).post(create_application),
        )
        // appointments
        .route(
            "/v1/appointments",
            get(list_appointments).post(create_appointment),
        )
        .route(
            "/v1/businesses/:bid/appointment-events",
            post(appointment_event),
        )
        // consent / erasure
        .route("/v1/consents", get(list_consents).post(create_consent))
        .route("/v1/consents/:id/withdraw", post(withdraw_consent))
        .route("/v1/privacy/delete-request", post(delete_request))
        .with_state(state)
}

/// The single combined router (public + admin) on one listener — the default (demo/local) topology.
/// When `ADMIN_LOOPBACK_ONLY` is set, `main.rs` serves `public_router` and `admin_router` separately.
pub fn router(state: AppState) -> Router {
    public_router(state.clone()).merge(admin_router(state))
}

#[cfg(test)]
mod tests {
    //! Unit coverage for the pure request-parsing / geo / validation helpers that previously had no
    //! direct tests (they were exercised only end-to-end through the HTTP handlers).
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn bearer_extracts_token_after_scheme() {
        assert_eq!(
            bearer(&headers(&[("authorization", "Bearer tok123")])),
            Some("tok123".to_string())
        );
    }

    #[test]
    fn bearer_is_scheme_sensitive_and_absent_is_none() {
        // No header at all.
        assert_eq!(bearer(&headers(&[])), None);
        // The prefix is the exact ASCII "Bearer " (capital B, trailing space); a lowercase scheme
        // or a bare token does not match.
        assert_eq!(bearer(&headers(&[("authorization", "bearer tok")])), None);
        assert_eq!(bearer(&headers(&[("authorization", "tok")])), None);
        // An empty token after the scheme is still Some("").
        assert_eq!(
            bearer(&headers(&[("authorization", "Bearer ")])),
            Some(String::new())
        );
    }

    #[test]
    fn client_ip_prefers_first_forwarded_hop() {
        // The first comma-separated hop is the originating client; later hops are proxies.
        let h = headers(&[("x-forwarded-for", "1.2.3.4, 5.6.7.8")]);
        assert_eq!(client_ip(&h, None), "1.2.3.4");
        // Surrounding whitespace on the chosen hop is trimmed.
        let h = headers(&[("x-forwarded-for", "  9.9.9.9  ,10.0.0.1")]);
        assert_eq!(client_ip(&h, None), "9.9.9.9");
    }

    #[test]
    fn client_ip_falls_back_to_peer_then_unknown() {
        let peer: SocketAddr = "203.0.113.7:55000".parse().unwrap();
        // No XFF -> raw socket peer IP (port dropped).
        assert_eq!(client_ip(&headers(&[]), Some(peer)), "203.0.113.7");
        // An empty XFF value is filtered out, so it still falls through to the peer.
        let h = headers(&[("x-forwarded-for", "")]);
        assert_eq!(client_ip(&h, Some(peer)), "203.0.113.7");
        // No XFF and no peer (in-process tests) -> stable "unknown" key.
        assert_eq!(client_ip(&headers(&[]), None), "unknown");
    }

    #[test]
    fn haversine_km_is_zero_for_identical_points_and_symmetric() {
        assert!(haversine_km(40.7, -74.0, 40.7, -74.0).abs() < 1e-9);
        let ab = haversine_km(0.0, 0.0, 51.5, -0.12);
        let ba = haversine_km(51.5, -0.12, 0.0, 0.0);
        assert!((ab - ba).abs() < 1e-9);
    }

    #[test]
    fn haversine_km_matches_known_one_degree_arc() {
        // One degree of longitude at the equator is ~111.19 km on a 6371 km sphere.
        let d = haversine_km(0.0, 0.0, 0.0, 1.0);
        assert!((d - 111.19).abs() < 0.5, "got {d}");
    }

    #[test]
    fn usda_nan_valid_requires_exactly_six_digits() {
        assert!(usda_nan_valid("123456"));
        assert!(!usda_nan_valid("12345")); // too short
        assert!(!usda_nan_valid("1234567")); // too long
        assert!(!usda_nan_valid("12345a")); // non-digit
        assert!(!usda_nan_valid("")); // empty
    }

    #[test]
    fn is_terminal_matches_only_the_four_terminal_states() {
        for s in ["DECLINED", "CANCELLED", "COMPLETED", "NO_SHOW"] {
            assert!(is_terminal(s), "{s} should be terminal");
        }
        for s in ["PENDING", "APPROVED", "REQUESTED", "", "declined"] {
            assert!(!is_terminal(s), "{s} should not be terminal");
        }
    }
}
