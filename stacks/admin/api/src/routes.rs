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

use crate::app::{AppState, DOG_PROFILE};
use crate::auth::{self, keccak256_hex, ShareClaims};
use crate::chain::{
    create_issuer_calldata, default_admin_role, delist_for_calldata, grant_issuer_role_calldata,
    record_type_key, verify_key, whitelist_admin_role, whitelist_for_calldata,
};
use crate::crypto;
use crate::governance::{self, Authority, GovernanceAction};
use crate::store::*;

/// The all-zero address as lowercase `0x..` — an unset/absent contract-address sentinel.
const ZERO_ADDR: &str = "0x0000000000000000000000000000000000000000";

/// Is `addr` the zero/unset address (config not wired)?
fn is_zero_addr(addr: &str) -> bool {
    addr.trim_start_matches("0x")
        .chars()
        .all(|c| c == '0' || c == 'x')
}

/// Is `s` a well-formed 20-byte `0x`-prefixed hex address? Guards against silent `parse_addr`
/// coercion of a malformed value to the zero address.
fn is_valid_addr(s: &str) -> bool {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => h.len() == 40 && h.chars().all(|c| c.is_ascii_hexdigit()),
        None => false,
    }
}

/// Coerce a recordType input into its bytes32 key: pass through an explicit `0x`+64-hex value, else
/// `keccak256(label)` (the factory salt / whitelist key convention, matching the government clones).
fn to_record_type_key(s: &str) -> String {
    let t = s.trim();
    if let Some(h) = t.strip_prefix("0x") {
        if h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()) {
            return format!("0x{}", h.to_lowercase());
        }
    }
    record_type_key(t)
}

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
    // Real password-hash verify against the STORED hash (audit L4). The previous code hashed the
    // plaintext fresh on every call and fell back to a plaintext `!=` compare — cosmetic hashing.
    // `verify_password` re-derives the salted/iterated hash of the submitted password and compares it
    // constant-time against the stored "<salt_hex>$<hash_hex>".
    if !auth::verify_password(&body.password, &st.cfg.admin_password_hash) {
        st.ratelimit.record_failure(&ip);
        return err(StatusCode::UNAUTHORIZED, "bad password");
    }
    st.ratelimit.record_success(&ip);
    let token = auth::new_session_token("admin");
    st.store.put_admin_session(token.clone()).await;
    ok(json!({ "token": token }))
}

// ============================================================================================
// §4.1 Pet management (historical issuance fields are read-only)
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
        // Retained for historical records; new pet rows are not issued by the admin backend.
        chain_id: None,
        protocol_version: None,
        verification_registry: None,
        issuer_addr: None,
        issuer_signer: None,
    };
    st.store.put_pet(pet.clone()).await;
    ok(pet_json(pet))
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
    // Project the stamped unified protocol block into queryable columns. An unstamped document is
    // assigned the single owner-hidden version/registry; the issuer clone comes from its envelope.
    let prov = doc
        .protocol
        .clone()
        .unwrap_or_else(|| dogtag_standard::wrap::ProtocolMeta {
            chain_id: st.cfg.chain_id,
            version: dogtag_standard::wrap::LEVEL_B_VERSION.to_string(),
            verification_registry: st.cfg.verification_registry_addr.clone(),
            issuer_clone: doc.issuer.document_store.clone(),
            issuer_signer: String::new(),
            status_base_url: None,
        });
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
            chain_id: Some(prov.chain_id),
            protocol_version: Some(prov.version),
            verification_registry: Some(prov.verification_registry),
            issuer_addr: Some(prov.issuer_clone),
            issuer_signer: if prov.issuer_signer.is_empty() {
                None
            } else {
                Some(prov.issuer_signer)
            },
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
// §4.1 verification receipts
// ============================================================================================

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
                "relayer": v.relayer, "status": v.status,
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
    // DogTagSBTConsent.ISSUER_ROLE to each signer address so it can issue owner-hidden tags via
    // `mintCustodial`. The
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
// ============================================================================================
// Control plane — factory deploys + governance authority (plan PR-A)
// ============================================================================================

