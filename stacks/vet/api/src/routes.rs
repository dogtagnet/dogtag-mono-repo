//! Axum router + all HTTP handlers (impl §3.1/§3.3/§3.4/§3.5/§3.8/§3.9, §11.4/§11.6/§11.7e).
//!
//! Route map:
//!   public router (operator-session-gated except where noted):
//!     POST /admin/login                              -> admin session (custody gate)
//!     POST /login                                    -> operator session
//!     POST /credentials/prepare | /credentials/confirm
//!     GET|PUT /settings/signing-mode
//!     POST /records  (legacy backend-mode shortcut)
//!     POST /records/{id}/revoke
//!     POST /records/{id}/share
//!     GET  /records/{id}                              (record-JWT — UNAUTHENTICATED by session)
//!     GET  /r/{token}                                 (short one-time share token — UNAUTHENTICATED)
//!     GET  /issuer/signers
//!     POST /import/pull
//!     POST /verify/session/start | /verify/consent/submit
//!   admin router (custody — mounted SEPARATELY; /admin/* requires the admin session):
//!     POST /admin/genesis/start | /admin/genesis/confirm | /admin/unlock | /admin/accounts

use std::net::SocketAddr;

use axum::{
    body::Bytes,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{self, AppState};
use crate::auth::{self, ShareClaims, VerifyClaims};
use crate::store::{ApptReplica, IssuerSettings, Record, RecordStatus, VerifySession};

type Resp = (StatusCode, Json<Value>);

fn ok(v: Value) -> Resp {
    (StatusCode::OK, Json(v))
}
fn err(code: StatusCode, msg: &str) -> Resp {
    (code, Json(json!({ "error": msg })))
}

// --------------------------------------------------------------------------------------------
// auth helpers
// --------------------------------------------------------------------------------------------

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

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

/// Require a valid operator session bearer token.
async fn require_operator(st: &AppState, headers: &HeaderMap) -> Result<(), Resp> {
    let token = bearer(headers).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing operator session"))?;
    if st.store.has_op_session(&token).await {
        Ok(())
    } else {
        Err(err(StatusCode::UNAUTHORIZED, "invalid operator session"))
    }
}

/// Dual gate for the verify consent/status endpoints: authorize EITHER a valid operator session OR a
/// valid verify-session JWT bound to `session_id`. The JWT may arrive as `body_jwt` (request field)
/// or as the `Authorization: Bearer` header. When the JWT path is taken, its `relayer/purpose/
/// challenge` claims are checked against the stored `VerifySession`. If `consume` is set, the JWT's
/// `jti` is consumed once-only (replay protection) — used for the SUBMIT, not the (idempotent) status
/// read. Returns `Ok(true)` if authorized via a session JWT, `Ok(false)` if via operator session.
async fn require_operator_or_session_jwt(
    st: &AppState,
    headers: &HeaderMap,
    session_id: &str,
    body_jwt: Option<&str>,
    consume: bool,
) -> Result<bool, Resp> {
    // Operator session first (portal + scripts/e2e-smoke.sh): an op_ bearer satisfies the gate.
    if let Some(token) = bearer(headers) {
        if st.store.has_op_session(&token).await {
            return Ok(false);
        }
    }
    // Otherwise try a verify-session JWT (the owner's phone). Accept it from the body field or the
    // Bearer header.
    let jwt = body_jwt
        .map(|s| s.to_string())
        .or_else(|| bearer(headers))
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing operator session or session JWT"))?;
    let claims = auth::verify_session_jwt(&st.jwt, &jwt, &st.cfg.deployment_url, session_id)
        .map_err(|e| err(StatusCode::UNAUTHORIZED, &format!("session jwt: {e}")))?;
    // Bind the claims to the stored session (relayer/purpose/challenge).
    let session = st
        .store
        .get_session(session_id)
        .await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "session not found"))?;
    if !claims.relayer.eq_ignore_ascii_case(&session.relayer)
        || claims.purpose != session.purpose
        || claims.challenge != session.challenge
    {
        return Err(err(StatusCode::UNAUTHORIZED, "session jwt does not match session"));
    }
    if consume && !st.store.consume_jti(&claims.jti).await {
        return Err(err(StatusCode::UNAUTHORIZED, "session jwt already used"));
    }
    Ok(true)
}

/// Require a valid admin session bearer (custody gate). Same mechanism, distinct token prefix.
async fn require_admin(st: &AppState, headers: &HeaderMap) -> Result<(), Resp> {
    let token = bearer(headers).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing admin session"))?;
    if token.starts_with("admin_") && st.store.has_op_session(&token).await {
        Ok(())
    } else {
        Err(err(StatusCode::UNAUTHORIZED, "invalid admin session"))
    }
}

// --------------------------------------------------------------------------------------------
// login
// --------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginReq {
    password: String,
}

async fn login(
    State(st): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<LoginReq>,
) -> Resp {
    let ip = client_ip(&headers, peer.map(|ConnectInfo(p)| p));
    if st.ratelimit.is_locked(&ip) {
        return err(StatusCode::TOO_MANY_REQUESTS, "too many attempts; try again later");
    }
    if body.password != st.cfg.operator_password {
        st.ratelimit.record_failure(&ip);
        return err(StatusCode::UNAUTHORIZED, "bad password");
    }
    st.ratelimit.record_success(&ip);
    let token = auth::new_op_token();
    st.store.put_op_session(token.clone()).await;
    ok(json!({ "token": token }))
}

