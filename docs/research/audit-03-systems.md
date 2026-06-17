# Audit 03 — Systems Architecture & Standards Compliance

> Scope: overall system design — internal consistency, completeness, security, real-world
> correctness. **Not** in scope: smart-contract internals, Merkle/keccak crypto correctness
> (covered by other auditors). Inputs audited: `architecture.md`, `implementation.md`,
> `research/01-data-standards.md`, `research/04-custody-qr.md`, `research/05-calendar-appointments.md`.
> Auditor: systems-architecture & standards-compliance. Date: 2026-06-17.

Severity legend: **Critical** (auth bypass / data loss / spoof / standards-illegal),
**High** (exploitable or breaks a core flow), **Medium** (correctness gap, recoverable),
**Low** (polish / future-proofing), **Info** (note).

---

## 0. Executive verdict

The design is coherent and unusually complete for a v1 spec: the trust triangle (integrity /
on-chain status / DNS identity) is sound, key custody follows current best practice, and the
appointment double-entry contract is well thought through. However there are **two Critical
auth gaps** (the user→business central `GET /share/{ref}` endpoint never validates audience
or path-binding the way the business side does; and the business→central `appointment-events`
callback has no per-business identity binding in the documented HMAC scheme — any whitelisted
business can post events for *another* business's appointment). There are also several **High**
issues around DNS identity (the QR origin is operator-controlled, so the "identity" pillar can
be satisfied by a typosquatting issuer that controls both the domain and the contract address it
publishes), schema-validator gaps vs. the regulation it claims to enforce, and a split-brain
window in appointment reconciliation. None are unfixable; all have concrete fixes below.

---

## 1. Internal consistency: architecture.md ↔ implementation.md

### 1.1 [Medium] JWT `exp` value disagrees across docs
`architecture.md §7` says `exp ~2–5 min`. `implementation.md §3.4` hardcodes `exp: now+180s`
(3 min). `research/04 §4` claims label "~5 minutes" in the JSON sample but text says "2–5 min".
Not contradictory in spirit, but the normative value should be pinned once. **Fix:** state the
canonical TTL (e.g. 180 s) in one place and reference it.

### 1.2 [High] Genesis endpoint paths differ between research and implementation
`research/04 §3` specifies `POST /admin/genesis/start`, `/admin/genesis/confirm`,
`/admin/unlock`, `/admin/accounts` — all under an **admin-auth + localhost/TLS-only** namespace.
`implementation.md §3.1` drops the `/admin` prefix (`POST /genesis/start`, `/unlock`,
`/accounts`) and **says nothing about who may call them**. Genesis/unlock/derive-account are the
most sensitive endpoints in the whole business backend (they mint and expose the signing seed's
account list and gate unlock). If they are mounted on the same public API surface as
`/records/{id}` with no auth gate, **anyone who can reach the API port (which is publicly
exposed per §2/§7) can call `POST /genesis/start` on an uninitialised box, or hammer
`POST /unlock` for passphrase brute force.** `genesis/start` does return `409` once initialised,
but pre-init it is wide open. **Fix:** (a) restore the `/admin` namespace; (b) require an
operator/admin session (the portal login) for all custody endpoints; (c) bind genesis/unlock to
localhost or an authenticated admin session only; (d) rate-limit `/unlock`. This is also a
completeness gap — see §9.

### 1.3 [Medium] `recordType` representation: `bytes32` hash vs human string
Contracts store `recordType` as `bytes32` and `Deploy.s.sol` initialises clones with
`keccak("VACCINATION")` / `keccak("DOG_PROFILE")`. But the wrapped-doc `issuer.recordType`
(arch §3.1) and the schema table (§3.6) use the human string `"VACCINATION"`. The verifier never
reads on-chain `recordType` (it only calls `isValid(root)` on `documentStore`), so this is not a
break — but the mapping `string → keccak(string) → clone address` is **implicit and
undocumented**. **Fix:** document that `issuer.recordType` is the human label and that the
on-chain `recordType` is `keccak256(label)`; ship the label→address map in the `businesses`
registry `documentStores{recordType→addr}` (already present in §9.1) so the verifier/clients
never have to compute it.

### 1.4 [Medium] `DogTagSBT.mint` uses `dogTagId` as a caller-supplied tokenId
`mint(address to, uint256 dogTagId, bytes32 root)` lets the (whitelisted) central signer pick
the tokenId. Architecture §4.2 says "microchip uniqueness enforced off-chain by central backend
before mint; one SBT per microchip." But **nothing maps microchip→tokenId on-chain**, and
`dogTagId` is an arbitrary `uint256`, not derived from the microchip. If the central backend's
off-chain uniqueness check has a bug or race (two `POST /v1/pets/{id}/mint` for the same chip),
two SBTs can be minted. **Fix:** either derive `dogTagId = uint256(keccak256(microchipId))` so
the chain enforces uniqueness via `_safeMint`'s existing-token revert, or add an on-chain
`mapping(bytes32 microchipHash => uint256 tokenId)` guard in `DogTagSBT.mint`. The former is
cleaner and removes a trust assumption on the central backend.

### 1.5 [Low] `DogTagSBT` ↔ `IssuerRegistry` whitelist conflation
`DogTagSBT.mint` is gated by the **same** `IssuerRegistry.isWhitelisted` as vaccination issuance.
That means any address whitelisted to issue *vaccinations* could also `mint` SBTs and
`setProfileRoot` on **any** pet (`setProfileRoot(id, root)` has no per-token ownership check).
Architecture §4.2 implies only "DogTag-protocol issuers" mint, but the contract does not
distinguish protocol-signers from vet-signers — it's one flat whitelist. **Fix:** use a separate
role (`PROFILE_ISSUER_ROLE`) for SBT mint/profile, or gate `setProfileRoot` so only the token's
original profile issuer (or admin) can update it. As written, a compromised *groomer* key could
rewrite every pet's profile root. (Borderline High; rated Low only because mint/profile is
described as protocol-only and the deploy script whitelists a dedicated `protocolSignerForProfiles`
— but the contract does not enforce that separation.)

