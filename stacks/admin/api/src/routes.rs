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
// NOTE `whitelist_for_calldata` / `delist_for_calldata` / `grant_issuer_role_calldata` are no longer
// imported here, because the deleted whitelist console was their only caller in this file. All three
// still exist inside `chain.rs`, backing the issuer-application approval flow below.
//
// CORRECTION, and it is not a nuance. This said those calls target "a DIFFERENT contract, still
// live, deliberately untouched". That was true when it was written and is not true now: #140 deleted
// `contracts/src/IssuerRegistry.sol` with the rest of generation 1, and no `IssuerRegistry` key
// exists in `contracts/deployments/roax.json`. `whitelistFor` and `delistFor` are implemented by NO
// contract in the launch set, and `ISSUER_REGISTRY_ADDR` ships blank with no fallback.
//
// So `approve_application` and `delist_application` below still build those calls against an address
// that resolves to nothing. What each of their two loops needs is NOT the same:
//
//   * the VERIFY loop has an exact successor - `ProviderRegistry.setVerifierCapability(purpose,
//     relayer, allowed)`, which is what `VerificationRegistryConsent` reads via `canVerify`. Note it
//     takes the RAW purpose and derives `verificationKey` itself, so feeding it `verify_key(purpose)`
//     would derive twice and grant a capability nothing reads.
//   * the ISSUANCE loop has NONE. `setIssuanceCapability` is keyed on a SERVICE address, and an
//     issuer application has no deployed clone at approval time. The replacement for that half is the
//     registrar journey on `/v1/admin/providers*`, walked live on chain in #139.
//
// Rewiring this is a live-behaviour change to two admin surfaces (`IssuerApplications.tsx`,
// `Wizard.tsx`), so it is left for a deliberate decision rather than made silently here. The on-chain
// acceptance test that covered the retired write (`tests/whitelist.rs`) has been deleted, since it
// deployed a contract that no longer exists; the route keeps its hermetic coverage in `central.rs`
// and `dns_gate.rs`.
use crate::chain::{
    attach_service_calldata, create_issuer_calldata, default_admin_role, purpose_key,
    record_type_key, register_provider_calldata, set_issuance_capability_calldata,
    set_provider_standing_calldata, set_resolver_approved_calldata,
    set_service_creation_approval_calldata, set_service_standing_calldata,
    set_verifier_capability_calldata, verify_key, whitelist_admin_role,
};
use crate::crypto;
use crate::governance::{self, Authority, GovernanceAction};
use crate::provider_registry::{
    fold_approvals, fold_capabilities, is_valid_provider_id, record_type_label, ApprovalsRead,
    CapabilitiesRead, ResolverKind, Standing, HASH_ALGORITHM_KECCAK256, IDENTITY_CODEC_NONE,
    KNOWN_VERIFY_PURPOSES, PROVIDER_IDENTITY_SCHEMA, PROVIDER_IDENTITY_SCHEMA_ID,
};
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
/// A structured error body, for cases where the client needs the OBSERVED facts (not just a message)
/// to render an honest prompt — e.g. the advisory DNS confirmation.
fn err_json(code: StatusCode, body: Value) -> Resp {
    (code, Json(body))
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

/// Query filters for `GET /v1/businesses`.
///
/// # `near` and `radius` are DEPRECATED. Do not add a caller.
///
/// They still work. This is a documentary deprecation, not a removal: the fields are still
/// deserialized and `list_businesses` still filters on them, so any existing third-party caller
/// keeps working. What was removed is the ability of anything in THIS repo to send them - see
/// `packages/ui/src/api/types.ts`, where `BusinessesQuery` no longer carries the fields, and
/// `central.ts`, where `qs()` no longer emits them.
///
/// ## Why
///
/// This route is mounted on the PUBLIC, UNAUTHENTICATED router (`public_router`). `near` is the
/// caller's own position, so it arrives beside their IP with no account attached and no gate. dogtag
/// is built on the owner never revealing where they are; an endpoint whose entire purpose is to be
/// told where the user is contradicts that at the most basic level.
///
/// Verified when this note was written: nothing sent it. The shared client could, but both callers
/// (`Businesses.tsx`, `Dashboard.tsx`) invoked `listBusinesses()` with no arguments, and neither
/// mobile app referenced the endpoint. So this closed a LOADED PATH rather than an active leak - and
/// the reason to close it before building "nearby" is that passing this argument is the obvious way
/// to build it. It would work on the first try, and it would make the leak live in one line.
///
/// ## The replacement
///
/// `packages/ui/src/geo/` computes distance on the device. The client fetches the provider set -
/// a request that is byte-identical whoever makes it, so it discloses nothing - and filters locally.
/// A provider's address is a business fact already on their door; a user's position is not.
///
/// The two produce the SAME results: for every radius below the half-circumference they admit
/// exactly the same providers. That is pinned from both ends against one fixture generated by this
/// very function - `geo_parity_fixture_is_what_this_haversine_actually_produces` below and
/// `packages/ui/test/geoParity.test.ts`.
///
/// ## A defect in this filter, which is why the replacement does not copy it
///
/// `haversine_km` ends in `asin(sqrt(a))`. For near-antipodal inputs `a` rounds two ulps above 1.0
/// in f64, so `asin` receives an argument outside `[-1, 1]` and returns NaN - and `NaN <= radius` is
/// `false`, so the provider is silently dropped with no error raised. A "could not compute" rendered
/// as a definite "out of range". Roughly 1 in 12,000 uniformly-sampled near-antipodal pairs hits it,
/// in both Rust's libm and V8. The TypeScript replacement uses the total
/// `atan2(sqrt(a), sqrt(max(0, 1 - a)))` form instead. See
/// `the_deprecated_filter_silently_drops_a_near_antipodal_provider`.
///
/// Left as-is here rather than fixed: this filter is deprecated and unused, and changing its results
/// would break the parity claim that makes the deprecation demonstrably safe. If a caller is ever
/// found, fix the formula then.
#[derive(Deserialize)]
struct BusinessesQuery {
    #[serde(rename = "type")]
    kind: Option<String>,
    /// DEPRECATED - "lat,lng". Discloses the caller's position on a public route. See the type note.
    near: Option<String>,
    /// DEPRECATED - km. Only meaningful alongside `near`. See the type note.
    radius: Option<f64>,
}

/// Great-circle distance in km. DEPRECATED alongside `BusinessesQuery::near` - the on-device
/// replacement is `haversineKm` in `packages/ui/src/geo/distance.ts`. Returns NaN for some
/// near-antipodal inputs; see the type note above.
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
            // A location-less provider is NOT within any radius. It is not distance zero, and it is
            // not silently admitted either - the same both-positions-must-be-usable rule
            // `withinRadiusKm` applies on the device. The deprecated formula below is untouched.
            Some((lat, lng)) => match b.location() {
                Some((blat, blng)) => haversine_km(lat, lng, blat, blng) <= radius,
                None => false,
            },
            None => true,
        })
        .map(|b| {
            // non-personal fields only — NEVER the HMAC secret.
            json!({
                "businessId": b.business_id, "type": b.kind, "name": b.name,
                "geo": business_geo_json(&b), "contact": b.contact,
                "services": b.services,
                "apiBaseUrl": b.api_base_url, "domain": b.domain,
                "documentStores": b.document_stores, "hmacKeyId": b.hmac_key_id,
            })
        })
        .collect();
    ok(json!({ "businesses": out }))
}

/// `geo` for the wire: the pair when this provider published a location, otherwise an EXPLICIT
/// `null`.
///
/// Emitted rather than omitted so the wire says "this provider has no location" instead of leaving
/// a consumer to guess whether the key was dropped by a serializer. (`packages/ui`'s row validator
/// accepts both, for the opposite reason - a foreign serializer that omits nulls must not take the
/// whole directory down - but our own response should state it.)
fn business_geo_json(b: &Business) -> Value {
    match b.location() {
        Some((lat, lng)) => json!({ "lat": lat, "lng": lng }),
        None => Value::Null,
    }
}

/// Contact channels a provider may publish. All optional: a provider chooses which it exposes.
///
/// BUSINESS contact details, not personal ones - see [`BusinessContact`].
#[derive(Deserialize)]
struct BusinessContactReq {
    #[serde(default)]
    phone: Option<String>,
    #[serde(default)]
    whatsapp: Option<String>,
    #[serde(default)]
    telegram: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    website: Option<String>,
}

