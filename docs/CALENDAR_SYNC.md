# Calendar sync — what ships, and what native two-way sync would actually cost

This is the decision document for connecting a DogTag shop's calendar to Google Calendar, Apple
Calendar and Luma.

**Short version.** A published `.ics` subscription feed ships today and covers all three providers
with no OAuth, no API keys and nothing that breaks when a provider changes its terms.
It is READ-ONLY and refreshes on the subscriber's own schedule.
Native two-way sync is a materially larger piece per provider, and this document exists so the
decision to build it (or not) is made against real numbers rather than a vague sense that "sync"
is one feature.

---

## 1. What ships now: the `.ics` feed (stage a)

### 1.1 Out — the subscription feed

`GET /api/calendar/feed/<secret>.ics` serves the shop's bookings as an RFC 5545 calendar.
The operator publishes, copies and revokes the address on **Calendar → Calendar sync** in the
groomer portal.

| Provider | Subscribes to an ICS URL? | How |
| --- | --- | --- |
| **Apple Calendar** | Yes | File → New Calendar Subscription, or a `webcal://` link. Refresh interval is user-configurable, down to every 5 minutes. Works against a `localhost` URL. |
| **Google Calendar** | Yes | Other calendars → From URL. Requires a PUBLICLY reachable URL. Refresh is on Google's schedule and is not configurable — historically hours, not minutes. |
| **Luma** | Yes | Added as an external calendar subscription in calendar settings. |

Implementation: `stacks/vet/api/src/ics.rs` (pure RFC 5545 serialization) and
`stacks/vet/api/src/calendar_ics.rs` (routes, identity, dedup).

**The feed URL is a credential.** A calendar client cannot present a bearer token, so authorization
is the secret in the path: 32 CSPRNG bytes, compared in constant time, revocable in one click.
Anyone holding the URL can read the shop's whole schedule — client names, pets and times.
The portal says this at the point the operator copies the link, and rotation is one button.

**What the feed does not give you, stated plainly:**

- **It is one direction.** Bookings made in DogTag appear in the subscribed calendar.
  An event created, moved or deleted in Google, Apple or Luma does **not** come back.
- **Refresh is the subscriber's decision.** The feed asks for `REFRESH-INTERVAL:PT15M`; Google in
  particular ignores that and polls on its own cadence, so a booking made now can take hours to
  appear in a Google-subscribed calendar. Apple honours a user-set interval and is far quicker.
- **The window is bounded.** 90 days back, 400 days forward, capped at 2000 events. Exceeding the cap
  emits a visible marker event in the calendar itself rather than truncating quietly.

### 1.2 In — `.ics` import

`POST /api/calendar/import` creates and updates bookings from an uploaded file. Same portal page.

Three deliberate limitations, each surfaced in the UI as a count and stamped on the booking's notes:

- **Recurrence is not expanded.** An event carrying an `RRULE` is imported as the single occurrence
  its `DTSTART` names. A weekly standing appointment becomes ONE booking, and the import says so.
- **All-day events become full-day slots** in the shop's local timezone. The data model has no
  all-day flag; a booking is a slot with a start and an end.
- **No client is invented.** An imported booking is UNASSIGNED (`clientId == ""`) until the operator
  links a real client by editing it. A calendar invite names an event, not one of the shop's
  customers, and fabricating directory entries to fill the column would be inventing customers.

**Re-importing the same file is safe.** Dedup is by the source event's `UID`, so a repeat import
updates the booking it already created rather than duplicating it. A re-import deliberately does not
overwrite the shop's own work: the calendar file owns WHEN (start, end, label), and the portal owns
WHO and HOW FAR ALONG (client, pet, groomer, workflow status). A `STATUS:CANCELLED` in the source
file is the one status change the file is allowed to make.