### 1.6 [Low] Field name drift in the rabies block
`research/01 §2` table uses `vaccineManufacturer` and `batchNumber`; `implementation.md §1.6`
validator requires `manufacturer` and `batchLotNumber`; arch §3.6 says "manufacturer, batch/lot
number". Three names for two fields. **Fix:** pin canonical field names in the SDK schema and
the test vectors.

### 1.7 [Info] Port / address / contract-set consistency — OK
Ports (admin 39741/39742/39743, vet 41873/41874/41875, groomer 43617/43618/43619) match across
arch §2, impl §7, and the per-stack `.env`/compose. Contract set (DogTagSBT, IssuerRegistry,
DogTagIssuer, DogTagIssuerFactory) is consistent across arch §4 and impl §2. chainId 135 / RPC /
explorer consistent. No dangling contract references found. The `share` deep-link format
`https://<host>/r?t=<jwt>&i=<recordId>` is consistent across arch §4.6/§7, impl §3.4, and
research/04 §5.

---

## 2. Key custody & JWT / QR

### 2.1 [Critical] Central `GET /share/{ref}` (user→business) lacks the path/audience binding the business side has
The two QR directions are **not symmetric** in their security checks, and the weaker one is the
one exposing the *owner's* PII.

- Business→user (`impl §3.4 GET /records/{id}`): correctly enforces `claims.sub == id`,
  `scope == read:record`, **and** one-time `jti`. Good.
- User→business (`impl §4.1 GET /share/{ref}` + `§3.5 /import/pull`): the spec says only
  "Bearer<jwt>" and "mint one-time JWT (aud dogtag-business)". It does **not** state that the
  central handler asserts `claims.sub == ref`, `claims.aud == "dogtag-business"`, scope, or `jti`
  consumption. Without `sub==ref` binding, a JWT minted to share credential A can be replayed
  against `/share/B` (any credential the user owns) if `ref` is not bound into the token.
  Without `aud` enforcement, a `dogtag-mobile` token (which the *same* central backend also mints
  for other purposes) could be accepted here → **audience confusion**, the exact attack arch §7
  introduced two audiences to prevent.

**Fix:** the central `/share/{ref}` handler MUST mirror the business-side checks verbatim:
`sub == ref`, `aud == "dogtag-business"`, `scope`, one-time `jti`, leeway 30 s, `iss == central`.
Document it in impl §4.1 with the same explicit asserts as §3.4. Rated Critical because it leaks
owner-controlled credential data (owner name/address fields live in EU/CDC subjects) on an
under-specified, replayable token.

### 2.2 [High] `aud` is not enough to prevent cross-deployment token replay; `iss`/origin must be pinned
A business-issued record JWT has `iss = that deployment URL` and `aud = dogtag-mobile`. The
mobile app derives the API base **from the QR origin** and sends the token there. But the app
must also verify, when it later *re-uses* or caches tokens, that it only ever sends a token whose
`iss`/origin matches the host it's calling — otherwise a malicious vet could embed in its QR a
token whose origin points at a *different* (victim) deployment and trick the app into leaking the
bearer token to the wrong host. The arch says "origin is the API base by construction" but does
not state the app validates `origin == iss` before sending the Bearer. **Fix:** mobile MUST
check the QR's origin equals the JWT `iss` claim before issuing the authenticated `GET`. Cheap,
closes a token-exfiltration vector.

### 2.3 [High] `/unlock` brute-force & no lockout; passphrase is the whole security of the seed at rest
The age blob is scrypt-protected, but `POST /unlock {passphrase}` is an online oracle. Nothing in
the spec rate-limits or locks out repeated `/unlock` attempts, and (per §1.2) the endpoint's auth
is unspecified. A weak operator passphrase + an exposed API port = offline-equivalent online
brute force. **Fix:** require admin-session auth on `/unlock`, add exponential backoff /
lockout, log attempts, and enforce a passphrase strength policy at `genesis/confirm`. Also
consider that age scrypt work factor must be set high (research/04 recommends keystore
`n=2^18`; the age scrypt mode should be tuned to a comparable cost) — pin the age work factor in
the spec.