#[derive(Deserialize)]
struct PredictIssuerReq {
    #[serde(rename = "recordType")]
    record_type: String,
    /// The `business` salt component. Optional: defaults to the hosted signer address (single-authority
    /// topology, matching the deployed government clones).
    #[serde(default)]
    business: Option<String>,
}

/// Resolve the `business` salt component: an explicit non-empty value (rejected if malformed), else
/// the hosted signer address. A caller-provided value must parse as a 20-byte `0x` address so a typo
/// is not silently coerced to the zero address by `parse_addr`.
async fn resolve_business(st: &AppState, business: &Option<String>) -> Result<String, Resp> {
    match business {
        Some(b) if !b.trim().is_empty() => {
            let t = b.trim();
            if !is_valid_addr(t) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    "business must be a valid 0x-prefixed 20-byte address",
                ));
            }
            Ok(t.to_lowercase())
        }
        _ => Ok(st
            .chain
            .signer_address(st.cfg.admin_signer_index)
            .await
            .unwrap_or_else(|| ZERO_ADDR.to_string())),
    }
}

/// `POST /v1/admin/factory/predict` — the deterministic clone address for a (recordType, business)
/// BEFORE any deploy (`salt = keccak256(recordType, business)`). Read-only; no tx.
async fn factory_predict(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PredictIssuerReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    if is_zero_addr(&st.cfg.factory_addr) {
        return err(StatusCode::BAD_REQUEST, "FACTORY_ADDR not configured");
    }
    let rt = to_record_type_key(&body.record_type);
    let business = match resolve_business(&st, &body.business).await {
        Ok(b) => b,
        Err(e) => return e,
    };
    match st
        .chain
        .predict_issuer(&st.cfg.factory_addr, &rt, &business)
        .await
    {
        Ok(addr) => ok(json!({
            "predicted": addr,
            "recordTypeKey": rt,
            "business": business,
            "factory": st.cfg.factory_addr,
        })),
        Err(e) => err(StatusCode::BAD_GATEWAY, &format!("predictIssuer: {e}")),
    }
}

#[derive(Deserialize)]
struct CreateIssuerReq {
    name: String,
    #[serde(rename = "recordType")]
    record_type: String,
    #[serde(default)]
    business: Option<String>,
}

/// `POST /v1/admin/factory/issuers` — deploy a new issuer clone from the factory. Routed through the
/// `GovernanceAction` abstraction (Authority = the factory `Ownable` owner): if the hosted key IS the
/// owner it broadcasts `createIssuer`; if ownership has moved to governance it returns the calldata
/// proposal instead. Either way the deterministic clone address is returned up-front.
async fn factory_create_issuer(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateIssuerReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    if is_zero_addr(&st.cfg.factory_addr) {
        return err(StatusCode::BAD_REQUEST, "FACTORY_ADDR not configured");
    }
    if body.name.trim().is_empty() || body.record_type.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "name and recordType are required");
    }
    let rt = to_record_type_key(&body.record_type);
    let business = match resolve_business(&st, &body.business).await {
        Ok(b) => b,
        Err(e) => return e,
    };
    let predicted = match st
        .chain
        .predict_issuer(&st.cfg.factory_addr, &rt, &business)
        .await
    {
        Ok(addr) => addr,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("predictIssuer: {e}")),
    };
    let action = GovernanceAction {
        target: st.cfg.factory_addr.clone(),
        calldata: create_issuer_calldata(&body.name, &rt, &business),
        authority: Authority::Owner {
            owner_target: st.cfg.factory_addr.clone(),
        },
        summary: format!(
            "createIssuer(name={}, recordType={}, business={})",
            body.name, body.record_type, business
        ),
    };
    match governance::dispatch(st.chain.as_ref(), st.cfg.admin_signer_index, &action).await {
        Ok(disp) => ok(json!({
            "predicted": predicted,
            "recordTypeKey": rt,
            "business": business,
            "result": disp,
        })),
        Err(e) => err(StatusCode::BAD_GATEWAY, &format!("createIssuer: {e}")),
    }
}

