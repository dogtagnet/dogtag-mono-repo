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
//!     GET  /x/{token}                                 (short-lived EXPORT token — UNAUTHENTICATED)
//!     POST /appointments/{id}/share                   -> mint the CLIENT calendar handoff
//!     GET  /a/{token}[.ics]                           (per-appointment client handoff: page + `.ics`
//!                                                      — UNAUTHENTICATED, NON-consuming; see
//!                                                      `appointment_share.rs`)
//!     GET  /issuer/signers
//!     POST /import/pull
//!     POST /verify/credential                         -> direct credential validity/revocation check
//!     POST /verify/session/start | /v1/verify/consent       (owner-hidden EXPORT flow)
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
use crate::chain::{GrantAtIssuance, IssuanceCapability};
use crate::issuance_allowed::RosterRead;
use crate::microchip::{MicrochipCheck, NotComparable as MicrochipNotComparable};
use crate::store::{ApptReplica, Record, RecordStatus, VerifySession};
use crate::verify::valid_contract_addr;

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
///
/// On an issuance-enabled role it also reports whether the owner-hidden dog-tag ANCHOR contracts
/// are configured (`dogTagIssuance`), so the portal's Register pet screen can refuse IN PLACE
/// instead of letting the owner's phone discover the custodial-bind 503 after the operator has
/// already allocated a tag and drawn a QR. Config facts only — no chain read; `ready` claims the
/// addresses are configured, never that the contracts work. A groomer carries NO block (it mounts
/// no issuance routes), so a consumer must read an ABSENT block as could-not-check, never as
/// either verdict.
async fn health(State(st): State<AppState>) -> Resp {
    let mut body = json!({ "status": "ok" });
    if st.cfg.issuance_enabled() {
        // `mintRole` is the ONE chain fact beside the config facts: `mintCustodial` is
        // `onlyRole(ISSUER_ROLE)` and a fresh SBT grants that role to nobody, so a stack can be
        // fully configured and still unable to mint a single tag — measured live 2026-08-07, where
        // the refusal surfaced as a silent estimation revert inside the bind's background task.
        // Bounded (3s) so a slow node can never hang the liveness probe; on elapse the answer is
        // "unknown", which the portal renders as could-not-check and never as either verdict.
        let mint_role = mint_role_gate(&st).await;
        body["dogTagIssuance"] = json!({
            "ready": st.cfg.dog_tag_anchor_refusal().is_none(),
            "profileIssuerConfigured": valid_contract_addr(&st.cfg.profile_issuer_addr),
            "sbtConsentConfigured": valid_contract_addr(&st.cfg.sbt_consent_addr),
            "mintRole": mint_role.wire_state(),
            "mintRoleDetail": mint_role.detail(),
        });
    }
    ok(body)
}

/// The three-way answer to "may the ACTIVE SIGNER `mintCustodial` on the configured SBT?".
///
/// Three states because the two non-held cases have different licenses: only a DEFINITE `Missing`
/// may refuse anything, while `Unknown` (locked custody, unconfigured SBT, a failed or timed-out
/// read) is could-not-check — it warns on the portal and never blocks, and the bind's own
/// background arms report the real failure if one comes.
enum MintRoleGate {
    Held,
    Missing(String),
    Unknown(String),
}

impl MintRoleGate {
    fn wire_state(&self) -> &'static str {
        match self {
            MintRoleGate::Held => "held",
            MintRoleGate::Missing(_) => "missing",
            MintRoleGate::Unknown(_) => "unknown",
        }
    }
    fn detail(&self) -> Option<&str> {
        match self {
            MintRoleGate::Held => None,
            MintRoleGate::Missing(d) | MintRoleGate::Unknown(d) => Some(d),
        }
    }
}

/// The operator-vocabulary refusal for a signer the SBT would refuse. The remedy is a BUTTON, not
/// a command: the admin portal's Providers page carries the "Dog-tag mint role" card that grants
/// exactly this role (captain's ruling 2026-08-07 — no cast command in the operator's path).
fn mint_role_refusal_message(signer: &str) -> String {
    format!(
        "the vet signing key {signer} does not hold the dog-tag mint role — DogTagSBTConsent's \
         ISSUER_ROLE, which every mint is gated on — so minting reverts before broadcasting. On \
         the ADMIN portal's Providers page, use the \"Dog-tag mint role\" card to grant it to \
         this signer, then retry"
    )
}

/// Resolve the mint-role gate: active signer -> `hasRole(ISSUER_ROLE, signer)` on the configured
/// SBT, bounded to 3s. Asked by `/health` (so the portal can refuse where the operator acts), by
/// session-start and retry (refuse BEFORE a QR exists), and by the bind (refuse BEFORE the
/// one-time token is consumed).
async fn mint_role_gate(st: &AppState) -> MintRoleGate {
    if !valid_contract_addr(&st.cfg.sbt_consent_addr) {
        return MintRoleGate::Unknown(
            "the DogTagSBTConsent address is not configured, so the mint role cannot be checked"
                .to_string(),
        );
    }
    let signer =
        match st.custody.active_address() {
            Ok(a) => a,
            Err(_) => return MintRoleGate::Unknown(
                "custody is locked, so the signing key's address cannot be resolved to check the \
                 mint role"
                    .to_string(),
            ),
        };
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        st.chain
            .sbt_issuer_role_held(&st.cfg.sbt_consent_addr, &signer),
    )
    .await
    {
        Ok(Ok(true)) => MintRoleGate::Held,
        Ok(Ok(false)) => MintRoleGate::Missing(mint_role_refusal_message(&signer)),
        Ok(Err(e)) => MintRoleGate::Unknown(format!(
            "could not check whether the vet signing key holds the dog-tag mint role: {e}"
        )),
        Err(_) => MintRoleGate::Unknown(
            "could not check whether the vet signing key holds the dog-tag mint role: the chain \
             read timed out"
                .to_string(),
        ),
    }
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

/// Require a valid operator session bearer token. `pub(crate)` so the CRM handlers in `crm.rs`
/// gate on the exact same operator session as every route here.
pub(crate) async fn require_operator(st: &AppState, headers: &HeaderMap) -> Result<(), Resp> {
    let token =
        bearer(headers).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing operator session"))?;
    if st.store.has_op_session(&token).await {
        Ok(())
    } else {
        Err(err(StatusCode::UNAUTHORIZED, "invalid operator session"))
    }
}