**Why the parser runs in the browser.** The hard part of reading a real calendar is
`DTSTART;TZID=Europe/London:20260330T100000` — turning a wall clock in a named zone into an exact
instant, with that zone's DST rules for that date. The Rust backend has no timezone database, and a
hand-rolled `VTIMEZONE` interpreter is a plausible-looking way to book appointments an hour off twice
a year. Every browser ships the full IANA database via `Intl.DateTimeFormat`, so
`packages/ui/src/calendar/ics.ts` resolves the offset exactly and POSTs unix seconds. The backend
performs no timezone interpretation at all. An event whose `TZID` the browser cannot resolve is
reported as skipped, naming the zone — never booked at a guessed offset.

---

## 2. What already exists in this repo for Google (and does not work yet)

This matters before scoping stage (b), because a good deal of Google two-way sync is already written.

**Present and wired:**

- `stacks/vet/api/src/calendar.rs` — a complete `GoogleCalendar` client against Calendar API v3:
  OAuth consent URL (`access_type=offline`, `prompt=consent`, `calendar.events` scope), authorization
  code exchange, `events.list` with `syncToken` (including HTTP-410 → full-resync signalling),
  `events.insert`/`update`, `events.watch`, and `freeBusy.query`.
- `stacks/vet/api/src/sync.rs` — the reconciliation engine: an **etag-primary echo discriminator**
  (an owned event whose stored etag matches is our own write and is skipped; a changed etag is a
  human edit and is detected, not dropped), untagged external events absorbed as read-only busy
  blocks, and full-resync on 410.
- Routes `GET /calendar/google/connect`, `GET /calendar/google/callback`, `POST /calendar/sync`.
- `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` / `GOOGLE_CALENDAR_ID` in `stacks/groomer/.env.example`.
- Substantial test coverage — but all of it against `MockCalendar`.

**Why it does not work today, honestly:**

1. **Never exercised against real Google.** There are no OAuth credentials in any test environment,
   so `GoogleCalendar` has never made a live call. It is wired, not proven.
2. **It syncs the wrong collection.** `sync::mirror_to_google` operates on `ApptReplica` — the
   Phase-7 replica of appointments pushed in from the CENTRAL backend via
   `PUT /v1/appointments/:id`. The groomer portal books into `crm_appointments` (`store::Appointment`),
   a different collection entirely. Even with working OAuth, the existing sync would not carry a
   single booking the portal made. Bridging the two models is real work, not a wiring change.
3. **The refresh token is stored unencrypted**, unlike the custody seed (which is age-encrypted).
   A refresh token is a long-lived credential to a third-party account and needs the same treatment.
4. **There is no webhook endpoint.** `events.watch` is called, but nothing serves the push
   notification Google sends back, and nothing renews the channel before it expires (max ~7 days
   for events; ~1 hour if unrenewed in some configurations).
5. **No portal UI.** Nothing in the groomer web app calls `/calendar/google/connect`.

---

## 3. What native two-way sync would require, per provider

### 3.1 Google Calendar

| Piece | What it means |
| --- | --- |
| **Cloud project + OAuth consent screen** | A Google Cloud project per deployment, or one shared project with each shop as a user. A shared project using the `calendar.events` scope is a RESTRICTED scope: Google requires an annual third-party security assessment (CASA) before it can be published beyond 100 test users. That is a recurring cost and a recurring calendar deadline, not a one-off. |
| **Token storage + refresh** | Refresh tokens encrypted at rest (the custody seal is the pattern to follow). Access tokens minted per request and cached. Refresh tokens can be revoked by the user at any time, so every call needs a "re-consent required" path that surfaces in the portal. |
| **Webhook channels** | `events.watch` gives push notifications, but channels expire and must be renewed on a schedule. The endpoint must be publicly reachable over HTTPS with a verified domain, which a self-hosted shop behind a home router does not have. Fallback: poll `events.list` with `syncToken` on a timer, which is what the existing engine already does on demand. |
| **The collection bridge** | Map `store::Appointment` ⇄ Google events, keeping the existing `dogtag.owned` / `dogtag.apptId` / `dogtag.rev` extended-property tagging so echoes stay distinguishable from human edits. |
| **Conflict resolution** | See §4. |

Realistic scope: the client is largely written; storage, refresh, the collection bridge, the webhook
endpoint and its renewal, the portal UI, and conflict resolution are not. The CASA assessment is the
item most likely to be the actual blocker for a product that ships to third-party shops.