/// `GET /v1/admin/governance/authority` — the live on-chain authority map (plan Part 2 + OPS-0): who
/// holds the factory `Ownable` owner, the registry `WHITELIST_ADMIN`, and the registry
/// `DEFAULT_ADMIN_ROLE`, whether the hosted operator key holds each, and any pending (timelocked)
/// transfers. The Phase-2 DEFAULT_ADMIN → governance handover surfaces as `pendingDefaultAdmin`. All
/// reads are best-effort: an unreachable/unconfigured target yields `null` rather than failing the map.
async fn governance_authority(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let hosted = st.chain.signer_address(st.cfg.admin_signer_index).await;
    let factory = &st.cfg.factory_addr;
    let registry = &st.cfg.issuer_registry_addr;
    let wl_role = whitelist_admin_role();
    let da_role = default_admin_role();

    // Factory Ownable owner + pending owner + does the hosted key own it.
    let factory_owner = st.chain.ownable_owner(factory).await.ok();
    let factory_pending_owner = st.chain.ownable_pending_owner(factory).await.ok();
    let hosted_is_factory_owner = match (&hosted, &factory_owner) {
        (Some(h), Some(o)) => Some(h.eq_ignore_ascii_case(o)),
        _ => None,
    };

    // Registry DEFAULT_ADMIN holder + pending transfer (Phase-2) + hosted holdership.
    let default_admin = st.chain.default_admin(registry).await.ok();
    let (pending_admin, pending_eta) = st
        .chain
        .pending_default_admin(registry)
        .await
        .unwrap_or_else(|_| (ZERO_ADDR.to_string(), 0));
    let pending_default_admin = if is_zero_addr(&pending_admin) {
        None
    } else {
        Some(json!({ "newAdmin": pending_admin, "acceptSchedule": pending_eta }))
    };
    let hosted_has_default_admin = match &hosted {
        Some(h) => st.chain.has_role(registry, &da_role, h).await.ok(),
        None => None,
    };
    let hosted_has_whitelist_admin = match &hosted {
        Some(h) => st.chain.has_role(registry, &wl_role, h).await.ok(),
        None => None,
    };

    ok(json!({
        "hostedSigner": hosted,
        "chainId": crate::chain::ROAX_CHAIN_ID,
        "factoryOwner": {
            "target": factory,
            "owner": factory_owner,
            "pendingOwner": factory_pending_owner.filter(|a| !is_zero_addr(a)),
            "heldByHosted": hosted_is_factory_owner,
            "capability": "createIssuer",
        },
        "whitelistAdmin": {
            "target": registry,
            "role": wl_role,
            "heldByHosted": hosted_has_whitelist_admin,
            "capability": "whitelistFor / delistFor",
        },
        "defaultAdmin": {
            "target": registry,
            "role": da_role,
            "holder": default_admin,
            "pendingTransfer": pending_default_admin,
            "heldByHosted": hosted_has_default_admin,
            "capability": "adminRevoke / role-admin / verifier+consent-key swaps",
        },
    }))
}

// ============================================================================================
// PR-E — direct whitelist management (grant / revoke) as a standalone control-plane action.
// Promotes the read-only whitelist viewer to a management console: an operator whitelists or delists
// an arbitrary (signer, capability) pair on demand — decoupled from the issuer-application lifecycle
// (key rotation, ad-hoc grants, incident response). Every write routes through the `GovernanceAction`
// abstraction (Authority = the registry WHITELIST_ADMIN role, and the SBT DEFAULT_ADMIN for the
// DOG_PROFILE ISSUER grant), so each action executes directly while the hosted key holds the
// authority and flips to a proposal the moment that role moves to the governance signer (Phase-2) —
// never assuming the old EOA holds it.
// ============================================================================================

#[derive(Deserialize)]
struct WhitelistActionReq {
    /// The signer address the capability is granted to / revoked from.
    signer: String,
    /// The issuance record type (a label like "VACCINATION" or an explicit `0x`+64-hex key). Optional:
    /// a grant/revoke may target only verify purposes.
    #[serde(rename = "recordType", default)]
    record_type: Option<String>,
    /// Optional VERIFY:<purpose> capabilities (each keyed via `verify_key`), mirroring approval.
    #[serde(rename = "verifyPurposes", default)]
    verify_purposes: Vec<String>,
}