/// Trim, and treat a blank string as absence.
///
/// A form that submits every field always sends `""` for the ones left empty; storing that would
/// make "no phone number" and "an empty phone number" two different states with identical meaning.
fn opt_trimmed(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

impl From<BusinessContactReq> for BusinessContact {
    fn from(r: BusinessContactReq) -> Self {
        BusinessContact {
            phone: opt_trimmed(r.phone),
            whatsapp: opt_trimmed(r.whatsapp),
            telegram: opt_trimmed(r.telegram),
            email: opt_trimmed(r.email),
            website: opt_trimmed(r.website),
        }
    }
}

#[derive(Deserialize)]
struct RegisterBusinessReq {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    /// OPTIONAL. Omit both `lat` and `lng` to register a provider with no location.
    #[serde(default)]
    lat: Option<f64>,
    /// OPTIONAL. See `lat`.
    #[serde(default)]
    lng: Option<f64>,
    #[serde(default)]
    contact: Option<BusinessContactReq>,
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
    // Both-or-neither: one coordinate is not a place, and a half-set row could never be served as a
    // position anyway. Refusing it here is what keeps `Business::location`'s half-set arm
    // unreachable through the API.
    if body.lat.is_some() != body.lng.is_some() {
        return err(
            StatusCode::BAD_REQUEST,
            "lat and lng must be supplied together; omit both for a provider with no location",
        );
    }
    // Range-check at the WRITE, because the read side cannot fix it: `packages/ui`'s directory row
    // validator keeps its all-or-nothing rule for a MALFORMED coordinate, so one out-of-range row
    // would take the whole directory to `unavailable` for every consumer. Absence is first class;
    // nonsense is not.
    if let (Some(lat), Some(lng)) = (body.lat, body.lng) {
        if !lat.is_finite() || !lng.is_finite() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
            return err(
                StatusCode::BAD_REQUEST,
                "lat must be within ±90 and lng within ±180",
            );
        }
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
        contact: body.contact.map(BusinessContact::from).unwrap_or_default(),
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

/// `GET /v1/admin/businesses/location-review` - the rows an operator has to answer for.
///
/// Admin-gated, read-only, and it changes nothing. It exists because making location optional
/// cannot repair the rows that were written BEFORE it was optional: a blank location was stored as
/// `0, 0`, and `0, 0` is a legal coordinate, so a row sitting there is either a provider genuinely
/// at that point in the Gulf of Guinea or a provider with no location at all - and no code can tell
/// which. Guessing would either plant a false pin or erase a real one.
///
/// So this route asks rather than decides. Each listed row needs one of three operator answers:
/// "this pin is correct", "this pin is wrong, here is the right one", or "this provider has no
/// location". Remediation itself is deliberately NOT here - there is no business-edit endpoint in
/// this slice, and inventing one to carry a review answer would put the decision back in code.
async fn businesses_location_review(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let all = st.store.all_businesses().await;
    let total = all.len();
    let rows: Vec<Value> = all
        .into_iter()
        .filter(Business::location_needs_review)
        .map(|b| {
            json!({
                "businessId": b.business_id, "type": b.kind, "name": b.name,
                "domain": b.domain, "geo": business_geo_json(&b),
                "hasContact": !b.contact.is_empty(),
            })
        })
        .collect();
    ok(json!({
        "totalBusinesses": total,
        "needsReview": rows.len(),
        "businesses": rows,
        "reason": "Stored at exactly 0,0 - a legal coordinate AND the value a blank location used \
                   to be stored as. Code cannot distinguish the two; each row needs an operator \
                   answer of 'pin is correct', 'pin is wrong, here is the right one', or 'this \
                   provider has no location'.",
    }))
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
            dns_state: String::new(),
            dns_checked_at: 0,
            dns_state_at_approval: String::new(),
            dns_proceeded_unverified: false,
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
                // The DNS legitimacy trace. `dnsState` is the LATEST observation (the future daily
                // re-check job overwrites it, so a binding can turn verified with no admin action);
                // `dnsStateAtApproval` + `dnsProceededUnverified` are immutable history, so an issuer
                // whitelisted on an override is never rendered identically to one that passed cleanly.
                "dnsState": a.dns_state, "dnsCheckedAt": a.dns_checked_at,
                "dnsStateAtApproval": a.dns_state_at_approval,
                "dnsProceededUnverified": a.dns_proceeded_unverified,
            })
        })
        .collect();
    ok(json!({ "applications": apps }))
}

/// USDA NAN is a 6-digit accreditation number.
fn usda_nan_valid(nan: &str) -> bool {
    nan.len() == 6 && nan.bytes().all(|b| b.is_ascii_digit())
}

/// Body of `POST /v1/issuer-applications/:id/approve`.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ApproveBody {
    /// The admin's EXPLICIT confirmation that they have seen a non-verified DNS observation and choose
    /// to whitelist anyway.
    ///
    /// Required only when the live check does NOT come back verified. It is deliberately an explicit
    /// act rather than a warning the UI can ignore: proceeding is recorded as a decision
    /// (`dns_proceeded_unverified`), and a decision needs someone to have made it.
    #[serde(default)]
    proceed_without_dns: bool,
}

async fn approve_application(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Option<Json<ApproveBody>>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let body = body.map(|Json(b)| b).unwrap_or_default();
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
    // The DNS legitimacy observation (architecture §13.3 H) — ADVISORY, never blocking.
    //
    // The lookup ALWAYS runs against the real domain and its real outcome is always what gets recorded
    // and reported. The three outcomes stay distinct: verified, definitively not listed, and
    // did-not-resolve. Nothing here synthesizes a state.
    //
    // It does not block, because an organisation is routinely KYC-approved before its DNS team
    // publishes anything, and a hard block is what drives operators to a bypass. What keeps that from
    // being fail-open is the trace: a non-verified observation requires the admin's EXPLICIT
    // `proceedWithoutDns`, and both the observation and the fact that they proceeded are persisted.
    let token = crate::dns::expected_txt(&app_rec.document_store);
    let dns_state = match st.dns.txt_contains(&app_rec.domain, &token).await {
        Ok(true) => "verified",
        Ok(false) => "notListed",
        Err(e) => {
            tracing::info!(
                domain = %app_rec.domain,
                error = %format!("{e}"),
                "DNS legitimacy lookup did not resolve"
            );
            "couldNotCheck"
        }
    };
    let dns_checked_at = crate::auth::now();

    // A non-verified observation needs a deliberate confirmation, not a dismissable warning. The 409
    // carries exactly what was OBSERVED so the UI can state the observation rather than a verdict.
    if dns_state != "verified" && !body.proceed_without_dns {
        return err_json(
            StatusCode::CONFLICT,
            json!({
                "error": "dnsConfirmationRequired",
                "dnsState": dns_state,
                "domain": app_rec.domain,
                "documentStore": app_rec.document_store,
                "expectedTxt": token,
                // Retry the same call with this set to record the decision and proceed.
                "retryWith": { "proceedWithoutDns": true },
            }),
        );
    }
    if dns_state != "verified" {
        tracing::warn!(
            domain = %app_rec.domain,
            dns_state,
            "whitelisting an issuer while its DNS legitimacy record is unverified (admin confirmed)"
        );
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
    // Latest observation — the future daily re-check job overwrites this pair when a record appears.
    app_rec.dns_state = dns_state.to_string();
    app_rec.dns_checked_at = dns_checked_at;
    // Immutable history — what we knew at the moment of granting, and whether we knowingly proceeded.
    // Written only here, never by the re-check job, so "whitelisted while DNS was unverified" stays
    // visible even after the binding later turns verified.
    app_rec.dns_state_at_approval = dns_state.to_string();
    app_rec.dns_proceeded_unverified = dns_state != "verified";
    let proceeded_unverified = app_rec.dns_proceeded_unverified;
    st.store.put_application(app_rec).await;
    ok(json!({
        "status": "approved",
        "whitelistTxs": txs,
        "issuerRoleGranted": issuer_role_granted,
        "issuerRoleTxHash": issuer_role_txs.first().cloned(),
        // The REAL outcome of the legitimacy lookup, always reported: "verified" | "notListed" |
        // "couldNotCheck". Never synthesized, never collapsed to a boolean.
        "dnsState": dns_state,
        "dnsCheckedAt": dns_checked_at,
        // True when this issuer was whitelisted on an explicit admin override rather than a clean pass.
        // The dashboard reads this so such an issuer is never rendered identically to one that passed.
        "dnsProceededUnverified": proceeded_unverified,
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
// The generation-2 `ProviderRegistry` REGISTRAR surface (registry plan C-2).
//
// `registerProvider` and `setServiceCreationApproval` had no caller outside contracts and tests, so
// `providerCount()` was 0 and every provider self-service action refused. These routes are the admin
// half of that journey. Like every other privileged admin write they go through `GovernanceAction`,
// which reads `owner()` LIVE and composes the chain's own predicate rather than re-deriving it - so
// an action flips executed→proposed by construction the moment the registry's owner moves off the
// hosted key, and the UI is never stricter than the contract.
//
// `send_action` awaits the receipt and errors on a reverted status, so `Disposition::Executed`
// means mined-and-succeeded rather than merely submitted.
// ============================================================================================

/// Refuse LOUDLY when `PROVIDER_REGISTRY_ADDR` is unset, in admin-api's existing shape (the
/// `FACTORY_ADDR not configured` precedent) rather than vet-api's silent degrade.
///
/// A registrar screen fed a zero address would answer "no providers exist" - a definite statement
/// about a registry it never asked. This is our own misconfiguration, and it must not be reported as
/// a fact about the chain.
fn provider_registry_addr(st: &AppState) -> Result<String, Resp> {
    let addr = st.cfg.provider_registry_addr.trim();
    if addr.is_empty() || is_zero_addr(addr) {
        return Err(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "PROVIDER_REGISTRY_ADDR not configured - the provider registrar surface cannot read or \
             write without it. Set it to the deployed ProviderRegistry (deployments/roax.json key \
             `ProviderRegistry`) and restart.",
        ));
    }
    if !is_valid_addr(addr) {
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "PROVIDER_REGISTRY_ADDR is malformed (expected a 0x-prefixed 20-byte address)",
        ));
    }
    Ok(addr.to_lowercase())
}

/// The `Authority` every registrar write is gated by: `ProviderRegistry` is `Ownable2Step` with ONE
/// authority key and no independently grantable role (`hasRole` merely projects `owner()`).
fn provider_registry_authority(registry: &str) -> Authority {
    Authority::Owner {
        owner_target: registry.to_string(),
    }
}

/// Read one provider's full registrar view: record, identity anchor, and service-creation approvals.
///
/// The approvals arm is deliberately tri-state. `_serviceCreationApprovals` is private with no
/// getter, so the only direct evidence is the `ServiceCreationApprovalSet` log; a log read that FAILS
/// yields `Unavailable` with its reason and never an empty approval set. Reporting the two the same
/// way would tell an admin a provider is approved for nothing on the strength of a read that never
/// happened - and the remedy for the two differs entirely.
async fn provider_view(st: &AppState, registry: &str, provider_id: &str) -> Result<Value, Resp> {
    let approvals = match st
        .chain
        .service_creation_approval_log(registry, provider_id)
        .await
    {
        Ok(events) => ApprovalsRead::Resolved {
            entries: fold_approvals(&events, &|k| record_type_label(k)),
        },
        Err(e) => approvals_unavailable(&e.to_string()),
    };
    provider_view_with_approvals(st, registry, provider_id, approvals).await
}

/// The `Unavailable` arm's one spelling, so the list and detail paths cannot describe the same
/// failure two ways.
fn approvals_unavailable(reason: &str) -> ApprovalsRead {
    ApprovalsRead::Unavailable {
        reason: format!("the ServiceCreationApprovalSet log could not be read: {reason}"),
    }
}

/// `provider_view` with the approvals already read, which is what lets the list path serve a whole
/// page from ONE registry-wide log scan instead of one scan per provider.
async fn provider_view_with_approvals(
    st: &AppState,
    registry: &str,
    provider_id: &str,
    approvals: ApprovalsRead,
) -> Result<Value, Resp> {
    let record = st
        .chain
        .provider_record(registry, provider_id)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, &format!("provider({provider_id}): {e}")))?;
    // The anchor is only meaningful for a registered provider; for an unregistered id the contract
    // answers a zero-filled struct, which would render as a real anchor with revision 0.
    let anchor = if record.registered {
        match st.chain.provider_identity_anchor(registry, provider_id).await {
            Ok(a) => json!(a),
            Err(e) => json!({ "unavailable": e.to_string() }),
        }
    } else {
        Value::Null
    };
    Ok(json!({
        "provider": record,
        "identityAnchor": anchor,
        "approvals": approvals,
    }))
}