async fn admin_login(
    State(st): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<LoginReq>,
) -> Resp {
    let ip = client_ip(&headers, peer.map(|ConnectInfo(p)| p));
    if st.ratelimit.is_locked(&ip) {
        return err(StatusCode::TOO_MANY_REQUESTS, "too many attempts; try again later");
    }
    if body.password != st.cfg.admin_password {
        st.ratelimit.record_failure(&ip);
        return err(StatusCode::UNAUTHORIZED, "bad password");
    }
    st.ratelimit.record_success(&ip);
    let token = format!("admin_{}", auth::new_op_token());
    st.store.put_op_session(token.clone()).await;
    ok(json!({ "token": token }))
}

// --------------------------------------------------------------------------------------------
// /admin/* custody (impl §3.1 / §11.4)
// --------------------------------------------------------------------------------------------

async fn genesis_start(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    // 409 unless uninitialized.
    if st.store.get_custody().await.map(|c| c.meta.state == "initialized").unwrap_or(false) {
        return err(StatusCode::CONFLICT, "already initialized");
    }
    let stash = match crate::custody::genesis_generate() {
        Ok(s) => s,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let words = crate::custody::words_of(&stash.mnemonic);
    let challenge = stash.challenge_indices.clone();
    st.custody.stash_genesis(stash);
    ok(json!({ "words": words, "challengeIndices": challenge }))
}

#[derive(Deserialize)]
struct GenesisConfirmReq {
    /// the words the operator re-typed at the challenge indices, in challenge-index order.
    words: Vec<String>,
    passphrase: String,
}

async fn genesis_confirm(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GenesisConfirmReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let stash = match st.custody.take_stash() {
        Some(s) => s,
        None => return err(StatusCode::CONFLICT, "no pending genesis"),
    };
    let all = crate::custody::words_of(&stash.mnemonic);
    // verify the typed challenge words match.
    if body.words.len() != stash.challenge_indices.len() {
        return err(StatusCode::BAD_REQUEST, "wrong number of challenge words");
    }
    for (typed, &idx) in body.words.iter().zip(stash.challenge_indices.iter()) {
        if all.get(idx).map(|w| w == typed).unwrap_or(false) == false {
            return err(StatusCode::BAD_REQUEST, "challenge words do not match");
        }
    }
    let signer0 = match crate::custody::derive_account(&stash.mnemonic, 0) {
        Ok(s) => s,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let addr0 = format!("{:#x}", signer0.address());
    let ct = match crate::custody::encrypt_seed(&stash.mnemonic, &body.passphrase) {
        Ok(c) => c,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let mut blob = crate::store::CustodyBlob::default();
    blob.encrypted_seed = ct;
    blob.meta.state = "initialized".to_string();
    blob.meta.accounts.push(crate::store::AccountMeta {
        index: 0,
        address: addr0.clone(),
        label: "account0".to_string(),
    });
    st.store.put_custody(blob).await;
    st.custody.clear_stash();
    ok(json!({ "address": addr0 }))
}

#[derive(Deserialize)]
struct UnlockReq {
    passphrase: String,
}

async fn unlock(
    State(st): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<UnlockReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let ip = client_ip(&headers, peer.map(|ConnectInfo(p)| p));
    if st.ratelimit.is_locked(&ip) {
        return err(StatusCode::TOO_MANY_REQUESTS, "too many attempts; try again later");
    }
    let blob = match st.store.get_custody().await {
        Some(b) if b.meta.state == "initialized" => b,
        _ => return err(StatusCode::CONFLICT, "not initialized"),
    };
    let phrase = match crate::custody::decrypt_seed(&blob.encrypted_seed, &body.passphrase) {
        Ok(p) => p,
        Err(_) => {
            st.ratelimit.record_failure(&ip);
            return err(StatusCode::UNAUTHORIZED, "wrong passphrase");
        }
    };
    st.ratelimit.record_success(&ip);
    if let Err(e) = st.custody.unlock_with(phrase) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    // wire the unlocked backend signers into the chain client (so backend mode can broadcast).
    for a in &blob.meta.accounts {
        if let (Ok(pk), addr) = (st.custody.private_key(a.index), a.address.clone()) {
            st.chain.register_signer(a.index, pk, addr).await;
        }
    }
    let accounts: Vec<Value> = blob
        .meta
        .accounts
        .iter()
        .map(|a| json!({ "index": a.index, "address": a.address, "label": a.label }))
        .collect();
    ok(json!({ "unlocked": true, "accounts": accounts }))
}

#[derive(Deserialize)]
struct AccountsReq {
    label: String,
}

async fn accounts(State(st): State<AppState>, headers: HeaderMap, Json(body): Json<AccountsReq>) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    let mut blob = st.store.get_custody().await.unwrap_or_default();
    let next = blob.meta.accounts.iter().map(|a| a.index).max().map(|m| m + 1).unwrap_or(0);
    let signer = match st.custody.signer(next) {
        Ok(s) => s,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let addr = format!("{:#x}", signer.address());
    blob.meta.accounts.push(crate::store::AccountMeta {
        index: next,
        address: addr.clone(),
        label: body.label,
    });
    st.store.put_custody(blob).await;
    ok(json!({ "index": next, "address": addr }))
}

// --------------------------------------------------------------------------------------------
// settings (impl §3.8 / §11.7e)
// --------------------------------------------------------------------------------------------

async fn get_signing_mode(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let s = st.store.get_settings().await;
    ok(json!({ "signingMode": s.signing_mode }))
}

#[derive(Deserialize)]
struct SigningModeReq {
    mode: String,
}

async fn put_signing_mode(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SigningModeReq>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    if body.mode != "wallet" && body.mode != "backend" {
        return err(StatusCode::BAD_REQUEST, "mode must be wallet|backend");
    }
    // 409 if any prepared record outstanding (no mid-flight split — §11.7e / audit-06 §2.3).
    if st.store.has_prepared().await {
        return err(StatusCode::CONFLICT, "prepared record outstanding; cannot switch mode");
    }
    st.store.put_settings(IssuerSettings { signing_mode: body.mode.clone() }).await;
    ok(json!({ "signingMode": body.mode }))
}

// --------------------------------------------------------------------------------------------
// credentials prepare/confirm (impl §11.6 — CANONICAL hardened)
// --------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct PrepareReq {
    #[serde(rename = "recordType")]
    record_type: String,
    #[serde(rename = "dogTagId")]
    dog_tag_id: String,
    fields: Value,
}

async fn prepare(State(st): State<AppState>, headers: HeaderMap, Json(body): Json<PrepareReq>) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    let issuer_addr = match st.cfg.issuer_addr_for(&body.record_type) {
        Some(a) => a,
        None => return err(StatusCode::BAD_REQUEST, "unknown recordType / no issuer address"),
    };
    // build (ALWAYS server-side, identical both modes).
    let meta = app::issuer_meta(&st.cfg, &body.record_type, &issuer_addr);
    let vc = app::build_vc(&body.record_type, &body.fields, &body.dog_tag_id);
    let doc = match app::wrap(&body.record_type, meta, &vc) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::BAD_REQUEST, &e),
    };
    let root = doc.signature.merkle_root.clone();
    let target = doc.signature.target_hash.clone();
    let calldata = crate::chain::issue_calldata(&root);
    let record_id = uuid::Uuid::new_v4().to_string();

    let mut record = Record {
        record_id: record_id.clone(),
        record_type: body.record_type.clone(),
        dog_tag_id: body.dog_tag_id.clone(),
        wrapped_doc: serde_json::to_value(&doc).unwrap(),
        root: root.clone(),
        prepared_calldata: calldata.clone(),
        issuer_addr: issuer_addr.clone(),
        status: RecordStatus::Prepared,
        tx_hash: None,
        confirmed_tx_hash: None,
        signer_address: None,
        signing_mode: None,
    };
    st.store.put_record(record.clone()).await;

    let mode = st.store.get_settings().await.signing_mode;
    if mode == "wallet" {
        return ok(json!({
            "recordId": record_id,
            "merkleRoot": root,
            "targetHash": target,
            "proof": [],
            "unsignedTx": { "to": issuer_addr, "data": calldata, "value": 0, "chainId": st.chain.chain_id() }
        }));
    }

    // backend mode: preflight whitelist, sign+broadcast, then confirm via the SAME hardened path.
    let signer_addr = match st.custody.active_address() {
        Ok(a) => a,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let rt_key = app::rt_key(&body.record_type);
    match st
        .chain
        .is_whitelisted_for(&st.cfg.issuer_registry_addr, &rt_key, &signer_addr)
        .await
    {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::FORBIDDEN, "address not approved for this recordType yet"),
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("preflight: {e}")),
    }
    let sent = match st.chain.sign_and_send(0, &issuer_addr, &calldata).await {
        Ok(s) => s,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("broadcast: {e}")),
    };
    record.tx_hash = Some(sent.tx_hash.clone());
    st.store.update_record(record.clone()).await;

    // confirm (hardened, on-chain re-verify).
    match confirm_inner(&st, &record_id, &sent.tx_hash).await {
        Ok(_) => ok(json!({
            "recordId": record_id,
            "merkleRoot": root,
            "txHash": sent.tx_hash,
            "signerAddress": signer_addr,
            "mode": "backend"
        })),
        Err(e) => e,
    }
}

