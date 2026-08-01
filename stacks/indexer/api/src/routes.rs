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

use std::collections::HashSet;

use axum::extract::{Query, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::Query as MultiQuery;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tower_http::compression::CompressionLayer;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

use crate::app::{keccak_key, AppState};
use crate::events::{EventType, Finality, IndexedEvent};
use crate::mirror::{ContentRead, MAX_CONTENT_BYTES};
use crate::scope::Principal;
use crate::store::{EventQuery, StoreError};

type ApiError = (StatusCode, Json<Value>);

const MAX_DIRECTORY_NAME_FILTER_CHARS: usize = 200;
const MAX_DIRECTORY_KIND_FILTERS: usize = 16;
const MAX_DIRECTORY_KIND_FILTER_CHARS: usize = 64;
const MAX_CONCURRENT_DIRECTORY_SCANS: usize = 2;

// Scanning a hundred-thousand-row snapshot is CPU work, not async I/O, on BOTH public directory
// routes: a `name` filter folds every row through NFD normalization whether or not a distance is also
// computed, and the loop must visit every match anyway to count `total`, so the page size bounds
// neither. One fixed permit is therefore the honest ceiling on total directory-scan CPU, and keeps
// these unauthenticated routes from filling Tokio's blocking pool or starving the indexer's
// health/query paths. Waiting for a permit is asynchronous; a durable spatial index can replace the
// scan without changing either route contract.
static DIRECTORY_SCAN_PERMITS: Semaphore = Semaphore::const_new(MAX_CONCURRENT_DIRECTORY_SCANS);

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
    #[serde(default)]
    name: Vec<String>,
    /// Caller-selected set of case-insensitive exact matches against the business row's `type`.
    #[serde(default)]
    kind: Vec<String>,
    /// Compatibility spelling used by the existing admin directory/client contract.
    #[serde(rename = "type", default)]
    business_type: Vec<String>,
    /// Page size. Uses the same configured default/maximum as the oversight feed.
    #[serde(default)]
    limit: Vec<usize>,
    /// Zero-based result offset.
    #[serde(default)]
    offset: Vec<usize>,
}

/// The caller's current position, at whatever precision the device reported.
///
/// Captain's ruling, 2026-07-30: the position is sent EXACTLY and is not rounded. An earlier revision
/// took a three-decimal approximation and rejected anything finer; that is gone, and the fields are
/// named `lat`/`lng` rather than `approximateLat`/`approximateLng` because a field named "approximate"
/// carrying a metre-precise fix would overstate the privacy the wire format actually provides - the
/// same class of false claim this service refuses everywhere else.
///
/// What did NOT change is where it may travel: a POST body, never the URI that conventional access
/// logs record, and never a log line, trace span, metric label, cache key, or stored row.
// Deliberately no `Debug` or `Serialize`: this value must not become loggable or echoable by
// convenience while it is in memory.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CallerPosition {
    lat: f64,
    lng: f64,
}

fn bounded_filter(
    value: Option<String>,
    field: &str,
    max_chars: usize,
) -> Result<Option<String>, ApiError> {
    match value {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Err(err(
            StatusCode::BAD_REQUEST,
            &format!("{field} must not be blank"),
        )),
        Some(value) => {
            let value = value.trim();
            if value.chars().count() > max_chars {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    &format!("{field} must be at most {max_chars} characters"),
                ));
            }
            Ok(Some(value.to_string()))
        }
    }
}

fn kind_filters(params: &BusinessDirectoryParams) -> Result<HashSet<String>, ApiError> {
    if !params.kind.is_empty() && !params.business_type.is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "kind and type are aliases; use only one spelling",
        ));
    }
    let (values, field) = if params.kind.is_empty() {
        (&params.business_type, "type")
    } else {
        (&params.kind, "kind")
    };
    if values.len() > MAX_DIRECTORY_KIND_FILTERS {
        return Err(err(
            StatusCode::BAD_REQUEST,
            &format!("{field} must be provided at most {MAX_DIRECTORY_KIND_FILTERS} times"),
        ));
    }
    let mut kinds = HashSet::with_capacity(values.len());
    for value in values {
        let kind = bounded_filter(Some(value.clone()), field, MAX_DIRECTORY_KIND_FILTER_CHARS)?
            .expect("a supplied nonblank kind has a value")
            .to_ascii_lowercase();
        kinds.insert(kind);
    }
    Ok(kinds)
}

fn name_filter(params: &BusinessDirectoryParams) -> Result<Option<String>, ApiError> {
    match params.name.as_slice() {
        [] => Ok(None),
        [name] => bounded_filter(Some(name.clone()), "name", MAX_DIRECTORY_NAME_FILTER_CHARS),
        _ => Err(err(
            StatusCode::BAD_REQUEST,
            "name must be provided at most once",
        )),
    }
}

