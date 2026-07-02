//! Axum router + HTTP handlers for the government credential authority.
//!
//! Route map (all JSON):
//!   GET  /health                         liveness (compose healthcheck) + chain/mode readiness
//!   POST /v1/travel-clearance/issue      ISSUER: build a TRAVEL_CLEARANCE/EU_HEALTH_CERT VC, compute
//!                                         its Poseidon root R, anchor it on-chain (DogTagIssuer.issue)
//!                                         when a signer + whitelisted clone are configured, persist.
//!   POST /v1/verify                      VERIFIER: recompute a wrapped credential's integrity, read
//!                                         DogTagIssuer.isValid(root) + IssuerRegistry.isWhitelistedFor
//!                                         off ROAX, fold to a verdict, persist an audit record.
//!   GET  /v1/records                     list issued credentials (off-chain DB surface).
//!   GET  /v1/records/:root               get one issued credential by root.
//!   GET  /v1/verifications               list the verification audit log.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use dogtag_standard::verify::{check_integrity, FragmentState};
use dogtag_standard::wrap::WrappedDoc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{self, AppState};
use crate::store::{CredentialStatus, IssuedCredential, VerificationRecord};

/// On-chain-derived / anchored keys that a metadata update must never mutate. Presence of any of
/// these in a PATCH body is rejected (they reflect immutable chain state).
const IMMUTABLE_KEYS: &[&str] = &[
    "root",
    "recordType",
    "record_type",
    "dogTagId",
    "dog_tag_id",
    "issuerAddr",
    "issuer_addr",
    "contractAddress",
    "wrappedDoc",
    "wrapped_doc",
    "txHash",
    "tx_hash",
    "blockNumber",
    "block_number",
    "explorerUrl",
    "explorer_url",
    "anchored",
    "revokedTxHash",
    "revokedBlockNumber",
    "revokeExplorerUrl",
];

type Resp = (StatusCode, Json<Value>);

fn ok(v: Value) -> Resp {
    (StatusCode::OK, Json(v))
}
fn err(code: StatusCode, msg: &str) -> Resp {
    eprintln!("[err {code}] {msg}");
    (code, Json(json!({ "error": msg })))
}

/// Gate for the record MUTATION endpoints (PATCH + revoke): require `Authorization: Bearer <token>`
/// matching the configured `GOV_API_TOKEN`. Unconfigured token fails closed (503). Reads, verify and
/// issue stay open.
fn require_api_token(st: &AppState, headers: &HeaderMap) -> Result<(), Resp> {
    let expected = match st.cfg.api_token.as_deref() {
        Some(t) => t,
        None => {
            return Err(err(
                StatusCode::SERVICE_UNAVAILABLE,
                "GOV_API_TOKEN not configured",
            ))
        }
    };
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match presented {
        Some(t) if t == expected => Ok(()),
        _ => Err(err(
            StatusCode::UNAUTHORIZED,
            "missing or invalid bearer token",
        )),
    }
}

/// Monotonic-ish wall clock (seconds). Government records are audit metadata, not consensus-critical.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// --------------------------------------------------------------------------------------------
// health
// --------------------------------------------------------------------------------------------

async fn health(State(st): State<AppState>) -> Resp {
    ok(json!({
        "status": "ok",
        "service": "government-api",
        "chainId": st.cfg.chain_id,
        "demo": st.cfg.demo,
        "canSign": st.chain.can_sign(),
        "signer": st.chain.signer_address(),
        "issuers": {
            app::TRAVEL_CLEARANCE: st.cfg.issuer_addr_for(app::TRAVEL_CLEARANCE),
            app::EU_HEALTH_CERT: st.cfg.issuer_addr_for(app::EU_HEALTH_CERT),
        }
    }))
}

// --------------------------------------------------------------------------------------------
// issue
// --------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct IssueBody {
    #[serde(default = "default_record_type")]
    record_type: String,
    dog_tag_id: String,
    #[serde(default)]
    fields: Value,
    /// When false (default), anchor the root on-chain if a signer is available. When true, only build
    /// + persist the credential (no gas) — useful before a signer is funded/whitelisted.
    #[serde(default)]
    dry_run: bool,
}

