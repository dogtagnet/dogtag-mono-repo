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
//!   POST /v1/records/:root/share         mint a one-time record-share QR token (owner's phone import).
//!   GET  /v1/verifications               list the verification audit log.
//!
//! PHONE-FACING surface. These four paths are HARD-CODED in both mobile apps
//! (`apps/android/.../CentralApi.kt`, `apps/ios/DogTag/Net.swift`) against whichever verifier host the
//! QR names, so they are byte-identical to vet-api's and must never be renamed or `/v1`-prefixed:
//!   GET  /r/:token                       resolve+CONSUME a share token -> the wrapped doc (import).
//!   GET  /x/:token                       resolve a verify-session export token (non-consuming).
//!   POST /v1/verify/consent              submit the owner-hidden consent proof.
//!   GET  /verify/session/:id             poll a verify session (operator bearer OR ?token=).
//! `GET /r/:id` is OVERLOADED: a 32-hex segment is a share token (JSON), anything else is a public
//! receipt id (the PII-free HTML status page). The two id shapes cannot collide — see `is_share_token`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use dogtag_standard::issuer_identity::{assert_issuer_domain, IssuerDomainAssertion};
use dogtag_standard::verify::{check_integrity, FragmentState};
use dogtag_standard::wrap::WrappedDoc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{self, AppState};
use crate::store::{CredentialStatus, IssuedCredential, VerificationRecord, VerifySession};

/// The "not configured" address. An address field left at zero means the deployment cannot make the
/// read at all — which is `unavailable`, never a negative answer.
const ZERO_ADDR: &str = "0x0000000000000000000000000000000000000000";

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

/// The presented `Authorization: Bearer <token>`, if any.
fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

/// Dual gate for the two PHONE-facing session routes (`POST /v1/verify/consent`,
/// `GET /verify/session/:id`): the government operator bearer OR a short-lived export token scoped to
/// THIS session. Returns `true` when the caller was authed by the export token.
///
/// This deliberately does NOT delegate to `require_api_token`: that fails closed with 503 when
/// `GOV_API_TOKEN` is unconfigured, and the owner's phone authenticates with its export token alone —
/// a deployment without an operator token must still let the owner submit and poll their own proof.
async fn require_operator_or_export_token(
    st: &AppState,
    headers: &HeaderMap,
    session_id: &str,
    body_token: Option<&str>,
) -> Result<bool, Resp> {
    // Operator bearer first (the portal). Only ever satisfied against a CONFIGURED token.
    if let (Some(expected), Some(presented)) = (st.cfg.api_token.as_deref(), bearer(headers)) {
        if presented == expected {
            return Ok(false);
        }
    }
    // Otherwise a short-lived export token (the owner's phone), from the body field or the Bearer header.
    let token = body_token
        .map(|s| s.to_string())
        .or_else(|| bearer(headers))
        .ok_or_else(|| {
            err(
                StatusCode::UNAUTHORIZED,
                "missing operator session or export token",
            )
        })?;
    let mapped = st
        .store
        .peek_export_token(&token)
        .await
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "export token missing or expired"))?;
    if mapped != session_id {
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "export token does not match session",
        ));
    }
    Ok(true)
}

/// Monotonic-ish wall clock (seconds). Government records are audit metadata, not consensus-critical.
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Mint a SHORT one-time QR token: 32 hex chars == 16 CSPRNG bytes. Short so the QR stays low-density
/// and a phone camera can focus on it instantly. Same shape as vet-api's share/export/bind tokens, so
/// the phone's URL parser (`QrPayload`) recognises it without a government-specific branch.
fn gen_qr_token() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    hex::encode(bytes)
}

