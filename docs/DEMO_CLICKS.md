# DogTag - literal click-through (LIVE on ROAX)

The exact buttons to press, in order, against the **live ROAX deployment** (chainId 135; contract addresses come from `contracts/.env` / `contracts/deployments/roax.json`).
The demo buttons fill every form, and all passwords are prefilled, so the operator **types nothing** except the `dogTagId` handle in §C - just click.
(Testnet only.) For the full runbook + phone networking + gotchas see **[DEMO.md](./DEMO.md)**.

Boot first: `scripts/demo-up.sh` (or with a public tunnel on corporate Wi-Fi:
`VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh` - see DEMO.md §6).
This also boots the **prover-service** on **:41875** (`POST /prove-consent`, the trusted server-prove fallback - see §E) and the browser-based **pet-owner (holder) wallet** on **:45931** (a phone-free way to receive/hold/share records; owner-hidden proving stays on the native apps - see [`stacks/owner/web/README.md`](../stacks/owner/web/README.md)).
Automated equivalents: `scripts/e2e-smoke.sh` (credential lifecycle, 7 steps) and `scripts/e2e-zk.sh` (the owner-hidden consent proof, end to end with a real proof).

> **You do not need to rebuild anything for the browser sections.**
> `demo-up.sh` runs `cargo build --release` for all four backends itself, and serves every portal with
> `vite dev` straight from source, so one boot picks up whatever is on your branch.
> The **only** things that do not refresh themselves are the **phone apps** (§D, §G), which carry a
> compiled Rust core and bundled proving artifacts. Rebuild those by hand when you want §G, following
> `docs/MOBILE_BUILD.md`.

Portals: **admin** http://localhost:39741 · **vet** http://localhost:41873 · **groomer** http://localhost:43617 · **government** http://localhost:44831 · **owner wallet** http://localhost:45931
Demo passwords (prefilled): operator `operator`, admin `admin`. Record type everywhere: **VACCINATION**.

New since the last revision of this guide, each with its own section below: the **verification bench**
(§F), **badges that stop over-claiming** (§G), the **pets collection** (§H), the **calendar and the
client handoff** (§I), **per-row on-chain provenance** (§J), and the **issuer domain binding** (§K).
Two of those surfaces will honestly tell you they could not check something. That is the feature, not a
fault; §J and §K say exactly when to expect it.

> Stale session? Backends keep sessions in memory, so a backend restart invalidates the saved token.
> The portal detects the 401, shows **"Session expired - please log in again"**, clears the token,
> and routes you back to login (vet/groomer Setup re-shows the Custody admin login). Just click Sign in
> again (password stays prefilled).
>
> A restart also **re-locks custody** (the seal survives in `.demo/*-custody.json`, the decrypted seed
> does not). After signing back in, the vet/groomer portal shows a **non-blocking banner** saying custody
> is locked - read-only pages stay reachable - and the first action that needs custody raises an
> **unlock prompt in place**, over the page you are on, then replays the refused request. You no longer
> go digging through Setup, and nothing you typed is discarded. The dedicated **`/unlock`** page is still
> there as a direct link. Both fields prefill in demo mode, so it is one click: **Unlock**. A wrong
> passphrase shows an inline error and does **not** trigger the "Session expired" path above.

---

## A0. Device creates a self-custodial wallet - DogTag app

The phone just needs a wallet - the **vet** issues the dog tag against it (§A1).
There is **no** central registration and **no** "Central API URL" setting; every host the device talks to comes from a scanned QR.

1. **Profile → "Create embedded wallet"** → the app shows a **24-word seed** → confirm.
2. Done - the device is ready to scan the vet's dog-tag QR (§A1).
   The seed is the root of the owner-hidden identity: the reserved owner leaves (owner-address, consent-key, owner-secret) and their salts are derived from it per tag, so restoring the phrase restores them.
   The wallet address itself never leaves the device during issuance.

---

## A. Admin onboards the business - admin portal (:39741)

1. **Sign in** (admin password prefilled).
2. Go to **Onboard issuer** (the wizard).
3. Click **Vet preset** (top-right) - fills all three steps with the vet demo data
   (record types incl. `VACCINATION` + `DOG_PROFILE`).
