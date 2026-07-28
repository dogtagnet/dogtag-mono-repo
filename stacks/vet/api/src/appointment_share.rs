//! The CLIENT half of the calendar: one appointment, handed to the person it belongs to.
//!
//! The shop side (the published `.ics` subscription feed and `.ics` import) lives in
//! `calendar_ics.rs` and is about the SHOP's whole schedule. This module is the opposite shape: ONE
//! booking, handed to ONE client, who scans a QR at the counter (or follows a link) and walks away
//! with it in their own calendar — Apple, Google, or whatever their phone offers.
//!
//! THREE WAYS OUT, one source. `GET /a/{token}` is a self-contained page a phone can render;
//! `GET /a/{token}.ics` is the same booking as a file iOS and Android open natively into "add to
//! calendar"; and the page also offers a Google Calendar template link for a client who lives in a
//! browser. All three are projections of ONE [`to_client_event`] through the ONE serializer in
//! `ics.rs` that the shop feed already uses — there is deliberately no second `.ics` writer to drift
//! out of step with the first.
//!
//! WHAT THE HANDOFF CARRIES, and what it does not. The token names exactly one booking and the page
//! renders exactly that booking: the service, the slot, the shop, the pet, and the groomer. It does
//! NOT carry the client's own name (they know it, and a leaked link should not name them), the
//! shop's internal `notes` (free text the shop writes for itself, which can say anything about
//! anyone), any internal id beyond the booking's own uuid, or — the point — any other client's
//! booking. [`to_client_event`] is a separate projection from the feed's `to_ics_event` for exactly
//! this reason: the feed is read by the SHOP and carries the operator's notes and the client's name,
//! and reusing it here would have handed both to whoever scanned the QR.
//!
//! THE URL MUST BE ONE A PHONE CAN REACH. A QR is worthless if it encodes a host the scanning phone
//! cannot resolve, and worse than worthless if it still LOOKS like a working link — that exact defect
//! shipped here once already, when receipt QRs were built from a `did:web` issuer NAME whose default
//! was RFC-2606 reserved. So the URL is built from `DEPLOYMENT_URL` and nothing else, and
//! [`client_base`] refuses two cases outright: an unset base, and a LOOPBACK base, which is the
//! shipped dev default and is reachable from the shop's own laptop and from nowhere else. In both
//! cases the mint returns NO QR plus a reason naming the fix. The link itself is still returned for
//! the loopback case, labelled — it works on the operator's own machine, which is exactly where a
//! demo needs it.
//!
//! WHAT A DOWNLOADED `.ics` CANNOT DO, stated here and on the page rather than discovered later. A
//! file import is a SNAPSHOT. Nothing pushes a later change into a calendar the client has already
//! imported it into; iCalendar has no such channel and neither does this service. Three things are
//! done about it, none of which is a claim that the copy stays fresh:
//!
//!   * every event carries a STABLE `UID` and a monotonic `SEQUENCE`, so RE-opening the link and
//!     re-adding supersedes the client's earlier copy in place instead of duplicating it;
//!   * every event carries `URL:` back to this page, so a stale entry in the client's calendar still
//!     names the surface that states the current answer;
//!   * the same `.ics` URL is LIVE — it re-reads the booking on every request — so a client whose
//!     calendar can subscribe (`webcal://`) gets real updates, on their calendar's own refresh
//!     schedule, which for Google has historically been hours rather than minutes. The page offers
//!     that as a distinct, labelled choice rather than implying the download behaves that way.
//!
//! A cancelled booking publishes `STATUS:CANCELLED`, which is what makes re-adding it REMOVE the
//! event; the page leads with the cancellation instead of offering a bare "add to calendar". A
//! DELETED booking is published the same way, as a tombstone, using the slot recorded when the token
//! was minted — a 404 there would leave a subscriber's stale event standing forever with nothing but
//! a sync error to explain it.
//!
//! SHARING IS ADDITIVE AND CANNOT BE TAKEN BACK, which is a deliberate difference from the shop
//! feed and is stated here because nothing else says it. Every mint issues a NEW token — the portal
//! dialog mints on each open, so an operator who opens it three times has three live URLs for one
//! booking — and there is no revoke: each link resolves for [`SHARE_TTL_AFTER_END`] past the slot.
//! The feed makes rotation a first-class action because its URL exposes the shop's ENTIRE schedule;
//! this one names a single appointment and carries neither the client's name nor the shop's notes,
//! so the blast radius of a leaked line is one booking, and the cost of getting revocation wrong —
//! breaking a link a client is relying on to find their appointment — is the larger risk. If that
//! trade stops holding (links shared onward, a shop wanting to un-share), revocation belongs here
//! as an explicit action, not as a shorter expiry that would silently strand working links.
//!
//! AND WHEN THE STORE CANNOT BE REACHED, that is its own answer. It is not a cancellation and it is
//! not a confirmation: the page says it could not check, in those words, and offers no
//! add-to-calendar affordance it cannot stand behind. This is why the module reads through
//! [`Store::try_get_appointment`] rather than the `Option`-shaped form, whose collapsed error would
//! have told a client their booking was gone on the strength of a read that never happened.
//!
//! The MINT's write is held to the same rule, and it is the one that fails furthest from where it is
//! noticed: [`Store::put_appointment_share`] is fallible — alone among the `put_*` methods — because
//! a dropped write still lets the mint answer 200 with a token and a QR that were never recorded.
//! The operator then hands a client a link whose scan says "this link is not one we recognise, ask
//! the shop for a new one" about a booking that is perfectly live, which is the same false claim the
//! fallible reads exist to prevent, arriving through the write instead. So it refuses with a 503 and
//! no token.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::auth::now;
use crate::ics::{self, IcsEvent, IcsStatus};
use crate::store::{Appointment, AppointmentShare, StoreReadError};

type Resp = (StatusCode, Json<Value>);
fn ok(v: Value) -> Resp {
    (StatusCode::OK, Json(v))
}
fn err(code: StatusCode, msg: &str) -> Resp {
    (code, Json(json!({ "error": msg })))
}

/// How long a handoff link outlives the booking it names.
///
/// Long, on purpose. The client's calendar may re-poll a `webcal://` subscription for as long as the
/// event sits in it, and a client who wants to re-check "is this still on?" the week after should
/// not meet a dead link. This is not a secret with a blast radius that shrinks by expiring fast: it
/// names one appointment and nothing else.
const SHARE_TTL_AFTER_END: u64 = 180 * 86_400;

/// Token width: 16 CSPRNG bytes -> 32 hex characters, matching the record-share and bind QRs. Low
/// density, so a phone camera focuses on it at counter distance and in shop lighting.
const SHARE_TOKEN_BYTES: usize = 16;

// ============================================================================================
// reachability — a QR that names a host the client's phone cannot reach is worse than no QR
// ============================================================================================

/// Why a handoff link cannot be turned into a QR, or [`BaseUsability::Usable`] when it can.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BaseUsability {
    /// A base a phone off the shop's own machine can plausibly resolve. Carries it, trailing
    /// slashes trimmed.
    Usable(String),
    /// `DEPLOYMENT_URL` is unset. There is no link at all to offer, dead or otherwise.
    Unset,
    /// `DEPLOYMENT_URL` names a LOOPBACK host. The link works on the machine serving it and nowhere
    /// else — in particular, not on the client's phone, which is the only device that matters here.
    /// Carries the link anyway, because on a dev box it is the working one.
    Loopback(String),
}