### 2.4 [Medium] `jti` store on Mongo `SETNX`-equivalent: atomicity must be guaranteed
`impl §3.4` says `consume_jti` = "SETNX/delete; 401 if already used". On Redis `SET NX EX` is
atomic. On Mongo, the equivalent is an `insert` with a unique index on `jti` (catch duplicate-key
= replay) **not** a read-then-write (which races under concurrent scans of the same QR). The spec
lists `jwt_jti` as "(or Redis)". **Fix:** mandate a unique-index insert (Mongo) or `SET NX`
(Redis); explicitly forbid read-check-then-write. Otherwise two simultaneous fetches of the same
one-time QR both succeed.

### 2.5 [Medium] Genesis `confirm` idempotency & seed-stash lifetime
`research/04 §3` ties `genesis/confirm` idempotency to a `genesis_token`; `impl §3.1` drops the
token and just "verify typed words match stash". If `genesis/start` is called twice (or the
in-memory stash is lost on a crash between start and confirm) the state machine can wedge in
`PENDING_BACKUP` with no recovery path, or a second `start` could overwrite the stash after the
operator wrote down the first mnemonic. **Fix:** (a) make `start` refuse if `PENDING_BACKUP`
(return the same challenge, not a new mnemonic) OR explicitly allow regen with a clear "previous
words are void" response; (b) carry the `genesis_token` from research/04 for confirm idempotency;
(c) define recovery from a crashed `PENDING_BACKUP` (must restart genesis since nothing was
persisted — document it).

### 2.6 [Info] Custody at-rest / in-memory model — sound
24-word BIP39 / OsRng, age-encrypted seed as source of truth, `secrecy::SecretBox` + `zeroize` +
`mlock` + core-dump disable, separate JWT key from chain key, unlock precedence TTY > file > env
— all match current best practice (research/04 §2). Multi-account derivation
(`m/44'/60'/0'/0/{n}`, store public address only) is correct. No findings.

---

## 3. Cross-backend appointment sync

### 3.1 [Critical] Business→central `appointment-events` callback: no per-business ownership binding on the appointment
`POST /v1/businesses/{businessId}/appointment-events {appointmentId, rev, event, occurredAt}`
is HMAC-verified, but the documented check is only "HMAC verify" (impl §4.4). Two gaps:

1. **Cross-tenant event injection.** Nothing in the spec asserts that `appointmentId` actually
   *belongs to* `{businessId}`. A whitelisted/registered business that knows or guesses another
   business's `appointmentId` (UUIDs, but they flow through the owner's mobile app and could leak)
   could POST a `CONFIRMED`/`COMPLETED`/`NO_SHOW` event against a *competitor's* appointment.
   Since "terminal states win," a malicious `COMPLETED`/`NO_SHOW` is destructive.
2. **HMAC key scoping.** The shared secret is per-business (`hmacKeyId`), but if central only
   checks "a valid HMAC from *some* business," not "the HMAC of *this* `businessId`," any business
   can sign for any path. **Fix:** central MUST (a) resolve the HMAC key by the path `businessId`
   and verify the signature against *that* key; (b) assert `appointments[appointmentId].businessId
   == path businessId` before applying any event; reject otherwise. This is the appointment
   analog of the JWT `sub==path` rule and is currently missing.

### 3.2 [High] Replay of appointment callbacks within the idempotency window
Idempotency-Key dedupes *retries*, but the body also carries `rev`/`event`. The terminal-wins +
apply-if-newer rule protects against stale revs, but a replayed `appointment-events` POST with a
**fresh** Idempotency-Key and a plausibly-newer rev (business proposes rev; "central is arbiter"
but still consumes the proposed event) could re-drive a state transition. The HMAC covers the
body, so a pure replay is caught by idempotency, but the spec doesn't say the HMAC includes a
timestamp/nonce to bound replay outside the 24h idempotency window. **Fix:** include
`occurredAt` (already present) **and** a signed timestamp in the HMAC base string, reject
callbacks older than a few minutes, and key idempotency on `(businessId, appointmentId, event,
occurredAt)` not a client-chosen UUID alone.

### 3.3 [High] Split-brain / lost-update window on simultaneous owner-cancel vs business-confirm
`research/05 §3.3` resolves "owner cancels while business confirms" via terminal-precedence
(CANCELLED wins). Good. But the **rev arbitration is ambiguous when both sides bump
concurrently**: business proposes `rev=5 CONFIRMED` via callback at the same instant the owner
cancels and central assigns `rev=5 CANCELLED`. Both are "rev 5." apply-if-*newer* (strictly `>`)
means whichever the business backend receives second is *rejected as not-newer*, so the business
replica can be stuck CONFIRMED while central is CANCELLED until a catch-up pull. During that
window the business mirrors a CONFIRMED event to Google and notifies staff for an appointment the
owner cancelled. **Fix:** central is the sole rev allocator — businesses must **never** assign a
rev; the callback should carry no rev (or a client-hint only), central assigns the next rev
monotonically and the PUT-back always wins ties because it has the strictly-higher central rev.
Make apply-if-newer `>=` only for central→business PUTs carrying the canonical rev, `>` elsewhere.
Tighten the contract so there is exactly one writer of `rev`.