/// Build the WHITELIST_ADMIN `GovernanceAction`s for a grant/revoke over `(record_type?, verify_purposes)`.
/// `grant=true` encodes `whitelistFor`, else `delistFor`. Returns an empty vec when no capability is
/// named (the caller rejects that as a 400). `signer` is assumed already validated + lowercased.
fn whitelist_actions(
    registry: &str,
    signer: &str,
    record_type: &Option<String>,
    verify_purposes: &[String],
    grant: bool,
) -> Vec<GovernanceAction> {
    let verb = if grant { "whitelistFor" } else { "delistFor" };
    let calldata = |key: &str| -> String {
        if grant {
            whitelist_for_calldata(key, signer)
        } else {
            delist_for_calldata(key, signer)
        }
    };
    let role_authority = || Authority::Role {
        role_target: registry.to_string(),
        role: whitelist_admin_role(),
        default_admin: false,
    };
    let mut actions = Vec::new();
    if let Some(rt) = record_type {
        let rt = rt.trim();
        if !rt.is_empty() {
            actions.push(GovernanceAction {
                target: registry.to_string(),
                calldata: calldata(&to_record_type_key(rt)),
                authority: role_authority(),
                summary: format!("{verb}(recordType={rt}, signer={signer})"),
            });
        }
    }
    for purpose in verify_purposes {
        let p = purpose.trim();
        if p.is_empty() {
            continue;
        }
        actions.push(GovernanceAction {
            target: registry.to_string(),
            calldata: calldata(&verify_key(p)),
            authority: role_authority(),
            summary: format!("{verb}(VERIFY:{p}, signer={signer})"),
        });
    }
    actions
}

/// Annotate a grant/revoke response with what the request actually did: the tri-state `outcome`, the
/// back-compat `executed` boolean, and the matching operator note.
///
/// A request where NOTHING executed changed nothing on-chain and used to be indistinguishable from
/// success. But "nothing executed" itself has two meanings - the declared out-of-band-signing flow, and
/// a stack booted on a key that lost its authority - so the outcome distinguishes them instead of
/// reporting both as one failure. `DispatchOutcome` owns that decision; see its doc comment.
fn dispatch_summary(
    st: &AppState,
    results: &[governance::Disposition],
) -> (&'static str, bool, Value) {
    let outcome = governance::DispatchOutcome::classify(results, st.cfg.propose_only);
    (
        outcome.as_str(),
        outcome.executed(),
        outcome.warning().map(Value::from).unwrap_or(Value::Null),
    )
}

/// Dispatch each action in order, short-circuiting to a 502 on the first chain error. A dispatched
/// action yields a `Disposition` (executed with a tx hash, or proposed with the calldata payload).
async fn dispatch_all(
    st: &AppState,
    actions: &[GovernanceAction],
) -> Result<Vec<governance::Disposition>, Resp> {
    let mut out = Vec::with_capacity(actions.len());
    for a in actions {
        match governance::dispatch(st.chain.as_ref(), st.cfg.admin_signer_index, a).await {
            Ok(d) => out.push(d),
            Err(e) => return Err(err(StatusCode::BAD_GATEWAY, &format!("{}: {e}", a.summary))),
        }
    }
    Ok(out)
}

