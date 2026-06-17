# 05 — Calendar Sync & Appointment Booking (DogTag)

Research notes and concrete design guidance for: (a) Google Calendar two-way
sync from a server, (b) platform-as-source-of-truth mirroring, (c) the
appointment booking state machine and the cross-backend "double-entry"
contract between the central backend and self-hosted business backends,
(d) availability computation, and (e) iCal/.ics fallback.

> Context: Vets & groomers run **self-hosted business backends**. Pet owners
> book through the **mobile app**, which talks to the **central backend**. The
> central platform is the **source of truth** for appointments; each business
> can additionally import/sync its **Google Calendar**. The central registry
> knows each business's API URL.

---

## 1. Google Calendar API integration

### 1.1 OAuth 2.0 — web server flow with offline access

Each business backend authorizes once against Google using the **OAuth 2.0
web server flow** and stores a long-lived **refresh token**.

- Flow doc: <https://developers.google.com/identity/protocols/oauth2/web-server>
- General OAuth: <https://developers.google.com/identity/protocols/oauth2>

Key request parameters on the authorization redirect:

- `access_type=offline` — required to receive a **refresh token** so the
  backend can mint new access tokens without user interaction.
- `prompt=consent` — forces re-consent so a refresh token is reliably issued
  (Google only returns a refresh token on first consent unless forced).
- `include_granted_scopes=true` — incremental authorization.
- `state` — CSRF token + correlation to the business record.

Token handling:

- Exchange the `code` at the token endpoint → `{access_token, refresh_token,
  expires_in, scope, token_type}`.
- **Persist the refresh token in secure long-term storage** (encrypted at rest,
  per-business). Access tokens are short-lived (~1h); refresh on demand.
- Note Google's **refresh-token limits** per client/user combination — older
  tokens stop working if you over-issue. Reuse one stored refresh token per
  business calendar connection rather than re-running consent repeatedly.

### 1.2 Scopes

Scope chooser: <https://developers.google.com/workspace/calendar/api/auth>

| Scope | Access | Use for DogTag |
|---|---|---|
| `https://www.googleapis.com/auth/calendar.events.readonly` | Read events only | Import-only businesses (one-way) |
| `https://www.googleapis.com/auth/calendar.events` | View **and edit** events on all the user's calendars | **Default** — two-way sync needs write |
| `https://www.googleapis.com/auth/calendar.readonly` | Read calendars + events | If you also need FreeBusy / calendar list read |
| `https://www.googleapis.com/auth/calendar` | Full: see/edit/share/delete calendars | Avoid unless managing calendars themselves |

**Recommendation:** request `calendar.events` (read+write events) for the
two-way path. FreeBusy works with `calendar.events.readonly`/`calendar.readonly`.
Request the **narrowest** scope that satisfies the feature; users grant
limited, clearly-described scopes more readily.

### 1.3 Events CRUD

API reference: <https://developers.google.com/workspace/calendar/api/v3/reference/events>

- **List**: `GET /calendar/v3/calendars/{calendarId}/events`
  <https://developers.google.com/workspace/calendar/api/v3/reference/events/list>
- **Insert**: `POST /calendar/v3/calendars/{calendarId}/events`
  <https://developers.google.com/workspace/calendar/api/v3/reference/events/insert>
- **Update / Patch**: `PUT|PATCH /calendar/v3/calendars/{calendarId}/events/{eventId}`
- **Delete**: `DELETE /calendar/v3/calendars/{calendarId}/events/{eventId}`
- **Get**: `GET .../events/{eventId}`

Event body essentials: `summary`, `description`, `location`,
`start.dateTime`/`end.dateTime` (RFC3339 + `timeZone`), `attendees[]`,
`status` (`confirmed` / `tentative` / `cancelled`), `extendedProperties`.

### 1.4 Incremental two-way sync via sync tokens

Sync guide: <https://developers.google.com/workspace/calendar/api/guides/sync>