#[derive(Deserialize)]
struct ConfirmReq {
    #[serde(rename = "recordId")]
    record_id: String,
    #[serde(rename = "txHash")]
    tx_hash: String,
}

async fn confirm(State(st): State<AppState>, headers: HeaderMap, Json(body): Json<ConfirmReq>) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    match confirm_inner(&st, &body.record_id, &body.tx_hash).await {
        Ok(v) => ok(v),
        Err(e) => e,
    }
}

/// The hardened confirm (impl §11.6): derive signer FROM the tx; bind tx.to/input/value/chainId to the
/// prepared draft; require RootIssued(root,by) from the PINNED issuer with ev.root==r.root && ev.by==signer;
/// require issuedAt[root]!=0 at N confirmations; idempotent on txHash; flip prepared->issued.
async fn confirm_inner(st: &AppState, record_id: &str, tx_hash: &str) -> Result<Value, Resp> {
    // idempotency: already confirmed at this txHash -> return success.
    if let Some(r) = st.store.record_by_confirmed_tx(tx_hash).await {
        if r.record_id == record_id {
            return Ok(json!({ "recordId": record_id, "status": "issued" }));
        }
    }
    let mut r = st
        .store
        .get_record(record_id)
        .await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "record not found"))?;
    if r.status != RecordStatus::Prepared || r.confirmed_tx_hash.is_some() {
        return Err(err(StatusCode::CONFLICT, "record not in prepared state"));
    }
    // issuerAddr resolved ONLY from trusted config (audit-04 V2-H3).
    let issuer_addr = r.issuer_addr.clone();

    let view = st
        .chain
        .get_tx_view(tx_hash, &issuer_addr, st.cfg.confirmations)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, &format!("tx fetch: {e}")))?;

    if !view.success {
        return Err(err(StatusCode::BAD_REQUEST, "tx not successful"));
    }
    // bind to THIS prepared draft.
    if view.to.to_lowercase() != issuer_addr.to_lowercase() {
        return Err(err(StatusCode::BAD_REQUEST, "tx.to mismatch"));
    }
    if view.input.to_lowercase() != r.prepared_calldata.to_lowercase() {
        return Err(err(StatusCode::BAD_REQUEST, "tx.input mismatch (not this draft)"));
    }
    if !view.value.is_zero() {
        return Err(err(StatusCode::BAD_REQUEST, "tx.value != 0"));
    }
    if view.chain_id != Some(st.chain.chain_id()) {
        return Err(err(StatusCode::BAD_REQUEST, "tx.chainId mismatch (wrong chain)"));
    }
    // DERIVE signer from the tx (never the body).
    let signer = view.from.to_lowercase();
    // authorized at confirm time.
    let rt_key = app::rt_key(&r.record_type);
    match st
        .chain
        .is_whitelisted_for(&st.cfg.issuer_registry_addr, &rt_key, &signer)
        .await
    {
        Ok(true) => {}
        Ok(false) => return Err(err(StatusCode::FORBIDDEN, "signer not whitelisted at confirm")),
        Err(e) => return Err(err(StatusCode::BAD_GATEWAY, &format!("whitelist: {e}"))),
    }
    // RootIssued(root,by) from the pinned issuer; ev.root==r.root && ev.by==signer.
    let matched = view
        .root_issued_logs
        .iter()
        .any(|(root, by)| root.to_lowercase() == r.root.to_lowercase() && by.to_lowercase() == signer);
    if !matched {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "RootIssued(root,by) not found / mismatch on pinned issuer",
        ));
    }
    // issuedAt[root] != 0 at N confirmations.
    match st.chain.issued_at(&issuer_addr, &r.root).await {
        Ok(v) if !v.is_zero() => {}
        Ok(_) => return Err(err(StatusCode::BAD_REQUEST, "issuedAt[root] == 0")),
        Err(e) => return Err(err(StatusCode::BAD_GATEWAY, &format!("issuedAt: {e}"))),
    }

    r.status = RecordStatus::Issued;
    r.confirmed_tx_hash = Some(tx_hash.to_string());
    r.tx_hash = Some(tx_hash.to_string());
    r.signer_address = Some(signer.clone());
    r.signing_mode = Some(st.store.get_settings().await.signing_mode);
    st.store.update_record(r).await;
    Ok(json!({ "recordId": record_id, "status": "issued" }))
}

