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
//!     POST /verify/credential                         -> direct credential validity/revocation check
//!     POST /verify/session/start | /verify/consent/submit   (EXPORT flow; route PATHS kept stable)
//!     GET  /verify/history                            (operator-gated verifier audit log)
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
use dogtag_standard::verify::{check_integrity, FragmentState};
use dogtag_standard::wrap::WrappedDoc;
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
    let token =
        bearer(headers).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing operator session"))?;
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
        .ok_or_else(|| {
            err(
                StatusCode::UNAUTHORIZED,
                "missing operator session or export token",
            )
        })?;
    // SUBMIT consumes the token once-only; the status read peeks without consuming.
    let mapped = if consume {
        st.store.take_export_token(&token).await
    } else {
        st.store.peek_export_token(&token).await
    };
    let mapped = mapped.ok_or_else(|| {
        err(
            StatusCode::UNAUTHORIZED,
            "export token missing, expired or already used",
        )
    })?;
    if mapped != session_id {
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "export token does not match session",
        ));
    }
    Ok(true)
}

/// Require a valid admin session bearer (custody gate). Same mechanism, distinct token prefix.
async fn require_admin(st: &AppState, headers: &HeaderMap) -> Result<(), Resp> {
    let token =
        bearer(headers).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing admin session"))?;
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
        return err(
            StatusCode::TOO_MANY_REQUESTS,
            "too many attempts; try again later",
        );
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
        return err(
            StatusCode::TOO_MANY_REQUESTS,
            "too many attempts; try again later",
        );
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
    if st
        .store
        .get_custody()
        .await
        .map(|c| c.meta.state == "initialized")
        .unwrap_or(false)
    {
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
        if !all.get(idx).map(|w| w == typed).unwrap_or(false) {
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
    let blob = crate::store::CustodyBlob {
        encrypted_seed: ct,
        meta: crate::store::KeystoreMeta {
            accounts: vec![crate::store::AccountMeta {
                index: 0,
                address: addr0.clone(),
                label: "account0".to_string(),
            }],
            state: "initialized".to_string(),
        },
    };
    st.store.put_custody(blob.clone()).await;
    // ALSO persist the seal to disk (if configured) so the signer survives a backend restart. We
    // write ONLY the ciphertext + non-secret meta (atomic temp+rename, 0600). A write failure here
    // is fatal to the request: the operator must know the seal is NOT durable before they navigate
    // away (otherwise a restart silently loses the just-genesised seed).
    if let Some(path) = st.cfg.custody_seal_path.as_deref() {
        if let Err(e) = crate::custody::write_seal_file(path, &blob.encrypted_seed, &blob.meta) {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("persist seal: {e}"),
            );
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
        return err(
            StatusCode::TOO_MANY_REQUESTS,
            "too many attempts; try again later",
        );
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

async fn accounts(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AccountsReq>,
) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    let mut blob = st.store.get_custody().await.unwrap_or_default();
    let next = blob
        .meta
        .accounts
        .iter()
        .map(|a| a.index)
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);
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
        return err(
            StatusCode::CONFLICT,
            "prepared record outstanding; cannot switch mode",
        );
    }
    st.store
        .put_settings(IssuerSettings {
            signing_mode: body.mode.clone(),
        })
        .await;
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

async fn prepare(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PrepareReq>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    let issuer_addr = match st.cfg.issuer_addr_for(&body.record_type) {
        Some(a) => a,
        None => {
            return err(
                StatusCode::BAD_REQUEST,
                "unknown recordType / no issuer address",
            )
        }
    };
    // build (ALWAYS server-side, identical both modes).
    let meta = app::issuer_meta(&st.cfg, &body.record_type, &issuer_addr);
    let vc = app::build_vc(&body.record_type, &body.fields, &body.dog_tag_id);
    let mut doc = match app::wrap(&body.record_type, meta, &vc) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::BAD_REQUEST, &e),
    };
    // Stamp the M7 provenance block (§4.2), BESIDE R - never inside it. `issuerSigner` is left empty
    // until the authoritative on-chain `issuedBy[R]` is derived at confirm (the `RootIssued` log).
    let chain_id = st.chain.chain_id();
    doc.protocol = Some(app::protocol_meta(&st.cfg, chain_id, &issuer_addr, ""));
    let root = doc.signature.merkle_root.clone();
    let target = doc.signature.target_hash.clone();
    let calldata = crate::chain::issue_calldata(&root);
    let record_id = uuid::Uuid::new_v4().to_string();

    let created_at = auth::now();
    let mut record = Record {
        record_id: record_id.clone(),
        record_type: body.record_type.clone(),
        dog_tag_id: body.dog_tag_id.clone(),
        wrapped_doc: serde_json::to_value(&doc).unwrap(),
        root: root.clone(),
        prepared_calldata: calldata.clone(),
        issuer_addr: issuer_addr.clone(),
        // M7 provenance mirror (§4.2): the routing-fixed fields are known now; `issuer_signer` is the
        // on-chain `issuedBy[R]`, filled at confirm from the `RootIssued` log.
        chain_id: Some(chain_id),
        protocol_version: Some(dogtag_standard::wrap::LEVEL_A_VERSION.to_string()),
        verification_registry: Some(st.cfg.verification_registry_addr.clone()),
        issuer_signer: None,
        status: RecordStatus::Prepared,
        tx_hash: None,
        confirmed_tx_hash: None,
        signer_address: None,
        signing_mode: None,
        block_number: None,
        explorer_url: None,
        created_at,
        updated_at: created_at,
        label: None,
        notes: None,
        revoked_tx_hash: None,
        revoked_block_number: None,
        revoke_explorer_url: None,
        invalidated_at: None,
        invalidation_reason: None,
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
        Ok(false) => {
            return err(
                StatusCode::FORBIDDEN,
                "address not approved for this recordType yet",
            )
        }
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

async fn confirm(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ConfirmReq>,
) -> Resp {
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
        return Err(err(
            StatusCode::BAD_REQUEST,
            "tx.input mismatch (not this draft)",
        ));
    }
    if !view.value.is_zero() {
        return Err(err(StatusCode::BAD_REQUEST, "tx.value != 0"));
    }
    if view.chain_id != Some(st.chain.chain_id()) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "tx.chainId mismatch (wrong chain)",
        ));
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
        Ok(false) => {
            return Err(err(
                StatusCode::FORBIDDEN,
                "signer not whitelisted at confirm",
            ))
        }
        Err(e) => return Err(err(StatusCode::BAD_GATEWAY, &format!("whitelist: {e}"))),
    }
    // RootIssued(root,by) from the pinned issuer; ev.root==r.root && ev.by==signer.
    let matched = view.root_issued_logs.iter().any(|(root, by)| {
        root.to_lowercase() == r.root.to_lowercase() && by.to_lowercase() == signer
    });
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
    // M7 provenance (§4.2): the authoritative issuer signer (== `clone.issuedBy[R]`, here the
    // `RootIssued`-derived `signer`) fills the mirror column AND the envelope block's `issuerSigner`.
    // The block sits OUTSIDE `R`, so patching it never perturbs the anchored root.
    r.issuer_signer = Some(signer.clone());
    if let Some(p) = r
        .wrapped_doc
        .get_mut("protocol")
        .and_then(|v| v.as_object_mut())
    {
        p.insert(
            "issuerSigner".to_string(),
            serde_json::Value::String(signer.clone()),
        );
    }
    r.signing_mode = Some(st.store.get_settings().await.signing_mode);
    // Persist the immutable on-chain proof: block number + a ready-to-click explorer link.
    r.block_number = view.block_number;
    r.explorer_url = Some(crate::chain::explorer_tx_url(tx_hash));
    r.updated_at = auth::now();
    st.store.update_record(r).await;
    Ok(json!({ "recordId": record_id, "status": "issued", "blockNumber": view.block_number }))
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
    if !matches!(r.status, RecordStatus::Issued | RecordStatus::Expired) {
        return err(StatusCode::CONFLICT, "record not issued or expired");
    }
    let calldata = crate::chain::revoke_calldata(&r.root);
    let sent = match st.chain.sign_and_send(0, &r.issuer_addr, &calldata).await {
        Ok(s) => s,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("revoke broadcast: {e}")),
    };
    // Soft-invalidation: NEVER delete. Flip to revoked and record the revoke tx's on-chain proof
    // (hash + block + explorer link) alongside the ORIGINAL issuance proof, which stays intact. The
    // record remains listable + verifiable (its on-chain `isValid` now reads false).
    let revoke_block = st
        .chain
        .get_tx_view(&sent.tx_hash, &r.issuer_addr, 1)
        .await
        .ok()
        .and_then(|v| v.block_number);
    r.status = RecordStatus::Revoked;
    r.revoked_tx_hash = Some(sent.tx_hash.clone());
    r.revoked_block_number = revoke_block;
    r.revoke_explorer_url = Some(crate::chain::explorer_tx_url(&sent.tx_hash));
    r.invalidated_at = Some(auth::now());
    r.updated_at = auth::now();
    st.store.update_record(r).await;
    ok(
        json!({ "recordId": id, "status": "revoked", "txHash": sent.tx_hash, "blockNumber": revoke_block }),
    )
}