**Initial full sync (once per calendar):**

1. `events.list` with **no** `syncToken`. Use `singleEvents=true` (expand
   recurrences), and a starting window (`timeMin`) if you don't want full
   history.
2. Paginate with `pageToken` until the final page. **`nextSyncToken` appears
   only on the final page** (the page that has no `nextPageToken`).
3. Persist `nextSyncToken` for that calendar.

**Incremental sync (every time after):**

1. `events.list` with `syncToken=<stored>` and the **same query parameters**
   as the initial request (the parameter set on incremental syncs is
   restricted; mismatched params return **400**). Do **not** send `timeMin`/
   `timeMax`/`q`/`updatedMin` alongside `syncToken`.
2. The response returns **only changed events** since the last sync, and
   **always includes deleted entries** (deletions surface as events with
   `status == "cancelled"` — remove them locally).
3. Paginate (each page may carry `nextPageToken`); the final page carries a
   fresh `nextSyncToken` — store it.

**410 GONE handling:** if an incremental request returns **HTTP 410**, the
stored `syncToken` is invalid (expired, ACL changes, several weeks idle, or
Google rotation). **Discard the token, wipe local mirror state for that
calendar, and re-run a full sync** to obtain a new `nextSyncToken`.

### 1.5 Push notifications (watch channels / webhooks)

Push guide: <https://developers.google.com/workspace/calendar/api/guides/push>
Events.watch: <https://developers.google.com/workspace/calendar/api/v3/reference/events/watch>

Set up a channel per watched calendar:

```
POST https://www.googleapis.com/calendar/v3/calendars/{calendarId}/events/watch
{
  "id":      "<uuid>",                       // unique channel id in project, <=64 chars
  "type":    "web_hook",
  "address": "https://hooks.business.example/gcal",   // HTTPS, valid CA-signed cert
  "token":   "biz=123&cal=primary",          // opaque verification string, <=256 chars
  "expiration": 1718600000000                // optional Unix ms; capped by Google
}
```

Notification mechanics:

- Google POSTs to `address` whenever the resource changes. The **notification
  has no body** — it only tells you *something* changed via headers; you must
  then run an **incremental `events.list` with the stored `syncToken`** to pull
  the actual deltas.
- Headers: `X-Goog-Channel-ID`, `X-Goog-Resource-ID` (stable),
  `X-Goog-Resource-State` (`sync` on channel creation; `exists` on change;
  `not_exists`), `X-Goog-Message-Number` (sequence), `X-Goog-Channel-Token`
  (your `token`, verify it), `X-Goog-Channel-Expiration`.
- **Expiration / renewal:** channels last **up to ~1 week**; there is **no
  auto-renew**. Run a cron that re-`watch`es before expiry and records the new
  channel id/resource id/expiration. Notifications are **not 100% reliable** —
  always keep a **periodic polling fallback** (e.g. incremental sync every
  N minutes) so dropped notifications self-heal.
- **Stop a channel:** `POST https://www.googleapis.com/calendar/v3/channels/stop`
  with `{"id": "<channelId>", "resourceId": "<resourceId>"}`.
- The webhook domain must be verified / served over HTTPS with a valid cert.

### 1.6 FreeBusy query (availability)

Reference: <https://developers.google.com/workspace/calendar/api/v3/reference/freebusy/query>

```
POST https://www.googleapis.com/calendar/v3/freeBusy
{
  "timeMin": "2026-06-18T00:00:00Z",
  "timeMax": "2026-06-25T00:00:00Z",
  "timeZone": "Europe/Berlin",
  "items": [ { "id": "primary" }, { "id": "groomer-room-2@group.calendar.google.com" } ]
}
```

Response: `calendars[id].busy[]` = list of `{start, end}` busy intervals
(also `errors[]` per calendar). `calendarExpansionMax` ≤ 50.