fn single_usize(values: &[usize], field: &str) -> Result<Option<usize>, ApiError> {
    match values {
        [] => Ok(None),
        [value] => Ok(Some(*value)),
        _ => Err(err(
            StatusCode::BAD_REQUEST,
            &format!("{field} must be provided at most once"),
        )),
    }
}

fn directory_page(
    st: &AppState,
    params: &BusinessDirectoryParams,
) -> Result<(usize, usize), ApiError> {
    let limit = single_usize(&params.limit, "limit")?
        .unwrap_or(st.cfg.default_page_limit)
        .clamp(1, st.cfg.max_page_limit);
    let offset = single_usize(&params.offset, "offset")?.unwrap_or(0);
    Ok((limit, offset))
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

fn matches_directory_filters(
    business: &crate::directory::BusinessRow,
    name: Option<&str>,
    kinds: &HashSet<String>,
) -> bool {
    let business_kind = business.kind.trim();
    name.map(|needle| search_fold(&business.name).contains(needle))
        .unwrap_or(true)
        && (kinds.is_empty()
            || kinds.contains(business_kind)
            || kinds
                .iter()
                .any(|expected| business_kind.eq_ignore_ascii_case(expected)))
}

/// Total great-circle distance in kilometres. The `atan2` form remains defined at antipodes.
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

/// Well-formedness only: finite, and on the globe.
///
/// There is deliberately NO precision test. An earlier revision refused anything finer than three
/// decimals as defense-in-depth behind client-side rounding; the captain has since ruled that the
/// device sends its exact fix, so such a check would now reject every real request. Do not reinstate
/// one without the rounding it was defending - a precision gate with nothing rounding in front of it
/// only rejects honest callers.
fn valid_caller_position(position: CallerPosition) -> bool {
    position.lat.is_finite()
        && position.lng.is_finite()
        && (-90.0..=90.0).contains(&position.lat)
        && (-180.0..=180.0).contains(&position.lng)
}

fn render_business(business: &crate::directory::BusinessRow, distance_km: Option<f64>) -> Value {
    let mut value =
        serde_json::to_value(business).expect("validated directory rows are JSON-serializable");
    if let (Value::Object(map), Some(distance_km)) = (&mut value, distance_km) {
        map.insert("distanceKm".into(), json!(distance_km));
    }
    value
}

struct DirectoryPage {
    businesses: Vec<Value>,
    total: usize,
    has_more: bool,
}

/// Run one directory scan on a blocking thread, under a scan permit held for its whole duration.
///
/// Both public directory routes go through here. Keeping the permit inside the blocking closure means
/// dropping or cancelling the HTTP future cannot release it while its detached work is still running.
async fn scan_directory<F>(scan: F) -> Result<DirectoryPage, ApiError>
where
    F: FnOnce() -> DirectoryPage + Send + 'static,
{
    let permit = DIRECTORY_SCAN_PERMITS.acquire().await.map_err(|_| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "provider directory search is unavailable",
        )
    })?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        scan()
    })
    .await
    .map_err(|_| {
        tracing::error!("provider directory scan worker failed");
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "provider directory search could not complete",
        )
    })
}

/// CPU-only source-order selection over one immutable directory snapshot.
///
/// Every matching row is visited so `total` is exact; only the requested window is serialized.
fn source_order_page(
    businesses: &[crate::directory::BusinessRow],
    name: Option<&str>,
    kinds: &HashSet<String>,
    limit: usize,
    offset: usize,
) -> DirectoryPage {
    let mut total = 0usize;
    let mut page = Vec::with_capacity(limit);
    for business in businesses
        .iter()
        .filter(|business| matches_directory_filters(business, name, kinds))
    {
        if total >= offset && page.len() < limit {
            page.push(render_business(business, None));
        }
        total = total.saturating_add(1);
    }
    let has_more = offset.saturating_add(page.len()) < total;
    DirectoryPage {
        businesses: page,
        total,
        has_more,
    }
}

