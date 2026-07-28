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
| **Apple Calendar** | Yes | File → New Calendar Subscription, or a `webcal://` link. The refresh interval is chosen by the subscriber. |
| **Google Calendar** | Yes | Other calendars → From URL. Needs a PUBLICLY reachable URL. Refresh is on Google's own schedule rather than the subscriber's, and can lag by hours. |
| **Luma** | Yes | Added as an external calendar subscription in calendar settings. |

> The per-provider details in this document — refresh behaviour, API terms, verification
> requirements — are the state of things as understood when it was written, and providers change
> them without notice. Treat them as the shape of the problem, not as current fact: re-check each
> against the provider's own documentation before scoping work that depends on it. What IS verified
> here is everything about this repo's own code, which the test suites pin.

Implementation: `stacks/vet/api/src/ics.rs` (pure RFC 5545 serialization) and
`stacks/vet/api/src/calendar_ics.rs` (routes, identity, dedup).

**The feed URL is a credential.** A calendar client cannot present a bearer token, so authorization
is the secret in the path: 32 CSPRNG bytes, compared in constant time, revocable in one click.
Anyone holding the URL can read the shop's whole schedule — client names, pets and times.
The portal says this at the point the operator copies the link, and rotation is one button.

**A path-borne secret is log-visible, and that is the residual risk.** The application never logs the
token, but the token is a URL path segment and the request URI is what a reverse proxy, CDN or tunnel
records in its access log by default — so every subscriber poll writes the credential to those logs
in plaintext, on the provider's polling schedule, indefinitely. A header- or query-borne credential
would not be exposed this way; a calendar client can present neither, which is the whole reason the
secret is in the path.
Both shipped nginx configs — `stacks/groomer/web/nginx.conf` and `stacks/vet/web/nginx.conf`, since
the feed route is mounted for every role — set `access_log off` for a feed read, matched on the
token's exact shape (64 lowercase hex characters, optionally `.ics`-suffixed).
`POST /calendar/feed/rotate` and any wrong-shaped token keep normal logging: neither carries a valid
credential, and that is where a probe still shows up.
Anything in front of those proxies (today, a Cloudflare tunnel) records the full URI unless
configured otherwise, and is outside this repo.
Rotation is the mitigation, which is why it is a one-click action rather than a recovery procedure.

**What the feed does not give you, stated plainly:**

- **It is one direction.** Bookings made in DogTag appear in the subscribed calendar.
  An event created, moved or deleted in Google, Apple or Luma does **not** come back.
- **Refresh is the subscriber's decision, not ours.** The feed asks for `REFRESH-INTERVAL:PT15M`, but
  that is advisory and a subscriber may poll on whatever cadence it likes. Google is the one to plan
  around: it does not expose the interval to the user, and a booking made now can take hours to show
  up in a Google-subscribed calendar. Apple lets the subscriber choose.
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

`notes` is the one field BOTH sides write, so it has its own rule. The import writes a block ending
with the line `Imported from an .ics calendar file.`, and on a re-import that block is re-stamped
from the current file while everything the operator typed around it is kept — above it or below it,
or replacing it wholesale. A line is only removed when it can be positively identified as one an
import wrote. The single cost of erring that way: when the source event's `DESCRIPTION` has changed
since the last import, the previous text is no longer identifiable and is kept as if the operator had
written it. Stale and visible beats silently deleted, for a box the shop types into.

**One unusable event never takes the file down with it.** An event whose start cannot be resolved, or
resolves before 1970, is counted as skipped with a reason and the rest of the calendar imports
normally. Above 1000 events in one upload the request is refused with a message naming the limit,
rather than left to fail as a body-size error that explains nothing — export a narrower date range
and import in parts.

**Why the parser runs in the browser.** The hard part of reading a real calendar is
`DTSTART;TZID=Europe/London:20260330T100000` — turning a wall clock in a named zone into an exact
instant, with that zone's DST rules for that date. The Rust backend has no timezone database, and a
hand-rolled `VTIMEZONE` interpreter is a plausible-looking way to book appointments an hour off twice
a year. Every browser ships the full IANA database via `Intl.DateTimeFormat`, so
`packages/ui/src/calendar/ics.ts` resolves the offset exactly and POSTs unix seconds. The backend
performs no timezone interpretation at all. An event whose `TZID` the browser cannot resolve is
reported as skipped, naming the zone — never booked at a guessed offset.

---

### 1.3 The other direction — one booking, handed to the CLIENT

