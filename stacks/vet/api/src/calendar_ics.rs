//! The shop's `.ics` CALENDAR INTEROP surface: a published subscription feed (out) and a normalized
//! import endpoint (in). No OAuth, no API keys, no provider SDK.
//!
//! WHY ICS FIRST. Google Calendar, Apple Calendar and Luma can all subscribe to an ICS URL, so ONE
//! published feed gives a shop all three at once, and it keeps working when a provider changes its
//! API terms. What it does NOT give is two-way sync: a subscription is READ-ONLY, and refresh runs
//! on the SUBSCRIBER's schedule — Google in particular has historically taken hours to re-poll a
//! subscribed URL, whatever `REFRESH-INTERVAL` the feed asks for. The portal states that plainly at
//! the point the operator copies the URL; see `docs/CALENDAR_SYNC.md` for what native two-way sync
//! would additionally require.
//!
//! THE FEED URL IS A CREDENTIAL. It is unauthenticated by construction — a calendar client cannot
//! present a bearer token — so the secret lives in the path. Hence: 32 CSPRNG bytes, compared in
//! constant time, revocable in one click, and never logged. Anyone holding the URL can read the
//! shop's entire schedule, which the UI says in as many words.
//!
//! WHERE THE PARSER IS. `.ics` FILES are parsed in the BROWSER, not here — this crate has no
//! timezone database, and `TZID=Europe/London` cannot be resolved to a UTC instant without one.
//! `packages/ui/src/calendar/ics.ts` does it with `Intl`, which resolves any IANA zone exactly, and
//! POSTs already-normalized UNIX SECONDS to [`import_events`]. This module therefore owns identity,
//! dedup and persistence for an import — never time-zone interpretation.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::auth::now;
use crate::ics::{self, IcsEvent, IcsStatus};
use crate::store::{Appointment, AppointmentQuery, IssuerSettings, Store, MAX_PAGE};

type Resp = (StatusCode, Json<Value>);
fn ok(v: Value) -> Resp {
    (StatusCode::OK, Json(v))
}
fn err(code: StatusCode, msg: &str) -> Resp {
    (code, Json(json!({ "error": msg })))
}

/// How far BACK the feed publishes. Subscribers want recent history for context, not the shop's
/// entire past; a bounded window also bounds the response size.
const FEED_PAST_SECS: u64 = 90 * 86_400;
/// How far FORWARD the feed publishes — comfortably past any booking a groomer takes in advance.
const FEED_FUTURE_SECS: u64 = 400 * 86_400;
/// Hard cap on published events. A feed that silently stops at a page boundary would look complete
/// while hiding bookings, so the cap is explicit and the truncation is ANNOUNCED (see [`feed`]).
const FEED_MAX_EVENTS: usize = 2_000;

// ============================================================================================
// token
// ============================================================================================

/// Mint a fresh feed secret: 32 CSPRNG bytes, hex. 256 bits of entropy in a URL path that is
/// otherwise unauthenticated — enumeration is not a threat model this needs to survive twice.
fn mint_token() -> String {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    hex::encode(bytes)
}

/// Constant-time equality for the feed secret.
///
/// A 256-bit random token is not realistically recoverable byte-by-byte over a network, but the
/// comparison costs nothing to do right, and "it's probably fine" is not a reason to hand an
/// attacker a timing oracle on a credential.
fn secret_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    // Length is not secret (it is a fixed 64 hex chars), so an early length check leaks nothing.
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Strip the `.ics` extension calendar clients expect on a feed URL. `/calendar/feed/<tok>` and
/// `/calendar/feed/<tok>.ics` are the same feed; the extension exists because some clients (and
/// every "download" affordance) key off it.
fn strip_ics_suffix(token: &str) -> &str {
    token.strip_suffix(".ics").unwrap_or(token)
}

// ============================================================================================
// feed (UNAUTHENTICATED — the path secret IS the credential)
// ============================================================================================