/// `POST /v1/admin/whitelist/grant` — grant an issuer/verifier capability directly. Whitelists the
/// signer for the record type + each verify purpose, and (for DOG_PROFILE) grants DogTagSBT
/// ISSUER_ROLE so it can call `mintCustodial` — the same machinery `approve_application` runs, but for one signer and
/// decoupled from the application queue. Each write is a `GovernanceAction` (executed if the hosted
/// key holds the authority, else proposed).
async fn whitelist_grant(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WhitelistActionReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    if !is_valid_addr(body.signer.trim()) {
        return err(
            StatusCode::BAD_REQUEST,
            "signer must be a valid 0x-prefixed 20-byte address",
        );
    }
    let signer = body.signer.trim().to_lowercase();
    let actions = whitelist_actions(
        &st.cfg.issuer_registry_addr,
        &signer,
        &body.record_type,
        &body.verify_purposes,
        true,
    );
    if actions.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "at least one of recordType or verifyPurposes is required",
        );
    }
    let results = match dispatch_all(&st, &actions).await {
        Ok(r) => r,
        Err(e) => return e,
    };

    // DOG_PROFILE onboarding: also grant DogTagSBTConsent.ISSUER_ROLE. Gated by the SBT's
    // DEFAULT_ADMIN authority (a distinct key post-Phase-2), so it too routes through GovernanceAction.
    // Idempotent: skipped when the signer already holds the role.
    let is_dog_tag_issuer = body
        .record_type
        .as_deref()
        .map(|rt| rt.trim().eq_ignore_ascii_case(DOG_PROFILE))
        .unwrap_or(false);
    let mut issuer_role_dispatched: Option<governance::Disposition> = None;
    let issuer_role = if is_dog_tag_issuer {
        match st.chain.has_issuer_role(&st.cfg.sbt_addr, &signer).await {
            Ok(true) => json!({ "status": "alreadyHeld" }),
            Ok(false) => {
                let action = GovernanceAction {
                    target: st.cfg.sbt_addr.clone(),
                    calldata: grant_issuer_role_calldata(&signer),
                    authority: Authority::Role {
                        role_target: st.cfg.sbt_addr.clone(),
                        role: default_admin_role(),
                        default_admin: true,
                    },
                    summary: format!("grantRole(ISSUER, signer={signer})"),
                };
                match governance::dispatch(st.chain.as_ref(), st.cfg.admin_signer_index, &action)
                    .await
                {
                    Ok(d) => {
                        let rendered = json!(d);
                        issuer_role_dispatched = Some(d);
                        rendered
                    }
                    Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("grantRole(ISSUER): {e}")),
                }
            }
            Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("hasRole(ISSUER): {e}")),
        }
    } else {
        Value::Null
    };

    // `executed`/`warning` describe the WHOLE request, so the separately dispatched ISSUER_ROLE action
    // is folded in: it is a real broadcast when executed, and the "nothing reached the chain" warning
    // may only be stated when NOT ONE action was. `alreadyHeld` / a non-DOG_PROFILE grant contribute
    // nothing here - neither is a broadcast, and neither makes the warning claim true on its own.
    let mut dispatched = results.clone();
    dispatched.extend(issuer_role_dispatched);
    let (outcome, executed, warning) = dispatch_summary(&st, &dispatched);
    ok(json!({
        "signer": signer,
        "recordType": body.record_type,
        "actions": results,
        "issuerRole": issuer_role,
        "outcome": outcome,
        "executed": executed,
        "warning": warning,
    }))
}

/// `POST /v1/admin/whitelist/revoke` — delist an issuer/verifier capability directly (the inverse of
/// grant): delists the record type + each verify purpose via `GovernanceAction` (WHITELIST_ADMIN).
/// Does NOT revoke DogTagSBT.ISSUER_ROLE or on-chain roots — those are DEFAULT_ADMIN governance
/// actions (`adminRevoke`) surfaced on the Governance page (plan PR-F), not here. Mirrors the existing
/// `delist_application` semantics (delistFor only).
async fn whitelist_revoke(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WhitelistActionReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    if !is_valid_addr(body.signer.trim()) {
        return err(
            StatusCode::BAD_REQUEST,
            "signer must be a valid 0x-prefixed 20-byte address",
        );
    }
    let signer = body.signer.trim().to_lowercase();
    let actions = whitelist_actions(
        &st.cfg.issuer_registry_addr,
        &signer,
        &body.record_type,
        &body.verify_purposes,
        false,
    );
    if actions.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "at least one of recordType or verifyPurposes is required",
        );
    }
    let results = match dispatch_all(&st, &actions).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let (outcome, executed, warning) = dispatch_summary(&st, &results);
    ok(json!({
        "signer": signer,
        "recordType": body.record_type,
        "actions": results,
        "outcome": outcome,
        "executed": executed,
        "warning": warning,
    }))
}

