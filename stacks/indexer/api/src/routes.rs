//! The scoped query API.
//!
//! One feed, two doctrines, enforced **server-side** by the caller's bearer token:
//!   - the **unscoped** government/oversight token sees every event across all issuers;
//!   - a **scoped** vet/groomer token sees only events for its own signer(s)/clone(s).
//!
//! Client filters (`type`, `signer`, `issuer`, `recordType`, `root`, `dogTagId`, `finality`,
//! `since`, `until`) only ever narrow *within* the token's ceiling — never widen it (see
//! `crate::scope`).
//!
//! Every event is joined to the admin business directory to NAME its signer/clone where possible, and
//! carries a ready-to-click `txUrl`. Nothing personal is served — the index holds only non-PII chain
//! data.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{keccak_key, AppState};
use crate::events::{EventType, Finality, IndexedEvent};
use crate::scope::Principal;
use crate::store::EventQuery;

type ApiError = (StatusCode, Json<Value>);

fn err(code: StatusCode, msg: &str) -> ApiError {
    (code, Json(json!({ "error": msg })))
}

/// Extract the `Authorization: Bearer <token>` value.
fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
}

/// Resolve the caller's principal from its bearer token, or 401. Fail-closed: an empty registry or an
/// unknown token is rejected (mirrors the government stack's "no token ⇒ refuse" posture).
fn authenticate<'a>(st: &'a AppState, headers: &HeaderMap) -> Result<&'a Principal, ApiError> {
    let token = bearer(headers).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    // The registry is Arc-held inside `st`, so the resolved reference is valid for `'a`.
    st.scopes
        .resolve(&token)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "unknown or unauthorized token"))
}

#[derive(Debug, Deserialize, Default)]
pub struct FeedParams {
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub signer: Option<String>,
    pub issuer: Option<String>,
    #[serde(rename = "recordType")]
    pub record_type: Option<String>,
    pub root: Option<String>,
    #[serde(rename = "dogTagId")]
    pub dog_tag_id: Option<String>,
    /// `finalized` | `pending` — restrict the feed to a finality state.
    pub finality: Option<String>,
    pub since: Option<u64>,
    pub until: Option<u64>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Translate a `?recordType=` param (human label like `TRAVEL_CLEARANCE`, or an already-hashed
/// `0x…32-byte` key) into the indexed `keccak256` key.
fn normalize_record_type(v: &str) -> String {
    let t = v.trim();
    if t.starts_with("0x") && t.len() == 66 {
        t.to_ascii_lowercase()
    } else {
        keccak_key(t)
    }
}

/// Build the `EventQuery` from the request params + config paging bounds.
fn build_query(st: &AppState, p: &FeedParams) -> Result<EventQuery, ApiError> {
    let event_type = match &p.event_type {
        Some(s) if !s.trim().is_empty() => Some(
            EventType::parse(s).ok_or_else(|| err(StatusCode::BAD_REQUEST, "unknown event type"))?,
        ),
        _ => None,
    };
    let finality = match &p.finality {
        Some(s) if !s.trim().is_empty() => Some(
            Finality::parse(s)
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "finality must be finalized|pending"))?,
        ),
        _ => None,
    };
    let limit = p
        .limit
        .unwrap_or(st.cfg.default_page_limit)
        .clamp(1, st.cfg.max_page_limit);
    Ok(EventQuery {
        event_type,
        signer: p
            .signer
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_ascii_lowercase()),
        issuer: p
            .issuer
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_ascii_lowercase()),
        record_type: p
            .record_type
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| normalize_record_type(s)),
        root: p
            .root
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_ascii_lowercase()),
        dog_tag_id: p
            .dog_tag_id
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string()),
        finality,
        since: p.since,
        until: p.until,
        limit,
        offset: p.offset.unwrap_or(0),
    })
}

/// Serialize one event and enrich it with directory names + an explorer link.
fn render_event(st: &AppState, ev: &IndexedEvent) -> Value {
    let mut v = serde_json::to_value(ev).unwrap_or_else(|_| json!({}));
    if let Value::Object(map) = &mut v {
        map.insert("txUrl".into(), json!(st.cfg.tx_url(&ev.tx_hash)));
        if let Some(a) = &ev.actor {
            if let Some(name) = st.directory.resolve(a) {
                map.insert("actorName".into(), json!(name));
            }
        }
        if let Some(c) = &ev.clone {
            if let Some(name) = st.directory.resolve(c) {
                map.insert("cloneName".into(), json!(name));
            }
        }
    }
    v
}

