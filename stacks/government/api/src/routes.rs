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
    response::{Html, IntoResponse, Response},
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
    // receiptId, the cleartext subject projection and the denormalized validUntil all mirror content
    // committed in the on-chain root R — editing them would desync the DB from the chain.
    "receiptId",
    "receipt_id",
    "subject",
    "validUntil",
    "valid_until",
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

/// Gate for the operator endpoints: issue, PATCH /v1/records/:root, POST /v1/records/:root/revoke,
/// and the operator record reads (GET /v1/records, GET /v1/records/:root) all require
/// `Authorization: Bearer <token>` matching the configured `GOV_API_TOKEN`. Unconfigured token fails
/// closed (503). Health, verify, the verifications audit log, and the public receipt status
/// endpoints (GET /v1/receipts/:id/status and GET /r/:id) stay open.
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

/// Civil (Y, M, D) from a days-since-Unix-epoch count (Howard Hinnant's algorithm — no date crate).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Format a Unix-seconds instant as an ISO-8601 UTC date (`YYYY-MM-DD`). ISO dates compare correctly
/// as plain strings, so `today_iso() > validUntil` is a valid expiry test.
fn iso_date(unix_secs: u64) -> String {
    let (y, m, d) = civil_from_days((unix_secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Today's UTC date (`YYYY-MM-DD`).
fn today_iso() -> String {
    iso_date(now())
}

/// Mint a 12-char Crockford-base32 receipt handle from a CSPRNG (~60 bits — unguessable enough for a
/// status-only endpoint, and NOT derived from any PII preimage). Excludes I/L/O/U (Crockford).
fn gen_receipt_id() -> String {
    const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut bytes = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    bytes
        .iter()
        .map(|b| CROCKFORD[(b & 0x1f) as usize] as char)
        .collect()
}

/// The holder-facing derived status, computed at read time from the stored lifecycle + the validity
/// window (arch §4 / D-derived-EXPIRED): `revoked ? REVOKED : (expired || today > validUntil) ?
/// EXPIRED : VALID`. A never-anchored draft renders as `DRAFT`. Pure (no chain read) — used for the
/// list/detail surfaces; the public status endpoint recomputes it against a LIVE on-chain read.
fn derive_effective_status(
    status: CredentialStatus,
    valid_until: Option<&str>,
    today: &str,
) -> &'static str {
    match status {
        CredentialStatus::Revoked => "REVOKED",
        CredentialStatus::Draft => "DRAFT",
        _ => expired_or_valid(status, valid_until, today),
    }
}

/// The shared EXPIRED-vs-VALID tail: a record is `EXPIRED` when its stored lifecycle already says so
/// or its validity window has passed (`today > validUntil`), else `VALID`. Callers apply their own
/// REVOKED/DRAFT precedence first, then delegate the final decision here so policy lives in one place.
fn expired_or_valid(
    status: CredentialStatus,
    valid_until: Option<&str>,
    today: &str,
) -> &'static str {
    if status == CredentialStatus::Expired || valid_until.map(|vu| today > vu).unwrap_or(false) {
        "EXPIRED"
    } else {
        "VALID"
    }
}

/// Serialize a credential to JSON with the read-time `effectiveStatus` injected (so every record
/// surface renders the derived VALID/EXPIRED/REVOKED verdict, not just the stored lifecycle field).
fn credential_json(cred: &IssuedCredential) -> Value {
    let eff = derive_effective_status(cred.status, cred.valid_until.as_deref(), &today_iso());
    let mut v = serde_json::to_value(cred).unwrap_or(Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.insert("effectiveStatus".to_string(), json!(eff));
    }
    v
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
/// GATED behind the government operator bearer (`GOV_API_TOKEN`) — an authority portal that anyone
/// could issue from would undermine the receipt's credibility (arch DP-6). Reads/verify/public
/// status stay open; demo mode keeps the baked demo token so `demo-up` flows still work.
async fn issue(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<IssueBody>,
) -> Resp {
    if let Err(e) = require_api_token(&st, &headers) {
        return e;
    }
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

    // Mint a unique public receipt handle (CSPRNG Crockford-base32). Retry on the astronomically
    // unlikely collision against an existing record so the unique index is never violated.
    let mut receipt_id = gen_receipt_id();
    for _ in 0..8 {
        if st
            .store
            .get_credential_by_receipt_id(&receipt_id)
            .await
            .is_none()
        {
            break;
        }
        receipt_id = gen_receipt_id();
    }

    // BUILD (server-side, shared open standard): typed leaves -> single Poseidon root R. The
    // receiptId is committed into R as a public salted leaf AND kept as the off-chain lookup handle.
    let vc = app::build_gov_vc(
        &st.cfg,
        &body.record_type,
        &body.fields,
        &body.dog_tag_id,
        &receipt_id,
    );
    // Denormalize the cleartext subject + validUntil for list/detail/status rendering (both mirror
    // content committed in R, hence IMMUTABLE — see IMMUTABLE_KEYS).
    let subject = vc.get("credentialSubject").cloned().unwrap_or(Value::Null);
    let valid_until = subject
        .get("validity")
        .and_then(|v| v.get("validUntil"))
        .or_else(|| subject.get("validUntil"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let meta = app::issuer_meta(&st.cfg, &body.record_type, &issuer_addr);
    let mut doc = match app::wrap(meta, &vc) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::BAD_REQUEST, &e),
    };
    // Stamp the M7 provenance block (§4.2), BESIDE R. This authority anchors server-side, so its
    // issuing signer (`signer_address`) is the authoritative on-chain `issuedBy[R]`; empty on dry-run
    // (no signer / Draft).
    let issuer_signer = st.chain.signer_address().unwrap_or_default();
    doc.protocol = Some(app::protocol_meta(&st.cfg, &issuer_addr, &issuer_signer));
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
        // M7 provenance mirror (§4.2), from the `protocol` block just stamped on `doc`.
        chain_id: Some(st.cfg.chain_id),
        protocol_version: Some(dogtag_standard::wrap::LEVEL_B_VERSION.to_string()),
        verification_registry: Some(st.cfg.verification_registry_addr.clone()),
        issuer_signer: if issuer_signer.is_empty() {
            None
        } else {
            Some(issuer_signer.clone())
        },
        receipt_id: Some(receipt_id.clone()),
        subject,
        valid_until,
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
        "receiptId": receipt_id,
        "issuerAddr": issuer_addr,
        "anchored": anchored,
        "txHash": tx_hash,
        "blockNumber": block_number,
        "explorerUrl": explorer_url,
        "statusUrl": format!("/v1/receipts/{receipt_id}/status"),
        "receiptUrl": format!("/r/{receipt_id}"),
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

async fn list_records(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_api_token(&st, &headers) {
        return e;
    }
    let records: Vec<Value> = st
        .store
        .list_credentials()
        .await
        .iter()
        .map(credential_json)
        .collect();
    ok(json!({ "records": records }))
}

async fn get_record(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(root): Path<String>,
) -> Resp {
    if let Err(e) = require_api_token(&st, &headers) {
        return e;
    }
    match st.store.get_credential(&root).await {
        Some(c) => ok(credential_json(&c)),
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

    // label / notes — free-form off-chain metadata (null clears; any other non-string JSON type is
    // rejected rather than silently clearing).
    for key in ["label", "notes"] {
        if let Some(v) = obj.get(key) {
            if !v.is_null() && !v.is_string() {
                return err(
                    StatusCode::BAD_REQUEST,
                    &format!("{key} must be a string or null"),
                );
            }
        }
    }
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
                if cred.status != CredentialStatus::Issued {
                    return err(
                        StatusCode::CONFLICT,
                        "only issued credentials can be expired",
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
    if reason.is_some() {
        cred.invalidation_reason = reason;
    }
    cred.updated_at = now();
    st.store.put_credential(cred.clone()).await;

    ok(serde_json::to_value(cred).unwrap_or(Value::Null))
}

async fn list_verifications(State(st): State<AppState>) -> Resp {
    ok(json!({ "verifications": st.store.list_verifications().await }))
}

// --------------------------------------------------------------------------------------------
// Government oversight console (govarch PR-5) — the UNSCOPED cross-issuer activity feed from the
// oversight indexer, joined to the government's OWN issued credentials. See `crate::oversight` +
// `crate::trace`. API-token-gated (the authority's own console).
// --------------------------------------------------------------------------------------------

/// Map an oversight-feed error to an HTTP response: an unconfigured indexer is a 503 (the oversight
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

/// Query params for `GET /v1/oversight/activity` — pass-through narrowing filters over the UNSCOPED
/// feed. Each only ever shrinks the (already cross-issuer) result set.
#[derive(Debug, Deserialize, Default)]
struct OversightParams {
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

impl From<OversightParams> for crate::oversight::FeedQuery {
    fn from(p: OversightParams) -> Self {
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

/// `GET /v1/oversight/activity` — the UNSCOPED cross-issuer on-chain activity feed, joined to the
/// government's own issued credentials (its own activity is highlighted; every other issuer's is shown
/// too). API-token-gated.
async fn oversight_activity(
    State(st): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(p): axum::extract::Query<OversightParams>,
) -> Resp {
    if let Err(e) = require_api_token(&st, &headers) {
        return e;
    }
    let q: crate::oversight::FeedQuery = p.into();
    let mut body = match st.feed.events(&q).await {
        Ok(b) => b,
        Err(e) => return feed_err(e),
    };
    let idx = crate::trace::build_index(st.store.as_ref()).await;
    let events = body
        .get("events")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let joined = crate::trace::join_events(events, &idx);
    if let Value::Object(map) = &mut body {
        map.insert("events".into(), json!(joined.events));
        // `matched` = how many cross-issuer events are the government's OWN credentials.
        map.insert("matched".into(), json!(joined.matched));
    }
    ok(body)
}

/// `GET /v1/oversight/stats` — cross-issuer aggregate counters from the indexer, plus the government's
/// own record counts. API-token-gated.
async fn oversight_stats(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_api_token(&st, &headers) {
        return e;
    }
    let mut body = match st.feed.stats().await {
        Ok(b) => b,
        Err(e) => return feed_err(e),
    };
    let creds = st.store.list_credentials().await;
    let verifs = st.store.list_verifications().await;
    if let Value::Object(map) = &mut body {
        map.insert(
            "local".into(),
            json!({ "credentials": creds.len(), "verifications": verifs.len() }),
        );
    }
    ok(body)
}

/// `GET /v1/oversight/issuers` — the deployed `DogTagIssuer` clones across ALL issuers with per-clone
/// issued/revoked/active counts. API-token-gated.
async fn oversight_issuers(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_api_token(&st, &headers) {
        return e;
    }
    match st.feed.issuers().await {
        Ok(body) => ok(body),
        Err(e) => feed_err(e),
    }
}

// --------------------------------------------------------------------------------------------
// public receipt status (PII-free) — resolves receiptId -> R, LIVE on-chain read
// --------------------------------------------------------------------------------------------

/// The resolved, live-on-chain status of a receipt. `effective_status` is recomputed against a LIVE
/// `DogTagIssuer.isValid(R)` read (NOT a DB echo), so a fresh on-chain revocation shows instantly.
struct ReceiptStatus {
    cred: IssuedCredential,
    effective_status: &'static str,
    /// On-chain `issuedAt[R]` (Unix seconds), 0 when not anchored — the authoritative issuance date.
    issued_at: u64,
}

/// Shared resolver for the two public receipt surfaces: look up the receipt, read the chain live, and
/// fold to a derived VALID/EXPIRED/REVOKED verdict. `Err` carries a ready HTTP error response.
async fn resolve_receipt_status(st: &AppState, receipt_id: &str) -> Result<ReceiptStatus, Resp> {
    let cred = st
        .store
        .get_credential_by_receipt_id(receipt_id)
        .await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "no receipt with that id"))?;

    // LIVE on-chain reads (gasless): isValid(R) is authoritative for revocation; issuedAt(R) is the
    // authoritative issuance instant (arch DP-2 — issuance date derived from the chain, not a leaf).
    let onchain_valid = st
        .chain
        .is_valid(&cred.issuer_addr, &cred.root)
        .await
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                &format!("on-chain isValid read failed: {e}"),
            )
        })?;
    let issued_at = st
        .chain
        .issued_at(&cred.issuer_addr, &cred.root)
        .await
        .map(|u| u.try_into().unwrap_or(0u64))
        .unwrap_or(0);

    // revoked is driven by the LIVE read (an anchored root that no longer isValid was revoked), so a
    // fresh revoke wins over the possibly-stale stored lifecycle field.
    let today = today_iso();
    let revoked = cred.status == CredentialStatus::Revoked || (cred.anchored && !onchain_valid);
    let effective_status = if !cred.anchored {
        "DRAFT"
    } else if revoked {
        "REVOKED"
    } else {
        expired_or_valid(cred.status, cred.valid_until.as_deref(), &today)
    };

    Ok(ReceiptStatus {
        cred,
        effective_status,
        issued_at,
    })
}

/// GET /v1/receipts/:receiptId/status — PUBLIC, PII-free JSON status of a receipt via a LIVE on-chain
/// `isValid(R)` read. Carries only non-PII fields (verdict + validity window + provenance links).
async fn receipt_status(State(st): State<AppState>, Path(receipt_id): Path<String>) -> Resp {
    let rs = match resolve_receipt_status(&st, &receipt_id).await {
        Ok(rs) => rs,
        Err(e) => return e,
    };
    let c = &rs.cred;
    let issuance_date = (rs.issued_at != 0).then(|| iso_date(rs.issued_at));
    ok(json!({
        "effectiveStatus": rs.effective_status,
        "recordType": c.record_type,
        "receiptId": receipt_id,
        "validUntil": c.valid_until,
        "issuanceDate": issuance_date,
        "root": c.root,
        "issuerAddr": c.issuer_addr,
        "explorerUrl": c.explorer_url,
        "revokeExplorerUrl": c.revoke_explorer_url,
        "checkedAt": now(),
    }))
}

/// Minimal HTML-escape for the few dynamic strings rendered into the public status page.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render a minimal, PII-free error page for the public `/r/:receiptId` surface, preserving the
/// upstream StatusCode so a chain-read failure (5xx) is never conflated with a missing receipt (404).
fn receipt_error_page(code: StatusCode, title: &str, message: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{t}</title></head>\
         <body style=\"font-family:system-ui,sans-serif;max-width:34rem;margin:4rem auto;padding:0 1rem\">\
         <h1>{t}</h1><p>{m}</p></body></html>",
        t = esc(title),
        m = esc(message),
    );
    (code, Html(html)).into_response()
}

/// GET /r/:receiptId — PUBLIC status page (status-only by default, arch DP-5). Renders the live
/// verdict + validity window + on-chain provenance links. NO Section A/B/C content — the official
/// reads the details off the paper/phone in front of them and this page confirms the receipt is
/// genuine and current. Enumeration is bounded by the ~60-bit receiptId.
async fn receipt_page(State(st): State<AppState>, Path(receipt_id): Path<String>) -> Response {
    let rs = match resolve_receipt_status(&st, &receipt_id).await {
        Ok(rs) => rs,
        Err((code, Json(body))) => {
            let msg = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("not found");
            if code == StatusCode::NOT_FOUND {
                return receipt_error_page(code, "Receipt not found", msg);
            }
            return receipt_error_page(
                code,
                "Status temporarily unavailable",
                "We could not reach the chain to confirm this receipt right now. \
                 This does NOT mean the receipt is invalid - please try again in a moment.",
            );
        }
    };
    let c = &rs.cred;
    let (accent, label) = match rs.effective_status {
        "VALID" => ("#16a34a", "VALID"),
        "EXPIRED" => ("#d97706", "EXPIRED"),
        "REVOKED" => ("#dc2626", "REVOKED"),
        other => ("#64748b", other),
    };
    let issuance = if rs.issued_at != 0 {
        iso_date(rs.issued_at)
    } else {
        "—".to_string()
    };
    let valid_until = c.valid_until.clone().unwrap_or_else(|| "—".to_string());
    let issue_link = c
        .explorer_url
        .as_deref()
        .map(|u| {
            format!(
                "<a href=\"{}\" rel=\"noopener noreferrer\">issue tx</a>",
                esc(u)
            )
        })
        .unwrap_or_default();
    let revoke_link = c
        .revoke_explorer_url
        .as_deref()
        .map(|u| {
            format!(
                " · <a href=\"{}\" rel=\"noopener noreferrer\">revoke tx</a>",
                esc(u)
            )
        })
        .unwrap_or_default();

    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>Receipt {rid} — {label}</title>\
         <style>body{{font-family:system-ui,-apple-system,Segoe UI,sans-serif;background:#f8fafc;color:#0f172a;margin:0;padding:2rem 1rem}}\
         .card{{max-width:34rem;margin:0 auto;background:#fff;border:1px solid #e2e8f0;border-radius:14px;padding:1.5rem 1.75rem;box-shadow:0 1px 3px rgba(0,0,0,.06)}}\
         .status{{display:inline-block;font-weight:700;font-size:1.05rem;color:#fff;background:{accent};padding:.35rem .9rem;border-radius:999px;letter-spacing:.03em}}\
         h1{{font-size:1.05rem;color:#475569;font-weight:600;margin:0 0 1rem}}\
         dl{{display:grid;grid-template-columns:auto 1fr;gap:.4rem 1rem;margin:1.25rem 0 0;font-size:.94rem}}\
         dt{{color:#64748b}}dd{{margin:0;font-variant-numeric:tabular-nums;word-break:break-all}}\
         .mono{{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.82rem}}\
         .foot{{margin-top:1.25rem;font-size:.8rem;color:#94a3b8}}a{{color:#2563eb}}</style></head>\
         <body><div class=\"card\">\
         <h1>Pet Travel Clearance Receipt</h1>\
         <span class=\"status\">● {label}</span>\
         <dl>\
         <dt>Receipt ID</dt><dd class=\"mono\">{rid}</dd>\
         <dt>Record type</dt><dd>{rtype}</dd>\
         <dt>Date of issuance</dt><dd>{issuance}</dd>\
         <dt>Valid until</dt><dd>{valid_until}</dd>\
         <dt>Credential root</dt><dd class=\"mono\">{root}</dd>\
         <dt>Anchored on</dt><dd>ROAX chainId {chain} · {issue_link}{revoke_link}</dd>\
         </dl>\
         <p class=\"foot\">Live on-chain status check · no personal data shown · checked {checked} (UTC epoch s).</p>\
         </div></body></html>",
        rid = esc(&receipt_id),
        rtype = esc(&c.record_type),
        root = esc(&c.root),
        chain = st.cfg.chain_id,
        checked = now(),
    );
    Html(html).into_response()
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
        // oversight console (govarch PR-5): unscoped cross-issuer activity joined to own credentials
        .route("/v1/oversight/activity", get(oversight_activity))
        .route("/v1/oversight/stats", get(oversight_stats))
        .route("/v1/oversight/issuers", get(oversight_issuers))
        // PUBLIC (no auth): PII-free receipt status JSON + human status page (live on-chain read).
        .route("/v1/receipts/:receipt_id/status", get(receipt_status))
        .route("/r/:receipt_id", get(receipt_page))
        .with_state(state)
}