/// GET /records — list every record this device has issued, most-recent first (operator-gated).
/// Surfaces the full history INCLUDING revoked/expired records with their on-chain proof + explorer
/// links, so the operator can trace each credential back to the chain and re-verify it.
async fn list_records(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let records = st.store.list_records().await;
    ok(json!({ "records": records }))
}

#[derive(Deserialize)]
struct UpdateRecordReq {
    /// Off-chain status transition — only "expired" is permitted here (a validity lapse that needs no
    /// chain tx). Use POST /records/:id/revoke for on-chain revocation.
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

/// PATCH /records/:id — update OFF-CHAIN metadata only (operator-gated). On-chain-derived fields (tx
/// hash, block number, contract/issuer address, root, the anchored wrapped doc) are IMMUTABLE and are
/// not accepted here — the typed body only exposes off-chain fields, and any on-chain-derived key in
/// the raw body is rejected with 400. Editable: `label`, `notes`, and `status` (only → `expired`).
async fn update_record_meta(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(raw): Json<Value>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    // Reject any attempt to set an on-chain-derived field — immutable chain state cannot be edited.
    const IMMUTABLE_KEYS: &[&str] = &[
        "recordId",
        "record_id",
        "recordType",
        "record_type",
        "dogTagId",
        "dog_tag_id",
        "root",
        "merkleRoot",
        "wrappedDoc",
        "wrapped_doc",
        "issuerAddr",
        "issuer_addr",
        "contractAddress",
        "preparedCalldata",
        "prepared_calldata",
        "txHash",
        "tx_hash",
        "confirmedTxHash",
        "confirmed_tx_hash",
        "blockNumber",
        "block_number",
        "explorerUrl",
        "explorer_url",
        "signerAddress",
        "signer_address",
        "revokedTxHash",
        "revoked_tx_hash",
        "revokedBlockNumber",
        "revoked_block_number",
        "revokeExplorerUrl",
        "revoke_explorer_url",
    ];
    if let Some(obj) = raw.as_object() {
        for k in obj.keys() {
            if IMMUTABLE_KEYS.contains(&k.as_str()) {
                return err(
                    StatusCode::BAD_REQUEST,
                    &format!("field '{k}' is on-chain-derived and immutable"),
                );
            }
        }
    }
    // label / notes — free-form off-chain metadata (present + null clears, present + string sets,
    // absent leaves unchanged; any other JSON type is rejected rather than silently clearing).
    for key in ["label", "notes"] {
        if let Some(v) = raw.get(key) {
            if !v.is_null() && !v.is_string() {
                return err(
                    StatusCode::BAD_REQUEST,
                    &format!("{key} must be a string or null"),
                );
            }
        }
    }
    let label_update = raw.get("label").map(|v| v.as_str().map(String::from));
    let notes_update = raw.get("notes").map(|v| v.as_str().map(String::from));
    let body: UpdateRecordReq = match serde_json::from_value(raw) {
        Ok(b) => b,
        Err(e) => return err(StatusCode::BAD_REQUEST, &format!("bad body: {e}")),
    };

    let mut r = match st.store.get_record(&id).await {
        Some(r) => r,
        None => return err(StatusCode::NOT_FOUND, "record not found"),
    };
    if let Some(v) = label_update {
        r.label = v;
    }
    if let Some(v) = notes_update {
        r.notes = v;
    }
    if let Some(s) = body.status.as_deref() {
        match s {
            "expired" => {
                if r.status != RecordStatus::Issued {
                    return err(StatusCode::CONFLICT, "only issued records can be expired");
                }
                r.status = RecordStatus::Expired;
                r.invalidated_at = Some(auth::now());
                if r.invalidation_reason.is_none() {
                    r.invalidation_reason = body.reason.clone();
                }
            }
            other => {
                return err(
                    StatusCode::BAD_REQUEST,
                    &format!("status can only be set to 'expired' via update (got '{other}')"),
                )
            }
        }
    }
    r.updated_at = auth::now();
    st.store.update_record(r.clone()).await;
    ok(serde_json::to_value(r).unwrap_or(Value::Null))
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
async fn get_record(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Resp {
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

async fn import_pull(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ImportPullReq>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let url = format!(
        "{}/share/{}",
        body.user_api_base.trim_end_matches('/'),
        body.record_ref
    );
    let client = reqwest::Client::new();
    let resp = client.get(&url).bearer_auth(&body.user_jwt).send().await;
    let doc_val: Value = match resp {
        Ok(r) if r.status().is_success() => match r.json().await {
            Ok(v) => v,
            Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("bad doc json: {e}")),
        },
        Ok(r) => {
            return err(
                StatusCode::BAD_GATEWAY,
                &format!("fetch failed: {}", r.status()),
            )
        }
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("fetch error: {e}")),
    };
    // third-party verify via the SDK.
    let doc: dogtag_standard::wrap::WrappedDoc = match serde_json::from_value(doc_val.clone()) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::BAD_REQUEST, &format!("not a WrappedDoc: {e}")),
    };
    let verdict = crate::verify::third_party_verify(&st, &doc).await;
    if !verdict.valid {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "third-party verify invalid",
        );
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
struct VerifyCredentialReq {
    /// The wrapped credential document to verify (as produced by any DogTag issuer).
    #[serde(rename = "wrappedDoc", alias = "wrapped_doc")]
    wrapped_doc: WrappedDoc,
    /// Override the DogTagIssuer clone to check `isValid`/`isRevoked` against. Defaults to the doc's
    /// `issuer.documentStore`.
    #[serde(rename = "issuerAddr", alias = "issuer_addr", default)]
    issuer_addr: Option<String>,
    /// Optional signer address to check issuer identity (`isWhitelistedFor(recordType, signer)`).
    #[serde(rename = "signerAddr", alias = "signer_addr", default)]
    signer_addr: Option<String>,
}

/// POST /verify/credential — operator-gated direct credential check. This is the verifier-facing
/// "paste a credential and learn whether it is currently valid or revoked" path: no tx, no storage, just
/// integrity recompute + gasless on-chain reads.
async fn verify_credential(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<VerifyCredentialReq>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }

    let doc = body.wrapped_doc;
    let record_type = doc.issuer.record_type.clone();
    let issuer_addr = body
        .issuer_addr
        .clone()
        .unwrap_or_else(|| doc.issuer.document_store.clone());
    let claimed_root = doc.signature.merkle_root.clone();