/// Dual gate for the verify/export consent/status endpoints: authorize EITHER a valid operator
/// session OR a valid short-lived EXPORT TOKEN bound to `session_id`. The export token is the
/// low-density QR token (16 random bytes hex) minted at session start.
/// The token may arrive as `export_token` (request field) or
/// as the `?token=` query / `Authorization: Bearer` value. It is validated to map to `session_id`.
/// The token is only peeked; the session's persisted status is the replay guard. Returns `Ok(true)`
/// if authorized via an export token, `Ok(false)` if via operator session.
async fn require_operator_or_export_token(
    st: &AppState,
    headers: &HeaderMap,
    session_id: &str,
    body_token: Option<&str>,
) -> Result<bool, Resp> {
    // Operator session first (portal + scripts/e2e-smoke.sh): an op_ bearer satisfies the gate.
    if let Some(token) = bearer(headers) {
        if st.store.has_op_session(&token).await {
            return Ok(false);
        }
    }
    // Otherwise try a short-lived export token (the owner's phone). Accept it from the body field or the
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
    let mapped = st.store.peek_export_token(&token).await;
    let mapped =
        mapped.ok_or_else(|| err(StatusCode::UNAUTHORIZED, "export token missing or expired"))?;
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
    // Read-modify-write: settings is one document, and switching the signing mode must not silently
    // drop a sibling field (the published calendar-feed secret would revoke every subscription).
    let mut settings = st.store.get_settings().await;
    settings.signing_mode = body.mode.clone();
    st.store.put_settings(settings).await;
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
                &format!(
                    "no issuer contract is configured for recordType {} — this backend cannot \
                     anchor that record type until its issuer clone address is configured (for \
                     VACCINATION: set VACCINATION_ISSUER_ADDR and restart the backend)",
                    body.record_type
                ),
            )
        }
    };
    // A PRESENT but blank/malformed address must be refused HERE with its cause named, never passed
    // into a chain read: `chain::parse_addr` coerces it to the zero address, whose `registry()`
    // eth_call answers empty returndata, and the decode failure then surfaces as
    // "preflight: rpc: ABI decoding failed: buffer overrun while deserializing" — a config hole
    // reading as a chain fault (measured on a live walk, 2026-08-07: `demo-up.sh` exports
    // `VACCINATION_ISSUER_ADDR=` empty-but-set when no clone is configured).
    if !valid_contract_addr(&issuer_addr) {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!(
                "the issuer contract address configured for recordType {} is blank or malformed, \
                 so this backend cannot anchor this record on-chain — configure the real issuer \
                 clone address (for VACCINATION: VACCINATION_ISSUER_ADDR) and restart the backend",
                body.record_type
            ),
        );
    }
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
        protocol_version: Some(dogtag_standard::wrap::LEVEL_B_VERSION.to_string()),
        verification_registry: Some(st.cfg.verification_registry_consent_addr.clone()),
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
    // PRESENT TENSE, deliberately: "may this signer anchor a new root right now". Asked
    // service-scoped against the clone we are about to write to, so the authority is that clone's
    // own `registry()` and this preflight refuses exactly what the write would refuse — on either
    // generation, with no configured registry address involved. See
    // `ChainClient::issuance_capability`.
    match st
        .chain
        .issuance_capability(&issuer_addr, &signer_addr)
        .await
    {
        Ok(IssuanceCapability::Authorized) => {}
        Ok(IssuanceCapability::NotAuthorized) => {
            return err(
                StatusCode::FORBIDDEN,
                "address not approved for this recordType yet",
            )
        }
        // Could not determine is neither verdict. Refusing the issuance is right — we will not spend
        // gas on a write we cannot show will land — but it must not be reported as the signer's
        // fault, which is what the FORBIDDEN arm above says.
        Ok(IssuanceCapability::Undetermined) => {
            return err(
                StatusCode::BAD_GATEWAY,
                "preflight: could not determine issuance authority for this issuer contract",
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
    // Authorized WHEN IT ANCHORED — not "authorized now".
    //
    // This is the same defect #127 fixed in the verification pillar, reached from the other side. The
    // transaction under confirmation has already mined, and `DogTagIssuer.issue` is `onlyWhitelisted`,
    // so the chain itself already established this signer's authority at that moment. Asking the
    // current-state getter here therefore cannot make the check stronger, and it can make it wrong:
    // delisting is forward-only, so a signer rotated between broadcast and confirm — an ordinary key
    // rotation, or the C-12 cutover freeze itself — would see its own genuine, mined issuance
    // rejected, with the record stranded in `Prepared` and no way to advance it.
    //
    // Asking the historical question also makes this path generation-correct for free, since
    // `whitelisted_at_issuance` resolves the authority off the clone rather than off configuration.
    match st
        .chain
        .whitelisted_at_issuance(&issuer_addr, &signer, &r.root)
        .await
    {
        Ok(GrantAtIssuance::Authorized) => {}
        Ok(GrantAtIssuance::NotAuthorized) => {
            return Err(err(
                StatusCode::FORBIDDEN,
                "signer was not whitelisted when this root was anchored",
            ))
        }
        // Never a pass and never an accusation. The remaining binding checks below still have to
        // hold, so a confirm is not waved through on an unanswerable authority.
        Ok(GrantAtIssuance::Undetermined) => {
            return Err(err(
                StatusCode::BAD_GATEWAY,
                "whitelist: could not determine the issuing signer's authority at anchoring",
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
    // Issuance-eligibility matrix across the configured record types. Same present-tense question as
    // the prepare preflight, asked the same service-scoped way, so the console cannot report a signer
    // as able to issue where the preflight would refuse it.
    //
    // `whitelisted` is now TRI-state on the wire: `true` / `false` / `null`. It was a bare bool
    // defaulting to `false` on any read failure, which stated "this signer is not approved" on the
    // strength of a read that never happened — an operator's own RPC blip rendered as a permissions
    // problem, and the very collapse this slice exists to remove. Consumers must branch on `null`
    // first; `issuer/signers` is operator-gated and read-only, so no gate depends on it.
    let mut matrix = Vec::new();
    for (rt, issuer_addr) in st.cfg.issuer_addrs.iter() {
        let wl = match st.chain.issuance_capability(issuer_addr, &active).await {
            Ok(IssuanceCapability::Authorized) => Some(true),
            Ok(IssuanceCapability::NotAuthorized) => Some(false),
            Ok(IssuanceCapability::Undetermined) | Err(_) => None,
        };
        matrix.push(json!({ "recordType": rt, "address": active, "whitelisted": wl }));
    }
    ok(json!({ "activeSigner": active, "matrix": matrix }))
}

// --------------------------------------------------------------------------------------------
// issuance-allowed roster — LAYER 2 of the two-layer issuance requirement
// --------------------------------------------------------------------------------------------

/// `GET /issuer/issuance-allowed` — for each contract this deployment issues through: who owns it,
/// which addresses its own issuance list has an opinion about, and whether OUR custody signer is
/// among them.
///
/// # Why this is a READ route and there is deliberately no write one
///
/// `DogTagIssuer.setIssuanceAllowed` admits only from the clone's `owner()`, and this backend is not
/// it — its custody signer is the address that needs admitting, which is the entire gap. Nor could a
/// route stand in for the owner: an operator session proves "staff of this shop", never "owner of
/// this contract", and this crate carries no signature-recovery path over an arbitrary address. A
/// backend that could admit would have to hold owner authority, which is precisely the second layer
/// collapsing back into the first. The write is a wallet transaction from the owner's own key.
///
/// # The one field that answers the question providers actually ask
///
/// `activeSignerAllowed`. Layer 1 has a screen and layer 2 had none, so "I am approved, I deployed my
/// contract, and issuing still fails" had no diagnosis anywhere in the product. This says which of
/// the two is missing, per contract.
///
/// Operator-gated and read-only, like `issuer/signers` beside it; nothing gates on it.
async fn issuance_allowed_roster(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    // Locked custody has no active address. That is `None`, never a zero address and never an
    // omitted key — "we do not know which signer this deployment uses" and "this signer may not
    // issue" are different sentences, and the page prints them differently.
    let active = st.custody.active_address().ok();

    // EVERY contract this deployment anchors through, both kinds. `PROFILE_ISSUER_ADDR` is a real
    // `DogTagIssuer` clone reached by `POST /profiles/issue/custodial-bind`, so it needs layer 2
    // exactly as the record-type clones do — and it is the half most likely to be forgotten, because
    // completing a dog-tag bind needs the phone app and so is rarely walked on a desktop.
    let mut configured: Vec<(String, String)> = st
        .cfg
        .issuer_addrs
        .iter()
        .map(|(rt, a)| (rt.clone(), a.clone()))
        .collect();
    if !st.cfg.profile_issuer_addr.is_empty() {
        configured.push((
            "DOG_PROFILE".to_string(),
            st.cfg.profile_issuer_addr.clone(),
        ));
    }
    // Deterministic order so two loads render the same list.
    configured.sort_by(|a, b| a.0.cmp(&b.0));

    let mut contracts = Vec::new();
    for (record_type, issuer_addr) in configured {
        // An unset address is the ZERO address here (see `main.rs`), and reading a contract that does
        // not exist would report a configuration hole as a chain fault. Omit it: this route describes
        // the contracts this deployment issues through, and an unconfigured one is not among them.
        if !valid_contract_addr(&issuer_addr) {
            continue;
        }
        let read = match st
            .chain
            .issuance_allowed_roster(&issuer_addr, active.as_deref())
            .await
        {
            Ok(roster) => {
                // Built FIRST, then asked - so the headline goes through `RosterRead::allowed`, the
                // same accessor a consumer would use, rather than a second inline copy of the fold.
                // Two implementations of one rule is what lets them drift, and it also leaves the
                // accessor's own tests pinning nothing that ships.
                let mut read = RosterRead::Resolved {
                    owner: crate::issuance_allowed::normalize_addr(&roster.owner),
                    entries: roster.entries,
                    active_signer_allowed: None,
                };
                let answered = active.as_deref().and_then(|a| read.allowed(a));
                if let RosterRead::Resolved {
                    active_signer_allowed,
                    ..
                } = &mut read
                {
                    *active_signer_allowed = answered;
                }
                read
            }
            // A read that failed is NOT an empty list. `Unavailable` carries no entries field at all,
            // so no consumer can spread it into one.
            Err(e) => RosterRead::Unavailable {
                reason: e.to_string(),
            },
        };
        contracts.push(json!({
            "recordType": record_type,
            "issuerAddr": issuer_addr,
            "read": read,
        }));
    }

    ok(json!({
        "activeSigner": active,
        "contracts": contracts,
    }))
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
        // The verdict rides ALONG WITH the error, not instead of it. A bare `{error}` here made the
        // pillar states unreachable on the only path that needs them: an operator whose import was
        // refused by a delisted issuer, a record-type relabel, or our own malformed `FACTORY_ADDR`
        // saw one generic message and could not tell the three apart. The `error` key is unchanged,
        // so anything already reading it keeps working.
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "third-party verify invalid",
                "verdict": crate::verify::verdict_json(&verdict),
            })),
        );
    }
    // upsert client cache keyed by dogTagId.
    let dog = crate::verify::dog_tag_id_of(&doc).unwrap_or_else(|| "unknown".to_string());
    // The MICROCHIP CROSS-CHECK, the other half of the tag↔pet binding (the first is
    // `crm::link_pet_dogtag`). This is the import direction: the tag was linked first and the
    // credential arrives afterwards, so the comparison has to happen here too or the same wrong
    // pairing simply lands in the opposite order.
    //
    // A mismatch refuses the IMPORT with 409 and, critically, does NOT touch `verdict`. The
    // credential verified; it is genuine and stays valid for everyone. What it does not do is
    // describe the animal this shop has on file under that tag — a different accusation with a
    // different remedy, kept apart for the same reason `issuer_mismatch` is kept apart from
    // `issuer_not_whitelisted`. Folding it into `valid` would report a real credential as forged.
    let credential_chip = crate::microchip::credential_microchip(&doc);
    // A credential this build cannot read the microchip OUT OF is decided here, before the pet
    // lookup and ahead of every pet-side fact — `compare` applies the same precedence for the rows
    // below. Without it the loud state would be swallowed by `NoLinkedPet` on exactly the imports
    // that arrive before a tag is linked, which is the ordinary order for this route.
    //
    // ONE lookup drives both the refusal and the reported state. Two would straddle a concurrent
    // edit and could refuse on one pet's code while reporting another's.
    let (check, holder) = if let Some(loud) = crate::microchip::unrunnable(&credential_chip) {
        (loud, None)
    } else {
        match st.store.try_find_pets_by_dog_tag(&dog).await {
            Ok(pets) if pets.is_empty() => (
                // No pet on file carries this tag, so there is no local record to compare against —
                // reported as its own reason rather than as a passing check.
                MicrochipCheck::NotComparable(MicrochipNotComparable::NoLinkedPet),
                None,
            ),
            // The pet whose code MISMATCHES decides, if there is one; otherwise the first pet on the
            // tag supplies the reported state. Uniqueness is enforced at every write, so this list
            // is normally one row — but it was never enforced retroactively over rows written before
            // that rule, so preferring the mismatch means a pre-existing duplicate cannot hide one.
            Ok(pets) => {
                let mut checked: Vec<_> = pets
                    .into_iter()
                    .map(|r| {
                        let c = crate::microchip::compare(
                            r.pet.microchip_code.as_deref(),
                            &credential_chip,
                        );
                        (c, Some(r))
                    })
                    .collect();
                let i = checked.iter().position(|(c, _)| c.refuses()).unwrap_or(0);
                checked.swap_remove(i)
            }
            // An unreadable pet store does NOT refuse the import. The import is not a claim about
            // any pet — it files a verified credential under its own tag — and the link routes run
            // this same check against a store they were able to read before any binding is written.
            // Refusing here would block ordinary work over a lookup the operator never asked for.
            // But it is REPORTED as a could-not-read rather than silently omitted, so the skipped
            // check stays visible.
            Err(e) => {
                tracing::warn!(
                    error = %e, dog_tag_id = %dog,
                    "microchip cross-check could not read the pet records on import"
                );
                (
                    MicrochipCheck::NotComparable(MicrochipNotComparable::CouldNotRead(
                        e.to_string(),
                    )),
                    None,
                )
            }
        }
    };
    if check.refuses() {
        let holder = holder
            .map(|r| {
                format!(
                    " This shop has that DogTag on file for {} ({}).",
                    r.pet.name, r.client_name
                )
            })
            .unwrap_or_default();
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("{}{holder}", check.refusal_message().unwrap_or_default()),
                "microchipCheck": check.to_json(),
                // The verdict rides along UNCHANGED, so the operator can see the credential itself
                // passed every pillar and that what was refused is the PAIRING.
                "verdict": crate::verify::verdict_json(&verdict),
            })),
        );
    }
    st.store.upsert_client_cache(dog, doc_val).await;
    // Emitted in ALL THREE states — including every `notComparable` reason — so an absent key can
    // never read as a check that passed.
    ok(json!({
        "imported": true,
        "verdict": crate::verify::verdict_json(&verdict),
        "microchipCheck": check.to_json(),
    }))
}