/// `GET /v1/admin/providers` - every registered provider with its standing, controller, identity
/// anchor and service-creation approvals. `_providerIds` is append-only so paging is stable.
async fn providers_list(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };

    // Page to exhaustion rather than truncating: a registrar screen that silently showed the first
    // page would render "the providers that exist" over a subset.
    let mut ids: Vec<String> = Vec::new();
    let mut cursor = 0u64;
    loop {
        let (page, next) = match st.chain.provider_page(&registry, cursor, 100).await {
            Ok(p) => p,
            Err(e) => {
                return err(
                    StatusCode::BAD_GATEWAY,
                    &format!("providerPage(cursor={cursor}): {e}"),
                )
            }
        };
        let empty = page.is_empty();
        ids.extend(page);
        if empty || next <= cursor {
            break;
        }
        cursor = next;
    }

    // ONE registry-wide log scan for the whole page. A per-provider scan cost an unbounded
    // `eth_getLogs` each AND could leave the page MIXED - early providers resolved, later ones
    // `Unavailable` once a rate-limiting or range-capping peer starts refusing - which reads as a
    // fact about those particular providers rather than the uniform "we could not check" it is.
    let approvals_by_provider = st
        .chain
        .service_creation_approval_log_by_provider(&registry)
        .await;

    let mut providers = Vec::with_capacity(ids.len());
    for id in &ids {
        let approvals = match &approvals_by_provider {
            // A provider the resolved log mentions nowhere has been approved for nothing. That IS an
            // answer, so it is an empty `Resolved` and never a could-not-check.
            Ok(by_provider) => ApprovalsRead::Resolved {
                entries: fold_approvals(
                    by_provider.get(&id.to_lowercase()).map_or(&[][..], |e| &e[..]),
                    &|k| record_type_label(k),
                ),
            },
            Err(e) => approvals_unavailable(&e.to_string()),
        };
        match provider_view_with_approvals(&st, &registry, id, approvals).await {
            Ok(v) => providers.push(v),
            Err(e) => return e,
        }
    }

    let hosted = st.chain.signer_address(st.cfg.admin_signer_index).await;
    let owner = st.chain.ownable_owner(&registry).await.ok();
    ok(json!({
        "registry": registry,
        "providers": providers,
        // Stated up front so the screen can say which path a write will take BEFORE the admin fills
        // a form in, exactly as the Issuers page does for factory deploys.
        "authority": {
            "target": registry,
            "owner": owner,
            "hostedSigner": hosted.clone(),
            "heldByHosted": match (&hosted, &owner) {
                (Some(h), Some(o)) => Some(h.eq_ignore_ascii_case(o)),
                _ => None,
            },
            "capability": "registerProvider / setProviderStanding / setServiceCreationApproval",
        },
        "identitySchema": {
            "schema": PROVIDER_IDENTITY_SCHEMA,
            "schemaId": PROVIDER_IDENTITY_SCHEMA_ID,
            "hashAlgorithm": HASH_ALGORITHM_KECCAK256,
        },
    }))
}

/// `GET /v1/admin/providers/:providerId` - one provider's registrar view.
async fn provider_detail(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    if !is_valid_provider_id(&provider_id) {
        return err(
            StatusCode::BAD_REQUEST,
            "providerId must be a non-zero 0x-prefixed 20-byte value",
        );
    }
    match provider_view(&st, &registry, &provider_id.to_lowercase()).await {
        Ok(v) => ok(v),
        Err(e) => e,
    }
}

#[derive(Deserialize)]
struct RegisterProviderReq {
    /// The opaque KYC-assigned identifier. `bytes20`, non-zero, permanent.
    #[serde(rename = "providerId")]
    provider_id: String,
    /// The key that will act AS this provider on the self-service portal.
    controller: String,
    /// keccak256 of the registrar's identity statement. The statement text itself is never sent to
    /// this backend or written on chain - the digest is a commitment to what the registrar asserted.
    #[serde(rename = "identityDigest")]
    identity_digest: String,
}

/// `POST /v1/admin/providers` - register a provider (`onlyOwner`, via `GovernanceAction`).
///
/// This is the real-world KYC gate: the caller is asserting it has cleared this entity. The contract
/// checks only that the ids are non-zero and the anchor is well-formed - everything meaningful about
/// the assertion is the registrar's own.
async fn provider_register(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RegisterProviderReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let provider_id = body.provider_id.trim().to_lowercase();
    if !is_valid_provider_id(&provider_id) {
        return err(
            StatusCode::BAD_REQUEST,
            "providerId must be a non-zero 0x-prefixed 20-byte value (the contract refuses zero with \
             ZeroProviderId)",
        );
    }
    let controller = body.controller.trim().to_lowercase();
    if !is_valid_addr(&controller) || is_zero_addr(&controller) {
        return err(
            StatusCode::BAD_REQUEST,
            "controller must be a valid non-zero 0x-prefixed 20-byte address",
        );
    }
    let digest = body.identity_digest.trim().to_lowercase();
    let digest_hex = digest.strip_prefix("0x").unwrap_or("");
    let digest_well_formed =
        digest_hex.len() == 64 && digest_hex.chars().all(|c| c.is_ascii_hexdigit());
    if !digest_well_formed || digest_hex.chars().all(|c| c == '0') {
        return err(
            StatusCode::BAD_REQUEST,
            "identityDigest must be a non-zero 0x-prefixed 32-byte value (the contract refuses a zero \
             digest with BadIdentityAnchor)",
        );
    }

    // Refuse a re-registration before spending gas on a transaction the contract will revert with
    // `AlreadyRegistered()`. A read that FAILS does not refuse - could-not-check may not stand in for
    // a definite answer, so an unreadable registry still gets its attempt and the on-chain guard
    // remains the real gate.
    match st.chain.provider_record(&registry, &provider_id).await {
        Ok(rec) if rec.registered => {
            return err_json(
                StatusCode::CONFLICT,
                json!({
                    "error": "this providerId is already registered - a providerId is permanent and \
                              cannot be reassigned",
                    "providerId": provider_id,
                    "controller": rec.controller,
                    "standing": rec.standing,
                }),
            )
        }
        Ok(_) => {}
        Err(_) => {}
    }

    let action = GovernanceAction {
        target: registry.clone(),
        calldata: register_provider_calldata(
            &provider_id,
            &controller,
            &digest,
            PROVIDER_IDENTITY_SCHEMA,
            IDENTITY_CODEC_NONE,
            HASH_ALGORITHM_KECCAK256,
        ),
        authority: provider_registry_authority(&registry),
        summary: format!("registerProvider(providerId={provider_id}, controller={controller})"),
    };
    let results = match dispatch_all(&st, &[action]).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let (outcome, executed, warning) = dispatch_summary(&st, &results);
    ok(json!({
        "providerId": provider_id,
        "controller": controller,
        "identityDigest": digest,
        "identitySchema": PROVIDER_IDENTITY_SCHEMA,
        "identitySchemaId": PROVIDER_IDENTITY_SCHEMA_ID,
        // Registration lands the provider at PENDING (ProviderRegistry.sol:300), and
        // `canWriteProvider` admits only ACTIVE - so this alone does NOT let the provider act.
        "standingAfterRegistration": Standing::Pending,
        "nextStep": "setProviderStanding(ACTIVE) - a newly registered provider is PENDING, and \
                     canCreateService folds canWriteProvider, which requires ACTIVE",
        "actions": results,
        "outcome": outcome,
        "executed": executed,
        "warning": warning,
    }))
}