/// CPU-only nearest selection over one immutable directory snapshot.
///
/// The two nth-element partitions select the same globally ordered window as a full sort while only
/// sorting the requested page. `source_index` is a total, deterministic tie-break inside this
/// snapshot. An offset beyond `total` is deliberately a valid empty page which echoes the requested
/// offset at the HTTP layer.
fn rank_nearest_page(
    businesses: &[crate::directory::BusinessRow],
    name: Option<&str>,
    kinds: &HashSet<String>,
    position: CallerPosition,
    limit: usize,
    offset: usize,
) -> DirectoryPage {
    let mut ranked: Vec<(f64, usize)> = businesses
        .iter()
        .enumerate()
        .filter(|(_, business)| matches_directory_filters(business, name, kinds))
        .filter_map(|(source_index, business)| {
            business.geo.filter(|geo| geo.is_valid()).map(|geo| {
                (
                    haversine_km(
                        position.lat,
                        position.lng,
                        geo.lat,
                        geo.lng,
                    ),
                    source_index,
                )
            })
        })
        .collect();
    let total = ranked.len();
    let start = offset.min(total);
    let end = offset.saturating_add(limit).min(total);
    let compare =
        |a: &(f64, usize), b: &(f64, usize)| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1));
    if start < end {
        if start > 0 {
            ranked.select_nth_unstable_by(start, compare);
        }
        if end < total {
            ranked[start..].select_nth_unstable_by(end - start, compare);
        }
        ranked[start..end].sort_by(compare);
    }

    let page = ranked[start..end]
        .iter()
        .map(|(distance_km, source_index)| {
            render_business(&businesses[*source_index], Some(*distance_km))
        })
        .collect();
    DirectoryPage {
        businesses: page,
        total,
        has_more: end < total,
    }
}

/// `GET /v1/businesses` — public provider discovery over the admin directory snapshot.
///
/// This non-location route accepts optional `name`, repeatable `kind` (or repeatable compatibility
/// alias `type`, but never both spellings), plus `limit`/`offset`. Repeated kinds are ORed; name and
/// kind compose with AND semantics. Results are paged in source order and include contact-only
/// providers with `geo: null`. The service has NO owner-app kind allowlist: a pet-owner caller sends
/// `kind=vet&kind=groomer`, while bare GET can page through every published kind, including admin and
/// government. Unknown future kind strings are not rejected or reinterpreted.
///
/// ## Nearest is a separate, body-only disclosure
///
/// Caller position is accepted only by `POST /v1/businesses/nearest`, in a JSON body, at whatever
/// precision the device reported - the captain ruled on 2026-07-30 that the exact fix is sent and is
/// not rounded, so nothing here may describe it as an approximation. It is never a URL/query
/// parameter, because conventional access logs record URLs. The indexer has no request/trace/metrics
/// middleware and this handler must never log, persist, label, or echo the position. The nearest
/// response is `Cache-Control: private, no-store`, ordered by server-computed `distanceKm`, and paged
/// so a hundred-thousand-row directory never crosses the device or gets scanned there.
///
/// There is deliberately no radius, map viewport, bounding box, geohash, place text, autocomplete, or
/// third-party geocoding parameter on either route. Unknown fields are rejected, not ignored.
async fn businesses(
    State(st): State<AppState>,
    MultiQuery(params): MultiQuery<BusinessDirectoryParams>,
) -> Result<Json<Value>, ApiError> {
    let name = name_filter(&params)?.map(|name| search_fold(&name));
    let kinds = kind_filters(&params)?;
    let (limit, offset) = directory_page(&st, &params)?;
    let businesses = st.directory.businesses().ok_or_else(|| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "provider directory has no successful source snapshot yet",
        )
    })?;

    let page = scan_directory(move || {
        source_order_page(businesses.as_ref(), name.as_deref(), &kinds, limit, offset)
    })
    .await?;

    Ok(Json(json!({
        "businesses": page.businesses,
        "total": page.total,
        "limit": limit,
        "offset": offset,
        "hasMore": page.has_more,
    })))
}

/// `POST /v1/businesses/nearest` — paged server-side proximity over the caller's exact fix.
///
/// Position MUST remain body-only and ephemeral. Do not add request logging around this route and do
/// not attach the body to a trace span, metric label, audit row, cache key, or error message.
async fn nearest_businesses(
    State(st): State<AppState>,
    MultiQuery(params): MultiQuery<BusinessDirectoryParams>,
    Json(position): Json<CallerPosition>,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    if !valid_caller_position(position) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "lat/lng must be finite coordinates within range",
        ));
    }
    let name = name_filter(&params)?.map(|name| search_fold(&name));
    let kinds = kind_filters(&params)?;
    let (limit, offset) = directory_page(&st, &params)?;
    let businesses = st.directory.businesses().ok_or_else(|| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "provider directory has no successful source snapshot yet",
        )
    })?;

    let nearest = scan_directory(move || {
        rank_nearest_page(
            businesses.as_ref(),
            name.as_deref(),
            &kinds,
            position,
            limit,
            offset,
        )
    })
    .await?;
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    Ok((
        headers,
        Json(json!({
            "businesses": nearest.businesses,
            "total": nearest.total,
            "limit": limit,
            "offset": offset,
            "hasMore": nearest.has_more,
        })),
    ))
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

// ------------------------------------------------------------------------------------------------
// The content-addressed mirror (S-17)
// ------------------------------------------------------------------------------------------------