// --------------------------------------------------------------------------------------------
// verify (impl §3.9)
// --------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct VerifyCredentialReq {
    /// The wrapped credential document to verify (as produced by any DogTag issuer).
    #[serde(rename = "wrappedDoc", alias = "wrapped_doc")]
    wrapped_doc: WrappedDoc,
    /// OPTIONAL *expected* DogTagIssuer clone. It can only TIGHTEN: the clone every verdict-deciding
    /// read is made against is the one the FACTORY names for this root, so this asserts an
    /// expectation rather than selecting a contract. It used to select one — and an operator able to
    /// nominate the contract that answers for a credential is the same forgery this pillar exists to
    /// close, reached through a second field.
    #[serde(rename = "issuerAddr", alias = "issuer_addr", default)]
    issuer_addr: Option<String>,
    /// OPTIONAL *expected* issuing signer. The issuer-whitelist pillar no longer depends on this: the
    /// signer is resolved from the chain (`issuedBy(root)`). Supplying it adds a strictly stronger
    /// assertion — the pillar fails when the on-chain originator is not this address.
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
    let claimed_root = doc.signature.merkle_root.clone();

    // WHICH contract issued this credential. `issuer.documentStore` is only the document's CLAIM: the
    // `issuer` block sits OUTSIDE the Merkle root, so an attacker can point it at a contract they
    // deployed and it will answer `isValid` however they like. The factory's write-once `rootIssuer[R]`
    // (`DogTagIssuerFactory.sol:19`, writable only from inside an `isClone` contract's `issue()`) is
    // authoritative, and it names the clone that issued THIS root.
    // Whether THIS deployment can evaluate the factory-anchored pillar at all — and, crucially, an
    // ABSENT factory is distinguished from a MALFORMED one. The classification is
    // `verify::factory_config`, shared with the SDK adapter that serves `POST /import/pull`, so the
    // two surfaces cannot drift on what counts as "no factory" versus "a broken one".
    //
    // Absent (unset, or the zero address) is a deployment that deliberately has no factory: the pillar
    // reports itself unavailable and does not condemn the credential. A MALFORMED value is different
    // in kind — that deployment INTENDED to check. Folding it into "no factory configured" would
    // silently convert an intent-to-check into a no-check, which is the misconfigure-to-bypass path
    // this pillar's explicit states exist to prevent, and a fat-fingered address is a likelier
    // mistake than a deliberately absent one. So it fails LOUDLY, as a configuration fault, and
    // returns no verdict at all rather than a fail-open one. The REACTION is this handler's own: the
    // SDK has no 500 channel and reports the same classification as an indeterminate pillar.
    let factory_cfg = match crate::verify::factory_config(&st.cfg.factory_addr) {
        crate::verify::FactoryConfig::Absent => None,
        crate::verify::FactoryConfig::Malformed => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::verify::FACTORY_ADDR_MALFORMED,
            )
        }
        crate::verify::FactoryConfig::Addr(a) => Some(a),
    };
    let factory_configured = factory_cfg.is_some();
    // THREE states, not two — and the distinction is the whole design.
    //   `noFactoryConfigured` = we never asked. That is OUR misconfiguration, not evidence about the
    //                           credential, so it must not condemn it.
    //   `noRecord`            = we DID ask and the chain has no record of this root. That IS evidence.
    // A READ FAILURE is neither, and is answered with 502 rather than folded into either: this handler
    // already refuses to turn an unreachable node into a verdict, and an anchor read is no different.
    let (resolved_issuer, issuer_resolution) = match factory_cfg.as_deref() {
        None => (None, "noFactoryConfigured"),
        Some(factory) => match st.chain.root_issuer(factory, &claimed_root).await {
            Ok(Some(a)) => (Some(a), "resolved"),
            Ok(None) => (None, "noRecord"),
            Err(e) => {
                return err(
                    StatusCode::BAD_GATEWAY,
                    &format!("on-chain rootIssuer read failed: {e}"),
                )
            }
        },
    };
    // The address every verdict-deciding read below is made against. The factory's answer wins; the
    // document's own claim is the LAST resort, reached only when the factory has no record of the root
    // (or none is configured) — and in the first of those cases the mandatory pillar below is
    // unresolved, so nothing read from that contract can produce a pass.
    //
    // Note what is NOT in this chain any more: `body.issuer_addr`. An operator override used to sit at
    // the front of it, which meant a caller could nominate the contract that vouches for the very
    // credential they submitted. It is now an assertion (`expected_clone_differs`), never a selector.
    let issuer_addr = resolved_issuer
        .clone()
        .unwrap_or_else(|| doc.issuer.document_store.clone());
    // The document names a different contract than the one the chain says issued this root. An ABSENT
    // `documentStore` is a mismatch like any other: exempting it would buy nothing (the factory
    // supplies the address regardless) while letting a caller strip one field to skip the check.
    let issuer_store_differs = resolved_issuer
        .as_deref()
        .map(|r| !r.eq_ignore_ascii_case(doc.issuer.document_store.trim()))
        .unwrap_or(false);
    // The caller's expected-clone assertion, which can only TIGHTEN. An assertion that could NOT be
    // evaluated — no clone resolved, so there is nothing authoritative to compare against — is
    // reported as `notEvaluated` rather than collapsed into the same `false` a satisfied assertion
    // produces. A caller must always be able to tell "checked and held" from "never checked"; that is
    // the same rule the pillar's own explicit states exist for, applied to the caller's own claim.
    let want_clone = body
        .issuer_addr
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let expected_clone_state = match (resolved_issuer.as_deref(), want_clone) {
        (_, None) => "notAsserted",
        (Some(actual), Some(want)) if actual.eq_ignore_ascii_case(want) => "matched",
        (Some(_), Some(_)) => "differs",
        (None, Some(_)) => "notEvaluated",
    };
    let expected_clone_differs = expected_clone_state == "differs";

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

    // The issuer-whitelist pillar — MANDATORY, and SELF-RESOLVING.
    //
    // This used to run only when an operator typed a signer in, and to default to a PASS when they did
    // not (`issuer_whitelisted.unwrap_or(true)`). Combined with reading `isValid` off the document's
    // own `documentStore`, that made a forged credential verify: deploy a contract answering
    // `issuedAt != 0` / `isValid = true` / `isRevoked = false`, point `issuer.documentStore` at it, and
    // the one pillar that would have objected simply never ran.
    //
    // It now asks the chain who issued the root — `issuedBy`, set to `msg.sender` under
    // `onlyWhitelisted` (`DogTagIssuer.sol:40,55`) — and asks whether THAT signer held the capability
    // AT THE MOMENT it anchored this root. Never an address named by the document, or the attacker
    // supplies both sides of the question.
    //
    // The authority is the GOVERNING registry — the address the resolved clone's own `registry()`
    // answers — never this deployment's separately-configured `ISSUER_REGISTRY_ADDR`. `IssuerRegistry`
    // `_wl` and its `Whitelisted`/`Delisted` events are per-CONTRACT, so a grant history read from any
    // other instance is a confident answer about a different mapping: a merely mis-paired deployment
    // would find no grant and print a definite refusal of a genuine credential, which is our own
    // misconfiguration rendered as an accusation. It opens no trust surface — `registry` is written
    // once in `initialize` from the factory's own immutable, on a clone THIS deployment's factory
    // resolved, so it is as anchored as the clone itself.
    //
    // Tri-state, and only a definite `true` may contribute to a pass:
    //   Some(true)  — resolved, and authorised for this record type at the anchoring point
    //   Some(false) — resolved, and it was not (or not the expected signer): a real failure
    //   None        — unresolvable: INDETERMINATE, and never a pass
    let rt_key = app::rt_key(&record_type);
    let want_signer = body
        .signer_addr
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let (resolved_signer, chain_whitelisted) = match resolved_issuer.as_deref() {
        // EVERY read here is made against the FACTORY-RESOLVED clone. Using `issuer_addr` would let the
        // document's own claim answer the question about itself whenever the factory had no record —
        // asking the suspect for its own references.
        Some(clone) if factory_configured => {
            match st.chain.issued_by(clone, &claimed_root).await {
                Err(e) => {
                    return err(
                        StatusCode::BAD_GATEWAY,
                        &format!("on-chain issuedBy read failed: {e}"),
                    )
                }
                // The clone the factory named never issued this root: indeterminate, never a pass.
                Ok(None) => (None, None),
                Ok(Some(signer)) => {
                    // Ask about the record type the CLONE says it issues, not the one the envelope
                    // claims: otherwise a credential relabelled across record types is checked against
                    // the wrong whitelist key, and a signer authorised for one type appears authorised
                    // for another.
                    match st.chain.issuer_record_type(clone).await {
                        Err(e) => {
                            return err(
                                StatusCode::BAD_GATEWAY,
                                &format!("on-chain recordType read failed: {e}"),
                            )
                        }
                        Ok(None) => (Some(signer), None),
                        Ok(Some(chain_rt_key)) if !chain_rt_key.eq_ignore_ascii_case(&rt_key) => {
                            (Some(signer), Some(false))
                        }
                        Ok(Some(_)) => {
                            // ...and about the moment this root was ANCHORED, not about now.
                            // Delisting is forward-only (`DogTagIssuer.sol:82`; `adminRevoke` is the
                            // retroactive lever), so `isWhitelistedFor` — a current-state getter —
                            // refuses every credential a since-rotated, retired or lapsed signer ever
                            // issued, fleet-wide, while the protocol says each one is genuine. The
                            // answer is reconstructed from the governing registry's own
                            // `Whitelisted`/`Delisted` logs, so any verifier with an RPC reproduces it.
                            match st
                                .chain
                                .whitelisted_at_issuance(clone, &signer, &claimed_root)
                                .await
                            {
                                Ok(GrantAtIssuance::Authorized) => (Some(signer), Some(true)),
                                Ok(GrantAtIssuance::NotAuthorized) => (Some(signer), Some(false)),
                                // The reads succeeded but there was no anchoring point or authority
                                // to sequence against: INDETERMINATE. Not a pass, and not an
                                // accusation either.
                                Ok(GrantAtIssuance::Undetermined) => (Some(signer), None),
                                Err(e) => {
                                    return err(
                                        StatusCode::BAD_GATEWAY,
                                        &format!("on-chain grant-history read failed: {e}"),
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
        // No factory configured, or the factory has no record of this root. Neither can be resolved,
        // and neither is a pass.
        _ => (None, None),
    };

    // The caller's expected-signer assertion, folded here rather than inside the resolved branch so
    // that it is evaluated on EVERY path. It used to live in the innermost arm, which meant an
    // explicit operator assertion was silently DISCARDED whenever the clone could not be resolved —
    // exactly the "a check stopped running and nothing said so" defect this whole pillar exists to
    // remove, and it landed hardest on deployments with no factory, where the assertion was the only
    // check left.
    //
    // It may only ever TIGHTEN: it can turn a pass into a definite failure and can NEVER manufacture
    // a pass. That asymmetry is what lets it stay live on an unresolvable clone without reopening the
    // hole — an unresolved pillar stays unresolved no matter how agreeable the assertion is.
    //
    //   matched / differs      — the chain named the originator, so the claim is decidable.
    //   unanchoredUnevaluable  — no originator to compare against AND no authority that can be asked
    //                            about this signer. Reported, never folded into a failure.
    //
    // WHY THE FACTORY-LESS BRANCH CAN NO LONGER REFUSE, stated plainly because it is a deliberate
    // narrowing rather than an oversight. Every issuance-axis read on the authority is
    // SERVICE-SCOPED — `canIssue(service, signer)` takes a service address — and this branch is
    // DEFINED by there being no resolved clone to pass. So there is no service to ask about, and the
    // one selector that takes no service (`isWhitelistedFor`) answers off the orthogonal VERIFY axis:
    // handed a record-type key it returns a confident `false` for every genuine issuer signer, which
    // would condemn each of them. Supplying `documentStore` to satisfy the signature is strictly
    // worse — it hands the attacker the choice of which contract answers, the exact substitution the
    // anchor exists to close.
    //
    // So the assertion is REPORTED and gates nothing. That is a real loss of a check, and the honest
    // one: we cannot ask, and an inability to check may never be stated as a verdict about the
    // subject. It is bounded to a factory-less deployment, and `FACTORY_ADDR` is what restores the
    // anchor — and with it the historical question the pillar asks everywhere else.
    let expected_signer_state = match (resolved_signer.as_deref(), want_signer) {
        (_, None) => "notAsserted",
        (Some(actual), Some(want)) if want.eq_ignore_ascii_case(actual) => "matched",
        (Some(_), Some(_)) => "differs",
        // No clone resolved, so there is no service to ask the authority about. Reported as
        // unevaluable rather than guessed at in either direction.
        (None, Some(_)) => "unanchoredUnevaluable",
    };
    let signer_assertion_failed = expected_signer_state == "differs";
    let issuer_whitelisted = if signer_assertion_failed {
        Some(false)
    } else {
        chain_whitelisted
    };

    // Only a DEFINITE `true` contributes to a pass. `unwrap_or(true)` is deliberately gone: a pillar
    // that could not be resolved is indeterminate, never a pass.
    //
    // The one exception is our OWN misconfiguration: with no factory configured we never asked, which
    // is not evidence about the credential and must not fail it. That state is reported explicitly
    // (`issuerWhitelistState`), never as a silent absence, because misconfigure-to-bypass would
    // otherwise be a real attack path. The `is_none()` term is load-bearing: "we never asked" is only
    // the unresolved case, so a DEFINITE failure the caller's assertion produced must still fail the
    // credential on a factory-less deployment. Without it, unavailable would swallow it.
    let issuer_pillar_ok =
        issuer_whitelisted == Some(true) || (!factory_configured && issuer_whitelisted.is_none());
    let verdict = integrity_valid
        && onchain_valid
        && issuer_pillar_ok
        && !issuer_store_differs
        && !expected_clone_differs;

    // `status` is the field the smoke test and the portals read, so it has to move with the verdict.
    // Leaving it at "valid" while the verdict was false would put "not evaluated" back on the wire as
    // "passed" — the same defect this change removes, relocated into a string.
    let status = if !integrity_valid {
        "integrity_failed"
    } else if !issued {
        "not_issued"
    } else if revoked {
        "revoked"
    } else if !onchain_valid {
        "invalid"
    } else if issuer_store_differs || expected_clone_differs {
        "issuer_mismatch"
    } else if issuer_whitelisted == Some(false) {
        "issuer_not_whitelisted"
    } else if !issuer_pillar_ok {
        "issuer_unresolved"
    } else {
        "valid"
    };

    // Why `issuerWhitelisted` is what it is, as an explicit state rather than a bare tri-state the
    // caller has to interpret. "not evaluated because this verifier has no factory configured" and
    // "evaluated and passed" must never be indistinguishable.
    //
    // A DEFINITE outcome outranks "we never asked", so `Some(false)` reads as `failed` even with no
    // factory configured: the caller's assertion genuinely was checked and genuinely did fail, and
    // reporting that as unavailable would hide a real failure behind our own gap.
    let whitelist_state = match issuer_whitelisted {
        Some(true) => "passed",
        Some(false) => "failed",
        None if !factory_configured => "unavailableNoFactoryConfigured",
        None => "unresolved",
    };

    ok(json!({
        "verdict": verdict,
        "status": status,
        "recordType": record_type,
        "root": claimed_root,
        "recomputedRoot": recomputed_hex,
        // The clone the reads were actually made against, and how it was arrived at. With
        // `issuerResolution != "resolved"` this is the document's own unverified claim, which is why
        // the fragments below cannot carry a verdict on their own.
        "issuerAddr": issuer_addr,
        "issuerResolution": issuer_resolution,
        "documentStore": doc.issuer.document_store,
        // The signer the CHAIN says issued this root, never one the caller supplied. `null` means the
        // resolved clone never issued it, which is why the pillar could not be resolved.
        "signerAddr": resolved_signer,
        // The caller's assertions, echoed so a mismatch is explainable. Neither selects a contract.
        "expectedSignerAddr": body.signer_addr,
        "expectedIssuerAddr": body.issuer_addr,
        "issuedAt": issued_at.to_string(),
        "checkedAt": auth::now(),
        "fragments": {
            "integrity": integrity_valid,
            "onchain": onchain_valid,
            "issued": issued,
            "revoked": revoked,
            "issuerWhitelisted": issuer_whitelisted,
            // Why `issuerWhitelisted` is what it is. A caller MUST be able to tell "not evaluated"
            // from "evaluated and passed" — a pillar that never ran must never read as one that
            // succeeded.
            //   "passed" | "failed" | "unresolved" | "unavailableNoFactoryConfigured"
            "issuerWhitelistState": whitelist_state,
            // The envelope names a contract the chain disagrees with, or the caller's expected-clone
            // assertion does.
            "documentStoreDiffers": issuer_store_differs,
            "expectedIssuerDiffers": expected_clone_differs,
            // What became of each of the caller's OWN assertions. A supplied assertion that could not
            // be evaluated must be distinguishable from one that was evaluated and held — the boolean
            // above spells both `false`, which is how a dropped check reads as a satisfied one.
            //   "notAsserted" | "matched" | "differs" | "notEvaluated"
            "expectedIssuerState": expected_clone_state,
            //   "notAsserted" | "matched" | "differs"
            //   | "unanchoredNotWhitelisted"  — definite failure, folded into the pillar
            //   | "unanchoredUnconfirmed"     — whitelisted, but never promotes the pillar
            "expectedSignerState": expected_signer_state,
        },
    }))
}

#[derive(Deserialize)]
struct SessionStartReq {
    purpose: String,
    #[serde(rename = "recordType")]
    record_type: String,
    /// OPTIONAL: the shop appointment this verification is being performed FOR. Supplying it links
    /// the resulting verification to that appointment and its client in the shop's history — the
    /// primary flow in the groomer portal. Omitting it yields an ad-hoc, unlinked verification, which
    /// is exactly what this endpoint did before and still does.
    #[serde(rename = "appointmentId", default)]
    appointment_id: Option<String>,
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
    // Refuse before the owner scans and spends time proving when the sole registry is unconfigured.
    if !crate::verify::valid_contract_addr(&st.cfg.verification_registry_consent_addr) {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "verification registry not configured",
        );
    }
    // Resolve the OPTIONAL appointment this verification belongs to. An id that does not resolve is
    // a 400, not a silent downgrade to an unlinked verification: the operator asked for the linked
    // flow and must not be told it succeeded when the linkage was dropped.
    let ctx = match body
        .appointment_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        Some(id) => match crate::crm::resolve_session_context(&st.store, id).await {
            Ok(c) => Some(c),
            Err(e) => return err(StatusCode::BAD_REQUEST, &e),
        },
        None => None,
    };
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut challenge = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut challenge);
    let challenge_hex = format!("0x{}", hex::encode(challenge));
    let now = auth::now();
    let session = VerifySession {
        session_id: session_id.clone(),
        relayer: relayer.clone(),
        purpose: body.purpose,
        record_type: body.record_type,
        challenge: challenge_hex,
        status: "pending".to_string(),
        tx_hash: None,
        nullifier: None,
        created_at: now,
        updated_at: now,
        disclosed_key_paths: Vec::new(),
        appointment_id: ctx.as_ref().map(|c| c.appointment_id.clone()),
        client_id: ctx.as_ref().and_then(|c| c.client_id.clone()),
        pet_id: ctx.as_ref().and_then(|c| c.pet_id.clone()),
    };
    st.store.put_session(session.clone()).await;
    // Open the history row now, so an in-flight (and an abandoned) verification is still visible in
    // "All verifications" rather than only appearing if it completes.
    crate::crm::start_log(&st.store, &session, ctx.as_ref()).await;
    // Mint a short-lived EXPORT token (32 hex chars == 16 random bytes) so the QR is low-density
    // and symmetric with the import `/r/<token>` flow. The server maps token -> export session
    // as a session-scoped capability. The QR carries {host, token, relayer address}; the phone resolves session
    // metadata via GET /x/<token>. A recorded/error session status prevents successful replay.
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    let token = hex::encode(bytes);
    // 10-minute TTL: the export path includes a slow on-device Groth16 proof (tens of seconds) plus the
    // owner's manual steps, so a 180s token can expire mid-flow. The session state and on-chain
    // nullifier independently prevent replay.
    let exp = auth::now() + 600;
    st.store.put_export_token(&token, &session_id, exp).await;
    let qr = format!("{}/x/{}?a={}", st.cfg.deployment_url, token, relayer);
    ok(json!({ "qrUrl": qr, "sessionId": session_id }))
}

/// GET /x/{token} — resolve a short-lived EXPORT token to the export session metadata the phone
/// needs ({ sessionId, relayer, purpose, recordType, challenge }). Unauthenticated and
/// non-consuming. A missing or expired token is a 404.
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
            let claims =
                app::convenience_claims(&st.cfg, st.chain.chain_id(), &issuer_clone, &s.purpose);
            ok(json!({
                "sessionId": s.session_id,
                "relayer": s.relayer,
                "purpose": s.purpose,
                "recordType": s.record_type,
                "challenge": s.challenge,
                "unverifiedClaims": serde_json::to_value(&claims).expect("ConvenienceClaims serializes"),
            }))
        }
        None => err(StatusCode::NOT_FOUND, "session not found"),
    }
}

/// POST /v1/verify/consent — submit an owner-hidden proof. An operator bearer may submit a cold proof;
/// an owner's phone authenticates with the export token for its session.
async fn verify_consent_submit(
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
    let authed_by_export_token =
        match require_operator_or_export_token(&st, &headers, &session_id, body_token.as_deref())
            .await
        {
            Ok(v) => v,
            Err(e) => return e,
        };
    // An operator bearer satisfies the gate without naming a session; that is the cold path, and it
    // mints its own row exactly as the operator-gated route does. Only a resolved session binds.
    //
    // A named session that does not load FAILS CLOSED. Falling through to `None` would silently
    // demote the request to the cold path — skipping the purpose/relayer/recordType binding AND the
    // status replay guard — and `MongoStore::get_session` maps driver errors to
    // `None` (`.ok().flatten()`), so a transient DB blip would otherwise disable every session-scoped
    // guard on this route for that request, including its only replay protection.
    let session = if session_id.is_empty() {
        None
    } else {
        match st.store.get_session(&session_id).await {
            Some(s) => Some(s),
            // Token-authed: the gate already matched this token to this session id, so the row not
            // loading is a BACKEND fault (a store read that failed), not a bad request.
            None if authed_by_export_token => {
                return err(
                    StatusCode::BAD_GATEWAY,
                    "session could not be read; retry with the same token",
                )
            }
            // Operator-authed with an explicit sessionId that does not exist: the operator named a
            // session, so honour that rather than quietly minting an unrelated cold row.
            None => return err(StatusCode::NOT_FOUND, "session not found"),
        }
    };
    crate::verify::consent_submit_levelb(&st, &body, session).await
}

#[derive(Deserialize)]
struct SessionStatusQuery {
    /// the short-lived export token (the owner's phone polling) — non-consuming peek. The operator
    /// portal omits this and relies on the Bearer operator-session instead.
    #[serde(default)]
    token: Option<String>,
}

/// GET /verify/session/{sessionId} — operator-gated status read so the portal's VerifyFlow can poll
/// pending -> recorded. Returns the stored session's status and (once recorded) the txHash +
/// nullifier. `nullifier` is exposed when present in the session row (ZK path); null otherwise.
async fn verify_session_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(q): Query<SessionStatusQuery>,
) -> Resp {
    // Dual gate: operator session OR a short-lived export token (the owner's phone polling). No consume
    // — status reads are idempotent and polled repeatedly (peek only). The token arrives via the
    // `?token=` query or the Bearer header.
    if let Err(e) =
        require_operator_or_export_token(&st, &headers, &session_id, q.token.as_deref()).await
    {
        return e;
    }
    let s = match st.store.get_session(&session_id).await {
        Some(s) => s,
        None => return err(StatusCode::NOT_FOUND, "session not found"),
    };
    ok(json!({
        "status": s.status,
        "txHash": s.tx_hash,
        "nullifier": s.nullifier,
    }))
}