#[derive(Deserialize)]
struct ProviderStandingReq {
    /// `active` | `suspended` | `retired`. The contract reverts `InvalidStanding()` on the other two.
    standing: String,
}

/// `POST /v1/admin/providers/:providerId/standing` - move a provider's standing (`onlyOwner`).
///
/// Required for the journey, not optional: `registerProvider` writes PENDING and `canWriteProvider`
/// admits only ACTIVE, so without this the provider's own deploy preflight reads `provider-standing:
/// fail` forever.
async fn provider_set_standing(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(body): Json<ProviderStandingReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let provider_id = provider_id.trim().to_lowercase();
    if !is_valid_provider_id(&provider_id) {
        return err(
            StatusCode::BAD_REQUEST,
            "providerId must be a non-zero 0x-prefixed 20-byte value",
        );
    }
    let Some(next) = Standing::parse(&body.standing) else {
        return err(
            StatusCode::BAD_REQUEST,
            "standing must be one of active, suspended, retired - the contract refuses none and \
             pending with InvalidStanding()",
        );
    };

    // Same rule as registration: a definite read may refuse a doomed transaction, an unreadable one
    // may not. `NoChange()` and `RetiredStanding()` are both real reverts worth not paying for.
    if let Ok(rec) = st.chain.provider_record(&registry, &provider_id).await {
        if !rec.registered {
            return err(
                StatusCode::NOT_FOUND,
                "this providerId is not registered - register it before setting a standing",
            );
        }
        if rec.standing == next {
            return err_json(
                StatusCode::CONFLICT,
                json!({
                    "error": format!("this provider is already {:?} - the contract refuses a no-op \
                                      standing change with NoChange()", next),
                    "standing": rec.standing,
                }),
            );
        }
        if rec.standing == Standing::Retired {
            return err_json(
                StatusCode::CONFLICT,
                json!({
                    "error": "RETIRED is terminal - the contract refuses every further transition \
                              with RetiredStanding()",
                    "standing": rec.standing,
                }),
            );
        }
    }

    let action = GovernanceAction {
        target: registry.clone(),
        calldata: set_provider_standing_calldata(&provider_id, next),
        authority: provider_registry_authority(&registry),
        summary: format!("setProviderStanding(providerId={provider_id}, standing={next:?})"),
    };
    let results = match dispatch_all(&st, &[action]).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let (outcome, executed, warning) = dispatch_summary(&st, &results);
    ok(json!({
        "providerId": provider_id,
        "standing": next,
        "actions": results,
        "outcome": outcome,
        "executed": executed,
        "warning": warning,
    }))
}

#[derive(Deserialize)]
struct ServiceApprovalReq {
    /// A record-type label ("VACCINATION") or an explicit `0x`+64-hex key.
    #[serde(rename = "recordType")]
    record_type: String,
    /// The intended state, sent explicitly rather than as a toggle so the request says what it wants
    /// rather than what it thinks is currently true.
    allowed: bool,
}

/// `POST /v1/admin/providers/:providerId/service-approval` - pre-authorize a provider to ask the
/// self-service factory for a clone of one record type (`onlyOwner`).
///
/// This grants no issuance and attaches no clone; it is the gate the provider's `createIssuer` is
/// checked against.
async fn provider_set_service_approval(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(body): Json<ServiceApprovalReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let provider_id = provider_id.trim().to_lowercase();
    if !is_valid_provider_id(&provider_id) {
        return err(
            StatusCode::BAD_REQUEST,
            "providerId must be a non-zero 0x-prefixed 20-byte value",
        );
    }
    let label = body.record_type.trim();
    if label.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "recordType is required (a label like VACCINATION, or an explicit 0x + 64 hex key)",
        );
    }
    let key = to_record_type_key(label);
    if key.trim_start_matches("0x").chars().all(|c| c == '0') {
        return err(
            StatusCode::BAD_REQUEST,
            "recordType resolves to the zero key - the contract refuses it with \
             InvalidServiceMetadata()",
        );
    }

    if let Ok(rec) = st.chain.provider_record(&registry, &provider_id).await {
        if !rec.registered {
            return err(
                StatusCode::NOT_FOUND,
                "this providerId is not registered - register it before approving a record type",
            );
        }
    }
    // `setServiceCreationApproval` reverts `NoChange()` on a redundant write, so a definite read of
    // the current bit avoids paying for a doomed transaction. An UNAVAILABLE log deliberately does
    // NOT refuse: we could not check, and could-not-check may not stand in for "already set" and
    // block a legitimate approval. The contract's own guard remains the real gate.
    match st
        .chain
        .service_creation_approval_log(&registry, &provider_id)
        .await
    {
        Ok(events) => {
            let read = ApprovalsRead::Resolved {
                entries: fold_approvals(&events, &|k| record_type_label(k)),
            };
            if read.approved(&key) == Some(body.allowed) {
                return err_json(
                    StatusCode::CONFLICT,
                    json!({
                        "error": format!(
                            "this provider's {label} approval is already {} - the contract refuses a \
                             no-op with NoChange()",
                            body.allowed
                        ),
                        "recordType": label,
                        "recordTypeKey": key,
                        "allowed": body.allowed,
                    }),
                );
            }
        }
        Err(_) => { /* could-not-check: attempt the write rather than refusing it */ }
    }

    let verb = if body.allowed { "approve" } else { "withdraw" };
    let action = GovernanceAction {
        target: registry.clone(),
        calldata: set_service_creation_approval_calldata(&provider_id, &key, body.allowed),
        authority: provider_registry_authority(&registry),
        summary: format!(
            "setServiceCreationApproval({verb} providerId={provider_id}, recordType={label})"
        ),
    };
    let results = match dispatch_all(&st, &[action]).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let (outcome, executed, warning) = dispatch_summary(&st, &results);
    ok(json!({
        "providerId": provider_id,
        "recordType": label,
        "recordTypeKey": key,
        "allowed": body.allowed,
        "actions": results,
        "outcome": outcome,
        "executed": executed,
        "warning": warning,
    }))
}

// ============================================================================================
// The rest of the journey: attach → stand up → grant issuance, plus the two orthogonal levers.
//
// Approving a record type lets a provider DEPLOY a contract and nothing more. Everything past that
// point is registrar work with no caller until now, which is why `serviceCount()` was 0 on a live
// registry while providers were already deploying: `repointService` refuses an address that was
// never attached, so a provider could deploy and then go nowhere.
//
// Five `onlyOwner` calls complete it, and two of them are easy to leave out:
//
//  - `attachService` binds a provider-deployed contract to its provider record. This is the one that
//    unblocks everything downstream.
//  - `setServiceStanding` is REQUIRED, not optional. Attachment lands the service at PENDING exactly
//    as registration lands a provider there, and `canIssue` folds the service standing - so a
//    journey that attaches and stops is still a broken journey.
//  - `setIssuanceCapability` / `setVerifierCapability` are the two capability axes.
//  - `setResolverApproved` is the fleet-wide lever a typed resolver needs before any provider can
//    select it, which is what unblocks the provider's domain and directory-listing flows.
// ============================================================================================

/// A malformed address is a 400 the caller can fix; this is the one shape check every service route
/// shares so they cannot disagree about what an address is.
fn valid_service_addr(addr: &str) -> Result<String, Resp> {
    let a = addr.trim().to_lowercase();
    if !is_valid_addr(&a) || is_zero_addr(&a) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "serviceAddress must be a valid non-zero 0x-prefixed 20-byte address",
        ));
    }
    Ok(a)
}

/// Read one capability log into the tri-state the screen renders.
///
/// The `Unavailable` arm's ONE spelling, so a failed issuance read and a failed verifier read cannot
/// describe the same class of failure two different ways.
fn capabilities_from(result: Result<Vec<(String, bool)>, crate::chain::ChainError>, what: &str) -> CapabilitiesRead {
    match result {
        Ok(events) => CapabilitiesRead::Resolved {
            entries: fold_capabilities(&events),
        },
        Err(e) => CapabilitiesRead::Unavailable {
            reason: format!("the {what} log could not be read: {e}"),
        },
    }
}