// ============================================================================================
// PR-B — oversight-indexer consumption + signer→business directory (the "see on-chain activity"
// data layer). All reads are UNSCOPED (cross-issuer) and non-PII: the admin/government sees every
// issuer's events, named via the admin business directory, never any role's PII Mongo.
// ============================================================================================

/// Query params for `GET /v1/admin/activity` — pass-through narrowing filters over the unscoped feed.
#[derive(Debug, Deserialize, Default)]
struct ActivityParams {
    #[serde(rename = "type")]
    event_type: Option<String>,
    signer: Option<String>,
    issuer: Option<String>,
    #[serde(rename = "recordType")]
    record_type: Option<String>,
    root: Option<String>,
    #[serde(rename = "dogTagId")]
    dog_tag_id: Option<String>,
    finality: Option<String>,
    since: Option<u64>,
    until: Option<u64>,
    limit: Option<usize>,
    offset: Option<usize>,
}

impl From<ActivityParams> for crate::indexer::FeedQuery {
    fn from(p: ActivityParams) -> Self {
        crate::indexer::FeedQuery {
            event_type: p.event_type,
            signer: p.signer,
            issuer: p.issuer,
            record_type: p.record_type,
            root: p.root,
            dog_tag_id: p.dog_tag_id,
            finality: p.finality,
            since: p.since,
            until: p.until,
            limit: p.limit,
            offset: p.offset,
        }
    }
}

/// Map an oversight-feed error to an HTTP response: an unconfigured indexer is a 503 (the surface is
/// simply unavailable), any transport/upstream error is a 502.
fn feed_err(e: crate::indexer::FeedError) -> Resp {
    use crate::indexer::FeedError;
    match e {
        FeedError::NotConfigured => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": e.to_string(), "indexer": "not-configured" })),
        ),
        FeedError::Transport(_) | FeedError::Status(_, _) => {
            err(StatusCode::BAD_GATEWAY, &e.to_string())
        }
    }
}

/// Re-enrich an indexer `events` array with the admin's AUTHORITATIVE signer→business names: for each
/// event, resolve `actor`→`actorName` and `clone`→`cloneName` from the admin directory, overriding the
/// indexer's own best-effort copy. Leaves the indexer's value in place when the admin can't resolve it.
fn enrich_events(dir: &crate::directory::SignerDirectory, body: &mut Value) {
    let Some(events) = body.get_mut("events").and_then(|v| v.as_array_mut()) else {
        return;
    };
    for ev in events.iter_mut() {
        let Some(obj) = ev.as_object_mut() else { continue };
        if let Some(actor) = obj.get("actor").and_then(|v| v.as_str()).map(str::to_string) {
            if let Some(name) = dir.name(&actor) {
                obj.insert("actorName".into(), json!(name));
            }
        }
        if let Some(clone) = obj.get("clone").and_then(|v| v.as_str()).map(str::to_string) {
            if let Some(name) = dir.name(&clone) {
                obj.insert("cloneName".into(), json!(name));
            }
        }
    }
}

/// `GET /v1/admin/activity` — the UNSCOPED cross-issuer oversight feed (plan §3.2). Proxies the PR-4
/// indexer's `/v1/events` with the admin's unscoped token, then re-enriches every event with the
/// admin's authoritative signer→business directory so signers read as business names. Filters
/// (`type`, `signer`, `issuer`, `recordType`, `root`, `dogTagId`, `finality`, `since`, `until`,
/// `limit`, `offset`) only ever narrow within the unscoped ceiling.
async fn admin_activity(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(p): Query<ActivityParams>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let q: crate::indexer::FeedQuery = p.into();
    match st.feed.events(&q).await {
        Ok(mut body) => {
            let dir = crate::directory::SignerDirectory::from_store(st.store.as_ref()).await;
            enrich_events(&dir, &mut body);
            ok(body)
        }
        Err(e) => feed_err(e),
    }
}

/// `GET /v1/admin/activity/stats` — cross-issuer aggregate counters (active vs revoked credentials,
/// verifications, whitelisted/delisted signers, distinct clones/signers, finalized/pending). The
/// aggregates PR-D's dashboard renders (plan §3.1). Proxies the indexer's `/v1/stats` unscoped.
async fn admin_activity_stats(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    match st.feed.stats().await {
        Ok(body) => ok(body),
        Err(e) => feed_err(e),
    }
}