// --------------------------------------------------------------------------------------------
// records: legacy issue, revoke, share, get (impl §3.3 / §3.4)
// --------------------------------------------------------------------------------------------

async fn revoke(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    let mut r = match st.store.get_record(&id).await {
        Some(r) => r,
        None => return err(StatusCode::NOT_FOUND, "record not found"),
    };
    if r.status != RecordStatus::Issued {
        return err(StatusCode::CONFLICT, "record not issued");
    }
    let calldata = crate::chain::revoke_calldata(&r.root);
    let sent = match st.chain.sign_and_send(0, &r.issuer_addr, &calldata).await {
        Ok(s) => s,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("revoke broadcast: {e}")),
    };
    r.status = RecordStatus::Revoked;
    st.store.update_record(r).await;
    ok(json!({ "recordId": id, "status": "revoked", "txHash": sent.tx_hash }))
}

async fn share(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    if st.store.get_record(&id).await.is_none() {
        return err(StatusCode::NOT_FOUND, "record not found");
    }
    // Mint a SHORT one-time token (32 hex chars == 16 random bytes) so the QR is low-density and
    // easy for a phone camera to focus on. The server maps token -> record (one-time, deleted on
    // first GET /r/:token), expiring after 180s — the same one-time-use guarantee as the old
    // embedded record-JWT, but with a tiny payload.
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    let token = hex::encode(bytes);
    let exp = auth::now() + 180;
    st.store.put_share_token(&token, &id, exp).await;
    let qr = format!("{}/r/{}", st.cfg.deployment_url, token);
    ok(json!({ "qrUrl": qr, "recordId": id }))
}

/// GET /r/:token — resolve a SHORT one-time share token to the record's wrapped doc. Unauthenticated
/// (like the legacy record-JWT GET). The token is CONSUMED (deleted) on first read — a second read is
/// a 404. An expired token is also a 404/410.
async fn get_shared(State(st): State<AppState>, Path(token): Path<String>) -> Resp {
    let record_id = match st.store.take_share_token(&token).await {
        Some(id) => id,
        None => return err(StatusCode::NOT_FOUND, "share token missing or expired"),
    };
    match st.store.get_record(&record_id).await {
        Some(r) => ok(r.wrapped_doc),
        None => err(StatusCode::NOT_FOUND, "record not found"),
    }
}