FreeBusy is preferable to reading raw events for the availability path: it
returns only busy intervals, costs less, and needs only read scope.

---

## 2. Two-way sync design (platform = source of truth, Google = mirror)

The platform appointment is authoritative. Each appointment is **mirrored**
into the business's Google Calendar as an event the platform owns.

### 2.1 Echo-loop avoidance

The danger: platform writes an event to Google → Google push fires → platform
re-ingests its own write as if external → loops / duplicates.

Defenses (layer all three):

1. **Tag platform-owned events with `extendedProperties.private`.** On
   insert/update set, e.g.:
   ```json
   "extendedProperties": {
     "private": {
       "dogtag.owned": "1",
       "dogtag.apptId": "appt_01HZX...",
       "dogtag.rev": "7"
     }
   }
   ```
   Extended properties limits: key ≤ 44 chars, value ≤ 1024 chars, ≤ 300
   props / 32kB per event.
   Doc: <https://developers.google.com/workspace/calendar/api/guides/extended-properties>
   - On incremental ingest, if `private["dogtag.owned"]=="1"` **and**
     `dogtag.rev` ≤ the rev the platform last wrote → it's our own echo,
     **skip**. If `dogtag.rev` is *less* than ours we ignore (stale echo); if
     a human edited it in Google, treat per conflict rules (2.3).
   - You can query these back via `events.list?privateExtendedProperty=dogtag.owned%3D1`.

2. **Mapping table (id ↔ id)** so an external event already mapped to a
   platform appointment never creates a second appointment (see 2.4).

3. **Revision/etag suppression.** Persist the Google `etag` and a monotonic
   `dogtag.rev`. When the platform pushes a change it bumps `rev` and records
   the resulting `etag`. An inbound change whose `etag` matches the one we just
   wrote is an echo → ignore.

### 2.2 Direction of changes

- **Platform → Google (mirror, default):** create/update/delete events for
  appointments. Platform owns title/time/status.
- **Google → Platform (import):** events created by the *business* directly in
  Google (NOT tagged `dogtag.owned`) become **external busy blocks** that
  reduce availability but are **not** bookable appointments. They never round-
  trip back as platform appointments.

### 2.3 Conflicts, reschedules, deletions

- **Reschedule (platform side):** bump `rev`, `PATCH` the Google event
  start/end. Push echo suppressed via etag/rev.
- **Reschedule of a platform-owned event done in Google (human edit):** policy
  choice — recommended **platform-wins**: on ingest, detect divergence
  (Google time ≠ platform time) for a `dogtag.owned` event and **overwrite
  Google back** to platform truth (and optionally flag for staff review).
  Alternatively **last-writer-wins** by comparing `updated` timestamps.
- **Deletion (platform):** set appointment `cancelled` → `DELETE` (or set
  Google `status:"cancelled"`) the mapped event.
- **Deletion in Google of a platform-owned event:** appears in incremental
  sync as `status:"cancelled"`. Policy: re-create it (platform-wins) or mark
  the appointment `cancelled` and notify staff. Recommended: **re-create +
  alert**, since platform is source of truth.
- **External (untagged) event deleted/changed in Google:** just update the
  busy-block mirror; no appointment impact.

### 2.4 Mapping table schema