/// Is this `/r/:id` path segment a record-share TOKEN rather than a public receipt id?
///
/// Share tokens are exactly 32 lowercase hex chars (16 random bytes); receipt ids are 12 Crockford
/// base32 chars (`gen_receipt_id`, uppercase, no I/L/O/U). The two shapes cannot collide — they differ
/// in length — so one path can serve both the phone's JSON import and the public HTML status page.
fn is_share_token(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// TTL of a record-share token (seconds). Matches vet-api: long enough to walk a phone over, short
/// enough that a photographed QR is worthless by the time anyone tries it.
const SHARE_TOKEN_TTL_SECS: u64 = 180;

/// TTL of a verify-session export token (seconds). Longer than a share token because this path includes
/// a slow on-device Groth16 proof (tens of seconds) plus the owner's own approval steps; a 180s token
/// would expire mid-flow. Replay is bounded by the session status guard and the on-chain nullifier, not
/// by this TTL. Matches vet-api.
const EXPORT_TOKEN_TTL_SECS: u64 = 600;

/// The QR base URL the PHONE must be able to reach — `DEPLOYMENT_URL`, never the API's own bind
/// address. On a LAN demo it is the host's LAN IP, in production the public domain (or tunnel).
/// A configured trailing slash is trimmed so the minted URL never contains `//r/`.
fn qr_base(st: &AppState) -> &str {
    st.cfg.deployment_url.trim_end_matches('/')
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

/// `GET /health` — liveness plus, critically, an HONEST statement of which chain surface is in use.
///
/// The reported `chainId` is read from the CHAIN CLIENT, never from `CHAIN_ID` config: a simulated
/// backend used to inherit the configured `135` and pair it with `canSign:true`, so a simulated stack
/// was indistinguishable from live ROAX in the one place an operator looks. Now:
///   - `backend`   — "live" | "simulated" (authoritative)
///   - `simulated` — the same fact as a boolean, for UI badges
///   - `chainId`   — the real id when live, `null` when simulated (never a network it is not on)
///   - `canSign`   — true only when a REAL tx from a REAL key would land on a REAL chain
///   - `signer` / `simulatedSigner` — a real signer address is never conflated with a stand-in
///
/// `demo` is retained but now means only what it actually controls: the ephemeral store and the
/// relaxed API-token fallback. It is NOT a statement about the chain.
async fn health(State(st): State<AppState>) -> Resp {
    let simulated = st.chain.is_simulated();
    let signer = st.chain.signer_address();
    ok(json!({
        "status": "ok",
        "service": "government-api",
        "backend": st.chain.backend().as_str(),
        "simulated": simulated,
        // Null rather than a real id when simulated — this backend is on no network at all.
        "chainId": (!simulated).then(|| st.chain.chain_id()),
        "demo": st.cfg.demo,
        "canSign": st.chain.can_broadcast_real_tx(),
        "signer": (!simulated).then(|| signer.clone()).flatten(),
        // The MemChain stand-in address, surfaced under a name that cannot be mistaken for a real key.
        "simulatedSigner": simulated.then_some(signer).flatten(),
        "issuers": {
            app::TRAVEL_CLEARANCE: st.cfg.issuer_addr_for(app::TRAVEL_CLEARANCE),
            app::EU_HEALTH_CERT: st.cfg.issuer_addr_for(app::EU_HEALTH_CERT),
        },
        // Surfaced so the two addresses the owner-hidden verify path depends on are eyeball-able. The
        // phone independently resolves `verificationRegistry` from the on-chain ProtocolRegistry and
        // REFUSES to prove if this deployment claims a different one, so a mismatch here is the single
        // likeliest cause of an otherwise-opaque scan failure.
        "issuerRegistry": st.cfg.issuer_registry_addr,
        "verificationRegistry": st.cfg.verification_registry_addr,
        // The host the QR codes point at — must be reachable from the OWNER'S PHONE, not just this box.
        "deploymentUrl": st.cfg.deployment_url,
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
        // M7 provenance mirror (§4.2). Read from the CHAIN CLIENT, never from `CHAIN_ID` config -
        // config is what let a simulated backend stamp `135` on every record it issued. `None` on a
        // simulated backend says "anchored on no real network", which is the truth.
        //
        // The envelope's `protocol.chainId` (`ProtocolMeta`) deliberately still carries the configured
        // id: it is a non-optional `u64` in the shared standard crate with no null representation, so
        // making it honest means inventing a sentinel in a cross-language type. This column is the
        // queryable, honest one.
        chain_id: (!st.chain.is_simulated()).then(|| st.chain.chain_id()),
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
    /// OPTIONAL *expected* DogTagIssuer clone. It can only TIGHTEN: the clone every read is made
    /// against is always the one the FACTORY names for this root, so this asserts an expectation
    /// rather than selecting a contract. This route is unauthenticated, so a caller able to pick the
    /// contract that answers for a credential would reopen the forgery this pillar exists to close.
    #[serde(default)]
    issuer_addr: Option<String>,
    /// OPTIONAL *expected* issuing signer. The issuer-whitelist pillar no longer depends on this: the
    /// signer is resolved from the chain (`issuedBy(root)`). Supplying it adds a strictly stronger
    /// assertion - the pillar fails when the on-chain originator is not this address.
    #[serde(default)]
    signer_addr: Option<String>,
}

/// Government verifier: integrity (offline recompute) + on-chain status + issuer-identity, folded to
/// a single verdict, recorded to the audit log. All chain reads are gasless.
///
/// This route is deliberately OPEN (verification is permissionless), so its chain-read cost is a
/// capacity question: the reads share one connection (`AlloyChain::provider`) and the independent ones
/// are issued concurrently, but `issuer_addr` derives from a caller-supplied value, so nothing caches
/// across callers. A rate limit in front of it is worth deciding on separately; it is not added here.
async fn verify(State(st): State<AppState>, Json(body): Json<VerifyBody>) -> Resp {
    let doc = body.wrapped_doc;
    let record_type = doc.issuer.record_type.clone();
    let claimed_root = doc.signature.merkle_root.clone();

    // Pin the WHOLE verification to one block, read once up front. Against a mutable world (DNS changes,
    // clones get superseded) a verdict without a block anchor is not auditable, and reads smeared across
    // several heights are not a consistent snapshot. `None` means the head could not be read — every
    // dependent read then falls back to `latest` and the answer is reported as unanchored rather than
    // pretending to a block it never saw.
    let at_block = st.chain.block_number().await.ok();

    // WHICH contract issued this credential. `issuer.documentStore` is only the document's CLAIM, and
    // pointing it at a contract the attacker controls is the sharper form of the relabelling attack. The
    // factory's write-once `rootIssuer[R]` is authoritative, and it names the clone that issued THIS
    // root — so an old credential resolves against the clone that issued it, never against a successor.
    let factory_cfg = st.cfg.factory_addr.trim().to_string();
    // A failed READ and "the factory has no record of this root" are different facts and stay apart all
    // the way to the wire — the same rule the DNS states obey. Collapsing them (the old `.ok().flatten()`)
    // made an RPC blip indistinguishable from a root that was never registered through a real clone.
    // The FALLBACK is unchanged in both cases (the document's claim is all that is left), so only what
    // gets REPORTED differs.
    let (resolved_issuer, root_issuer_read, root_issuer_read_detail) =
        if factory_cfg.is_empty() || factory_cfg.eq_ignore_ascii_case(ZERO_ADDR) {
            (None, "noFactoryConfigured", None)
        } else {
            match st
                .chain
                .root_issuer(&factory_cfg, &claimed_root, at_block)
                .await
            {
                Ok(Some(a)) => (Some(a), "resolved", None),
                Ok(None) => (None, "noRecord", None),
                Err(e) => (None, "readFailed", Some(format!("{e}"))),
            }
        };
    let document_issuer = doc.issuer.document_store.trim().to_lowercase();
    // An explicit override still wins (operators use it to check a document against a specific clone),
    // then the authoritative on-chain resolution, then the document's claim as a last resort.
    let issuer_addr = body
        .issuer_addr
        .clone()
        .or_else(|| resolved_issuer.clone())
        .unwrap_or_else(|| doc.issuer.document_store.clone());
    let issuer_store_differs = resolved_issuer
        .as_deref()
        .map(|r| r.to_lowercase() != document_issuer)
        .unwrap_or(false);

    // LINK 1, resolved ONCE for the whole verification and pinned to `at_block`, because everything that
    // reads an IDENTITY off that contract has to sit behind it. Being on-chain is not the property that
    // matters; being FACTORY-DESCENDED is — anyone can deploy a contract that returns whatever `name()`
    // they like, and `issuer_addr` falls back to the document's own `documentStore` whenever the factory
    // has no record of the root. Reading it once also means the name gate and the domain binding cannot
    // disagree with each other within a single answer.
    let provenance = resolve_clone_provenance(&st, &issuer_addr, at_block).await;

    // 1) integrity — recompute the root from the salted leaves and compare (offline, no chain).
    let (integrity_state, recomputed) = check_integrity(&doc);
    let integrity_valid = integrity_state == FragmentState::Valid;
    let recomputed_hex = dogtag_standard::to_hex32(&recomputed);

    // 2/3/4) the remaining reads, ISSUED CONCURRENTLY.
    //
    // Everything below depends on `issuer_addr` and on `provenance`, both already resolved above, and on
    // nothing else — so running them serially bought no ordering the security model needs, while costing
    // an unauthenticated caller four sequential round trips against our own node. They stay pinned to the
    // SAME `at_block`, so the batch is still one consistent snapshot.
    //
    // The security-critical ordering is preserved BY CONSTRUCTION: link-1 provenance is established
    // before this point and is passed IN, so no identity read here can precede it.
    //
    // (a) Has the document been relabelled? `issuer.domain`/`issuer.name` sit OUTSIDE the Merkle root,
    //     so a genuine credential can be re-badged as any authority and still pass integrity + isValid.
    //     The root-covered `data.issuer` DID is the true identity; assert the two agree. (Offline.)
    let did_assertion = assert_issuer_domain(&doc);

    let rt_key = app::record_type_key(&record_type);
    let (onchain_valid, whitelist_pillar, onchain_name, issuer_domain_json) = tokio::join!(
        // on-chain status — DogTagIssuer.isValid(root) over ROAX (gasless read).
        //
        // Read against the FACTORY-RESOLVED clone whenever the factory has a record of this root, and
        // only fall back to `issuer_addr` (which honours an operator override, then the document's own
        // claim) when it does not. Without that preference an `issuer_addr` override SELECTS which
        // contract answers the issuance pillar, so a hostile contract vouches for the very credential
        // that names it — the same "asking the suspect for its own references" failure the issuer
        // pillar closes, reached through a different door.
        st.chain.is_valid(
            resolved_issuer.as_deref().unwrap_or(&issuer_addr),
            &claimed_root,
            at_block,
        ),
        // issuer identity — MANDATORY, and SELF-RESOLVING.
        //
        // This used to run only when an operator typed a signer in, and to default to a pass when they
        // did not: `issuer_whitelisted.unwrap_or(true)`. That is how the audit's relabelled credential
        // returned `verdict: true` with `issuerWhitelisted: null` - the one pillar that catches a forged
        // `issuer` block simply never ran.
        //
        // It now asks the chain who issued the root - `issuedBy`, set to `msg.sender` under
        // `onlyWhitelisted` - and checks THAT signer against THIS deployment's configured registry.
        // Never an address named by the document, or the attacker supplies both sides of the question.
        // The read is made against `issuer_addr`, which upstream already resolved through the factory's
        // `rootIssuer`, so a hostile contract cannot answer on its own behalf either.
        //
        // Tri-state, and only a definite `true` may contribute to a pass:
        //   Some(true)  - resolved, and whitelisted for this record type
        //   Some(false) - resolved, but not whitelisted (or not the expected signer): a real failure
        //   None        - unresolvable (this clone never issued this root): INDETERMINATE, never a pass
        async {
            // EVERY read here is made against the FACTORY-RESOLVED clone, never `issuer_addr`.
            // `issuer_addr` honours an operator override and falls back to the document's own
            // `documentStore`, so using it would let the caller choose which contract answers the
            // question about their own credential - the "asking the suspect for its references"
            // failure this pillar exists to close. No factory record => indeterminate, never a pass.
            let Some(clone) = resolved_issuer.clone() else {
                return Ok::<_, crate::chain::ChainError>((None, None));
            };
            let signer = st.chain.issued_by(&clone, &claimed_root, at_block).await?;
            let Some(signer) = signer else {
                return Ok::<_, crate::chain::ChainError>((None, None));
            };
            // Ask about the record type the CLONE says it issues, not the one the envelope claims:
            // otherwise a credential relabelled across record types is checked against the wrong
            // whitelist key and a signer authorised for one type appears authorised for another.
            let Some(chain_rt_key) = st.chain.issuer_record_type(&clone, at_block).await? else {
                return Ok::<_, crate::chain::ChainError>((Some(signer), None));
            };
            if !chain_rt_key.eq_ignore_ascii_case(&rt_key) {
                return Ok::<_, crate::chain::ChainError>((Some(signer), Some(false)));
            }
            let whitelisted = st
                .chain
                .is_whitelisted_for(&st.cfg.issuer_registry_addr, &chain_rt_key, &signer, at_block)
                .await?;
            // An explicitly expected signer only ever makes the pillar STRICTER - it can tighten, never
            // enable. Supplying one is now an assertion, not the thing that switches the check on.
            let matches_expected = body
                .signer_addr
                .as_deref()
                .map(|want| want.trim().eq_ignore_ascii_case(&signer))
                .unwrap_or(true);
            Ok((Some(signer), Some(whitelisted && matches_expected)))
        },
        //     The DID only carries a domain, so a name-only relabel slips past it. The clone's own
        //     `name()` was written by the factory's `onlyOwner` `createIssuer` at KYC time, which makes
        //     it the one authoritative issuer name available — but ONLY because `createIssuer` is where
        //     it came from. So the read is gated on link 1 being a DEFINITE yes: a contract we have not
        //     proven descends from the factory has no authoritative name to offer, and reading one
        //     anyway is precisely how a fabricated authority reaches a surface labelled "from the
        //     issuing contract". Neither a definite no nor an unread provenance qualifies. `None`
        //     therefore means "no authoritative name", and `onchainNameAvailable` says so, so a client
        //     falls back to the document value and labels it.
        async {
            if provenance.is_factory_deployed() {
                // `None` here means the read itself failed — reported as such, never silently
                // substituted with the document's claim.
                st.chain
                    .issuer_onchain_name(&issuer_addr, at_block)
                    .await
                    .ok()
            } else {
                None
            }
        },
        // (b) Which domain does the ISSUING CONTRACT itself claim, and does that domain's DNS zone name
        //     this contract back? The claim is read from the chain, never from the document — that is
        //     what makes it unforgeable by relabelling. The DNS half is resolved SERVER-SIDE (see
        //     AppState::dns).
        //
        //     Four outcomes are kept distinct all the way to the client, and none of them is a boolean:
        //       - no registry configured / read failed  -> unavailable  (we do not know)
        //       - registry read, no claim               -> noDomainClaimed (normal on day one)
        //       - claim + DNS record present            -> verified, with the domain's own description
        //       - claim + DNS says absent / unreachable  -> notListed / couldNotCheck
        resolve_issuer_domain_binding(&st, &issuer_addr, at_block, &provenance),
    );
    let onchain_valid = match onchain_valid {
        Ok(v) => v,
        Err(e) => {
            return err(
                StatusCode::BAD_GATEWAY,
                &format!("on-chain rootIssuer read failed: {e}"),
            )
        }
    };
    let (resolved_signer, issuer_whitelisted) = match whitelist_pillar {
        Ok(v) => v,
        Err(e) => {
            return err(
                StatusCode::BAD_GATEWAY,
                &format!("on-chain issuer-whitelist read failed: {e}"),
            )
        }
    };

    // Verdict: integrity + on-chain issuance + the issuer-whitelist pillar must ALL pass
    // (architecture §5 authenticity pillars).
    //
    // `issuer_whitelisted` is compared against `Some(true)` rather than defaulted with
    // `unwrap_or(true)`: a pillar that could not be RESOLVED is an indeterminate verdict, never a
    // pass. That default is what let the audit's relabelled credential return `verdict: true` with
    // `issuerWhitelisted: null` - an unanswered check counted as a passed one.
    //
    // A positive DID MISMATCH also fails the verdict: it is proof the issuer block was rewritten after
    // issuance, which is exactly the attack the audit demonstrated. `NotAssertable` deliberately does
    // NOT fail it and does NOT contribute a pass either - a document that simply carries no root-covered
    // DID is not evidence of forgery, and the surfaces report it as un-asserted rather than verified.
    //
    // So does a DEFINITE link-1 failure. `onchain_valid` above was read from `issuer_addr`, which falls
    // back to the document's own `documentStore` whenever the factory has no record of the root - so a
    // contract the attacker deployed can answer `isValid` however it likes. Reporting
    // `notFactoryDeployed` beside `verdict: true` is worse than not checking at all: it is
    // checked, failed, and passed anyway. Only the DEFINITE negative fails; `Unknown` (no factory
    // configured, or the read failed) is evidence of nothing and leaves the verdict alone.
    let verdict = integrity_valid
        && onchain_valid
        && issuer_whitelisted == Some(true)
        && !did_assertion.is_mismatch()
        && !provenance.is_definitely_not_factory_deployed();

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
        // THE block anchor for every on-chain read in this response. `null` means the head could not be
        // read, in which case the reads used `latest` and this answer is not reproducible — say so
        // rather than implying a block.
        "blockNumber": at_block,
        "issuerResolution": {
            // How `issuerAddr` was chosen. "rootIssuer" is the authoritative path.
            "source": if body.issuer_addr.is_some() {
                "operatorOverride"
            } else if resolved_issuer.is_some() {
                "rootIssuer"
            } else {
                "documentClaim"
            },
            // WHY the `rootIssuer` lookup produced what it did. "noRecord" (the factory has no record of
            // this root) and "readFailed" (we could not ask) both land on the document's claim, but they
            // are different facts and a caller must be able to tell them apart — a failed read is not
            // evidence of absence.
            "rootIssuerRead": root_issuer_read,
            "rootIssuerReadDetail": root_issuer_read_detail,
            "rootIssuer": resolved_issuer,
            "documentDocumentStore": document_issuer,
            // The document names a different contract than the one the chain says issued this root.
            "documentStoreDiffers": issuer_store_differs,
        },
        // The signer the CHAIN says issued this root, never one the caller supplied. `null` means
        // the clone never issued it, which is why the whitelist pillar could not be resolved.
        "signerAddr": resolved_signer,
        "fragments": {
            "integrity": integrity_valid,
            "onchain": onchain_valid,
            "issuerWhitelisted": issuer_whitelisted,
            // "match" | "mismatch" | "notAssertable" — never a boolean, so a client cannot read an
            // un-assertable identity as a verified one.
            "issuerDidAssertion": did_assertion.as_str(),
            // Link 1, as a pillar rather than only as identity metadata: "factoryDeployed" |
            // "notFactoryDeployed" | "unknown". Only the middle one fails the verdict, so a client
            // reading `verdict: false` can see WHICH pillar fell without re-deriving it.
            "issuerProvenance": provenance.as_str(),
        },
        // The issuer identity a surface should RENDER — on-chain first, document only as a diff.
        "issuerIdentity": issuer_identity_json(&doc, &did_assertion, &onchain_name, &provenance),
        "issuerDomainBinding": issuer_domain_json,
        "verificationId": rec.id,
    }))
}

/// The issuer identity a surface should render, and the untrusted document values it was checked
/// against.
///
/// `onchainName` is AUTHORITATIVE and is what a UI must display. `documentName`/`documentDomain` come
/// from the credential's `issuer` block, which sits outside the Merkle root — they are shown only to
/// state a disagreement, never as the issuer's identity.
///
/// Why the name needs its own source: `data.issuer` is a `did:web:` value, so it carries a DOMAIN and
/// nothing else. Relabelling ONLY `issuer.name` therefore passes integrity, passes the DID assertion,
/// and passes the DNS binding — the genuine domain really does publish the record. Without the on-chain
/// name, that attack renders a fabricated authority beside a green check, which is worse than showing
/// nothing.
///
/// `onchain_name` is only ever `Some` when link-1 provenance came back a DEFINITE yes (see `verify`).
/// The `provenance` field is carried here so a surface can say WHY a name is unavailable rather than
/// having to infer it from a bare `null`.
fn issuer_identity_json(
    doc: &WrappedDoc,
    assertion: &IssuerDomainAssertion,
    onchain_name: &Option<String>,
    provenance: &CloneProvenance,
) -> Value {
    let (root_covered, domain_conflict) = match assertion {
        IssuerDomainAssertion::Match { domain } => (Some(domain.clone()), false),
        IssuerDomainAssertion::Mismatch {
            root_covered,
            displayed: _,
        } => (Some(root_covered.clone()), true),
        IssuerDomainAssertion::NotAssertable => (None, false),
    };

    // Compare names only after normalising whitespace and case: a free-form label differing by padding
    // is not evidence of anything, and treating it as such would cry wolf on legitimate credentials.
    let name_conflict = match onchain_name {
        Some(on) if !on.trim().is_empty() => normalise_name(on) != normalise_name(&doc.issuer.name),
        _ => false,
    };

    json!({
        // What to display. Falls back to the document's name ONLY when the chain could not be read, and
        // `onchainNameAvailable` says which happened so a UI never presents a fallback as authoritative.
        "onchainName": onchain_name,
        "onchainNameAvailable": onchain_name.as_deref().map(|n| !n.trim().is_empty()).unwrap_or(false),
        // Link 1, the reason an on-chain name may be withheld: "factoryDeployed" | "notFactoryDeployed"
        // | "unknown". Only the first authorises reading an identity off that contract.
        "provenance": provenance.as_str(),
        // Untrusted, for diffing only.
        "documentName": doc.issuer.name,
        "documentDomain": doc.issuer.domain,
        "rootCoveredDomain": root_covered,
        "assertion": assertion.as_str(),
        // The document's issuer block contradicts what was actually issued. Either flag is enough for a
        // UI to stop presenting the document's identity as the issuer.
        "documentNameDiffers": name_conflict,
        "documentDomainDiffers": domain_conflict,
        "relabelled": name_conflict || domain_conflict,
    })
}

/// Casefold + collapse whitespace, for comparing free-form issuer labels.
fn normalise_name(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// LINK 1 of the issuer↔domain chain — `DogTagIssuerFactory.isClone(addr)` — as a THREE-valued fact.
///
/// Kept as a type rather than a `bool` because the three cases authorise different things and only one
/// of them authorises reading an IDENTITY (`name()`, the domain claim) off the contract. A read failure
/// is emphatically not "this is not a DogTag issuer", and neither is a missing factory address; folding
/// either into a boolean silently picks one of those meanings, and whichever one it picks is wrong.
enum CloneProvenance {
    /// The factory confirms it deployed this contract.
    FactoryDeployed,
    /// The factory confirms it did NOT. Categorically stronger than any DNS observation.
    NotFactoryDeployed,
    /// We could not find out — no factory configured, or the read failed. Evidence of nothing.
    Unknown { detail: String },
}

impl CloneProvenance {
    /// The ONLY gate that may authorise reading an identity off this contract.
    fn is_factory_deployed(&self) -> bool {
        matches!(self, CloneProvenance::FactoryDeployed)
    }
    /// The factory was ASKED and answered no. The only provenance state that may fail a verdict.
    ///
    /// Deliberately NOT `!is_factory_deployed()`, and the asymmetry is the whole point: `Unknown` — no
    /// factory configured, or the read failed — is evidence of nothing and must never be treated as a
    /// definite negative, exactly as `couldNotCheck` is never treated as `notListed`. A deployment
    /// running without `FACTORY_ADDR` would otherwise start failing every legitimate credential.
    fn is_definitely_not_factory_deployed(&self) -> bool {
        matches!(self, CloneProvenance::NotFactoryDeployed)
    }
    fn as_str(&self) -> &'static str {
        match self {
            CloneProvenance::FactoryDeployed => "factoryDeployed",
            CloneProvenance::NotFactoryDeployed => "notFactoryDeployed",
            CloneProvenance::Unknown { .. } => "unknown",
        }
    }
}

/// Read link 1 once, pinned to `at_block`.
async fn resolve_clone_provenance(
    st: &AppState,
    clone_addr: &str,
    at_block: Option<u64>,
) -> CloneProvenance {
    let factory = st.cfg.factory_addr.trim();
    if factory.is_empty() || factory.eq_ignore_ascii_case(ZERO_ADDR) {
        return CloneProvenance::Unknown {
            detail: "no DogTagIssuerFactory configured, so clone provenance cannot be checked"
                .into(),
        };
    }
    match st
        .chain
        .is_factory_clone(factory, clone_addr, at_block)
        .await
    {
        Ok(true) => CloneProvenance::FactoryDeployed,
        Ok(false) => CloneProvenance::NotFactoryDeployed,
        Err(e) => CloneProvenance::Unknown {
            detail: format!("clone provenance read failed: {e}"),
        },
    }
}

/// Resolve the issuer↔domain binding as a THREE-LINK CHAIN. All three links are required, and each is
/// re-checked here rather than inherited from a stored claim.
///
/// 1. **Factory provenance** — `DogTagIssuerFactory.isClone(clone)`. Without this the rest is
///    worthless: anyone can deploy their own contract, claim a domain for it, publish a matching TXT
///    record, and present as verified. The DNS would agree, the registry would agree, and none of it
///    would mean anything, because that contract never passed through the KYC-gated `createIssuer`.
///    Factory provenance is what ties the binding back to the whitelisting that gives it value.
///
///    The domain registry ALSO refuses a write for a non-clone, so bad bindings cannot be stored at
///    all. This check is not redundant with that: a stored binding is a CLAIM, and an app must not
///    inherit trust it did not verify itself (the registry could be swapped, or a future one could be
///    laxer).
///
/// 2. **The on-chain domain claim** — read from `IssuerDomainRegistry`, never from the document, whose
///    `issuer` block is outside the Merkle root.
///
/// 3. **The DNS half** — that domain's zone lists this clone address.
///
/// The failure modes stay CATEGORICALLY distinct, because they mean very different things:
///
/// * `notADogTagIssuer` — link 1 failed. A far stronger statement than a missing DNS record, and it
///   must never be rendered as merely "not listed".
/// * `noDomainClaimed`  — links 1 holds, no domain claimed. Normal on day one.
/// * `notListed`        — links 1 and 2 hold; DNS does not list the address.
/// * `couldNotCheck`    — a real attempt failed. Evidence of nothing.
/// * `unavailable`      — a prerequisite could not be read at all (no address configured, read failed).
async fn resolve_issuer_domain_binding(
    st: &AppState,
    clone_addr: &str,
    at_block: Option<u64>,
    provenance: &CloneProvenance,
) -> Value {
    // ---- link 1: factory provenance -------------------------------------------------------------
    //
    // Taken as a PARAMETER rather than re-read: the caller already resolved it at `at_block` to gate the
    // on-chain name, and one verification making two `isClone` calls could answer them differently
    // (different heights, one read failing) — leaving a single response internally inconsistent about
    // whether the issuer is a DogTag issuer at all.
    match provenance {
        CloneProvenance::FactoryDeployed => {}
        // A definitive "this contract was not deployed by the DogTag factory". Its own state.
        CloneProvenance::NotFactoryDeployed => {
            return json!({
                "state": "notADogTagIssuer",
                "cloneAddress": clone_addr.to_lowercase(),
                "blockNumber": at_block,
            })
        }
        // The read failed, or there is no factory to ask. NOT the same as "not a clone" — we simply do
        // not know.
        CloneProvenance::Unknown { detail } => {
            return json!({
                "state": "unavailable",
                "detail": detail,
            })
        }
    }

    // ---- link 2: the on-chain domain claim ------------------------------------------------------
    let registry = st.cfg.issuer_domain_registry_addr.trim();
    if registry.is_empty() || registry.eq_ignore_ascii_case(ZERO_ADDR) {
        return json!({
            "state": "unavailable",
            "detail": "no IssuerDomainRegistry configured for this deployment",
        });
    }

    let claimed = match st
        .chain
        .issuer_claimed_domain(registry, clone_addr, at_block)
        .await
    {
        Ok(Some(d)) => d,
        // The registry answered: this issuer has published no domain claim. Unremarkable.
        Ok(None) => return json!({ "state": "noDomainClaimed", "blockNumber": at_block }),
        // The READ failed — which is not the same as "no claim", and must not be shown as one.
        Err(e) => {
            return json!({
                "state": "unavailable",
                "detail": format!("on-chain domain claim read failed: {e}"),
            })
        }
    };

    // ---- link 3: the DNS half ---------------------------------------------------------------------
    //
    // THE ASYMMETRY, stated plainly because it governs what this result can honestly claim:
    //
    //   * Chain state is REPRODUCIBLE. Anyone with an archive node can re-run every read above pinned
    //     to `blockNumber` and get the same answer. (Verified against the ROAX node: `eth_call` at
    //     head-5000 returns full historical state, so the anchor is not decorative.)
    //   * DNS has NO HISTORY. There is no way to ask what a zone published at block N. A TXT record is
    //     only ever observable NOW.
    //
    // So the DNS half is an OBSERVATION that can never be recomputed, and it is labelled as such:
    // `dnsObservation: "live"` with its own wall-clock `checkedAt`, carried alongside — never inside —
    // the block anchor. A stored observation must never be presented as live, and a live one must never
    // be presented as proving the past.
    let check = st.dns.check(clone_addr, &claimed.domain).await;
    let mut out = serde_json::to_value(&check).unwrap_or_else(|_| json!({}));
    // Flatten the tagged status up one level so a client reads a single `state` field for every case
    // rather than having to branch on nesting depth first.
    if let (Some(obj), Ok(status)) = (out.as_object_mut(), serde_json::to_value(&check.status)) {
        obj.remove("status");
        if let Some(status_obj) = status.as_object() {
            for (k, v) in status_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    if let Some(obj) = out.as_object_mut() {
        // The block the CHAIN half was read at.
        obj.insert("blockNumber".into(), json!(at_block));
        // When the issuer last CHANGED its domain claim, and at which block — so "what did this clone
        // claim at block N" is answerable rather than only "what does it claim now".
        obj.insert("claimUpdatedAt".into(), json!(claimed.updated_at));
        obj.insert(
            "claimUpdatedAtBlock".into(),
            json!(claimed.updated_at_block),
        );
        obj.insert("claimSetBy".into(), json!(claimed.set_by));
        // Whether this answer came off the wire just now or out of the resolver's cache — DERIVED from
        // the observation's own `checked_at`, never asserted. `BindingResolver` serves a cached answer
        // for up to `CacheTtl::answer_max` (15 min) and deliberately keeps its ORIGINAL timestamp, so
        // hardcoding "live" here printed "DNS checked just now" over an observation a quarter of an hour
        // old. That is the same fabrication as a badge claiming a lookup that never ran.
        obj.insert("dnsObservation".into(), json!(dns_observation(check.checked_at)));
        obj.insert("dnsHistorical".into(), json!(false));
    }
    out
}

/// How fresh a DNS observation has to be to be called live, in seconds.
///
/// 60s, matching the Swift `IssuerBinding.provenanceLine` and the Kotlin `provenanceLine` thresholds, so
/// the four legs (TS renderer, this API, and both phones) agree on when "just now" stops being true.
const DNS_LIVE_WINDOW_SECS: u64 = 60;

/// `"live"` for an observation made within [`DNS_LIVE_WINDOW_SECS`], `"stored"` otherwise.
///
/// A clock that has gone backwards yields no elapsed time, which reads as `"live"` — the conservative
/// direction here is the one that does not invent an age the observation does not have.
fn dns_observation(checked_at: u64) -> &'static str {
    let age = now().saturating_sub(checked_at);
    if age < DNS_LIVE_WINDOW_SECS {
        "live"
    } else {
        "stored"
    }
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
// record share — the owner's phone imports an issued credential by scanning a one-time QR.
// Mirror of vet-api `POST /records/:id/share` + `GET /r/:token`: same 32-hex token, same 180s TTL,
// same consume-on-first-read guarantee, so `RecordImporter` on both phones needs no gov branch.
// --------------------------------------------------------------------------------------------

/// POST /v1/records/:root/share — mint a one-time share token for an issued credential and return the
/// QR URL the owner scans. Operator-gated (the authority's own custodial record is not public); the
/// RESOLVE side is unauthenticated, because the QR itself is the capability.
///
/// Re-minting is the refresh path: each call issues an independent token, and a previously displayed QR
/// simply expires unused. Any lifecycle state is shareable — the owner is entitled to a copy of a
/// revoked or expired credential they hold, and the phone re-derives the verdict from the chain.
async fn share_record(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(root): Path<String>,
) -> Resp {
    if let Err(e) = require_api_token(&st, &headers) {
        return e;
    }
    if st.store.get_credential(&root).await.is_none() {
        return err(StatusCode::NOT_FOUND, "no credential for that root");
    }
    let token = gen_qr_token();
    let expires_at = now() + SHARE_TOKEN_TTL_SECS;
    st.store.put_share_token(&token, &root, expires_at).await;
    ok(json!({
        "qrUrl": format!("{}/r/{}", qr_base(&st), token),
        "root": root,
        // Both are returned so the portal can render a countdown WITHOUT trusting browser/server clock
        // agreement: count down from `ttlSecs` at response receipt, and show `expiresAt` as the fact.
        "ttlSecs": SHARE_TOKEN_TTL_SECS,
        "expiresAt": expires_at,
    }))
}

/// Resolve + CONSUME a record-share token: returns the credential's wrapped doc as raw JSON, exactly as
/// vet-api's `GET /r/:token` does. One-time — a second read is a 404, and so is an expired token.
///
/// Always answers JSON (never the HTML receipt page): once the segment is known to be a share token, an
/// HTML body would reach the phone as "bad wrapped doc" instead of an honest "expired".
async fn get_shared(st: &AppState, token: &str) -> Resp {
    let root = match st.store.take_share_token(token).await {
        Some(r) => r,
        None => return err(StatusCode::NOT_FOUND, "share token missing or expired"),
    };
    match st.store.get_credential(&root).await {
        Some(c) => ok(c.wrapped_doc),
        None => err(StatusCode::NOT_FOUND, "no credential for that root"),
    }
}

// --------------------------------------------------------------------------------------------
// owner-hidden verification (the authority as a VERIFIER) — QR handoff to the owner's phone.
//
// The government is a verifier here, so it goes through the SAME ZK consent path as a groomer: the
// owner approves an owner-hidden proof on their device and this backend only relays it. Route paths
// and payload shapes mirror vet-api exactly. See `crate::verify`.
// --------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct SessionStartReq {
    purpose: String,
    #[serde(rename = "recordType", alias = "record_type")]
    record_type: String,
}

/// POST /verify/session/start — start an owner-hidden verify session and return its QR.
///
/// Every precondition is checked BEFORE the QR exists, so the owner never spends tens of seconds
/// proving into a dead end:
///   * a signer must be loaded (`GOV_SIGNER_KEY`) — it is the relayer the QR names and the account that
///     will submit the proof,
///   * that signer must be whitelisted for `VERIFY:<purpose>` on the `IssuerRegistry` — else 403
///     `relayer not whitelisted for this purpose`, the same honest failure the groomer portal shows,
///   * the verification registry must be configured — else 503.
async fn verify_session_start(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SessionStartReq>,
) -> Resp {
    if let Err(e) = require_api_token(&st, &headers) {
        return e;
    }
    if body.purpose.trim().is_empty() || body.record_type.trim().is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "purpose and recordType are required",
        );
    }
    let relayer = match st.chain.signer_address() {
        Some(a) if st.chain.can_sign() && !a.is_empty() => a,
        _ => {
            return err(
                StatusCode::SERVICE_UNAVAILABLE,
                "no government signer configured (set GOV_SIGNER_KEY) to record a verification",
            )
        }
    };
    // whitelistedFor(keccak256(abi.encode("VERIFY:", purpose)), relayer)
    let verify_key = crate::verify::verify_key(&body.purpose);
    match st
        .chain
        .is_whitelisted_for(&st.cfg.issuer_registry_addr, &verify_key, &relayer, None)
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
    if !crate::verify::valid_contract_addr(&crate::verify::consent_registry(&st)) {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "verification registry not configured",
        );
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let mut challenge = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut challenge);
    let ts = now();
    st.store
        .put_session(VerifySession {
            session_id: session_id.clone(),
            relayer: relayer.clone(),
            purpose: body.purpose.clone(),
            record_type: body.record_type.clone(),
            challenge: format!("0x{}", hex::encode(challenge)),
            status: "pending".to_string(),
            tx_hash: None,
            nullifier: None,
            created_at: ts,
            updated_at: ts,
            disclosed_key_paths: Vec::new(),
        })
        .await;

    let token = gen_qr_token();
    let expires_at = now() + EXPORT_TOKEN_TTL_SECS;
    st.store
        .put_export_token(&token, &session_id, expires_at)
        .await;
    ok(json!({
        "qrUrl": format!("{}/x/{}?a={}", qr_base(&st), token, relayer),
        "sessionId": session_id,
        "purpose": body.purpose,
        "recordType": body.record_type,
        "relayer": relayer,
        "ttlSecs": EXPORT_TOKEN_TTL_SECS,
        "expiresAt": expires_at,
    }))
}