/// GET /records/{id} — record-JWT bearer; UNAUTHENTICATED by operator session (§11.7e).
async fn get_record(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Resp {
    let token = match bearer(&headers) {
        Some(t) => t,
        None => return err(StatusCode::UNAUTHORIZED, "missing record JWT"),
    };
    let claims: ShareClaims = match auth::verify_jwt(&st.jwt, &token, 30) {
        Ok(c) => c,
        Err(e) => return err(StatusCode::UNAUTHORIZED, &format!("jwt: {e}")),
    };
    if claims.sub != id || claims.scope != "read:record" {
        return err(StatusCode::UNAUTHORIZED, "claim mismatch");
    }
    // consume jti atomically — 401 if reused.
    if !st.store.consume_jti(&claims.jti).await {
        return err(StatusCode::UNAUTHORIZED, "jti already used");
    }
    match st.store.get_record(&id).await {
        Some(r) => ok(r.wrapped_doc),
        None => err(StatusCode::NOT_FOUND, "record not found"),
    }
}

// --------------------------------------------------------------------------------------------
// issuer signers (impl §3.8)
// --------------------------------------------------------------------------------------------

async fn issuer_signers(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    if !st.custody.is_unlocked() {
        return ok(json!({ "signers": [] }));
    }
    let active = st.custody.active_address().unwrap_or_default();
    // whitelist matrix across the configured record types.
    let mut matrix = Vec::new();
    for rt in st.cfg.issuer_addrs.keys() {
        let key = app::rt_key(rt);
        let wl = st
            .chain
            .is_whitelisted_for(&st.cfg.issuer_registry_addr, &key, &active)
            .await
            .unwrap_or(false);
        matrix.push(json!({ "recordType": rt, "address": active, "whitelisted": wl }));
    }
    ok(json!({ "activeSigner": active, "matrix": matrix }))
}

// --------------------------------------------------------------------------------------------
// import/pull (impl §3.5) — DECOUPLED from /verify
// --------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct ImportPullReq {
    #[serde(rename = "userApiBase")]
    user_api_base: String,
    #[serde(rename = "userJwt")]
    user_jwt: String,
    #[serde(rename = "recordRef")]
    record_ref: String,
}

async fn import_pull(State(st): State<AppState>, headers: HeaderMap, Json(body): Json<ImportPullReq>) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let url = format!("{}/share/{}", body.user_api_base.trim_end_matches('/'), body.record_ref);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .bearer_auth(&body.user_jwt)
        .send()
        .await;
    let doc_val: Value = match resp {
        Ok(r) if r.status().is_success() => match r.json().await {
            Ok(v) => v,
            Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("bad doc json: {e}")),
        },
        Ok(r) => return err(StatusCode::BAD_GATEWAY, &format!("fetch failed: {}", r.status())),
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("fetch error: {e}")),
    };
    // third-party verify via the SDK.
    let doc: dogtag_standard::wrap::WrappedDoc = match serde_json::from_value(doc_val.clone()) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::BAD_REQUEST, &format!("not a WrappedDoc: {e}")),
    };
    let verdict = crate::verify::third_party_verify(&st, &doc).await;
    if !verdict.valid {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "third-party verify invalid");
    }
    // upsert client cache keyed by dogTagId.
    let dog = crate::verify::dog_tag_id_of(&doc).unwrap_or_else(|| "unknown".to_string());
    st.store.upsert_client_cache(dog, doc_val).await;
    ok(json!({ "imported": true, "verdict": crate::verify::verdict_json(&verdict) }))
}

// --------------------------------------------------------------------------------------------
// verify (impl §3.9)
// --------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct SessionStartReq {
    purpose: String,
    #[serde(rename = "recordType")]
    record_type: String,
    #[serde(default)]
    mode: Option<String>,
}

async fn verify_session_start(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SessionStartReq>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    let relayer = match st.custody.active_address() {
        Ok(a) => a,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    // whitelistedFor(keccak256("VERIFY:"||purpose), relayer)
    let verify_key = crate::verify::verify_key(&body.purpose);
    match st
        .chain
        .is_whitelisted_for(&st.cfg.issuer_registry_addr, &verify_key, &relayer)
        .await
    {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::FORBIDDEN, "relayer not whitelisted for this purpose"),
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("verify-wl: {e}")),
    }
    let mode = body.mode.clone().unwrap_or_else(|| "zk".to_string());
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut challenge = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut challenge);
    let challenge_hex = format!("0x{}", hex::encode(challenge));
    let n = auth::now();
    let claims = VerifyClaims {
        iss: st.cfg.deployment_url.clone(),
        sub: session_id.clone(),
        aud: "dogtag-mobile".to_string(),
        relayer: relayer.clone(),
        purpose: body.purpose.clone(),
        record_type: body.record_type.clone(),
        challenge: challenge_hex.clone(),
        mode: mode.clone(),
        exp: n + 180,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    let token = auth::sign_jwt(&st.jwt, &claims);
    st.store
        .put_session(VerifySession {
            session_id: session_id.clone(),
            relayer,
            purpose: body.purpose,
            record_type: body.record_type,
            mode,
            challenge: challenge_hex,
            status: "pending".to_string(),
            tx_hash: None,
            nullifier: None,
        })
        .await;
    let qr = format!("{}/v?t={}", st.cfg.deployment_url, token);
    ok(json!({ "qrUrl": qr, "sessionId": session_id }))
}