```sql
-- One row per (business calendar connection)
CREATE TABLE gcal_connection (
  id              UUID PRIMARY KEY,
  business_id     UUID NOT NULL,
  google_account  TEXT NOT NULL,
  calendar_id     TEXT NOT NULL,          -- e.g. "primary"
  refresh_token   BYTEA NOT NULL,         -- encrypted
  scopes          TEXT NOT NULL,
  sync_token      TEXT,                   -- nextSyncToken; null => need full sync
  channel_id      TEXT,                   -- current watch channel
  channel_res_id  TEXT,
  channel_expires TIMESTAMPTZ,
  last_full_sync  TIMESTAMPTZ,
  UNIQUE (business_id, google_account, calendar_id)
);

-- platform appointment <-> google event mapping (echo-loop guard)
CREATE TABLE gcal_event_map (
  appointment_id  UUID NOT NULL REFERENCES appointment(id),
  connection_id   UUID NOT NULL REFERENCES gcal_connection(id),
  google_event_id TEXT NOT NULL,
  google_etag     TEXT,                   -- last etag we wrote/observed
  dogtag_rev      INT NOT NULL DEFAULT 0, -- last rev we pushed
  direction       TEXT NOT NULL,          -- 'mirror' (platform-owned) | 'import' (external busy)
  last_synced_at  TIMESTAMPTZ,
  PRIMARY KEY (connection_id, google_event_id)
);
CREATE UNIQUE INDEX ON gcal_event_map (appointment_id, connection_id)
  WHERE direction = 'mirror';
```

Sync loop (per connection): on watch ping OR poll tick → incremental
`events.list(syncToken)` → for each event: look up `gcal_event_map`. If
`dogtag.owned` and (etag matches last written OR rev ≤ ours) → echo, skip.
Else apply per direction + conflict policy. On 410 → wipe `sync_token`, full
resync.

---

## 3. Appointment booking state machine + cross-backend "double-entry"

### 3.1 State machine

```
                 owner requests slot
   (none) ─────────────────────────────▶ REQUESTED
                                          │   │
                  business approves       │   │ business declines / owner cancels
                                          ▼   ▼
                                      CONFIRMED   DECLINED
                                          │
            ┌─────────────────────────────┼───────────────────────────┐
            │ owner/biz reschedule         │ cancel                    │ time passes
            ▼                              ▼                           ▼
        CONFIRMED (new time)           CANCELLED              COMPLETED | NO_SHOW
```

States: `REQUESTED → {CONFIRMED | DECLINED}`; `CONFIRMED → {RESCHEDULED(stays
CONFIRMED) | CANCELLED | COMPLETED | NO_SHOW}`. Terminal: `DECLINED`,
`CANCELLED`, `COMPLETED`, `NO_SHOW`. Reschedule is a CONFIRMED→CONFIRMED
transition that changes `start/end` and bumps `rev`.

Auto-confirm is a per-business policy flag (skip REQUESTED, go straight to
CONFIRMED for instant-book slots).

### 3.2 Double-entry: who owns the record, and how both sides see it

Requirement: an appointment must be **consistently visible to both** the pet
owner (mobile → central backend) and the business (self-hosted backend).

**Ownership model — central backend is the source of truth (system of
record).** The self-hosted business backend keeps a **replica** of each of its
appointments. The central registry stores each business's
`business_api_url` + shared credentials. Sync between the two is a
**bidirectional, idempotent, eventually-consistent replication** driven by
signed HTTP calls + callbacks. Both sides persist the same `appointment_id`
(allocated by central) so the record is a single logical entity with two
physical copies.

Why central-as-truth (not the self-hosted side): the mobile client can always
reach central; the self-hosted backend may be offline/unreachable. Central can
queue and retry pushes; the owner always gets a coherent view.

**Lifecycle ownership of transitions:**
- `REQUESTED` is created at **central** (owner initiates) → pushed to business.
- `CONFIRMED / DECLINED / COMPLETED / NO_SHOW` are **business-driven** →
  business calls back to central (or central polls). Central records and pushes
  to mobile (push notification).
- `CANCELLED` / `RESCHEDULE` can originate on **either** side → whoever
  receives it persists locally and propagates to the other; central reconciles.

Each appointment carries a monotonically increasing `rev` (lamport-style
version) owned by central. Every propagation includes `rev`; the receiver
applies a change only if `rev` is newer than what it holds, making replays /
out-of-order delivery safe.

### 3.3 API contract: central ↔ business

All calls authenticated (per-business HMAC-signed JWT or mTLS), all mutating
calls **idempotent** via `Idempotency-Key` header (and the `appointment_id`
+ `rev` pair). Both directions speak the same payload shape.