/// Classify `DEPLOYMENT_URL` for use as the base of a QR a CLIENT'S PHONE will scan.
///
/// Two rejections, both learned rather than theoretical:
///
/// * UNSET — nothing to build from. Falling back to the request's `Host` header would let any
///   caller choose what the QR points at, and falling back to `ISSUER_DOMAIN` is the precise defect
///   that shipped in the receipt QR: that is a `did:web` IDENTITY, which need not resolve and whose
///   shipped default is RFC-2606 reserved.
/// * LOOPBACK — `localhost`, `127.0.0.0/8`, `::1` or a `.localhost` name. This is the DEFAULT
///   (`main.rs` falls back to `http://localhost:{port}`), so it is the case a real shop hits first,
///   and it is the one that most convincingly masquerades as working: the operator scans it with
///   their own laptop's browser, it resolves, and the QR looks proven. On the client's phone
///   `localhost` is the PHONE, and the scan fails or — worse — lands on something else entirely.
///
/// Anything else is taken at face value. Whether a LAN address or a tunnel hostname actually routes
/// from where the client is standing is not knowable from inside this process, and pretending to
/// check would be its own false claim.
pub fn client_base(deployment_url: &str) -> BaseUsability {
    let base = deployment_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return BaseUsability::Unset;
    }
    if is_loopback(base) {
        return BaseUsability::Loopback(base.to_string());
    }
    BaseUsability::Usable(base.to_string())
}

/// Does this URL's HOST resolve only on the machine that serves it?
///
/// Parsed with `url`, which is already a dependency, so the host is read as a host and not by string
/// matching — `http://localhost.evil.example/` is not loopback and `http://[::1]:8080/` is, and only
/// a real parse gets both right. A URL that does not parse has no host this can vouch for, so it is
/// NOT called loopback; it falls through to `Usable` and fails visibly as a bad link rather than
/// being silently suppressed as a dev address.
fn is_loopback(base: &str) -> bool {
    let Ok(parsed) = url::Url::parse(base) else {
        return false;
    };
    match parsed.host() {
        Some(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d == "localhost" || d.ends_with(".localhost")
        }
        // 127.0.0.0/8 is loopback in its entirety, not just 127.0.0.1.
        Some(url::Host::Ipv4(v4)) => v4.is_loopback() || v4.is_unspecified(),
        Some(url::Host::Ipv6(v6)) => v6.is_loopback() || v6.is_unspecified(),
        None => false,
    }
}

/// The operator-facing sentence explaining why no QR was drawn, naming the fix. `None` when a QR was
/// drawn.
fn qr_unavailable_reason(u: &BaseUsability) -> Option<String> {
    match u {
        BaseUsability::Usable(_) => None,
        BaseUsability::Unset => Some(
            "This deployment has no DEPLOYMENT_URL set, so there is no address to point a QR at. \
             Set DEPLOYMENT_URL to the host a client's phone can reach — the shop's LAN address or \
             its public tunnel — and share this booking again."
                .to_string(),
        ),
        BaseUsability::Loopback(base) => Some(format!(
            "DEPLOYMENT_URL is {base}, which is a loopback address: it resolves only on the machine \
             running this server. A client's phone would not reach it, so no QR is shown — scanning \
             one would fail, or land somewhere else entirely. The link below works on THIS machine, \
             for testing. Set DEPLOYMENT_URL to the shop's LAN address or public tunnel to hand this \
             to a client."
        )),
    }
}

// ============================================================================================
// mint (operator-gated)
// ============================================================================================

fn mint_token() -> String {
    let mut bytes = [0u8; SHARE_TOKEN_BYTES];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    hex::encode(bytes)
}

/// `POST /appointments/{id}/share` — mint the client handoff for ONE booking.
///
/// Returns the paths unconditionally and the absolute `qrUrl` only when the base is one a client's
/// phone could reach; see [`client_base`]. The portal draws a QR if and only if `qrUrl` is present,
/// and shows `qrUnavailableReason` when it is not.
async fn mint_share(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    let appt = match st.store.try_get_appointment(&id).await {
        Ok(Some(a)) => a,
        Ok(None) => return err(StatusCode::NOT_FOUND, "appointment not found"),
        Err(e) => {
            tracing::warn!(error = %e, "appointment share: store read failed");
            return err(
                StatusCode::SERVICE_UNAVAILABLE,
                "could not read this appointment right now, so no share link was minted — try again",
            );
        }
    };

    let token = mint_token();
    let ts = now();
    // The link outlives the booking, and an already-past booking still gets a usable window rather
    // than a link that is born expired. Saturating, because nothing upstream bounds how far out a
    // slot may sit — `validate_slot` only rejects an end at or before the start — and wrapping here
    // would produce a tiny `expires_at`, i.e. exactly the link born expired this line exists to
    // avoid, silently and only for the far-future booking that triggered it.
    let expires_at = appt
        .end_at
        .max(appt.start_at)
        .max(ts)
        .saturating_add(SHARE_TTL_AFTER_END);
    // A dropped write must never become a 200. Handing back a token whose scan resolves to nothing
    // tells the client their link is unrecognised — a false claim about a live booking, and the one
    // this surface exists to eliminate. Refusing costs the operator a retry; the alternative costs a
    // client their appointment.
    if let Err(e) = st
        .store
        .put_appointment_share(
            &token,
            AppointmentShare {
                appointment_id: appt.appointment_id.clone(),
                start_at: appt.start_at,
                expires_at,
            },
        )
        .await
    {
        tracing::warn!(error = %e, "appointment share: store write failed");
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "could not create a share link for this appointment right now — nothing was shared, try again",
        );
    }

    let usability = client_base(&st.cfg.deployment_url);
    let path = format!("/a/{token}");
    let (qr_url, link) = match &usability {
        BaseUsability::Usable(base) => (Some(format!("{base}{path}")), Some(format!("{base}{path}"))),
        // No QR — but the link is the working one on this machine, so a dev/demo run still has
        // something to open. It is labelled by `qrUnavailableReason`.
        BaseUsability::Loopback(base) => (None, Some(format!("{base}{path}"))),
        BaseUsability::Unset => (None, None),
    };

    ok(json!({
        "appointmentId": appt.appointment_id,
        "token": token,
        "path": path,
        "icsPath": format!("{path}.ics"),
        // Present only when a client's phone could reach it. The portal keys the QR off THIS field.
        "qrUrl": qr_url,
        "url": link,
        "qrUnavailableReason": qr_unavailable_reason(&usability),
        "expiresAt": expires_at,
    }))
}

// ============================================================================================
// the client-facing projection
// ============================================================================================

/// The `UID` a booking publishes to its CLIENT under.
///
/// Always `{appointment_id}@{domain}`, deliberately NOT the feed's `event_uid` (which prefers an
/// imported booking's originating `external_uid`). Two reasons. The client's calendar is a different
/// calendar from the shop's, so borrowing the source UID buys no dedup. And this id must stay
/// derivable from the TOKEN alone, because a DELETED booking has to be publishable as a tombstone
/// under the same UID the client already holds — at which point there is no row left to read an
/// `external_uid` off.
pub fn client_uid(appointment_id: &str, domain: &str) -> String {
    format!("{appointment_id}@{domain}")
}

/// Revision of this booking, for `SEQUENCE`.
///
/// Seconds the booking has existed as of its last edit: monotonically non-decreasing across edits
/// (RFC 5545 §3.8.7.4's actual requirement), `0` for one never edited, and SMALL — which matters,
/// because iCalendar's INTEGER is a signed 32-bit value that a raw unix timestamp overflows in 2038.
/// Clamped rather than wrapped, so the worst case is a revision that stops advancing instead of one
/// that goes backwards and un-supersedes an update the client already has.
///
/// GRANULARITY, stated rather than left to be discovered: this is derived from `updated_at`, which
/// is unix SECONDS, so two edits inside the same second yield the same revision. That is a property
/// of the stored timestamp and not of this function — `LAST-MODIFIED` and `DTSTAMP` are second-
/// granular for the same reason, so no per-event revision could be finer without the row carrying a
/// counter of its own. A shop moving a booking does so seconds-to-days after making it, so the
/// practical case advances; a sub-second double edit publishes the later slot under the earlier
/// revision, which a client may treat as a duplicate rather than an update.
pub fn client_sequence(a: &Appointment) -> u32 {
    a.updated_at
        .saturating_sub(a.created_at)
        .min(i32::MAX as u64) as u32
}