/// `GET /v1/admin/providers/:providerId/services` - every service attached to one provider, with the
/// five lifecycle terms and the current issuance holders.
///
/// A SEPARATE route from the provider list rather than a field on it: this is one `eth_call` plus one
/// `eth_getLogs` PER SERVICE, so folding it into the list would make an unbounded per-provider walk
/// out of a page that is already walking every provider.
async fn provider_services(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let provider_id = provider_id.trim().to_lowercase();
    if !is_valid_provider_id(&provider_id) {
        return err(
            StatusCode::BAD_REQUEST,
            "providerId must be a non-zero 0x-prefixed 20-byte value",
        );
    }

    // Page to exhaustion: a screen showing the first page as "the services that exist" would let an
    // admin attach a duplicate, or miss the one that is actually current.
    let mut addrs: Vec<String> = Vec::new();
    let mut cursor = 0u64;
    loop {
        let (page, next) = match st
            .chain
            .provider_service_page(&registry, &provider_id, cursor, 100)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                return err(
                    StatusCode::BAD_GATEWAY,
                    &format!("providerServicePage(cursor={cursor}): {e}"),
                )
            }
        };
        let empty = page.is_empty();
        addrs.extend(page);
        if empty || next <= cursor {
            break;
        }
        cursor = next;
    }

    let mut services = Vec::with_capacity(addrs.len());
    for addr in &addrs {
        let record = match st.chain.service_record(&registry, addr).await {
            Ok(r) => r,
            Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("service({addr}): {e}")),
        };
        // The five terms, reported APART. Pre-ANDing them would leave an admin able to see that
        // something is wrong and unable to see which of five different remedies applies.
        let effective = match st.chain.service_effective(&registry, addr).await {
            Ok(e) => json!(e),
            Err(e) => json!({ "unavailable": e.to_string() }),
        };
        // Whether the PROVIDER has published this service as its current pointer for that record
        // type. That is the provider's own `repointService` decision and no registrar route writes
        // it - the screen shows it so an admin can see the journey is finished, not act on it.
        let current = match st
            .chain
            .current_service(&registry, &provider_id, &record.record_type_key)
            .await
        {
            Ok(c) => json!({
                "state": "resolved",
                "service": c,
                "isCurrent": c.eq_ignore_ascii_case(addr),
            }),
            Err(e) => json!({ "state": "unavailable", "reason": e.to_string() }),
        };
        let issuance = capabilities_from(
            st.chain.issuance_capability_log(&registry, addr).await,
            "IssuanceCapabilitySet",
        );
        services.push(json!({
            "service": record,
            "effective": effective,
            "currentPointer": current,
            "issuance": issuance,
        }));
    }

    ok(json!({ "registry": registry, "providerId": provider_id, "services": services }))
}

#[derive(Deserialize)]
struct AttachPreflightReq {
    #[serde(rename = "serviceAddress")]
    service_address: String,
}

/// `POST /v1/admin/providers/:providerId/services/preflight` - what the chain says about a candidate
/// contract, BEFORE anything is signed.
///
/// It mirrors the three reads `attachService` itself makes and re-derives none of them:
/// `factory.isClone(service)` against each ACTIVE generation, then `owner()` and `recordType()` off
/// the service. Two rules keep it honest:
///
///  - It is never STRICTER than the chain. A probe that FAILED is could-not-run and does not refuse
///    the send; the on-chain guard is the real gate, and a preflight that refuses what the contract
///    would accept is a worse defect than no preflight at all.
///  - `expectedOwner` is prefilled from the LIVE `owner()`. It is a transaction guard against a
///    second handover between review and send, never a selector - the resolved owner is what the
///    contract stores whatever this says.
///
/// The single most likely thing an admin will try first is a generation-1 `DogTagIssuer`, which is
/// `Initializable` only and has NO `owner()` at all. That is a permanent property of the contract
/// rather than a fixable form error, so it is said in words here rather than surfacing as a raw
/// `InvalidServiceMetadata()` revert.
async fn provider_service_preflight(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(body): Json<AttachPreflightReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let provider_id = provider_id.trim().to_lowercase();
    if !is_valid_provider_id(&provider_id) {
        return err(
            StatusCode::BAD_REQUEST,
            "providerId must be a non-zero 0x-prefixed 20-byte value",
        );
    }
    let service = match valid_service_addr(&body.service_address) {
        Ok(a) => a,
        Err(e) => return e,
    };

    // Already attached? A definite yes refuses (`AlreadyRegistered()`); a read that failed does not.
    let already = match st.chain.service_record(&registry, &service).await {
        Ok(r) if r.attached => Some(r),
        Ok(_) => None,
        Err(_) => None,
    };

    // Which generation's pinned factory recognizes this clone. The admin is never asked to type a
    // bytes32 for it: `attachService` resolves the factory from the generation and proves
    // `isClone`, so probing that same predicate is composing the chain's answer rather than guessing.
    let generations = st.chain.factory_generations(&registry).await;
    let generation = match &generations {
        Ok(gens) => {
            let mut resolved: Option<(String, String)> = None;
            let mut probe_failed: Option<String> = None;
            for (gid, factory, active) in gens {
                if !active {
                    continue;
                }
                match st.chain.is_clone(factory, &service).await {
                    Ok(true) => {
                        resolved = Some((gid.clone(), factory.clone()));
                        break;
                    }
                    Ok(false) => {}
                    // A probe that could not be MADE is not evidence that the factory disowns the
                    // clone. Remembered so a "no generation recognizes this" verdict is never
                    // reported on the strength of a read that never happened.
                    Err(e) => probe_failed = Some(e.to_string()),
                }
            }
            match (resolved, probe_failed) {
                (Some((gid, factory)), _) => {
                    json!({ "state": "resolved", "generationId": gid, "factory": factory })
                }
                (None, Some(reason)) => json!({
                    "state": "unavailable",
                    "reason": format!("a factory isClone probe could not be made: {reason}"),
                }),
                (None, None) => json!({
                    "state": "none",
                    "reason": "no active factory generation recognizes this address as one of its \
                               clones - it was not deployed by this registry's factory",
                }),
            }
        }
        Err(e) => json!({
            "state": "unavailable",
            "reason": format!("the factory generation list could not be read: {e}"),
        }),
    };

    let metadata = match st.chain.service_metadata(&service).await {
        Ok((owner, rt_key)) => {
            let zero_rt = rt_key.trim_start_matches("0x").chars().all(|c| c == '0');
            if is_zero_addr(&owner) || zero_rt {
                json!({
                    "state": "refused",
                    "reason": "the contract answered a zero owner or record type, which \
                               attachService refuses with InvalidServiceMetadata()",
                })
            } else {
                json!({
                    "state": "resolved",
                    "owner": owner,
                    "recordTypeKey": rt_key,
                    "recordType": record_type_label(&rt_key),
                })
            }
        }
        Err(e) => json!({
            "state": "unavailable",
            "reason": format!(
                "the contract did not answer owner() and recordType(): {e}. A generation-1 \
                 DogTagIssuer has no owner at all, so it can never be attached - that is a property \
                 of the contract, not something a different expected owner would fix"
            ),
        }),
    };

    // The verdict is the FOLD, and it has three values because two of the inputs do. `refused` says
    // the chain would reject this; `unavailable` says we could not establish it and the send is
    // still offered, because could-not-check may not refuse an action the contract might accept.
    let (verdict, reason) = if let Some(r) = &already {
        (
            "refused",
            format!(
                "this address is already attached to provider {} - a service binds to one provider \
                 and attachService refuses a second with AlreadyRegistered()",
                r.provider_id
            ),
        )
    } else if generation["state"] == "none" {
        ("refused", generation["reason"].as_str().unwrap_or("").to_string())
    } else if metadata["state"] == "refused" {
        ("refused", metadata["reason"].as_str().unwrap_or("").to_string())
    } else if generation["state"] == "resolved" && metadata["state"] == "resolved" {
        ("ready", String::new())
    } else {
        (
            "couldNotRun",
            "one of the reads attachService itself makes could not be completed, so whether it \
             would succeed is not established - the send is still offered and the contract's own \
             guards remain the real gate"
                .into(),
        )
    };

    ok(json!({
        "registry": registry,
        "providerId": provider_id,
        "serviceAddress": service,
        "alreadyAttached": already,
        "generation": generation,
        "metadata": metadata,
        "verdict": verdict,
        "reason": reason,
    }))
}

#[derive(Deserialize)]
struct AttachServiceReq {
    #[serde(rename = "serviceAddress")]
    service_address: String,
    /// The generation whose pinned factory deployed this clone, as the preflight resolved it.
    #[serde(rename = "generationId")]
    generation_id: String,
    /// The owner as REVIEWED. A transaction guard against a second handover between review and send;
    /// the contract compares it against the owner it reads and stores the resolved one either way.
    #[serde(rename = "expectedOwner")]
    expected_owner: String,
}

/// `POST /v1/admin/providers/:providerId/services` - attach a provider-deployed contract (`onlyOwner`).
async fn provider_attach_service(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(body): Json<AttachServiceReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let provider_id = provider_id.trim().to_lowercase();
    if !is_valid_provider_id(&provider_id) {
        return err(
            StatusCode::BAD_REQUEST,
            "providerId must be a non-zero 0x-prefixed 20-byte value",
        );
    }
    let service = match valid_service_addr(&body.service_address) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let expected_owner = body.expected_owner.trim().to_lowercase();
    if !is_valid_addr(&expected_owner) || is_zero_addr(&expected_owner) {
        return err(
            StatusCode::BAD_REQUEST,
            "expectedOwner must be a valid non-zero address (the contract refuses zero with \
             ZeroAddress) - it is the owner you reviewed, and a mismatch refuses the transaction",
        );
    }
    let generation_id = body.generation_id.trim().to_lowercase();
    let gen_hex = generation_id.strip_prefix("0x").unwrap_or("");
    if gen_hex.len() != 64 || !gen_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return err(
            StatusCode::BAD_REQUEST,
            "generationId must be a 0x-prefixed 32-byte value - take it from the preflight rather \
             than typing it",
        );
    }

    // Refuse a doomed second attach on a DEFINITE read only. An unreadable registry still gets its
    // attempt: could-not-check may not stand in for a definite answer.
    if let Ok(r) = st.chain.service_record(&registry, &service).await {
        if r.attached {
            return err_json(
                StatusCode::CONFLICT,
                json!({
                    "error": "this address is already attached - attachService refuses a second \
                              binding with AlreadyRegistered(). Use reassignServiceProvider to \
                              correct a mistaken provider binding.",
                    "serviceAddress": service,
                    "providerId": r.provider_id,
                    "standing": r.standing,
                }),
            );
        }
    }

    let action = GovernanceAction {
        target: registry.clone(),
        calldata: attach_service_calldata(&provider_id, &service, &generation_id, &expected_owner),
        authority: provider_registry_authority(&registry),
        summary: format!("attachService(providerId={provider_id}, service={service})"),
    };
    let results = match dispatch_all(&st, &[action]).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let (outcome, executed, warning) = dispatch_summary(&st, &results);
    ok(json!({
        "providerId": provider_id,
        "serviceAddress": service,
        "generationId": generation_id,
        "expectedOwner": expected_owner,
        // Attachment lands the service at PENDING, and `canIssue` folds the service standing - so
        // this alone does NOT let the provider issue, exactly as registration alone does not let a
        // provider act.
        "standingAfterAttach": Standing::Pending,
        "nextStep": "setServiceStanding(ACTIVE), then setIssuanceCapability for the signer that \
                     will issue - attachment alone grants nothing",
        "actions": results,
        "outcome": outcome,
        "executed": executed,
        "warning": warning,
    }))
}