    let (integrity_state, recomputed) = check_integrity(&doc);
    let integrity_valid = integrity_state == FragmentState::Valid;
    let recomputed_hex = dogtag_standard::to_hex32(&recomputed);

    let issued_at = match st.chain.issued_at(&issuer_addr, &claimed_root).await {
        Ok(v) => v,
        Err(e) => {
            return err(
                StatusCode::BAD_GATEWAY,
                &format!("on-chain issuedAt read failed: {e}"),
            )
        }
    };
    let issued = !issued_at.is_zero();
    let onchain_valid = match st.chain.is_valid(&issuer_addr, &claimed_root).await {
        Ok(v) => v,
        Err(e) => {
            return err(
                StatusCode::BAD_GATEWAY,
                &format!("on-chain isValid read failed: {e}"),
            )
        }
    };
    let revoked = match st.chain.is_revoked(&issuer_addr, &claimed_root).await {
        Ok(v) => v,
        Err(e) => {
            return err(
                StatusCode::BAD_GATEWAY,
                &format!("on-chain isRevoked read failed: {e}"),
            )
        }
    };

    let issuer_whitelisted = match &body.signer_addr {
        Some(signer) if !signer.trim().is_empty() => {
            let rt_key = app::rt_key(&record_type);
            match st
                .chain
                .is_whitelisted_for(&st.cfg.issuer_registry_addr, &rt_key, signer.trim())
                .await
            {
                Ok(v) => Some(v),
                Err(e) => {
                    return err(
                        StatusCode::BAD_GATEWAY,
                        &format!("on-chain whitelist read failed: {e}"),
                    )
                }
            }
        }
        _ => None,
    };

    let status = if !integrity_valid {
        "integrity_failed"
    } else if !issued {
        "not_issued"
    } else if revoked {
        "revoked"
    } else if onchain_valid {
        "valid"
    } else {
        "invalid"
    };
    let verdict = integrity_valid && onchain_valid && issuer_whitelisted.unwrap_or(true);

    ok(json!({
        "verdict": verdict,
        "status": status,
        "recordType": record_type,
        "root": claimed_root,
        "recomputedRoot": recomputed_hex,
        "issuerAddr": issuer_addr,
        "signerAddr": body.signer_addr,
        "issuedAt": issued_at.to_string(),
        "checkedAt": auth::now(),
        "fragments": {
            "integrity": integrity_valid,
            "onchain": onchain_valid,
            "issued": issued,
            "revoked": revoked,
            "issuerWhitelisted": issuer_whitelisted,
        },
    }))
}

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
        Ok(false) => {
            return err(
                StatusCode::FORBIDDEN,
                "relayer not whitelisted for this purpose",
            )
        }
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("verify-wl: {e}")),
    }
    let mode = body.mode.clone().unwrap_or_else(|| "zk".to_string());
    // Validate against the known vocabulary (`store::VerifySession::mode`) rather than storing
    // whatever arrives. An unrecognised mode previously fell through the `if mode == "normal"`
    // dispatch into the LEVEL-A ZK branch, so a typo ("level-b", "levelB") would silently produce a
    // Level-A session that advertises Level-A claims — the failure would surface only as a confusing
    // on-chain revert. Fail closed on the name instead.
    if !matches!(mode.as_str(), "normal" | "zk" | "levelb") {
        return err(
            StatusCode::BAD_REQUEST,
            "mode must be one of: normal, zk, levelb",
        );
    }
    // A `levelb` session is a promise the phone will be told to submit to the Level-B registry. If
    // that registry is unconfigured the submit would 503 AFTER the owner has scanned, consented and
    // spent tens of seconds proving on-device. Refuse at session start instead — same fail-closed
    // posture `consent_submit_levelb` applies, just moved to the first point it is knowable.
    if mode == "levelb"
        && !crate::verify::valid_contract_addr(&st.cfg.verification_registry_consent_addr)
    {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "level-b verification registry not configured",
        );
    }
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut challenge = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut challenge);
    let challenge_hex = format!("0x{}", hex::encode(challenge));
    let now = auth::now();
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
            created_at: now,
            updated_at: now,
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
        Some(s) => {
            // M7 P4 (§5.2): the CONVENIENCE tier — platform-OWNED, UNVERIFIED claims. Additive to the
            // existing fields (back-compat). The verify-flow issuer clone is the one for this record
            // type; the purpose is the session's verify purpose. The app validates these against the
            // dogtag ProtocolRegistry / signed-manifest anchor before trusting them.
            let issuer_clone = st.cfg.issuer_addr_for(&s.record_type).unwrap_or_default();
            // M-4: a `levelb` session advertises the Level-B version + registry; every other mode is
            // unchanged (Level-A). This is the per-session opt-in that makes Level-B AVAILABLE
            // without making it the DEFAULT — the default flip is M-5.
            let claims = app::convenience_claims_for_mode(
                &st.cfg,
                st.chain.chain_id(),
                &issuer_clone,
                &s.purpose,
                &s.mode,
            );
            ok(json!({
                "sessionId": s.session_id,
                "relayer": s.relayer,
                "purpose": s.purpose,
                "recordType": s.record_type,
                "challenge": s.challenge,
                "mode": s.mode,
                "unverifiedClaims": serde_json::to_value(&claims).expect("ConvenienceClaims serializes"),
            }))
        }
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

/// POST /verify/consent/levelb — the LEVEL-B, owner-hidden submit path (M-3).
///
/// Added ALONGSIDE [`verify_consent_submit`] (Level-A), which stays untouched and keeps serving the
/// shipped phone consumers until the M-4 cutover.
///
/// Operator-gated, matching the Level-A precedent: the on-chain gates decide whether a proof is
/// VALID, but the relayer pays gas to find out, so an ungated endpoint would be a free way to drain
/// the relayer's balance on proofs that revert. Note this is only a spend gate — it grants no
/// authority over the submission itself. The proof names its own submitter (`pub[2]`, bound into both
/// the consent message and the nullifier), so an operator cannot direct a proof anywhere the owner
/// did not already consent to.
async fn verify_consent_levelb_submit(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    // Cold operator call: no session to bind to, so a fresh owner-blind audit row is minted.
    crate::verify::consent_submit_levelb(&st, &body, None).await
}

