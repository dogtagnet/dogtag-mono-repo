//! The scoped oversight query API + public provider directory.
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
//!
//! `GET /v1/businesses` is the separate, public discovery surface. Its business contacts and optional
//! premises are deliberately published provider facts, not owner PII.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::compression::CompressionLayer;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

use crate::app::{keccak_key, AppState};
use crate::events::{EventType, Finality, IndexedEvent};
use crate::scope::Principal;
use crate::store::{EventQuery, StoreError};

type ApiError = (StatusCode, Json<Value>);

fn err(code: StatusCode, msg: &str) -> ApiError {
    (code, Json(json!({ "error": msg })))
}

/// An unreadable index answers 503 with no counters at all - never `200 {"total": 0}`.
///
/// "The store could not be read" and "no events matched" are different facts with different remedies,
/// and only one of them is evidence about the chain. Emitting an empty feed for the former is the same
/// could-not-check-rendered-as-a-neighbour defect the generation watch-set closes on the scan side.
fn store_unreadable(e: StoreError) -> ApiError {
    tracing::error!("event index read failed: {e}");
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        "event index could not be read; refusing to answer with an empty feed",
    )
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BusinessDirectoryParams {
    /// Case- and diacritic-insensitive substring of the provider's published name.
    name: Option<String>,
    /// Case-insensitive exact match against the business row's `type`.
    kind: Option<String>,
    /// Compatibility spelling used by the existing admin directory/client contract.
    #[serde(rename = "type")]
    business_type: Option<String>,
    /// Center of a location the user explicitly searched for, typed, or picked on a map.
    search_center_lat: Option<f64>,
    /// See `search_center_lat`. This is never an implicit current-device coordinate.
    search_center_lng: Option<f64>,
    /// Inclusive search radius around the explicitly selected search center, in kilometres.
    search_radius_km: Option<f64>,
}

#[derive(Clone, Copy)]
struct SearchArea {
    lat: f64,
    lng: f64,
    radius_km: f64,
}

fn nonempty_filter(value: Option<String>, field: &str) -> Result<Option<String>, ApiError> {
    match value {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Err(err(
            StatusCode::BAD_REQUEST,
            &format!("{field} must not be blank"),
        )),
        Some(value) => Ok(Some(value.trim().to_string())),
    }
}

fn kind_filter(params: &BusinessDirectoryParams) -> Result<Option<String>, ApiError> {
    let value = match (params.kind.clone(), params.business_type.clone()) {
        (Some(_), Some(_)) => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "kind and type are aliases; provide only one",
            ));
        }
        (Some(kind), None) | (None, Some(kind)) => Some(kind),
        (None, None) => None,
    };
    Ok(nonempty_filter(value, "kind/type")?.map(|kind| kind.to_ascii_lowercase()))
}

fn search_area(params: &BusinessDirectoryParams) -> Result<Option<SearchArea>, ApiError> {
    match (
        params.search_center_lat,
        params.search_center_lng,
        params.search_radius_km,
    ) {
        (None, None, None) => Ok(None),
        (Some(lat), Some(lng), Some(radius_km))
            if lat.is_finite()
                && lng.is_finite()
                && radius_km.is_finite()
                && (-90.0..=90.0).contains(&lat)
                && (-180.0..=180.0).contains(&lng)
                && radius_km >= 0.0 =>
        {
            Ok(Some(SearchArea { lat, lng, radius_km }))
        }
        (Some(_), Some(_), Some(_)) => Err(err(
            StatusCode::BAD_REQUEST,
            "searchCenterLat/searchCenterLng must be valid coordinates and searchRadiusKm must be finite and non-negative",
        )),
        _ => Err(err(
            StatusCode::BAD_REQUEST,
            "searchCenterLat, searchCenterLng, and searchRadiusKm must be provided together",
        )),
    }
}

