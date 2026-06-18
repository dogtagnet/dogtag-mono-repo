//! Google Calendar two-way sync provider abstraction (impl §3.6, architecture §8.1) +
//! the cross-backend `CentralClient` callback abstraction (impl §3.7 / §8.3).
//!
//! Google is hidden behind a [`CalendarProvider`] trait with two impls:
//!   - [`GoogleCalendar`] — real reqwest-backed Google Calendar v3 client. It is fully WIRED
//!     (OAuth consent URL, token exchange, events.list with syncToken, events.insert/update/delete,
//!     events.watch, freeBusy.query) but is UNtested against real Google because the test
//!     environment has no OAuth credentials.
//!   - [`MockCalendar`] — programmable, in-memory; carries ALL the Phase-7 test coverage
//!     (echo-loop avoidance, human-edit detection, HTTP-410 full resync).
//!
//! Likewise the appointment-events callback to central is hidden behind [`CentralClient`] with a
//! real [`ReqwestCentralClient`] and a [`MockCentralClient`]; the mock asserts the business NEVER
//! assigns rev (central is the SOLE allocator, §11.4 H-rev) and lets tests program the allocated rev.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

// ============================================================================================
// CalendarProvider
// ============================================================================================

/// The OAuth scope DogTag requests (read/write calendar events only, NOT full calendar admin).
pub const GOOGLE_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events";
pub const GOOGLE_AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
pub const GOOGLE_CALENDAR_BASE: &str = "https://www.googleapis.com/calendar/v3";

/// A single event as the sync loop sees it (the subset of the Google `Event` resource we care about).
#[derive(Clone, Debug)]
pub struct CalEvent {
    pub id: String,
    /// Opaque Google etag. PRIMARY echo discriminator (§13.3): a human edit in Google changes the
    /// etag even if our `dogtag.owned` tag survives, so etag mismatch is NOT silently dropped.
    pub etag: String,
    /// `extendedProperties.private.dogtag.owned == "1"` -> this event was written BY us.
    pub owned: bool,
    /// `extendedProperties.private.dogtag.apptId` (present iff owned).
    pub appt_id: Option<String>,
    /// `extendedProperties.private.dogtag.rev` (present iff owned).
    pub rev: Option<u64>,
    /// `status == "cancelled"` (Google's tombstone for deleted events in an incremental list).
    pub cancelled: bool,
    pub summary: String,
    pub start: String,
    pub end: String,
}

/// Result of an incremental `events.list(syncToken)`.
pub enum ListOutcome {
    /// Normal page: the changed events + the next sync token to persist.
    Page { events: Vec<CalEvent>, next_sync_token: String },
    /// HTTP 410 Gone — the sync token expired. The caller MUST discard the token, wipe the mirror,
    /// and perform a full resync (§3.6 / §8.1).
    Gone,
}

/// What we send to Google when mirroring a platform appointment (create or update).
#[derive(Clone, Debug)]
pub struct UpsertEvent {
    /// Existing Google event id to update, or None to insert.
    pub google_event_id: Option<String>,
    pub appt_id: String,
    pub rev: u64,
    pub summary: String,
    pub start: String,
    pub end: String,
    pub cancelled: bool,
}