/// Client-facing label for a booking's workflow status.
fn status_label(status: &str) -> &str {
    match status {
        "scheduled" => "Scheduled",
        "confirmed" => "Confirmed",
        "in_progress" => "In progress",
        "completed" => "Completed",
        "cancelled" => "Cancelled",
        "no_show" => "Missed",
        other => other,
    }
}

fn service_label(a: &Appointment) -> String {
    let s = a.service.trim();
    if s.is_empty() {
        "Appointment".to_string()
    } else {
        s.to_string()
    }
}

/// What the client's calendar entry is called: the service, and their pet if the booking names one.
///
/// The CLIENT's own name is deliberately absent — it is their calendar, they know who they are, and
/// a link that leaks names the person it leaked about.
fn client_summary(a: &Appointment) -> String {
    let service = service_label(a);
    let pet = a.pet_name.trim();
    if pet.is_empty() {
        service
    } else {
        format!("{service} - {pet}")
    }
}

/// Project a booking into the event the CLIENT receives.
///
/// Separate from the feed's `to_ics_event` on purpose, and the difference IS the privacy boundary:
/// the feed carries `notes` (arbitrary operator text) and `client_name`, both of which belong to the
/// shop's own view of its book. Nothing here reads either field.
///
/// `page_url` is the handoff page, or empty when this deployment has no base to name one with — in
/// which case no `URL:` is published rather than a fabricated one.
pub fn to_client_event(
    a: &Appointment,
    domain: &str,
    shop_name: &str,
    page_url: &str,
) -> IcsEvent {
    let cancelled = a.status == "cancelled";
    let mut desc: Vec<String> = Vec::new();
    if !shop_name.trim().is_empty() {
        desc.push(format!("With {}.", shop_name.trim()));
    }
    desc.push(format!("Status: {}", status_label(&a.status)));
    if !a.groomer.trim().is_empty() {
        desc.push(format!("Your groomer: {}", a.groomer.trim()));
    }
    if cancelled {
        desc.push(
            "This appointment was CANCELLED by the shop. Adding this file to your calendar removes \
             it."
                .to_string(),
        );
    }
    // The limitation, on the calendar entry itself — not only on a page the client visited once.
    desc.push(
        "This is a copy of the booking as it stood when you added it. If the shop changes or \
         cancels it, your calendar will not update on its own."
            .to_string(),
    );
    if !page_url.is_empty() {
        desc.push(format!("Check the current details: {page_url}"));
    }

    IcsEvent {
        uid: client_uid(&a.appointment_id, domain),
        start: a.start_at,
        end: a.end_at,
        summary: client_summary(a),
        description: desc.join("\n"),
        status: if cancelled {
            IcsStatus::Cancelled
        } else {
            IcsStatus::Confirmed
        },
        last_modified: a.updated_at,
        url: page_url.to_string(),
        location: shop_name.trim().to_string(),
        sequence: client_sequence(a),
    }
}

/// The tombstone for a booking the shop has DELETED.
///
/// A deleted booking still exists in the client's calendar, and a 404 would leave it there forever
/// with only a sync error to explain it. Publishing a `STATUS:CANCELLED` event under the same `UID`
/// is what actually removes it. `start` is the slot recorded when the token was minted — the only
/// one left to name, and the one the client's copy was written from.
pub fn deleted_event(share: &AppointmentShare, domain: &str, shop_name: &str, page_url: &str) -> IcsEvent {
    IcsEvent {
        uid: client_uid(&share.appointment_id, domain),
        start: share.start_at,
        // No end: a cancelled event needs no duration, and inventing one would state a slot this no
        // longer knows.
        end: 0,
        summary: "Cancelled appointment".to_string(),
        description: if shop_name.trim().is_empty() {
            "This appointment is no longer on the shop's calendar.".to_string()
        } else {
            format!(
                "This appointment is no longer on {}'s calendar.",
                shop_name.trim()
            )
        },
        status: IcsStatus::Cancelled,
        last_modified: 0,
        url: page_url.to_string(),
        location: shop_name.trim().to_string(),
        // Above any live revision this booking could have reached, so a client holding an older copy
        // takes the cancellation rather than ignoring it as stale.
        sequence: i32::MAX as u32,
    }
}

/// The calendar name for a ONE-appointment document. Names the booking, not the shop's schedule —
/// a client subscribing to this must not see it labelled as if it were the whole shop calendar.
fn calendar_name(summary: &str) -> String {
    summary.to_string()
}

fn ics_document(shop_name: &str, event: IcsEvent, ts: u64) -> String {
    let name = calendar_name(&event.summary);
    let prodid = if shop_name.trim().is_empty() {
        "-//DogTag//Appointment//EN".to_string()
    } else {
        format!("-//DogTag//{}//EN", shop_name.trim())
    };
    ics::calendar(&prodid, &name, ts, &[event])
}

// ============================================================================================
// add-to-Google
// ============================================================================================

/// A Google Calendar "template" link for one event.
///
/// `dates` uses the RFC 5545 UTC basic form (`…Z/…Z`) and NO `ctz` parameter, which is what makes
/// this immune to the whole daylight-saving class: an absolute instant needs no timezone
/// interpretation, so there is no wall-clock to re-resolve and nothing to get an hour wrong twice a
/// year. Google renders it in the viewer's own zone, which is the correct answer for a client who
/// may not live in the shop's.
///
/// Returns `""` when there is no instant to name — a link to an event with no time is not a link.
pub fn google_calendar_url(e: &IcsEvent) -> String {
    if e.start == 0 {
        return String::new();
    }
    // Google requires both halves; a booking with no usable end gets a nominal hour rather than a
    // malformed range. This is a display default in a link, not a claim about the slot — the `.ics`,
    // which is the authoritative copy, omits DTEND entirely in that case. Saturating for the same
    // reason as the mint's expiry: this branch is reached by any booking stored with no end at all
    // (`validate_slot` permits `end_at == 0`), so the addend is applied to an unbounded `start`.
    let end = if e.end > e.start {
        e.end
    } else {
        e.start.saturating_add(3600)
    };
    let dates = format!("{}/{}", ics::format_utc(e.start), ics::format_utc(end));
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    q.append_pair("action", "TEMPLATE");
    q.append_pair("text", &e.summary);
    q.append_pair("dates", &dates);
    if !e.description.is_empty() {
        q.append_pair("details", &e.description);
    }
    if !e.location.is_empty() {
        q.append_pair("location", &e.location);
    }
    format!(
        "https://calendar.google.com/calendar/render?{}",
        q.finish()
    )
}

// ============================================================================================
// public resolve — page + .ics
// ============================================================================================

/// What a token resolved to. The three outcomes are kept apart all the way to the response, because
/// rendering any one of them as another is the exact failure this surface exists to avoid.
enum Resolved {
    /// The booking, as the store currently holds it.
    Live(Box<Appointment>),
    /// The read SUCCEEDED and the booking is gone — the shop deleted it.
    Deleted,
    /// The read did not resolve. NOT a cancellation, NOT a confirmation.
    Unreadable,
}