/// `GET /calendar/feed/{token}.ics` — the shop's schedule as a subscribable iCalendar document.
///
/// Answers 404 for an unknown, revoked or absent token: a 401/403 would confirm to a prober that a
/// feed exists at all, and there is nothing useful to tell someone who does not hold the secret.
async fn feed(State(st): State<AppState>, Path(token): Path<String>) -> Response {
    let supplied = strip_ics_suffix(&token);
    let settings = st.store.get_settings().await;
    let live = match settings.ics_feed_token.as_deref() {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => return not_found(),
    };
    if !secret_eq(&live, supplied) {
        return not_found();
    }

    let ts = now();
    let from = ts.saturating_sub(FEED_PAST_SECS);
    let to = ts + FEED_FUTURE_SECS;
    let (appts, truncated) = load_window(&st, from, to).await;

    let domain = if st.cfg.issuer_domain.is_empty() {
        "dogtag".to_string()
    } else {
        st.cfg.issuer_domain.clone()
    };
    let mut events: Vec<IcsEvent> = appts.iter().map(|a| to_ics_event(a, &domain)).collect();
    if truncated {
        // Say it INSIDE the calendar, where the operator's own calendar app will show it, rather
        // than only in a log nobody subscribed to.
        events.push(truncation_notice(ts, &domain));
    }

    let name = if st.cfg.issuer_name.is_empty() {
        "DogTag appointments".to_string()
    } else {
        format!("{} — appointments", st.cfg.issuer_name)
    };
    let body = ics::calendar("-//DogTag//Appointments//EN", &name, ts, &events);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/calendar; charset=utf-8"),
            // The URL carries a secret; keep it out of shared caches and proxies.
            (header::CACHE_CONTROL, "no-store, private"),
            (
                header::CONTENT_DISPOSITION,
                "inline; filename=\"dogtag-appointments.ics\"",
            ),
        ],
        body,
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "calendar feed not found" })),
    )
        .into_response()
}

/// Read the publishable window, paging through the store's bounded pages up to [`FEED_MAX_EVENTS`].
/// Returns `(appointments, truncated)`.
async fn load_window(st: &AppState, from: u64, to: u64) -> (Vec<Appointment>, bool) {
    let mut out: Vec<Appointment> = Vec::new();
    let mut offset = 0usize;
    loop {
        let page = st
            .store
            .list_appointments(&AppointmentQuery {
                from: Some(from),
                to: Some(to),
                limit: MAX_PAGE,
                offset,
                ..Default::default()
            })
            .await;
        let fetched = page.rows.len();
        // The store reports the window's TOTAL match count alongside the page, so completeness is
        // read from that rather than inferred from where a page happened to end.
        let total = page.total as usize;
        out.extend(page.rows);
        if out.len() >= FEED_MAX_EVENTS {
            out.truncate(FEED_MAX_EVENTS);
            return (out, total > FEED_MAX_EVENTS);
        }
        // A short page means the window is exhausted. A store that returns nothing at a non-zero
        // offset would otherwise spin forever, so this is also the loop's only exit.
        if fetched < MAX_PAGE {
            let complete = total <= out.len();
            return (out, !complete);
        }
        offset += fetched;
    }
}

/// The `UID` an appointment publishes under.
///
/// An imported booking re-publishes the ORIGINATING calendar's UID, so a shop that imports its old
/// calendar and then subscribes to this feed sees one event per booking rather than two.
fn event_uid(a: &Appointment, domain: &str) -> String {
    match a.external_uid.as_deref() {
        Some(uid) if !uid.is_empty() => uid.to_string(),
        _ => format!("{}@{}", a.appointment_id, domain),
    }
}

/// Human label for an appointment status, shared by the feed's SUMMARY/DESCRIPTION.
fn status_label(status: &str) -> &str {
    match status {
        "scheduled" => "Scheduled",
        "confirmed" => "Confirmed",
        "in_progress" => "In progress",
        "completed" => "Completed",
        "cancelled" => "Cancelled",
        "no_show" => "No show",
        other => other,
    }
}