fn default_record_type() -> String {
    app::TRAVEL_CLEARANCE.to_string()
}

/// Government issuer: build the authority-endorsed credential, compute R, anchor on-chain, persist.
async fn issue(State(st): State<AppState>, Json(body): Json<IssueBody>) -> Resp {
    if !app::is_supported_record_type(&body.record_type) {
        return err(
            StatusCode::BAD_REQUEST,
            "unsupported record type (TRAVEL_CLEARANCE | EU_HEALTH_CERT)",
        );
    }
    if body.dog_tag_id.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "dog_tag_id is required");
    }
    let issuer_addr = match st.cfg.issuer_addr_for(&body.record_type) {
        Some(a) => a,
        None => {
            return err(
                StatusCode::SERVICE_UNAVAILABLE,
                "no DogTagIssuer clone configured for this record type (set *_ISSUER_ADDR)",
            )
        }
    };

    // BUILD (server-side, shared open standard): typed leaves -> single Poseidon root R.
    let vc = app::build_gov_vc(&st.cfg, &body.record_type, &body.fields, &body.dog_tag_id);
    let meta = app::issuer_meta(&st.cfg, &body.record_type, &issuer_addr);
    let doc = match app::wrap(meta, &vc) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::BAD_REQUEST, &e),
    };
    let root = doc.signature.merkle_root.clone();

    // ANCHOR on-chain unless dry-run / no signer. issue() is idempotent-guarded on-chain (a
    // re-issue of the same root reverts); we surface that as a 409.
    let mut tx_hash: Option<String> = None;
    let mut block_number: Option<u64> = None;
    let mut anchored = false;
    if !body.dry_run && st.chain.can_sign() {
        match st.chain.issue(&issuer_addr, &root).await {
            Ok(sent) => {
                tx_hash = Some(sent.tx_hash);
                block_number = sent.block_number;
                anchored = true;
            }
            Err(e) => {
                return err(
                    StatusCode::BAD_GATEWAY,
                    &format!("on-chain issue failed: {e}"),
                )
            }
        }
    }
    let explorer_url = tx_hash.as_deref().map(crate::chain::explorer_tx_url);

    let ts = now();
    let cred = IssuedCredential {
        root: root.clone(),
        record_type: body.record_type.clone(),
        dog_tag_id: body.dog_tag_id.clone(),
        issuer_addr: issuer_addr.clone(),
        wrapped_doc: serde_json::to_value(&doc).unwrap_or(Value::Null),
        tx_hash: tx_hash.clone(),
        block_number,
        explorer_url: explorer_url.clone(),
        anchored,
        status: if anchored {
            CredentialStatus::Issued
        } else {
            CredentialStatus::Draft
        },
        label: None,
        notes: None,
        revoked_tx_hash: None,
        revoked_block_number: None,
        revoke_explorer_url: None,
        invalidated_at: None,
        invalidation_reason: None,
        created_at: ts,
        updated_at: ts,
    };
    st.store.put_credential(cred).await;

    ok(json!({
        "root": root,
        "recordType": body.record_type,
        "dogTagId": body.dog_tag_id,
        "issuerAddr": issuer_addr,
        "anchored": anchored,
        "txHash": tx_hash,
        "blockNumber": block_number,
        "explorerUrl": explorer_url,
        "wrappedDoc": doc,
    }))
}

// --------------------------------------------------------------------------------------------
// verify
// --------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct VerifyBody {
    /// The wrapped credential document to verify (as produced by any DogTag issuer).
    wrapped_doc: WrappedDoc,
    /// Override the DogTagIssuer clone to check `isValid` against. Defaults to the doc's
    /// `issuer.documentStore`.
    #[serde(default)]
    issuer_addr: Option<String>,
    /// Optional signer address to check issuer-identity (`isWhitelistedFor(recordType, signer)`).
    #[serde(default)]
    signer_addr: Option<String>,
}