#[derive(Deserialize)]
struct ServiceStandingReq {
    /// `active` | `suspended` | `retired`, same three the contract admits.
    standing: String,
}

/// `POST /v1/admin/services/:serviceAddress/standing` - move a service's standing (`onlyOwner`).
async fn service_set_standing(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(service_address): Path<String>,
    Json(body): Json<ServiceStandingReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let service = match valid_service_addr(&service_address) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Some(next) = Standing::parse(&body.standing) else {
        return err(
            StatusCode::BAD_REQUEST,
            "standing must be one of active, suspended, retired - the contract refuses none and \
             pending with InvalidStanding()",
        );
    };

    if let Ok(rec) = st.chain.service_record(&registry, &service).await {
        if !rec.attached {
            return err(
                StatusCode::NOT_FOUND,
                "this address is not an attached service - attach it to a provider first",
            );
        }
        if rec.standing == next {
            return err_json(
                StatusCode::CONFLICT,
                json!({
                    "error": format!("this service is already {:?} - the contract refuses a no-op \
                                      standing change with NoChange()", next),
                    "standing": rec.standing,
                }),
            );
        }
        if rec.standing == Standing::Retired {
            return err_json(
                StatusCode::CONFLICT,
                json!({
                    "error": "RETIRED is terminal - the contract refuses every further transition \
                              with RetiredStanding()",
                    "standing": rec.standing,
                }),
            );
        }
    }

    let action = GovernanceAction {
        target: registry.clone(),
        calldata: set_service_standing_calldata(&service, next),
        authority: provider_registry_authority(&registry),
        summary: format!("setServiceStanding(service={service}, standing={next:?})"),
    };
    let results = match dispatch_all(&st, &[action]).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let (outcome, executed, warning) = dispatch_summary(&st, &results);
    ok(json!({
        "serviceAddress": service,
        "standing": next,
        "actions": results,
        "outcome": outcome,
        "executed": executed,
        "warning": warning,
    }))
}

#[derive(Deserialize)]
struct IssuanceCapabilityReq {
    /// The key that will sign issuances on this service.
    signer: String,
    /// Stated explicitly rather than as a toggle, so the request says what it wants rather than what
    /// it believes is currently true.
    allowed: bool,
}

/// `POST /v1/admin/services/:serviceAddress/issuance-capability` - grant or withdraw the right to
/// issue on one service (`onlyOwner`).
///
/// This is one half of what replaced the deleted record-type whitelist. Note what it does NOT do:
/// `setServiceDelegate` grants CONTENT-WRITE permissions and does not satisfy `canIssue`, so a
/// server-held key is granted here and by the registrar alone - "the provider grants its own server
/// key" is not reachable on the deployed contract.
async fn service_set_issuance_capability(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(service_address): Path<String>,
    Json(body): Json<IssuanceCapabilityReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let service = match valid_service_addr(&service_address) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let signer = body.signer.trim().to_lowercase();
    if !is_valid_addr(&signer) || is_zero_addr(&signer) {
        return err(
            StatusCode::BAD_REQUEST,
            "signer must be a valid non-zero 0x-prefixed 20-byte address",
        );
    }

    if let Ok(rec) = st.chain.service_record(&registry, &service).await {
        if !rec.attached {
            return err(
                StatusCode::NOT_FOUND,
                "this address is not an attached service - attachService binds it to a provider \
                 first, and setIssuanceCapability refuses an unknown service with UnknownService()",
            );
        }
    }
    // A definite current bit refuses a `NoChange()` revert; an UNAVAILABLE log deliberately does not,
    // because could-not-check must not block a legitimate grant. The contract stays the real gate.
    let read = capabilities_from(
        st.chain.issuance_capability_log(&registry, &service).await,
        "IssuanceCapabilitySet",
    );
    if read.allowed(&signer) == Some(body.allowed) {
        return err_json(
            StatusCode::CONFLICT,
            json!({
                "error": format!(
                    "this signer's issuance capability on {service} is already {} - the contract \
                     refuses a no-op with NoChange()",
                    body.allowed
                ),
                "signer": signer,
                "allowed": body.allowed,
            }),
        );
    }

    let verb = if body.allowed { "grant" } else { "withdraw" };
    let action = GovernanceAction {
        target: registry.clone(),
        calldata: set_issuance_capability_calldata(&service, &signer, body.allowed),
        authority: provider_registry_authority(&registry),
        summary: format!("setIssuanceCapability({verb} service={service}, signer={signer})"),
    };
    let results = match dispatch_all(&st, &[action]).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let (outcome, executed, warning) = dispatch_summary(&st, &results);
    ok(json!({
        "serviceAddress": service,
        "signer": signer,
        "allowed": body.allowed,
        "actions": results,
        "outcome": outcome,
        "executed": executed,
        "warning": warning,
    }))
}

/// `GET /v1/admin/verifier-capabilities` - who may verify, per purpose.
///
/// Keyed by PURPOSE and not by service, because the verify axis is ORTHOGONAL to issuance: rendering
/// it inside a service row would present it as a property of that service, which it is not. An issuer
/// is not implicitly a verifier and vice versa.
async fn verifier_capabilities(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let mut purposes = Vec::with_capacity(KNOWN_VERIFY_PURPOSES.len());
    for label in KNOWN_VERIFY_PURPOSES {
        let key = purpose_key(label);
        let relayers = capabilities_from(
            st.chain.verifier_capability_log(&registry, &key).await,
            "VerifierCapabilitySet",
        );
        purposes.push(json!({
            "purpose": label,
            // The bytes32 the contract takes. It derives `verificationKey(purpose)` itself, so this
            // is the RAW purpose and never an already-derived key.
            "purposeKey": key,
            "relayers": relayers,
        }));
    }
    ok(json!({ "registry": registry, "purposes": purposes }))
}

#[derive(Deserialize)]
struct VerifierCapabilityReq {
    /// A purpose label ("travel_check") or an explicit `0x`+64-hex purpose word.
    purpose: String,
    /// The relayer address that submits verifications for that purpose.
    relayer: String,
    allowed: bool,
}

/// `POST /v1/admin/verifier-capabilities` - grant or withdraw a relayer's right to verify for one
/// purpose (`onlyOwner`). The other half of what replaced the deleted record-type whitelist.
async fn verifier_capability_set(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<VerifierCapabilityReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let relayer = body.relayer.trim().to_lowercase();
    if !is_valid_addr(&relayer) || is_zero_addr(&relayer) {
        return err(
            StatusCode::BAD_REQUEST,
            "relayer must be a valid non-zero 0x-prefixed 20-byte address",
        );
    }
    let label = body.purpose.trim();
    if label.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "purpose is required (a label like travel_check, or an explicit 0x + 64 hex word)",
        );
    }
    // The RAW purpose word. `setVerifierCapability` derives `verificationKey(purpose)` itself, so
    // passing an already-derived key would derive twice and write the capability under a key
    // `canVerify` never reads - a transaction that succeeds and grants nothing.
    //
    // An explicit word is validated rather than passed through: `parse_b256` coerces a malformed
    // value to the ZERO word, which the contract accepts (it refuses only a zero relayer), so the
    // grant would land under a purpose no verifier ever asks about while the response echoed the
    // malformed string back as `purposeKey`. Same guard `attachService`'s `generationId` applies.
    let key = match label.strip_prefix("0x") {
        Some(h) if h.len() == 64 => {
            if !h.chars().all(|c| c.is_ascii_hexdigit()) || h.chars().all(|c| c == '0') {
                return err(
                    StatusCode::BAD_REQUEST,
                    "an explicit purpose must be a non-zero 0x-prefixed 32-byte hex word - a \
                     malformed one coerces to the zero word and would grant under a purpose no \
                     verifier reads. Send a label like travel_check to have it keccak'd instead",
                );
            }
            format!("0x{}", h.to_lowercase())
        }
        _ => purpose_key(label),
    };

    let read = capabilities_from(
        st.chain.verifier_capability_log(&registry, &key).await,
        "VerifierCapabilitySet",
    );
    if read.allowed(&relayer) == Some(body.allowed) {
        return err_json(
            StatusCode::CONFLICT,
            json!({
                "error": format!(
                    "this relayer's {label} verify capability is already {} - the contract refuses \
                     a no-op with NoChange()",
                    body.allowed
                ),
                "purpose": label,
                "purposeKey": key,
                "allowed": body.allowed,
            }),
        );
    }

    let verb = if body.allowed { "grant" } else { "withdraw" };
    let action = GovernanceAction {
        target: registry.clone(),
        calldata: set_verifier_capability_calldata(&key, &relayer, body.allowed),
        authority: provider_registry_authority(&registry),
        summary: format!("setVerifierCapability({verb} purpose={label}, relayer={relayer})"),
    };
    let results = match dispatch_all(&st, &[action]).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let (outcome, executed, warning) = dispatch_summary(&st, &results);
    ok(json!({
        "purpose": label,
        "purposeKey": key,
        "relayer": relayer,
        "allowed": body.allowed,
        "actions": results,
        "outcome": outcome,
        "executed": executed,
        "warning": warning,
    }))
}