#[derive(Deserialize)]
struct ConsentSubmitReq {
    #[serde(rename = "sessionId")]
    session_id: String,
    consent: Value,
    sig: String,
    #[serde(default)]
    mode: Option<String>,
    /// the disclosed wrapped doc (normal path third-party verify input).
    #[serde(rename = "disclosedDoc", default)]
    disclosed_doc: Option<Value>,
    /// the client-supplied Groth16 proof `{a,b,c,pubSignals}` (on-device ZK path). When present the
    /// backend SKIPS server-prove and broadcasts these values as the relayer.
    #[serde(default)]
    proof: Option<Value>,
    /// OPTIONAL relayer-sponsored consent-key bind authorization `{ subject, keyHash, ownerSig }`.
    /// When the ZK path needs keyOf(subject)==keyHash and the key isn't bound yet, the backend
    /// broadcasts `bindConsentKeyFor` from the relayer signer using the owner's EIP-712 `ownerSig`
    /// (gasless for the owner). Ignored when the key is already bound.
    #[serde(default)]
    bind: Option<Value>,
    /// the verify-session JWT (phone auth). May also arrive as a `Bearer` token; either is accepted.
    #[serde(rename = "sessionJwt", default)]
    session_jwt: Option<String>,
}

async fn verify_consent_submit(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ConsentSubmitReq>,
) -> Resp {
    // Dual gate: operator session OR a verify-session JWT (the owner's phone). The JWT's jti is
    // consumed once-only here so a captured submit can't be replayed.
    if let Err(e) = require_operator_or_session_jwt(
        &st,
        &headers,
        &body.session_id,
        body.session_jwt.as_deref(),
        true,
    )
    .await
    {
        return e;
    }
    crate::verify::consent_submit(
        &st,
        body.session_id,
        body.consent,
        body.sig,
        body.mode,
        body.disclosed_doc,
        body.proof,
        body.bind,
    )
    .await
}

/// GET /verify/session/{sessionId} — operator-gated status read so the portal's VerifyFlow can poll
/// pending -> recorded. Returns the stored session's status/mode and (once recorded) the txHash +
/// nullifier. `nullifier` is exposed when present in the session row (ZK path); null otherwise.
async fn verify_session_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Resp {
    // Dual gate: operator session OR a verify-session JWT (the owner's phone polling). No jti consume
    // — status reads are idempotent and polled repeatedly. The JWT arrives via the Bearer header.
    if let Err(e) = require_operator_or_session_jwt(&st, &headers, &session_id, None, false).await {
        return e;
    }
    let s = match st.store.get_session(&session_id).await {
        Some(s) => s,
        None => return err(StatusCode::NOT_FOUND, "session not found"),
    };
    ok(json!({
        "status": s.status,
        "mode": s.mode,
        "txHash": s.tx_hash,
        "nullifier": s.nullifier,
    }))
}

// --------------------------------------------------------------------------------------------
// Google Calendar sync (impl §3.6 / §8.1) — operator-session gated.
// --------------------------------------------------------------------------------------------

/// GET /calendar/google/connect -> the OAuth 2.0 consent URL (access_type=offline + prompt=consent,
/// scope calendar.events). Operator-session gated.
async fn google_connect(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    // CSRF state ties the callback back to this deployment.
    let state = uuid::Uuid::new_v4().to_string();
    let url = st.calendar.consent_url(&state);
    ok(json!({ "consentUrl": url, "state": state }))
}

#[derive(Deserialize)]
struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// GET /calendar/google/callback?code= -> exchange the code for tokens; store the refresh token via
/// the Store (opaque/encrypted at rest). Operator-session gated.
async fn google_callback(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let code = match q.code {
        Some(c) if !c.is_empty() => c,
        _ => return err(StatusCode::BAD_REQUEST, "missing code"),
    };
    match st.calendar.exchange_code(&code).await {
        Ok(refresh_token) => {
            st.store_refresh_token(refresh_token).await;
            // best-effort: stand up the watch channel on first connect.
            let _ = crate::sync::renew_watch_if_due(&st, auth::now()).await;
            ok(json!({ "connected": true, "state": q.state }))
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, &format!("token exchange: {e}")),
    }
}

/// POST /calendar/sync -> run an incremental sync pass (410 -> wipe + full resync). Operator gated.
async fn calendar_sync(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let r = crate::sync::run_sync(&st).await;
    ok(json!({
        "echoesSkipped": r.echoes_skipped,
        "busyBlocks": r.busy_blocks,
        "humanEdits": r.human_edits,
        "reconciled": r.reconciled,
        "fullResync": r.full_resync,
    }))
}

// --------------------------------------------------------------------------------------------
// Appointment replica — business side (impl §3.7 / §8.3). Inbound from central: HMAC + Idempotency.
// --------------------------------------------------------------------------------------------