/// Locale-independent name-search fold, matching the native directory's NFD + combining-mark removal
/// rather than making `avila` fail to find a provider named `Ávila`.
fn search_fold(value: &str) -> String {
    value
        .trim()
        .nfd()
        .filter(|ch| !is_combining_mark(*ch))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Total great-circle distance in kilometres. The `atan2` form stays defined at antipodes, unlike the
/// deprecated admin route's `asin(sqrt(a))` implementation.
fn haversine_km(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6371.0;
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let dphi = (lat2 - lat1).to_radians();
    let dlambda = (lng2 - lng1).to_radians();
    let a = (dphi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (dlambda / 2.0).sin().powi(2);
    let bounded = a.clamp(0.0, 1.0);
    2.0 * EARTH_RADIUS_KM * bounded.sqrt().atan2((1.0 - bounded).sqrt())
}

/// `GET /v1/businesses` — public provider discovery over the admin directory snapshot.
///
/// With no query this serves the WHOLE list, including contact-only providers with `geo: null`.
/// Optional `name`, `kind` (or compatibility alias `type`, never both), and the all-or-none
/// `searchCenterLat`/`searchCenterLng`/`searchRadiusKm` group narrow it with AND semantics. The route
/// preserves source order and never fabricates a coordinate; a spatial search necessarily excludes a
/// provider that published no premises.
///
/// ## Privacy boundary: search center is deliberate; current GPS is not
///
/// A search center is a location the user intentionally typed, searched for, or picked on a map. That
/// chosen place is disclosed search intent, like a provider name. The phone's live/current GPS fix is
/// involuntary and continuous and MUST NOT be put into these fields. "Near me" still performs the bare
/// full fetch and computes distance/sort on the device. This indexer is centralized and not
/// caller-swappable, so a filtered request is intentionally distinguishable beside the caller's IP and
/// timing; that is acceptable only for the intent the user chose to disclose. The server sees only
/// numbers and cannot prove their provenance, so the semantic names and the native clients' no-query
/// request tests are load-bearing. If the product ever submits current position to a server, that must
/// be a separate, deliberate, disclosed feature — never a quiet reuse of this chosen-search-location
/// group.
///
/// Ambiguous aliases (`near`, bare `lat`/`lng`, `radius`, bounding boxes, geohashes) are rejected by
/// `deny_unknown_fields`; they cannot silently become a loaded current-position path.
///
/// ## Full-fetch scale
///
/// At roughly 100 compressed bytes per provider, 50,000 rows are about 5,000,000 bytes (4.77 MiB):
/// the point where a cold full fetch becomes a live cellular-budget question. The service negotiates
/// gzip for large responses. Delta sync is the next full-set optimization; explicit search filters are
/// already available for the featureful search flow.
async fn businesses(
    State(st): State<AppState>,
    Query(params): Query<BusinessDirectoryParams>,
) -> Result<Json<Value>, ApiError> {
    let name = nonempty_filter(params.name.clone(), "name")?.map(|name| search_fold(&name));
    let kind = kind_filter(&params)?;
    let area = search_area(&params)?;
    let businesses = st.directory.businesses().ok_or_else(|| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "provider directory has no successful source snapshot yet",
        )
    })?;

    let businesses: Vec<_> = businesses
        .into_iter()
        .filter(|business| {
            name.as_ref()
                .map(|needle| search_fold(&business.name).contains(needle))
                .unwrap_or(true)
        })
        .filter(|business| {
            kind.as_ref()
                .map(|expected| business.kind.trim().eq_ignore_ascii_case(expected))
                .unwrap_or(true)
        })
        .filter(|business| {
            area.map(|area| {
                business
                    .geo
                    .filter(|geo| geo.is_valid())
                    .map(|geo| haversine_km(area.lat, area.lng, geo.lat, geo.lng) <= area.radius_km)
                    .unwrap_or(false)
            })
            .unwrap_or(true)
        })
        .collect();

    Ok(Json(json!({ "businesses": businesses })))
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
    let (page, total) = st
        .store
        .query_events(&q, &principal.scope)
        .await
        .map_err(store_unreadable)?;
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
    let (all, total) = st
        .store
        .query_events(&q, &principal.scope)
        .await
        .map_err(store_unreadable)?;

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
    let (all, _) = st
        .store
        .query_events(&q, &principal.scope)
        .await
        .map_err(store_unreadable)?;

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
/// Compression lives here so every embedding of the public whole-directory route negotiates gzip,
/// including hermetic tests and deployments that do not use the standalone binary's assembly.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/businesses", get(businesses))
        .route("/v1/status", get(status))
        .route("/v1/events", get(events))
        .route("/v1/stats", get(stats))
        .route("/v1/issuers", get(issuers))
        .with_state(state)
        .layer(CompressionLayer::new())
}