/// GET /verify/history — operator-gated verifier audit log. This intentionally stores verifier-side
/// operational proof metadata (purpose, relayer, tx/nullifier), not credential PII.
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
// OWNER-HIDDEN CONSENT PROVING API. Gated behind the `prover`
// feature; the route is mounted only when compiled with `--features prover`.
// --------------------------------------------------------------------------------------------

/// `POST /prove-consent` body — the owner-hidden consent proving request.
///
/// The consent witness cannot be assembled without the owner's wallet seed, and the seed must never
/// leave the device. The device therefore assembles the consent circuit input locally (cheap
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
    /// OPTIONAL protocol version. If named it MUST be the unified consent version; a different one
    /// is refused (fail-closed) rather than proven with the consent key. Absent ⇒ that version.
    #[serde(default)]
    version: Option<String>,
}

/// `POST /prove-consent` — the TRUSTED CONSENT PROVER SERVICE (M7 P0).
///
/// Selects the version-keyed CONSENT artifact set and generates a Groth16 proof for the frozen
/// `consent.circom` from the device-assembled `circuitInput`. The consent prover is loaded LAZILY on
/// the first request (from `CIRCUITS_BUILD_DIR`) and is FAIL-CLOSED PER REQUEST: a missing/hash-
/// mismatched artifact set errors THIS request (503), never boot. Returns the Solidity calldata
/// `{a, b, c, pub}` with `pub` in the frozen output order `[dogTagId, purpose, relayer, nullifier, R,
/// recordType, deadline]`.
#[cfg(feature = "prover")]
async fn prove_consent(State(st): State<AppState>, Json(body): Json<ProveConsentReq>) -> Resp {
    // The consent route serves only the unified consent version; naming another is refused up front.
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
            "pub": proof.public_signals,
        })),
        // A malformed device-assembled circuitInput is a client error — 400, not a retryable 5xx.
        Err(crate::prover::ProverError::BadInput(m)) => {
            err(StatusCode::BAD_REQUEST, &format!("consent prover: {m}"))
        }
        // No consent artifacts on THIS instance (or they failed to load) — fail closed per request.
        Err(crate::prover::ProverError::Unavailable(m)) => err(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("consent prover: {m}"),
        ),
        Err(e) => err(StatusCode::BAD_GATEWAY, &format!("consent prover: {e}")),
    }
}

// --------------------------------------------------------------------------------------------
// DOG_PROFILE issuance — the operator starts a session showing a QR; the device scans and posts its
// owner-hidden root; the vet anchors the root and mints the neutral-custody SBT.
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
/// `pub[0]` and the contract's `profileRoot` key. The DOG_PROFILE SBT MUST be minted under this
/// (not the raw numeric handle), so the owner's later ZK export passes `profileRoot(pub[0])`. The raw handle
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

/// D1 identity-leaf keyPaths - the sanctioned `owner.identity.*` namespace
/// (`profile_tree::OWNER_IDENTITY_PREFIX`; q4 §1.1). One leaf per attribute, never a blob: leaf
/// granularity IS disclosure granularity, and `R` is write-once.
pub const KP_IDENTITY_FULL_NAME: &str = "owner.identity.fullName";
pub const KP_IDENTITY_COUNTRY: &str = "owner.identity.country";
pub const KP_IDENTITY_DOC_NUMBER: &str = "owner.identity.docNumber";

/// Build the session's D1 identity attribute leaves from the operator-collected identity block:
/// one leaf per NON-BLANK field, each salted with a fresh vet-generated 16-byte salt.
///
/// The VET generates the salts (not the device): that is what lets the bind-time integrity gate
/// (`verify_leaf_commitment`) require the device's posted `owner.identity.*` openings to EXACTLY
/// equal the vet's OWN retained `{keyPath, salt, value}` set while rebuilding the posted `R` from
/// the full leaf list - the property that makes the identity genuinely VET-ATTESTED rather than
/// device-asserted. High-entropy salts also keep low-entropy values (a country has ~200
/// possibilities) unguessable behind the public root.
fn identity_leaves_for(identity: &crate::store::OwnerIdentity) -> Vec<crate::store::IdentityLeaf> {
    let mut leaves = Vec::new();
    let mut push = |key_path: &str, value: &str| {
        if value.trim().is_empty() {
            return; // an absent leaf is unprovable; an empty one would be provably empty
        }
        let mut salt = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut salt);
        leaves.push(crate::store::IdentityLeaf {
            key_path: key_path.to_string(),
            salt_hex: format!("0x{}", hex::encode(salt)),
            tag: dogtag_standard::types::TypeTag::String as u8,
            value: value.to_string(),
        });
    };
    push(KP_IDENTITY_FULL_NAME, &identity.name);
    push(KP_IDENTITY_COUNTRY, &identity.country_of_identification);
    push(KP_IDENTITY_DOC_NUMBER, &identity.identification);
    leaves
}

/// How long a freshly drawn Register-pet QR waits to be SCANNED (mint -> first `GET /p/{token}`).
///
/// What this deadline protects: the QR is a bearer capability — whoever resolves it reads the
/// vet-collected owner identity, and whoever binds it defines the tag's owner — so a leaked QR
/// (photo, screen share) must stop being redeemable once the co-present ceremony is plausibly
/// over. Ten minutes still guarantees that; the old 180s did not survive the ceremony itself: it
/// started at MINT, so it also had to cover the owner walking over, opening the app, possibly
/// creating a wallet, and scanning — none of which the operator controls. Measured live
/// 2026-08-07: a legitimate run died at 180s with nothing minted.
const BIND_TOKEN_SCAN_TTL_SECS: u64 = 600;

/// Once a device HAS picked the session up (first `GET /p/{token}`), the token is guaranteed to
/// live at least this much longer, so the scan-window clock can never kill a session mid-bind.
///
/// The deadline moves to `max(current, resolve + this)` — never shortened, so an early pickup
/// keeps the full scan window (re-scans and a wallet-creation detour stay covered), and a pickup
/// near the scan deadline still gets the whole bind window: read the screen, tap, authenticate
/// with biometrics, fold the profile tree, POST. Worst-case token life is SCAN + BIND ≈ 15
/// minutes, still inside the co-present ceremony. Only the FIRST resolve extends — repeated polls
/// of `/p/` must not keep a token alive indefinitely — and the token stays strictly one-time: the
/// bind still consumes it atomically, and an unresolved or overrun token still dies.
const BIND_TOKEN_BIND_TTL_SECS: u64 = 300;