fn to_ics_event(a: &Appointment, domain: &str) -> IcsEvent {
    let service = if a.service.trim().is_empty() {
        "Appointment"
    } else {
        a.service.trim()
    };
    // Who the booking is for, most specific first. An `.ics`-imported booking has no client yet, so
    // it publishes as the bare service line rather than inventing a name.
    let who: Vec<&str> = [a.pet_name.trim(), a.client_name.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    let summary = if who.is_empty() {
        service.to_string()
    } else {
        format!("{service} - {}", who.join(" / "))
    };

    let mut desc: Vec<String> = vec![format!("Status: {}", status_label(&a.status))];
    if !a.groomer.trim().is_empty() {
        desc.push(format!("Groomer: {}", a.groomer.trim()));
    }
    if !a.notes.trim().is_empty() {
        desc.push(a.notes.trim().to_string());
    }
    if a.client_id.is_empty() {
        desc.push("Unassigned — imported from a calendar file, no client linked yet.".to_string());
    }

    IcsEvent {
        uid: event_uid(a, domain),
        start: a.start_at,
        end: a.end_at,
        summary,
        description: desc.join("\n"),
        // Only `cancelled` is published as CANCELLED, because that is the one state where the
        // subscriber should DROP the event. A no-show still occupied the slot and stays on the
        // calendar with its status spelled out in the description.
        status: if a.status == "cancelled" {
            IcsStatus::Cancelled
        } else {
            IcsStatus::Confirmed
        },
        last_modified: a.updated_at,
    }
}

/// A visible marker event emitted when the window exceeded [`FEED_MAX_EVENTS`], so a truncated feed
/// never masquerades as a complete one.
fn truncation_notice(ts: u64, domain: &str) -> IcsEvent {
    IcsEvent {
        uid: format!("truncated@{domain}"),
        start: ts,
        end: ts + 900,
        summary: "DogTag: this calendar feed is truncated".to_string(),
        description: format!(
            "This feed publishes at most {FEED_MAX_EVENTS} bookings per refresh and the current \
             window has more. Bookings beyond that cap are NOT in this calendar — open the \
             Appointments list in the portal for the complete schedule."
        ),
        status: IcsStatus::Confirmed,
        last_modified: ts,
    }
}

// ============================================================================================
// feed administration (operator-gated)
// ============================================================================================

/// Persist a settings mutation without clobbering the sibling fields.
async fn update_settings(store: &std::sync::Arc<dyn Store>, mutate: impl FnOnce(&mut IssuerSettings)) -> IssuerSettings {
    let mut s = store.get_settings().await;
    mutate(&mut s);
    store.put_settings(s.clone()).await;
    s
}

fn feed_json(settings: &IssuerSettings) -> Value {
    match settings.ics_feed_token.as_deref().filter(|t| !t.is_empty()) {
        Some(token) => json!({
            "enabled": true,
            "token": token,
            // Path only. The absolute URL is composed in the browser against the portal's own
            // origin + configured API base, which is what a subscriber actually reaches — the
            // backend's own `DEPLOYMENT_URL` can legitimately differ from it (dev proxy, tunnel).
            "path": format!("/calendar/feed/{token}.ics"),
        }),
        None => json!({ "enabled": false, "token": null, "path": null }),
    }
}

/// `GET /calendar/feed` — is a feed published, and under which secret.
async fn get_feed_settings(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    ok(feed_json(&st.store.get_settings().await))
}

/// `POST /calendar/feed/rotate` — publish a feed, or replace the secret of an existing one.
///
/// Rotation is also the REVOCATION-and-republish path: the previous URL stops working the instant
/// this returns, so a shop that shared the link too widely can hand out a new one without losing the
/// feed. Every subscriber must be re-pointed, which the UI says before the operator clicks.
async fn rotate_feed(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    let token = mint_token();
    let s = update_settings(&st.store, |s| s.ics_feed_token = Some(token)).await;
    ok(feed_json(&s))
}

/// `DELETE /calendar/feed` — revoke. The URL 404s immediately; subscribers see the calendar stop
/// updating (and, in most clients, an error on next refresh).
async fn revoke_feed(State(st): State<AppState>, headers: HeaderMap) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    let s = update_settings(&st.store, |s| s.ics_feed_token = None).await;
    ok(feed_json(&s))
}

// ============================================================================================
// import (operator-gated)
// ============================================================================================

/// One event from an uploaded `.ics`, ALREADY normalized by the browser parser: instants are UNIX
/// SECONDS resolved against the event's own `TZID`/`VALUE=DATE` semantics, so this endpoint performs
/// no time-zone interpretation of its own.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ImportEvent {
    /// The source calendar's `UID`. Required: it is the ONLY thing that makes a repeated import of
    /// the same file idempotent.
    uid: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    location: String,
    start_at: u64,
    #[serde(default)]
    end_at: u64,
    /// The source event was `VALUE=DATE` (all-day). Recorded in the notes, because an `Appointment`
    /// is a slot with a start and an end and has no all-day flag to set.
    #[serde(default)]
    all_day: bool,
    /// The source event carried an `RRULE`. This import does NOT expand recurrence — see
    /// [`import_events`].
    #[serde(default)]
    recurring: bool,
    /// The source event's `STATUS` (`cancelled` is the only value acted on).
    #[serde(default)]
    status: String,
}