/// GET /x/:token — resolve a verify-session export token to the metadata the owner's phone needs.
/// Unauthenticated (the token IS the capability) and NON-consuming: the phone re-reads it while proving
/// and polling, and the session status is what blocks replay.
async fn verify_session_resolve(State(st): State<AppState>, Path(token): Path<String>) -> Resp {
    let session_id = match st.store.peek_export_token(&token).await {
        Some(id) => id,
        None => return err(StatusCode::NOT_FOUND, "export token missing or expired"),
    };
    let s = match st.store.get_session(&session_id).await {
        Some(s) => s,
        None => return err(StatusCode::NOT_FOUND, "session not found"),
    };
    // M7 P4 (§5.2) CONVENIENCE tier: platform-OWNED, UNVERIFIED claims. The app validates these against
    // the on-chain ProtocolRegistry / signed-manifest anchor before trusting any of them — and REFUSES
    // the whole flow if they are absent, so this block is mandatory, not decorative.
    let issuer_clone = st.cfg.issuer_addr_for(&s.record_type).unwrap_or_default();
    let claims = app::convenience_claims(&st.cfg, st.chain.chain_id(), &issuer_clone, &s.purpose);
    ok(json!({
        "sessionId": s.session_id,
        "relayer": s.relayer,
        "purpose": s.purpose,
        "recordType": s.record_type,
        "challenge": s.challenge,
        "unverifiedClaims": serde_json::to_value(&claims).expect("ConvenienceClaims serializes"),
    }))
}