### 3.4 [Medium] Catch-up pull `updatedSince` clock dependency & missed deletions
`GET /v1/appointments?updatedSince=<ts>` relies on synchronized clocks and on every state change
touching `updatedAt`. Cancellations/terminal transitions must update the timestamp or they won't
appear in the catch-up feed (cf. the Google-sync lesson that deletions must surface). Also
`updatedSince` is exclusive/inclusive-ambiguous → an event exactly at the boundary can be missed
or double-applied (idempotency saves the latter). **Fix:** use a monotonic server cursor (the
`rev` or an opaque sequence) for the catch-up feed instead of wall-clock `updatedSince`, and
include terminal transitions in the feed.

### 3.5 [Medium] Soft slot-hold has no documented authority or expiry contract
Availability subtracts "soft slot holds … during the request window" (arch §8.3, research/05 §4).
But holds live where? If central holds (it computes availability) but the business is the
capacity authority, two owners hitting two different… actually both go through central, so
central can hold — fine. But the **hold TTL, release-on-decline, and what happens if the business
declines after the hold expired and the slot was rebooked** are unspecified. **Fix:** define hold
TTL (e.g. = REQUESTED timeout), release on DECLINED/CANCELLED/timeout, and that a CONFIRMED
converts the hold to a firm booking atomically.

### 3.6 [Info] Idempotency + rev + terminal-wins core — sound
The double-entry model (central source of truth, business idempotent replica keyed by
`appointmentId`+`rev`, catch-up pulls as the safety net behind callbacks) is a good design and
mirrors the Google watch+poll pattern correctly. Findings above are hardening, not redesign.

---

## 4. Google Calendar two-way sync

### 4.1 [Medium] Watch-channel webhook has a verifiable token but no body-auth; ingest must not trust the ping
research/05 §1.5 correctly notes the notification has **no body** and you must verify
`X-Goog-Channel-Token`. The arch/impl (§8.1, §3.6) reduce this to "a ping just triggers an
incremental list" but don't restate token verification. An attacker who learns the webhook URL
can spam pings (DoS / quota burn) since the endpoint is public HTTPS. **Fix:** verify
`X-Goog-Channel-Token` and `X-Goog-Channel-ID` against the stored channel on every ping; drop
unknown channels; rate-limit. (The actual data only ever comes from the authenticated
`events.list`, so there's no data-injection risk — this is DoS/quota hardening.)

### 4.2 [Medium] Echo-suppression `rev ≤ ours` rule can drop a legitimate human edit
research/05 §2.1 rule: if `dogtag.owned==1` and `dogtag.rev ≤ last-written rev → skip as echo`.
But a human who edits a platform-owned event in Google **does not change** `dogtag.rev` (it's our
private property), so the edited event still carries the old rev ≤ ours and is **skipped as an
echo** — the human reschedule is silently lost (until platform-wins overwrites it, which §2.3 says
to do based on *time divergence*, not rev). The two rules can conflict: echo-skip-by-rev fires
before the divergence check. **Fix:** order the checks — first compare `etag` (echo iff etag ==
last-written etag); only if etag differs, it's a real external edit → apply conflict policy
(platform-wins overwrite + staff flag). Do **not** use `rev ≤ ours` alone to classify echoes,
because the human edit preserves our rev. Use etag as the primary echo discriminator (the design
already stores it).

### 4.3 [Low] 410 full-resync wipes mirror including platform-owned mappings
On 410, research/05 §1.4 / impl §3.6 say "wipe local mirror state for that calendar, full
resync." If `gcal_event_map` rows for *platform-owned* appointments are wiped, the platform loses
the event-id↔appointment binding and a full resync re-ingests its own events as **untagged?** —
no, they carry `dogtag.owned`/`apptId` in extendedProperties, so they can be re-bound by
`apptId`. **Fix:** on 410, do NOT delete `gcal_event_map`; instead re-fetch and re-bind by
`extendedProperties.private["dogtag.apptId"]`. Only the `sync_token` should be discarded. The
word "wipe mirror" is dangerous if taken literally for the mapping table.

### 4.4 [Low] No `singleEvents`/recurring-event handling in the appointment mirror
research/05 §1.4 uses `singleEvents=true` for the busy-block import path, good. But appointments
mirrored *out* are single events — fine. The risk is an external **recurring** busy block: each
instance must reduce availability. Ensure FreeBusy (which expands) is the availability source, not
raw event listing. The design already prefers FreeBusy for availability (§1.6) — just confirm the
busy-block ingest path doesn't double-count. Info/Low.

---

## 5. DNS identity verification

### 5.1 [High] DNS binding is operator-controlled end-to-end → it authenticates "the operator controls this domain + names this contract," not "this is a legitimate vet"
The identity pillar checks that `issuer.domain`'s TXT names `issuer.documentStore`. But in the
self-hosted model the **operator controls the domain, the contract address, the TXT record, AND
the QR they hand you**. So the DNS pillar proves only *internal consistency* (the domain and the
contract agree), not *external legitimacy*. The real legitimacy gate is **on-chain whitelisting**
(`IssuerRegistry`), which is correct. The risk is that a user/verifier sees three green pillars
and concludes "trusted vet" when in fact a typosquatting domain (`vet-seaport.example` vs
`vet.seaport.example`) with its own whitelisted-by-mistake or not-yet-delisted contract passes
all three. **Fix:** the verifier UI must present identity as "issued by *domain X* (verify you
trust this domain)" and the **discovery registry** (central `businesses.domain` +
`documentStores`) is the authoritative human-trust anchor — cross-check the scanned
`issuer.documentStore`/`domain` against the central registry entry for that business and warn
loudly on mismatch. DNS alone is necessary-not-sufficient; document that explicitly in §5.