/// Government verifier: integrity (offline recompute) + on-chain status + issuer-identity, folded to
/// a single verdict, recorded to the audit log. All chain reads are gasless.
async fn verify(State(st): State<AppState>, Json(body): Json<VerifyBody>) -> Resp {
    let doc = body.wrapped_doc;
    let record_type = doc.issuer.record_type.clone();
    let issuer_addr = body
        .issuer_addr
        .clone()
        .unwrap_or_else(|| doc.issuer.document_store.clone());
    let claimed_root = doc.signature.merkle_root.clone();

    // 1) integrity — recompute the root from the salted leaves and compare (offline, no chain).
    let (integrity_state, recomputed) = check_integrity(&doc);
    let integrity_valid = integrity_state == FragmentState::Valid;
    let recomputed_hex = dogtag_standard::to_hex32(&recomputed);

    // 2) on-chain status — DogTagIssuer.isValid(root) over ROAX (gasless read).
    let onchain_valid = match st.chain.is_valid(&issuer_addr, &claimed_root).await {
        Ok(v) => v,
        Err(e) => {
            return err(
                StatusCode::BAD_GATEWAY,
                &format!("on-chain isValid read failed: {e}"),
            )
        }
    };

    // 3) issuer identity (optional) — IssuerRegistry.isWhitelistedFor(keccak(recordType), signer).
    let issuer_whitelisted = match &body.signer_addr {
        Some(signer) => {
            let rt_key = app::record_type_key(&record_type);
            match st
                .chain
                .is_whitelisted_for(&st.cfg.issuer_registry_addr, &rt_key, signer)
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
        None => None,
    };

    // Verdict: integrity + on-chain issuance are the required authenticity pillars here; the issuer
    // whitelist, when supplied, must also pass (architecture §5 authenticity pillars).
    let verdict = integrity_valid && onchain_valid && issuer_whitelisted.unwrap_or(true);

    let rec = VerificationRecord {
        id: uuid::Uuid::new_v4().to_string(),
        record_type: record_type.clone(),
        root: claimed_root.clone(),
        issuer_addr: issuer_addr.clone(),
        integrity_valid,
        onchain_valid,
        issuer_whitelisted,
        verdict,
        checked_at: now(),
    };
    st.store.put_verification(rec.clone()).await;

    ok(json!({
        "verdict": verdict,
        "recordType": record_type,
        "root": claimed_root,
        "recomputedRoot": recomputed_hex,
        "issuerAddr": issuer_addr,
        "fragments": {
            "integrity": integrity_valid,
            "onchain": onchain_valid,
            "issuerWhitelisted": issuer_whitelisted,
        },
        "verificationId": rec.id,
    }))
}

// --------------------------------------------------------------------------------------------
// records / audit-log reads
// --------------------------------------------------------------------------------------------

async fn list_records(State(st): State<AppState>) -> Resp {
    ok(json!({ "records": st.store.list_credentials().await }))
}

async fn get_record(State(st): State<AppState>, Path(root): Path<String>) -> Resp {
    match st.store.get_credential(&root).await {
        Some(c) => ok(serde_json::to_value(c).unwrap_or(Value::Null)),
        None => err(StatusCode::NOT_FOUND, "no credential for that root"),
    }
}

/// PATCH /v1/records/:root — update the OFF-CHAIN metadata of a credential only. On-chain-derived
/// fields (root, tx hash, block number, contract address, the anchored wrapped doc) are IMMUTABLE:
/// any attempt to set one is rejected with 400. Editable: `label`, `notes`, and `status` (only to
/// `expired`, the off-chain validity lapse — use the revoke endpoint for on-chain invalidation).
async fn update_record(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(root): Path<String>,
    Json(body): Json<Value>,
) -> Resp {
    if let Err(e) = require_api_token(&st, &headers) {
        return e;
    }
    let obj = match body.as_object() {
        Some(o) => o,
        None => return err(StatusCode::BAD_REQUEST, "body must be a JSON object"),
    };
    // Reject any on-chain-derived field — immutable chain state cannot be edited.
    for k in obj.keys() {
        if IMMUTABLE_KEYS.contains(&k.as_str()) {
            return err(
                StatusCode::BAD_REQUEST,
                &format!("field '{k}' is on-chain-derived and immutable"),
            );
        }
    }

    let mut cred = match st.store.get_credential(&root).await {
        Some(c) => c,
        None => return err(StatusCode::NOT_FOUND, "no credential for that root"),
    };

    // label / notes — free-form off-chain metadata (null clears).
    if let Some(v) = obj.get("label") {
        cred.label = v.as_str().map(|s| s.to_string());
    }
    if let Some(v) = obj.get("notes") {
        cred.notes = v.as_str().map(|s| s.to_string());
    }
    // status — only "expired" is a permitted off-chain transition here (soft-invalidation without a
    // chain tx). Reactivation / arbitrary states are not allowed; on-chain revocation has its own path.
    if let Some(v) = obj.get("status") {
        match v.as_str() {
            Some("expired") => {
                if cred.status == CredentialStatus::Revoked {
                    return err(
                        StatusCode::CONFLICT,
                        "credential is revoked on-chain; revoked is terminal and cannot be downgraded to expired",
                    );
                }
                cred.status = CredentialStatus::Expired;
                cred.invalidated_at = Some(now());
                if cred.invalidation_reason.is_none() {
                    cred.invalidation_reason =
                        obj.get("reason").and_then(|r| r.as_str()).map(String::from);
                }
            }
            Some(other) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    &format!("status can only be set to 'expired' via update (got '{other}')"),
                )
            }
            None => {}
        }
    }

    cred.updated_at = now();
    st.store.put_credential(cred.clone()).await;
    ok(serde_json::to_value(cred).unwrap_or(Value::Null))
}