#[derive(Deserialize, Debug, Default)]
struct ImportBody {
    #[serde(default)]
    events: Vec<ImportEvent>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ImportQuery {
    /// `?dryRun=1` (or `true`) -> validate and report, write NOTHING. Lets the portal show the
    /// operator exactly what an import would do before it touches their book.
    #[serde(default)]
    dry_run: Option<String>,
}

impl ImportQuery {
    /// Absent -> a real import. `1`/`true` -> a preview. ANYTHING ELSE is an error, not a guess:
    /// silently treating an unrecognized value as "go ahead and write" would turn a client-side typo
    /// into an unwanted write to the shop's book, and silently treating it as a preview would make a
    /// real import quietly do nothing.
    fn dry_run(&self) -> Result<bool, Resp> {
        match self.dry_run.as_deref() {
            None => Ok(false),
            Some("1") | Some("true") => Ok(true),
            Some(_) => Err(err(
                StatusCode::BAD_REQUEST,
                "dryRun must be omitted, \"1\" or \"true\"",
            )),
        }
    }
}

/// `POST /calendar/import` — create/update bookings from a parsed `.ics`.
///
/// WHAT THIS DOES NOT DO, stated plainly rather than discovered later:
///
/// * **Recurrence is not expanded.** An event carrying an `RRULE` is imported as the SINGLE
///   occurrence its `DTSTART` names. Every such booking is stamped in its notes, and the response
///   returns `recurringNotExpanded` so the portal can report the count rather than let a weekly
///   standing appointment quietly become one booking.
/// * **All-day events become midnight-to-midnight slots** in the shop's local timezone (resolved by
///   the browser parser), stamped as all-day in the notes. The data model has no all-day flag.
/// * **No client is invented.** An imported booking is UNASSIGNED (`clientId == ""`); the operator
///   links a real client by editing it. Fabricating directory entries to fill a column would be
///   inventing customers.
///
/// Idempotency: dedup is by source `UID`. Re-importing the same file UPDATES the slot and label of
/// the booking it already created, and never duplicates it. A re-import deliberately does NOT
/// overwrite the shop's own work on that booking — the linked client, pet, groomer and a status the
/// operator has since advanced all survive — because the calendar file is the source of truth for
/// WHEN, and the portal is the source of truth for WHO.
async fn import_events(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ImportQuery>,
    Json(body): Json<ImportBody>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    let dry_run = match q.dry_run() {
        Ok(v) => v,
        Err(e) => return e,
    };

    let mut created = 0u32;
    let mut updated = 0u32;
    let mut cancelled = 0u32;
    let mut skipped = 0u32;
    let mut recurring_not_expanded = 0u32;
    let mut all_day = 0u32;
    // Within ONE upload the same UID can legitimately appear more than once (a recurring event's
    // overrides share it). Whichever we take first wins; the rest are duplicates, not new bookings.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ev in &body.events {
        let uid = ev.uid.trim();
        if uid.is_empty() || ev.start_at == 0 {
            skipped += 1;
            continue;
        }
        if !seen.insert(uid.to_string()) {
            skipped += 1;
            continue;
        }
        if ev.recurring {
            recurring_not_expanded += 1;
        }
        if ev.all_day {
            all_day += 1;
        }

        let existing = st.store.appointment_by_external_uid(uid).await;
        let is_cancelled = ev.status.eq_ignore_ascii_case("cancelled");
        match (existing, is_cancelled) {
            // A cancellation for something we never imported is a tombstone with nothing to bury.
            (None, true) => skipped += 1,
            (None, false) => {
                if !dry_run {
                    st.store.put_appointment(new_from_import(ev, uid)).await;
                }
                created += 1;
            }
            (Some(a), cancel) => {
                if !dry_run {
                    st.store.put_appointment(merge_import(a, ev, cancel)).await;
                }
                if cancel {
                    cancelled += 1;
                } else {
                    updated += 1;
                }
            }
        }
    }

    ok(json!({
        "dryRun": dry_run,
        "total": body.events.len(),
        "created": created,
        "updated": updated,
        "cancelled": cancelled,
        "skipped": skipped,
        "recurringNotExpanded": recurring_not_expanded,
        "allDay": all_day,
    }))
}