### 5.2 [Medium] Delisted-but-DNS-still-present: revocation correctly still works via `isValid` — confirmed, with a caveat
If a vet is delisted on-chain but keeps its TXT record: identity pillar still passes (TXT present),
but **issuance pillar uses `isValid(root)` which only checks issued && !revoked on the
`DogTagIssuer` clone — it does NOT check `IssuerRegistry.isWhitelisted` at read time.** So a
delisted issuer's **already-issued, never-revoked** roots **still verify as VALID forever.**
Delisting only prevents *future* `issue()`/`revoke()` calls; it does **not** invalidate past
credentials. This may be intended (delisting ≠ mass-revocation) but it means "compromised signer
delisted globally O(1)" (arch §11) does **not** retroactively kill credentials that signer already
issued. **Fix:** decide and document the semantics. If delisting should invalidate past
credentials, `isValid` (or the verifier) must also check `registry.isWhitelisted(issuer-of-record)`
— but the clone doesn't store *who* issued each root by default (the event does). Options: (a)
verifier additionally checks the issuing address from the `RootIssued` event against current
whitelist; (b) accept that delisting is forward-only and credentials must be explicitly
`revoke()`d (requires the now-delisted key — chicken/egg → admin must be able to revoke). Add an
admin/registry-level revoke path for the delisted-key case. **This is a real operational gap.**

### 5.3 [Medium] DoH resolver trust & no DNSSEC requirement
The verifier resolves TXT "over DNS-over-HTTPS" (arch §5, impl §1.7) but doesn't pin *which* DoH
provider or require DNSSEC validation. A compromised/coerced DoH resolver can forge the TXT
answer, defeating the identity pillar. **Fix:** pin a small set of reputable DoH resolvers, prefer
DNSSEC-validating resolvers and check the AD bit, and/or query ≥2 independent resolvers and
require agreement. At minimum, document the resolver and that the binding is only as trustworthy
as the resolver + the absence of DNSSEC.

### 5.4 [Low] TXT format parsing is loose
`dogtag net=ethereum chainId=135 addr=<documentStore>` matched "case-insensitive addr, matching
chainId." Spec should pin: exact token order not required (match key=value pairs), `net` must
equal `ethereum`, `chainId` must equal the configured chain, `addr` checksum-insensitive equality,
reject if multiple conflicting `dogtag` records exist (don't accept "any one matches" if another
contradicts). **Fix:** define strict parse + conflict handling.

---

## 6. Standards compliance (research/01)

### 6.1 [High] Microchip regex contradicts ISO/EU and the standard's own §7
`impl §1.6` enforces `microchipId` `^[0-9]{15}$` **unconditionally for every credential**
(`require regex(c.credentialSubject.microchipId, ...)`). Problems:
- research/01 §7 and §1 say the EU rule allows a **tattoo applied before 2011-07-03** *instead of*
  a chip. Unconditionally requiring a 15-digit chip rejects legitimately tattoo-identified
  animals.
- The DOT service-animal form and the DOT/`DOG_PROFILE` credentials don't necessarily carry a
  microchip; requiring it on *every* credential type is wrong. The validator runs for all types
  before the type-specific branch.
- CDC low-risk path does **not** collect a microchip number (research/01 §4 "gotcha") — chip is
  physically required but **not a form field**; a `CDC_IMPORT_FORM` credential should not be
  forced to carry it.
**Fix:** make microchip presence conditional: MANDATED for EU travel + rabies + high-risk CDC;
allow `tattooCode` alternative for pre-2011 EU; not required for DOT/profile/low-risk-CDC. Move
the `^[0-9]{15}$` check inside the relevant type branches.