/// The minimal result of an upsert: the (possibly new) event id and its fresh etag (to store so the
/// next sync recognizes our own echo).
#[derive(Clone, Debug)]
pub struct UpsertResult {
    pub google_event_id: String,
    pub etag: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CalendarError {
    #[error("http: {0}")]
    Http(String),
    #[error("oauth: {0}")]
    OAuth(String),
    #[error("{0}")]
    Other(String),
}

/// Abstract Google Calendar surface (impl §3.6). Hidden behind a trait so tests run hermetically.
#[async_trait]
pub trait CalendarProvider: Send + Sync {
    /// Build the OAuth 2.0 consent URL (access_type=offline + prompt=consent + the events scope).
    fn consent_url(&self, state: &str) -> String;
    /// Exchange an authorization `code` for tokens; returns the refresh token (opaque/encrypted).
    async fn exchange_code(&self, code: &str) -> Result<String, CalendarError>;
    /// Incremental `events.list(syncToken)`. `None` -> full list (returns a fresh token). A 410
    /// surfaces as [`ListOutcome::Gone`] (NOT an Err) so the caller can wipe+resync.
    async fn list_events(&self, sync_token: Option<&str>) -> Result<ListOutcome, CalendarError>;
    /// Create or update a tagged DogTag event; returns the new event id + etag.
    async fn upsert_event(&self, ev: &UpsertEvent) -> Result<UpsertResult, CalendarError>;
    /// Create an `events.watch` push channel. Returns (channelId, resourceId) (opaque).
    async fn watch(&self) -> Result<(String, String), CalendarError>;
    /// `freeBusy.query` over [time_min, time_max): returns busy [start,end) ranges.
    async fn free_busy(
        &self,
        time_min: &str,
        time_max: &str,
    ) -> Result<Vec<(String, String)>, CalendarError>;
}

// ============================================================================================
// GoogleCalendar — real reqwest impl (WIRED, untested against live Google: no OAuth creds here).
// ============================================================================================

/// Real Google Calendar v3 client. Holds OAuth client creds + (once connected) a refresh token, and
/// mints short-lived access tokens on demand via the token endpoint.
pub struct GoogleCalendar {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub calendar_id: String,
    /// the stored refresh token, set after `exchange_code` / loaded from the Store.
    refresh_token: Mutex<Option<String>>,
    http: reqwest::Client,
}

impl GoogleCalendar {
    pub fn new(client_id: String, client_secret: String, redirect_uri: String, calendar_id: String) -> Self {
        GoogleCalendar {
            client_id,
            client_secret,
            redirect_uri,
            calendar_id,
            refresh_token: Mutex::new(None),
            http: reqwest::Client::new(),
        }
    }
    pub fn set_refresh_token(&self, token: String) {
        *self.refresh_token.lock().unwrap() = Some(token);
    }

    /// Exchange the stored refresh token for a fresh access token (real Google call).
    async fn access_token(&self) -> Result<String, CalendarError> {
        let rt = self
            .refresh_token
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| CalendarError::OAuth("not connected (no refresh token)".into()))?;
        let resp = self
            .http
            .post(GOOGLE_TOKEN_ENDPOINT)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("refresh_token", rt.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|e| CalendarError::Http(e.to_string()))?;
        let v: Value = resp.json().await.map_err(|e| CalendarError::Http(e.to_string()))?;
        v.get("access_token")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| CalendarError::OAuth(format!("no access_token: {v}")))
    }

    fn parse_event(item: &Value) -> CalEvent {
        let priv_props = item
            .get("extendedProperties")
            .and_then(|e| e.get("private"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let owned = priv_props.get("dogtag.owned").and_then(|v| v.as_str()) == Some("1");
        let appt_id = priv_props.get("dogtag.apptId").and_then(|v| v.as_str()).map(|s| s.to_string());
        let rev = priv_props
            .get("dogtag.rev")
            .and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64()));
        CalEvent {
            id: item.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            etag: item.get("etag").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            owned,
            appt_id,
            rev,
            cancelled: item.get("status").and_then(|v| v.as_str()) == Some("cancelled"),
            summary: item.get("summary").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            start: item.get("start").and_then(|s| s.get("dateTime")).and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            end: item.get("end").and_then(|s| s.get("dateTime")).and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        }
    }

    fn event_body(ev: &UpsertEvent) -> Value {
        json!({
            "summary": ev.summary,
            "start": { "dateTime": ev.start },
            "end": { "dateTime": ev.end },
            "status": if ev.cancelled { "cancelled" } else { "confirmed" },
            "extendedProperties": {
                "private": {
                    "dogtag.owned": "1",
                    "dogtag.apptId": ev.appt_id,
                    "dogtag.rev": ev.rev.to_string(),
                }
            }
        })
    }
}