/// `GET /v1/content/:address` — serve the bytes stored at one content address.
///
/// **Public and unauthenticated, like the rest of the provider-directory read surface.** What it
/// serves is publication-safe by construction: a provider's own contact blob and logo, whose digest
/// is already published on chain for anyone to read.
///
/// Serving is NOT evidence and this response does not ask to be believed. The caller recomputes the
/// digest over the body and compares it to the address it requested; that recomputation is the
/// security boundary. See `crate::mirror` for why the checks on this side are defence in depth.
///
/// Three outcomes, kept apart because they have three different remedies:
///   - **200** — bytes that re-hashed to the requested address on the way out;
///   - **404** — the mirror holds nothing here (a FACT: never published, or withdrawn);
///   - **503** — the store could not be read, or holds bytes that no longer hash to their key.
///
/// A store failure must never arrive as a 404. "We hold nothing" and "we could not look" are the
/// could-not-check/absence pair this codebase refuses to collapse everywhere else, and a publisher
/// told 404 would re-publish content that is already there while the real fault went uninvestigated.
async fn content(
    State(st): State<AppState>,
    axum::extract::Path(address): axum::extract::Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let read = st.mirror.get(&address).await.map_err(|e| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("content mirror unavailable: {e}"),
        )
    })?;

    match read {
        ContentRead::Found { bytes, media_type } => {
            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_str(&media_type)
                    .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
            );
            // Content-addressed bytes are immutable by construction: the address cannot name
            // different bytes later, because different bytes have a different address.
            headers.insert(
                CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            );
            // The bytes are rendered in operator portals. Even though the media type is sniffed and
            // allowlisted on ingest, nothing served from here may be interpreted as a document.
            headers.insert(
                axum::http::header::HeaderName::from_static("x-content-type-options"),
                HeaderValue::from_static("nosniff"),
            );
            headers.insert(
                axum::http::header::CONTENT_DISPOSITION,
                HeaderValue::from_static("inline"),
            );
            Ok((headers, bytes))
        }
        ContentRead::Absent => Err(err(
            StatusCode::NOT_FOUND,
            "no content is mirrored at this address",
        )),
        ContentRead::Corrupt => Err(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "mirrored content no longer hashes to its address and will not be served",
        )),
    }
}

/// `PUT /v1/content/:address` — publish bytes under their own content address.
///
/// Bearer-gated by the ordinary scope registry: writing to the mirror is an operator action, and an
/// open write surface on a public read endpoint is a free content host. Any recognised principal may
/// publish, scoped or unscoped, because the address constrains what can be written far more tightly
/// than a scope could — a caller can only ever store the one blob that hashes to the address it
/// names, so it cannot overwrite, displace or shadow anybody else's content.
///
/// The ingest check refuses content that does not hash to `address`. That is what makes this a
/// content-addressed store rather than a key-value store with hexadecimal keys.
async fn put_content(
    State(st): State<AppState>,
    axum::extract::Path(address): axum::extract::Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    authenticate(&st, &headers)?;

    if body.len() > MAX_CONTENT_BYTES {
        return Err(err(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("content is over the {MAX_CONTENT_BYTES}-byte limit"),
        ));
    }
    let media_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        // Parameters (`; charset=utf-8`) describe the transfer, not the type being allowlisted.
        .map(|v| v.split(';').next().unwrap_or(v).trim().to_ascii_lowercase())
        .unwrap_or_default();

    let outcome = st
        .mirror
        .put(&address, body.to_vec(), &media_type)
        .await
        .map_err(|e| {
            err(
                StatusCode::SERVICE_UNAVAILABLE,
                &format!("content mirror unavailable: {e}"),
            )
        })?;

    match outcome {
        Ok(stored) => Ok(Json(json!({ "address": stored, "bytes": body.len() }))),
        // 400 rather than 409: the request is wrong, not in conflict with existing state. A
        // mismatched address means the caller computed one of the two values incorrectly.
        Err(rejection) => Err(err(StatusCode::BAD_REQUEST, &rejection.message())),
    }
}

/// The router. `demo` toggles a permissive posture only via env in `main`; scoping is always enforced.
/// Compression lives here so every embedding of the public directory routes negotiates gzip,
/// including hermetic tests and deployments that do not use the standalone binary's assembly.
///
/// There is intentionally no HTTP trace/access-log middleware. In particular, never attach a
/// request URI or the nearest-search body to tracing: the caller position is ephemeral
/// request data, not an event, metric dimension, or audit fact.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/businesses", get(businesses))
        .route("/v1/businesses/nearest", post(nearest_businesses))
        .route("/v1/content/:address", get(content).put(put_content))
        .route("/v1/status", get(status))
        .route("/v1/events", get(events))
        .route("/v1/stats", get(stats))
        .route("/v1/issuers", get(issuers))
        .with_state(state)
        .layer(CompressionLayer::new())
}