### 6.2 [High] Several MANDATED real-world invariants are not in the validator
`impl §1.6` enforces: microchip regex, rabies field presence, microchipDate ≤ vaccinationDate,
age ≥12wk, validFrom == vaccinationDate+21d (primary). It marks the rest as "… (EU AHC 10d/4mo,
CDC ≥6mo, titer ≥0.5 …)" — i.e. **not actually coded.** Missing vs. research/01:
- Titer ≥ 0.5 IU/ml **and** the timing windows (blood ≥30d after vax, ≥90d before cert issue) for
  non-listed-third-country entry (research/01 §1).
- EU AHC validity: 10 days to entry, then 4 months (research/01 §1) — `validUntil` should be
  enforced relative to `validFrom`.
- CDC: dog ≥6 months **at entry** (not at vaccination), receipt valid 6 months (research/01 §4).
- Echinococcus treatment 24–120h before entry for dogs → FI/IE/MT/NO (research/01 §1) — entirely
  absent.
- The 21-day rule is **only** for *primary* series; boosters given in-window must skip it
  (research/01 §2). The validator's `validFrom == vaccinationDate + 21d` would **wrongly reject a
  valid booster**. The `vaccinationType` enum exists in research/01 §2 but the validator ignores
  it.
**Fix:** implement each invariant guarded by type and by `vaccinationType`; for booster, require
`validFrom ≤ previousValidUntil` continuity instead of +21d. Add titer/timing, CDC age-at-entry,
echinococcus.

### 6.3 [Medium] W3C VC envelope `validFrom`/`validUntil` placement vs validator
research/01 §6 mandates that validity windows live on the **envelope** (`validFrom`/`validUntil`),
not in `credentialSubject`. But `impl §1.6` reads `vaccinationDate`, `validFrom`, `validUntil`
flatly and the rabies field-presence list mixes envelope and subject fields. The wrap step
flattens everything into leaves regardless, so cryptographically it's fine, but the **schema
validator must look in the right place** and the W3C type array must include
`"VerifiableCredential"` + the specific type (the validator branches on `c.type includes
"Vaccination"` — ensure the canonical type string is `"RabiesVaccinationCertificate"` per
research/01 §6, not `"Vaccination"`). **Fix:** align type strings and validity-field locations
with research/01 §6.

### 6.4 [Info] CDC import form correctly off-chain — confirmed
`CDC_IMPORT_FORM` is "Off-chain only (app + email)", not anchored (arch §3.6 table, §11). This is
correct: the CDC receipt is CDC-issued out-of-band and contains heavy PII (importer DOB, passport
no., phone, email per research/01 §4). Keeping it off-chain and out of the registry is the right
call. No finding, but ensure the app does not accidentally try to `wrapDocument`+anchor it.

### 6.5 [Info] DOT self-attestation trust level — handled, with one gap
`impl §1.6` marks DOT `trustLevel = SELF_ATTESTED` and arch §3.6/§11 weight it lower / keep it
off-chain. Correct. **Gap:** the wrapped-doc shape has no `trustLevel` field — where does
SELF_ATTESTED surface to the verifier/UI? **Fix:** carry trust level either as a credential
subject field or derive it from `recordType == DOT_SERVICE_FORM` in the verifier, and surface a
distinct "self-attested, not authority-verified" badge in the mobile UI. (Low/Info.)

---

## 7. Multi-tenancy / self-hosted compromise

### 7.1 [High] A compromised business backend can spoof discovery-adjacent data via the registry-trusted fields it controls
By design, a whitelisted business **can** issue bad credentials for pets it sees — accepted, that's
the trust model (accreditation gate). The question is blast radius beyond its own records:
- **Forge other issuers' records?** No — each record's `merkleRoot` must be `issue()`d on **that
  record-type clone** by a **whitelisted address**, and verification checks `documentStore`
  matches the clone + DNS binds the domain. A vet cannot make a root verify against *another*
  vet's `documentStore` without that vet's whitelisted key. **Good** — the per-business clone +
  per-address whitelist contains this.
- **Forge central appointments?** Per §3.1 (Critical), **currently yes** if the HMAC-key-by-path
  binding isn't enforced. Fix in §3.1 closes this.
- **Spoof discovery?** The `businesses` registry (`apiBaseUrl`, `domain`, `documentStores`,
  `geo`, `services`) is admin-written (`POST /v1/businesses (admin)`), so a business can't edit its
  own registry entry — **good**, provided the admin endpoint is truly admin-only and businesses
  cannot self-register. **Fix/confirm:** ensure `issuer/apply` (§3.2) and `POST /v1/businesses`
  are separate — application ≠ registry write; only admin approval writes the registry and the
  whitelist. Document that a business never controls its own `documentStores`/`domain` registry
  fields.

### 7.2 [Medium] Onboarding/whitelist gate relies entirely on off-chain accreditation review with no documented verification rigor
`whitelistIssuer(addr)` is called after "admin verifies accreditation off-chain (USDA#,
license#)" (arch §4.3, impl §4.3). The strength of the **entire** issuance trust model is this
manual review. The spec doesn't define what "verify" means (call the licensing board? check NVAP
NAN format only?). A forged USDA#/license# that passes a cursory check gets a permanent issuing
key. **Fix:** define the minimum verification (e.g. NAN is 6 digits per research/01 §3, verified
against APHIS NVAP lookup; state license verified against the state board), record the evidence,
and support periodic re-attestation / expiry of whitelisting. Add the ability to delist on
accreditation lapse (ties to §5.2 revocation gap).