Sections 1.1 and 1.2 are both about the SHOP's whole schedule.
This one is the opposite shape: ONE appointment, handed to the one person it belongs to, who scans a
QR at the counter (or follows a link) and walks away with it in their own calendar.

Implementation: `stacks/vet/api/src/appointment_share.rs`, which reuses the same RFC 5545 serializer
in `ics.rs` that the feed does. There is deliberately no second `.ics` writer to drift out of step
with the first; what differs is the PROJECTION, for the privacy reason below.

| Surface | What it is |
| --- | --- |
| `POST /appointments/{id}/share` | Operator-gated. Mints the handoff and returns the link + QR URL. |
| `GET /a/{token}` | Public. A self-contained page a phone renders, showing the booking's live state. |
| `GET /a/{token}.ics` | Public. The same booking as a file iOS and Android open natively into "add to calendar". |

The page offers three ways out: the `.ics` download, an **add-to-Google** template link for a client
who lives in a browser, and — because the `.ics` URL is live rather than a stored file — a
`webcal://` **subscribe** option for a client whose calendar can poll it.

**The handoff carries the client's booking and nothing else of the shop's.**
It publishes the service, the slot, the shop, the pet and the groomer.
It does NOT publish the shop's internal `notes` (free text the shop writes for itself, which can say
anything about anyone), the client's own name (they know it, and a leaked link should not name the
person it leaked about), or any other booking.
This is why `to_client_event` is a SEPARATE projection from the feed's `to_ics_event`: the feed is
read by the shop and carries both of those fields, so reusing it here would have handed them to
whoever scanned the QR.

**The QR is built from `DEPLOYMENT_URL`, or it is not drawn at all.**
A QR encoding a host the scanning phone cannot resolve is worse than no QR, because it still looks
like a working link — that defect shipped here once already, in receipt QRs built from a `did:web`
issuer name whose default was RFC-2606 reserved.
Two bases are refused outright: an unset one, and a LOOPBACK one (`localhost`, `127.0.0.0/8`, `::1`),
which is the shipped dev default and resolves on the operator's own laptop and nowhere else.
In both cases the mint returns `qrUrl: null` plus a reason naming the fix, and the portal renders no
QR and shows the reason. For the loopback case the link itself is still returned and labelled,
because on a dev box it is the working one.

`/a/` is proxied alongside `/api` in **both** portals' `vite.config.ts` and `nginx.conf`.
Without that, a deployment pointing `DEPLOYMENT_URL` at the portal origin would send a scanning phone
into the SPA's history fallback — a 200 from a live host serving the operator app's `index.html`,
which reads as working far more convincingly than a dead link.
Those nginx blocks also set `access_log off` for a well-formed token, for the same path-borne-secret
reason as the feed above; the blast radius here is one booking rather than the whole schedule.

**What a downloaded `.ics` cannot do, and what is done about it.**
A file import is a SNAPSHOT. Nothing pushes a later change into a calendar it has been imported into;
iCalendar has no such channel and neither does this service. Three things follow, none of which
claims the copy stays fresh:

- every event carries a stable `UID` and a monotonic `SEQUENCE`, so re-opening the link and re-adding
  SUPERSEDES the client's earlier copy in place rather than duplicating it;
- every event carries `URL:` back to the handoff page, so a stale entry still names the surface that
  states the current answer;
- the `webcal://` option gives real updates, on the client's calendar's own refresh schedule — which
  for Google has historically been hours. The page offers it as a distinct, labelled choice rather
  than implying the download behaves that way.

A **cancelled** booking publishes `STATUS:CANCELLED`, which is what makes re-adding it remove the
event; the page leads with the cancellation instead of offering a bare "add to calendar".
A **deleted** booking publishes the same way, as a tombstone under the same `UID`, using the slot
recorded when the token was minted — a 404 there would leave a subscriber's stale event standing
forever with nothing but a sync error to explain it.