/// Verify the inbound cross-backend HMAC (METHOD\nPATH\nBODY) with the shared central secret, and the
/// Idempotency-Key (replay-dedup). Returns the parsed body on success, or an error Resp.
async fn verify_central_inbound(
    st: &AppState,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    raw: &Bytes,
) -> Result<Value, Resp> {
    let sig = headers
        .get("X-DogTag-HMAC")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing HMAC"))?;
    if !auth::hmac_verify(&st.cfg.central_hmac_secret, method, path, raw, sig) {
        return Err(err(StatusCode::UNAUTHORIZED, "bad HMAC"));
    }
    // Idempotency-Key: dedupe replays (atomic record).
    if let Some(key) = headers.get("Idempotency-Key").and_then(|h| h.to_str().ok()) {
        if !st.store.record_idempotency_key(key).await {
            // already processed: idempotent noop with the current replica state.
            return Err(idempotent_replay(st, raw).await);
        }
    }
    serde_json::from_slice(raw).map_err(|e| err(StatusCode::BAD_REQUEST, &format!("bad json: {e}")))
}

/// Build a 200 idempotent-replay response from the stored replica (Idempotency-Key already seen).
async fn idempotent_replay(st: &AppState, raw: &Bytes) -> Resp {
    let id = serde_json::from_slice::<Value>(raw)
        .ok()
        .and_then(|v| v.get("id").and_then(|x| x.as_str()).map(|s| s.to_string()));
    if let Some(id) = id {
        if let Some(a) = st.store.get_appt(&id).await {
            return ok(appt_json(&a));
        }
    }
    ok(json!({ "idempotent": true }))
}

fn appt_json(a: &ApptReplica) -> Value {
    json!({
        "id": a.appointment_id, "businessId": a.business_id, "dogTagId": a.dog_tag_id,
        "slot": a.slot, "rev": a.rev, "state": a.state, "updatedAt": a.updated_at,
    })
}

fn is_terminal(state: &str) -> bool {
    matches!(state, "DECLINED" | "CANCELLED" | "COMPLETED" | "NO_SHOW")
}

/// Core idempotent upsert keyed by appointmentId + central-assigned rev. Apply-if-rev-newer; a
/// strictly-older rev is `409 stale_rev`; terminal states win over a later CONFIRMED with older rev.
async fn upsert_replica(st: &AppState, incoming: ApptReplica) -> Resp {
    if let Some(existing) = st.store.get_appt(&incoming.appointment_id).await {
        // apply-if-newer: an OLDER rev is stale.
        if incoming.rev < existing.rev {
            return err(StatusCode::CONFLICT, "stale_rev");
        }
        // same rev -> idempotent noop.
        if incoming.rev == existing.rev {
            return ok(appt_json(&existing));
        }
        // terminal wins: never move OUT of a terminal state even if a newer rev arrives.
        if is_terminal(&existing.state) && !is_terminal(&incoming.state) {
            return ok(appt_json(&existing));
        }
    }
    st.store.put_appt(incoming.clone()).await;
    // mirror the platform appointment to Google (tagged + store etag for echo recognition).
    crate::sync::mirror_to_google(st, &incoming).await;
    ok(appt_json(&incoming))
}