/// The service label an imported event books under (its `SUMMARY`, or an honest placeholder).
fn import_service(ev: &ImportEvent) -> String {
    let s = ev.summary.trim();
    if s.is_empty() {
        "Imported event".to_string()
    } else {
        s.to_string()
    }
}

/// The notes block, carrying the source event's own text plus every caveat this import applied — so
/// the operator reads the compromise on the booking itself, not only in a summary they dismissed.
fn import_notes(ev: &ImportEvent) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !ev.description.trim().is_empty() {
        parts.push(ev.description.trim().to_string());
    }
    if !ev.location.trim().is_empty() {
        parts.push(format!("Location: {}", ev.location.trim()));
    }
    if ev.all_day {
        parts.push(
            "Imported from an all-day calendar event — booked as a full local day because a \
             booking is a slot."
                .to_string(),
        );
    }
    if ev.recurring {
        parts.push(
            "The source event repeats (RRULE). This import does NOT expand recurrence: only this \
             single occurrence was created."
                .to_string(),
        );
    }
    parts.push("Imported from an .ics calendar file.".to_string());
    parts.join("\n")
}

/// Default slot length when the source event carried no usable `DTEND`/`DURATION` — matches the
/// portal's own one-hour default for a booking entered without an end time.
const DEFAULT_SLOT_SECS: u64 = 3600;

fn new_from_import(ev: &ImportEvent, uid: &str) -> Appointment {
    let ts = now();
    let mut a = Appointment {
        appointment_id: uuid::Uuid::new_v4().to_string(),
        // Deliberately UNASSIGNED: a calendar invite names an event, not a DogTag client.
        client_id: String::new(),
        pet_id: None,
        service: import_service(ev),
        start_at: ev.start_at,
        end_at: import_end(ev),
        status: "scheduled".to_string(),
        notes: import_notes(ev),
        groomer: String::new(),
        created_at: ts,
        updated_at: ts,
        client_name: String::new(),
        pet_name: String::new(),
        search_key: String::new(),
        source: Some("ics".to_string()),
        external_uid: Some(uid.to_string()),
    };
    a.rebuild_search_key();
    a
}

fn import_end(ev: &ImportEvent) -> u64 {
    if ev.end_at > ev.start_at {
        ev.end_at
    } else {
        ev.start_at + DEFAULT_SLOT_SECS
    }
}

/// Fold a re-imported event onto the booking it already created.
///
/// The calendar file owns WHEN (start, end) and the event's own text; the portal owns WHO and HOW
/// FAR ALONG (client, pet, groomer, workflow status). A cancellation in the source file is the one
/// status change the file is allowed to make.
fn merge_import(mut a: Appointment, ev: &ImportEvent, cancel: bool) -> Appointment {
    a.start_at = ev.start_at;
    a.end_at = import_end(ev);
    a.service = import_service(ev);
    a.notes = import_notes(ev);
    if cancel {
        a.status = "cancelled".to_string();
    }
    a.source = Some("ics".to_string());
    a.updated_at = now();
    a.rebuild_search_key();
    a
}

// ============================================================================================
// routers
// ============================================================================================

/// The UNAUTHENTICATED subscription feed. Split out because a calendar client cannot present a
/// bearer token: authorization is the secret in the path, and nothing else here may be public.
pub fn ics_feed_router() -> Router<AppState> {
    Router::new().route("/calendar/feed/:token", get(feed))
}

