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
//!     GET  /r/{token}                                 (short one-time share/IMPORT token — UNAUTHENTICATED)
//!     GET  /x/{token}                                 (short one-time EXPORT token — UNAUTHENTICATED)
//!     GET  /issuer/signers
//!     POST /import/pull
//!     POST /verify/session/start | /verify/consent/submit   (EXPORT flow; route PATHS kept stable)
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
use crate::auth::{self, ShareClaims};
use crate::store::{ApptReplica, IssuerSettings, Record, RecordStatus, VerifySession};

type Resp = (StatusCode, Json<Value>);

fn ok(v: Value) -> Resp {
    (StatusCode::OK, Json(v))
}
fn err(code: StatusCode, msg: &str) -> Resp {
    // stderr is captured to .demo/<svc>.log (demo-up redirects 2>&1) so failed requests surface the
    // exact reason during the live demo, even without RUST_LOG.
    eprintln!("[err {code}] {msg}");
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

/// Dual gate for the verify/export consent/status endpoints: authorize EITHER a valid operator
/// session OR a valid one-time EXPORT TOKEN bound to `session_id`. The export token is the low-density
/// QR token (16 random bytes hex) minted at session start — symmetric with the import `/r/<token>`
/// flow (no EdDSA JWT on the export path). The token may arrive as `export_token` (request field) or
/// as the `?token=` query / `Authorization: Bearer` value. It is validated to map to `session_id`. If
/// `consume` is set, the token is CONSUMED once-only (replay protection) — used for the SUBMIT, not
/// the (idempotent, repeatedly-polled) status read which only PEEKs. Returns `Ok(true)` if authorized
/// via an export token, `Ok(false)` if via operator session.
async fn require_operator_or_export_token(
    st: &AppState,
    headers: &HeaderMap,
    session_id: &str,
    body_token: Option<&str>,
    consume: bool,
) -> Result<bool, Resp> {
    // Operator session first (portal + scripts/e2e-smoke.sh): an op_ bearer satisfies the gate.
    if let Some(token) = bearer(headers) {
        if st.store.has_op_session(&token).await {
            return Ok(false);
        }
    }
    // Otherwise try a one-time export token (the owner's phone). Accept it from the body field or the
    // Bearer header.
    let token = body_token
        .map(|s| s.to_string())
        .or_else(|| bearer(headers))
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing operator session or export token"))?;
    // SUBMIT consumes the token once-only; the status read peeks without consuming.
    let mapped = if consume {
        st.store.take_export_token(&token).await
    } else {
        st.store.peek_export_token(&token).await
    };
    let mapped = mapped.ok_or_else(|| err(StatusCode::UNAUTHORIZED, "export token missing, expired or already used"))?;
    if mapped != session_id {
        return Err(err(StatusCode::UNAUTHORIZED, "export token does not match session"));
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
    // Report custody state so the portal routes correctly after a restart: an already-initialized
    // custody (seal hydrated from disk) must go to Unlock, NOT re-genesis.
    let initialized = st
        .store
        .get_custody()
        .await
        .map(|c| c.meta.state == "initialized")
        .unwrap_or(false);
    let unlocked = st.custody.is_unlocked();
    ok(json!({ "token": token, "initialized": initialized, "unlocked": unlocked }))
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
    st.store.put_custody(blob.clone()).await;
    // ALSO persist the seal to disk (if configured) so the signer survives a backend restart. We
    // write ONLY the ciphertext + non-secret meta (atomic temp+rename, 0600). A write failure here
    // is fatal to the request: the operator must know the seal is NOT durable before they navigate
    // away (otherwise a restart silently loses the just-genesised seed).
    if let Some(path) = st.cfg.custody_seal_path.as_deref() {
        if let Err(e) = crate::custody::write_seal_file(path, &blob.encrypted_seed, &blob.meta) {
            return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("persist seal: {e}"));
        }
    }
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

async fn export_session_start(
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
    st.store
        .put_session(VerifySession {
            session_id: session_id.clone(),
            relayer: relayer.clone(),
            purpose: body.purpose,
            record_type: body.record_type,
            mode,
            challenge: challenge_hex,
            status: "pending".to_string(),
            tx_hash: None,
            nullifier: None,
        })
        .await;
    // Mint a SHORT one-time EXPORT token (32 hex chars == 16 random bytes) so the QR is low-density
    // and symmetric with the import `/r/<token>` flow. The server maps token -> export session
    // (one-time, consumed on consent submit), expiring after 180s. The QR carries {host, token,
    // groomer wallet address (relayer)} — the phone resolves session metadata via GET /x/<token>.
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    let token = hex::encode(bytes);
    // 10-minute TTL: the export path includes a slow on-device Groth16 proof (tens of seconds) plus the
    // owner's manual steps, so a 180s token can expire mid-flow. Still one-time (consumed on a
    // successful record); the on-chain nullifier independently prevents replay.
    let exp = auth::now() + 600;
    st.store.put_export_token(&token, &session_id, exp).await;
    let qr = format!("{}/x/{}?a={}", st.cfg.deployment_url, token, relayer);
    ok(json!({ "qrUrl": qr, "sessionId": session_id }))
}

/// GET /x/{token} — resolve a SHORT one-time EXPORT token to the export session metadata the phone
/// needs ({ sessionId, relayer, purpose, recordType, challenge, mode }). Unauthenticated and
/// NON-consuming — the token is consumed only on consent submit, not here. A missing/expired token is
/// a 404. Symmetric with `GET /r/{token}` on the import side.
async fn export_session_resolve(State(st): State<AppState>, Path(token): Path<String>) -> Resp {
    let session_id = match st.store.peek_export_token(&token).await {
        Some(id) => id,
        None => return err(StatusCode::NOT_FOUND, "export token missing or expired"),
    };
    match st.store.get_session(&session_id).await {
        Some(s) => ok(json!({
            "sessionId": s.session_id,
            "relayer": s.relayer,
            "purpose": s.purpose,
            "recordType": s.record_type,
            "challenge": s.challenge,
            "mode": s.mode,
        })),
        None => err(StatusCode::NOT_FOUND, "session not found"),
    }
}

#[derive(Deserialize)]
struct ConsentSubmitReq {
    /// OPTIONAL: the phone authenticates with the one-time `exportToken`, which already maps to a
    /// session, so it need not echo the sessionId. The operator portal still sends it. Resolved from
    /// the token when absent.
    #[serde(rename = "sessionId", default)]
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
    /// the one-time EXPORT token (phone auth). May also arrive as a `Bearer` token; either is
    /// accepted. Consumed once-only on submit (replay protection) — symmetric with import `/r/<token>`.
    #[serde(rename = "exportToken", default)]
    export_token: Option<String>,
}

async fn verify_consent_submit(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ConsentSubmitReq>,
) -> Resp {
    // The phone authenticates with the one-time export token, which already maps to a session — so
    // the sessionId in the body is optional; resolve it from the token (peek, non-consuming) when the
    // phone omits it. The operator portal sends sessionId directly.
    let session_id = if body.session_id.is_empty() {
        match body.export_token.as_deref() {
            Some(t) => st.store.peek_export_token(t).await.unwrap_or_default(),
            None => String::new(),
        }
    } else {
        body.session_id.clone()
    };
    // Dual gate: operator session OR a one-time export token (the owner's phone). PEEK the token
    // (consume=false) so a FAILED verification does not burn the owner's one-time token — they can
    // retry with the same QR. The token is consumed ONLY when the on-chain record SUCCEEDS — and that
    // record is now ASYNC (it outlives the phone's 8s submit timeout), so the consume happens in the
    // background task inside `consent_submit`, NOT here. We pass the token in and the task owns the
    // consume-on-record-success (the on-chain nullifier independently prevents a recorded
    // verification from being replayed).
    if let Err(e) = require_operator_or_export_token(
        &st,
        &headers,
        &session_id,
        body.export_token.as_deref(),
        false,
    )
    .await
    {
        return e;
    }
    crate::verify::consent_submit(
        &st,
        session_id,
        body.consent,
        body.sig,
        body.mode,
        body.disclosed_doc,
        body.proof,
        body.bind,
        body.export_token,
    )
    .await
}

#[derive(Deserialize)]
struct SessionStatusQuery {
    /// the one-time export token (the owner's phone polling) — non-consuming peek. The operator
    /// portal omits this and relies on the Bearer operator-session instead.
    #[serde(default)]
    token: Option<String>,
}

/// GET /verify/session/{sessionId} — operator-gated status read so the portal's VerifyFlow can poll
/// pending -> recorded. Returns the stored session's status/mode and (once recorded) the txHash +
/// nullifier. `nullifier` is exposed when present in the session row (ZK path); null otherwise.
async fn verify_session_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(q): Query<SessionStatusQuery>,
) -> Resp {
    // Dual gate: operator session OR a one-time export token (the owner's phone polling). No consume
    // — status reads are idempotent and polled repeatedly (peek only). The token arrives via the
    // `?token=` query or the Bearer header.
    if let Err(e) =
        require_operator_or_export_token(&st, &headers, &session_id, q.token.as_deref(), false).await
    {
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
// SERVER-SIDE PROVING API (Workstream A — 32-bit Android fallback). Gated behind the `prover`
// feature; the route is mounted only when compiled with `--features prover`.
// --------------------------------------------------------------------------------------------

/// `POST /prove-verification` body — the SAME inputs the on-device
/// `dogtag_standard::prover_ffi::prove_verification` takes: the stored wrapped doc, the §1.10 consent
/// JSON, and the EdDSA-BabyJubjub consent signature + public key.
#[cfg(feature = "prover")]
#[derive(Deserialize)]
struct ProveVerificationReq {
    /// The stored `WrappedDoc` (raw salted leaves; the WITNESS). Accepts either an embedded JSON
    /// object or a stringified JSON document (the phone has it as a string).
    #[serde(rename = "wrappedDoc")]
    wrapped_doc: Value,
    /// The §1.10 consent (all 0x.. hex fields), same shape as `eddsaConsentJson` / the POSTed consent.
    consent: Value,
    /// The EdDSA-BabyJubjub consent signature + public key: `{ r8xDec, r8yDec, sDec, axHex, ayHex }`.
    #[serde(rename = "eddsaSig")]
    eddsa_sig: EddsaSigReq,
}

/// The pass-through EdDSA signature fields (mirrors the on-device `EddsaSigInput` UniFFI record).
#[cfg(feature = "prover")]
#[derive(Deserialize)]
struct EddsaSigReq {
    #[serde(rename = "r8xDec")]
    r8x_dec: String,
    #[serde(rename = "r8yDec")]
    r8y_dec: String,
    #[serde(rename = "sDec")]
    s_dec: String,
    #[serde(rename = "axHex")]
    ax_hex: String,
    #[serde(rename = "ayHex")]
    ay_hex: String,
}

/// `POST /prove-verification` — the TRUSTED PROVER SERVICE.
///
/// A 32-bit-only Android phone cannot generate a valid Groth16 proof on-device (no off-the-shelf
/// 32-bit-ARM prover). It instead POSTs `{ wrappedDoc, consent, eddsaSig }` here; this handler
/// assembles the 19 circuit inputs (REUSING the SAME `dogtag_standard::prover_assemble` assembly the
/// on-device path uses) and generates the proof with the 64-bit-correct ark-0.6 Arkworks prover
/// (`ArkProver`, the same one whose proofs cast-verify on the live `Groth16Verifier`). It returns the
/// Solidity calldata `{ a, b, c, pub }` — the exact shape the groomer's `/v1/verify/consent` accepts
/// as its `proof`. The phone then submits THAT proof to the groomer itself.
///
/// TRUST NOTE: this service sees the witness (the wrapped doc + the EdDSA sig). It is therefore NOT
/// the groomer — the groomer never sees the witness, only the resulting proof. In PRODUCTION this is
/// the OWNER's own trusted prover (or a service the owner trusts); the demo runs it as a platform
/// service (a dedicated vet-api instance with `CIRCUITS_BUILD_DIR` set so the real `ArkProver` — not
/// the `StubProver` — is loaded). The route is unauthenticated by design (anyone can ask for a proof
/// of THEIR OWN record); it discloses nothing the caller did not already hold.
#[cfg(feature = "prover")]
async fn prove_verification(State(st): State<AppState>, Json(body): Json<ProveVerificationReq>) -> Resp {
    // The wrapped doc / consent may arrive as an embedded object or a JSON string; normalize to text.
    let as_text = |v: &Value| -> String {
        match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    };
    let wrapped_doc_json = as_text(&body.wrapped_doc);
    let consent_json = as_text(&body.consent);

    let sig = dogtag_standard::prover_assemble::EddsaSigInput {
        r8x_dec: body.eddsa_sig.r8x_dec,
        r8y_dec: body.eddsa_sig.r8y_dec,
        s_dec: body.eddsa_sig.s_dec,
        ax_hex: body.eddsa_sig.ax_hex,
        ay_hex: body.eddsa_sig.ay_hex,
    };

    // Assemble the 19 named circuit inputs from the witness — the SAME assembly the on-device prover
    // runs. (ark-0.5 field types stay internal to dogtag-standard; only decimal strings escape.) The
    // returned Value is already in the `ProveInputs::from_circuit_input_json` shape.
    let circuit_input_json =
        match dogtag_standard::prover_assemble::assemble_circuit_input(&wrapped_doc_json, &consent_json, &sig) {
            Ok(v) => v,
            Err(e) => return err(StatusCode::BAD_REQUEST, &format!("assemble: {e}")),
        };

    // Prove with the backend's ark-0.6 Arkworks prover (the live-verifier-correct one). Driving it
    // through the shared `ProverClient` with the assembled `circuit_input_json` runs the real
    // `ArkProver`; if this instance has no `CIRCUITS_BUILD_DIR` it is the `StubProver` and returns a
    // placeholder — so a real prover-service MUST set `CIRCUITS_BUILD_DIR` (demo-up.sh does).
    let input = crate::prover::ProveInput {
        circuit_input_json: Some(circuit_input_json),
        ..Default::default()
    };
    let proof = match st.prover.prove(input).await {
        Ok(p) => p,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("prover: {e}")),
    };

    // Return the Solidity calldata in the EXACT `{a, b, c, pub}` shape the groomer's
    // `/v1/verify/consent` accepts as `proof` (mirrors `dogtag_prover::Groth16Output`). All decimal.
    ok(json!({
        "a": proof.a,
        "b": proof.b,
        "c": proof.c,
        "pub": proof.pub_signals,
    }))
}

// --------------------------------------------------------------------------------------------
// DOG_PROFILE (SBT) issuance — the VET issues dog tags (operator starts a session showing a QR; the
// device scans, posts its wallet + signature; the vet mints the DOG_PROFILE SBT to that wallet with
// the owner-identity baked into the merkle leaves).
// --------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct ProfileOwnerIdentityReq {
    #[serde(rename = "countryOfIdentification", default)]
    country_of_identification: String,
    #[serde(default)]
    identification: String,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct ProfilePetReq {
    name: String,
    #[serde(default)]
    species: Option<String>,
    #[serde(rename = "breedVbo", default)]
    breed_vbo: Option<String>,
    #[serde(rename = "breedLabel", default)]
    breed_label: Option<String>,
    #[serde(default)]
    sex: Option<String>,
    #[serde(rename = "neuterStatus", default)]
    neuter_status: Option<String>,
    #[serde(rename = "dateOfBirth", default)]
    date_of_birth: Option<String>,
    #[serde(rename = "weightHistory", default)]
    weight_history: Vec<Value>,
    #[serde(default)]
    microchip: Option<ProfileMicrochipReq>,
}

#[derive(Deserialize)]
struct ProfileMicrochipReq {
    #[serde(default)]
    code: String,
    #[serde(default)]
    standard: String,
    #[serde(rename = "implantDate", default)]
    implant_date: String,
    #[serde(rename = "bodyLocation", default)]
    body_location: String,
}

#[derive(Deserialize)]
struct ProfileIssueStartReq {
    #[serde(rename = "ownerIdentity")]
    owner_identity: ProfileOwnerIdentityReq,
    pet: ProfilePetReq,
}

/// The CANONICAL on-chain dogTagId = `field_of_value(Integer(handle))` — the verification circuit's
/// `pub[0]` and the contract's `ownerOf`/`profileRoot` key. The DOG_PROFILE SBT MUST be minted under this
/// (not the raw numeric handle), so the owner's later ZK export passes `ownerOf(pub[0])`. The raw handle
/// stays the operator-facing id + the credential's `dogTagId` leaf (which the circuit field-hashes to
/// exactly this). Mirrors the `field-hash` bin / `dog_tag_id_field_hex` FFI.
fn onchain_dog_tag_id(handle: &str) -> Result<String, String> {
    let scalar =
        dogtag_standard::wrap::scalar_from_packed(dogtag_standard::types::TypeTag::Integer, handle)
            .map_err(|e| e.to_string())?;
    let f = dogtag_standard::leaf::field_of_value(&scalar).map_err(|e| e.to_string())?;
    Ok(dogtag_standard::field::to_hex32(&f))
}

/// POST /profiles/issue/session/start — operator-session gated. Allocate a dogTagId, persist a
/// ProfileIssueSession with a fresh 16-byte one-time bind token (180s TTL), and return the QR URL
/// `<deployment_url>/p/<token>` the device scans. Returns `{ token, dogTagId, sessionId, qr }`.
async fn profile_issue_session_start(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ProfileIssueStartReq>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    // Allocate a dogTagId that is NOT already minted on the (shared) DogTagSBT. The local counter
    // resets on restart and the SBT is shared across issuers, so a fresh counter can collide with an
    // already-minted id — `DogTagSBT.mint` reverts on a duplicate token. Skip taken ids: owner_of
    // returns Err(NotFound) for an unminted id (free) and Ok(_) for a minted one (taken).
    let mut dog_tag_id = st.store.next_dog_tag_id().await.to_string();
    for _ in 0..256 {
        // The SBT is minted under the field-hashed id, so the collision-check must query
        // ownerOf(field_of_value(handle)), not the raw handle.
        let onchain = match onchain_dog_tag_id(&dog_tag_id) {
            Ok(v) => v,
            Err(_) => break, // non-numeric handle can't collide via this path; proceed
        };
        match st.chain.owner_of(&st.cfg.sbt_addr, &onchain).await {
            Err(crate::chain::ChainError::NotFound) => break, // unminted -> free
            Ok(_) => dog_tag_id = st.store.next_dog_tag_id().await.to_string(), // taken -> next
            Err(_) => break, // transient RPC error: proceed (a real dup would revert at mint)
        }
    }

    let owner_identity = crate::store::OwnerIdentity {
        country_of_identification: body.owner_identity.country_of_identification,
        identification: body.owner_identity.identification,
        name: body.owner_identity.name,
    };
    let microchip = match body.pet.microchip {
        Some(m) => crate::store::Microchip {
            code: m.code,
            standard: m.standard,
            implant_date: m.implant_date,
            body_location: m.body_location,
        },
        None => crate::store::Microchip::default(),
    };
    let weight_history: Vec<crate::store::WeightEntry> = body
        .pet
        .weight_history
        .iter()
        .filter_map(|w| {
            Some(crate::store::WeightEntry {
                unit: w.get("unit").and_then(|v| v.as_str())?.to_string(),
                value: w.get("value").and_then(|v| v.as_str())?.to_string(),
                measured_on: w.get("measuredOn").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            })
        })
        .collect();
    let profile = crate::store::PetProfile {
        species: body.pet.species,
        breed_vbo: body.pet.breed_vbo,
        breed_label: body.pet.breed_label,
        sex: body.pet.sex,
        neuter_status: body.pet.neuter_status,
        date_of_birth: body.pet.date_of_birth,
        weight_history,
    };

    let session_id = uuid::Uuid::new_v4().to_string();
    st.store
        .put_profile_session(crate::store::ProfileIssueSession {
            session_id: session_id.clone(),
            dog_tag_id: dog_tag_id.clone(),
            owner_identity,
            pet_name: body.pet.name,
            microchip,
            profile,
            status: "pending".to_string(),
            created_at: auth::now(),
            wallet_address: None,
            root: None,
            tx_hash: None,
        })
        .await;

    // one-time 16-byte bind token (180s TTL) -> session; the QR carries `<deployment_url>/p/<token>`.
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    let token = hex::encode(bytes);
    let exp = auth::now() + 180;
    st.store.put_bind_token(&token, &session_id, exp).await;
    let qr = format!("{}/p/{}", st.cfg.deployment_url, token);
    ok(json!({ "token": token, "dogTagId": dog_tag_id, "sessionId": session_id, "qr": qr }))
}

/// GET /p/{token} — resolve a one-time bind token to the session metadata the device needs to build
/// its registration signature ({ sessionId, dogTagId, registrationMessageWalletField }). Unauthenticated
/// and NON-consuming (consumed only on bind). A missing/expired token is a 404. Symmetric with `/x/`.
async fn profile_bind_resolve(State(st): State<AppState>, Path(token): Path<String>) -> Resp {
    let session_id = match st.store.peek_bind_token(&token).await {
        Some(id) => id,
        None => return err(StatusCode::NOT_FOUND, "bind token missing or expired"),
    };
    match st.store.get_profile_session(&session_id).await {
        Some(s) => ok(json!({
            "sessionId": s.session_id,
            "dogTagId": s.dog_tag_id,
            "status": s.status,
            // the device signs `register_message(walletAddress)` = "DogTag wallet registration: <lc>".
            "registrationMessagePrefix": "DogTag wallet registration: ",
        })),
        None => err(StatusCode::NOT_FOUND, "session not found"),
    }
}

#[derive(Deserialize)]
struct ProfileBindReq {
    token: String,
    #[serde(rename = "walletAddress")]
    wallet_address: String,
    signature: String,
}

/// POST /profiles/issue/bind — device, token-authenticated (unauthenticated by operator session). The
/// device posts `{ token, walletAddress, signature }`; `signature` is an EIP-191 personal_sign over
/// `register_message(walletAddress)`. Recover the signer; require it == walletAddress. Consume the
/// token atomically (one-time). Build the DOG_PROFILE VC -> wrap -> root R (all off-chain, fast), then
/// RESPOND IMMEDIATELY with `{ wrappedDoc, dogTagId, root, walletAddress, status: "minting" }` and mint
/// the SBT to the wallet via the vet signer (ISSUER_ROLE) in the BACKGROUND (the on-chain receipt is
/// ~12-24s, exceeding the phone's read timeout). The phone polls the chain until the mint lands; the
/// operator portal polls `GET /profiles/issue/session/{id}` (pending -> bound+txHash, or error).
async fn profile_issue_bind(State(st): State<AppState>, Json(body): Json<ProfileBindReq>) -> Resp {
    let wallet = body.wallet_address.trim().to_lowercase();
    // shape check: a 20-byte 0x address.
    if wallet.len() != 42 || !wallet.starts_with("0x") || !wallet[2..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return err(StatusCode::BAD_REQUEST, "walletAddress must be a 0x.. 20-byte address");
    }
    // recover the EIP-191 signer of the canonical message; require it == walletAddress.
    let message = auth::register_message(&wallet);
    let recovered = match auth::recover_personal_sign(&message, &body.signature) {
        Some(a) => a,
        None => return err(StatusCode::BAD_REQUEST, "malformed signature"),
    };
    if !recovered.eq_ignore_ascii_case(&wallet) {
        return err(StatusCode::UNAUTHORIZED, "signature does not match walletAddress");
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    // consume the one-time token atomically (second call -> 404/410).
    let session_id = match st.store.take_bind_token(&body.token).await {
        Some(id) => id,
        None => return err(StatusCode::GONE, "bind token missing, expired or already used"),
    };
    let mut session = match st.store.get_profile_session(&session_id).await {
        Some(s) if s.status == "pending" => s,
        Some(_) => return err(StatusCode::CONFLICT, "session already bound"),
        None => return err(StatusCode::NOT_FOUND, "session not found"),
    };

    // build the DOG_PROFILE VC with the owner-identity baked in, wrap -> root.
    let meta = app::profile_issuer_meta(&st.cfg);
    let vc = app::build_profile_vc(
        &st.cfg,
        &session.pet_name,
        &session.microchip,
        &session.profile,
        &session.owner_identity,
        &wallet,
        &session.dog_tag_id,
    );
    let doc = match app::wrap_vc(meta, &vc) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::BAD_REQUEST, &e),
    };
    let root = doc.signature.merkle_root.clone();

    // Persist the wallet + root immediately (status stays "pending" until the async mint lands). This
    // lets the portal/device observe the bound wallet/root before the on-chain receipt arrives.
    session.wallet_address = Some(wallet.clone());
    session.root = Some(root.clone());
    st.store.update_profile_session(session.clone()).await;

    // Mint the SBT to the device wallet ASYNC: ROAX blocks are ~12s apart so the on-chain receipt takes
    // ~12-24s — far longer than the phone's HTTP read timeout. We RESPOND IMMEDIATELY with the wrapped
    // doc (built off-chain) and `status: "minting"`, and run the mint in the background; the phone polls
    // the chain until the mint lands. The spawned task updates the session (-> "bound"+txHash, or
    // "error"+message) so the operator portal status poll keeps working.
    //
    // Clone the needed Arcs/values OUT of the app state before the spawn so the future is `Send + 'static`.
    let chain = st.chain.clone();
    let store = st.store.clone();
    let sbt_addr = st.cfg.sbt_addr.clone();
    let signer_index = st.cfg.vet_signer_index;
    // Mint + read-back under the field-hashed on-chain id (== the export's pub[0]); the raw handle stays
    // the credential's dogTagId. Computed here so a bad handle fails synchronously, before the spawn.
    let onchain_id = match onchain_dog_tag_id(&session.dog_tag_id) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("dogTagId field-hash: {e}")),
    };
    let mint_wallet = wallet.clone();
    let mint_root = root.clone();
    let mut bg_session = session.clone();
    tokio::spawn(async move {
        match chain
            .mint(signer_index, &sbt_addr, &mint_wallet, &onchain_id, &mint_root)
            .await
        {
            Ok(sent) => {
                // VERIFY ON-CHAIN before marking the issuance correct: the mint receipt is not enough —
                // read back ownerOf(dogTagId)==device wallet AND profileRoot(dogTagId)==the issued root.
                // Only a chain-confirmed match flips to "bound"; anything else is "error" (so the device
                // never accepts an unverified issuance).
                let owner_ok = matches!(
                    chain.owner_of(&sbt_addr, &onchain_id).await,
                    Ok(o) if o.eq_ignore_ascii_case(&mint_wallet)
                );
                let root_ok = matches!(
                    chain.profile_root_of(&sbt_addr, &onchain_id).await,
                    Ok(r) if r.eq_ignore_ascii_case(&mint_root)
                );
                if owner_ok && root_ok {
                    bg_session.status = "bound".to_string();
                    bg_session.tx_hash = Some(sent.tx_hash);
                } else {
                    bg_session.status = "error".to_string();
                    bg_session.tx_hash =
                        Some(format!("on-chain verify failed (owner_ok={owner_ok} root_ok={root_ok})"));
                }
            }
            Err(e) => {
                bg_session.status = "error".to_string();
                bg_session.tx_hash = Some(format!("mint error: {e}"));
            }
        }
        store.update_profile_session(bg_session).await;
    });

    ok(json!({
        "wrappedDoc": serde_json::to_value(&doc).unwrap(),
        "dogTagId": session.dog_tag_id,
        "root": root,
        "walletAddress": wallet,
        "status": "minting",
    }))
}