/// `GET /health` - no auth. Liveness plus, critically, an HONEST statement of which event surface
/// this indexer is serving.
///
/// The reported `chainId` comes from the LOG SOURCE OBJECT rather than from `Config`. A demo-mode
/// indexer used to inherit the configured `135` and report `{"ok":true,"chainId":135}` - byte-identical
/// to a live one - while serving a scripted in-memory log with FABRICATED block numbers, merkle roots
/// and `txUrl` links to transactions that do not exist. Nothing in the response let an operator, or the
/// consoles that consume it, tell the difference. Now:
///   - `backend`   - "live" | "simulated" (authoritative, from the source in use)
///   - `simulated` - the same fact as a boolean, for UI badges
///   - `chainId`   - the live source's id, `null` when simulated (never a network it is not on)
///
/// Sourcing the id from the object is what makes the SIMULATED case structurally sound: a scripted
/// source has no network id to give, so it cannot echo a real one however `CHAIN_ID` is set. The LIVE
/// case is weaker and this claim stops there: `main.rs` builds `AlloyLogSource::with_chain_id` from the
/// `CHAIN_ID` env var, so a live id is still OPERATOR-ASSERTED and is never checked against the node -
/// point `ROAX_RPC` at another network while leaving `CHAIN_ID=135` and this still answers `135`.
///
/// Both keys are emitted on BOTH paths. A flag present only when simulated would make its absence
/// ambiguous between "live" and "a build too old to tell you" - i.e. "could not check" rendering as
/// its own neighbour, which is the failure this endpoint exists to prevent.
async fn health(State(st): State<AppState>) -> impl IntoResponse {
    let backend = st.source.backend();
    let simulated = backend.is_simulated();
    Json(json!({
        "ok": true,
        // Null rather than a real id when simulated - this source is on no network at all.
        "chainId": (!simulated).then(|| st.source.chain_id()),
        "backend": backend.as_str(),
        "simulated": simulated,
    }))
}

/// `GET /v1/status` — indexer progress + finality watermark + this principal's scope.
///
/// Carries the same `backend`/`simulated`/`chainId` disclosure as `/health`, and for the same reason:
/// this is the endpoint the oversight consoles poll for chain health, so `headBlock: 8` against a real
/// chain head of ~282,800 must be readable as scripted rather than as a catastrophically lagging
/// indexer. The progress numbers themselves are unchanged - this annotates them, it does not correct
/// them, because they are a faithful report of the source actually in use.
///
/// `watchedGenerations` reports the exact anti-spoof allowlist (immutable generation id + factory /
/// issuer-registry / verification-registry triple + seeded clones). A feed with no rows can therefore
/// be distinguished from a scanner that simply omitted the generation an operator expected.
async fn status(State(st): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let principal = authenticate(&st, &headers)?;
    let backend = st.source.backend();
    let simulated = backend.is_simulated();
    let cursor = st.store.get_cursor().await;
    let head = st.source.head_block().await.ok();
    // Report the live finalized watermark + whether it comes from the chain's finality tag or the
    // confirmations-depth fallback, so an oversight consumer knows how strong "finalized" is here.
    let (finalized_block, finality_source) = match st.source.finalized_block().await {
        Ok(Some(n)) => (head.map(|h| n.min(h)).or(Some(n)), "finalized-tag"),
        Ok(None) => (
            head.map(|h| h.saturating_sub(st.cfg.confirmations)),
            "confirmations-fallback",
        ),
        Err(_) => (None, "unknown"),
    };
    let lag = match (head, cursor.last_finalized) {
        (Some(h), Some(lb)) => Some(h.saturating_sub(lb)),
        _ => None,
    };
    Ok(Json(json!({
        // Null rather than a real id when simulated - see the `/health` doc above.
        "chainId": (!simulated).then(|| st.source.chain_id()),
        "backend": backend.as_str(),
        "simulated": simulated,
        "headBlock": head,
        "finalizedBlock": finalized_block,
        "finalitySource": finality_source,
        "lastFinalizedIndexed": cursor.last_finalized,
        "lag": lag,
        "confirmations": st.cfg.confirmations,
        "watchedGenerations": &st.cfg.generations,
        "scope": {
            "label": principal.label,
            "unscoped": principal.scope.is_unscoped(),
        }
    })))
}