### 7.3 [Medium] Self-hosted backend holds the HMAC shared secret AND its issuer key — single compromise = both issuance and appointment-event abuse
A compromised business box yields its chain signing key (revocable via delist, but see §5.2) **and**
its `HMAC_SHARED_SECRET` (lets it post appointment-events). The HMAC secret has no on-chain
revocation; the only kill switch is central rotating/removing the registry entry. **Fix:** support
HMAC key rotation (`hmacKeyId` already in the registry — make rotation a first-class admin action)
and ensure delisting a business also disables its appointment-event credentials.

---

## 8. Privacy / PII

### 8.1 [Info] On-chain footprint is roots-only — confirmed
On-chain: `DogTagIssuer` stores `issuedAt[root]`/`revokedAt[root]` (bytes32 roots + timestamps);
`DogTagSBT` stores `tokenId` + `profileRoot` (bytes32) + owner address. **No PII or cleartext
field on-chain.** Salted leaves + selective disclosure mean even the root reveals nothing.
Correct and consistent with arch §8/§11/§3.5.

### 8.2 [Medium] The SBT owner address is on-chain and links all of a pet's credentials
`profileRoot` updates and the owner `address` are public. If the owner address is reused or
linkable to a person (e.g. a custodial address tied to their account, or they reuse it), an
observer can enumerate that owner's pets and see issuance/revocation timing patterns. Arch §4.1
mentions "userWalletOrCustodial." **Fix:** prefer a fresh custodial address per pet (or per user)
that is not otherwise linkable, and document that owners should not reuse a doxxed address. Note in
privacy model.

### 8.3 [Low] `data` blob self-describing format leaks field *names* and *types* to anyone who gets the wrapped doc
Selective disclosure removes *values* of obfuscated fields but the `keyPath`s of remaining fields
(and salts) are visible to whoever holds the doc. That's inherent to the design and acceptable
(the holder chooses what to share), but the **obfuscated set still reveals which fields existed**
via `privacy.obfuscated` count and the leaf structure. Minor metadata leak. Info/Low — document
that obfuscation hides values, not the existence/shape of fields.

### 8.4 [Info] Central registry stores only non-personal business data — confirmed
`businesses` collection is explicitly "Non-personal discovery data" (arch §9.1). `appointments`
at central do carry `userId/petId` (necessarily). PII minimization in discovery is correct.

---

## 9. Missing pieces blocking a buildable v1, and build-order sanity

### 9.1 [High] No auth model for the business backend's own portal/API
Beyond the custody endpoints (§1.2), the spec never defines **how the vet/groomer staff
authenticate to their own backend** (the React SPA → Rust API). `/records`, `/records/{id}/revoke`,
`/import/pull`, `/calendar/*`, staff appointment actions all mutate state and sign transactions but
have **no stated authentication**. This blocks implementation and is a security hole if shipped
open. **Fix:** specify operator auth (session/JWT for the portal), role(s), and which endpoints are
public (only `GET /records/{id}` with a record-JWT, and the cross-backend HMAC endpoints) vs.
operator-only (everything else, especially custody, issue, revoke, calendar connect).

### 9.2 [Medium] Mobile-user auth (`POST /v1/auth/...`) is a stub
Central `§4.1` lists `POST /v1/auth/...` with no scheme (password? OTP? session tokens? refresh?).
The entire owner identity, pet ownership, and share-token issuance hangs off this. **Fix:** specify
the auth scheme, session/refresh tokens, and how `userId` is bound to pets and to share-JWT `iss`.

### 9.3 [Medium] No DNS-TXT *verification* step in onboarding, only "instructions"
The setup wizard "set DNS-TXT instructions" (impl §5.1) but nothing **verifies** the operator
actually published the correct TXT before/at whitelisting. A vet could be whitelisted while its DNS
binding is wrong/missing → every credential it issues fails the identity pillar in the field.
**Fix:** central (or the wizard) should resolve the TXT and confirm it names the right
`documentStore` before marking the business "active" in the registry; re-check periodically.

### 9.4 [Medium] `dogTagId` allocation authority is unspecified
Who allocates `dogTagId`? `POST /v1/pets/{id}/mint` mints with *some* `dogTagId`. If it's a
sequential counter, it's enumerable; if random, collision handling unspecified; if derived from
microchip (recommended §1.4) it's deterministic. **Fix:** define allocation (recommend
`keccak256(microchipId)` → uniqueness + non-enumerability).