/// GET /profiles/issue/session/{id} — operator-gated status poll so the portal can show whether the
/// device has bound + surface the txHash/root/wallet. Returns the stored session row's status.
async fn profile_issue_session_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let s = match st.store.get_profile_session(&session_id).await {
        Some(s) => s,
        None => return err(StatusCode::NOT_FOUND, "session not found"),
    };
    ok(json!({
        "status": s.status,
        "dogTagId": s.dog_tag_id,
        "walletAddress": s.wallet_address,
        "root": s.root,
        "txHash": s.tx_hash,
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
    // SERVER-SIDE PROVING API (32-bit Android fallback). Mounted only when compiled with the
    // `prover` feature AND this instance is the dedicated prover-service (CIRCUITS_BUILD_DIR set ->
    // a real ArkProver). The groomer instance is built WITHOUT this feature, so it can never be asked
    // to prove and therefore never sees a witness through this path. See `prove_verification`.
    #[cfg(feature = "prover")]
    let prove_route = Router::new().route("/prove-verification", post(prove_verification));
    #[cfg(not(feature = "prover"))]
    let prove_route = Router::<AppState>::new();

    Router::new()
        .merge(prove_route)
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
        // short one-time EXPORT token resolver (unauthenticated; NON-consuming — consume on submit)
        .route("/x/:token", get(export_session_resolve))
        // DOG_PROFILE (SBT) issuance — vet issues dog tags
        .route("/profiles/issue/session/start", post(profile_issue_session_start))
        .route("/profiles/issue/session/:id", get(profile_issue_session_status))
        .route("/profiles/issue/bind", post(profile_issue_bind))
        // short one-time bind token resolver (unauthenticated; NON-consuming — consume on bind)
        .route("/p/:token", get(profile_bind_resolve))
        // issuer signers
        .route("/issuer/signers", get(issuer_signers))
        // import
        .route("/import/pull", post(import_pull))
        // verify
        .route("/verify/session/start", post(export_session_start))
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