/// POST /v1/verify/consent/levelb — the OWNER'S PHONE alias for the Level-B submit path (M-4).
///
/// The Level-B twin of the `/v1/verify/consent` alias, and it exists for the same single reason: a
/// DIFFERENT gate. [`verify_consent_levelb_submit`] requires an operator session, which the phone
/// does not have; this one accepts the owner's one-time export token
/// ([`require_operator_or_export_token`]) and is therefore the route the phone can actually reach.
/// Until M-4 there was no phone Level-B path at all, so a twin would have advertised a capability it
/// lacked — that is why M-3 deliberately left it out.
///
/// # The gate is MIRRORED from Level-A, never loosened
///
///   - Same dual gate (operator session OR one-time export token), same extractor.
///   - Same PEEK semantics: `consume=false`, so a FAILED verification does not burn the owner's
///     one-time token and they can retry with the same QR. The token is consumed only when the
///     on-chain record SUCCEEDS, in the detached broadcast task — and because Level-B's broadcast is
///     likewise detached, that is the same place Level-A consumes it.
///   - Same optional `sessionId`: the phone authenticates with a token that already maps to a
///     session, so it need not echo one; the operator portal still may.
///
/// On top of that, `consent_submit_levelb` REFUSES a session whose `mode` is not `"levelb"` and binds
/// the proof's `purpose`/`relayer` to the session — the token must not fund an unrelated submission.
async fn verify_consent_levelb_submit_v1(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Resp {
    let body_token = body
        .get("exportToken")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let session_id = match body.get("sessionId").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => match body_token.as_deref() {
            Some(t) => st.store.peek_export_token(t).await.unwrap_or_default(),
            None => String::new(),
        },
    };
    if let Err(e) =
        require_operator_or_export_token(&st, &headers, &session_id, body_token.as_deref(), false)
            .await
    {
        return e;
    }
    // An operator bearer satisfies the gate without naming a session; that is the cold path, and it
    // mints its own row exactly as the operator-gated route does. Only a token-resolved session binds.
    let session = if session_id.is_empty() {
        None
    } else {
        st.store.get_session(&session_id).await
    };
    crate::verify::consent_submit_levelb(&st, &body, session).await
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
        require_operator_or_export_token(&st, &headers, &session_id, q.token.as_deref(), false)
            .await
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

/// GET /verify/history — operator-gated verifier audit log. This intentionally stores verifier-side
/// operational proof metadata (purpose, mode, relayer, tx/nullifier), not credential PII.
async fn verify_history(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let verifications: Vec<Value> = st
        .store
        .list_sessions()
        .await
        .into_iter()
        .map(|s| {
            let explorer_url = s.tx_hash.as_deref().map(crate::chain::explorer_tx_url);
            json!({
                "sessionId": s.session_id,
                "relayer": s.relayer,
                "purpose": s.purpose,
                "recordType": s.record_type,
                "mode": s.mode,
                "status": s.status,
                "txHash": s.tx_hash,
                "explorerUrl": explorer_url,
                "nullifier": s.nullifier,
                "createdAt": s.created_at,
                "updatedAt": s.updated_at,
            })
        })
        .collect();
    ok(json!({ "verifications": verifications }))
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
    /// OPTIONAL protocol version to prove for (e.g. `"dogtag-levela/1"`).
    ///
    /// Absent ⇒ the version this service loaded, which is the current Level-A one unless configured
    /// otherwise — so every pre-M7 caller keeps working unchanged. A version this service did not
    /// load is refused, never silently proven with the artifacts it has.
    #[serde(default)]
    version: Option<String>,
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
async fn prove_verification(
    State(st): State<AppState>,
    Json(body): Json<ProveVerificationReq>,
) -> Resp {
    // Resolve the artifact version BEFORE touching the witness: a request naming no version gets the
    // one this service loaded (back-compat — the pre-M7 body has no `version` field), and a request
    // naming one this service cannot serve is refused rather than proven with the wrong key. The
    // proof would verify against the wrong VK, so answering is worse than refusing (M7 §7.4).
    if let Err(e) = st.prover.accepts_version(body.version.as_deref()) {
        return err(StatusCode::BAD_REQUEST, &format!("prover: {e}"));
    }

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
    let circuit_input_json = match dogtag_standard::prover_assemble::assemble_circuit_input(
        &wrapped_doc_json,
        &consent_json,
        &sig,
    ) {
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

/// `POST /prove-consent` body — the Level-B consent proving request (M7 P0).
///
/// Unlike `/prove-verification` (which assembles the witness server-side from `{wrappedDoc, consent,
/// eddsaSig}`), the consent witness cannot be assembled without the owner's wallet SEED, and the seed
/// must never leave the device. So the device assembles the consent circuit input LOCALLY (cheap
/// field math that runs fine on 32-bit ARM, via `consent_assemble` / the `prove_consent` FFI's
/// assembly step) and POSTs the assembled `circuitInput`; only the heavy Groth16 prove runs here.
///
/// The seed staying on-device is NOT owner-unlinkability against this service: the assembled
/// `circuitInput` carries `ownerSecret` + `ownerAddress`. See the per-adversary trust-boundary note on
/// `ConsentProver::prove` (`prover.rs`) before describing this route's privacy.
#[cfg(feature = "prover")]
#[derive(Deserialize)]
struct ProveConsentReq {
    /// The PRE-ASSEMBLED `DogTagConsent(6)` circuit input (the shape
    /// `dogtag_standard::consent_assemble::consent_circuit_input_value` emits): scalars as decimal
    /// strings, the three `*Siblings` signals as length-6 arrays.
    #[serde(rename = "circuitInput")]
    circuit_input: Value,
    /// OPTIONAL protocol version. If named it MUST be the Level-B consent version; a different one is
    /// refused (fail-closed) rather than proven with the consent key. Absent ⇒ the consent version.
    #[serde(default)]
    version: Option<String>,
}

/// `POST /prove-consent` — the TRUSTED CONSENT PROVER SERVICE (M7 P0).
///
/// Selects the version-keyed CONSENT artifact set and generates a Groth16 proof for the frozen
/// `consent.circom` from the device-assembled `circuitInput`. The consent prover is loaded LAZILY on
/// the first request (from `CIRCUITS_BUILD_DIR`) and is FAIL-CLOSED PER REQUEST: a missing/hash-
/// mismatched artifact set errors THIS request (503), never boot — so an instance serving Level-A
/// `/prove-verification` coexists with `/prove-consent` without either blocking the other. Returns
/// the Solidity calldata `{a, b, c, pub}` with `pub` in the frozen OUTPUT order `[dogTagId, purpose,
/// relayer, nullifier, R, recordType, deadline]`.
#[cfg(feature = "prover")]
async fn prove_consent(State(st): State<AppState>, Json(body): Json<ProveConsentReq>) -> Resp {
    // The consent route serves ONLY the Level-B consent version; naming another is refused up front.
    if let Some(v) = body.version.as_deref() {
        if v != st.consent_prover.version() {
            return err(
                StatusCode::BAD_REQUEST,
                &format!(
                    "prover: unsupported version {v:?} (this route serves {:?})",
                    st.consent_prover.version()
                ),
            );
        }
    }

    match st.consent_prover.prove(body.circuit_input).await {
        Ok(proof) => ok(json!({
            "a": proof.a,
            "b": proof.b,
            "c": proof.c,
            "pub": proof.pub_signals,
        })),
        // A malformed device-assembled circuitInput is a CLIENT error — 400, not a 5xx a client
        // would retry (mirrors the Level-A route's in-route assemble 400).
        Err(crate::prover::ProverError::BadInput(m)) => {
            err(StatusCode::BAD_REQUEST, &format!("consent prover: {m}"))
        }
        // No consent artifacts on THIS instance (or they failed to load) — fail closed per request.
        Err(crate::prover::ProverError::Unavailable(m)) => {
            err(StatusCode::SERVICE_UNAVAILABLE, &format!("consent prover: {m}"))
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, &format!("consent prover: {e}")),
    }
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
///
/// Public so integration tests resolve the SBT under the SAME key the mint route uses (the field
/// element), rather than the raw handle — keeping the test bound to production behaviour.
pub fn onchain_dog_tag_id(handle: &str) -> Result<String, String> {
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
    // Allocate a dogTagId that is NOT already taken on EITHER SBT. The local counter resets on
    // restart and the SBTs are shared across issuers, so a fresh counter can collide with an
    // already-minted id. The two levels retire an id by different markers and BOTH must be consulted
    // while they run side by side:
    //   Level-A `DogTagSBT.mint`  -> writes an owner; `owner_of` is Err(NotFound) when free.
    //   Level-B `mintCustodial`   -> writes NO owner the allocator can see (the holder is the neutral
    //     custodian), and retires the id through the write-once `profileRoot[id]`, a marker that
    //     SURVIVES a burn — so an id consumed by a Level-B issuance reads as free via `owner_of`.
    // Getting this wrong is expensive on the Level-B path: `issue(R)` runs BEFORE the mint and
    // `registerRoot` is globally write-once, so a collision burns both the operator's one-time QR and
    // the device-computed `R`.
    // None on a Level-A-only deployment (unset/zero), which skips the Level-B leg entirely.
    let level_b = &st.cfg.sbt_consent_addr;
    let level_b_sbt = valid_contract_addr(level_b).then(|| level_b.clone());
    let mut dog_tag_id = st.store.next_dog_tag_id().await.to_string();
    for _ in 0..256 {
        // Both SBTs key state by the field-hashed id, so the collision-check must query
        // field_of_value(handle), not the raw handle.
        let onchain = match onchain_dog_tag_id(&dog_tag_id) {
            Ok(v) => v,
            Err(_) => break, // non-numeric handle can't collide via this path; proceed
        };
        let taken = match st.chain.owner_of(&st.cfg.sbt_addr, &onchain).await {
            Ok(_) => true,                                    // minted on Level-A -> taken
            Err(crate::chain::ChainError::NotFound) => false, // unminted -> free so far
            Err(_) => break, // transient RPC error: proceed (a real dup would revert at mint)
        };
        let taken = taken
            || match &level_b_sbt {
                // A real node returns 0x0..0 for an id that was never sealed; MemChain returns
                // NotFound. Both mean free — only a non-zero root retires the id.
                Some(sbt) => match st.chain.profile_root_of(sbt, &onchain).await {
                    Ok(r) => is_nonzero_word(&r),
                    Err(crate::chain::ChainError::NotFound) => false,
                    Err(_) => break,
                },
                None => false,
            };
        if !taken {
            break;
        }
        dog_tag_id = st.store.next_dog_tag_id().await.to_string();
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
                measured_on: w
                    .get("measuredOn")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
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
            // Unset at start: the session does not yet know WHICH path will redeem it. The device
            // chooses by picking an endpoint - `/profiles/issue/bind` (Level-A) or
            // `/profiles/issue/custodial-bind` (Level-B) - and that handler stamps the version.
            protocol_version: None,
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
        Some(s) => {
            // M7 P4 (§5.2): the CONVENIENCE tier — platform-OWNED, UNVERIFIED claims, additive to the
            // existing fields. Issuance has no verify-purpose, so the purpose is the record type
            // (`DOG_PROFILE`) — the namespace the app independently knows for this flow (never
            // fabricated). The issuer clone is this deployment's DOG_PROFILE documentStore. The app
            // validates these against the dogtag anchor before trusting them.
            let claims = app::convenience_claims(
                &st.cfg,
                st.chain.chain_id(),
                &st.cfg.profile_document_store,
                app::DOG_PROFILE,
            );
            ok(json!({
                "sessionId": s.session_id,
                "dogTagId": s.dog_tag_id,
                "status": s.status,
                // the device signs `register_message(walletAddress)` = "DogTag wallet registration: <lc>".
                "registrationMessagePrefix": "DogTag wallet registration: ",
                "unverifiedClaims": serde_json::to_value(&claims).expect("ConvenienceClaims serializes"),
            }))
        }
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
    if wallet.len() != 42
        || !wallet.starts_with("0x")
        || !wallet[2..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return err(
            StatusCode::BAD_REQUEST,
            "walletAddress must be a 0x.. 20-byte address",
        );
    }
    // recover the EIP-191 signer of the canonical message; require it == walletAddress.
    let message = auth::register_message(&wallet);
    let recovered = match auth::recover_personal_sign(&message, &body.signature) {
        Some(a) => a,
        None => return err(StatusCode::BAD_REQUEST, "malformed signature"),
    };
    if !recovered.eq_ignore_ascii_case(&wallet) {
        return err(
            StatusCode::UNAUTHORIZED,
            "signature does not match walletAddress",
        );
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    // consume the one-time token atomically (second call -> 404/410).
    let session_id = match st.store.take_bind_token(&body.token).await {
        Some(id) => id,
        None => {
            return err(
                StatusCode::GONE,
                "bind token missing, expired or already used",
            )
        }
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
    // Stamp the axis this tag is bound to. This path is and stays Level-A (M-2 adds the Level-B path
    // beside it; it does not flip this one).
    session.protocol_version = Some(dogtag_standard::wrap::LEVEL_A_VERSION.to_string());
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
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("dogTagId field-hash: {e}"),
            )
        }
    };
    let mint_wallet = wallet.clone();
    let mint_root = root.clone();
    let mut bg_session = session.clone();
    tokio::spawn(async move {
        match chain
            .mint(
                signer_index,
                &sbt_addr,
                &mint_wallet,
                &onchain_id,
                &mint_root,
            )
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
                    bg_session.tx_hash = Some(format!(
                        "on-chain verify failed (owner_ok={owner_ok} root_ok={root_ok})"
                    ));
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

#[derive(Deserialize)]
struct ProfileCustodialBindReq {
    token: String,
    /// The DEVICE-computed profile root `R` (0x + 64 hex). Opaque to the server.
    root: String,
}

/// Shape-check a device-supplied `R`: `0x` + 64 hex, non-zero.
///
/// This is the ONLY validation the server can do on `R`. It cannot recompute it - `R` folds the
/// owner's wallet-seed-derived secret, which the server has (and must have) no access to. The
/// non-zero check mirrors `mintCustodial`'s own `root == 0 -> BadRoot` so an obviously-bad request
/// fails at the edge instead of burning gas.
fn valid_root_hex(root: &str) -> bool {
    root.len() == 66
        && root.starts_with("0x")
        && root[2..].bytes().all(|b| b.is_ascii_hexdigit())
        && root[2..].bytes().any(|b| b != b'0')
}

/// POST /profiles/issue/custodial-bind — the **Level-B custodial issuance bridge** (M-2 / unifyplan
/// §P-1, closing datamodel D-1). Device, token-authenticated. The Level-A owner-revealing
/// `/profiles/issue/bind` above is untouched and still live; this is an ADDITIONAL path.
///
/// # What inverts versus Level-A
///
/// Level-A builds `R` on the SERVER (`wrap_vc` over a VC with the owner identity baked in) and mints
/// it to the device wallet. Level-B cannot: `R` is a profile tree folded on the DEVICE from the
/// owner's wallet seed (`apps/ios/DogTag/ProfileTreeStore.swift`), and the server has no seed. So the
/// device posts `R` and the server only ANCHORS it. Nothing here builds a VC.
///
/// # Why there is no wallet and no signature
///
/// `mintCustodial(id, root)` takes no recipient - the tag goes to the contract's immutable custodian,
/// so an owner wallet is not expressible in the calldata. Level-A's EIP-191 signature exists purely to
/// prove control of the wallet being bound; with no wallet to bind it has nothing to attest, and
/// accepting one anyway would hand the server exactly the owner link this milestone removes. The
/// authorization is therefore the **one-time bind token** alone: it is minted only by the
/// operator-gated `/profiles/issue/session/start`, is consumed atomically, and expires in 180s.
/// Whoever redeems it defines ownership through the owner-secret sealed inside `R` - the same trust
/// boundary Level-A actually had, minus the wallet disclosure.
///
/// # The two on-chain conditions (datamodel §3.5)
///
/// A Level-B verify checks `R` TWICE, against independent state
/// (`VerificationRegistryConsent.sol:163` and `:188-192`):
///   1. `R == profileRoot[dogTagId]`  — sealed by `mintCustodial`
///   2. `rootIssuer[R] != 0 && isValid(R)` — anchored by `issue(R)` on a `DogTagIssuer` clone
///
/// Doing only the first yields a tag that reverts `unknown root` on EVERY verify
/// (`contracts/test/CustodialIssuance.t.sol:367`), so this route does both, in that order, in one
/// flow. **`issue(R)` goes first** because the mint is the irreversible half: `profileRoot[id]` is
/// write-once and survives a burn, so a mint that lands before a failing `issue` retires the
/// `dogTagId` forever. The contract prescribes this ordering itself
/// (`DogTagSBTConsent.sol:139-143`: "Issue the root, then mint.").
///
/// Because `issue(R)` is itself irreversible and GLOBAL (`registerRoot` is write-once), the seal on
/// `dogTagId` is re-read here immediately before it — see the bind-time re-check below, which also
/// records why the residual cross-instance race is a tracked follow-up rather than an oversight.
///
/// Responds immediately with `status: "minting"` and runs the chain writes in the background - ROAX
/// blocks are ~12s apart, well past the phone's read timeout - exactly as the Level-A path does. The
/// operator portal polls `GET /profiles/issue/session/{id}`.
async fn profile_issue_custodial_bind(
    State(st): State<AppState>,
    Json(body): Json<ProfileCustodialBindReq>,
) -> Resp {
    let root = body.root.trim().to_lowercase();
    if !valid_root_hex(&root) {
        return err(
            StatusCode::BAD_REQUEST,
            "root must be a non-zero 0x.. 32-byte profile root",
        );
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    // Fail closed on an unconfigured OR MISCONFIGURED deployment BEFORE consuming the one-time token:
    // a half-wired Level-B stack must not burn the operator's QR (and, worse, must never mint without
    // a place to anchor the root). Both addresses are shape-checked, not merely tested for non-zero —
    // see `valid_contract_addr` for why a malformed value would otherwise reach the chain as 0x0..0.
    let sbt_addr = st.cfg.sbt_consent_addr.clone();
    let issuer_addr = st.cfg.profile_issuer_addr.clone();
    if !valid_contract_addr(&sbt_addr) || !valid_contract_addr(&issuer_addr) {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "level-B custodial issuance not configured (SBT_CONSENT_ADDR / PROFILE_ISSUER_ADDR)",
        );
    }
    // consume the one-time token atomically (second call -> 410).
    let session_id = match st.store.take_bind_token(&body.token).await {
        Some(id) => id,
        None => {
            return err(
                StatusCode::GONE,
                "bind token missing, expired or already used",
            )
        }
    };
    let mut session = match st.store.get_profile_session(&session_id).await {
        Some(s) if s.status == "pending" => s,
        Some(_) => return err(StatusCode::CONFLICT, "session already bound"),
        None => return err(StatusCode::NOT_FOUND, "session not found"),
    };

    // CANONICAL-FIELD DISCIPLINE (§P-1.3). The id sealed on-chain is `field_of_value(Integer(handle))`,
    // never the raw operator handle: the device folded that SAME field into `R` as a KDF binding
    // input, so minting under the raw handle produces `R != profileRoot(id)` and every verify fails
    // closed. The fixtures take the raw-handle shortcut and the SDK warns against copying it
    // (`profile_tree.rs` `device_root_fixture_witness`); the correct pattern is `consent_assemble.rs`,
    // which computes the field ONCE and reuses it for the circuit input, the tree KDF and the mint id.
    // Computed here so a bad handle fails synchronously, before the spawn.
    let onchain_id = match onchain_dog_tag_id(&session.dog_tag_id) {
        Ok(v) => v,
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("dogTagId field-hash: {e}"),
            )
        }
    };

    // Persist what is known before the chain writes. `wallet_address` stays None BY DESIGN - this path
    // never learns an owner wallet, and writing one would defeat the whole point.
    //
    // Scope the owner-blindness claim precisely: "owner-hidden" means hidden from DOWNSTREAM parties
    // - the chain, verifiers, relayers - NOT from the issuing authority. The session row this handler
    // updates still carries the `owner_identity` block (name, country of identification,
    // identification number) collected by the operator-gated `/profiles/issue/session/start`, and that
    // is DELIBERATE: the vet legitimately holds the identity of the person it issues to, exactly as on
    // the Level-A path. Level-B narrows who ELSE learns it; it does not ask the issuer to forget it.
    // This handler builds no verifiable credential, so it does not read that block today - a property
    // of the current stage, not surplus data: owner identity is PLANNED (not yet implemented) to be
    // committed into `R` as a hidden, selectively-disclosable leaf. See docs/DPIA.md §2.1.
    session.root = Some(root.clone());
    session.protocol_version = Some(dogtag_standard::wrap::LEVEL_B_VERSION.to_string());
    st.store.update_profile_session(session.clone()).await;

    // RE-CHECK the Level-B write-once seal HERE, synchronously, BEFORE the spawn — so it is causally
    // before `issue(R)`, the first and GLOBALLY irreversible write (`registerRoot` is write-once, so a
    // consumed `R` can never be re-anchored). `/profiles/issue/session/start` already refuses an id
    // whose `profileRoot` is set, but that check runs up to 180s earlier: without this one a collision
    // that opens in between burns `issue(R)` and only then reverts inside `mintCustodial`, destroying
    // the operator's QR, the device-computed `R` and the gas, and reporting a generic mint error. With
    // it, a detected collision refuses having written NOTHING to the chain.
    //
    // This NARROWS the window; it does not close it. Two vet instances share the SBT but each keeps
    // its own `next_dog_tag_id` counter, so the read here and the write in the spawned task remain a
    // TOCTOU. Eliminating it needs a shared-store id reservation, tracked as a follow-up; the
    // contract's write-once mint stays the real backstop.
    match st.chain.profile_root_of(&sbt_addr, &onchain_id).await {
        // A real node returns 0x0..0 for an id that was never sealed; MemChain returns NotFound.
        // Both mean free — only a non-zero root retires the id.
        Ok(r) if !is_nonzero_word(&r) => {}
        Err(crate::chain::ChainError::NotFound) => {}
        Ok(_) => {
            let msg = format!(
                "dogTagId {} is already sealed on the Level-B SBT (profileRoot is write-once and \
                 survives a burn, so this id is retired forever) — start a FRESH session",
                session.dog_tag_id
            );
            session.status = "error".to_string();
            session.tx_hash = Some(msg.clone());
            st.store.update_profile_session(session).await;
            return err(StatusCode::CONFLICT, &msg);
        }
        // Fail CLOSED on an inconclusive read: proceeding is what spends the irreversible write, and
        // the token is already consumed, so the row must be settled or the operator poll hangs.
        Err(e) => {
            let msg = format!(
                "could not confirm dogTagId {} is unsealed on the Level-B SBT: {e} — start a FRESH \
                 session",
                session.dog_tag_id
            );
            session.status = "error".to_string();
            session.tx_hash = Some(msg.clone());
            st.store.update_profile_session(session).await;
            return err(StatusCode::SERVICE_UNAVAILABLE, &msg);
        }
    }

    let chain = st.chain.clone();
    let store = st.store.clone();
    let signer_index = st.cfg.vet_signer_index;
    let bg_root = root.clone();
    let bg_id = onchain_id.clone();
    let mut bg_session = session.clone();
    tokio::spawn(async move {
        // (1) ANCHOR FIRST — issue(R) into the DogTagIssuer clone, so `rootIssuer[R]` resolves. If
        // this fails nothing has been minted and the dogTagId is still free for a retry.
        if let Err(e) = chain.issue(signer_index, &issuer_addr, &bg_root).await {
            bg_session.status = "error".to_string();
            bg_session.tx_hash = Some(format!("issue(R) error: {e}"));
            store.update_profile_session(bg_session).await;
            return;
        }
        // (2) THEN SEAL — mintCustodial(id, R). Irreversible past this point.
        let sent = match chain
            .mint_custodial(signer_index, &sbt_addr, &bg_id, &bg_root)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                bg_session.status = "error".to_string();
                bg_session.tx_hash = Some(format!("mintCustodial error: {e}"));
                store.update_profile_session(bg_session).await;
                return;
            }
        };
        // Read BOTH conditions back on-chain before calling the issuance good — a receipt is not
        // proof. Deliberately NO `owner_of` comparison: the Level-A path checks `ownerOf == wallet`,
        // but here the owner is the neutral custodian and comparing it to anything would reintroduce
        // the linkage this path exists to remove.
        let root_ok = matches!(
            chain.profile_root_of(&sbt_addr, &bg_id).await,
            Ok(r) if r.eq_ignore_ascii_case(&bg_root)
        );
        // `isValid(R)` on OUR clone is strictly stronger than `rootIssuer[R] != 0`: `issue` calls
        // `rootIndex.registerRoot(R)`, which is globally write-once ("root taken"), so a successful
        // issue on this clone proves `rootIssuer[R] == this clone`. isValid then adds not-revoked.
        let anchored_ok = matches!(chain.is_valid(&issuer_addr, &bg_root).await, Ok(true));
        if root_ok && anchored_ok {
            bg_session.status = "bound".to_string();
            bg_session.tx_hash = Some(sent.tx_hash);
        } else {
            bg_session.status = "error".to_string();
            bg_session.tx_hash = Some(format!(
                "on-chain verify failed (root_ok={root_ok} anchored_ok={anchored_ok})"
            ));
        }
        store.update_profile_session(bg_session).await;
    });

    ok(json!({
        "dogTagId": session.dog_tag_id,
        "onchainDogTagId": onchain_id,
        "root": root,
        "protocolVersion": dogtag_standard::wrap::LEVEL_B_VERSION,
        "status": "minting",
    }))
}

/// True for a well-formed, non-zero contract address (`0x` + exactly 40 hex digits, not all zero).
///
/// The zero check alone is NOT enough, and the gap is expensive. `chain::parse_addr` coerces an
/// unparseable address to `Address::ZERO` (`h.parse::<Address>().unwrap_or(Address::ZERO)`), so a
/// typo'd `SBT_CONSENT_ADDR` / `PROFILE_ISSUER_ADDR` would pass a non-zero-string test, consume the
/// one-time bind token, and dispatch both `issue(R)` and `mintCustodial` at the zero address — txs
/// that SUCCEED against a codeless address, so the failure only surfaces at the read-back, after gas
/// is spent and the operator's QR is burned. Shape-check at the edge, mirroring [`valid_root_hex`].
fn valid_contract_addr(a: &str) -> bool {
    a.len() == 42
        && a.starts_with("0x")
        && a[2..].bytes().all(|b| b.is_ascii_hexdigit())
        && a[2..].bytes().any(|b| b != b'0')
}

/// True for a 0x.. word that is not all zeros (an unset `profileRoot` reads back as 0x0..0).
fn is_nonzero_word(w: &str) -> bool {
    w.trim_start_matches("0x").bytes().any(|b| b != b'0')
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
        // Which path redeemed this session — Level-A (owner-revealing) or Level-B (owner-hidden).
        // `walletAddress` is null for the latter BY DESIGN, so the portal needs this to tell an
        // owner-hidden issuance apart from a Level-A one that failed before binding a wallet.
        "protocolVersion": s.protocol_version,
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
async fn apply_central_transition(
    st: &AppState,
    id: &str,
    body: &Value,
    default_state: &str,
) -> Resp {
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
            body.get("slot")
                .and_then(|v| v.as_str())
                .unwrap_or(&e.slot)
                .to_string(),
            body.get("state")
                .and_then(|v| v.as_str())
                .unwrap_or(default_state)
                .to_string(),
        ),
        None => (
            body.get("businessId")
                .and_then(|v| v.as_str())
                .unwrap_or(&st.cfg.business_id)
                .to_string(),
            body.get("dogTagId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            body.get("slot")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            body.get("state")
                .and_then(|v| v.as_str())
                .unwrap_or(default_state)
                .to_string(),
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
    let appts: Vec<Value> = st
        .store
        .appts_updated_since(since)
        .await
        .iter()
        .map(appt_json)
        .collect();
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
    if !matches!(
        body.event.as_str(),
        "CONFIRMED" | "DECLINED" | "COMPLETED" | "NO_SHOW"
    ) {
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

// --------------------------------------------------------------------------------------------
// Traceability portal (govarch PR-5) — this operator's own on-chain credential activity, scoped
// server-side by the oversight indexer to this business's signer/clone, re-gated by a LOCAL scope
// check, and joined to this operator's own DB records. See `crate::oversight` + `crate::trace`.
// --------------------------------------------------------------------------------------------

/// Map an oversight-feed error to an HTTP response: an unconfigured indexer is a 503 (the traceability
/// surface is simply unavailable — the rest of the backend runs), any transport/upstream error is 502.
fn feed_err(e: crate::oversight::FeedError) -> Resp {
    use crate::oversight::FeedError;
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

/// Query params for `GET /trace/activity` — narrowing filters over the SCOPED oversight feed. Each
/// only ever shrinks the result set; the indexer's server-side scope ceiling (this business's
/// signer/clone) can never be widened by a client filter, and the local gate drops anything foreign.
#[derive(Debug, Deserialize, Default)]
struct TraceParams {
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

impl From<TraceParams> for crate::oversight::FeedQuery {
    fn from(p: TraceParams) -> Self {
        crate::oversight::FeedQuery {
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

/// `GET /trace/activity` — this operator's own on-chain credential activity, joined to its own DB
/// records. The indexer scopes the feed to this business's signer/clone by its bearer; the local scope
/// gate re-checks every event server-side (defense-in-depth); the join attaches each event's matching
/// local record. Operator-gated. A vet can never fetch another vet's activity through this route.
async fn trace_activity(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(p): Query<TraceParams>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let q: crate::oversight::FeedQuery = p.into();
    let mut body = match st.feed.events(&q).await {
        Ok(b) => b,
        Err(e) => return feed_err(e),
    };
    let records = st.store.list_records().await;
    let sessions = st.store.list_sessions().await;
    let scope = crate::trace::build_scope(st.store.as_ref(), &st.cfg, &records, &sessions).await;
    let idx = crate::trace::build_index(&records, &sessions);
    let events = body
        .get("events")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let joined = crate::trace::join_events(events, Some(&scope), &idx);
    if let Value::Object(map) = &mut body {
        map.insert("events".into(), json!(joined.events));
        // `total` (from the indexer) is the scoped total; report the local page stats alongside so the
        // portal can show "N in scope, M joined to a local record".
        map.insert("inScope".into(), json!(joined.in_scope));
        map.insert("matched".into(), json!(joined.matched));
        map.insert(
            "droppedOutOfScope".into(),
            json!(joined.dropped_out_of_scope),
        );
        map.insert(
            "localScope".into(),
            json!({ "signers": scope.signers.len(), "clones": scope.clones.len() }),
        );
    }
    ok(body)
}

/// `GET /trace/stats` — this operator's in-scope on-chain counters (proxied from the indexer's scoped
/// `/v1/stats`) plus its own off-chain record/verification counts. Operator-gated.
async fn trace_stats(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let mut body = match st.feed.stats().await {
        Ok(b) => b,
        Err(e) => return feed_err(e),
    };
    let records = st.store.list_records().await;
    let sessions = st.store.list_sessions().await;
    if let Value::Object(map) = &mut body {
        map.insert(
            "local".into(),
            json!({ "records": records.len(), "verifications": sessions.len() }),
        );
    }
    ok(body)
}

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
    let prove_route = Router::new()
        .route("/prove-verification", post(prove_verification))
        .route("/prove-consent", post(prove_consent));
    #[cfg(not(feature = "prover"))]
    let prove_route = Router::<AppState>::new();

    Router::new()
        .merge(prove_route)
        // health (no auth) — used by compose healthchecks
        .route("/health", get(health))
        // login
        .route("/login", post(login))
        // settings
        .route(
            "/settings/signing-mode",
            get(get_signing_mode).put(put_signing_mode),
        )
        // credentials
        .route("/credentials/prepare", post(prepare))
        .route("/credentials/confirm", post(confirm))
        // records — list (own DB), metadata update (off-chain only), revoke (soft-invalidate), share
        .route("/records", get(list_records))
        .route("/records/:id/revoke", post(revoke))
        .route("/records/:id/share", post(share))
        .route("/records/:id", get(get_record).patch(update_record_meta))
        // short one-time share token resolver (unauthenticated; consumed on first read)
        .route("/r/:token", get(get_shared))
        // short one-time EXPORT token resolver (unauthenticated; NON-consuming — consume on submit)
        .route("/x/:token", get(export_session_resolve))
        // DOG_PROFILE (SBT) issuance — vet issues dog tags
        .route(
            "/profiles/issue/session/start",
            post(profile_issue_session_start),
        )
        .route(
            "/profiles/issue/session/:id",
            get(profile_issue_session_status),
        )
        .route("/profiles/issue/bind", post(profile_issue_bind))
        // Level-B (M-2) owner-hidden issuance, ALONGSIDE the Level-A route above - not replacing it.
        .route(
            "/profiles/issue/custodial-bind",
            post(profile_issue_custodial_bind),
        )
        // short one-time bind token resolver (unauthenticated; NON-consuming — consume on bind)
        .route("/p/:token", get(profile_bind_resolve))
        // discovery signed-manifest fallback (M7 §5.1 1B) — the dogtag-signed version manifest an app
        // verifies OFFLINE. A NEW route, distinct from the resolve GET (`/p/`, `/x/`); on any conflict
        // the on-chain ProtocolRegistry (1C) wins. UNAUTHENTICATED (public discovery data).
        .route("/protocol/manifest", get(crate::protocol::get_manifest))
        // issuer signers
        .route("/issuer/signers", get(issuer_signers))
        // import
        .route("/import/pull", post(import_pull))
        // traceability portal (govarch PR-5): this operator's own on-chain activity joined to its DB
        .route("/trace/activity", get(trace_activity))
        .route("/trace/stats", get(trace_stats))
        // verify
        .route("/verify/session/start", post(export_session_start))
        .route("/verify/credential", post(verify_credential))
        .route("/verify/session/:id", get(verify_session_status))
        .route("/verify/history", get(verify_history))
        .route("/verify/consent/submit", post(verify_consent_submit))
        // Level-B (M-3), owner-hidden. A SEPARATE route from the Level-A one above, not a mode flag
        // on it: the two carry different public-signal orders and target different registries, so a
        // request routed to the wrong one would encode for the wrong selector and revert empty.
        .route("/verify/consent/levelb", post(verify_consent_levelb_submit))
        .route("/v1/verify/credential", post(verify_credential))
        // alias so the owner's phone can POST consent+proof directly to the groomer host.
        .route("/v1/verify/consent", post(verify_consent_submit))
        // M-4: the Level-B twin of the alias above, and it exists for the same reason — a DIFFERENT
        // gate (`require_operator_or_export_token`), which is what lets the phone POST directly.
        // M-3 deliberately omitted it because Level-B had no phone path; M-4 adds the path, so the
        // twin now has the reason to exist that it lacked. The gate is mirrored, not loosened.
        .route(
            "/v1/verify/consent/levelb",
            post(verify_consent_levelb_submit_v1),
        )
        // calendar sync (Phase 7, §3.6)
        .route("/calendar/google/connect", get(google_connect))
        .route("/calendar/google/callback", get(google_callback))
        .route("/calendar/sync", post(calendar_sync))
        // appointment replica (Phase 7, §3.7) — inbound from central (HMAC) + business-driven actions
        .route("/v1/appointments/:id", put(put_appointment))
        .route("/v1/appointments/:id/cancel", post(cancel_appointment))
        .route(
            "/v1/appointments/:id/reschedule",
            post(reschedule_appointment),
        )
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

// Behavior-preserving unit tests for the pure request-parsing/validation free helpers
// (client_ip / bearer / onchain_dog_tag_id / is_terminal). Mirrors admin-api routes.rs coverage.
#[cfg(test)]
mod tests {
    use super::*;

    fn headers_from(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    fn peer(s: &str) -> Option<SocketAddr> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn client_ip_prefers_first_forwarded_hop_trimmed() {
        let h = headers_from(&[("x-forwarded-for", "  203.0.113.7 , 10.0.0.1 , 10.0.0.2")]);
        // first hop wins, whitespace trimmed, peer ignored when XFF present
        assert_eq!(client_ip(&h, peer("198.51.100.9:443")), "203.0.113.7");
    }

    #[test]
    fn client_ip_empty_forwarded_falls_through_to_peer() {
        // a present-but-empty XFF value is filtered, so the socket peer is used instead of ""
        let h = headers_from(&[("x-forwarded-for", "")]);
        assert_eq!(client_ip(&h, peer("198.51.100.9:443")), "198.51.100.9");
    }

    #[test]
    fn client_ip_no_headers_no_peer_defaults_unknown() {
        let h = HeaderMap::new();
        assert_eq!(client_ip(&h, None), "unknown");
        // peer-only path (no XFF) returns the peer ip without the port
        assert_eq!(client_ip(&h, peer("127.0.0.1:8080")), "127.0.0.1");
    }

    #[test]
    fn bearer_requires_exact_scheme_and_extracts_token() {
        assert_eq!(
            bearer(&headers_from(&[("authorization", "Bearer op_abc123")])),
            Some("op_abc123".to_string())
        );
        // scheme is case-sensitive and the trailing space is part of the prefix
        assert_eq!(
            bearer(&headers_from(&[("authorization", "bearer op_abc123")])),
            None
        );
        assert_eq!(
            bearer(&headers_from(&[("authorization", "Bearertoken")])),
            None
        );
        // empty token after the scheme is returned verbatim (empty string), and absent header -> None
        assert_eq!(
            bearer(&headers_from(&[("authorization", "Bearer ")])),
            Some(String::new())
        );
        assert_eq!(bearer(&HeaderMap::new()), None);
    }

    #[test]
    fn onchain_dog_tag_id_is_deterministic_and_canonically_shaped() {
        // field_of_value Poseidon-hashes the encoded integer (not a raw interpretation), so the
        // output is opaque; pin the contract structurally rather than couple to a Poseidon constant.
        let a = onchain_dog_tag_id("123456789").unwrap();
        let b = onchain_dog_tag_id("123456789").unwrap();
        assert_eq!(a, b, "same handle -> same hashed id");
        assert!(a.starts_with("0x"));
        assert_eq!(
            a.len(),
            66,
            "0x + 64 hex nibbles = canonical 32-byte field element"
        );
        assert!(a[2..].chars().all(|c| c.is_ascii_hexdigit()));
        // distinct handles produce distinct ids
        assert_ne!(
            onchain_dog_tag_id("1").unwrap(),
            onchain_dog_tag_id("2").unwrap()
        );
    }

    #[test]
    fn onchain_dog_tag_id_rejects_non_integer_handle() {
        // a non-numeric handle fails the Integer field decode rather than panicking
        assert!(onchain_dog_tag_id("not-a-number").is_err());
        assert!(onchain_dog_tag_id("").is_err());
    }

    #[test]
    fn is_terminal_matches_only_the_four_terminal_states() {
        for s in ["DECLINED", "CANCELLED", "COMPLETED", "NO_SHOW"] {
            assert!(is_terminal(s), "{s} should be terminal");
        }
        for s in ["CONFIRMED", "PENDING", "declined", "no_show", ""] {
            assert!(!is_terminal(s), "{s} should not be terminal");
        }
    }
}