async fn resolve(st: &AppState, share: &AppointmentShare) -> Resolved {
    match st.store.try_get_appointment(&share.appointment_id).await {
        Ok(Some(a)) => Resolved::Live(Box::new(a)),
        Ok(None) => Resolved::Deleted,
        Err(StoreReadError(e)) => {
            tracing::warn!(error = %e, "appointment handoff: store read failed");
            Resolved::Unreadable
        }
    }
}

fn issuer_domain(st: &AppState) -> String {
    if st.cfg.issuer_domain.is_empty() {
        "dogtag".to_string()
    } else {
        st.cfg.issuer_domain.clone()
    }
}

/// The absolute URL of this handoff page, or `""` when this deployment has no reachable base to
/// build one from. Empty means no `URL:` property and no printed link — never a guessed host.
fn page_url(st: &AppState, token: &str) -> String {
    match client_base(&st.cfg.deployment_url) {
        BaseUsability::Usable(base) => format!("{base}/a/{token}"),
        // A loopback base is a real address for the machine reading it, and the `.ics` is being
        // fetched BY that machine in a dev run — but it is not something to stamp into a client's
        // calendar as the place to check. Publish no link rather than one that dead-ends.
        BaseUsability::Loopback(_) | BaseUsability::Unset => String::new(),
    }
}

/// `GET /a/{token}` and `GET /a/{token}.ics` — the client's handoff.
///
/// One route, dispatched on the extension, mirroring the feed's `/calendar/feed/{token}[.ics]`: the
/// `.ics` form is what a phone opens into its calendar, the bare form is the page a human reads.
async fn handoff(State(st): State<AppState>, Path(raw): Path<String>) -> Response {
    let wants_ics = raw.ends_with(".ics");
    let token = raw.strip_suffix(".ics").unwrap_or(&raw);

    // The TOKEN lookup, and it is the read where a collapsed error does the most damage: it runs
    // first, and the "not valid" copy below tells the client to DISCARD the link. Answering that on
    // a driver fault would throw away a perfectly good link on the strength of a read that never
    // happened, so an unreadable store takes the same "could not check" branch as everything else.
    let share = match st.store.try_peek_appointment_share(token).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return if wants_ics {
                // No token means no UID, so there is nothing to tombstone — this really is a 404.
                (
                    StatusCode::NOT_FOUND,
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    "This appointment link is not valid or has expired.",
                )
                    .into_response()
            } else {
                not_found_page()
            };
        }
        Err(StoreReadError(e)) => {
            tracing::warn!(error = %e, "appointment handoff: share-token read failed");
            return unreadable_response(wants_ics, &st.cfg.issuer_name);
        }
    };

    let ts = now();
    let domain = issuer_domain(&st);
    let shop = st.cfg.issuer_name.clone();
    let url = page_url(&st, token);

    match resolve(&st, &share).await {
        Resolved::Live(a) => {
            let event = to_client_event(&a, &domain, &shop, &url);
            if wants_ics {
                ics_response(ics_document(&shop, event, ts))
            } else {
                live_page(&a, &event, &shop, &url, token)
            }
        }
        Resolved::Deleted => {
            let event = deleted_event(&share, &domain, &shop, &url);
            if wants_ics {
                ics_response(ics_document(&shop, event, ts))
            } else {
                deleted_page(&shop, token)
            }
        }
        Resolved::Unreadable => unreadable_response(wants_ics, &shop),
    }
}

/// The one "could not check" answer, shared by BOTH reads this route performs (the token and the
/// booking), so neither can drift into rendering an unreadable store as something else.
///
/// For the `.ics`: 503 and NO calendar body. Publishing a tombstone here would cancel a booking that
/// may be perfectly live; publishing the last-known event would state a slot this process did not
/// read. A subscriber treats 503 as "try later", which is exactly the truth.
fn unreadable_response(wants_ics: bool, shop: &str) -> Response {
    if wants_ics {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "Could not check this appointment right now. This does NOT mean it was cancelled — \
             please try again shortly.",
        )
            .into_response()
    } else {
        unreadable_page(shop)
    }
}

fn ics_response(body: String) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/calendar; charset=utf-8"),
            // The URL carries a token; keep it out of shared caches. `no-store` would also stop a
            // subscribing client re-reading it, which is the one thing that DOES keep a client's
            // copy fresh, so this is `private` + revalidate rather than no-store.
            (header::CACHE_CONTROL, "private, no-cache"),
            // `attachment` is what makes iOS and Android hand the file to the calendar app rather
            // than rendering it as text.
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"appointment.ics\"",
            ),
        ],
        body,
    )
        .into_response()
}

// ============================================================================================
// pages
// ============================================================================================

/// Escape text for HTML. Every value below is shop-authored, so this is not optional.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

const PAGE_CSS: &str = "\
:root{color-scheme:light dark}\
*{box-sizing:border-box}\
body{font-family:system-ui,-apple-system,'Segoe UI',sans-serif;background:#f1f5f9;color:#0f172a;margin:0;padding:1.5rem 1rem;line-height:1.5}\
.card{max-width:30rem;margin:0 auto;background:#fff;border:1px solid #e2e8f0;border-radius:16px;padding:1.5rem;box-shadow:0 1px 3px rgba(0,0,0,.07)}\
.shop{font-size:.8rem;letter-spacing:.06em;text-transform:uppercase;color:#64748b;margin:0 0 .35rem}\
h1{font-size:1.4rem;margin:0 0 .25rem;line-height:1.25}\
.when{font-size:1.1rem;font-weight:600;margin:.75rem 0 0}\
.tz{font-size:.8rem;color:#64748b;margin:.15rem 0 0}\
.pill{display:inline-block;font-weight:600;font-size:.78rem;padding:.25rem .7rem;border-radius:999px;margin:.9rem 0 0}\
.ok{background:#dcfce7;color:#166534}\
.warn{background:#fef3c7;color:#92400e}\
.bad{background:#fee2e2;color:#991b1b}\
.muted{background:#e2e8f0;color:#475569}\
dl{display:grid;grid-template-columns:auto 1fr;gap:.35rem .9rem;margin:1.1rem 0 0;font-size:.92rem}\
dt{color:#64748b}dd{margin:0}\
.actions{display:flex;flex-direction:column;gap:.6rem;margin:1.4rem 0 0}\
a.btn{display:block;text-align:center;text-decoration:none;font-weight:600;padding:.8rem 1rem;border-radius:10px;font-size:.95rem}\
a.primary{background:#2563eb;color:#fff}\
a.secondary{background:#fff;color:#1e293b;border:1px solid #cbd5e1}\
.note{font-size:.82rem;color:#64748b;margin:1.2rem 0 0;padding-top:1rem;border-top:1px solid #e2e8f0}\
@media(prefers-color-scheme:dark){\
body{background:#0f172a;color:#e2e8f0}\
.card{background:#1e293b;border-color:#334155}\
h1{color:#f8fafc}\
dd{color:#e2e8f0}\
a.secondary{background:#1e293b;color:#e2e8f0;border-color:#475569}\
.note,.tz,dt,.shop{color:#94a3b8}\
}";

/// Render a page.
///
/// `no-store` is load-bearing rather than boilerplate. This page's entire job is to state what the
/// booking IS RIGHT NOW — the client is expected to re-open the link precisely to re-check it — and a
/// cached copy states what it WAS, which on a cancelled booking means rendering `Scheduled` beside a
/// working "Add to calendar" button. That is a stale surface making a confident wrong claim, the one
/// thing this surface exists not to do, and nothing about the response would otherwise stop a browser
/// or an intermediary heuristically caching a `200 text/html`. `private` additionally keeps it out of
/// shared caches, because the URL carries a token.
fn page(code: StatusCode, title: &str, body: String) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{t}</title><style>{css}</style></head>\
         <body><div class=\"card\">{body}</div></body></html>",
        t = esc(title),
        css = PAGE_CSS,
    );
    (
        code,
        [(header::CACHE_CONTROL, "no-store, private")],
        Html(html),
    )
        .into_response()
}