**Daylight saving cannot bite this surface, and that is structural rather than lucky.**
An appointment is stored as an instant; the `.ics` publishes it in UTC and the Google link uses the
UTC basic form with no `ctz` parameter. Nothing re-derives a wall clock, so there is no offset to
resolve and no transition to land on the wrong side of. The page formats the instant in the READER's
own zone with `Intl` (the client may not live in the shop's), and ships a labelled UTC rendering in
the markup for a reader with no JavaScript.
Tests pin the exact published UTC for bookings either side of both transitions in `Europe/London` and
`America/New_York`, with a guard asserting those fixtures genuinely straddle a transition so they
cannot pass vacuously.

**When the store cannot be read, that is its own answer.** The page says it could not check, in those
words, and offers no add-to-calendar affordance it cannot stand behind; the `.ics` answers 503 with
no calendar body at all, because a tombstone there would cancel a booking that may be perfectly live.
This is why the module reads through `Store::try_get_appointment` rather than the `Option`-shaped
form, whose collapsed error would have told a client their booking was gone on the strength of a read
that never happened.

**Sharing is additive and cannot be taken back.**
Every mint issues a NEW token — the portal dialog mints on each open, so opening it three times leaves
three live URLs for one booking — and there is no revoke: each link resolves for 180 days past the
slot.
That is a deliberate difference from the feed, which makes rotation first-class because its URL
exposes the shop's *entire* schedule. This one names a single appointment and carries neither the
client's name nor the shop's notes, so a leaked line costs one booking, while breaking a link a client
is relying on to find their appointment is the larger risk.
If that trade stops holding, revocation belongs here as an explicit action — not as a shorter expiry,
which would silently strand working links.

**Not in scope here.** The handoff has no client-initiated actions: a client cannot confirm, cancel or
reschedule from this page. It states what the shop's book says and hands it over.

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
| **Cloud project + OAuth verification** | A Google Cloud project per deployment, or one shared project with every shop as a user of it. Calendar scopes sit in Google's sensitive/restricted tiers, and an app cannot be published past the unverified test-user cap without going through Google's verification; the restricted tier additionally requires a periodic third-party security assessment. **Which tier `calendar.events` falls in, and what verification currently costs in money and elapsed time, must be checked against Google's own policy** — this is the item most likely to decide whether native Google sync is feasible at all for a self-hosted product, and it is not something to assume from memory. |
| **Token storage + refresh** | Refresh tokens encrypted at rest (the custody seal is the pattern to follow). Access tokens minted per request and cached. A user can revoke the grant at any time, so every call needs a "re-consent required" path that surfaces in the portal. |
| **Webhook channels** | `events.watch` gives push notifications, but channels expire and must be renewed on a schedule, and the endpoint has to be publicly reachable over HTTPS — which a self-hosted shop behind a home router is not. Fallback: poll `events.list` with `syncToken` on a timer, which is what the existing engine already does on demand. |
| **The collection bridge** | Map `store::Appointment` ⇄ Google events, keeping the existing `dogtag.owned` / `dogtag.apptId` / `dogtag.rev` extended-property tagging so echoes stay distinguishable from human edits. |
| **Conflict resolution** | See §4. |

Realistic scope: the client is largely written; storage, refresh, the collection bridge, the webhook
endpoint and its renewal, the portal UI, and conflict resolution are not. OAuth verification is the
item most likely to be the actual blocker for a product that ships to third-party shops, and it is
the first thing to check rather than the last.

### 3.2 Apple Calendar

Apple has no public REST calendar API for third parties. Two-way against iCloud means **CalDAV**
(RFC 4791):

- Authentication is an **app-specific password** the shop generates in their Apple ID settings — a
  long-lived credential DogTag would then store and be responsible for. There is no OAuth flow and
  no per-service scoping to lean on, so **what an app-specific password actually grants access to
  should be established before deciding this is acceptable**; storing one on a shop's behalf is a
  posture question, not an implementation detail.
- Discovery via `PROPFIND` on the principal URL, then the calendar-home set, then each collection.
- Change detection via `sync-collection` (RFC 6578) sync tokens, or `getctag` polling. CalDAV has no
  push, so polling is the mechanism.
- Writes are `PUT`s of whole `VEVENT` bodies with `If-Match` ETags for optimistic concurrency.
- iCloud's CalDAV endpoint is not covered by a published Apple specification, so behaviour has to be
  discovered and re-verified rather than read.

Realistic scope: a CalDAV client is a meaningful piece of work in its own right, and the
credential-handling posture is the part to settle before any of the code.

### 3.3 Luma

- Luma's public API is built around running events and guest lists on a Luma calendar. Whether it
  can serve as a general two-way sync target for an external booking system needs checking against
  its current API docs; ICS subscription is what it documents for bringing an outside calendar in,
  which is the case here.
- API-key auth, so there is no token refresh to build — but key storage is still on us.

Realistic scope: the smallest of the three, and the one where ICS is most likely the intended answer
rather than a workaround.

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

Before starting stage (b), three questions need answers, and none of them are engineering questions:
what Google's current verification tier and cost actually are for the calendar scope we would need;
whether storing an Apple app-specific password on a shop's behalf is an acceptable posture once we
know what it grants; and which of the §4.2 conflict answers the shop actually wants when two people
edit the same booking. The first two are provider facts to go and check, not to assume.