/// `GET /v1/admin/activity/status` — the oversight indexer's progress + finality watermark (plan §3.1
/// chain-health card): `headBlock` (chain head), `finalizedBlock`/`finalitySource`,
/// `lastFinalizedIndexed` (how far the scanner has settled), `lag` (head − last-indexed), and
/// `confirmations`. Proxies the indexer's `/v1/status` unscoped; PR-B wired the `status()` feed method
/// specifically to fuel this card. No directory enrichment (no addresses in the payload).
async fn admin_activity_status(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    match st.feed.status().await {
        Ok(body) => ok(body),
        Err(e) => feed_err(e),
    }
}

/// `GET /v1/admin/activity/issuers` — per-clone issued/revoked/active counts across all issuers (plan
/// §3.3 list). Proxies the indexer's `/v1/issuers` unscoped and re-enriches each clone with the
/// admin's authoritative directory name.
async fn admin_activity_issuers(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    match st.feed.issuers().await {
        Ok(mut body) => {
            let dir = crate::directory::SignerDirectory::from_store(st.store.as_ref()).await;
            if let Some(list) = body.get_mut("issuers").and_then(|v| v.as_array_mut()) {
                for it in list.iter_mut() {
                    let Some(obj) = it.as_object_mut() else { continue };
                    if let Some(clone) =
                        obj.get("clone").and_then(|v| v.as_str()).map(str::to_string)
                    {
                        if let Some(name) = dir.name(&clone) {
                            obj.insert("cloneName".into(), json!(name));
                        }
                    }
                }
            }
            ok(body)
        }
        Err(e) => feed_err(e),
    }
}

/// `GET /v1/admin/directory` — the full signer→business directory (plan §3.5): every indexed signer
/// address joined to its business identity (name, entity, recordTypes). Non-PII; the naming source the
/// activity feed + PR-D dashboard consume. Built live from the admin business registry + applications.
async fn admin_directory(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let dir = crate::directory::SignerDirectory::from_store(st.store.as_ref()).await;
    let entries = dir.entries();
    ok(json!({ "signers": entries, "total": entries.len() }))
}

/// `GET /v1/admin/directory/signer/:addr` — resolve one on-chain signer/clone address to its business
/// identity (`{business, entity, recordTypes, …}`). 404 when the address maps to no known issuer.
async fn admin_directory_signer(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(addr): Path<String>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let dir = crate::directory::SignerDirectory::from_store(st.store.as_ref()).await;
    match dir.resolve(&addr) {
        Some(entry) => ok(json!(entry)),
        None => err(StatusCode::NOT_FOUND, "signer not found in directory"),
    }
}

pub fn admin_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/admin/login", post(admin_login))
        // control plane: factory deploys + governance authority map (PR-A)
        .route("/v1/admin/factory/predict", post(factory_predict))
        .route("/v1/admin/factory/issuers", post(factory_create_issuer))
        .route("/v1/admin/governance/authority", get(governance_authority))
        // PR-E: direct whitelist management (grant / revoke) via GovernanceAction
        .route("/v1/admin/whitelist/grant", post(whitelist_grant))
        .route("/v1/admin/whitelist/revoke", post(whitelist_revoke))
        // PR-B: unscoped oversight-indexer consumption + signer→business directory
        .route("/v1/admin/activity", get(admin_activity))
        .route("/v1/admin/activity/stats", get(admin_activity_stats))
        .route("/v1/admin/activity/status", get(admin_activity_status))
        .route("/v1/admin/activity/issuers", get(admin_activity_issuers))
        .route("/v1/admin/directory", get(admin_directory))
        .route(
            "/v1/admin/directory/signer/:addr",
            get(admin_directory_signer),
        )
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
        // credentials
        .route("/v1/credentials", get(list_credentials))
        .route("/v1/credentials/import", post(import_credential))
        .route("/v1/share/:id", post(share_credential))
        .route("/share/:ref", get(get_share))
        // verify relay
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