/// PUT /v1/appointments/{id} — from central; Idempotency-Key + HMAC verify; idempotent replica upsert.
async fn put_appointment(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Resp {
    let path = format!("/v1/appointments/{id}");
    let body = match verify_central_inbound(&st, "PUT", &path, &headers, &raw).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let now = auth::now();
    let mut incoming = match crate::sync::replica_from_json(&body, now) {
        Some(a) => a,
        None => return err(StatusCode::BAD_REQUEST, "malformed appointment body"),
    };
    // path id is authoritative.
    incoming.appointment_id = id;
    upsert_replica(&st, incoming).await
}

/// POST /v1/appointments/{id}/cancel — terminal transition from central (terminal wins).
async fn cancel_appointment(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Resp {
    let path = format!("/v1/appointments/{id}/cancel");
    let body = match verify_central_inbound(&st, "POST", &path, &headers, &raw).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    apply_central_transition(&st, &id, &body, "CANCELLED").await
}

/// POST /v1/appointments/{id}/reschedule — slot change at a newer rev from central.
async fn reschedule_appointment(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Resp {
    let path = format!("/v1/appointments/{id}/reschedule");
    let body = match verify_central_inbound(&st, "POST", &path, &headers, &raw).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    apply_central_transition(&st, &id, &body, "REQUESTED").await
}

/// Apply a central-driven transition (cancel/reschedule) carrying a `rev` + optional `slot`/`state`.
async fn apply_central_transition(st: &AppState, id: &str, body: &Value, default_state: &str) -> Resp {
    let rev = match body.get("rev").and_then(|v| v.as_u64()) {
        Some(r) => r,
        None => return err(StatusCode::BAD_REQUEST, "rev required"),
    };
    let now = auth::now();
    let existing = st.store.get_appt(id).await;
    let (business_id, dog_tag_id, slot, state) = match &existing {
        Some(e) => (
            e.business_id.clone(),
            e.dog_tag_id.clone(),
            body.get("slot").and_then(|v| v.as_str()).unwrap_or(&e.slot).to_string(),
            body.get("state").and_then(|v| v.as_str()).unwrap_or(default_state).to_string(),
        ),
        None => (
            body.get("businessId").and_then(|v| v.as_str()).unwrap_or(&st.cfg.business_id).to_string(),
            body.get("dogTagId").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            body.get("slot").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            body.get("state").and_then(|v| v.as_str()).unwrap_or(default_state).to_string(),
        ),
    };
    let incoming = ApptReplica {
        appointment_id: id.to_string(),
        business_id,
        dog_tag_id,
        slot,
        rev,
        state,
        updated_at: now,
    };
    upsert_replica(st, incoming).await
}

#[derive(Deserialize)]
struct ApptListQuery {
    #[serde(rename = "updatedSince", default)]
    updated_since: Option<u64>,
}

/// GET /v1/appointments?updatedSince= — catch-up pull (operator gated).
async fn list_appointments(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ApptListQuery>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let since = q.updated_since.unwrap_or(0);
    let appts: Vec<Value> = st.store.appts_updated_since(since).await.iter().map(appt_json).collect();
    ok(json!({ "appointments": appts }))
}

#[derive(Deserialize)]
struct StaffActionReq {
    event: String, // CONFIRMED | DECLINED | COMPLETED | NO_SHOW
}

/// POST /v1/appointments/{id}/staff-action — a business-driven transition. The business NEVER assigns
/// rev; it POSTs {appointmentId, lastRev, event, occurredAt} to central (HMAC-signed) and applies the
/// central-allocated rev back to the replica. Operator-session gated.
async fn staff_action(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<StaffActionReq>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let appt = match st.store.get_appt(&id).await {
        Some(a) => a,
        None => return err(StatusCode::NOT_FOUND, "appointment not found"),
    };
    // ownership binding (C-2): this business may only drive transitions on ITS OWN appointments.
    if appt.business_id != st.cfg.business_id {
        return err(StatusCode::FORBIDDEN, "appointment not owned by business");
    }
    // validate the event is an allowed business-driven transition.
    if !matches!(body.event.as_str(), "CONFIRMED" | "DECLINED" | "COMPLETED" | "NO_SHOW") {
        return err(StatusCode::BAD_REQUEST, "invalid staff event");
    }
    let now = auth::now();
    // POST to central; central is the SOLE rev allocator -> we send lastRev, receive the new rev.
    match st
        .central
        .post_appointment_event(&appt.business_id, &id, appt.rev, &body.event, now)
        .await
    {
        Ok(ack) => {
            // apply the central-allocated rev + (terminal-aware) state back to the replica.
            let mut updated = appt.clone();
            updated.rev = ack.rev;
            updated.state = ack.state;
            updated.updated_at = now;
            st.store.put_appt(updated.clone()).await;
            crate::sync::mirror_to_google(&st, &updated).await;
            ok(appt_json(&updated))
        }
        Err(crate::calendar::CentralError::Status(403)) => {
            err(StatusCode::FORBIDDEN, "appointment not owned by business")
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, &format!("central callback: {e}")),
    }
}

// --------------------------------------------------------------------------------------------
// router assembly
// --------------------------------------------------------------------------------------------

/// The `/admin/*` custody routes (admin-session/loopback isolated). Mounted on the public listener
/// by default; when `ADMIN_LOOPBACK_ONLY` is set, served on a separate 127.0.0.1 listener instead.
pub fn admin_router(state: AppState) -> Router {
    Router::new()
        .route("/admin/login", post(admin_login))
        // admin custody (admin-session gated inside handlers)
        .route("/admin/genesis/start", post(genesis_start))
        .route("/admin/genesis/confirm", post(genesis_confirm))
        .route("/admin/unlock", post(unlock))
        .route("/admin/accounts", post(accounts))
        .with_state(state)
}

/// The public (non-admin) routes. Always mounted on the public `0.0.0.0:PORT` listener.
pub fn public_router(state: AppState) -> Router {
    Router::new()
        // health (no auth) — used by compose healthchecks
        .route("/health", get(health))
        // login
        .route("/login", post(login))
        // settings
        .route("/settings/signing-mode", get(get_signing_mode).put(put_signing_mode))
        // credentials
        .route("/credentials/prepare", post(prepare))
        .route("/credentials/confirm", post(confirm))
        // records
        .route("/records/:id/revoke", post(revoke))
        .route("/records/:id/share", post(share))
        .route("/records/:id", get(get_record))
        // short one-time share token resolver (unauthenticated; consumed on first read)
        .route("/r/:token", get(get_shared))
        // issuer signers
        .route("/issuer/signers", get(issuer_signers))
        // import
        .route("/import/pull", post(import_pull))
        // verify
        .route("/verify/session/start", post(verify_session_start))
        .route("/verify/session/:id", get(verify_session_status))
        .route("/verify/consent/submit", post(verify_consent_submit))
        // alias so the owner's phone can POST consent+proof directly to the groomer host.
        .route("/v1/verify/consent", post(verify_consent_submit))
        // calendar sync (Phase 7, §3.6)
        .route("/calendar/google/connect", get(google_connect))
        .route("/calendar/google/callback", get(google_callback))
        .route("/calendar/sync", post(calendar_sync))
        // appointment replica (Phase 7, §3.7) — inbound from central (HMAC) + business-driven actions
        .route("/v1/appointments/:id", put(put_appointment))
        .route("/v1/appointments/:id/cancel", post(cancel_appointment))
        .route("/v1/appointments/:id/reschedule", post(reschedule_appointment))
        .route("/v1/appointments/:id/staff-action", post(staff_action))
        .route("/v1/appointments", get(list_appointments))
        .with_state(state)
}

/// The single combined router (public + admin) on one listener — the default (demo/local) topology.
/// Admin routes carry their own admin-session gate. When `ADMIN_LOOPBACK_ONLY` is set, `main.rs`
/// serves `public_router` and `admin_router` on separate listeners instead of calling this.
pub fn router(state: AppState) -> Router {
    public_router(state.clone()).merge(admin_router(state))
}
