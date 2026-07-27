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
//! constant time, revocable in one click. Anyone holding the URL can read the shop's entire
//! schedule, which the UI says in as many words.
//!
//! WHAT A PATH-BORNE SECRET COSTS, stated exactly rather than glossed. THIS APPLICATION never logs
//! the token — it appears in no `tracing` call, and the router carries no request-logging layer — but
//! that is the whole of what this code can promise. The secret is a URL PATH SEGMENT, and the request
//! URI is precisely what a reverse proxy, CDN or tunnel records in its access log BY DEFAULT, so on a
//! deployed stack every subscriber poll writes the credential to those logs in plaintext, on the
//! provider's own polling schedule, indefinitely. A header- or query-borne credential would not be
//! exposed that way; a calendar client can present neither. This is inherent to the design and is the
//! residual risk it accepts. Two mitigations, both real, neither of which closes it. First, the
//! shipped reverse proxies suppress it: BOTH `stacks/groomer/web/nginx.conf` and
//! `stacks/vet/web/nginx.conf` set `access_log off` for a feed READ — both, because
//! [`ics_feed_router`] is merged for every role, so a vet stack serves a feed too. That is the limit
//! of this repo's reach: a CDN, tunnel or load balancer in FRONT of them logs the full URI by default
//! and is configured somewhere else entirely. Second, ROTATE — one click mints a new secret and kills
//! the old URL. That is why rotation is a first-class action here rather than a recovery procedure:
//! it is the only mitigation that still works when the exposure is a log this code does not own.
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

/// Hard cap on events accepted in ONE import.
///
/// The parsed calendar arrives as a single JSON body, so the real ceiling is axum's 2 MB default body
/// limit — nothing in this service raises it — and that limit rejects the request before any handler
/// runs, as a bare size error that mentions neither calendars nor what to do next. Refusing at a
/// stated COUNT well under it lets the portal tell the operator what happened in words. A 244-event
/// Google export is an ordinary size, so this is a backstop rather than a working limit; a file whose
/// events carry unusually long `DESCRIPTION`s can still reach the body limit first, which this
/// narrows but cannot close.
const MAX_IMPORT_EVENTS: usize = 1_000;

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
    /// Unix seconds — OPTIONAL and SIGNED, and both of those are load-bearing.
    ///
    /// A `DTSTART` before 1970 is an ordinary thing to find in an exported calendar (a birthday, or a
    /// malformed year that a browser reads as 1901) and resolves to a NEGATIVE instant; an
    /// unresolvable value can serialize as `null`. Against a bare `u64` serde rejects either one, and
    /// it rejects the WHOLE BODY — so a single unusable event would 422 the request before this
    /// handler ran, aborting the import of every other event in the file. Accepted here, then skipped
    /// individually in [`import_events`], which is this module's stated posture: report an unusable
    /// event, never guess at it, and never let it take the file down with it.
    #[serde(default)]
    start_at: Option<i64>,
    #[serde(default)]
    end_at: Option<i64>,
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