4. Step 1 **Register business** → **Register business**.
5. Step 2 **Submit issuer application** → **Submit application**.
6. Step 3 **Approve (whitelists on-chain)** → **Approve & whitelist** → done (tx hashes shown).
   - **Issuer** approval whitelists the issuance record-types per address; when the record types
     include `DOG_PROFILE`, Approve **also grants `DogTagSBTConsent.ISSUER_ROLE`** (the custodial
     mint capability) to the signer.
   - For a **groomer/verifier** application (Groomer preset), Approve instead whitelists each
     `VERIFY:<purpose>` on-chain (from the application's **verify purposes** field) - see §B and the
     Groomer variant.

> The left-hand nav is **Dashboard · Activity · Issuers / Factory · Onboard issuer · Business registry ·
> Issuer applications · Whitelist · Governance · Verification bench**.
> Two labels moved since the last revision of this guide: the read-only "Whitelist viewer" is now a
> full grant/revoke console called simply **Whitelist**, and **Verification bench** is new (§F).

> Funding gas: the on-chain signer still needs PLASMA. If not already done, run
> `scripts/demo-bootstrap.sh 0x<signerAddress>` once (the address is the genesis signer from step B) -
> it funds PLASMA and idempotently re-grants the same whitelists + `ISSUER_ROLE` the Approve grants.
> **Whitelisting + the role grant are the admin Approve above - the script's unique job is gas.**
> Note for prod: `ISSUER_ROLE` is a trust escalation (a holder can seal any unminted `dogTagId` to any
> root) - grant only to accredited vets.

---

## A1. Vet issues the dog tag - vet portal (:41873) + DogTag app

The dog-tag is issued by the **vet**, not by an admin page (there is no admin "Registered devices" /
"Mint dog-tag" page). Prereq: the vet signer is set up (§B), funded + whitelisted, and holds
`ISSUER_ROLE` (the funding note above).

1. Vet portal → **Register pet** → **Fill demo data**. The form collects the **`ownerIdentity`**
   block (demo-prefilled: **countryOfIdentification** `GB`, **identification** `P1234567`,
   **name** `Alex Doe`) plus the pet fields.
   The operator types **no wallet address anywhere** - the flow has no field for one.
2. **Start issuance** → `POST /profiles/issue/session/start` → allocates the **`dogTagId` handle**
   (skipping any id already sealed on-chain) and renders a one-time QR `<vetHost>/p/<token>`
   (32-hex token, 180s TTL). **Note the handle** - you type it into the vaccination Issue form in §C.
3. On the phone: **Scan** the `/p/<token>` QR.
   The app resolves the session, derives the per-tag owner leaves from its wallet seed + the
   `dogTagId`, folds the **owner-hidden profile tree** to the root `R` **on the device**, and POSTs
   `<vetHost>/profiles/issue/custodial-bind { token, root }` - the token is one-time (a second bind
   gets 410), and no owner address or signature crosses this boundary.
4. The vet backend anchors **`issue(R)`** on the `DOG_PROFILE` clone, then seals
   **`DogTagSBTConsent.mintCustodial(dogTagId, R)`** (write-once `profileRoot`; the ERC-721 holder is
   the contract's neutral custodian, never the pet owner). It responds `status:"minting"`; the portal
   polls `GET /profiles/issue/session/{id}` while the phone polls `profileRoot(dogTagId)` on ROAX
   until it equals its own `R` (the tag-forging animation), then stores the tag.
   **Gasless for the device**; the vet, the chain, and every later verifier learn only `R`.
5. On the phone, open **Profile**. The new tag is listed under **Dog-tags**, highest handle first,
   showing the handle and that this phone holds its owner-secret and built its profile root.
   A tag issued this way used to be missing from that card until a record was also imported, so a
   correctly issued tag could read as "No dog tag yet"; it now appears from the issuance alone.

---

## B. Vet stands up its signer + applies - vet portal (:41873)

Setup is a linear wizard; each step auto-advances on success.

1. **Sign in** (operator password prefilled).
2. Navigate to **Setup**.
3. **Continue** (Custody admin login - admin password prefilled).
4. **Generate 24-word seed** → tick **"I have written down all 24 words."** → **Continue to confirmation**.
5. Confirm screen: (demo) re-type the challenge words shown on the seed screen + any passphrase →
   **Confirm & encrypt**. (The derived signer address is now auto-saved.)
6. **Unlock** (enter the same passphrase) → **Unlock** → **Continue**.
7. **Accounts** → **Continue to whitelist application** (no extra accounts needed).
8. **Whitelist** → **Fill demo data** (signer address auto-filled from genesis). For a **groomer/verifier**
   this also fills the **verify purposes** field (`grooming_intake/boarding_intake/daycare_access`), carried
   on the application as `verifyPurposes`. → **Submit application** → **Continue**.
9. **DNS** → **Done**.

> The signer address is auto-carried - you never copy/paste it. After this, **approve in the admin portal
> (section A)** - that is what whitelists the address on-chain (issuance record-types + the `DOG_PROFILE`
> `ISSUER_ROLE` grant for a vet, `VERIFY:<purpose>` for a verifier). The only script step left is
> **funding** PLASMA: `scripts/demo-bootstrap.sh 0x<signerAddress>`.

---

## C. Vet issues a vaccination credential → IMPORT QR - vet portal (:41873)

1. Go to **Issue a record**.
2. Click **Fill demo data** (valid rabies cert; recordType `VACCINATION`). This fills the cert fields
   but **leaves `dogTagId` blank** - the demo-fill no longer clobbers it (a fixed footgun).
3. **Set the `dogTagId` field = the dog tag's handle from §A1** (the numeric `dogTagId` the Register-pet
   wizard allocated). It **must match**: the credential's `dogTagId` leaf is what ties the record to
   the sealed tag (on-chain the tag key is `field_of_value(handle)`), and the phone attaches an
   imported record to a pet by that handle - a mismatch leaves the record orphaned from the tag, so it
   cannot be presented for that pet in §E.
4. **Sign & Issue**.
5. **Create QR** → the **IMPORT** QR (device ← vet) renders. It carries a SHORT one-time token
   (`http://<host>/r/<32-hex>`), NOT an embedded record payload - a low-density QR the camera focuses
   on instantly. The token is **deleted after the first scan** (one-time; 180s expiry), so re-scanning
   the same QR yields a 404.

> **The "Valid until" field is the knob §G uses.** It is a required date on this form and the demo fills
> it a year out. Type a date in the **past** instead and you get a genuinely issued, genuinely anchored,
> genuinely expired credential - which is the cleanest way to watch a badge tell the truth (§G) and the
> cleanest thing to feed the bench (§F).

---

## D. Phone (DogTag app) - scan → import → verified on-chain → view fields

> Prereq: the device has a **wallet** (§A0) and the dog-tag was **issued against it by the vet** (§A1) -
> the record's `dogTagId` handle must match a tag the device holds.

1. Open **Scan** (the Home header **Scan** button) → scan the vet's QR.
2. Watch **Anchoring… → Verified on-chain ✓**. The record lands under the pet with the matching handle.
3. **Tap the imported credential** → the detail view decodes **every Merkle leaf** and shows the field
   values + the on-chain root/issuer/verdict (Android + iOS).
   The detail also shows an **Imported** timestamp and when the verdict was last checked, and carries a
   **Refresh** action (re-reads the record's anchor on ROAX) and **Delete from this phone** (removes this
   phone's copy only; nothing on-chain changes).

> Phone can't reach `localhost`. Same Wi-Fi: set the app server base to this Mac's LAN IP (demo-up.sh
> already sets the QR host to it via `LAN_IP`/`DEPLOYMENT_URL`). Corporate/VPN Wi-Fi: boot with
> `VET_PUBLIC_URL=https://<sub>.trycloudflare.com`. Full details: docs/DEMO.md §6.

---

## E. (Optional) Owner-hidden proof-of-verification on-chain - vet or groomer portal

The owner proves consent to the groomer without revealing who they are (symmetric counterpart of the §C-D import).
There is **no mode picker** - owner-hidden ZK consent is the only verify flow.

1. Open the verify surface: in the **vet** portal the **Verification** tab; in the **groomer** portal
   **Appointments** → open the booking → **Start verification** (the result is then filed against that
   visit and its client, and is searchable under **All verifications**), or **Ad-hoc verification**
   for a walk-in with no booking.
2. Pick a **Purpose** from the dropdown (e.g. boarding intake).
3. **Start export** → the session QR renders. It carries the groomer's wallet address + a
   one-time token + host: `http://<host>/x/<token>?a=<groomerAddr>`.
4. On the phone: scan → the app resolves `GET /x/<token>` (purpose, recordType, relayer), confirms the
   groomer is whitelisted on-chain for that `VERIFY:<purpose>` (prod/remote also DNS-verifies the
   groomer; skipped for the `.local` demo) → review → the app signs consent with the tag's **in-tree
   consent key** and **generates the `DogTagConsent` Groth16 proof on-device** → POSTs
   `{ exportToken, proof: {a, b, c, pubSignals} }` to `/v1/verify/consent`.
5. The groomer backend relays `recordVerificationZK` on-chain; the portal polls and flips to
   **Verified on-chain ✓**. The `Verified` event is **owner-blind** (no subject, no key hash): the
   groomer learns only "the owner of tag `dogTagId` consented to `purpose` for me" - never the record,
   the witness, or the owner.

> **Server-prove fallback.** A device that cannot run the on-device prover can POST its (private)
> witness to the **prover-service** (`POST /prove-consent`, booted by `demo-up.sh` on **:41875**) and
> submit the returned proof to the groomer itself - the groomer still never sees the witness. The
> prover-service sees the witness, so in production it must be a prover the OWNER trusts; the demo runs
> it as a platform service. Tunnel it with `cloudflared tunnel --url http://localhost:41875` →
> `PROVER_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh`.

---

## F. Verification bench - throw a record at it and watch every check answer

**Admin portal → Verification bench** (`:39741/bench`).
This is the page that shows you, one row at a time, what verification actually establishes: every check
answers **Pass**, **Fail** or **Could not run**, and each row shows the evidence it rests on and which
contract was asked.
Nothing here is simulated; the rows come from the same verification path the wallet and the verify
panels use, and the reads are made against the real chain.

### F1. Get a record into the box

The bench takes a **wrapped credential JSON**. Two reliable ways to get one:

- **Government portal (:44831) → Issue** → pick **TRAVEL_CLEARANCE** → fill → issue → click
  **Copy wrapped document**.
  That record is anchored live on ROAX **only if this stack booted with a funded, whitelisted
  `GOV_SIGNER_KEY` and a `TRAVEL_CLEARANCE_ISSUER_ADDR` clone**; `scripts/demo-provision-government.sh`
  provisions both, and `demo-up.sh` prints a warning at boot naming whichever is missing.
  Without them `/issue` only dry-runs: the result card badges **built (not anchored)** rather than
  **✓ anchored on-chain**, and benching that record gives **Verifier verdict: not valid** with *"Was this
  issued by a contract that genuinely descends from the DogTag factory?"* reading **Fail** ("The factory
  has NO record of this root"), not the Pass rows in §F2.
  Prefer `TRAVEL_CLEARANCE`: on the fresh contract set it is the record type with a deployed clone and a
  whitelisted government signer. `EU_HEALTH_CERT` may have no clone provisioned, in which case issuing
  it only dry-runs the same way.
- **Owner wallet (:45931)** → open a held credential → **Share a redacted copy** →
  **Copy redacted credential**. Useful on its own: redaction leaves the Merkle root untouched, so the
  bench still passes the redacted copy.

The vet portal has **no** copy-the-document button, on Issue or on Records, so on this deployment there is
no vet JSON to paste and a vet-issued VACCINATION does not reach the bench by either route above.
It reaches the bench through the bench's own **QR share link** field instead, fed a fresh share token as
described next.

> **Do not feed the bench the `/r/<token>` QR link from §C.5.** The bench has a "QR share link" field and
> that link would work, but the token is consumed on first read: fetching it here means the phone gets a
> 404 in §D.
> Mint a **second** token instead: vet portal → **Records** → the row's **QR** button, which issues a
> fresh one-time token on every click.
> Paste that URL into the bench's **QR share link** field and click **Fetch**; §C.5's QR stays intact for
> the phone.
> The Issue page's **Create QR** button cannot do this, because once it has rendered the QR it is replaced
> by that QR, leaving only **Issue another**, which resets the whole card.

### F2. Run it on a genuine record

1. Paste the JSON into **Wrapped document JSON**.
2. Click **Run the checks**.

On a genuine, anchored, in-date record you get **Verifier verdict: valid** and these nine rows:

| Row | Expect | In the verdict? |
|---|---|---|
| Is the document's content intact - does it still hash to the root it claims? | Pass | yes |
| Was this issued by a contract that genuinely descends from the DogTag factory? | Pass | yes |
| Does the document name the same contract the factory says issued it? | Pass | yes |
| Was the signer that issued this whitelisted for this record type? | Pass | yes |
| Is this root actually anchored on-chain by its issuing contract? | Pass | yes |
| Has the issuer revoked this credential? | Pass | yes |
| Is this credential still within its validity window? | Pass | **no** |
| Does the domain the document claims match the one the issuer published on-chain? | **Could not run** | **no** |
| Does that domain's DNS zone name this issuing contract back? | **Could not run** | **no** |

Those last two are **expected** and are not a bug. §K explains them.

The outcomes above were observed on a `VACCINATION`. A `TRAVEL_CLEARANCE` gives the same nine rows: its
validity window sits at `credentialSubject.validity.validUntil` rather than the top level, and the
government backend fills that leaf even if you leave the form field blank, so the expiry row still reads
Pass on a freshly issued one. §G lists all three places a window can live.

Scroll to **Reads made** for the raw material: every chain read the verifier actually performed, with
the contract, the method and the answer. If the chain head was readable, the description above the rows
tells you the block every read was pinned to, so the report is reproducible.

### F3. Now try to slip a forgery past it

Bottom card: **Try to slip a fraudulent record past the checks**.
Each button tells one specific lie with the record above and re-runs everything.
Read the "what will NOT catch this" list on each card as carefully as the result.

Click these in order and watch **which** row objects:

1. **Point it at a different issuer contract** → **Apply & re-run**.
   The verdict flips to **not valid**. The row that names the lie is *"Does the document name the same
   contract the factory says issued it?"*, now **Fail**, and it reports both addresses side by side: the
   contract the factory names, and the contract the document names.
   (*"Was the signer that issued this whitelisted for this record type?"* goes **Fail** alongside it;
   read that row's own finding for why rather than assuming.)
   The one to notice is that **integrity still passes**: the `issuer` block sits outside the Merkle root,
   so the document is free to name any contract it likes. Nothing but the factory anchor catches this.
   This is the whole reason verification resolves the issuing clone from the chain's write-once
   `rootIssuer[R]` rather than believing the document.
2. Re-paste the genuine record, then **Tamper with a covered field** → **Apply & re-run**.
   Now the mirror image: **integrity Fails** and every on-chain row still passes, because the chain
   anchors a root and knows nothing about the content behind it.
3. Re-paste the genuine record, then **Relabel the issuer's name** → **Apply & re-run**.
   **Nothing catches it.** The verdict stays **valid** and the card says so up front:
   "Designed to be caught by: nothing on this verification path".
   That is honest, and it is the shape of the guarantee: this path authenticates the issuing **key**,
   not the display name printed beside it. Binding the name is what the issuer domain work in §K is for.

Re-pasting between mutations matters - they stack, and the amber **Applied to the loaded record** strip
tells you what is currently applied.

### F4. The row that fails while the verdict stays valid

Issue a record with a **past** "Valid until" (§C, the note) and bench it.
You get **Verifier verdict: valid** sitting above a red *"Is this credential still within its validity
window?"* row.

That is not a contradiction, and the page marks it: rows tagged **not in the verdict** are reported
beside it rather than folded into it.
The chain records anchoring and revocation and has **no concept of a validity window**, so an expired
credential really is on-chain-valid.
The verifier's verdict is integrity, on-chain status and the issuer pillar; expiry and the two domain
rows are reported next to it so you can see them without them silently changing the answer.

For the opposite case, revoke the record and bench it again: *"Has the issuer revoked this credential?"*
turns **Fail** and the verdict goes **not valid**.
Revoking is vet portal → **Records** → the row's **Revoke** button, which asks you to confirm ("Revoke
this credential on chain? It stays on record (as revoked) and remains verifiable."). The government
portal's Records page has the same action for its own records. Nothing is deleted either way: the row
keeps its original issuance proof and gains a revoke-tx proof beside it.

---

## G. Watching a badge tell the truth - DogTag app

> **Prereq: this section needs the phone app, rebuilt.** The apps carry a compiled Rust core and bundled
> proving artifacts and do not refresh with `demo-up.sh`; build and install per `docs/MOBILE_BUILD.md`
> (iOS device build) before expecting the behaviour below. Everything else in this guide is browser-only.

A credential badge in the app used to be a flat green VALID resting on whatever the chain said at import
time, however long ago that was. It now states an accurate observation or says it could not check.
The rule, most severe first: **INVALID**, then **EXPIRED**, then **VALID · STALE**, then **VALID**, then
**UNVERIFIED**.

Each state, and exactly how to reach it:

- **EXPIRED (amber).** No clock change and no revocation needed. Issue the record in §C with a **past**
  "Valid until" date, import it in §D. The badge reads EXPIRED as soon as it lands.
  Expiry is read from the document itself, not from the chain, so this works offline.
  Note the field it comes from **differs by record type**: `TRAVEL_CLEARANCE` carries it nested at
  `credentialSubject.validity.validUntil`, `EU_HEALTH_CERT` flat at `credentialSubject.rabiesValidUntil`,
  and `VACCINATION` at the top level as `validUntil`. All three are tried in that order, so a rabies cert
  and a travel receipt both badge correctly.
- **VALID · STALE (neutral).** Import a record, then come back to it more than **an hour** later - the
  freshness window is one hour. The label keeps the previous answer and goes neutral rather than
  collapsing to a bare STALE, because "I have not looked recently" is its own state and must not borrow
  the colour of either neighbour. Tap **Refresh** on the record and it returns to VALID.
- **INVALID (red).** Revoke the record: vet portal → **Records** → the row's **Revoke** → confirm. Then
  tap **Refresh** on the phone. This is the one state that needs an action on the issuing side.
- **UNVERIFIED (neutral).** Run this one on a record whose stored verdict is still **VALID**, which is
  neither the record you just revoked nor the expired one.
  If the revoke above used your only in-date credential, issue and import one more first (§C and §D,
  leaving "Valid until" at the demo's default) so there is a VALID record to refresh.
  Put the phone in airplane mode and tap **Refresh**.
  The chain read cannot be made, so the app says so with a reason instead of guessing.
  Now repeat exactly that airplane-mode **Refresh** on the record you revoked: it stays **INVALID**, and
  that is the guarantee working rather than a step that failed.
  Two things worth checking here, because they are the point of the change: an established **INVALID is
  not laundered** into "could not check" by a failed refresh, and a **stale answer never renders as
  INVALID**. Age may only ever weaken a claim.

The rule itself is a pure, clock-injected function, mirrored case for case on both platforms and pinned
by unit tests. The Android side runs from a plain checkout and is green on this branch:

```
cd apps/android && gradle :app:testDebugUnitTest \
  --tests '*VerdictBadgeStalenessTest' --tests '*CredentialExpiryFromDocTest' \
  --tests '*RefreshCannotUpgradeVerdictTest'
```

(40 tests: the badge ordering including the exact one-hour boundary, the three expiry tiers, and the
rule that a failed refresh may not overwrite an established verdict. It needs `apps/android/local.properties`
with `sdk.dir=...`, which is gitignored, so write it once.)
The iOS mirror is `DogTagTests/VerdictDisplayTests`. What no test covers is the phone actually rendering
the badge, which is why the steps above are the way to see it.

---

## H. Pets as their own collection - groomer portal (:43617)

> Prereq: the groomer portal is up and you are signed in (Groomer variant, below).
> **No custody unlock and no whitelist is needed for this section** - it is the shop's own book. The
> "Custody is locked" banner may be showing; read-only and CRM pages stay reachable behind it.

A pet is now addressable in its own right, not just a line inside a client.

1. **Clients → New client**. Fill name/phone/email, click **Add pet**, give a pet name and breed, then
   **Create client**.
2. **Pets** in the left nav. The pet is listed under four columns, **Pet**, **Species / breed**, **Owner**
   and **DogTag**, with species and breed sharing that one cell.
   The **DogTag** column shows **No tag** until you link one.
3. Click the pet's name to open it. The **Owner** cell is a link too, so the round trip
   pet → owner → pet is one click each way.
4. On the pet page, under **DogTag**, type the tag handle from §A1 into **DogTag id** and click
   **Add DogTag**. The page now shows the id as linked and offers **Remove from this record**, and the
   Pets list shows the handle in the **DogTag** column.
5. **Watch the one-pet-per-tag rule.** Add a second pet for the same owner (**New pet** on the Pets
   page, type the owner's name and pick them from the suggestions), open it, and try to link the **same**
   tag id. It is refused, and the message names the pet already holding it and that pet's owner, for
   example: "DogTag 7 is already linked to Rex (Alex Doe). Remove it from that pet first, or check the
   id."
6. **Search** takes pet name, breed, species, **the DogTag id**, or **the owner's name** - searching the
   owner returns all their pets, searching the tag id returns the one pet holding it.

The pet page also states plainly that this portal cannot mint or revoke anything: a groomer is
whitelisted to VERIFY, not to issue, and adding or removing a tag here only edits the shop's own record.

---

## I. Calendar - the shop's book, and the client's copy

> Same prereq as §H: groomer portal, signed in, no custody needed.

### I1. The shop side

1. **Calendar** in the left nav. **Day / Week / Month** switch the view and **Today** returns to now;
   Month is a proper 7-column grid.
2. **New appointment**, or from a pet's page **Book appointment**. Pick the client and pet, set a time,
   save. The booking appears in the grid.
3. **Calendar sync** (link at the top of the Calendar page) is where the subscription feed lives.
   Click **Publish calendar address**. You get a URL of the form `/calendar/feed/<64-hex>.ics`, with
   copy-and-paste instructions for Apple Calendar, Google Calendar and Luma. It is one direction only:
   the feed publishes your book outwards, it does not take bookings in.
   That URL is **unauthenticated** - a calendar client cannot present a bearer token, so the secret in
   the path is the whole gate. Treat it as a credential. The same panel can replace it or switch it off,
   and any wrong token returns a flat **404**, never a 401, so the feed's existence is not confirmed to
   a guesser.
4. **Import an .ics file** on the same page → **Choose an .ics file** → **Import**.
   Re-importing the same file does **not** duplicate: matching is by the source event's `UID`, so a
   second import reports the events as *updated* rather than created.
   An imported booking arrives **unassigned** - a calendar invite names an event, not one of your
   clients, and the import refuses to invent a client row. Open it and pick a client before it can be
   saved.

### I2. The client side - scan one appointment, get it in your own calendar

1. Open the booking (**Appointments** → the row) and click **Send to client**.
   The dialog mints a link `/a/<token>` and shows a QR for it.
2. Scanning it (no login, on the client's own phone) opens a small page with the pet's name, the time,
   the status, and **Add to calendar** / **Add to Google Calendar**.
3. What the client gets is deliberately **not** the shop's feed entry. The shop's own feed line reads
   `Appointment - Rex / Alex Doe`; the client's copy reads `Appointment - Rex` and carries no client
   name and none of the operator's notes.
   The page also says outright that adding it saves a copy as it is now, and that the calendar will not
   update itself if the shop changes or cancels the booking.

> **If the dialog says "No QR for this deployment", read the reason under it.** When `DEPLOYMENT_URL` is
> a loopback address the handoff deliberately withholds the QR and explains that a client's phone would
> not reach the link, so scanning one would fail or land somewhere else entirely. The link itself is
> still shown and still works on this machine.
> `demo-up.sh` sets that to your Mac's LAN IP, so you normally get a QR; you will hit the loopback case
> only if you started a backend by hand.

---

## J. Per-row on-chain provenance - and why every row may say "not chain-addressable"

The activity tables now judge **each row** on whether its transaction hash is one a chain could actually
address, and label it. The three places this appears:

- admin portal → **Activity**
- government portal → **Oversight**
- vet portal → **Traceability**

Start with the government and vet ones.
`demo-up.sh` passes `INDEXER_API_BASE` to the vet, groomer and government backends but not to admin-api,
which defaults to no feed at all and answers those routes 503, so admin → Activity shows its
indexer-not-configured state rather than rows unless your `contracts/.env` exports that variable.

Each row carries either **chain-addressable**, whose transaction hash is a working
`explorer.roax.net/tx/...` link, or **not chain-addressable**, whose hash is rendered as inert text with
**no link at all**.

> **Expect every row to say "not chain-addressable" on a `demo-up.sh` stack, and do not report it as a
> bug.** `demo-up.sh` boots the oversight indexer with `INDEXER_DEMO_MODE=1`, whose scripted history uses
> placeholder hashes like `0x0800`. Those are perfectly good demo data and cannot be transaction hashes on
> any chain, so labelling them and refusing to link them is the correct answer.
> The label is decided per row from the hash itself, not from a demo flag, so a single scripted row inside
> an otherwise real feed is still caught.
> **Switching this indexer to the live chain is out of scope for this guide, and unsetting
> `INDEXER_DEMO_MODE` alone will not do it.** The indexer treats `DEMO_MODE` and `VITE_DEMO_MODE` as the
> same switch, and `demo-up.sh` sources `contracts/.env`, which sets `DEMO_MODE`.
> Even with all three unset, the scope registry is then empty and every query fails closed with a **401**
> until `INDEXER_SCOPES` is authored; the two well-known tokens `demo-up.sh` hands the vet, groomer and
> government backends exist only in demo mode, so all three would need re-issuing as well.
> The scripted rows are what a `demo-up.sh` stack shows, and the label above is the correct answer for them.

While you are here, the indexer now says what it is. On a **freshly built** demo-mode indexer,
`curl http://localhost:46001/health` returns:

```json
{"backend":"simulated","chainId":null,"ok":true,"simulated":true}
```

`chainId` is `null` because a scripted source is on no network at all, and both keys are emitted on the
live path too, so their absence can never be mistaken for "live". A build predating this reports neither
key, which is exactly the ambiguity it removes.

---

## K. The issuer domain binding - and the two rows that will say "Could not run"

The bench's last two rows ask whether the issuer's claimed domain is really theirs:

1. *"Does the domain the document claims match the one the issuer published on-chain?"*
2. *"Does that domain's DNS zone name this issuing contract back?"*

**Both report "Could not run" today, on every surface, and that is correct behaviour rather than a
fault.** The reasons are printed in the rows themselves:

- The on-chain half says: *"No `IssuerDomainRegistry` address is configured, so the issuer's own on-chain
  domain claim could not be read."*
  The `IssuerDomainRegistry` contract exists in the source tree but **is not deployed on ROAX** - it is
  absent from `contracts/deployments/roax.json`, so `demo-up.sh` resolves it to the zero address and the
  bench is deliberately given no fallback. This one check reads no default address on purpose: the
  contract set is still being revised, and reading a constant that may have moved would be worse than
  saying nothing.
- The DNS half says: *"The TXT lookup is resolved server-side by the verifier backends; this bench runs
  in the browser and cannot perform it. A passing on-chain claim above is NOT evidence that DNS agrees."*

They are two separate rows rather than one merged green tick precisely so that neither can imply a check
that never ran. Neither feeds the verdict, so their absence changes no answer - it only means the
issuer's **name and domain** are unproven, which is the same gap §F3's third mutation demonstrates from
the other direction.

Deploying the registry is what turns these two rows on. Until then, expect **Could not run**, in the
bench and anywhere else a binding is shown.

---

## Groomer variant (groomer portal :43617)

Same as B + E, but the groomer onboards as a **verifier** via apply→approve:
1. Groomer Setup → genesis/unlock → **Whitelist → Fill demo data** (groomer preset): this fills the
   **verify purposes** field (`grooming_intake/boarding_intake/daycare_access`) → **Submit application**.
2. Fund the relayer signer: `scripts/demo-bootstrap.sh 0x<groomerSignerAddress>` (a groomer is a
   verifier/relayer - no `ISSUER_ROLE` needed; the script funds PLASMA + re-grants the VERIFY whitelist).
3. Admin portal → step A.3 click **Groomer preset** → **Approve** → this whitelists each
   `VERIFY:<purpose>` on-chain (`key = keccak256(abi.encode("VERIFY:", keccak256(label) mod r))`,
   `whitelistFor(verifyKey, groomerRelayer)`).

The groomer is then an authorized verifier for those purposes (gated separately from issuer roles), and
the §E owner-hidden consent-proof flow works against it.

For the shop-application demo, create a **Client** (with a pet) → book an **Appointment** → run §E
**from that appointment**, then find the result under **All verifications**, filtered by that client
or appointment. The groomer's own book is §H (pets) and §I (calendar), neither of which needs custody
unlocked. The groomer portal has no issuance surface at all: no "Issue a record" entry, no
Records page, and `BUSINESS_TYPE=groomer` does not mount the issuance routes on its backend either.

### The direct credential check, on the groomer's own Verify page

Beside the owner-consent flow, both the vet and groomer **Verification** pages carry a paste-a-record
panel. It runs the same checks §F shows you, and the same issuer pillar is mandatory there: a record
whose `issuer.documentStore` has been repointed comes back **not valid** with the issuer contract
mismatch named, distinctly from a record whose signer simply is not whitelisted - two different
accusations with two different remedies.
That pillar needs `FACTORY_ADDR` configured, which `demo-up.sh` passes to every verifier it starts.

---

## What this guide does not walk you through

Two things landed recently that need a device and a build this guide cannot stand in for. They are
listed here so their absence is not mistaken for them being missing from the product.

- **Android on-device consent proving.** Android can now generate the `DogTagConsent` Groth16 proof on
  the phone, which it could not do before; §E's step 4 is the flow. Exercising it needs an **arm64**
  device or emulator (the prover ships arm64-only), the native libraries built with `cargo ndk`, and the
  proving artifacts vendored with `make vendor-mobile-artifacts` - none of which is in the repo, all of
  which is in `docs/MOBILE_BUILD.md`. There is a one-command self-check: the debug build's Profile screen
  has a ZK self-test that proves and checks all seven public signals on-device.
- **§A1 step 5 and all of §G** need the rebuilt app installed, as flagged at those steps.