### 9.5 [Medium] RPC liveness / read-path availability for the verifier
Verification pillar 2 requires a live ROAX RPC read from the **mobile app**. RPC was returning 502
at design time (arch header). If RPC is down, every verification fails the issuance pillar.
**Fix:** define behavior on RPC unavailability (degrade to "issuance: UNKNOWN" rather than INVALID;
retry; possibly a central read-proxy/cache the mobile can fall back to). Currently the SDK returns
a boolean → an outage looks like "not issued."

### 9.6 [Low] Two SDKs "byte-for-byte equivalent" but only one set of test vectors guards it
Good that `testvectors.json` is asserted in both CIs (impl §1, §9). Ensure the vectors also cover
the **edge cases** that bite cross-language: empty/NULL values, decimals with trailing zeros,
NFC normalization of multi-codepoint strings, integer with leading zeros, lone-odd-node promotion,
and obfuscated-field reconstruction. **Fix:** mandate these specific vectors.

### 9.7 [Info] Build order — sane
1 SDK → 2 contracts → 3 vet backend → 4 central → 5 portals → 6 mobile → 7 calendar/appointments →
8 hardening is the right dependency order (trust core first, then anchoring, then the producers,
then consumers). One tweak: pull the **auth model (§9.1/§9.2)** forward into steps 3–4 (it's a
prerequisite, currently implicit), and add a **DNS-verification gate** to step 4 (§9.3).

---

## 10. Inconsistencies between architecture.md and implementation.md (quick list)

1. Genesis/custody endpoint paths: `/admin/genesis/*` (research/04) vs `/genesis/*`
   (impl §3.1); and **no auth specified** on them in impl. (§1.2)
2. JWT `exp`: "2–5 min" (arch §7) vs hardcoded `180s` (impl §3.4). (§1.1)
3. Rabies field names: `vaccineManufacturer`/`batchNumber` (research/01) vs
   `manufacturer`/`batchLotNumber` (impl §1.6) vs prose in arch §3.6. (§1.6)
4. `recordType` is a human string in the wrapped doc/registry but `keccak256(label)` on-chain;
   mapping never documented. (§1.3)
5. W3C type string: validator branches on `type includes "Vaccination"` (impl §1.6) vs canonical
   `"RabiesVaccinationCertificate"` (research/01 §6). (§6.3)
6. `validFrom`/`validUntil` mandated on the envelope (research/01 §6) but the validator/rabies
   field list treats them flatly alongside subject fields. (§6.3)
7. Microchip required "MANDATED for EU+CDC" (arch §3.6) but validator requires it on **every**
   credential type (impl §1.6). (§6.1)
8. Schema invariants listed as enforced in arch §3.6 are only partially coded; several are
   `…`-elided in impl §1.6 (titer, CDC age-at-entry, echinococcus, booster rule). (§6.2)
9. Appointment-events callback: arch §8.3 / impl §4.4 say "HMAC verify" only; no per-business
   ownership/path binding stated. (§3.1)

---

## 11. Top fixes, prioritized

1. **(Critical, §2.1)** Make central `GET /share/{ref}` enforce `sub==ref` + `aud==dogtag-business`
   + scope + one-time `jti` + `iss`, identical to the business-side record JWT checks.
2. **(Critical, §3.1)** On `appointment-events`, verify HMAC with the **path business's** key and
   assert `appointment.businessId == path businessId` before applying.
3. **(High, §1.2 / §9.1)** Define and enforce operator/admin auth for all custody + issue + revoke
   + calendar endpoints; restore `/admin` namespace; rate-limit `/unlock`.
4. **(High, §3.3)** Single rev allocator (central only); businesses never assign rev; eliminate the
   tie-rev split-brain window.
5. **(High, §5.1/§9.3)** Treat DNS as necessary-not-sufficient; cross-check scanned issuer
   `domain`/`documentStore` against the central registry; verify TXT at onboarding before going
   active.
6. **(High, §6.1/§6.2)** Fix the schema validator: microchip conditional (allow pre-2011 tattoo,
   don't require on every type), booster-aware 21-day rule, and add titer/CDC-age/echinococcus
   invariants.
7. **(High, §7.1/§3.1)** Confirm businesses cannot self-write the registry; only admin approval
   writes `documentStores`/`domain`/whitelist.
8. **(Medium, §5.2)** Decide & document delisting semantics; add an admin/registry revoke path for
   already-issued roots of a compromised/delisted key.
9. **(Medium, §2.4)** Mandate atomic `jti` consumption (unique-index insert / `SET NX`), forbid
   read-then-write.
10. **(Medium, §4.2)** Use `etag` (not `rev ≤ ours`) as the primary echo discriminator so human
    edits aren't silently dropped.

---

## 12. Bottom line

A buildable, well-reasoned v1 architecture whose cryptographic trust core and custody model are
sound — but ship-blocking work remains on **symmetric auth enforcement** (the user→business share
endpoint and the appointment-events callback both lack the path/audience binding their
counterparts have), an **undefined operator-auth model**, **partial standards-validator coverage**,
and **clarifying DNS-as-identity and delisting semantics**. Fix the two Critical auth gaps and the
operator-auth model before any external deployment.