**A. Central → Business (push appointment intents)**

```
PUT  /v1/appointments/{appointmentId}        # upsert (create or update by rev)
     Headers: Authorization, Idempotency-Key: <uuid>, X-DogTag-Signature
     Body:
     {
       "appointmentId": "appt_01HZX...",
       "rev": 4,
       "state": "REQUESTED",                  # or CONFIRMED/CANCELLED/...
       "businessId": "biz_123",
       "service": { "code": "grooming.full", "durationMin": 60 },
       "start": "2026-06-20T09:00:00+02:00",
       "end":   "2026-06-20T10:00:00+02:00",
       "pet":   { "id":"pet_9", "name":"Rex", "species":"dog" },
       "owner": { "id":"usr_4", "displayName":"K. Wu", "contact":"masked" },
       "notes": "nervous around clippers"
     }
     → 200 {"appointmentId","rev","state","accepted":true}
        409 {"error":"stale_rev","currentRev":5}     # business has newer; central reconciles
        202 {"queued":true}                          # accepted async

POST /v1/appointments/{appointmentId}/cancel  { "rev":N, "reason":"...", "actor":"owner" }
POST /v1/appointments/{appointmentId}/reschedule { "rev":N, "start":..., "end":... }
```

**B. Business → Central (callbacks / lifecycle events)**

The business calls central's webhook endpoint for transitions it owns. Central
URL is fixed and known to all businesses.

```
POST https://api.dogtag.io/v1/businesses/{businessId}/appointment-events
     Headers: Authorization (business token), Idempotency-Key, X-DogTag-Signature
     Body:
     {
       "appointmentId": "appt_01HZX...",
       "rev": 5,                              # business proposes next rev; central is arbiter
       "event": "CONFIRMED",                  # CONFIRMED|DECLINED|RESCHEDULED|CANCELLED|COMPLETED|NO_SHOW
       "occurredAt": "2026-06-18T12:00:00Z",
       "start": "...", "end": "...",          # present for RESCHEDULED
       "reason": "fully booked"               # present for DECLINED/CANCELLED
     }
     → 200 {"appointmentId","rev": <central-assigned>,"state":"CONFIRMED"}
        409 {"error":"conflict","currentRev":6,"state":"CANCELLED"}  # owner already cancelled
```

**C. Reconciliation / catch-up (heals missed deliveries)**

```
GET  /v1/appointments?updatedSince=<ts>&cursor=<c>   # both sides expose this
     → { "items":[ {appointmentId,rev,state,...} ], "nextCursor":"..." }
```
Both backends run a periodic pull against the other's `updatedSince` feed and
fast-forward any record where the remote `rev` is newer. This is the safety
net behind the real-time callbacks (mirrors the watch+poll pattern from §1.5).

**Idempotency & ordering rules**
- `Idempotency-Key` dedupes retries (store key→result for 24h+).
- Apply a change only if incoming `rev` > stored `rev`; otherwise return
  current state (no-op). Central is the **rev arbiter** — when business
  proposes a rev, central assigns the canonical rev and echoes it back.
- Conflicts (e.g. owner cancels while business confirms): central resolves by
  precedence (CANCELLED/DECLINED are terminal and win over CONFIRMED) and
  pushes the resolved state to both sides via PUT (central→business) and the
  mobile push channel (central→owner).

### 3.4 How add / remove / reschedule stays in sync (worked flows)

- **Add (owner books):** mobile → central creates `REQUESTED rev0` → central
  `PUT` to business. Business shows it, staff `CONFIRMED` → callback to central
  → central sets `CONFIRMED rev1`, pushes mobile notification, and the business
  backend mirrors to Google Calendar (§2, `dogtag.owned`).
- **Remove (cancel):** whichever side initiates persists `CANCELLED` and
  propagates; the other applies if rev newer; Google mirror event deleted.