/// POST /profiles/issue/session/start — operator-session gated. Allocate a dogTagId, persist a
/// ProfileIssueSession with a fresh 16-byte one-time bind token (scan TTL above), and return the QR
/// URL `<deployment_url>/p/<token>` the device scans. Returns `{ token, dogTagId, sessionId, qr,
/// ttlSecs, qrAddress }`.
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
    // Refuse BEFORE allocating a tag or minting a QR token. The bind route's own fail-closed check
    // (the point of use, below) is what protects the chain — but by the time it fires a second
    // human has already scanned a QR for an issuance this backend knew at boot it could not
    // complete, and the refusal surfaces on the owner's phone, where nobody can act on it. This is
    // the same predicate the bind route applies, asked where the OPERATOR is.
    if let Some(refusal) = st.cfg.dog_tag_anchor_refusal() {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("cannot start dog-tag issuance: {refusal}"),
        );
    }
    // The SBT mint role, same placement and same license as the anchor refusal: only a DEFINITE
    // "the SBT would refuse this signer" answer refuses (could-not-check warns on the portal and
    // never blocks — the bind's background arms report the real failure if one comes). Without
    // this, a fully configured stack whose signer was never granted ISSUER_ROLE hands a QR to a
    // second human for a mint that reverts silently at estimation (measured live, 2026-08-07).
    if let MintRoleGate::Missing(refusal) = mint_role_gate(&st).await {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("cannot start dog-tag issuance: {refusal}"),
        );
    }
    if body.pet.name.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "pet.name must not be blank");
    }
    // THE TWO-LAYER ISSUE GATE, asked BEFORE a QR exists. `DogTagIssuer.issue` requires BOTH the
    // authority's `canIssue` (the registrar's address-keyed ISSUE grant folded with the service
    // lifecycle) AND the clone's OWN `issuanceAllowed` list — and this backend knows its issuer
    // address and its signer, so nothing about the question needs a device. Without this gate the
    // portal drew a QR and had a second human scan, build and POST for an issuance the backend was
    // always going to refuse — the refusal surfaced as a 403 on the owner's phone (measured live
    // 2026-08-07: rightsOf carried no ISSUE bit AND the clone had not admitted the signer).
    //
    // A DEFINITE `false` on either layer refuses HERE, naming which half is missing and which
    // portal fixes it. Could-not-check is NOT a refusal: an unreadable chain (or a generation-1
    // clone, which has neither `canIssue` nor `issuanceAllowed` to ask) proceeds and the response
    // carries a `signerIssuance` warning instead — the bind path's own preflights still stand.
    let signer_addr = match st.custody.active_address() {
        Ok(a) => a,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let issuer = st.cfg.profile_issuer_addr.clone();
    let registry_grant = st.chain.issuance_capability(&issuer, &signer_addr).await;
    let clone_list = st.chain.issuance_allowed(&issuer, &signer_addr).await;
    let registry_missing = matches!(
        registry_grant,
        Ok(crate::chain::IssuanceCapability::NotAuthorized)
    );
    let clone_missing = matches!(clone_list, Ok(false));
    if registry_missing || clone_missing {
        let mut halves = Vec::new();
        if registry_missing {
            halves.push(
                "the DogTag registrar has not granted it the ISSUE right — ask DogTag to grant \
                 it from the admin portal's Providers page",
            );
        }
        if clone_missing {
            halves.push(
                "this clinic's own DOG_PROFILE contract has not admitted it — the contract owner \
                 admits it on this portal's Signing keys page",
            );
        }
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!(
                "cannot start dog-tag issuance: this clinic's signing key {signer_addr} is not \
                 approved to issue dog tags. {}. Nothing was allocated and no QR was drawn.",
                halves.join("; and ")
            ),
        );
    }
    let mut unchecked = Vec::new();
    if !matches!(
        registry_grant,
        Ok(crate::chain::IssuanceCapability::Authorized)
    ) {
        unchecked.push("the registrar's ISSUE grant");
    }
    if clone_list.is_err() {
        unchecked.push("the contract's own signer list");
    }
    let signer_issuance = if unchecked.is_empty() {
        json!({ "state": "authorized" })
    } else {
        json!({
            "state": "unknown",
            "detail": format!(
                "{} could not be checked from here — if the issuance later fails with \"not \
                 approved\", the registrar grant (admin portal, Providers) and this portal's \
                 Signing keys page are where to fix it",
                unchecked.join(" and ")
            ),
        })
    };
    // Allocate a dogTagId whose owner-hidden SBT profileRoot is still unset. The local counter resets
    // on restart and the SBT is shared across issuers, so a fresh counter can collide with an already
    // minted id. `mintCustodial` retires an id through the write-once `profileRoot[id]`, a marker that
    // survives a burn. Getting this wrong is expensive: `issue(R)` runs BEFORE the mint and
    // `registerRoot` is globally write-once, so a collision burns both the operator's one-time QR and
    // the device-computed `R`.
    let sbt = &st.cfg.sbt_consent_addr;
    let mut dog_tag_id = st.store.next_dog_tag_id().await.to_string();
    for _ in 0..256 {
        // The SBT keys state by the field-hashed id, so query field_of_value(handle), not the raw handle.
        let onchain = match onchain_dog_tag_id(&dog_tag_id) {
            Ok(v) => v,
            Err(_) => break, // non-numeric handle can't collide via this path; proceed
        };
        let taken = match st.chain.profile_root_of(sbt, &onchain).await {
            // A real node returns 0x0..0 for an id that was never sealed; MemChain returns NotFound.
            Ok(r) => is_nonzero_word(&r),
            Err(crate::chain::ChainError::NotFound) => false,
            Err(_) => break,
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

    // D1: one identity attribute leaf per non-blank identity field, salted fresh HERE so the vet
    // retains the exact `{keyPath, salt, value}` triples the bind-time integrity gate recomputes.
    let identity_leaves = identity_leaves_for(&owner_identity);

    let session_id = uuid::Uuid::new_v4().to_string();
    // one-time 16-byte bind token -> session; the QR carries `<deployment_url>/p/<token>`. The
    // deadline starts as the SCAN window and is extended at first resolve (see the BIND_TOKEN_*
    // constants); `token_exp` mirrors it onto the session so the status poll can report honest
    // seconds-left without holding the token.
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    let token = hex::encode(bytes);
    let exp = auth::now() + BIND_TOKEN_SCAN_TTL_SECS;
    st.store
        .put_profile_session(crate::store::ProfileIssueSession {
            session_id: session_id.clone(),
            dog_tag_id: dog_tag_id.clone(),
            owner_identity,
            identity_leaves,
            pet_name: body.pet.name,
            microchip,
            profile,
            status: "pending".to_string(),
            created_at: auth::now(),
            resolved_at: None,
            token_exp: exp,
            root: None,
            tx_hash: None,
            protocol_version: None,
            error_stage: None,
            error_reason: None,
        })
        .await;
    st.store.put_bind_token(&token, &session_id, exp).await;
    let qr = format!("{}/p/{}", st.cfg.deployment_url, token);
    ok(json!({
        "token": token,
        "dogTagId": dog_tag_id,
        "sessionId": session_id,
        "qr": qr,
        // Countdowns count from ttlSecs at response receipt (never expiresAt - Date.now(): the two
        // clocks are the backend's and the browser's, and skew shows an expired timer over a good QR).
        "ttlSecs": BIND_TOKEN_SCAN_TTL_SECS,
        // Whether THIS machine still answers at the address the QR names — the address is baked at
        // boot, so a machine that moved networks prints dead QRs with nothing else saying so.
        "qrAddress": crate::qr_address::qr_address_json(&st.cfg.deployment_url),
        // The two-layer issue gate's answer: "authorized" (both halves read true), or "unknown"
        // with what could not be checked. A definite refusal never reaches this response — it is
        // the 503 above, before anything was allocated.
        "signerIssuance": signer_issuance,
    }))
}

/// GET /p/{token} — resolve a one-time bind token to the session metadata the device needs to build
/// its owner-hidden profile root: ids/status plus the `pet` attributes the device folds into `R`,
/// the vet-collected `ownerIdentity` block, and (D1) the `identityLeaves` — the salted
/// `{keyPath, saltHex, tag, value}` identity attributes the device MUST fold into `R` alongside the
/// pet attributes (the bind-time integrity gate refuses an `R` that does not commit them). Both
/// mobile parsers fail closed without the `pet` and `ownerIdentity` containers, so
/// they are emitted unconditionally — `ownerIdentity` with empty-string fields when the operator
/// collected none (the parsers require the container, not the fields; `identityLeaves` is then
/// empty and the bind degrades to the pet-only fold). Unauthenticated
/// and NON-consuming (consumed only on bind). A missing/expired token is a 404. Symmetric with `/x/`.
/// The bind consumes the token atomically before the session can leave "pending", so a token that
/// still resolves implies the pre-bind state — the metadata needs no extra status gating.
async fn profile_bind_resolve(State(st): State<AppState>, Path(token): Path<String>) -> Resp {
    let session_id = match st.store.peek_bind_token(&token).await {
        Some(id) => id,
        None => return err(StatusCode::NOT_FOUND, "bind token missing or expired"),
    };
    match st.store.get_profile_session(&session_id).await {
        Some(mut s) => {
            // FIRST resolve: record that a device picked the session up (the portal renders
            // "nobody scanned" and "a device resolved it and went quiet" as different faults), and
            // guarantee the token outlives the device's remaining work — read the screen, tap,
            // biometrics, fold the tree, POST — by extending the deadline to at least
            // now + BIND_TOKEN_BIND_TTL_SECS. max() so an early pickup never SHORTENS the scan
            // window; first-resolve-only so repeated polls cannot keep a token alive forever.
            if s.resolved_at.is_none() {
                let now = auth::now();
                s.resolved_at = Some(now);
                s.token_exp = s.token_exp.max(now + BIND_TOKEN_BIND_TTL_SECS);
                st.store
                    .put_bind_token(&token, &session_id, s.token_exp)
                    .await;
                st.store.update_profile_session(s.clone()).await;
            }
            let s = s;
            // M7 P4 (§5.2): the CONVENIENCE tier — platform-OWNED, UNVERIFIED claims, additive to the
            // existing fields. Issuance has no verify-purpose, so the purpose is the record type
            // (`DOG_PROFILE`) — the namespace the app independently knows for this flow (never
            // fabricated). The issuer clone is this deployment's DOG_PROFILE anchor. The app
            // validates these against the dogtag anchor before trusting them.
            let claims = app::convenience_claims(
                &st.cfg,
                st.chain.chain_id(),
                &st.cfg.profile_issuer_addr,
                app::DOG_PROFILE,
            );
            ok(json!({
                "sessionId": s.session_id,
                "dogTagId": s.dog_tag_id,
                "status": s.status,
                // The session's pet record, in the nested-`pet` shape both mobile parsers target:
                // scalar profile fields + weightHistory under `pet.profile`, the microchip beside it,
                // and the name at the `pet` level — the session row's own structure. Weight values
                // stay decimal STRINGS end to end (store::WeightEntry.value).
                "pet": {
                    "name": s.pet_name,
                    "profile": serde_json::to_value(&s.profile).expect("PetProfile serializes"),
                    "microchip": serde_json::to_value(&s.microchip).expect("Microchip serializes"),
                },
                "ownerIdentity": serde_json::to_value(&s.owner_identity).expect("OwnerIdentity serializes"),
                // D1: the salted identity attributes the device folds into R. Carrying the salts
                // here is deliberate — the device must retain them as disclosure openings — and is
                // no wider than the ownerIdentity block above: this route is already the one-time,
                // short-TTL token capability that hands the vet-held identity to the owner's device.
                "identityLeaves": serde_json::to_value(&s.identity_leaves).expect("IdentityLeaf serializes"),
                "unverifiedClaims": serde_json::to_value(&claims).expect("ConvenienceClaims serializes"),
                // Countdown convention: seconds remaining at response receipt, never a wall-clock
                // deadline (the device's clock and this one can disagree).
                "ttlSecs": s.token_exp.saturating_sub(auth::now()),
            }))
        }
        None => err(StatusCode::NOT_FOUND, "session not found"),
    }
}

#[derive(Deserialize)]
struct ProfileCustodialBindReq {
    token: String,
    /// The DEVICE-computed profile root `R` (0x + 64 hex). The server cannot fold it itself (the
    /// owner's seed never leaves the phone), but the D1 gate below rebuilds it from the posted
    /// openings + reserved hashes and refuses any `R` that commits anything else.
    root: String,
    /// D1: the opening of EVERY attribute leaf of the tree — pet AND identity alike, in any order.
    /// The gate recomputes each leaf hash from its opening (`hash_leaf` discipline; a supplied
    /// hash is never trusted for an opened leaf).
    #[serde(default)]
    leaves: Vec<LeafOpeningReq>,
    /// D1: exactly THREE opaque `0x..` 32-byte leaf hashes — the reserved owner-control triple
    /// (`owner.address` / `owner.consentKey` / `owner.secret` leaf hashes). Never opened: their
    /// preimages are the owner's private material.
    #[serde(rename = "reservedLeafHashes", default)]
    reserved_leaf_hashes: Vec<String>,
}

/// One posted attribute-leaf opening: `{keyPath, saltHex, tag, value}`.
#[derive(Deserialize)]
struct LeafOpeningReq {
    #[serde(rename = "keyPath")]
    key_path: String,
    #[serde(rename = "saltHex")]
    salt_hex: String,
    tag: u8,
    value: String,
}

/// The depth-6 consent tree (`DogTagConsent(6)`) caps a profile at 64 leaves.
const PROFILE_TREE_CAPACITY: usize = 64;

fn decode_salt_bytes(salt_hex: &str) -> Result<Vec<u8>, String> {
    hex::decode(salt_hex.strip_prefix("0x").unwrap_or(salt_hex))
        .map_err(|e| format!("bad salt hex: {e}"))
}

/// D1 ATTESTATION-INTEGRITY GATE — a FULL-LEAF-LIST commitment check on the posted `R`.
///
/// The device opens EVERY attribute leaf of its tree (`leaves`) and names the three reserved
/// owner-control leaf hashes opaquely (`reservedLeafHashes`); the vet then:
///   1. requires exactly 3 reserved hashes and a total leaf count within the depth-6 capacity, and
///      RECOMPUTES every attribute leaf hash from its posted opening (the `hash_leaf` discipline —
///      a supplied hash is never trusted for an opened leaf);
///   2. requires the posted `owner.identity.*` openings to EXACTLY equal the session's stored
///      identity `{keyPath, salt, value}` set — no missing, extra, duplicate, or altered entry.
///      On the degrade path (the operator collected no identity) that subset must be EMPTY.
///      Non-identity openings are not checked against session data — only recomputed and folded;
///   3. rebuilds the Merkle root from [the 3 reserved hashes + all recomputed attribute hashes]
///      and requires it to equal the posted `R` exactly.
///
/// Soundness: a forged `owner.identity.*` leaf must either be OPENED — step 2 refuses it — or
/// hide among the 3 opaque hashes, which displaces a reserved leaf; a tree missing a reserved
/// leaf can never produce a consent proof, and disclosures are only ever accepted alongside a
/// consent proof for the same `R`. INJECTION of an unattested identity leaf is therefore closed,
/// not merely replacement of an attested one (the predecessor per-leaf inclusion-proof gate
/// proved the vet's leaves were included in `R` but could not stop a device from ALSO committing
/// forged identity leaves beside them).
///
/// The bind reveals the device-random pet-attribute salts to the vet — deliberate and zero-cost:
/// the vet supplied every attribute value in the first place, so the openings add nothing it did
/// not already know. See docs/DPIA.md §2.1.
fn verify_leaf_commitment(
    session_identity: &[crate::store::IdentityLeaf],
    leaves: &[LeafOpeningReq],
    reserved_leaf_hashes: &[String],
    root_hex: &str,
) -> Result<(), String> {
    // (1) the reserved triple, the capacity bound, and every opening recomputed.
    if reserved_leaf_hashes.len() != 3 {
        return Err(format!(
            "expected exactly 3 reservedLeafHashes (the owner-control triple), got {}",
            reserved_leaf_hashes.len()
        ));
    }
    if 3 + leaves.len() > PROFILE_TREE_CAPACITY {
        return Err(format!(
            "3 reserved + {} attribute leaves exceeds the depth-6 tree capacity of {PROFILE_TREE_CAPACITY}",
            leaves.len()
        ));
    }
    let mut leaf_hexes: Vec<String> = Vec::with_capacity(3 + leaves.len());
    for h in reserved_leaf_hashes {
        let s = h.strip_prefix("0x").unwrap_or(h);
        if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "reserved leaf hash {h:?} is not a 0x.. 32-byte hex word"
            ));
        }
        leaf_hexes.push(h.clone());
    }
    for leaf in leaves {
        let hash = dogtag_standard::ffi::hash_leaf_hex(
            leaf.key_path.clone(),
            leaf.salt_hex.clone(),
            leaf.tag,
            leaf.value.clone(),
        )
        .map_err(|e| format!("leaf {:?}: {e}", leaf.key_path))?;
        leaf_hexes.push(hash);
    }

    // (2) EXACT identity-set equality against the vet's own stored openings. Classified and
    // compared on the NFC-normalized keyPath — the same predicate `build_profile_tree` guards
    // with, since the leaf commits `field_of_keypath(nfc(kp))`.
    let mut expected: Vec<&crate::store::IdentityLeaf> = session_identity.iter().collect();
    for leaf in leaves {
        let normalized = dogtag_standard::encode::nfc(&leaf.key_path);
        if !normalized.starts_with(dogtag_standard::profile_tree::OWNER_IDENTITY_PREFIX) {
            continue;
        }
        let salt = decode_salt_bytes(&leaf.salt_hex)
            .map_err(|e| format!("identity opening {:?}: {e}", leaf.key_path))?;
        let matched = expected.iter().position(|stored| {
            dogtag_standard::encode::nfc(&stored.key_path) == normalized
                && decode_salt_bytes(&stored.salt_hex)
                    .map(|b| b == salt)
                    .unwrap_or(false)
                && stored.tag == leaf.tag
                && stored.value == leaf.value
        });
        match matched {
            Some(i) => {
                expected.remove(i);
            }
            None => {
                return Err(format!(
                    "identity opening {:?} does not match any vet-attested identity leaf of this \
                     session (forged, duplicate, or altered value/salt)",
                    leaf.key_path
                ));
            }
        }
    }
    if let Some(missing) = expected.first() {
        return Err(format!(
            "missing identity opening for vet-attested leaf {:?}",
            missing.key_path
        ));
    }

    // (3) the posted openings + reserved hashes must rebuild the posted R exactly.
    let rebuilt = dogtag_standard::ffi::build_merkle_root_hex(leaf_hexes)
        .map_err(|e| format!("rebuilding R from the posted leaves: {e}"))?;
    if !rebuilt.eq_ignore_ascii_case(root_hex) {
        return Err(
            "the posted leaf openings and reserved hashes do not rebuild the posted R".to_string(),
        );
    }
    Ok(())
}