/// Operator-gated feed administration + `.ics` import.
pub fn ics_admin_router() -> Router<AppState> {
    Router::new()
        .route(
            "/calendar/feed",
            get(get_feed_settings).delete(revoke_feed),
        )
        .route("/calendar/feed/rotate", post(rotate_feed))
        .route("/calendar/import", post(import_events))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appt() -> Appointment {
        Appointment {
            appointment_id: "appt-1".into(),
            client_id: "cli-1".into(),
            pet_id: Some("pet-1".into()),
            service: "Full groom".into(),
            start_at: 1_772_020_800,
            end_at: 1_772_024_400,
            status: "scheduled".into(),
            notes: String::new(),
            groomer: "Sam".into(),
            created_at: 1,
            updated_at: 2,
            client_name: "Alice Tan".into(),
            pet_name: "Rex".into(),
            search_key: String::new(),
            source: None,
            external_uid: None,
        }
    }

    // ---- secret_eq ----------------------------------------------------------------------------

    #[test]
    fn secret_eq_accepts_only_an_exact_match() {
        assert!(secret_eq("abc123", "abc123"));
        assert!(!secret_eq("abc123", "abc124"));
        assert!(!secret_eq("abc123", "abc12"));
        assert!(!secret_eq("", "x"));
        assert!(secret_eq("", ""));
    }

    #[test]
    fn secret_eq_compares_every_byte_not_a_prefix() {
        // A prefix-only comparison would accept this; the difference is in the LAST byte.
        let a = "0".repeat(63) + "a";
        let b = "0".repeat(63) + "b";
        assert!(!secret_eq(&a, &b));
    }

    // ---- mint_token ---------------------------------------------------------------------------

    #[test]
    fn mint_token_is_64_hex_chars_and_never_repeats() {
        let a = mint_token();
        assert_eq!(a.len(), 64, "32 random bytes, hex");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, mint_token());
    }

    // ---- strip_ics_suffix ---------------------------------------------------------------------

    #[test]
    fn strip_ics_suffix_accepts_both_url_shapes() {
        assert_eq!(strip_ics_suffix("abc.ics"), "abc");
        assert_eq!(strip_ics_suffix("abc"), "abc");
        // only a TRAILING .ics is an extension
        assert_eq!(strip_ics_suffix("a.icsb"), "a.icsb");
    }

    // ---- event_uid ----------------------------------------------------------------------------

    #[test]
    fn event_uid_defaults_to_the_appointment_id_at_the_issuer_domain() {
        assert_eq!(event_uid(&appt(), "shop.example"), "appt-1@shop.example");
    }

    #[test]
    fn event_uid_reuses_the_source_uid_for_an_imported_booking() {
        // Round-trip identity: import then publish must not mint a SECOND identity for one booking.
        let mut a = appt();
        a.external_uid = Some("evt-9@google.com".into());
        assert_eq!(event_uid(&a, "shop.example"), "evt-9@google.com");
        // an empty string is not an identity
        a.external_uid = Some(String::new());
        assert_eq!(event_uid(&a, "shop.example"), "appt-1@shop.example");
    }

    // ---- to_ics_event -------------------------------------------------------------------------

    #[test]
    fn to_ics_event_summarizes_service_pet_and_client() {
        let e = to_ics_event(&appt(), "shop.example");
        assert_eq!(e.summary, "Full groom - Rex / Alice Tan");
        assert!(e.description.contains("Status: Scheduled"));
        assert!(e.description.contains("Groomer: Sam"));
        assert_eq!(e.status, IcsStatus::Confirmed);
        assert_eq!(e.last_modified, 2);
    }

    #[test]
    fn to_ics_event_falls_back_to_a_plain_label_when_there_is_no_service_or_client() {
        let mut a = appt();
        a.service = "  ".into();
        a.client_name = String::new();
        a.pet_name = String::new();
        a.groomer = String::new();
        let e = to_ics_event(&a, "shop.example");
        assert_eq!(e.summary, "Appointment");
        assert!(!e.description.contains("Groomer:"));
    }

    #[test]
    fn to_ics_event_flags_an_unassigned_imported_booking() {
        let mut a = appt();
        a.client_id = String::new();
        a.client_name = String::new();
        a.pet_name = String::new();
        let e = to_ics_event(&a, "shop.example");
        assert!(e.description.contains("Unassigned"));
    }

    #[test]
    fn to_ics_event_publishes_only_cancelled_as_cancelled() {
        let mut a = appt();
        a.status = "cancelled".into();
        assert_eq!(to_ics_event(&a, "d").status, IcsStatus::Cancelled);
        // a no-show still occupied the slot: it stays on the subscriber's calendar
        a.status = "no_show".into();
        let e = to_ics_event(&a, "d");
        assert_eq!(e.status, IcsStatus::Confirmed);
        assert!(e.description.contains("Status: No show"));
    }

    // ---- feed_json ----------------------------------------------------------------------------

    #[test]
    fn feed_json_reports_disabled_when_no_feed_was_ever_published() {
        let s = IssuerSettings::default();
        let v = feed_json(&s);
        assert_eq!(v["enabled"], false);
        assert!(v["token"].is_null());
        assert!(v["path"].is_null());
    }

    #[test]
    fn feed_json_treats_an_empty_token_as_no_feed() {
        let s = IssuerSettings {
            ics_feed_token: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(feed_json(&s)["enabled"], false);
    }

    #[test]
    fn feed_json_publishes_the_dot_ics_path_for_a_live_feed() {
        let s = IssuerSettings {
            ics_feed_token: Some("tok123".into()),
            ..Default::default()
        };
        let v = feed_json(&s);
        assert_eq!(v["enabled"], true);
        assert_eq!(v["token"], "tok123");
        assert_eq!(v["path"], "/calendar/feed/tok123.ics");
    }

    // ---- import projections -------------------------------------------------------------------

    fn import_ev() -> ImportEvent {
        ImportEvent {
            uid: "evt-1@example.com".into(),
            summary: "Bath & brush".into(),
            description: "bring the muzzle".into(),
            location: "Shop".into(),
            start_at: 1_772_020_800,
            end_at: 1_772_024_400,
            all_day: false,
            recurring: false,
            status: String::new(),
        }
    }

    #[test]
    fn new_from_import_creates_an_unassigned_booking_carrying_its_source_uid() {
        let a = new_from_import(&import_ev(), "evt-1@example.com");
        assert_eq!(a.client_id, "", "an import never invents a client");
        assert_eq!(a.client_name, "");
        assert_eq!(a.service, "Bath & brush");
        assert_eq!(a.start_at, 1_772_020_800);
        assert_eq!(a.end_at, 1_772_024_400);
        assert_eq!(a.status, "scheduled");
        assert_eq!(a.source.as_deref(), Some("ics"));
        assert_eq!(a.external_uid.as_deref(), Some("evt-1@example.com"));
        assert!(a.notes.contains("bring the muzzle"));
        assert!(a.notes.contains("Location: Shop"));
        assert!(a.search_key.contains("bath & brush"));
    }

    #[test]
    fn import_end_defaults_a_missing_or_inverted_end_to_one_hour() {
        let mut ev = import_ev();
        ev.end_at = 0;
        assert_eq!(import_end(&ev), ev.start_at + 3600);
        ev.end_at = ev.start_at;
        assert_eq!(import_end(&ev), ev.start_at + 3600);
        ev.end_at = ev.start_at - 1;
        assert_eq!(import_end(&ev), ev.start_at + 3600);
    }

    #[test]
    fn import_notes_state_the_recurrence_and_all_day_compromises_on_the_booking_itself() {
        let mut ev = import_ev();
        ev.recurring = true;
        ev.all_day = true;
        let n = import_notes(&ev);
        assert!(n.contains("does NOT expand recurrence"));
        assert!(n.contains("all-day"));
        assert!(n.contains("Imported from an .ics calendar file."));
    }

    #[test]
    fn import_service_never_leaves_a_booking_unlabelled() {
        let mut ev = import_ev();
        ev.summary = "   ".into();
        assert_eq!(import_service(&ev), "Imported event");
    }

    #[test]
    fn merge_import_moves_the_slot_but_keeps_the_shop_s_own_work() {
        let mut existing = appt();
        existing.external_uid = Some("evt-1@example.com".into());
        existing.status = "completed".into();
        let mut ev = import_ev();
        ev.start_at += 7200;
        ev.end_at += 7200;

        let merged = merge_import(existing.clone(), &ev, false);
        // the file owns WHEN
        assert_eq!(merged.start_at, ev.start_at);
        assert_eq!(merged.end_at, ev.end_at);
        assert_eq!(merged.service, "Bath & brush");
        // the portal owns WHO and HOW FAR ALONG
        assert_eq!(merged.client_id, "cli-1");
        assert_eq!(merged.client_name, "Alice Tan");
        assert_eq!(merged.pet_id.as_deref(), Some("pet-1"));
        assert_eq!(merged.groomer, "Sam");
        assert_eq!(merged.status, "completed");
        assert_eq!(merged.appointment_id, "appt-1", "identity survives a re-import");
    }

    #[test]
    fn merge_import_honours_a_cancellation_from_the_source_file() {
        let mut existing = appt();
        existing.external_uid = Some("evt-1@example.com".into());
        let merged = merge_import(existing, &import_ev(), true);
        assert_eq!(merged.status, "cancelled");
    }

    // ---- truncation notice --------------------------------------------------------------------

    #[test]
    fn truncation_notice_says_the_feed_is_incomplete() {
        let e = truncation_notice(1_772_020_800, "shop.example");
        assert!(e.summary.contains("truncated"));
        assert!(e.description.contains("NOT in this calendar"));
        assert!(e.end > e.start);
    }
}