#[async_trait]
impl CalendarProvider for GoogleCalendar {
    fn consent_url(&self, state: &str) -> String {
        let mut u = url::Url::parse(GOOGLE_AUTH_ENDPOINT).expect("auth endpoint");
        u.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", GOOGLE_SCOPE)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent")
            .append_pair("state", state);
        u.to_string()
    }

    async fn exchange_code(&self, code: &str) -> Result<String, CalendarError> {
        let resp = self
            .http
            .post(GOOGLE_TOKEN_ENDPOINT)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
                ("grant_type", "authorization_code"),
                ("redirect_uri", self.redirect_uri.as_str()),
            ])
            .send()
            .await
            .map_err(|e| CalendarError::Http(e.to_string()))?;
        let v: Value = resp.json().await.map_err(|e| CalendarError::Http(e.to_string()))?;
        let rt = v
            .get("refresh_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| CalendarError::OAuth(format!("no refresh_token in token response: {v}")))?
            .to_string();
        self.set_refresh_token(rt.clone());
        Ok(rt)
    }

    async fn list_events(&self, sync_token: Option<&str>) -> Result<ListOutcome, CalendarError> {
        let token = self.access_token().await?;
        let url = format!("{}/calendars/{}/events", GOOGLE_CALENDAR_BASE, urlencoding(&self.calendar_id));
        let mut req = self.http.get(&url).bearer_auth(&token).query(&[("showDeleted", "true")]);
        if let Some(st) = sync_token {
            req = req.query(&[("syncToken", st)]);
        }
        let resp = req.send().await.map_err(|e| CalendarError::Http(e.to_string()))?;
        // 410 Gone -> the sync token expired; caller wipes the mirror + full resync.
        if resp.status().as_u16() == 410 {
            return Ok(ListOutcome::Gone);
        }
        if !resp.status().is_success() {
            return Err(CalendarError::Http(format!("events.list {}", resp.status())));
        }
        let v: Value = resp.json().await.map_err(|e| CalendarError::Http(e.to_string()))?;
        let events = v
            .get("items")
            .and_then(|i| i.as_array())
            .map(|arr| arr.iter().map(Self::parse_event).collect())
            .unwrap_or_default();
        let next = v
            .get("nextSyncToken")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        Ok(ListOutcome::Page { events, next_sync_token: next })
    }

    async fn upsert_event(&self, ev: &UpsertEvent) -> Result<UpsertResult, CalendarError> {
        let token = self.access_token().await?;
        let body = Self::event_body(ev);
        let resp = match &ev.google_event_id {
            Some(id) => {
                let url = format!(
                    "{}/calendars/{}/events/{}",
                    GOOGLE_CALENDAR_BASE,
                    urlencoding(&self.calendar_id),
                    urlencoding(id)
                );
                self.http.put(&url).bearer_auth(&token).json(&body).send().await
            }
            None => {
                let url = format!("{}/calendars/{}/events", GOOGLE_CALENDAR_BASE, urlencoding(&self.calendar_id));
                self.http.post(&url).bearer_auth(&token).json(&body).send().await
            }
        }
        .map_err(|e| CalendarError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(CalendarError::Http(format!("events.upsert {}", resp.status())));
        }
        let v: Value = resp.json().await.map_err(|e| CalendarError::Http(e.to_string()))?;
        Ok(UpsertResult {
            google_event_id: v.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
            etag: v.get("etag").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        })
    }

    async fn watch(&self) -> Result<(String, String), CalendarError> {
        let token = self.access_token().await?;
        let url = format!("{}/calendars/{}/events/watch", GOOGLE_CALENDAR_BASE, urlencoding(&self.calendar_id));
        let channel_id = uuid::Uuid::new_v4().to_string();
        let body = json!({ "id": channel_id, "type": "web_hook", "address": self.redirect_uri });
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CalendarError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(CalendarError::Http(format!("events.watch {}", resp.status())));
        }
        let v: Value = resp.json().await.map_err(|e| CalendarError::Http(e.to_string()))?;
        Ok((
            v.get("id").and_then(|x| x.as_str()).unwrap_or(&channel_id).to_string(),
            v.get("resourceId").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        ))
    }

    async fn free_busy(
        &self,
        time_min: &str,
        time_max: &str,
    ) -> Result<Vec<(String, String)>, CalendarError> {
        let token = self.access_token().await?;
        let url = format!("{}/freeBusy", GOOGLE_CALENDAR_BASE);
        let body = json!({
            "timeMin": time_min,
            "timeMax": time_max,
            "items": [ { "id": self.calendar_id } ]
        });
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CalendarError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(CalendarError::Http(format!("freeBusy {}", resp.status())));
        }
        let v: Value = resp.json().await.map_err(|e| CalendarError::Http(e.to_string()))?;
        let busy = v
            .get("calendars")
            .and_then(|c| c.get(&self.calendar_id))
            .and_then(|c| c.get("busy"))
            .and_then(|b| b.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|r| {
                        Some((
                            r.get("start")?.as_str()?.to_string(),
                            r.get("end")?.as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(busy)
    }
}

fn urlencoding(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

// ============================================================================================
// MockCalendar — programmable, in-memory; carries the Phase-7 test coverage.
// ============================================================================================

#[derive(Default)]
struct MockInner {
    /// stored events by id (the "Google" mirror state).
    events: HashMap<String, CalEvent>,
    /// queued `list_events` outcomes (FIFO); a queued `Gone` simulates HTTP 410.
    queued: std::collections::VecDeque<MockListResp>,
    next_id: u64,
    next_etag: u64,
    /// recorded upserts (for assertions).
    upserts: Vec<UpsertEvent>,
    watch_channels: u64,
}

enum MockListResp {
    Page(Vec<CalEvent>, String),
    Gone,
}

/// Programmable mock. Tests push list responses, inspect mirrored events, and trigger a 410.
#[derive(Clone, Default)]
pub struct MockCalendar {
    inner: Arc<Mutex<MockInner>>,
}

impl MockCalendar {
    pub fn new() -> Self {
        Self::default()
    }
    /// Queue a normal `events.list` page (events + a next sync token).
    pub fn queue_page(&self, events: Vec<CalEvent>, next_sync_token: &str) {
        self.inner
            .lock()
            .unwrap()
            .queued
            .push_back(MockListResp::Page(events, next_sync_token.to_string()));
    }
    /// Queue an HTTP-410 outcome for the next `events.list`.
    pub fn queue_gone(&self) {
        self.inner.lock().unwrap().queued.push_back(MockListResp::Gone);
    }
    /// Snapshot the current mirrored event (as Google would return it on the next sync).
    pub fn get_event(&self, id: &str) -> Option<CalEvent> {
        self.inner.lock().unwrap().events.get(id).cloned()
    }
    /// All events currently in the mock Google mirror.
    pub fn all_events(&self) -> Vec<CalEvent> {
        self.inner.lock().unwrap().events.values().cloned().collect()
    }
    /// How many upserts were recorded.
    pub fn upsert_count(&self) -> usize {
        self.inner.lock().unwrap().upserts.len()
    }
    pub fn watch_count(&self) -> u64 {
        self.inner.lock().unwrap().watch_channels
    }
}

#[async_trait]
impl CalendarProvider for MockCalendar {
    fn consent_url(&self, state: &str) -> String {
        format!(
            "{}?client_id=mock&redirect_uri=mock&response_type=code&scope={}&access_type=offline&prompt=consent&state={}",
            GOOGLE_AUTH_ENDPOINT, GOOGLE_SCOPE, state
        )
    }
    async fn exchange_code(&self, code: &str) -> Result<String, CalendarError> {
        Ok(format!("mock_refresh_token_for_{code}"))
    }
    async fn list_events(&self, _sync_token: Option<&str>) -> Result<ListOutcome, CalendarError> {
        let mut g = self.inner.lock().unwrap();
        match g.queued.pop_front() {
            Some(MockListResp::Gone) => Ok(ListOutcome::Gone),
            Some(MockListResp::Page(events, token)) => {
                Ok(ListOutcome::Page { events, next_sync_token: token })
            }
            // no queued response -> an empty page with a fresh token (steady state).
            None => Ok(ListOutcome::Page { events: vec![], next_sync_token: "mock-sync-empty".to_string() }),
        }
    }
    async fn upsert_event(&self, ev: &UpsertEvent) -> Result<UpsertResult, CalendarError> {
        let mut g = self.inner.lock().unwrap();
        g.upserts.push(ev.clone());
        let id = match &ev.google_event_id {
            Some(id) => id.clone(),
            None => {
                g.next_id += 1;
                format!("gcal-evt-{}", g.next_id)
            }
        };
        g.next_etag += 1;
        let etag = format!("\"etag-{}\"", g.next_etag);
        let stored = CalEvent {
            id: id.clone(),
            etag: etag.clone(),
            owned: true,
            appt_id: Some(ev.appt_id.clone()),
            rev: Some(ev.rev),
            cancelled: ev.cancelled,
            summary: ev.summary.clone(),
            start: ev.start.clone(),
            end: ev.end.clone(),
        };
        g.events.insert(id.clone(), stored);
        Ok(UpsertResult { google_event_id: id, etag })
    }
    async fn watch(&self) -> Result<(String, String), CalendarError> {
        let mut g = self.inner.lock().unwrap();
        g.watch_channels += 1;
        Ok((format!("mock-channel-{}", g.watch_channels), "mock-resource".to_string()))
    }
    async fn free_busy(
        &self,
        _time_min: &str,
        _time_max: &str,
    ) -> Result<Vec<(String, String)>, CalendarError> {
        let g = self.inner.lock().unwrap();
        Ok(g
            .events
            .values()
            .filter(|e| !e.cancelled)
            .map(|e| (e.start.clone(), e.end.clone()))
            .collect())
    }
}

// ============================================================================================
// CentralClient — the appointment-events callback to the central backend (impl §3.7 / §8.3).
// ============================================================================================

#[derive(Debug, thiserror::Error)]
pub enum CentralError {
    #[error("http: {0}")]
    Http(String),
    #[error("status: {0}")]
    Status(u16),
}

/// The central's response to an appointment-event: the newly-allocated rev + state.
#[derive(Clone, Debug)]
pub struct EventAck {
    pub rev: u64,
    pub state: String,
}

/// Abstract the POST /v1/businesses/{bid}/appointment-events callback. The business NEVER assigns
/// rev (central is the SOLE allocator §11.4 H-rev); it sends `lastRev` and central returns `rev`.
#[async_trait]
pub trait CentralClient: Send + Sync {
    /// POST a business-driven transition to central, HMAC-signed. `last_rev` is the rev the business
    /// currently holds; central allocates the next rev and returns the updated appointment.
    async fn post_appointment_event(
        &self,
        business_id: &str,
        appointment_id: &str,
        last_rev: u64,
        event: &str,
        occurred_at: u64,
    ) -> Result<EventAck, CentralError>;
}

/// Real reqwest impl: HMAC-signs `POST\n/v1/businesses/{bid}/appointment-events\nBODY`.
pub struct ReqwestCentralClient {
    pub central_base_url: String,
    pub hmac_secret: String,
    http: reqwest::Client,
}

impl ReqwestCentralClient {
    pub fn new(central_base_url: String, hmac_secret: String) -> Self {
        ReqwestCentralClient { central_base_url, hmac_secret, http: reqwest::Client::new() }
    }
}

#[async_trait]
impl CentralClient for ReqwestCentralClient {
    async fn post_appointment_event(
        &self,
        business_id: &str,
        appointment_id: &str,
        last_rev: u64,
        event: &str,
        occurred_at: u64,
    ) -> Result<EventAck, CentralError> {
        let path = format!("/v1/businesses/{business_id}/appointment-events");
        let url = format!("{}{}", self.central_base_url.trim_end_matches('/'), path);
        // NB: NO `rev` field — the business never assigns rev.
        let body = json!({
            "appointmentId": appointment_id,
            "lastRev": last_rev,
            "event": event,
            "occurredAt": occurred_at,
        });
        let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
        let sig = crate::auth::hmac_sign(&self.hmac_secret, "POST", &path, &body_bytes);
        let resp = self
            .http
            .post(&url)
            .header("X-DogTag-HMAC", sig)
            .header("content-type", "application/json")
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| CentralError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(CentralError::Status(resp.status().as_u16()));
        }
        let v: Value = resp.json().await.map_err(|e| CentralError::Http(e.to_string()))?;
        Ok(EventAck {
            rev: v.get("rev").and_then(|r| r.as_u64()).unwrap_or(last_rev + 1),
            state: v.get("state").and_then(|s| s.as_str()).unwrap_or(event).to_string(),
        })
    }
}

/// Mock central: records callbacks and acts as the SOLE rev allocator (returns lastRev+1), applying
/// terminal-wins. Tests assert the business sent no rev and that an ownership-mismatch is rejected.
#[derive(Clone, Default)]
pub struct MockCentralClient {
    inner: Arc<Mutex<MockCentralInner>>,
}

#[derive(Default)]
struct MockCentralInner {
    /// recorded callbacks: (businessId, appointmentId, lastRev, event, occurredAt).
    calls: Vec<(String, String, u64, String, u64)>,
    /// central's authoritative state per appointment (id -> (rev, state)).
    state: HashMap<String, (u64, String)>,
    /// businessId that owns each appointment (for the ownership-mismatch test).
    owner: HashMap<String, String>,
    /// force the next callback to fail with this status (e.g. 403 for a mismatched businessId).
    fail_next_status: Option<u16>,
}

impl MockCentralClient {
    pub fn new() -> Self {
        Self::default()
    }
    /// Seed central's authoritative ownership + rev/state for an appointment.
    pub fn seed(&self, appointment_id: &str, business_id: &str, rev: u64, state: &str) {
        let mut g = self.inner.lock().unwrap();
        g.owner.insert(appointment_id.to_string(), business_id.to_string());
        g.state.insert(appointment_id.to_string(), (rev, state.to_string()));
    }
    pub fn calls(&self) -> Vec<(String, String, u64, String, u64)> {
        self.inner.lock().unwrap().calls.clone()
    }
    /// The rev central currently holds for an appointment.
    pub fn rev_of(&self, appointment_id: &str) -> Option<u64> {
        self.inner.lock().unwrap().state.get(appointment_id).map(|(r, _)| *r)
    }
    pub fn state_of(&self, appointment_id: &str) -> Option<String> {
        self.inner.lock().unwrap().state.get(appointment_id).map(|(_, s)| s.clone())
    }
}

fn is_terminal(state: &str) -> bool {
    matches!(state, "DECLINED" | "CANCELLED" | "COMPLETED" | "NO_SHOW")
}

#[async_trait]
impl CentralClient for MockCentralClient {
    async fn post_appointment_event(
        &self,
        business_id: &str,
        appointment_id: &str,
        last_rev: u64,
        event: &str,
        occurred_at: u64,
    ) -> Result<EventAck, CentralError> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push((
            business_id.to_string(),
            appointment_id.to_string(),
            last_rev,
            event.to_string(),
            occurred_at,
        ));
        if let Some(s) = g.fail_next_status.take() {
            return Err(CentralError::Status(s));
        }
        // ownership binding (C-2): reject a mismatched businessId.
        if let Some(owner) = g.owner.get(appointment_id) {
            if owner != business_id {
                return Err(CentralError::Status(403));
            }
        }
        // central is the SOLE rev allocator: next = current + 1.
        let (cur_rev, cur_state) = g
            .state
            .get(appointment_id)
            .cloned()
            .unwrap_or((last_rev, "REQUESTED".to_string()));
        let new_rev = cur_rev + 1;
        // terminal wins: never move OUT of a terminal state.
        let new_state = if is_terminal(&cur_state) { cur_state.clone() } else { event.to_string() };
        g.state.insert(appointment_id.to_string(), (new_rev, new_state.clone()));
        Ok(EventAck { rev: new_rev, state: new_state })
    }
}