/// `GET /v1/events` — the scoped oversight feed.
async fn events(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(p): Query<FeedParams>,
) -> Result<Json<Value>, ApiError> {
    let principal = authenticate(&st, &headers)?;
    let q = build_query(&st, &p)?;
    let (page, total) = st.store.query_events(&q, &principal.scope).await;
    let items: Vec<Value> = page.iter().map(|e| render_event(&st, e)).collect();
    Ok(Json(json!({
        "events": items,
        "total": total,
        "limit": q.limit,
        "offset": q.offset,
        "scope": { "label": principal.label, "unscoped": principal.scope.is_unscoped() },
    })))
}

/// `GET /v1/stats` — in-scope counters (dashboard fuel; unscoped for gov/admin, scoped for a business).
async fn stats(State(st): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let principal = authenticate(&st, &headers)?;
    let q = EventQuery { limit: usize::MAX, ..Default::default() };
    let (all, total) = st.store.query_events(&q, &principal.scope).await;

    let mut issued = 0u64;
    let mut revoked = 0u64;
    let mut verifications = 0u64;
    let mut whitelisted = 0u64;
    let mut delisted = 0u64;
    let mut finalized = 0u64;
    let mut pending = 0u64;
    let mut clones = std::collections::BTreeSet::new();
    let mut signers = std::collections::BTreeSet::new();
    for e in &all {
        match e.finality {
            Finality::Finalized => finalized += 1,
            Finality::Pending => pending += 1,
        }
        match e.event_type {
            EventType::RootIssued => issued += 1,
            EventType::RootRevoked => revoked += 1,
            EventType::Verified => verifications += 1,
            EventType::Whitelisted => whitelisted += 1,
            EventType::Delisted => delisted += 1,
            EventType::IssuerCreated => {
                if let Some(c) = &e.clone {
                    clones.insert(c.clone());
                }
            }
            EventType::RootRegistered => {}
        }
        if let Some(a) = &e.actor {
            signers.insert(a.clone());
        }
        if let Some(c) = &e.clone {
            clones.insert(c.clone());
        }
    }
    Ok(Json(json!({
        "totalEvents": total,
        "finalized": finalized,
        "pending": pending,
        "rootIssued": issued,
        "rootRevoked": revoked,
        "activeCredentials": issued.saturating_sub(revoked),
        "verifications": verifications,
        "whitelisted": whitelisted,
        "delisted": delisted,
        "clones": clones.len(),
        "signers": signers.len(),
        "scope": { "label": principal.label, "unscoped": principal.scope.is_unscoped() },
    })))
}

/// `GET /v1/issuers` — the deployed `DogTagIssuer` clones visible in scope, each with issuance /
/// revocation / verification-relayer counts derived from the index.
async fn issuers(State(st): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let principal = authenticate(&st, &headers)?;
    let q = EventQuery { limit: usize::MAX, ..Default::default() };
    let (all, _) = st.store.query_events(&q, &principal.scope).await;

    use std::collections::BTreeMap;
    // clone addr -> (name, recordType, issued, revoked)
    let mut map: BTreeMap<String, (Option<String>, Option<String>, u64, u64)> = BTreeMap::new();
    for e in &all {
        if e.event_type == EventType::IssuerCreated {
            if let Some(c) = &e.clone {
                let entry = map.entry(c.clone()).or_insert((None, None, 0, 0));
                entry.0 = e.name.clone();
                entry.1 = e.record_type.clone();
            }
        }
    }
    for e in &all {
        if let Some(c) = &e.clone {
            let entry = map.entry(c.clone()).or_insert((None, None, 0, 0));
            match e.event_type {
                EventType::RootIssued => entry.2 += 1,
                EventType::RootRevoked => entry.3 += 1,
                _ => {}
            }
        }
    }
    let out: Vec<Value> = map
        .into_iter()
        .map(|(addr, (name, rt, issued, revoked))| {
            json!({
                "clone": addr,
                "name": name,
                "cloneName": st.directory.resolve(&addr),
                "recordType": rt,
                "issued": issued,
                "revoked": revoked,
                "active": issued.saturating_sub(revoked),
            })
        })
        .collect();
    Ok(Json(json!({ "issuers": out, "scope": { "label": principal.label } })))
}

/// The router. `demo` toggles a permissive posture only via env in `main`; scoping is always enforced.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/status", get(status))
        .route("/v1/events", get(events))
        .route("/v1/stats", get(stats))
        .route("/v1/issuers", get(issuers))
        .with_state(state)
}