/// POST /v1/verify/consent — submit an owner-hidden consent proof. The owner's phone authenticates with
/// its export token (`exportToken` in the body); an operator bearer may submit a cold proof.
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
    // A named session that does not load FAILS CLOSED. Falling through to `None` would silently demote
    // the request to the cold path — skipping the purpose/relayer/recordType binding AND the status
    // replay guard — and `MongoStore::get_session` maps driver errors to `None`, so a transient DB blip
    // would otherwise disable every session-scoped guard on this route for that request.
    let session = if session_id.is_empty() {
        None
    } else {
        match st.store.get_session(&session_id).await {
            Some(s) => Some(s),
            // Token-authed: the gate already matched this token to this session id, so the row not
            // loading is a BACKEND fault, not a bad request.
            None if authed_by_export_token => {
                return err(
                    StatusCode::BAD_GATEWAY,
                    "session could not be read; retry with the same token",
                )
            }
            None => return err(StatusCode::NOT_FOUND, "session not found"),
        }
    };
    crate::verify::consent_submit_levelb(&st, &body, session).await
}

#[derive(Deserialize)]
struct SessionStatusQuery {
    /// The short-lived export token (the owner's phone polling) — a non-consuming peek. The operator
    /// portal omits it and relies on the operator bearer instead.
    #[serde(default)]
    token: Option<String>,
}