/// `GET /v1/admin/resolvers` - the typed resolver allowlist, both kinds.
///
/// A typed resolver answers NOTHING until BOTH halves hold: the registrar approves it here, AND each
/// provider or service selects it. The core never clears a stored selection when a resolver is
/// deapproved - that is exactly why the approval is a fleet-wide lever - so the two are reported
/// separately and are never pre-ANDed into one "working" bool.
async fn resolvers_list(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let mut kinds = Vec::new();
    for kind in [ResolverKind::Directory, ResolverKind::Domain] {
        let entry = match st.chain.approved_resolvers(&registry, kind).await {
            Ok(list) => json!({
                "state": "resolved",
                "resolvers": list
                    .into_iter()
                    .map(|(addr, approved)| json!({ "resolver": addr, "approved": approved }))
                    .collect::<Vec<_>>(),
            }),
            // Could-not-read is its own state: an empty allowlist is a fact about the registry,
            // and reporting a failed read as one would say nothing is approved on the strength of a
            // read that never happened.
            Err(e) => json!({ "state": "unavailable", "reason": e.to_string() }),
        };
        kinds.push(json!({ "kind": kind, "listing": entry }));
    }
    ok(json!({ "registry": registry, "kinds": kinds }))
}

#[derive(Deserialize)]
struct ResolverApprovalReq {
    /// `directory` | `domain`.
    kind: String,
    resolver: String,
    approved: bool,
}

/// `POST /v1/admin/resolvers` - approve or pull a typed resolver (`onlyOwner`).
///
/// This is what unblocks the provider's domain claim and directory listing: both refuse with
/// `ResolverNotApproved()` until the registrar has approved the resolver, and the provider's own
/// selection cannot precede it.
async fn resolver_set_approved(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ResolverApprovalReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let registry = match provider_registry_addr(&st) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Some(kind) = ResolverKind::parse(&body.kind) else {
        return err(
            StatusCode::BAD_REQUEST,
            "kind must be one of directory, domain",
        );
    };
    let resolver = body.resolver.trim().to_lowercase();
    if !is_valid_addr(&resolver) || is_zero_addr(&resolver) {
        return err(
            StatusCode::BAD_REQUEST,
            "resolver must be a valid non-zero 0x-prefixed 20-byte address",
        );
    }

    if let Ok(list) = st.chain.approved_resolvers(&registry, kind).await {
        let current = list
            .iter()
            .find(|(a, _)| a.eq_ignore_ascii_case(&resolver))
            .map(|(_, approved)| *approved)
            .unwrap_or(false);
        if current == body.approved {
            return err_json(
                StatusCode::CONFLICT,
                json!({
                    "error": format!(
                        "this resolver's {:?} approval is already {} - the contract refuses a no-op \
                         with NoChange()",
                        kind, body.approved
                    ),
                    "resolver": resolver,
                    "approved": body.approved,
                }),
            );
        }
    }

    let verb = if body.approved { "approve" } else { "pull" };
    let action = GovernanceAction {
        target: registry.clone(),
        calldata: set_resolver_approved_calldata(kind, &resolver, body.approved),
        authority: provider_registry_authority(&registry),
        summary: format!("setResolverApproved({verb} kind={kind:?}, resolver={resolver})"),
    };
    let results = match dispatch_all(&st, &[action]).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let (outcome, executed, warning) = dispatch_summary(&st, &results);
    ok(json!({
        "kind": kind,
        "resolver": resolver,
        "approved": body.approved,
        // Approval is only HALF: the provider or service must still select this resolver, and the
        // registrar cannot do that for them.
        "nextStep": "the provider must now select this resolver - approval alone resolves nothing",
        "actions": results,
        "outcome": outcome,
        "executed": executed,
        "warning": warning,
    }))
}

