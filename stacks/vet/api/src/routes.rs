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
//!     GET  /issuer/signers
//!     POST /import/pull
//!     POST /verify/session/start | /verify/consent/submit
//!   admin router (custody — mounted SEPARATELY; /admin/* requires the admin session):
//!     POST /admin/genesis/start | /admin/genesis/confirm | /admin/unlock | /admin/accounts

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{self, AppState};
use crate::auth::{self, ShareClaims, VerifyClaims};
use crate::store::{IssuerSettings, Record, RecordStatus, VerifySession};

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

async fn login(State(st): State<AppState>, Json(body): Json<LoginReq>) -> Resp {
    if body.password != st.cfg.operator_password {
        return err(StatusCode::UNAUTHORIZED, "bad password");
    }
    let token = auth::new_op_token();
    st.store.put_op_session(token.clone()).await;
    ok(json!({ "token": token }))
}

async fn admin_login(State(st): State<AppState>, Json(body): Json<LoginReq>) -> Resp {
    if body.password != st.cfg.admin_password {
        return err(StatusCode::UNAUTHORIZED, "bad password");
    }
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

async fn unlock(State(st): State<AppState>, headers: HeaderMap, Json(body): Json<UnlockReq>) -> Resp {
    if let Err(e) = require_admin(&st, &headers).await {
        return e;
    }
    let blob = match st.store.get_custody().await {
        Some(b) if b.meta.state == "initialized" => b,
        _ => return err(StatusCode::CONFLICT, "not initialized"),
    };
    let phrase = match crate::custody::decrypt_seed(&blob.encrypted_seed, &body.passphrase) {
        Ok(p) => p,
        Err(_) => return err(StatusCode::UNAUTHORIZED, "wrong passphrase"),
    };
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
            "unsignedTx": { "to": issuer_addr, "data": calldata, "value": 0, "chainId": 135 }
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
    if view.chain_id != Some(crate::chain::ROAX_CHAIN_ID) {
        return Err(err(StatusCode::BAD_REQUEST, "tx.chainId != 135"));
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
    let n = auth::now();
    let jti = uuid::Uuid::new_v4().to_string();
    let claims = ShareClaims {
        iss: st.cfg.deployment_url.clone(),
        sub: id.clone(),
        aud: "dogtag-mobile".to_string(),
        scope: "read:record".to_string(),
        iat: n,
        nbf: n,
        exp: n + 180,
        jti,
    };
    let token = auth::sign_jwt(&st.jwt, &claims);
    let qr = format!("{}/r?t={}&i={}", st.cfg.deployment_url, token, id);
    ok(json!({ "qrUrl": qr }))
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
}

async fn verify_consent_submit(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ConsentSubmitReq>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    crate::verify::consent_submit(&st, body.session_id, body.consent, body.sig, body.mode, body.disclosed_doc).await
}

// --------------------------------------------------------------------------------------------
// router assembly
// --------------------------------------------------------------------------------------------

/// The single combined router (public + admin). Admin routes carry their own admin-session gate.
pub fn router(state: AppState) -> Router {
    Router::new()
        // login
        .route("/login", post(login))
        .route("/admin/login", post(admin_login))
        // admin custody (admin-session gated inside handlers)
        .route("/admin/genesis/start", post(genesis_start))
        .route("/admin/genesis/confirm", post(genesis_confirm))
        .route("/admin/unlock", post(unlock))
        .route("/admin/accounts", post(accounts))
        // settings
        .route("/settings/signing-mode", get(get_signing_mode).put(put_signing_mode))
        // credentials
        .route("/credentials/prepare", post(prepare))
        .route("/credentials/confirm", post(confirm))
        // records
        .route("/records/:id/revoke", post(revoke))
        .route("/records/:id/share", post(share))
        .route("/records/:id", get(get_record))
        // issuer signers
        .route("/issuer/signers", get(issuer_signers))
        // import
        .route("/import/pull", post(import_pull))
        // verify
        .route("/verify/session/start", post(verify_session_start))
        .route("/verify/consent/submit", post(verify_consent_submit))
        .with_state(state)
}