/// The script that renders the slot in the READER'S OWN timezone.
///
/// Times travel to the browser as UNIX SECONDS in data attributes and are formatted by `Intl`, which
/// carries the full IANA database. No arithmetic is done on the epoch seconds to derive a wall clock
/// — the instant is handed to `Date` and formatted — so there is no fixed-86400 stepping and no
/// daylight-saving transition to land wrong on. The markup already contains a UTC rendering, so a
/// reader with no JavaScript still sees a correct, explicitly-labelled time rather than a blank.
const TIME_SCRIPT: &str = r#"<script>
(function(){
  var el=document.getElementById('when');
  if(!el||!window.Intl)return;
  var s=Number(el.getAttribute('data-start'))*1000;
  var e=Number(el.getAttribute('data-end'))*1000;
  if(!s)return;
  var d=new Date(s);
  var day=d.toLocaleDateString(undefined,{weekday:'long',day:'numeric',month:'long',year:'numeric'});
  var t=d.toLocaleTimeString(undefined,{hour:'2-digit',minute:'2-digit'});
  var txt=day+' at '+t;
  if(e>s){txt+=' – '+new Date(e).toLocaleTimeString(undefined,{hour:'2-digit',minute:'2-digit'});}
  el.textContent=txt;
  var z=document.getElementById('tz');
  if(z){
    try{z.textContent='Times shown in '+Intl.DateTimeFormat().resolvedOptions().timeZone+'.';}
    catch(err){}
  }
})();
</script>"#;

/// UTC rendering of the slot, used as the no-JavaScript fallback. Labelled UTC, because an unlabelled
/// time in a zone the reader does not know is a wrong time.
fn utc_slot(start: u64, end: u64) -> String {
    let s = ics::format_utc(start);
    let pretty = |v: &str| format!("{}-{}-{} {}:{}", &v[0..4], &v[4..6], &v[6..8], &v[9..11], &v[11..13]);
    if end > start {
        let e = ics::format_utc(end);
        format!("{} – {} UTC", pretty(&s), pretty(&e))
    } else {
        format!("{} UTC", pretty(&s))
    }
}

fn live_page(
    a: &Appointment,
    event: &IcsEvent,
    shop: &str,
    url: &str,
    token: &str,
) -> Response {
    let cancelled = a.status == "cancelled";
    let ics_href = format!("/a/{}.ics", esc(token));
    let google = google_calendar_url(event);

    let shop_line = if shop.trim().is_empty() {
        String::new()
    } else {
        format!("<p class=\"shop\">{}</p>", esc(shop.trim()))
    };

    let (pill_class, pill_text) = if cancelled {
        ("bad", "Cancelled")
    } else {
        match a.status.as_str() {
            "completed" => ("muted", "Completed"),
            "no_show" => ("muted", "Missed"),
            "confirmed" => ("ok", "Confirmed"),
            "in_progress" => ("ok", "In progress"),
            _ => ("ok", "Scheduled"),
        }
    };

    let mut rows = String::new();
    if !a.pet_name.trim().is_empty() {
        rows.push_str(&format!(
            "<dt>Pet</dt><dd>{}</dd>",
            esc(a.pet_name.trim())
        ));
    }
    if !a.groomer.trim().is_empty() {
        rows.push_str(&format!(
            "<dt>Groomer</dt><dd>{}</dd>",
            esc(a.groomer.trim())
        ));
    }

    // A cancelled booking gets no cheerful "add to calendar". The only action offered is the one
    // that is true: importing it REMOVES the event from a calendar that already holds it.
    let actions = if cancelled {
        format!(
            "<div class=\"actions\">\
             <a class=\"secondary btn\" href=\"{ics}\">Update my calendar (removes it)</a>\
             </div>"
        , ics = ics_href)
    } else {
        let google_btn = if google.is_empty() {
            String::new()
        } else {
            format!(
                "<a class=\"secondary btn\" href=\"{g}\" rel=\"noopener noreferrer\" target=\"_blank\">Add to Google Calendar</a>",
                g = esc(&google)
            )
        };
        // A subscribe option only exists when there is a reachable absolute URL to subscribe TO.
        let subscribe = if url.is_empty() {
            String::new()
        } else {
            let webcal = url
                .strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"))
                .map(|rest| format!("webcal://{rest}.ics"))
                .unwrap_or_default();
            if webcal.is_empty() {
                String::new()
            } else {
                format!(
                    "<a class=\"secondary btn\" href=\"{w}\">Subscribe (keeps updating)</a>",
                    w = esc(&webcal)
                )
            }
        };
        format!(
            "<div class=\"actions\">\
             <a class=\"primary btn\" href=\"{ics}\">Add to calendar</a>\
             {google_btn}{subscribe}\
             </div>",
            ics = ics_href
        )
    };

    // The limitation, said plainly at the point the client acts on it.
    let note = if cancelled {
        "The shop cancelled this appointment. If you already added it to your calendar, use the \
         button above to remove it."
            .to_string()
    } else {
        let mut n = "Adding this saves a copy of the booking as it is now. If the shop changes or \
                     cancels it, your calendar will not update on its own — re-open this page to \
                     see the current details."
            .to_string();
        if !url.is_empty() {
            n.push_str(" \"Subscribe\" does keep updating, but your calendar app decides how often \
                        it checks — Google can take several hours.");
        }
        n
    };

    let body = format!(
        "{shop_line}\
         <h1>{summary}</h1>\
         <p class=\"when\" id=\"when\" data-start=\"{start}\" data-end=\"{end}\">{utc}</p>\
         <p class=\"tz\" id=\"tz\">Shown in UTC until your browser localises it.</p>\
         <span class=\"pill {pill_class}\">{pill_text}</span>\
         <dl>{rows}</dl>\
         {actions}\
         <p class=\"note\">{note}</p>\
         {script}",
        summary = esc(&event.summary),
        start = a.start_at,
        end = a.end_at,
        utc = esc(&utc_slot(a.start_at, a.end_at)),
        note = esc(&note),
        script = TIME_SCRIPT,
    );
    page(StatusCode::OK, &event.summary, body)
}

fn deleted_page(shop: &str, token: &str) -> Response {
    let who = if shop.trim().is_empty() {
        "the shop".to_string()
    } else {
        esc(shop.trim())
    };
    let body = format!(
        "<h1>This appointment is no longer booked</h1>\
         <span class=\"pill bad\">Removed</span>\
         <p class=\"note\">{who} has removed this appointment from their calendar. If you added it \
          to your own calendar, use the button below to clear it.</p>\
         <div class=\"actions\">\
          <a class=\"secondary btn\" href=\"/a/{tok}.ics\">Remove it from my calendar</a>\
         </div>",
        tok = esc(token),
    );
    page(StatusCode::NOT_FOUND, "Appointment removed", body)
}

/// The store could not be read. This is its OWN state — it must never render as a cancellation and
/// never as a confirmation, and it offers no add-to-calendar action, because this process does not
/// currently know what it would be adding.
fn unreadable_page(shop: &str) -> Response {
    let who = if shop.trim().is_empty() {
        "the shop".to_string()
    } else {
        esc(shop.trim())
    };
    let body = format!(
        "<h1>We could not check this appointment</h1>\
         <span class=\"pill warn\">Could not check</span>\
         <p class=\"note\">Something went wrong reading {who}'s calendar just now, so this page \
          cannot show the booking. <strong>This does not mean it was cancelled.</strong> Please \
          reload in a moment, or contact the shop directly.</p>",
    );
    page(StatusCode::SERVICE_UNAVAILABLE, "Could not check", body)
}