#[derive(Deserialize, Default)]
struct RevokeBody {
    #[serde(default)]
    reason: Option<String>,
}

/// POST /v1/records/:root/revoke — INVALIDATE a credential on-chain (`DogTagIssuer.revoke`). This is a
/// soft-invalidation: the record is NEVER deleted. It flips to `revoked`, keeps its original issuance
/// on-chain proof intact, and gains the revoke tx proof — so it stays historically visible and still
/// verifiable on the block explorer (its `isValid` now reads false).
async fn revoke_record(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(root): Path<String>,
    body: Option<Json<RevokeBody>>,
) -> Resp {
    if let Err(e) = require_api_token(&st, &headers) {
        return e;
    }
    let reason = body.and_then(|Json(b)| b.reason);

    let mut cred = match st.store.get_credential(&root).await {
        Some(c) => c,
        None => return err(StatusCode::NOT_FOUND, "no credential for that root"),
    };
    if cred.status == CredentialStatus::Revoked {
        return err(StatusCode::CONFLICT, "credential already revoked");
    }
    if !cred.anchored {
        return err(
            StatusCode::CONFLICT,
            "credential was never anchored on-chain; nothing to revoke",
        );
    }
    if !st.chain.can_sign() {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "no government signer configured (set GOV_SIGNER_KEY) to revoke on-chain",
        );
    }

    let sent = match st.chain.revoke(&cred.issuer_addr, &root).await {
        Ok(s) => s,
        Err(e) => {
            return err(
                StatusCode::BAD_GATEWAY,
                &format!("on-chain revoke failed: {e}"),
            )
        }
    };

    cred.status = CredentialStatus::Revoked;
    cred.revoked_tx_hash = Some(sent.tx_hash.clone());
    cred.revoked_block_number = sent.block_number;
    cred.revoke_explorer_url = Some(crate::chain::explorer_tx_url(&sent.tx_hash));
    cred.invalidated_at = Some(now());
    cred.invalidation_reason = reason;
    cred.updated_at = now();
    st.store.put_credential(cred.clone()).await;

    ok(serde_json::to_value(cred).unwrap_or(Value::Null))
}

async fn list_verifications(State(st): State<AppState>) -> Resp {
    ok(json!({ "verifications": st.store.list_verifications().await }))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/travel-clearance/issue", post(issue))
        .route("/v1/verify", post(verify))
        .route("/v1/records", get(list_records))
        .route("/v1/records/:root", get(get_record).patch(update_record))
        .route("/v1/records/:root/revoke", post(revoke_record))
        .route("/v1/verifications", get(list_verifications))
        .with_state(state)
}