- **Reschedule:** new `start/end`, `rev++`, propagated both ways; Google event
  PATCHed. Echo suppressed by etag/rev (§2.1).

---

## 4. Availability computation

Goal: produce bookable slots for the mobile app from three inputs.

```
bookable_slots(business, service, dateRange) =
    grid(working_hours, service.duration, slot_step)         # candidate slots
  − platform_appointments(CONFIRMED|REQUESTED-held)          # central truth
  − google_busy(FreeBusy over business calendars)            # external busy
  − resource/staff capacity constraints
```

Steps:

1. **Working hours**: per-business, per-staff/resource weekly schedule +
   holiday/closure overrides → generate candidate slots of `service.duration`
   stepped by `slot_step` (e.g. 15 min), within open hours.
2. **Subtract platform appointments**: remove slots overlapping CONFIRMED (and
   short-lived holds for REQUESTED) appointments from central's own DB —
   no Google call needed; central is source of truth.
3. **Subtract Google busy**: one `freeBusy.query` per business spanning
   `dateRange` over the relevant calendars (`calendarExpansionMax ≤ 50`).
   Subtract returned `busy[]` intervals. Cache busy results briefly (the watch
   channel invalidates the cache on change).
4. **Capacity**: if a resource (groom bay, vet) handles N concurrent, a slot is
   bookable while concurrent count < N.
5. Expose remaining slots via central API to the mobile app
   (`GET /v1/businesses/{id}/availability?service=...&from=...&to=...`),
   normalized to the business time zone (and converted client-side).

To avoid double-book races: hold a slot (soft lock with TTL) at REQUESTED time,
confirm on CONFIRMED, release on DECLINED/timeout.

---

## 5. iCal / .ics fallback (non-Google calendars)

For businesses on Apple/Outlook/other calendars without Google OAuth:

- **One-way import (read):** business provides a **secret .ics feed URL**
  (CalDAV/webcal). Central/business backend periodically `GET`s it, parses
  `VEVENT`s (RFC 5545) into **external busy blocks** feeding §4. `UID` is the
  stable key; `LAST-MODIFIED`/`SEQUENCE` drive change detection; `STATUS:
  CANCELLED` = removal. Polling only (no push) — poll every N minutes.
- **One-way export (mirror out):** publish each business's DogTag appointments
  as a **subscribable .ics feed** the business adds to its calendar app, so
  appointments appear there read-only.
- **Two-way** is only practical via **CalDAV** (`PUT`/`DELETE` to the
  collection) where supported; otherwise treat .ics as one-way and keep central
  authoritative.

This is intentionally lower-fidelity than Google: no real-time push, no
extended-property tagging — so dedupe imported events by `UID` and never treat
them as platform appointments.

---

## Source URLs

- OAuth web server flow: <https://developers.google.com/identity/protocols/oauth2/web-server>
- OAuth overview: <https://developers.google.com/identity/protocols/oauth2>
- Choose scopes: <https://developers.google.com/workspace/calendar/api/auth>
- Events reference: <https://developers.google.com/workspace/calendar/api/v3/reference/events>
- Events: list: <https://developers.google.com/workspace/calendar/api/v3/reference/events/list>
- Events: insert: <https://developers.google.com/workspace/calendar/api/v3/reference/events/insert>
- Events: watch: <https://developers.google.com/workspace/calendar/api/v3/reference/events/watch>
- Sync guide (syncToken, 410, full resync): <https://developers.google.com/workspace/calendar/api/guides/sync>
- Push notifications guide: <https://developers.google.com/workspace/calendar/api/guides/push>
- Extended properties: <https://developers.google.com/workspace/calendar/api/guides/extended-properties>
- FreeBusy: query: <https://developers.google.com/workspace/calendar/api/v3/reference/freebusy/query>
- iCalendar RFC 5545: <https://datatracker.ietf.org/doc/html/rfc5545>