fn not_found_page() -> Response {
    let body = "<h1>This link is not valid</h1>\
         <span class=\"pill muted\">Unknown link</span>\
         <p class=\"note\">This appointment link is not one we recognise, or it has expired. Ask \
          the shop for a new one.</p>"
        .to_string();
    page(StatusCode::NOT_FOUND, "Link not valid", body)
}

// ============================================================================================
// routers
// ============================================================================================

/// The UNAUTHENTICATED client handoff. Split out for the same reason as the feed: the token in the
/// path IS the authorization, and nothing else here may be public.
pub fn appointment_share_public_router() -> Router<AppState> {
    Router::new().route("/a/:token", get(handoff))
}

/// Operator-gated minting.
pub fn appointment_share_admin_router() -> Router<AppState> {
    Router::new().route("/appointments/:id/share", post(mint_share))
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
            start_at: 1_772_020_800, // 2026-02-25T12:00:00Z
            end_at: 1_772_024_400,   // 2026-02-25T13:00:00Z
            status: "scheduled".into(),
            notes: "Owner says the dog bites. Muzzle on arrival.".into(),
            groomer: "Sam".into(),
            created_at: 1_772_000_000,
            updated_at: 1_772_000_000,
            client_name: "Alice Tan".into(),
            pet_name: "Rex".into(),
            search_key: String::new(),
            source: None,
            external_uid: None,
        }
    }

    // ---- client_base: the QR must name a host the CLIENT'S PHONE can reach --------------------

    #[test]
    fn client_base_accepts_a_routable_host() {
        assert_eq!(
            client_base("http://192.168.1.20:43618"),
            BaseUsability::Usable("http://192.168.1.20:43618".into())
        );
        assert_eq!(
            client_base("https://shop.example/"),
            BaseUsability::Usable("https://shop.example".into()),
            "a trailing slash is trimmed so the path is not doubled"
        );
    }

    #[test]
    fn client_base_refuses_an_unset_deployment_url() {
        assert_eq!(client_base(""), BaseUsability::Unset);
        assert_eq!(client_base("   "), BaseUsability::Unset);
    }

    #[test]
    fn client_base_refuses_every_loopback_shape() {
        // The shipped default (`main.rs` -> http://localhost:{port}`) is the case a real shop hits
        // first, and the one that most convincingly looks like it works from the operator's laptop.
        for base in [
            "http://localhost:43618",
            "http://LOCALHOST:43618",
            "http://127.0.0.1:43618",
            "http://127.1.2.3:43618",
            "http://[::1]:43618",
            "http://0.0.0.0:43618",
            "http://api.localhost:43618",
        ] {
            assert!(
                matches!(client_base(base), BaseUsability::Loopback(_)),
                "{base} must be refused as loopback"
            );
        }
    }

    #[test]
    fn client_base_does_not_mistake_a_lookalike_domain_for_loopback() {
        // String matching on "localhost" would wrongly suppress this REAL host.
        assert!(matches!(
            client_base("https://localhost.evil.example"),
            BaseUsability::Usable(_)
        ));
        assert!(matches!(
            client_base("https://mylocalhost.example"),
            BaseUsability::Usable(_)
        ));
    }

    #[test]
    fn every_refused_base_states_a_reason_naming_the_fix() {
        let unset = qr_unavailable_reason(&client_base("")).expect("unset must give a reason");
        assert!(unset.contains("DEPLOYMENT_URL"));
        let loop_r = qr_unavailable_reason(&client_base("http://localhost:43618"))
            .expect("loopback must give a reason");
        assert!(loop_r.contains("loopback"));
        assert!(loop_r.contains("DEPLOYMENT_URL"));
        // ...and a usable base states none.
        assert_eq!(qr_unavailable_reason(&client_base("https://shop.example")), None);
    }

    // ---- the client projection: what it carries, and what it must NOT -------------------------

    #[test]
    fn the_client_event_never_carries_the_shop_s_notes_or_the_client_s_name() {
        // The two fields the SHOP's feed publishes and this must not. `notes` is arbitrary operator
        // text; `client_name` names the person a leaked link would be about.
        let e = to_client_event(&appt(), "shop.example", "Pampered Paws", "https://shop.example/a/tok");
        let rendered = ics_document("Pampered Paws", e.clone(), 1_772_000_000);
        for leaked in ["Owner says the dog bites", "Muzzle on arrival", "Alice Tan"] {
            assert!(
                !rendered.contains(leaked),
                "the handoff leaked {leaked:?}:\n{rendered}"
            );
        }
        // ...while still carrying what the client actually needs.
        assert!(rendered.contains("SUMMARY:Full groom - Rex"));
        assert!(rendered.contains("Your groomer: Sam"));
        assert!(rendered.contains("LOCATION:Pampered Paws"));
    }

    #[test]
    fn the_client_event_summary_is_the_service_and_their_own_pet() {
        assert_eq!(client_summary(&appt()), "Full groom - Rex");
        let mut a = appt();
        a.pet_name = "  ".into();
        assert_eq!(client_summary(&a), "Full groom");
        a.service = "".into();
        assert_eq!(client_summary(&a), "Appointment");
    }

    #[test]
    fn the_client_uid_is_the_booking_id_and_ignores_an_imported_source_uid() {
        let mut a = appt();
        a.external_uid = Some("evt-9@google.com".into());
        let e = to_client_event(&a, "shop.example", "", "");
        assert_eq!(
            e.uid, "appt-1@shop.example",
            "the UID must stay derivable from the id alone, so a DELETED booking is still tombstonable"
        );
    }

    #[test]
    fn the_client_event_states_the_snapshot_limitation_on_the_entry_itself() {
        let e = to_client_event(&appt(), "d", "Pampered Paws", "https://shop.example/a/tok");
        assert!(
            e.description.contains("will not update on its own"),
            "the calendar entry must carry the caveat, not only the page: {}",
            e.description
        );
        assert!(e.description.contains("https://shop.example/a/tok"));
        assert_eq!(e.url, "https://shop.example/a/tok");
    }

    #[test]
    fn a_cancelled_booking_publishes_as_cancelled_so_re_adding_removes_it() {
        let mut a = appt();
        a.status = "cancelled".into();
        let e = to_client_event(&a, "d", "Pampered Paws", "");
        assert_eq!(e.status, IcsStatus::Cancelled);
        assert!(e.description.contains("CANCELLED"));
        assert!(ics_document("Pampered Paws", e, 1).contains("STATUS:CANCELLED\r\n"));
    }

    #[test]
    fn a_completed_or_missed_booking_is_not_published_as_cancelled() {
        // Only a cancellation removes the client's event. A finished appointment still happened.
        for s in ["completed", "no_show", "confirmed", "in_progress", "scheduled"] {
            let mut a = appt();
            a.status = s.into();
            assert_eq!(
                to_client_event(&a, "d", "", "").status,
                IcsStatus::Confirmed,
                "status {s} must not read as a cancellation"
            );
        }
    }

    // ---- SEQUENCE: what makes a re-download supersede rather than duplicate --------------------

    #[test]
    fn the_sequence_advances_when_the_booking_is_edited() {
        let mut a = appt();
        assert_eq!(client_sequence(&a), 0, "never edited");
        a.updated_at = a.created_at + 3600;
        assert_eq!(client_sequence(&a), 3600);
        // ...and a later edit is strictly higher, which is what RFC 5545 §3.8.7.4 requires.
        a.updated_at = a.created_at + 7200;
        assert!(client_sequence(&a) > 3600);
    }

    #[test]
    fn the_sequence_never_overflows_the_icalendar_integer_range() {
        // iCalendar INTEGER is signed 32-bit; a raw unix timestamp exceeds it in 2038 and would wrap
        // to a NEGATIVE revision, un-superseding an update the client already holds.
        let mut a = appt();
        a.created_at = 0;
        a.updated_at = u64::MAX;
        assert_eq!(client_sequence(&a), i32::MAX as u32);
    }

    #[test]
    fn a_clock_that_went_backwards_never_yields_a_negative_revision() {
        let mut a = appt();
        a.updated_at = a.created_at - 500;
        assert_eq!(client_sequence(&a), 0, "saturating, not wrapping");
    }

    // ---- the deleted-booking tombstone --------------------------------------------------------

    #[test]
    fn a_deleted_booking_publishes_a_cancellation_under_the_same_uid() {
        let share = AppointmentShare {
            appointment_id: "appt-1".into(),
            start_at: 1_772_020_800,
            expires_at: 9_999_999_999,
        };
        let live = to_client_event(&appt(), "shop.example", "Pampered Paws", "");
        let dead = deleted_event(&share, "shop.example", "Pampered Paws", "");
        assert_eq!(
            dead.uid, live.uid,
            "a tombstone that does not match the UID the client holds cancels nothing"
        );
        assert_eq!(dead.status, IcsStatus::Cancelled);
        assert_eq!(dead.start, 1_772_020_800, "the slot recorded at mint time");
        assert!(
            dead.sequence > live.sequence,
            "a tombstone must supersede the copy the client already has"
        );
        let doc = ics_document("Pampered Paws", dead, 1);
        assert!(doc.contains("STATUS:CANCELLED\r\n"));
        assert!(doc.contains("UID:appt-1@shop.example\r\n"));
    }

    // ---- the calendar document ----------------------------------------------------------------

    #[test]
    fn the_handoff_document_holds_exactly_one_event() {
        let doc = ics_document(
            "Pampered Paws",
            to_client_event(&appt(), "d", "Pampered Paws", ""),
            1,
        );
        assert_eq!(doc.matches("BEGIN:VEVENT").count(), 1);
        assert_eq!(doc.matches("END:VEVENT").count(), 1);
    }

    #[test]
    fn the_handoff_document_names_the_one_appointment_not_the_shop_s_schedule() {
        let doc = ics_document(
            "Pampered Paws",
            to_client_event(&appt(), "d", "Pampered Paws", ""),
            1,
        );
        assert!(
            doc.contains("X-WR-CALNAME:Full groom - Rex\r\n"),
            "a client subscribing must not see this labelled as the shop's whole calendar: {doc}"
        );
    }

    // ---- the cross-language contract with the parser test ------------------------------------

    /// The handoff document, byte for byte.
    ///
    /// These exact bytes are also pinned as a fixture by
    /// `packages/ui/test/clientHandoffIcs.test.ts`, which feeds them through this repo's real
    /// iCalendar PARSER — the one the shop's `.ics` import uses — and checks a client's calendar
    /// would read the right instants, the right UID and an unfolded description.
    ///
    /// This assertion is what makes that pairing an actual contract rather than a snapshot that
    /// quietly goes stale: a change to the writer's grammar (a new property, a different fold point,
    /// a changed escape) fails HERE, naming the TS fixture that has to be regenerated with it.
    /// Without it, the writer could drift and the parser test would go on happily parsing bytes
    /// nothing produces any more.
    #[test]
    fn the_handoff_document_is_byte_for_byte_the_fixture_the_parser_test_pins() {
        let mut a = appt();
        a.updated_at = a.created_at + 3600;
        a.notes = "muzzle on arrival".into();
        let url = "https://shop.example/a/deadbeefdeadbeefdeadbeefdeadbeef";
        let out = ics_document(
            "Pampered Paws",
            to_client_event(&a, "groomer.example", "Pampered Paws", url),
            1_772_000_000,
        );
        let expect = concat!(
            "BEGIN:VCALENDAR\r\n",
            "VERSION:2.0\r\n",
            "PRODID:-//DogTag//Pampered Paws//EN\r\n",
            "CALSCALE:GREGORIAN\r\n",
            "METHOD:PUBLISH\r\n",
            "X-WR-CALNAME:Full groom - Rex\r\n",
            "REFRESH-INTERVAL;VALUE=DURATION:PT15M\r\n",
            "X-PUBLISHED-TTL:PT15M\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:appt-1@groomer.example\r\n",
            "DTSTAMP:20260225T061320Z\r\n",
            "DTSTART:20260225T120000Z\r\n",
            "DTEND:20260225T130000Z\r\n",
            "SUMMARY:Full groom - Rex\r\n",
            "DESCRIPTION:With Pampered Paws.\\nStatus: Scheduled\\nYour groomer: Sam\\nThis\r\n",
            "  is a copy of the booking as it stood when you added it. If the shop chang\r\n",
            " es or cancels it\\, your calendar will not update on its own.\\nCheck the cu\r\n",
            " rrent details: https://shop.example/a/deadbeefdeadbeefdeadbeefdeadbeef\r\n",
            "LOCATION:Pampered Paws\r\n",
            "URL:https://shop.example/a/deadbeefdeadbeefdeadbeefdeadbeef\r\n",
            "STATUS:CONFIRMED\r\n",
            "SEQUENCE:3600\r\n",
            "LAST-MODIFIED:20260225T071320Z\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n",
        );
        assert_eq!(
            out, expect,
            "the handoff bytes changed — regenerate the fixture in \
             packages/ui/test/clientHandoffIcs.test.ts to match, so the parser test keeps checking \
             what this actually writes"
        );
    }

    /// The tombstone, byte for byte. Same contract, same fixture file.
    #[test]
    fn the_tombstone_document_is_byte_for_byte_the_fixture_the_parser_test_pins() {
        let share = AppointmentShare {
            appointment_id: "appt-1".into(),
            start_at: 1_772_020_800,
            expires_at: 0,
        };
        let out = ics_document(
            "Pampered Paws",
            deleted_event(&share, "groomer.example", "Pampered Paws", ""),
            1_772_000_000,
        );
        let expect = concat!(
            "BEGIN:VCALENDAR\r\n",
            "VERSION:2.0\r\n",
            "PRODID:-//DogTag//Pampered Paws//EN\r\n",
            "CALSCALE:GREGORIAN\r\n",
            "METHOD:PUBLISH\r\n",
            "X-WR-CALNAME:Cancelled appointment\r\n",
            "REFRESH-INTERVAL;VALUE=DURATION:PT15M\r\n",
            "X-PUBLISHED-TTL:PT15M\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:appt-1@groomer.example\r\n",
            "DTSTAMP:20260225T061320Z\r\n",
            "DTSTART:20260225T120000Z\r\n",
            "SUMMARY:Cancelled appointment\r\n",
            "DESCRIPTION:This appointment is no longer on Pampered Paws's calendar.\r\n",
            "LOCATION:Pampered Paws\r\n",
            "STATUS:CANCELLED\r\n",
            "SEQUENCE:2147483647\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n",
        );
        assert_eq!(
            out, expect,
            "the tombstone bytes changed — regenerate the fixture in \
             packages/ui/test/clientHandoffIcs.test.ts"
        );
    }

    // ---- Google link: UTC instants, so the DST class cannot arise ------------------------------

    #[test]
    fn the_google_link_uses_absolute_utc_instants_and_no_timezone_parameter() {
        let e = to_client_event(&appt(), "d", "Pampered Paws", "");
        let u = google_calendar_url(&e);
        assert!(u.starts_with("https://calendar.google.com/calendar/render?"));
        assert!(u.contains("action=TEMPLATE"));
        assert!(
            u.contains("dates=20260225T120000Z%2F20260225T130000Z"),
            "must be the UTC basic form, which needs no zone interpretation: {u}"
        );
        assert!(
            !u.contains("ctz="),
            "naming a timezone would reintroduce a wall-clock to re-resolve: {u}"
        );
    }

    #[test]
    fn the_google_link_escapes_shop_text_rather_than_breaking_the_query() {
        let mut a = appt();
        a.service = "Wash & trim".into();
        a.pet_name = "Rex #2".into();
        let u = google_calendar_url(&to_client_event(&a, "d", "Paws & Claws", ""));
        assert!(u.contains("text=Wash+%26+trim+-+Rex+%232"), "{u}");
        assert!(u.contains("location=Paws+%26+Claws"), "{u}");
    }

    #[test]
    fn a_booking_with_no_usable_end_still_yields_a_well_formed_google_range() {
        let mut a = appt();
        a.end_at = a.start_at; // zero-length slot
        let u = google_calendar_url(&to_client_event(&a, "d", "", ""));
        assert!(u.contains("dates=20260225T120000Z%2F20260225T130000Z"), "{u}");
    }

    #[test]
    fn an_event_with_no_start_yields_no_google_link() {
        let mut e = to_client_event(&appt(), "d", "", "");
        e.start = 0;
        assert_eq!(google_calendar_url(&e), "");
    }

    // ---- DST: the same wall clock in two zones, across both transitions ------------------------

    /// Bookings that sit either side of a daylight-saving transition, in TWO zones and in BOTH
    /// directions. Each entry is `(what a human in that zone calls it, the instant, the exact UTC
    /// the handoff must publish)`.
    ///
    /// The property under test is that NOTHING in this module re-derives a wall clock. A booking is
    /// an instant; the `.ics` publishes it in UTC and the Google link publishes the same UTC. So a
    /// booking on either side of a transition round-trips exactly, in every zone at once. That is
    /// what makes the #89 grid class — fixed-86400 stepping drifting off local midnight and silently
    /// dropping bookings — structurally impossible here rather than merely untested.
    ///
    /// Verified against the IANA database rather than computed by hand. Note the London fall-back
    /// case: 00:30 BST on the 25th is 23:30 UTC on the 24TH, so anything deriving "the day" from the
    /// UTC date lands on the wrong one.
    const DST_CASES: &[(&str, u64, &str)] = &[
        // Europe/London, 2026-03-29: BST begins (01:00 GMT -> 02:00 BST).
        ("Europe/London 00:30 GMT, before the clocks go forward", 1_774_744_200, "20260329T003000Z"),
        ("Europe/London 03:30 BST, after the clocks went forward", 1_774_751_400, "20260329T023000Z"),
        // Europe/London, 2026-10-25: BST ends (02:00 BST -> 01:00 GMT).
        ("Europe/London 00:30 BST, before the clocks go back", 1_792_884_600, "20261024T233000Z"),
        ("Europe/London 03:30 GMT, after the clocks went back", 1_792_899_000, "20261025T033000Z"),
        // America/New_York, 2026-03-08: EDT begins (02:00 EST -> 03:00 EDT).
        ("America/New_York 01:30 EST, before the clocks go forward", 1_772_951_400, "20260308T063000Z"),
        ("America/New_York 03:30 EDT, after the clocks went forward", 1_772_955_000, "20260308T073000Z"),
        // America/New_York, 2026-11-01: EST returns (02:00 EDT -> 01:00 EST).
        ("America/New_York 00:30 EDT, before the clocks go back", 1_793_507_400, "20261101T043000Z"),
        ("America/New_York 03:30 EST, after the clocks went back", 1_793_521_800, "20261101T083000Z"),
    ];

    #[test]
    fn the_dst_fixtures_really_do_straddle_a_transition() {
        // Guards the tests below from passing vacuously on instants that turned out to be ordinary
        // ones. For each pair: how far apart the two bookings look ON THE LOCAL CLOCK, and how much
        // real time actually separates them. The two must differ by exactly one hour — that
        // discrepancy IS the daylight-saving transition, and its presence is what proves these
        // fixtures sit on one. `(label, before, after, local_hours, real_secs)`.
        let pairs = [
            ("London spring forward", DST_CASES[0].1, DST_CASES[1].1, 3u64, 2 * 3600u64),
            ("London fall back", DST_CASES[2].1, DST_CASES[3].1, 3, 4 * 3600),
            ("New York spring forward", DST_CASES[4].1, DST_CASES[5].1, 2, 3600),
            ("New York fall back", DST_CASES[6].1, DST_CASES[7].1, 3, 4 * 3600),
        ];
        for (label, before, after, local_hours, real_secs) in pairs {
            assert_eq!(
                after - before,
                real_secs,
                "{label}: real elapsed time between the two fixtures"
            );
            let naive = local_hours * 3600;
            assert_eq!(
                naive.abs_diff(real_secs),
                3600,
                "{label}: reading the local clock as elapsed time must be off by exactly the \
                 transition's hour — if it is not, this pair does not straddle one"
            );
        }
    }

    #[test]
    fn a_booking_on_a_dst_boundary_publishes_the_exact_utc_instant_in_both_zones() {
        for (label, start, expect_dtstart) in DST_CASES {
            let mut a = appt();
            a.start_at = *start;
            a.end_at = start + 3600;
            let doc = ics_document("Paws", to_client_event(&a, "d", "Paws", ""), 1);
            assert!(
                doc.contains(&format!("DTSTART:{expect_dtstart}\r\n")),
                "{label}: expected DTSTART:{expect_dtstart} in\n{doc}"
            );
        }
    }

    #[test]
    fn a_booking_on_a_dst_boundary_yields_the_exact_google_range_in_both_zones() {
        for (label, start, expect_dtstart) in DST_CASES {
            let mut a = appt();
            a.start_at = *start;
            a.end_at = start + 3600;
            let u = google_calendar_url(&to_client_event(&a, "d", "", ""));
            let end = ics::format_utc(start + 3600);
            assert!(
                u.contains(&format!("dates={expect_dtstart}%2F{end}")),
                "{label}: expected dates={expect_dtstart}/{end} in {u}"
            );
        }
    }

    #[test]
    fn an_hour_long_booking_is_an_hour_long_on_every_side_of_a_transition() {
        // The end is the stored instant, never start + "one local hour" re-derived from a calendar
        // day — which is where the 23h/25h day would bite.
        for (label, start, _) in DST_CASES {
            let mut a = appt();
            a.start_at = *start;
            a.end_at = start + 3600;
            let e = to_client_event(&a, "d", "", "");
            assert_eq!(e.end - e.start, 3600, "{label}");
        }
    }

    // ---- the no-JavaScript time fallback ------------------------------------------------------

    #[test]
    fn the_utc_fallback_labels_its_zone_rather_than_showing_a_bare_time() {
        // An unlabelled time in a zone the reader does not know is a wrong time.
        let s = utc_slot(1_772_020_800, 1_772_024_400);
        assert_eq!(s, "2026-02-25 12:00 – 2026-02-25 13:00 UTC");
        assert_eq!(utc_slot(1_772_020_800, 0), "2026-02-25 12:00 UTC");
    }

    // ---- escaping -----------------------------------------------------------------------------

    #[test]
    fn shop_text_is_html_escaped_on_the_page() {
        assert_eq!(esc("<script>x</script>"), "&lt;script&gt;x&lt;/script&gt;");
        assert_eq!(esc("Paws & Claws"), "Paws &amp; Claws");
        assert_eq!(esc("a\"b'c"), "a&quot;b&#39;c");
    }
}