impl ImportEvent {
    /// The start this event books at, or `None` when the source gave one no booking can be made at:
    /// absent, `null`, or at/before the epoch. Never clamped to a plausible-looking value — a booking
    /// invented at the wrong time is worse than one the operator is told about.
    fn start(&self) -> Option<u64> {
        match self.start_at {
            Some(s) if s > 0 => Some(s as u64),
            _ => None,
        }
    }
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
/// overwrite the shop's own work on that booking — the linked client, pet, groomer, a status the
/// operator has since advanced, and anything they TYPED IN THE NOTES all survive — because the
/// calendar file is the source of truth for WHEN, and the portal is the source of truth for WHO. The
/// exact notes rule is in [`merge_notes`], because "notes" is the one field both sides write.
///
/// A single unusable event never takes the file down with it: it is counted in `skipped` and the rest
/// import normally. Beyond [`MAX_IMPORT_EVENTS`] the whole request is refused, in words, rather than
/// left to fail as a body-size error that names nothing.
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
    if body.events.len() > MAX_IMPORT_EVENTS {
        return err(
            StatusCode::BAD_REQUEST,
            &format!(
                "this calendar has {} events and an import accepts at most {MAX_IMPORT_EVENTS} at a \
                 time. Export a narrower date range from the other calendar and import it in parts.",
                body.events.len()
            ),
        );
    }

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
        // No identity, or no instant a booking can be made at. Skipped INDIVIDUALLY — see the note on
        // `ImportEvent::start_at` for why one such event must not be allowed to reject the body.
        let start = match ev.start() {
            Some(s) if !uid.is_empty() => s,
            _ => {
                skipped += 1;
                continue;
            }
        };
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
                    st.store.put_appointment(new_from_import(ev, uid, start)).await;
                }
                created += 1;
            }
            (Some(a), cancel) => {
                if !dry_run {
                    st.store.put_appointment(merge_import(a, ev, cancel, start)).await;
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

/// The LAST line of every notes block an import writes, and therefore the boundary between what an
/// import owns and what the operator has typed since. Load-bearing: [`operator_notes`] finds it.
const IMPORT_MARKER: &str = "Imported from an .ics calendar file.";
const ALL_DAY_NOTE: &str =
    "Imported from an all-day calendar event — booked as a full local day because a booking is a \
     slot.";
const RECURRING_NOTE: &str =
    "The source event repeats (RRULE). This import does NOT expand recurrence: only this single \
     occurrence was created.";
/// Prefix of the one import-written line whose text is not fixed.
const LOCATION_PREFIX: &str = "Location: ";

/// The notes block, carrying the source event's own text plus every caveat this import applied — so
/// the operator reads the compromise on the booking itself, not only in a summary they dismissed.
fn import_notes(ev: &ImportEvent) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !ev.description.trim().is_empty() {
        parts.push(ev.description.trim().to_string());
    }
    if !ev.location.trim().is_empty() {
        parts.push(format!("{LOCATION_PREFIX}{}", ev.location.trim()));
    }
    if ev.all_day {
        parts.push(ALL_DAY_NOTE.to_string());
    }
    if ev.recurring {
        parts.push(RECURRING_NOTE.to_string());
    }
    parts.push(IMPORT_MARKER.to_string());
    parts.join("\n")
}

/// Is this line one a previous import wrote itself, rather than something the operator typed?
///
/// Everything except the source description has a fixed shape, so identification is exact for those.
/// The description is arbitrary file text and can only be matched against the CURRENT event's — a
/// deliberate asymmetry, see [`operator_notes`].
fn is_import_line(line: &str, ev: &ImportEvent) -> bool {
    let l = line.trim();
    l.is_empty()
        || l == IMPORT_MARKER
        || l == ALL_DAY_NOTE
        || l == RECURRING_NOTE
        || l.starts_with(LOCATION_PREFIX)
        || ev.description.trim().lines().any(|d| d.trim() == l)
}

/// Everything on this booking's notes that the OPERATOR is responsible for.
///
/// [`import_notes`] always writes its block FIRST and always ends it with [`IMPORT_MARKER`], so on a
/// booking an import created, that marker is the end of the block a previous import wrote. Notes with
/// no marker at all mean the operator replaced them wholesale: every line is theirs, kept verbatim.
///
/// Within the leading block, a line is dropped only when it can be POSITIVELY identified as
/// import-written — an operator who typed ABOVE the imported text keeps that line too, which is the
/// whole point. Two asymmetries that identification cannot resolve, both erring toward KEEPING text:
/// when the source file's DESCRIPTION has changed since the last import, its previous text is no
/// longer identifiable and survives as if the operator had written it (stale and visible, rather than
/// deleted); and conversely a `Location: ` line is treated as import-written by CONVENTION rather
/// than by fact, so an operator opening a line that way inside the block does lose it. Safe direction
/// first, for a field the shop types into.
fn operator_notes(existing: &str, ev: &ImportEvent) -> String {
    let lines: Vec<&str> = existing.lines().collect();
    let marker = lines.iter().position(|l| l.trim() == IMPORT_MARKER);
    let Some(end) = marker else {
        return existing.trim().to_string();
    };
    let mut kept: Vec<&str> = lines[..end]
        .iter()
        .copied()
        .filter(|l| !is_import_line(l, ev))
        .collect();
    // Past the marker is the operator's alone; it is never filtered, because a line of theirs that
    // happens to read like a stamp is still theirs.
    kept.extend(lines[end + 1..].iter().copied());
    kept.join("\n").trim().to_string()
}

/// Re-stamp the import's own block while keeping whatever the operator typed.
///
/// `notes` is the one field BOTH sides write: the file carries the source event's description,
/// location and this import's caveats, and the operator edits the same box through
/// `PUT /appointments/:id`. Rebuilding it wholesale from the file — which is what a re-import used to
/// do — silently destroyed the shop's own text on every refresh of an already-imported calendar.
fn merge_notes(existing: &str, ev: &ImportEvent) -> String {
    let kept = operator_notes(existing, ev);
    let block = import_notes(ev);
    if kept.is_empty() {
        block
    } else {
        format!("{block}\n{kept}")
    }
}

/// Default slot length when the source event carried no usable `DTEND`/`DURATION` — matches the
/// portal's own one-hour default for a booking entered without an end time.
const DEFAULT_SLOT_SECS: u64 = 3600;

fn new_from_import(ev: &ImportEvent, uid: &str, start: u64) -> Appointment {
    let ts = now();
    let mut a = Appointment {
        appointment_id: uuid::Uuid::new_v4().to_string(),
        // Deliberately UNASSIGNED: a calendar invite names an event, not a DogTag client.
        client_id: String::new(),
        pet_id: None,
        service: import_service(ev),
        start_at: start,
        end_at: import_end(ev, start),
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

/// The slot's end, given the start that was already validated as storable. An end that is absent,
/// `null`, at/before the start, or unrepresentable falls back to the portal's own default length.
fn import_end(ev: &ImportEvent, start: u64) -> u64 {
    match ev.end_at {
        Some(e) if e > 0 && (e as u64) > start => e as u64,
        _ => start + DEFAULT_SLOT_SECS,
    }
}

/// Fold a re-imported event onto the booking it already created.
///
/// The calendar file owns WHEN (start, end) and the event's own text; the portal owns WHO and HOW
/// FAR ALONG (client, pet, groomer, workflow status) — and, per [`merge_notes`], everything the
/// operator typed into the notes. A cancellation in the source file is the one status change the file
/// is allowed to make.
fn merge_import(mut a: Appointment, ev: &ImportEvent, cancel: bool, start: u64) -> Appointment {
    a.start_at = start;
    a.end_at = import_end(ev, start);
    a.service = import_service(ev);
    a.notes = merge_notes(&a.notes, ev);
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
            start_at: Some(1_772_020_800),
            end_at: Some(1_772_024_400),
            all_day: false,
            recurring: false,
            status: String::new(),
        }
    }

    #[test]
    fn new_from_import_creates_an_unassigned_booking_carrying_its_source_uid() {
        let a = new_from_import(&import_ev(), "evt-1@example.com", 1_772_020_800);
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
        let start = 1_772_020_800u64;
        let mut ev = import_ev();
        ev.end_at = None;
        assert_eq!(import_end(&ev, start), start + 3600);
        ev.end_at = Some(0);
        assert_eq!(import_end(&ev, start), start + 3600);
        ev.end_at = Some(start as i64);
        assert_eq!(import_end(&ev, start), start + 3600);
        ev.end_at = Some(start as i64 - 1);
        assert_eq!(import_end(&ev, start), start + 3600);
        // A pre-epoch end is nonsense, not a slot that ends before it began.
        ev.end_at = Some(-10);
        assert_eq!(import_end(&ev, start), start + 3600);
    }

    // ---- unstorable instants (one bad event must never abort the file) ------------------------

    #[test]
    fn start_rejects_every_instant_a_booking_cannot_be_made_at() {
        let mut ev = import_ev();
        assert_eq!(ev.start(), Some(1_772_020_800));
        // A pre-1970 DTSTART: real in exported calendars, and NEGATIVE once resolved.
        ev.start_at = Some(-2_145_916_800);
        assert_eq!(ev.start(), None);
        ev.start_at = Some(0);
        assert_eq!(ev.start(), None);
        // `null` on the wire — what a browser sends for an instant that resolved to NaN.
        ev.start_at = None;
        assert_eq!(ev.start(), None);
    }

    #[test]
    fn a_negative_or_null_start_deserializes_instead_of_rejecting_the_whole_body() {
        // The point of the Option<i64>: serde must ACCEPT these so the handler can skip them one by
        // one. Against a bare u64 this parse fails, and axum answers 422 for the entire upload.
        let body: ImportBody = serde_json::from_str(
            r#"{"events":[
                {"uid":"a","startAt":-2145916800},
                {"uid":"b","startAt":null},
                {"uid":"c","startAt":1772020800}
            ]}"#,
        )
        .expect("one unstorable event must not reject the file");
        assert_eq!(body.events.len(), 3);
        assert_eq!(body.events[0].start(), None);
        assert_eq!(body.events[1].start(), None);
        assert_eq!(body.events[2].start(), Some(1_772_020_800));
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
        let moved = 1_772_020_800 + 7200;
        ev.start_at = Some(moved);
        ev.end_at = Some(moved + 3600);

        let merged = merge_import(existing.clone(), &ev, false, moved as u64);
        // the file owns WHEN
        assert_eq!(merged.start_at, moved as u64);
        assert_eq!(merged.end_at, moved as u64 + 3600);
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
        let merged = merge_import(existing, &import_ev(), true, 1_772_020_800);
        assert_eq!(merged.status, "cancelled");
    }

    // ---- notes: the one field both the file and the operator write ----------------------------

    #[test]
    fn a_reimport_restamps_its_own_block_without_duplicating_it() {
        let ev = import_ev();
        let first = import_notes(&ev);
        let second = merge_notes(&first, &ev);
        assert_eq!(second, first, "an unchanged re-import must be a no-op on the notes");
    }

    #[test]
    fn a_reimport_keeps_what_the_operator_typed_below_the_imported_block() {
        let ev = import_ev();
        let edited = format!("{}\nOwner says the dog bites. Muzzle on arrival.", import_notes(&ev));
        let merged = merge_notes(&edited, &ev);
        assert!(merged.contains("Owner says the dog bites. Muzzle on arrival."));
        assert!(merged.contains(IMPORT_MARKER));
        assert_eq!(merged.matches(IMPORT_MARKER).count(), 1);
    }

    #[test]
    fn a_reimport_keeps_what_the_operator_typed_above_the_imported_block() {
        // A prefilled textarea invites typing at the top as readily as at the bottom, so the leading
        // block is filtered line by line rather than dropped wholesale.
        let ev = import_ev();
        let edited = format!("CALL THE OWNER FIRST\n{}", import_notes(&ev));
        let merged = merge_notes(&edited, &ev);
        assert!(merged.contains("CALL THE OWNER FIRST"));
        assert_eq!(merged.matches("bring the muzzle").count(), 1);
        assert_eq!(merged.matches("Location: Shop").count(), 1);
    }

    #[test]
    fn a_reimport_keeps_notes_the_operator_replaced_wholesale() {
        // No marker anywhere: none of this came from an import, so none of it may be dropped.
        let merged = merge_notes("Rebooked by phone. Nervous dog.", &import_ev());
        assert!(merged.contains("Rebooked by phone. Nervous dog."));
        assert!(merged.contains(IMPORT_MARKER));
    }

    #[test]
    fn a_reimport_drops_the_previous_caveats_when_the_file_no_longer_carries_them() {
        let mut ev = import_ev();
        ev.recurring = true;
        ev.all_day = true;
        let first = import_notes(&ev);
        ev.recurring = false;
        ev.all_day = false;
        let merged = merge_notes(&first, &ev);
        assert!(!merged.contains("does NOT expand recurrence"));
        assert!(!merged.contains("all-day"));
        assert!(merged.contains(IMPORT_MARKER));
    }

    #[test]
    fn a_changed_description_is_preserved_rather_than_deleted() {
        // The only line an import cannot positively identify on a re-import. Erring toward keeping it
        // leaves a stale line the operator can see and delete; erring the other way silently eats
        // text they may have written themselves.
        let mut ev = import_ev();
        let first = import_notes(&ev);
        ev.description = "bring the harness".into();
        let merged = merge_notes(&first, &ev);
        assert!(merged.contains("bring the harness"), "the file's current text");
        assert!(merged.contains("bring the muzzle"), "never silently deleted");
    }

    #[test]
    fn merge_import_keeps_the_operator_s_notes() {
        let mut existing = appt();
        existing.external_uid = Some("evt-1@example.com".into());
        existing.notes = format!("{}\nAllergic to the oatmeal shampoo.", import_notes(&import_ev()));
        let merged = merge_import(existing, &import_ev(), false, 1_772_020_800);
        assert!(merged.notes.contains("Allergic to the oatmeal shampoo."));
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