### 3.2 Apple Calendar

Apple has no public REST calendar API. Two-way means **CalDAV** (RFC 4791) against iCloud:

- Basic auth with an **app-specific password** the shop generates in their Apple ID settings — a
  credential DogTag would then store and be responsible for. There is no OAuth, no scoping, and an
  app-specific password grants iCloud access beyond calendars.
- Discovery via `PROPFIND` on the principal URL, then the calendar-home set, then each collection.
- Change detection via `sync-collection` (RFC 6578) sync tokens, or `getctag` polling. There is no
  push; polling is the only option.
- Writes are `PUT`s of whole `VEVENT` bodies with `If-Match` ETags for optimistic concurrency.
- iCloud's CalDAV behaviour is undocumented and has changed without notice.

Realistic scope: a CalDAV client is a meaningful piece of work in its own right, and the
credential-handling posture (storing an app-specific password that unlocks more than calendars) is
the part to argue about before any of the code.

### 3.3 Luma

- Luma's public API is oriented around events and guests for a Luma calendar, not around being a
  general two-way sync target for an external booking system.
- API-key auth, so token refresh is not an issue, but key storage is.
- Luma's own ICS subscription support is the intended integration path for exactly this case.

Realistic scope: the smallest of the three, and also the one where ICS is most clearly the
intended answer rather than a workaround.

---

## 4. What two-way actually implies (the part that is not per-provider)

Two-way sync is not "the same code, in reverse". Once both sides can write, every one of these has to
be given a defined answer, and the answer has to be the same in every provider adapter:

1. **Echo suppression.** Our write comes back on the next list as a change. The existing engine's
   etag-primary discriminator is the right shape and already handles the subtle case (an event we
   own whose etag CHANGED is a human edit, not an echo).
2. **Simultaneous edits.** The operator moves a booking in the portal while the groomer moves it in
   Google. Options: last-writer-wins by timestamp (silent data loss, and clock skew decides),
   portal-always-wins (the phone edit vanishes, which is what a groomer will actually try to do), or
   surface the conflict for a human. Only the third is honest, and it needs a UI that does not exist.
3. **Deletion vs cancellation.** Deleting an event in Google is unambiguous there. In DogTag a
   booking with a recorded verification cannot be deleted — it is evidence. Inbound deletes therefore
   have to map to `status = cancelled`, and the shop has to understand that they did not delete it.
4. **Field ownership.** Which side owns the client link, the pet, the groomer, the workflow status?
   Google has none of these. The ICS import already answers this ("the file owns WHEN, the portal
   owns WHO"); a two-way sync must answer it in both directions.
5. **Recurrence, for real.** One-way ICS import can decline to expand `RRULE`. Two-way cannot: a
   recurring event edited on one side produces `RECURRENCE-ID` overrides, `EXDATE` exclusions and
   THIS-AND-FOLLOWING splits that must round-trip. This is the single largest hidden cost.
6. **Failure and backfill.** A provider outage, an expired token or a revoked grant means a window of
   unsynced changes. Reconciling on recovery needs a durable change log, not just a sync token.
7. **Multi-tenant blast radius.** A shared OAuth client means one compromised deployment implicates
   every shop's calendar.

---

## 5. Recommendation

**Ship the ICS feed and see whether it is enough**, which is what this change does.

It covers all three of the captain's calendars today, at zero ongoing integration cost, and the two
things it does not do — write back, and refresh promptly on Google — are worth measuring against real
use before paying for them. If the complaint that comes back is "Google is slow", the answer is Apple
Calendar or a shorter-polling client, not OAuth. If the complaint is "I moved an appointment on my
phone and DogTag didn't know", that is a real signal for stage (b), and it points at Google first
(where most of the client already exists) rather than at all three.

Before starting stage (b), the three questions to answer are: who pays for and maintains Google's
annual CASA assessment; whether storing an Apple app-specific password is acceptable given it grants
more than calendar access; and which of the §4.2 conflict answers the shop actually wants. None of
those are engineering questions.