/// Shape-check a device-supplied `R`: `0x` + 64 hex, non-zero.
///
/// The server cannot FOLD `R` itself - it folds the owner's wallet-seed-derived secret, which the
/// server has (and must have) no access to - but `verify_leaf_commitment` above rebuilds it from
/// the posted openings + reserved hashes, so this shape check is only the cheap edge filter. The
/// non-zero check mirrors `mintCustodial`'s own `root == 0 -> BadRoot` so an obviously-bad request
/// fails at the edge instead of burning gas.
fn valid_root_hex(root: &str) -> bool {
    root.len() == 66
        && root.starts_with("0x")
        && root[2..].bytes().all(|b| b.is_ascii_hexdigit())
        && root[2..].bytes().any(|b| b != b'0')
}

/// POST /profiles/issue/custodial-bind — the sole owner-hidden issuance bind. Device,
/// token-authenticated. `R` is a profile tree folded on the device from the owner's wallet seed, so
/// the device posts `R` and the server only anchors it. Nothing here builds an owner-bearing VC.
///
/// # Why there is no wallet and no signature
///
/// `mintCustodial(id, root)` takes no recipient - the tag goes to the contract's immutable custodian,
/// so an owner wallet is not expressible in the calldata. The authorization is the **one-time bind
/// token** alone: it is minted only by the operator-gated `/profiles/issue/session/start`, is
/// consumed atomically, and expires (scan window from mint, extended at first resolve so the
/// deadline cannot land mid-bind — see the BIND_TOKEN_* constants). Whoever redeems it defines
/// ownership through the owner secret sealed inside `R`.
///
/// # The two on-chain conditions (datamodel §3.5)
///
/// Owner-hidden verification checks `R` twice, against independent state
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
/// blocks are ~12s apart, well past the phone's read timeout. The operator portal polls
/// `GET /profiles/issue/session/{id}`.
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
    // a half-wired owner-hidden stack must not burn the operator's QR (and, worse, must never mint without
    // a place to anchor the root). Both addresses are shape-checked, not merely tested for non-zero —
    // see `verify::valid_contract_addr` for why a malformed value would otherwise reach the chain as
    // 0x0..0. `dog_tag_anchor_refusal` is that same predicate pair plus the operator-vocabulary
    // message; the session-start route asks it too, so within one process this arm is unreachable
    // through the normal flow — it stays load-bearing because sessions OUTLIVE a process under the
    // Mongo store, so a bind can arrive on a restart whose env no longer carries the addresses the
    // starting process had.
    let sbt_addr = st.cfg.sbt_consent_addr.clone();
    let issuer_addr = st.cfg.profile_issuer_addr.clone();
    if let Some(refusal) = st.cfg.dog_tag_anchor_refusal() {
        return err(StatusCode::SERVICE_UNAVAILABLE, &refusal);
    }
    // The SBT mint role, BEFORE the one-time token is consumed and for the same reason as the
    // config check above: a definite "the SBT would refuse this signer" means the mint cannot
    // complete, and refusing here leaves the QR alive for a retry once the role is granted.
    // Could-not-check proceeds — the background arms report the real failure if one comes.
    if let MintRoleGate::Missing(refusal) = mint_role_gate(&st).await {
        return err(StatusCode::SERVICE_UNAVAILABLE, &refusal);
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

    // D1 ATTESTATION-INTEGRITY GATE — BEFORE anything is persisted or reaches the chain. The
    // device opens EVERY attribute leaf and names the reserved triple's hashes opaquely; the vet
    // recomputes each opened leaf, requires the identity openings to EXACTLY match its own stored
    // {keyPath, salt, value} set, and rebuilds the posted R from the full leaf list. On any
    // mismatch the bind is REFUSED with nothing written on-chain. This is what makes the committed
    // identity vet-ATTESTED: a device can neither REPLACE a vet-attested value nor INJECT an extra
    // owner.identity.* leaf the vet never saw (see `verify_leaf_commitment` for the soundness
    // argument). Refusal settles the session (the one-time token is already consumed, mirroring
    // the sealed-collision refusal below) — the operator starts a fresh session.
    if let Err(reason) = verify_leaf_commitment(
        &session.identity_leaves,
        &body.leaves,
        &body.reserved_leaf_hashes,
        &root,
    ) {
        let msg = format!(
            "identity attestation integrity failed: {reason} — the posted R must commit exactly \
             the vet-collected identity and nothing else in owner.identity.*; start a FRESH session"
        );
        settle_bind_error(&mut session, "attestation", &msg);
        st.store.update_profile_session(session).await;
        return err(StatusCode::BAD_REQUEST, &msg);
    }

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

    // Scope the owner-blindness claim precisely: "owner-hidden" means hidden from DOWNSTREAM parties
    // - the chain, verifiers, relayers - NOT from the issuing authority. The session row this handler
    // updates still carries the `owner_identity` block (name, country of identification,
    // identification number) collected by the operator-gated `/profiles/issue/session/start`, and that
    // is deliberate: the issuing vet legitimately holds the identity of the person it issues to.
    // D1: that identity is now COMMITTED into `R` as hidden, selectively-disclosable
    // `owner.identity.*` attribute leaves — folded on the DEVICE (the seed never leaves the phone)
    // and verified into `R` by the integrity gate above. See docs/DPIA.md §2.1.
    session.root = Some(root.clone());
    session.protocol_version = Some(dogtag_standard::wrap::LEVEL_B_VERSION.to_string());
    // "minting": a device's bind was ACCEPTED and the chain writes are what remains. The portal
    // renders this apart from "pending" — while the row still said "pending" through the whole
    // anchoring phase, the operator's screen could declare the QR dead over an issuance that was
    // already minting. No deadline applies past this point; the spawned task settles the row to
    // "bound" or "error" on both arms.
    session.status = "minting".to_string();
    st.store.update_profile_session(session.clone()).await;

    // Re-check the write-once seal here, synchronously, before the spawn — so it is causally
    // before `issue(R)`, the first and GLOBALLY irreversible write (`registerRoot` is write-once, so a
    // consumed `R` can never be re-anchored). `/profiles/issue/session/start` already refuses an id
    // whose `profileRoot` is set, but that check runs up to the token's whole life earlier
    // (scan + bind windows, ~15 minutes worst case): without this one a collision
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
                "dogTagId {} is already sealed (profileRoot is write-once and \
                 survives a burn, so this id is retired forever) — start a FRESH session",
                session.dog_tag_id
            );
            settle_bind_error(&mut session, "seal", &msg);
            st.store.update_profile_session(session).await;
            return err(StatusCode::CONFLICT, &msg);
        }
        // Fail CLOSED on an inconclusive read: proceeding is what spends the irreversible write, and
        // the token is already consumed, so the row must be settled or the operator poll hangs.
        Err(e) => {
            let msg = format!(
                "could not confirm dogTagId {} is unsealed: {e} — start a FRESH \
                 session",
                session.dog_tag_id
            );
            settle_bind_error(&mut session, "seal", &msg);
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
        // Every failure arm here does THREE things, and all three exist because a live failure
        // (2026-08-07) did none of them where anyone looked: LOG the stage + cause where an
        // operator tails the backend, SETTLE the session with the stage + reason (the portal's
        // issuance card and the device's `/p/<token>/status` peek both read the row), and RETURN.
        // The phone is chain-polling `profileRoot`, so without the settled row it waits ~190s for
        // an anchor that will never exist and then guesses.

        // (0) RESUME CHECK. `issue(R)` and `mintCustodial` are two transactions, so a crash or a
        // mint-stage revert between them leaves R anchored on this clone with the tag unminted —
        // and `registerRoot` is globally write-once, so re-running `issue(R)` for that R can only
        // revert "root taken". A root THIS CLONE already anchored is stage (1) already done:
        // resume at the mint. A failed read falls through to the plain issue — could-not-check
        // must not invent either answer, and the write itself reports the truth.
        let already_anchored = matches!(
            chain.issued_at(&issuer_addr, &bg_root).await,
            Ok(at) if !at.is_zero()
        );
        if already_anchored {
            // CROSS-SESSION GUARD: an anchored root may only be COMPLETED by the session whose
            // dogTagId it was derived for. A shipped device can never trip this — it rebuilds a
            // tree keyed on THIS session's dogTagId, so on a fresh session it posts a fresh root —
            // but a hand-crafted bind could post another session's stranded root, and the mint
            // below would then retire THIS session's dogTagId onto a root whose KDF binds a
            // different id: every consent proof fails `R == profileRoot(dogTagId)` forever. The
            // owning session still records the root (an errored row keeps it; only its own retry
            // clears it), so the refusal can name the one path that completes it.
            let owner = store
                .list_profile_sessions()
                .await
                .into_iter()
                .find(|s| {
                    s.session_id != bg_session.session_id
                        && s.root
                            .as_deref()
                            .is_some_and(|r| r.eq_ignore_ascii_case(&bg_root))
                });
            if let Some(owner) = owner {
                let msg = format!(
                    "this profile root was anchored by the issuance session for dogTagId {} — it \
                     can only be completed by retrying THAT session, and completing it here would \
                     retire dogTagId {} onto a root derived for a different tag (every verify \
                     would fail forever)",
                    owner.dog_tag_id, bg_session.dog_tag_id
                );
                tracing::error!(
                    stage = "issue(R)",
                    dog_tag_id = %bg_session.dog_tag_id,
                    owning_dog_tag_id = %owner.dog_tag_id,
                    session = %bg_session.session_id,
                    root = %bg_root,
                    "dog-tag issuance refused: {msg}"
                );
                // Clear the root the handler stamped pre-spawn: THIS session never owned it, and a
                // retained claim would make the OWNING session's own retry-bind see this row as a
                // second claimant and refuse — the refused adoption locking out the legitimate
                // recovery it was refused to protect.
                bg_session.root = None;
                settle_bind_error(&mut bg_session, "issue", &msg);
                store.update_profile_session(bg_session).await;
                return;
            }
            // Anchored AND revoked can never verify (`isValid` folds not-revoked), and minting
            // would retire the dogTagId onto a dead anchor — refuse with the D3 remedy instead.
            if !matches!(chain.is_valid(&issuer_addr, &bg_root).await, Ok(true)) {
                let msg = format!(
                    "this profile root is already anchored on the DOG_PROFILE clone but is not \
                     valid there (revoked, or the read failed) — it can never verify, and \
                     minting would retire dogTagId {} onto it. Start a FRESH session (a new \
                     dogTagId derives a new root)",
                    bg_session.dog_tag_id
                );
                tracing::error!(
                    stage = "issue(R)",
                    dog_tag_id = %bg_session.dog_tag_id,
                    session = %bg_session.session_id,
                    root = %bg_root,
                    "dog-tag issuance failed: {msg}"
                );
                settle_bind_error(&mut bg_session, "issue", &msg);
                store.update_profile_session(bg_session).await;
                return;
            }
            tracing::info!(
                dog_tag_id = %bg_session.dog_tag_id,
                session = %bg_session.session_id,
                root = %bg_root,
                "dog-tag issuance resuming: this clone already anchored the root (an earlier \
                 attempt's issue(R) landed and its mint did not) — skipping issue(R) and \
                 completing the mint"
            );
        } else {
            // (1) ANCHOR FIRST — issue(R) into the DogTagIssuer clone, so `rootIssuer[R]`
            // resolves. If this fails nothing has been minted and the dogTagId is still free.
            if let Err(e) = chain.issue(signer_index, &issuer_addr, &bg_root).await {
                let msg = format!(
                    "issue(R) failed on the DOG_PROFILE clone: {e}. Nothing was minted and \
                     dogTagId {} is still free — fix the cause, then use Retry issuance on this \
                     session (the owner re-scans a fresh QR; the phone re-posts the same root)",
                    bg_session.dog_tag_id
                );
                tracing::error!(
                    stage = "issue(R)",
                    dog_tag_id = %bg_session.dog_tag_id,
                    session = %bg_session.session_id,
                    root = %bg_root,
                    error = %e,
                    "dog-tag issuance failed at issue(R); session settled to error"
                );
                settle_bind_error(&mut bg_session, "issue", &msg);
                store.update_profile_session(bg_session).await;
                return;
            }
        }
        // (2) THEN SEAL — mintCustodial(id, R). Irreversible past this point.
        let sent = match chain
            .mint_custodial(signer_index, &sbt_addr, &bg_id, &bg_root)
            .await
        {
            Ok(s) => Some(s),
            Err(e) => {
                // The stranded-root state: R is anchored (issue(R) landed, above or on an earlier
                // attempt) and the tag is unminted. A FRESH session cannot complete it — a new
                // dogTagId derives a NEW root, and re-issuing THIS root reverts "root taken" — so
                // the reason names the one path that can: retry THIS session, which the resume
                // check above turns into a mint-only completion.
                let msg = format!(
                    "mintCustodial failed on the SBT: {e}. The root IS anchored on-chain \
                     (issue(R) landed), so this dog tag can only be completed by fixing the \
                     cause and using Retry issuance on THIS session — a fresh session derives a \
                     different root and strands this one forever",
                );
                tracing::error!(
                    stage = "mintCustodial",
                    dog_tag_id = %bg_session.dog_tag_id,
                    session = %bg_session.session_id,
                    root = %bg_root,
                    error = %e,
                    "dog-tag issuance failed at mintCustodial with the root already anchored; \
                     session settled to error"
                );
                settle_bind_error(&mut bg_session, "mint", &msg);
                store.update_profile_session(bg_session).await;
                return;
            }
        };
        // Read BOTH conditions back on-chain before calling the issuance good — a receipt is not
        // proof. There is deliberately no owner comparison: the contract holds the token in neutral
        // custody, and owner linkage is not part of this model.
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
            bg_session.tx_hash = sent.map(|s| s.tx_hash);
            bg_session.error_stage = None;
            bg_session.error_reason = None;
            tracing::info!(
                dog_tag_id = %bg_session.dog_tag_id,
                session = %bg_session.session_id,
                "dog-tag issuance bound: issue(R) + mintCustodial landed and both on-chain \
                 conditions read back"
            );
        } else {
            let msg = format!(
                "the mint transaction landed but the on-chain read-back failed \
                 (profileRoot-matches={root_ok} isValid={anchored_ok}) — check the vet portal \
                 and the chain before retrying anything"
            );
            tracing::error!(
                stage = "verify",
                dog_tag_id = %bg_session.dog_tag_id,
                session = %bg_session.session_id,
                root = %bg_root,
                root_ok,
                anchored_ok,
                "dog-tag issuance read-back failed after the mint; session settled to error"
            );
            settle_bind_error(&mut bg_session, "verify", &msg);
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

/// True for a 0x.. word that is not all zeros (an unset `profileRoot` reads back as 0x0..0).
fn is_nonzero_word(w: &str) -> bool {
    w.trim_start_matches("0x").bytes().any(|b| b != b'0')
}

/// Settle an issuance session as failed: status, WHERE (`error_stage` — machine-readable, the
/// device peek's key) and WHY (`error_reason` — the operator's full detail). `tx_hash` is left
/// alone: it carries a REAL transaction hash or nothing, never prose.
fn settle_bind_error(session: &mut crate::store::ProfileIssueSession, stage: &str, reason: &str) {
    session.status = "error".to_string();
    session.error_stage = Some(stage.to_string());
    session.error_reason = Some(reason.to_string());
}

/// BOOT RECOVERY: settle every issuance session a dead process left at `"minting"` to a RETRYABLE
/// error. Called from `main.rs` once the store is built, before the listener binds.
///
/// A `"minting"` row means a device's bind was accepted and the spawned chain-write task was
/// driving `issue(R)` → `mintCustodial`. That task dies with the process, and NOTHING else ever
/// settles the row — so without this, a restart mid-mint leaves a row the retry route refuses
/// forever (`only a FAILED issuance can be retried`) while the root may already be anchored
/// on-chain: the stranded-root state with its one recovery path locked shut. Every tunnel rotation
/// forces exactly such a restart.
///
/// The settled reason does not guess what landed: the retry's own resume check reads the chain
/// (`issued_at` on the DOG_PROFILE clone) and completes whichever half remains, so "unknown from
/// here" is honest AND recoverable. Stage `"interrupted"` keeps this apart from a failure the
/// process itself observed.
///
/// Under a SHARED store (Mongo, several instances) a booting instance can settle a sibling's LIVE
/// minting row; the live task then overwrites with its own terminal state, and the write-once
/// contracts backstop any operator retry racing it. Transiently mislabelling a live mint as
/// interrupted is the cheaper wrong against a row that would otherwise be stuck forever.
pub async fn settle_interrupted_issuances(store: &std::sync::Arc<dyn crate::store::Store>) -> usize {
    let mut settled = 0;
    for mut s in store.list_profile_sessions().await {
        if s.status != "minting" {
            continue;
        }
        let msg = format!(
            "the vet backend restarted while this issuance's chain writes were in flight — what \
             landed on-chain is unknown from here. Use Retry issuance on this session: if the root \
             was already anchored, the retry completes the remaining mint for dogTagId {}; if \
             nothing landed, it re-anchors and mints",
            s.dog_tag_id
        );
        tracing::warn!(
            dog_tag_id = %s.dog_tag_id,
            session = %s.session_id,
            "dog-tag issuance was interrupted by a restart mid-minting; session settled to a \
             retryable error"
        );
        settle_bind_error(&mut s, "interrupted", &msg);
        store.update_profile_session(s).await;
        settled += 1;
    }
    settled
}

/// The DEVICE-safe sentence for a failed bind, derived from the STAGE alone. The stored
/// `error_reason` is the operator's — it carries chain errors and backend configuration detail
/// that belong on the portal, not on an unauthenticated token peek — so the phone gets what the
/// owner can act on: what happened, and that the clinic's portal names the rest.
fn device_bind_failure_sentence(stage: Option<&str>) -> &'static str {
    match stage {
        Some("attestation") => {
            "The profile this phone posted did not match the vet's attested records. Ask the \
             clinic to start a fresh issuance."
        }
        Some("seal") => {
            "The vet could not confirm this dog tag id is still free on-chain. Ask the clinic to \
             start a fresh issuance."
        }
        Some("issue") => {
            "The vet's on-chain anchoring of this dog tag failed. The vet portal names the reason \
             — ask the clinic to fix it and retry the issuance."
        }
        Some("mint") => {
            "This dog tag's root was anchored on-chain but the tag itself could not be minted. \
             The vet portal names the reason — ask the clinic to fix it and retry this same \
             issuance."
        }
        Some("verify") => {
            "The vet could not confirm this dog tag on-chain after minting. Ask the clinic to \
             check the vet portal before scanning again."
        }
        Some("interrupted") => {
            "The clinic's system restarted while this dog tag was being issued. Ask the clinic to \
             retry the issuance — you scan a fresh QR and the same dog tag completes."
        }
        _ => {
            "This issuance failed at the vet's backend. The vet portal names the reason — ask \
              the clinic."
        }
    }
}