// ============================================================================================
// Shared governance dispatch helpers.
//
// These were introduced with the whitelist console (PR-E) and OUTLIVED it: the registrar routes
// above are now their only callers, and the tri-state `outcome` they produce is what keeps a
// designed out-of-band proposal apart from a stack booted on a key that lost its authority.
// ============================================================================================


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
        let Some(obj) = ev.as_object_mut() else {
            continue;
        };
        if let Some(actor) = obj
            .get("actor")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        {
            if let Some(name) = dir.name(&actor) {
                obj.insert("actorName".into(), json!(name));
            }
        }
        if let Some(clone) = obj
            .get("clone")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        {
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
                    let Some(obj) = it.as_object_mut() else {
                        continue;
                    };
                    if let Some(clone) = obj
                        .get("clone")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
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
        // The `ProviderRegistry` REGISTRAR surface - the whole provider journey, register through
        // capability grant. `setIssuanceCapability` and `setVerifierCapability` below are what
        // replaced the deleted `whitelistFor` console: that console called `isWhitelistedFor` and
        // `whitelistFor` on the single authority, which answers the first off an orthogonal axis
        // (a definite `false` for every genuine issuer signer) and does not implement the second.
        .route("/v1/admin/providers", get(providers_list).post(provider_register))
        .route("/v1/admin/providers/:providerId", get(provider_detail))
        .route(
            "/v1/admin/providers/:providerId/standing",
            post(provider_set_standing),
        )
        .route(
            "/v1/admin/providers/:providerId/service-approval",
            post(provider_set_service_approval),
        )
        .route(
            "/v1/admin/providers/:providerId/services",
            get(provider_services).post(provider_attach_service),
        )
        .route(
            "/v1/admin/providers/:providerId/services/preflight",
            post(provider_service_preflight),
        )
        .route(
            "/v1/admin/services/:serviceAddress/standing",
            post(service_set_standing),
        )
        .route(
            "/v1/admin/services/:serviceAddress/issuance-capability",
            post(service_set_issuance_capability),
        )
        .route(
            "/v1/admin/verifier-capabilities",
            get(verifier_capabilities).post(verifier_capability_set),
        )
        .route(
            "/v1/admin/resolvers",
            get(resolvers_list).post(resolver_set_approved),
        )
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
        // rows whose stored location is exactly 0,0 and therefore needs an operator's answer
        .route(
            "/v1/admin/businesses/location-review",
            get(businesses_location_review),
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

    // ========================================================================================
    // Cross-language parity for the DEPRECATED `near=` filter.
    //
    // `packages/ui/src/geo/` replaces this server-side distance filter with an on-device one (see
    // the deprecation note on `BusinessesQuery`). "Replaces" is only a safe claim if the two admit
    // the SAME providers, so the claim is pinned from BOTH ends against one committed fixture, in
    // the same shape as `clientHandoffIcs.test.ts` pins the `.ics` bytes:
    //
    //   * this test asserts the fixture is what THIS `haversine_km` and THIS `<= radius` filter
    //     actually produce - so the fixture is generated by the deprecated code, not by someone's
    //     reading of it;
    //   * `packages/ui/test/geoParity.test.ts` asserts the TypeScript reproduces the same
    //     `admits` decisions.
    //
    // A hand-written TS port as the oracle would only have proved the port self-consistent.
    //
    // To regenerate after an intentional change:
    //     cargo test -p admin-api -- --ignored --nocapture print_geo_parity_fixture
    // and paste the printed JSON over `packages/ui/test/fixtures/geo-parity.json`.
    // ========================================================================================

    const GEO_PARITY_FIXTURE: &str =
        include_str!("../../../../packages/ui/test/fixtures/geo-parity.json");

    /// (name, lat1, lon1, lat2, lon2). Chosen for the cases that actually bite: the antimeridian,
    /// both poles, identical points, and the equator-versus-high-latitude gap where a naive
    /// equirectangular approximation diverges from haversine.
    const GEO_PARITY_CASES: &[(&str, f64, f64, f64, f64)] = &[
        ("identical_points_equator", 0.0, 0.0, 0.0, 0.0),
        (
            "identical_points_high_lat",
            51.5074,
            -0.1278,
            51.5074,
            -0.1278,
        ),
        ("one_degree_lon_at_equator", 0.0, 0.0, 0.0, 1.0),
        // The equirectangular divergence: one degree of longitude shrinks by cos(lat), so these
        // three are ~111 km, ~56 km and ~19 km. An approximation that ignores cos(lat) reports all
        // three as ~111 km.
        ("one_degree_lon_at_lat_60", 60.0, 0.0, 60.0, 1.0),
        ("one_degree_lon_at_lat_80", 80.0, 0.0, 80.0, 1.0),
        ("one_degree_lat_meridian", 0.0, 0.0, 1.0, 0.0),
        // Antimeridian: 0.2 degrees apart, NOT 359.8. An implementation that subtracts raw
        // longitudes without wrapping reports half the planet.
        ("antimeridian_short_hop", 0.0, 179.9, 0.0, -179.9),
        ("antimeridian_high_lat", 60.0, 179.9, 60.0, -179.9),
        ("north_pole_to_equator", 90.0, 0.0, 0.0, 0.0),
        ("south_pole_to_equator", -90.0, 0.0, 0.0, 0.0),
        ("pole_to_pole", 90.0, 0.0, -90.0, 0.0),
        // Same physical point; longitude is meaningless at a pole.
        ("north_pole_differing_longitudes", 90.0, 0.0, 90.0, 73.0),
        ("exact_antipode_equator", 0.0, 0.0, 0.0, 180.0),
        // `a` rounds above 1.0 here, so `asin(sqrt(a))` is NaN - in BOTH Rust and V8, which is what
        // makes it usable as a cross-language fixture case. See
        // `the_deprecated_filter_silently_drops_a_near_antipodal_provider`.
        (
            "near_antipode_nan",
            66.517_526_663_907_92,
            121.879_438_417_269_34,
            -66.517_526_664_399_88,
            -58.120_561_582_730_66,
        ),
        ("london_to_singapore", 51.5074, -0.1278, 1.3521, 103.8198),
        ("nyc_to_la", 40.7128, -74.0060, 34.0522, -118.2437),
        // The actual use case: a city-scale hop between two providers.
        ("city_scale_hop", 1.3521, 103.8198, 1.3600, 103.8300),
        ("sub_metre", 0.0, 0.0, 0.0, 0.000_001),
    ];

    /// Radii the fixture records an admit/reject decision for, spanning the range the deprecated
    /// route accepts (its default is 50 km).
    const GEO_PARITY_RADII: &[f64] = &[0.5, 1.0, 5.0, 50.0, 500.0, 5_000.0, 20_000.0];

    /// The exact admission predicate `list_businesses` applies (`routes.rs`, the `near` filter).
    /// Pulled out verbatim so the parity fixture records the real decision, NaN semantics included.
    fn geo_admits(d_km: f64, radius_km: f64) -> bool {
        d_km <= radius_km
    }

    #[test]
    #[ignore = "generator, not a check: prints the fixture for packages/ui/test/fixtures"]
    fn print_geo_parity_fixture() {
        let cases: Vec<Value> = GEO_PARITY_CASES
            .iter()
            .map(|(name, lat1, lon1, lat2, lon2)| {
                let d = haversine_km(*lat1, *lon1, *lat2, *lon2);
                json!({
                    "name": name,
                    "from": [lat1, lon1],
                    "to": [lat2, lon2],
                    // JSON cannot carry NaN; `null` means "the deprecated Rust returns NaN here".
                    "serverKm": if d.is_nan() { Value::Null } else { json!(d) },
                    "admits": GEO_PARITY_RADII.iter().map(|r| geo_admits(d, *r)).collect::<Vec<_>>(),
                })
            })
            .collect();
        let doc = json!({
            "_source": "generated by `cargo test -p admin-api -- --ignored --nocapture \
                        print_geo_parity_fixture` from stacks/admin/api/src/routes.rs::haversine_km",
            "_note": "serverKm: null means the deprecated Rust returns NaN for that pair. \
                      admits[i] is `haversine_km(..) <= radiiKm[i]` as list_businesses evaluates it.",
            "earthRadiusKm": 6371.0,
            "radiiKm": GEO_PARITY_RADII,
            "cases": cases,
        });
        println!("{}", serde_json::to_string_pretty(&doc).unwrap());
    }

    #[test]
    fn geo_parity_fixture_is_what_this_haversine_actually_produces() {
        let doc: Value = serde_json::from_str(GEO_PARITY_FIXTURE).expect("fixture parses");
        assert_eq!(
            doc["radiiKm"].as_array().unwrap().len(),
            GEO_PARITY_RADII.len(),
            "fixture radii drifted from GEO_PARITY_RADII"
        );
        for (i, r) in GEO_PARITY_RADII.iter().enumerate() {
            assert_eq!(doc["radiiKm"][i].as_f64().unwrap(), *r);
        }
        let cases = doc["cases"].as_array().expect("cases array");
        assert_eq!(
            cases.len(),
            GEO_PARITY_CASES.len(),
            "fixture case count drifted; regenerate with `cargo test -p admin-api -- --ignored \
             --nocapture print_geo_parity_fixture`"
        );
        for (case, (name, lat1, lon1, lat2, lon2)) in cases.iter().zip(GEO_PARITY_CASES) {
            assert_eq!(case["name"].as_str().unwrap(), *name);
            let d = haversine_km(*lat1, *lon1, *lat2, *lon2);
            match case["serverKm"].as_f64() {
                None => assert!(
                    d.is_nan(),
                    "{name}: fixture says NaN but haversine_km returned {d}"
                ),
                Some(want) => {
                    assert!(!d.is_nan(), "{name}: haversine_km returned NaN, fixture has {want}");
                    // Tight: the fixture came from this same function, so only JSON round-tripping
                    // is allowed to move it.
                    assert!(
                        (d - want).abs() <= 1e-9 * want.abs().max(1.0),
                        "{name}: haversine_km = {d}, fixture = {want}. Regenerate with \
                         `cargo test -p admin-api -- --ignored --nocapture print_geo_parity_fixture`"
                    );
                }
            }
            let admits = case["admits"].as_array().unwrap();
            for (i, r) in GEO_PARITY_RADII.iter().enumerate() {
                assert_eq!(
                    admits[i].as_bool().unwrap(),
                    geo_admits(d, *r),
                    "{name}: admit decision at radius {r} km drifted"
                );
            }
        }
    }

    #[test]
    fn the_deprecated_filter_silently_drops_a_near_antipodal_provider() {
        // The one place the server-side filter and the on-device replacement genuinely differ, and
        // it is a defect in THIS code rather than in the replacement.
        //
        // `haversine_km` ends in `asin(sqrt(a))`. For near-antipodal inputs `a` rounds two ulps
        // above 1.0 in f64 (measured: 1.0000000000000004), `sqrt` stays above 1, and `asin` of an
        // argument outside [-1, 1] is NaN. Rust's `NaN <= radius` is `false`, so the provider is
        // dropped from the results with no error raised anywhere - a "could not compute" rendered
        // as a definite "out of range", which is the inverse of the rule this codebase applies to
        // verification verdicts.
        //
        // Two measured facts about how narrow this is, both worth keeping so nobody re-derives them:
        //   * it needs `a` at least TWO ulps above 1.0. One ulp is not enough: `sqrt` rounds
        //     1.0000000000000002 back to exactly 1.0 and `asin(1.0)` is `PI/2`, not NaN.
        //   * roughly 1 in 12,000 uniformly-sampled near-antipodal pairs reaches it, and the exact
        //     set differs between Rust's libm and V8 because their `sin`/`cos` disagree in the last
        //     ulp. This pair was chosen from the intersection, so it reproduces in both.
        //
        // `packages/ui/src/geo/distance.ts` uses `atan2(sqrt(a), sqrt(max(0, 1-a)))`, which is
        // total, and returns the half-circumference (~20015 km) here. That is why the parity claim
        // is about INCLUSION SETS at a given radius rather than about equal distances: for every
        // radius below the half-circumference both drop this provider, so the sets agree.
        let (lat1, lon1, lat2, lon2) = (
            66.517_526_663_907_92,
            121.879_438_417_269_34,
            -66.517_526_664_399_88,
            -58.120_561_582_730_66,
        );
        let d = haversine_km(lat1, lon1, lat2, lon2);
        assert!(d.is_nan(), "expected the NaN path, got {d}");
        assert!(!geo_admits(d, 20_000.0), "NaN must not admit");
        assert!(!geo_admits(d, f64::MAX), "NaN does not admit at any radius");

        // The same inputs through the total form the TypeScript uses.
        let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
        let (dphi, dl) = ((lat2 - lat1).to_radians(), (lon2 - lon1).to_radians());
        let a = (dphi / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
        assert!(a > 1.0, "expected a to round above 1.0, got {a}");
        let total = 2.0 * 6371.0 * a.sqrt().atan2((1.0f64 - a).max(0.0).sqrt());
        assert!(
            (total - std::f64::consts::PI * 6371.0).abs() < 1e-6,
            "total form should give the half-circumference, got {total}"
        );
    }

    #[test]
    fn every_fixture_pair_agrees_on_admission_below_the_half_circumference() {
        // The parity claim, stated as a property rather than a table: for any radius under the
        // half-circumference, the deprecated `haversine_km <= radius` and the total `atan2` form
        // admit exactly the same pairs. Above it they can differ, and only in the NaN case above.
        let half_circumference = std::f64::consts::PI * 6371.0;
        for (name, lat1, lon1, lat2, lon2) in GEO_PARITY_CASES {
            let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
            let (dphi, dl) = ((lat2 - lat1).to_radians(), (lon2 - lon1).to_radians());
            let a = (dphi / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
            let total = 2.0 * 6371.0 * a.sqrt().atan2((1.0f64 - a).max(0.0).sqrt());
            let server = haversine_km(*lat1, *lon1, *lat2, *lon2);
            for r in GEO_PARITY_RADII {
                if *r >= half_circumference {
                    continue;
                }
                assert_eq!(
                    geo_admits(server, *r),
                    geo_admits(total, *r),
                    "{name}: admission at radius {r} km differs between the deprecated and total forms"
                );
            }
        }
    }
}