/// GET /verify/session/:id — status read so BOTH the portal and the owner's phone can poll
/// pending → recording → recorded. Dual-gated (operator bearer OR the session's export token) and
/// non-consuming: status reads are idempotent and polled repeatedly.
async fn verify_session_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(q): Query<SessionStatusQuery>,
) -> Resp {
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
        "disclosedKeyPaths": s.disclosed_key_paths,
    }))
}

/// GET /verify/history — operator-gated owner-hidden verification audit log. Verifier-side operational
/// proof metadata only (purpose, relayer, tx, nullifier) — never credential PII, and never anything that
/// identifies the owner.
async fn verify_history(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = require_api_token(&st, &headers) {
        return e;
    }
    let verifications: Vec<Value> = st
        .store
        .list_sessions()
        .await
        .into_iter()
        .map(|s| {
            let explorer_url = s
                .tx_hash
                .as_deref()
                .filter(|t| t.starts_with("0x"))
                .map(crate::chain::explorer_tx_url);
            json!({
                "sessionId": s.session_id,
                "relayer": s.relayer,
                "purpose": s.purpose,
                "recordType": s.record_type,
                "status": s.status,
                "txHash": s.tx_hash,
                "explorerUrl": explorer_url,
                "nullifier": s.nullifier,
                "disclosedKeyPaths": s.disclosed_key_paths,
                "createdAt": s.created_at,
                "updatedAt": s.updated_at,
            })
        })
        .collect();
    ok(json!({ "verifications": verifications }))
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
    /// The real chain this receipt is anchored on; `None` on a SIMULATED backend.
    chain_id: Option<u64>,
    /// Issue/revoke block-explorer links, **suppressed on a simulated backend**. The stored URLs point
    /// at `explorer.roax.net` for a tx that was never broadcast, and these two surfaces are the
    /// unauthenticated ones an outside party checks - handing them a dead ROAX link is the same
    /// dishonesty `/health` used to commit. Suppressed HERE, once, so both public surfaces agree; the
    /// stored record and the operator-facing `/v1/records` surfaces keep their links unchanged.
    issue_explorer_url: Option<String>,
    revoke_explorer_url: Option<String>,
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
        .is_valid(&cred.issuer_addr, &cred.root, None)
        .await
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                &format!("on-chain isValid read failed: {e}"),
            )
        })?;
    let issued_at = st
        .chain
        .issued_at(&cred.issuer_addr, &cred.root, None)
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

    let simulated = st.chain.is_simulated();
    Ok(ReceiptStatus {
        chain_id: (!simulated).then(|| st.chain.chain_id()),
        issue_explorer_url: (!simulated).then(|| cred.explorer_url.clone()).flatten(),
        revoke_explorer_url: (!simulated)
            .then(|| cred.revoke_explorer_url.clone())
            .flatten(),
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
        // Chain provenance, honest on a simulated backend: null id, no dead explorer links, and an
        // explicit `simulated` flag so a consumer sees WHY they are absent rather than guessing.
        "chainId": rs.chain_id,
        "simulated": rs.chain_id.is_none(),
        "explorerUrl": rs.issue_explorer_url,
        "revokeExplorerUrl": rs.revoke_explorer_url,
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

/// GET /r/:id — the OVERLOADED public `/r/` surface, dispatched on the id's shape:
///
///   * a 32-hex segment is a one-time RECORD-SHARE token: resolve+consume it to the wrapped doc as JSON
///     (what the owner's phone scans to import a credential — the vet-api `/r/:token` contract),
///   * anything else is a public RECEIPT id: render the PII-free HTML status page below.
///
/// The shapes are disjoint (32 hex vs 12 Crockford base32), so neither can shadow the other. Both mobile
/// apps treat ANY `/r/<segment>` with no query string as an import token, which is exactly why the JSON
/// branch must own the hex shape.
async fn public_r(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    if is_share_token(&id) {
        return get_shared(&st, &id).await.into_response();
    }
    receipt_page(State(st), Path(id)).await
}

/// PUBLIC receipt status page (status-only by default, arch DP-5). Renders the live verdict + validity
/// window + on-chain provenance links. NO Section A/B/C content — the official reads the details off the
/// paper/phone in front of them and this page confirms the receipt is genuine and current. Enumeration
/// is bounded by the ~60-bit receiptId.
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
    let issue_link = rs
        .issue_explorer_url
        .as_deref()
        .map(|u| {
            format!(
                "<a href=\"{}\" rel=\"noopener noreferrer\">issue tx</a>",
                esc(u)
            )
        })
        .unwrap_or_default();
    let revoke_link = rs
        .revoke_explorer_url
        .as_deref()
        .map(|u| {
            format!(
                " · <a href=\"{}\" rel=\"noopener noreferrer\">revoke tx</a>",
                esc(u)
            )
        })
        .unwrap_or_default();
    // The provenance row is the one thing an outside party reads this page FOR. On a simulated backend
    // it must not read "ROAX chainId 135" beside links to txs that were never broadcast - it says so,
    // in place, rather than silently dropping the row and leaving the reader to assume the best.
    let anchored_row = match rs.chain_id {
        Some(chain) => format!(
            "<dt>Anchored on</dt><dd>ROAX chainId {chain} · {issue_link}{revoke_link}</dd>"
        ),
        None => "<dt>Anchored on</dt>\
                 <dd><strong>NOT a real chain - SIMULATED backend.</strong> This receipt was produced \
                 by a demonstration stack running an in-process chain emulation. Nothing was \
                 broadcast, no block explorer holds a record of it, and it carries no legal effect.\
                 </dd>"
            .to_string(),
    };

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
         {anchored_row}\
         </dl>\
         <p class=\"foot\">Live {chain_note} status check · no personal data shown · checked {checked} (UTC epoch s).</p>\
         </div></body></html>",
        rid = esc(&receipt_id),
        rtype = esc(&c.record_type),
        root = esc(&c.root),
        chain_note = if rs.chain_id.is_some() {
            "on-chain"
        } else {
            "simulated-chain"
        },
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
        // one-time record-share QR (operator mints; the owner's phone resolves it at /r/:token)
        .route("/v1/records/:root/share", post(share_record))
        .route("/v1/verifications", get(list_verifications))
        // OWNER-HIDDEN verify (the authority as a verifier). Paths mirror vet-api EXACTLY — the phone
        // apps hard-code /x/:token, /v1/verify/consent and /verify/session/:id.
        .route("/verify/session/start", post(verify_session_start))
        .route("/verify/session/:id", get(verify_session_status))
        .route("/verify/history", get(verify_history))
        .route("/v1/verify/consent", post(verify_consent_submit))
        .route("/x/:token", get(verify_session_resolve))
        // oversight console (govarch PR-5): unscoped cross-issuer activity joined to own credentials
        .route("/v1/oversight/activity", get(oversight_activity))
        .route("/v1/oversight/stats", get(oversight_stats))
        .route("/v1/oversight/issuers", get(oversight_issuers))
        // PUBLIC (no auth): PII-free receipt status JSON, plus the overloaded /r/ surface — a 32-hex
        // segment is a one-time record-share token (JSON, consumed), anything else is the human status
        // page (live on-chain read). See `public_r`.
        .route("/v1/receipts/:receipt_id/status", get(receipt_status))
        .route("/r/:id", get(public_r))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The threshold itself. An observation is `"live"` only while it really is recent; the moment it is
    /// older than the window it is a REPLAY of a past look, and saying "checked just now" over it is the
    /// same fabrication as a badge claiming a lookup that never ran.
    #[test]
    fn a_stale_observation_is_reported_as_stored_not_live() {
        let now = now();
        assert_eq!(dns_observation(now), "live");
        assert_eq!(dns_observation(now.saturating_sub(1)), "live");
        assert_eq!(
            dns_observation(now.saturating_sub(DNS_LIVE_WINDOW_SECS)),
            "stored",
            "at the boundary the answer is already a recorded one"
        );
        // The resolver serves a cached answer for up to `CacheTtl::answer_max` (15 min), keeping its
        // ORIGINAL timestamp — this is the case the hardcoded "live" got wrong.
        assert_eq!(dns_observation(now.saturating_sub(600)), "stored");
    }

    /// A clock that has gone backwards yields no elapsed time. It must not underflow into a huge age and
    /// silently relabel a fresh observation.
    #[test]
    fn a_future_timestamp_does_not_underflow_into_stored() {
        assert_eq!(dns_observation(now() + 5), "live");
    }

    /// Only the DEFINITE negative may fail a verdict. `Unknown` — no factory configured, or the read
    /// failed — is evidence of nothing, exactly as `couldNotCheck` is not `notListed`.
    #[test]
    fn only_a_definite_provenance_failure_is_a_definite_negative() {
        assert!(CloneProvenance::NotFactoryDeployed.is_definitely_not_factory_deployed());
        assert!(!CloneProvenance::FactoryDeployed.is_definitely_not_factory_deployed());
        assert!(!CloneProvenance::Unknown {
            detail: "no factory configured".into()
        }
        .is_definitely_not_factory_deployed());
        // And the read gate stays the mirror image: only a definite YES authorises an identity read.
        assert!(CloneProvenance::FactoryDeployed.is_factory_deployed());
        assert!(!CloneProvenance::NotFactoryDeployed.is_factory_deployed());
        assert!(!CloneProvenance::Unknown {
            detail: String::new()
        }
        .is_factory_deployed());
    }
}