/// GET /p/{token}/status — the DEVICE's answer to "did my bind complete?", keyed by the SAME
/// one-time token the QR carried. Unauthenticated and non-consuming, exactly like `GET /p/{token}`
/// — but unlike the resolve, it keeps answering AFTER the bind consumed the token (the store
/// retains consumed tokens for [`crate::store::BIND_STATUS_GRACE_SECS`]), because the phone's
/// need for it BEGINS at the bind: the bind response can be lost after the token is consumed, and
/// the chain poll can only ever observe success. A session that settled to "error" is a definite
/// answer the phone could otherwise never learn — it would poll the chain for an anchor that will
/// never exist and then guess.
///
/// The response is deliberately MINIMAL: derived status, the dogTagId the phone already knows,
/// and a stage-derived device-safe sentence. Never the session row — it carries the owner's
/// identity — and never the operator's `error_reason`, which names backend configuration.
async fn profile_bind_status(State(st): State<AppState>, Path(token): Path<String>) -> Resp {
    let session_id = match st.store.bind_session_for_status(&token).await {
        Some(id) => id,
        None => return err(StatusCode::NOT_FOUND, "bind token unknown or expired"),
    };
    let s = match st.store.get_profile_session(&session_id).await {
        Some(s) => s,
        None => return err(StatusCode::NOT_FOUND, "session not found"),
    };
    // Derived, not stored, so this surface adds no portal-visible state: a pending row whose root
    // is set is a bind the backend accepted and is still driving ("minting" — the ack's own word).
    let (status, reason) = match s.status.as_str() {
        "error" => (
            "error",
            Some(device_bind_failure_sentence(s.error_stage.as_deref())),
        ),
        "bound" => ("bound", None),
        _ if s.root.is_some() => ("minting", None),
        _ => ("pending", None),
    };
    ok(json!({
        "status": status,
        "dogTagId": s.dog_tag_id,
        "reason": reason,
    }))
}

/// POST /profiles/issue/session/{id}/retry — operator-gated: re-arm an ERRORED issuance session
/// with a fresh one-time QR token, keeping the SAME dogTagId and the SAME vet-salted identity
/// leaves.
///
/// This exists because of the STRANDED-ROOT state: `issue(R)` and `mintCustodial` are two
/// transactions, so a mint-stage failure leaves R anchored with the tag unminted — and that R can
/// only ever be completed through THIS session. A fresh session allocates a new dogTagId, the
/// device derives a NEW root from it (the id is a KDF input), and re-issuing the old root reverts
/// "root taken" forever. Retrying the SAME session hands the device the same id and the same
/// identity salts, so it reuses its persisted profile record, re-posts the SAME root, and the
/// bind's resume check skips straight to the mint. Measured live 2026-08-07: dog tag 6's root was
/// anchored at block 362224 with no mint and, before this route, no way forward.
async fn profile_issue_session_retry(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    if let Some(refusal) = st.cfg.dog_tag_anchor_refusal() {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("cannot retry dog-tag issuance: {refusal}"),
        );
    }
    // Same license as session-start: only a DEFINITE "the SBT would refuse this signer" refuses —
    // a retry whose mint the SBT will certainly revert re-strands the session it exists to rescue.
    if let MintRoleGate::Missing(refusal) = mint_role_gate(&st).await {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("cannot retry dog-tag issuance: {refusal}"),
        );
    }
    let mut session = match st.store.get_profile_session(&session_id).await {
        Some(s) => s,
        None => return err(StatusCode::NOT_FOUND, "session not found"),
    };
    if session.status != "error" {
        return err(
            StatusCode::CONFLICT,
            "only a FAILED issuance can be retried — this session has not errored",
        );
    }
    // The id must still be unsealed: another instance may have minted it meanwhile (the counter
    // resets on restart under the in-memory store). Definite-taken refuses with the D3 remedy;
    // an inconclusive read proceeds — the bind's own synchronous seal re-check fails closed.
    if let Ok(onchain) = onchain_dog_tag_id(&session.dog_tag_id) {
        if let Ok(r) = st
            .chain
            .profile_root_of(&st.cfg.sbt_consent_addr, &onchain)
            .await
        {
            if is_nonzero_word(&r) {
                return err(
                    StatusCode::CONFLICT,
                    &format!(
                        "dogTagId {} was sealed on-chain since this session failed — it cannot \
                         be retried; start a FRESH session",
                        session.dog_tag_id
                    ),
                );
            }
        }
    }
    // Re-arm with a FRESH token under the same deadline model as session-start: the scan window
    // from now, extended at first resolve (see the BIND_TOKEN_* constants). `resolved_at` is
    // cleared — no device has picked THIS QR up, and a stale mark would make the portal report
    // "a device resolved it and went quiet" about a code nobody scanned.
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    let token = hex::encode(bytes);
    let exp = auth::now() + BIND_TOKEN_SCAN_TTL_SECS;
    session.status = "pending".to_string();
    session.root = None;
    session.tx_hash = None;
    session.error_stage = None;
    session.error_reason = None;
    session.resolved_at = None;
    session.token_exp = exp;
    st.store.update_profile_session(session.clone()).await;
    st.store.put_bind_token(&token, &session_id, exp).await;
    let qr = format!("{}/p/{}", st.cfg.deployment_url, token);
    tracing::info!(
        dog_tag_id = %session.dog_tag_id,
        session = %session_id,
        "dog-tag issuance retry armed: fresh bind token minted for the errored session"
    );
    ok(json!({
        "token": token,
        "dogTagId": session.dog_tag_id,
        "sessionId": session_id,
        "qr": qr,
        "ttlSecs": BIND_TOKEN_SCAN_TTL_SECS,
        "qrAddress": crate::qr_address::qr_address_json(&st.cfg.deployment_url),
    }))
}

/// GET /profiles/issue/session/{id} — operator-gated status poll so the portal can show whether the
/// device has bound and surface the txHash/root. Returns the stored session row's status plus the
/// facts the portal needs to say something TRUE about a QR that is going nowhere: `resolvedAt`
/// (has any device picked this up?), `tokenSecondsLeft` (the SERVER's deadline — the portal must
/// never run its own clock; it once declared "expired" from a hardcoded 180s while the server
/// would still have accepted a bind), and `qrAddress` (whether this machine still answers at the
/// address the QR names — re-checked per poll, because the address can change while the QR is on
/// screen).
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
    // Rows written before `error_reason` existed carry their reason as prose in `tx_hash`; fold
    // that legacy shape into `error` here so an old failed row still names its reason on the
    // portal, while `txHash` itself is only ever surfaced as a link on a BOUND session.
    let legacy_reason = (s.status == "error" && s.error_reason.is_none())
        .then(|| s.tx_hash.clone())
        .flatten();
    ok(json!({
        "status": s.status,
        "dogTagId": s.dog_tag_id,
        "root": s.root,
        "txHash": s.tx_hash,
        "protocolVersion": s.protocol_version,
        "resolvedAt": s.resolved_at,
        // Meaningful only while status == "pending" (the bind consumes the token); 0 = the token
        // is dead and, with the session still pending, no bind can ever arrive.
        "tokenSecondsLeft": s.token_exp.saturating_sub(auth::now()),
        "qrAddress": crate::qr_address::qr_address_json(&st.cfg.deployment_url),
        "error": s.error_reason.clone().or(legacy_reason),
        "errorStage": s.error_stage,
    }))
}

/// How many issuance-session rows `GET /profiles/issue/sessions` returns at most (newest first).
/// A recovery surface, not an archive: the operator is looking for the handful of recent
/// failures, and the full history is reachable per-session by id.
const PROFILE_SESSION_LIST_LIMIT: usize = 50;

/// GET /profiles/issue/sessions — operator-gated: the recent dog-tag issuance sessions, newest
/// first. This is the OPERATOR'S ROUTE BACK to a failed issuance once the portal page no longer
/// holds it in memory — after a page reload, and above all after a backend restart, which every
/// tunnel rotation forces. Before it existed, the Retry card could only reach a session the same
/// browser page had started: the stranded-root recovery shipped by the retry route was erased by
/// the restart that most often causes the strand.
///
/// Each row is the SUMMARY the operator needs to recognise the issuance (pet name, owner name,
/// tag id, when, what failed) — never the full session row, which carries the identity leaves'
/// salts. `error` folds the same legacy shape as the per-session status route.
async fn profile_issue_sessions_list(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_operator(&st, &headers).await {
        return e;
    }
    let rows: Vec<serde_json::Value> = st
        .store
        .list_profile_sessions()
        .await
        .into_iter()
        .take(PROFILE_SESSION_LIST_LIMIT)
        .map(|s| {
            let legacy_reason = (s.status == "error" && s.error_reason.is_none())
                .then(|| s.tx_hash.clone())
                .flatten();
            json!({
                "sessionId": s.session_id,
                "dogTagId": s.dog_tag_id,
                "status": s.status,
                "createdAt": s.created_at,
                "petName": s.pet_name,
                "ownerName": s.owner_identity.name,
                "error": s.error_reason.clone().or(legacy_reason),
                "errorStage": s.error_stage,
            })
        })
        .collect();
    ok(json!({ "sessions": rows }))
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
    let mints = st.store.list_profile_sessions().await;
    let scope = crate::trace::build_scope(st.store.as_ref(), &st.cfg, &records, &sessions).await;
    let idx = crate::trace::build_index(&records, &sessions, &mints);
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
/// `/v1/stats`) plus its own off-chain record/verification/mint counts. Operator-gated.
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
    let mints = st.store.list_profile_sessions().await;
    if let Value::Object(map) = &mut body {
        map.insert(
            "local".into(),
            json!({
                "records": records.len(),
                "verifications": sessions.len(),
                // dog tags actually sealed on-chain (bound), not sessions merely started.
                "dogTagsMinted": mints.iter().filter(|m| m.status == "bound").count(),
            }),
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
    // Consent proving is mounted only on prover-service builds.
    #[cfg(feature = "prover")]
    let prove_route = Router::new().route("/prove-consent", post(prove_consent));
    #[cfg(not(feature = "prover"))]
    let prove_route = Router::<AppState>::new();

    // ISSUANCE SURFACES — mounted only for a role that issues (see `Config::issuance_enabled`). A
    // groomer runs this same binary as a pure VERIFIER: it records proofs-of-verification against the
    // `VERIFY:<purpose>` whitelist and never mints credentials, so for `BUSINESS_TYPE=groomer` these
    // routes do not exist at all rather than existing-and-refusing. Every other role (the vet) is
    // completely unaffected.
    let issuance = if state.cfg.issuance_enabled() {
        Router::new()
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
            // DOG_PROFILE (SBT) issuance — vet issues dog tags
            .route(
                "/profiles/issue/session/start",
                post(profile_issue_session_start),
            )
            // the operator's route BACK to a failed issuance once the portal page no longer holds
            // it (page reload, backend restart) — the Retry card's list source.
            .route(
                "/profiles/issue/sessions",
                get(profile_issue_sessions_list),
            )
            .route(
                "/profiles/issue/session/:id",
                get(profile_issue_session_status),
            )
            // re-arm a FAILED issuance with a fresh QR: same session, same dogTagId, same
            // identity salts — the only path that can complete a stranded (anchored, unminted)
            // root. Operator-gated like start.
            .route(
                "/profiles/issue/session/:id/retry",
                post(profile_issue_session_retry),
            )
            // owner-hidden issuance is the sole bind path.
            .route(
                "/profiles/issue/custodial-bind",
                post(profile_issue_custodial_bind),
            )
            // short one-time bind token resolver (unauthenticated; NON-consuming — consume on bind)
            .route("/p/:token", get(profile_bind_resolve))
            // the DEVICE's bind-status peek: unauthenticated, non-consuming, and — unlike the
            // resolve above — still answering after the bind consumed the token, so the phone can
            // learn a definite "error" instead of chain-polling into a guess.
            .route("/p/:token/status", get(profile_bind_status))
    } else {
        Router::<AppState>::new()
    };

    Router::new()
        .merge(prove_route)
        .merge(issuance)
        // the shop's own clients / appointments / verification history (operator-gated)
        .merge(crate::crm::crm_router())
        // `.ics` calendar interop: the UNAUTHENTICATED subscription feed (the secret is in the
        // path — a calendar client cannot present a bearer token) + the operator-gated feed
        // administration and import routes.
        .merge(crate::calendar_ics::ics_feed_router())
        .merge(crate::calendar_ics::ics_admin_router())
        // The CLIENT half of the calendar: `/a/{token}` is the UNAUTHENTICATED per-appointment
        // handoff a client scans (page + `.ics`), and the mint beside it is operator-gated. Mounted
        // for EVERY role, not only issuers — a groomer books appointments and is exactly who needs to
        // hand one to a client.
        .merge(crate::appointment_share::appointment_share_public_router())
        .merge(crate::appointment_share::appointment_share_admin_router())
        // health (no auth) — used by compose healthchecks
        .route("/health", get(health))
        // login
        .route("/login", post(login))
        // settings
        .route(
            "/settings/signing-mode",
            get(get_signing_mode).put(put_signing_mode),
        )
        // short-lived EXPORT token resolver (unauthenticated; non-consuming; session-scoped)
        .route("/x/:token", get(export_session_resolve))
        // discovery signed-manifest fallback (M7 §5.1 1B) — the dogtag-signed version manifest an app
        // verifies OFFLINE. A NEW route, distinct from the resolve GET (`/p/`, `/x/`); on any conflict
        // the on-chain ProtocolRegistry (1C) wins. UNAUTHENTICATED (public discovery data).
        .route("/protocol/manifest", get(crate::protocol::get_manifest))
        // issuer signers
        .route("/issuer/signers", get(issuer_signers))
        // LAYER 2 of the two-layer issuance requirement — the clone's OWN list. Mounted in the
        // PUBLIC router rather than the issuance one, deliberately: a groomer is configured with the
        // same clones and REMOVAL is the safety direction, so a role that does not issue must still
        // be able to see who may sign in its name and withdraw a compromised key.
        .route("/issuer/issuance-allowed", get(issuance_allowed_roster))
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
        .route("/v1/verify/credential", post(verify_credential))
        // canonical owner-hidden consent submission route.
        .route("/v1/verify/consent", post(verify_consent_submit))
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
