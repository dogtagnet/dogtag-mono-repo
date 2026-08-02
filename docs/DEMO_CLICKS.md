# DogTag - literal click-through (LIVE on ROAX)

The exact buttons to press, in order, against the **live ROAX deployment** (chainId 135; contract addresses come from `contracts/.env` / `contracts/deployments/roax.json`).
The demo buttons fill every form, and all passwords are prefilled, so the operator **types nothing** except the `dogTagId` handle in §C.
(Testnet only.) For the full runbook + phone networking + gotchas see **[DEMO.md](./DEMO.md)**.

Boot first: `scripts/demo-up.sh` (or with a public tunnel on corporate Wi-Fi:
`VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh` - see DEMO.md §6).
This also boots the **prover-service** on **:41875** (`POST /prove-consent`, the trusted server-prove fallback - see §F) and the browser-based **pet-owner (holder) wallet** on **:45931** (a phone-free way to receive/hold/share records; owner-hidden proving stays on the native apps - see [`stacks/owner/web/README.md`](../stacks/owner/web/README.md)).
Automated equivalents: `scripts/e2e-smoke.sh` (credential lifecycle, 7 steps) and `scripts/e2e-zk.sh` (the owner-hidden consent proof, end to end with a real proof).

> **You do not need to rebuild anything for the browser sections.**
> `demo-up.sh` runs `cargo build --release` for all four backends itself, and serves every portal with
> `vite dev` straight from source, so one boot picks up whatever is on your branch.
> The **only** things that do not refresh themselves are the **phone apps**, which carry a
> compiled Rust core and bundled proving artifacts. Rebuild those by hand when you want §H, following
> `docs/MOBILE_BUILD.md`.

Portals: **admin** http://localhost:39741 · **vet** http://localhost:41873 · **groomer** http://localhost:43617 · **government** http://localhost:44831 · **owner wallet** http://localhost:45931
Backends: admin `:39742` · vet `:41874` · prover `:41875` · groomer `:43618` · government `:44832` · oversight indexer `:46001`
Demo passwords (prefilled): operator `operator`, admin `admin`. Record type in the vet flow: **VACCINATION**.

---

## Two facts to hold before you start

Both change how you read everything below, and neither was true when this guide was last revised.

### 1. The shop stores are backed by MongoDB, so shop data survives a restart

vet-api and groomer-api are started with `MONGO_URI` (the demo uses `mongodb://127.0.0.1:27018`), so
**clients, pets, appointments and issued records persist across a backend restart**.
Earlier revisions of this guide were written when those lived in an in-memory store, and several notes
implied a restart wipes them. It does not.

What a restart *does* still reset: **operator sessions** and **custody** (see the box below), plus the
**government** stack's own records, which run on an ephemeral `MemStore` in demo mode.
So after a restart expect your clients and pets to still be there, and expect to sign in and unlock again.

### 2. Nothing is repointed to the generation-2 contracts

Eight generation-2 contracts were deployed live on ROAX on 2026-08-01 (`_s14_cutover` and
`_c7_typed_resolvers` in `contracts/deployments/roax.json`): `ProviderRegistry`, `DogTagIssuerV2Impl`,
`DogTagIssuerFactoryV2`, `CloneProvenanceRouter`, `VerificationRegistryConsentV2`, `ProtocolRegistryV2`,
`ProviderDirectory` and `ServiceDomainResolver`.

**Every portal, every backend and both phone apps still read the generation-1 set, deliberately.**
Deployed is not the same as wired: repointing clients is a separate, captain-authorised step that has
not happened. So when you see a generation-1 address in a portal, that is correct and not a stale
config. §N walks the one surface that tells you this out loud.

Verified on chain while writing this guide:

```
cast call 0xf374f4cA5ebBBAFf0dFcE48D8Cda2e47F9D5da01 "generationCount()(uint256)" --rpc-url https://devrpc.roax.net   # 2
cast call 0x25a318a0Bf83a7ea64fB0a7b1cDe8847722C7bC0 "resolverApproved()(bool)"   --rpc-url https://devrpc.roax.net   # false
cast call 0xD3B121FEaCde93b95288912EAdbB10824550FdBF "boundCloneCount()(uint256)" --rpc-url https://devrpc.roax.net   # 0
```

---

## Which stack am I on? Check before reporting a bug

Two sections below (§M oversight, §O the mirror) behave **completely differently** depending on how the
oversight indexer was booted, and the difference is not visible in any portal.
A stack booted by `scripts/demo-up.sh` is not the only shape you may meet: the indexer is often started
by hand against the live chain instead. One command tells you which you have:

```
curl -s http://localhost:46001/health
```

| `/health` says | You have | Expect |
|---|---|---|
| `"simulated":true, "chainId":null` | the `demo-up.sh` indexer (`INDEXER_DEMO_MODE=1`) | scripted rows with placeholder tx hashes; the two well-known tokens work; the content mirror accepts uploads |
| `"simulated":false, "chainId":135` | a hand-booted live indexer | oversight routes **401** unless `INDEXER_SCOPES` was authored; the content mirror **503**s unless `MIRROR_INGEST_TOKEN` was set |

`chainId` is `null` in the simulated case because a scripted source is on no network at all, and **both
keys are emitted on both paths**, so their absence can never be mistaken for "live".
A build predating this reports neither key, which is exactly the ambiguity it removes.

The tunnel hostnames are the other thing that moves.
`demo-up.sh` gives **each backend its own** public URL, and they **rotate on every restart**, so never
copy one out of this guide or out of an old note.
Read the current one from the backend itself: government publishes it at `GET /v1/../health` as
`deploymentUrl`, and for vet and groomer it is whatever you passed as `VET_PUBLIC_URL` /
`GROOMER_PUBLIC_URL` (otherwise `http://$LAN_IP:<port>`), which is also the host printed inside every QR
that backend mints.

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
> there as a direct link. Both fields prefill in demo mode, so it is one click: **Unlock and continue**.
> A wrong passphrase shows an inline error and does **not** trigger the "Session expired" path above.
> (Walked: the banner, the in-place prompt and the one-click unlock all behave as described.)

---

## A0. Device creates a self-custodial wallet - DogTag app

> **NOT WALKABLE IN A BROWSER.** This step and every other step marked with this box needs a physical
> phone or an arm64 simulator running a **rebuilt** app (`docs/MOBILE_BUILD.md`).
> `demo-up.sh` does not rebuild the apps. Nothing in this section was exercised while revising the guide.

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
     `VERIFY:<purpose>` on-chain (from the application's **verify purposes** field) - see §F and the
     Groomer variant.

> The left-hand nav is **Dashboard · Activity · Issuers / Factory · Onboard issuer · Business registry ·
> Issuer applications · Whitelist · Governance · Verification bench · Settings**.
> **Settings** is new since the last revision of this guide (§P).

> **The admin portal degrades one page at a time when its `VITE_*` addresses are missing, and it says so.**
> `demo-up.sh` passes them; a hand-started `vite dev` typically does not.
> On a portal started without them, **Whitelist** reports *"VITE_ISSUER_REGISTRY_ADDR is not set - the
> live on-chain column is unavailable"* while grant/revoke still work through the backend, and the
> bench's issuer-domain row reports itself unconfigured (§L).
> Neither is a fault in the chain or in your data. Check how the portal was launched before chasing it.

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

> **Steps 3 to 5 are NOT WALKABLE IN A BROWSER** - they are the phone's half.
> Read them as a description of what the device does, not as buttons you can press today.

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

> **You can still walk §C onwards without a phone.** The vaccination Issue form takes any handle you
> type, and none of the browser sections below need a minted `DOG_PROFILE` SBT to exist. What you lose
> by skipping the phone is only the owner's own copy of the tag.

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

> **If the stack has already been through Setup once, skip this section.** Custody survives a restart as
> a sealed file, so the wizard is a first-boot step, not a per-session one. What you do need after a
> restart is the one-click unlock from the box near the top.

---

## C. Vet issues a vaccination credential → IMPORT QR - vet portal (:41873)

1. Go to **Issue a record**.
2. Click **Fill demo data** (valid rabies cert; recordType `VACCINATION`). This fills the cert fields
   but **leaves `dogTagId` blank** - the demo-fill no longer clobbers it (a fixed footgun).
   It also fills a **microchip** number (`985112636323787` in the demo data). Note it: §I compares that
   exact leaf against what the shop has on file for the pet.
3. **Set the `dogTagId` field = the dog tag's handle from §A1** (the numeric `dogTagId` the Register-pet
   wizard allocated). It **must match**: the credential's `dogTagId` leaf is what ties the record to
   the sealed tag (on-chain the tag key is `field_of_value(handle)`), and the phone attaches an
   imported record to a pet by that handle - a mismatch leaves the record orphaned from the tag, so it
   cannot be presented for that pet in §F.
4. **Sign & Issue**. The result card flips to **Verified on-chain** and prints the Merkle root and the
   anchoring tx.
5. **Create QR** → the **IMPORT** QR (device ← vet) renders. It carries a SHORT one-time token
   (`https://<vetHost>/r/<32-hex>`), NOT an embedded record payload - a low-density QR the camera focuses
   on instantly. The token is **deleted after the first scan** (one-time; 180s expiry), so re-scanning
   the same QR yields a 404.

> **The "Valid until" field is the knob §H uses.** It is a required date on this form and the demo fills
> it a year out. Type a date in the **past** instead and you get a genuinely issued, genuinely anchored,
> genuinely expired credential - which is the cleanest way to watch a badge tell the truth (§H) and the
> cleanest thing to feed the bench (§E).

---

## D. Phone (DogTag app) - scan → import → verified on-chain → view fields

> **NOT WALKABLE IN A BROWSER.** Needs a rebuilt app and a device on a network that can reach the vet's
> QR host. Nothing here was exercised while revising the guide.
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

## E. Verification bench - throw a record at it and watch every check answer

**Admin portal → Verification bench** (`:39741/bench`).
This is the page that shows you, one row at a time, what verification actually establishes: every check
answers **Pass**, **Fail** or **Could not run**, and each row shows the evidence it rests on and which
contract was asked.
Nothing here is simulated; the rows come from the same verification path the wallet and the verify
panels use, and the reads are made against the real chain.

### E1. Get a record into the box

The bench takes a **wrapped credential JSON**. The government portal is the one reliable source of one,
and the wallet route below builds on it rather than replacing it:

- **Government portal (:44831) → Issue** → pick **TRAVEL_CLEARANCE** → **Fill demo data** → set the
  **Dog tag id** (it is deliberately never demo-filled) → **Issue + anchor** → click
  **Copy wrapped document**.
  That record is anchored live on ROAX **only if this stack booted with a funded, whitelisted
  `GOV_SIGNER_KEY` and a `TRAVEL_CLEARANCE_ISSUER_ADDR` clone**; `scripts/demo-provision-government.sh`
  provisions both, and `demo-up.sh` prints a warning at boot naming whichever is missing.
  The portal tells you which you got: the top bar reads **LIVE CHAIN** with *"can anchor on-chain"*
  beside the signer, and the result card badges **✓ anchored on-chain**.
  Without them `/issue` only dry-runs: the card badges **built (not anchored)**, and benching that record
  gives **Verifier verdict: not valid** with *"Was this issued by a contract that genuinely descends from
  the DogTag factory?"* reading **Fail** ("The factory has NO record of this root"), not the Pass rows in §E2.
  Prefer `TRAVEL_CLEARANCE`: on the fresh contract set it is the record type with a deployed clone and a
  whitelisted government signer. `EU_HEALTH_CERT` may have no clone provisioned, in which case issuing
  it only dry-runs the same way.
- **Owner wallet (:45931)** → open a held credential → **Share a redacted copy** →
  **Copy redacted credential**. Useful on its own: redaction leaves the Merkle root untouched, so a
  redacted copy of an **anchored** credential still passes every on-chain row.
  That qualifier is the prerequisite: the held credential has to be one from a real issuance, so paste
  the government JSON from the bullet above into the wallet's **Receive a credential** box first.
  The wallet's two **Fill sample** buttons are the only zero-setup way to fill it, and their documents
  were never anchored on any chain, so benching a redacted copy of one fails the on-chain rows in exactly
  the way the dry-run case above describes.

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

### E2. Run it on a genuine record

1. Paste the JSON into **Wrapped document JSON**.
2. Click **Run the checks**.

On a genuine, anchored, in-date record you get **Verifier verdict: valid** and these **eleven** rows.
(Walked on a live-anchored `TRAVEL_CLEARANCE`; earlier revisions of this guide listed nine, and two of
the row questions have been reworded since.)

| Row | Expect | In the verdict? |
|---|---|---|
| Is the document's content intact - does it still hash to the root it claims? | Pass | yes |
| Was this issued by a contract that genuinely descends from the DogTag factory? | Pass | yes |
| Does the document name the same contract the factory says issued it? | Pass | yes |
| Was the signer that issued this authorised for this record type **when it anchored the root**? | Pass | yes |
| Is this root actually anchored on-chain by its issuing contract? | Pass | yes |
| Has the issuer revoked this credential? | Pass | yes |
| Is this credential still within its validity window? | Pass | **no** |
| Is the registry this client is configured with the one that gates this issuing contract? | Pass | **no** |
| Was that signer authorised at the moment this root was anchored? | Pass | **no** |
| Does the domain the document claims match the one the issuer published on-chain? | **Could not run** | **no** |
| Does that domain's DNS zone name this issuing contract back? | **Could not run** | **no** |

The last two are **expected**; §L explains them.

**Two of these rows are the point of the current release, so read them rather than skimming.**

*"Was the signer that issued this authorised for this record type **when it anchored the root**?"* used to
ask whether the signer is whitelisted **now**. It asks about the past instead, and its own finding says
why: *"Whether it still holds it today is a separate question, and deliberately not this one: delisting is
forward-only."* Its evidence line is a **grant history** read from the governing registry's `Whitelisted`
/ `Delisted` log, folded at the block the root was anchored, not a current-state getter.
§G walks both directions of that.

*"Is the registry this client is configured with the one that gates this issuing contract?"* is new, and it
is **advisory**. It reports a fault in **this client's own configuration** (its factory and registry
addresses naming contracts that do not go together), never a fault in the credential. That is why it sits
outside the verdict.

Scroll to **Reads made** for the raw material: every chain read the verifier actually performed, with
the contract, the method and the answer. On a walked run this listed `rootIssuer` against the factory,
then `isRevoked` / `issuedAt` / `isValid` / `recordType` against **the clone the factory named** - never
against the address the document names. That ordering is the whole issuer pillar in one table.
If the chain head was readable, the description above the rows tells you the block every read was pinned
to, so the report is reproducible.

### E3. Now try to slip a forgery past it

Bottom of the first card: **Try to slip a fraudulent record past the checks**.
Each button tells one specific lie with the record above and re-runs everything.
Read the "what will NOT catch this" list on each card as carefully as the result.

There are **five** buttons now (this guide previously described three):
**Relabel the issuer's name**, **Point it at a different issuer contract**, **Tamper with a covered
field**, **Present it as an expired record**, and **Forge a longer validity window**.

Click these in order and watch **which** row objects:

1. **Point it at a different issuer contract** → **Apply & re-run**.
   The verdict flips to **not valid**. The row that names the lie is *"Does the document name the same
   contract the factory says issued it?"*, now **Fail**, and it reports both addresses side by side: the
   contract the factory names, and the contract the document names.
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
   not the display name printed beside it. Binding the name is what the issuer domain work in §L is for.
4. **Forge a longer validity window** is the interesting new one. It is caught by **integrity**, not by
   the expiry row: `validUntil` is a covered leaf, so moving it breaks the Merkle root. The expiry row
   only ever reports an **honestly** expired credential (§E4).

Re-pasting between mutations matters - they stack, and the amber **Applied to the loaded record** strip
tells you what is currently applied.

### E4. The row that fails while the verdict stays valid

Issue a record with a **past** "Valid until" (§C, the note), then get it into the bench by the vet route
from §E1: **Records** → the row's **QR** → paste that URL into **QR share link** → **Fetch**.
You get **Verifier verdict: valid** sitting above a red *"Is this credential still within its validity
window?"* row.

That is not a contradiction, and the page marks it: rows tagged **not in the verdict** are reported
beside it rather than folded into it.
The chain records anchoring and revocation and has **no concept of a validity window**, so an expired
credential really is on-chain-valid.
The verifier's verdict is integrity, on-chain status and the issuer pillar; expiry, the configured-registry
row and the two domain rows are reported next to it so you can see them without them silently changing
the answer.

For the opposite case, revoke the record and bench it again: *"Has the issuer revoked this credential?"*
turns **Fail** and the verdict goes **not valid**.
Click the row's **QR** once more for that second run, since the first **Fetch** consumed the token.
Revoking is vet portal → **Records** → the row's **Revoke** button, which asks you to confirm ("Revoke
this credential on chain? It stays on record (as revoked) and remains verifiable."). The government
portal's Records page has the same action for its own records. Nothing is deleted either way: the row
keeps its original issuance proof and gains a revoke-tx proof beside it.

---

## F. (Optional) Owner-hidden proof-of-verification on-chain - vet or groomer portal

> **Step 4 is NOT WALKABLE IN A BROWSER** - it is the phone's half, and it is where the proof is made.
> Steps 1 to 3 (starting the session and rendering the QR) are browser-only and do work.

The owner proves consent to the groomer without revealing who they are (symmetric counterpart of the §C-D import).
There is **no mode picker** - owner-hidden ZK consent is the only verify flow.

1. Open the verify surface: in the **vet** portal the **Verification** tab; in the **groomer** portal
   **Appointments** → open the booking → **Start verification** (the result is then filed against that
   visit and its client, and is searchable under **All verifications**), or **Ad-hoc verification**
   for a walk-in with no booking.
2. Pick a **Purpose** from the dropdown (e.g. boarding intake).
3. **Start export** → the session QR renders. It carries the groomer's wallet address + a
   one-time token + host: `https://<groomerHost>/x/<token>?a=<groomerAddr>`.
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

## G. The attack catalogue - eleven complete frauds, each caught by a named check

**Admin portal → Verification bench**, the card below the mutation buttons: **The attack catalogue**.
This is new since the last revision of this guide, and it is the fastest way to see the whole guarantee
in one click.

Unlike the §E3 buttons these need **no loaded record and make no network call**. Each is a complete
fraudulent record with **its own scripted chain**, so they can pose chain states a live chain cannot be
asked to produce: a signer delisted after it issued, a contract the factory never deployed vouching for a
root, a registry that does not govern the clone.
**Each scenario declares which check must refuse it, and a run that diverges from that declaration is
called out in red.**

Click **Run the whole catalogue**. Walked result: all eleven matched their declarations, no red.

| Scenario | Verdict | Why it is in the set |
|---|---|---|
| A genuine credential (**control**) | **valid** | Without it, a catalogue of nothing but frauds would look perfect against a verifier that refused everything. |
| A covered field was edited after issuance | not valid | integrity |
| A contract the attacker controls vouches for the record | not valid | issuer-descends-from-factory |
| A real credential relabelled as a different record type | not valid | issuer-whitelisted |
| **The signer was delisted BEFORE the root was anchored** | **not valid** | the credential claims an issuance that could not have happened |
| **The signer was delisted AFTER the root was anchored** | **valid** | delisting is forward-only; a genuine credential must survive a key rotation |
| A revoked credential presented as live | not valid | not-revoked |
| The authority this client is configured with does not govern this issuer | **valid** | the objecting row is advisory: our mis-pairing is not evidence about the credential |
| A perfectly-formed credential that was never anchored | not valid | issuer-descends-from-factory |
| A record claiming membership of an anchored batch | not valid | integrity |
| The endpoint is a different chain | **no verdict** | the verifier fails closed and produces no answer at all |

**The two delisting rows are the pair worth pressing individually.**
They are the same credential shape differing only in *when* the delisting happened, and they must come out
opposite ways. A verifier that read the whitelist as current state would refuse both, turning every
ordinary key rotation, retirement or lapsed licence into a fleet-wide forgery verdict against genuine
credentials. The "after" scenario is the regression detector for exactly that, and its own note says so:
both whitelist rows pass, and revert the verifier to a current-state read and the gating row alone turns
red beside a green historical one.

> **The real-chain version of that pair is deliberately not in this guide.** Producing it means calling
> `delistFor` against a live signer on the admin **Whitelist** page, which retires that signer for every
> future issuance on this deployment. The scripted scenarios above prove the same property with no chain
> write, so use them. If you do want the live version, use a signer you are willing to delist, and expect
> to re-grant it afterwards.

Two more results are worth reading rather than counting:

- **"The endpoint is a different chain" produces NO verdict, and that is the assertion.** Not one row is
  a `fail`. The chain guard refuses to send an address-bound read to a peer reporting the wrong chain, so
  every on-chain row reports **could not run** naming the mismatch, and the verdict is absent rather than
  `false`. "The factory has no record of this root" would be an accusation nobody was in a position to
  make. This is the same guard you can drive by hand in §P.
- **The mis-paired-registry scenario stays `valid`.** The row that objects is the advisory
  configured-registry row from §E2. The credential is genuine; the fault is ours.

> **Two rows read "could not run" for every scenario in this card, BY CONSTRUCTION.** No scenario
> configures an `IssuerDomainRegistry`, so `issuer-domain-claim` and `issuer-domain-dns` never run here.
> The card says so itself. That is not a finding about any scenario - load a real record in §E to
> exercise that axis, and read §L for what it will say.

---

## H. Watching a badge tell the truth - DogTag app

> **NOT WALKABLE IN A BROWSER.** Needs the phone app, rebuilt. The apps carry a compiled Rust core and
> bundled proving artifacts and do not refresh with `demo-up.sh`; build and install per
> `docs/MOBILE_BUILD.md` before expecting the behaviour below.

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
by unit tests. The Android side runs from a plain checkout:

```
cd apps/android && gradle :app:testDebugUnitTest \
  --tests '*VerdictBadgeStalenessTest' --tests '*CredentialExpiryFromDocTest' \
  --tests '*RefreshCannotUpgradeVerdictTest'
```

(It needs `apps/android/local.properties` with `sdk.dir=...`, which is gitignored, so write it once.)
The iOS mirror is `DogTagTests/VerdictDisplayTests`. What no test covers is the phone actually rendering
the badge, which is why the steps above are the way to see it.

---

## I. Pets, and the microchip cross-check - groomer portal (:43617)

> Prereq: the groomer portal is up and you are signed in (Groomer variant, below).
> **No custody unlock and no whitelist is needed for this section** - it is the shop's own book. The
> "Custody is locked" banner may be showing; read-only and CRM pages stay reachable behind it.
> (Walked: a client and pet were created with custody locked throughout.)

A pet is addressable in its own right, not just a line inside a client.

1. **Clients → New client**. Fill name/phone/email, click **Add pet**, give a pet name and breed, then
   **Create client**. The pet block also has a **Microchip** field - leave it blank for now.
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
   tag id. It is refused, and the message names the pet already holding it and that pet's owner.
6. **Search** takes pet name, breed, species, **the DogTag id**, or **the owner's name** - searching the
   owner returns all their pets, searching the tag id returns the one pet holding it.

The pet page also states plainly that this portal cannot mint or revoke anything: a groomer is
whitelisted to VERIFY, not to issue, and adding or removing a tag here only edits the shop's own record.

### I1. The microchip cross-check

Linking a DogTag to a pet is otherwise an unchecked act of typing. If the shop holds a credential for
that tag, the tag binding now **compares the credential's microchip leaf against the microchip on the
shop's own pet record**, and reports one of **four** outcomes right under the DogTag block.

**The rule that matters most: an absent microchip is completely normal and never blocks anything.**
Many animals are not chipped - cats routinely are not - so the check only fires when **both** sides carry
a code.

- **Not compared** (neutral). What you get from step 4 above with the microchip left blank.
  Walked: linking DogTag 7 to a pet with no microchip **succeeded**, and reported
  *"Microchip not compared - this shop holds no credential for that DogTag, so there is nothing to
  compare the microchip against."*
  Other neutral reasons in this family: the pet has no microchip on file, the credential carries none, or
  no pet holds that tag. These are **facts**, and they render neutral, not as warnings.
  A separate group of reasons in the same state are **failures** to look (a read that did not resolve),
  and those do warn. The two are told apart deliberately, so an unchipped cat never looks like a fault.
- **Match**. Both sides carry the same code. The link is recorded as normal.
- **Mismatch - the link is REFUSED** (409 Conflict). The credential describes a different animal.
  Note what this does **not** do: it does not touch the credential's verdict. The credential is genuine
  and stays valid for everyone; what is refused is the **pairing**. That is the same discipline that keeps
  "this document names the wrong contract" apart from "this signer is not authorised" in §E - different
  accusations, different remedies.
- **Microchip could not be read** (loud, and it still does **not** block the link). The credential
  carries microchip-shaped data at a key path this build does not recognise, and the message **names the
  paths it found**.

**That fourth state is worth understanding, because it exists to stop this whole feature going silently
inert.** The check first shipped reading a single key path that no real issuer emits, so it was dead on
every real credential while passing all its own tests. What let that survive is that "the credential has
no microchip" and "the credential has a microchip somewhere I do not recognise" produced the *same* quiet,
benign answer - our own not-comparable state camouflaged a broken reader. They are now structurally
different outcomes: the first is silent and ordinary, the second is loud and names the paths.
It still does not refuse the link, because it is evidence that **our reader** is wrong, never evidence
about the animal.

For the same reason the reader matches on a key-path **suffix** across the four shapes real issuers
actually emit: the vet portal's top-level `microchip.code`, the schema-conformant
`credentialSubject.microchip.code`, government `EU_HEALTH_CERT`'s `credentialSubject.microchipNumber`, and
government `TRAVEL_CLEARANCE`'s `credentialSubject.animal.microchipNumber`.

> **The match and mismatch outcomes could not be walked in a browser, and here is exactly why.**
> Both need this shop to already hold a credential for that tag, and on this deployment a shop acquires
> one through **Import from user**, whose form wants a **Customer API base + Record reference + Customer
> JWT** - the customer app's own pull triple. Pasting a vet `/r/<token>` share URL into "Scanned payload"
> does not populate those fields, so that route is phone-shaped like §D.
> What was walked is the neutral **not compared** outcome and, more importantly, that a blank microchip
> **does not block the link**.
> The refusal itself is covered by `cargo test -p vet-api --test microchip_binding` (35 hermetic tests,
> including the mismatch 409 and the six write points it guards).

> **One thing to check if you edit a client afterwards.** `PUT /clients/{id}` replaces the owner's whole
> pet list, so a client edit that omits the microchip **erases it silently**. The portal's own form
> re-sends it, so this only bites custom API callers - but it is why the field is worth re-checking after
> an edit.

### I2. What else is on the pet page now

Two blocks the previous revision of this guide predates:

- **Credentials** - the credential documents this shop holds for that pet, "each re-checked against the
  chain now rather than trusting the verdict from when it was imported". With nothing held it says so
  explicitly, and it is careful about the claim: *"That is a statement about THIS shop's records only -
  the owner may well hold credentials it has never shared."*
- **On-chain discovery** - reads the chain for everything recorded against that pet's DogTag, **including
  records this shop never created**. Nothing is written. It names the endpoint it used and says it goes
  "through the chain-guarded endpoint selection", which is the §P setting.
  A tag id is what it keys on, so it tells you to link one first if you have not.

---

## J. Calendar - the shop's book, and the client's copy

> Same prereq as §I: groomer portal, signed in, no custody needed.

### J1. The shop side

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

### J2. The client side - scan one appointment, get it in your own calendar

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
> **The failure to watch for on a `demo-up.sh` stack is the opposite one.** Only a loopback
> `DEPLOYMENT_URL` is withheld like this, and the script defaults `LAN_IP` to an address baked into it
> rather than detecting your Mac's, documenting `LAN_IP=192.168.x.x scripts/demo-up.sh` as the override.
> A LAN address is not loopback, so if the baked-in one is not your Mac's current address the QR is drawn
> exactly as normal and the phone simply cannot reach it.
> Check the host in the link shown beside the QR before concluding the handoff is broken.

---

## K. Per-row on-chain provenance, and links that refuse to be offered

### K1. The three activity tables

The activity tables judge **each row** on whether its transaction hash is one a chain could actually
address, and label it. The three places this appears:

- admin portal → **Activity**
- government portal → **Oversight**
- vet portal → **Traceability**

Each row carries either **chain-addressable**, whose transaction hash is a working
`explorer.roax.net/tx/...` link, or **not chain-addressable**, whose hash is rendered as inert text with
**no link at all**. The label is decided per row from the hash itself, not from a demo flag, so a single
scripted row inside an otherwise real feed is still caught.

**What you actually see depends on how the indexer was booted** - go back to "Which stack am I on?" and
run the `curl` first. There are three distinct states and they look nothing alike:

| Indexer | Admin → Activity | Government → Oversight, vet → Traceability |
|---|---|---|
| `demo-up.sh` (`simulated:true`) | *"Oversight indexer not connected"* | scripted rows, **every one** labelled **not chain-addressable** |
| hand-booted live, no `INDEXER_SCOPES` | *"Oversight indexer not connected"* | **`indexer returned 401`** and zero counters |
| live with scopes authored | needs wiring, see below | real rows, real explorer links |

- **Admin → Activity is unconfigured on both stock stacks, and that is not a bug.** `demo-up.sh` passes
  the indexer's base and token to the vet, groomer and government backends but **not** to admin-api,
  which defaults to no feed and answers those routes 503. Walked: the page reports *"Oversight indexer
  not connected - set INDEXER_API_BASE on the central backend to enable it."*
  Wiring it needs **both** `INDEXER_API_BASE` and `INDEXER_OVERSIGHT_TOKEN`, not the base alone:
  admin-api builds the feed from the base and defaults the bearer to empty, and logs its own warning
  that the indexer will 401 without the token.
- **On a `demo-up.sh` stack, expect every row to say "not chain-addressable" and do not report it as a
  bug.** The scripted history uses placeholder hashes like `0x0800`. Those are perfectly good demo data
  and cannot be transaction hashes on any chain, so labelling them and refusing to link them is the
  correct answer.
- **On a hand-booted live indexer, expect a 401 instead of rows.** Walked: government → Oversight showed
  `indexer returned 401: {"error":"unknown or unauthorized token"}` above six zeroed counters.
  The cause is that a non-demo indexer with an empty `INDEXER_SCOPES` registry **fails closed on every
  query by design**, while the backends are still presenting the two well-known demo tokens, which only
  exist in demo mode. Authoring `INDEXER_SCOPES` and re-issuing matching tokens to the three backends is
  what turns this on; it is out of scope for this guide.
  **Read the empty state carefully here.** The page prints "No on-chain activity yet." underneath the
  error. The error is the real answer; the emptiness is a consequence of it, not a statement about the
  chain.

### K2. No link is offered for anything that is not on chain

This is a rule across the whole admin portal now, not just the Activity table.
Wherever a transaction hash or an address is shown, the portal decides whether that value could be looked
up before it offers a link. Three states, and the middle one is the one to check:

- **Linked.** A well-formed value; the explorer link works. Walked on the vet **Records** page, whose row
  showed the anchoring tx and block with a live link beside them.
- **Present but not addressable.** The value is rendered **struck through in amber** with the reason
  beside it, and **no anchor at all**. It is deliberately not a tooltip: a state you can only discover by
  hovering survives neither a screenshot nor a touch device.
  Because the displayed value is truncated and its middle characters are genuinely not in the page, this
  state always carries a **copy button**, so the full value is still recoverable.
- **Absent.** Nothing is claimed and nothing is shown.

> This state needs a value that failed to reach the chain, so it is not reachable on a freshly booted
> stack with no proposals or scripted rows in it. It was not walked while revising this guide, for that
> reason. The most likely places to meet it are the **Whitelist** page after a grant that came back
> `proposed` rather than `executed`, and **Issuer applications** after an approval that broadcast
> nothing.

---

## L. The issuer domain binding - and the two rows that will say "Could not run"

The bench's last two rows ask whether the issuer's claimed domain is really theirs:

1. *"Does the domain the document claims match the one the issuer published on-chain?"*
2. *"Does that domain's DNS zone name this issuing contract back?"*

**Both report "Could not run" today, on every surface, and that is correct behaviour rather than a
fault.** But **the reason has changed since the last revision of this guide, and the old reason is now
wrong.**

**`IssuerDomainRegistry` IS deployed on ROAX**, at `0xD3B121FEaCde93b95288912EAdbB10824550FdBF`, and it is
in `contracts/deployments/roax.json`. Earlier revisions said it was absent from the ledger and undeployed.
Deploying it published no claims, though: `boundCloneCount()` is **0**, so no issuing contract has bound a
domain yet.

So read the row's own **finding line** to know which of two situations you are in. They both say "Could
not run", and they mean different things:

- *"No issuer-domain registry is configured for this bench."* The portal was started without
  `VITE_ISSUER_DOMAIN_REGISTRY_ADDR`. This check reads **no default address on purpose** - the contract
  set is still being revised, and reading a constant that may have moved would be worse than saying
  nothing. `demo-up.sh` **does** pass it, resolved from the ledger, so a `demo-up.sh` stack should not
  show this; a hand-started `vite dev` will. (This is the one that was walked.)
- *"This issuer has published no domain claim."* The address **is** configured, the registry **was** read,
  and it holds no binding for this contract. The reason line spells out the consequence: the document's
  claimed domain "cannot be corroborated or contradicted". This is the normal day-one state, and it is
  what a correctly-configured stack shows today, given `boundCloneCount() == 0`.

The DNS half is unchanged and will not move until a verifier backend is in the loop:
*"The TXT lookup is resolved server-side by the verifier backends; this bench runs in the browser and
cannot perform it. A passing on-chain claim above is NOT evidence that DNS agrees."*

They are two separate rows rather than one merged green tick precisely so that neither can imply a check
that never ran. Neither feeds the verdict, so their absence changes no answer - it only means the
issuer's **name and domain** are unproven, which is the same gap §E3's third mutation demonstrates from
the other direction.

**Binding a domain is what turns the first row on.** Until someone does, expect **Could not run** with
the "published no domain claim" wording, in the bench and anywhere else a binding is shown.

---

## M. The generation-2 contracts are live, and nothing reads them

This section exists so that a tester who has heard "the registry cutover shipped" does not misread every
address in every portal.

Eight contracts were deployed to ROAX on 2026-08-01. **Not one client points at any of them.** The
generation-1 `DogTagIssuerFactory`, `IssuerRegistry`, `VerificationRegistryConsent` and `ProtocolRegistry`
remain the addresses every portal, every backend and both phone apps read.

There is nothing to click here, and that is the point - but there are three things you can check in about
a minute, and one surface that states it out loud:

1. **§N is the surface.** Open the vet or groomer portal → **Provider self-service**. It refuses to do
   anything and says why, in these words: *"The generation-2 contracts are deployed but no client reads
   them yet, so a baked address here would repoint this deployment by accident."*
2. **The ledger names them under distinct keys** (`ProviderRegistry`, `DogTagIssuerV2Impl`,
   `DogTagIssuerFactoryV2`, `CloneProvenanceRouter`, `VerificationRegistryConsentV2`, `ProtocolRegistryV2`,
   `ProviderDirectory`, `ServiceDomainResolver`). The distinct names are deliberate: `demo-up.sh` resolves
   ledger keys **by name**, so reusing a generation-1 key would have silently repointed a running stack.
3. **The chain agrees.** The three `cast` calls at the top of this guide show the router carrying two
   generations, the typed directory resolver **not approved**, and the domain registry **empty**.

Two consequences worth stating so they are not mistaken for defects:

- **The provider directory has no on-chain source yet.** The indexer still serves its provider directory
  from the admin business registry, not from `ProviderDirectory`. A cold indexer with no `ADMIN_API_BASE`
  answers `GET /v1/businesses` with *"provider directory has no successful source snapshot yet"*, which is
  a 503 and honest: never-loaded is not the same as empty.
- **The mandatory issuer-whitelist pillar (§E) does not answer for a generation-2 root.** It asks the
  generation-1 registry. Since nothing issues through generation 2, nothing is exposed by this today, but
  it is a cutover blocker rather than a wiring detail.

---

## N. Provider self-service - vet and groomer portals

**Vet portal → Provider self-service**, or **groomer portal → Provider self-service** (`/provider` on
either). A new nav item since the last revision of this guide.

This is the provider-facing surface for the generation-2 set: deploy your own issuer clone, choose which
of your contracts is current, claim a domain, and publish your listing (pins, contacts, profile). Every
write is a wallet transaction to a contract the provider owns; there is no backend on this path at all.

**Walked result: the page refuses to start, and the refusal is the useful part.** It reads:

> **Provider self-service is not configured.**
> This page reads the generation-2 registry set, and the addresses are not set on this deployment.
> **Nothing about your provider record has been checked.**
> Set: `VITE_PROVIDER_REGISTRY_ADDR`, `VITE_DOGTAG_ISSUER_FACTORY_V2_ADDR`,
> `VITE_SERVICE_DOMAIN_RESOLVER_ADDR`, `VITE_PROVIDER_DIRECTORY_ADDR`.
> There is deliberately no built-in default. The generation-2 contracts are deployed but no client reads
> them yet, so a baked address here would repoint this deployment by accident.

Three things to take from that, because each is a deliberate design decision rather than an unfinished
edge:

- **"Nothing has been checked" is stated explicitly.** The page does not render an empty provider record
  and let you assume it looked. A surface that cannot check says so.
- **The four variables ship blank in both portals' `.env.example`, with no code fallback.** Read a blank
  fallback-free variable as *stronger* evidence of being unwired than a missing one: a missing key can be
  added and silently pick up a bundled constant, and this one cannot.
- **Filling them in is not enough to make the flows work.** See the box below.

> **THE SEND PATHS CANNOT BE WALKED TODAY, even with the four addresses set, and this is not a portal
> bug.** Three independent things are missing on chain:
> 1. **No provider is registered.** `registerProvider` is registrar-only KYC work that has not been done,
>    so there is no provider record for the page to read or write.
> 2. **The typed directory resolver is not approved.** `ProviderDirectory.resolverApproved()` is `false`
>    (verified above), and a typed resolver answers nothing until the registrar approves it **and** each
>    provider selects it. Every store on it is empty.
> 3. **Generation-1 clones cannot be attached at all.** `ProviderRegistry.attachService` reads `owner()`
>    off the service, and a generation-1 `DogTagIssuer` has no owner, so all five existing clones revert.
>
> So the buttons on this page have never been executed against a chain by anyone. What is testable today
> is the preflight logic and the refusals, which is what you are seeing.
> The contract-level journeys are covered by `cd contracts && forge test --match-path
> 'test/ProviderSelfService.t.sol'` (23 tests against the real core, router, both factory generations and
> both typed resolvers).

---

## O. The profile and logo mirror - three states, and an unverified logo renders nothing

Provider listings carry contacts, address text and a logo in a content-addressed blob, published to a
mirror that the indexer serves at `GET /v1/content/:address` and that the portals read back and
**re-verify themselves**.

**The rule to test for: an unverified logo renders NOTHING.** Not a placeholder, not a broken-image icon,
not a generic avatar, not an initials block. A logo is the strongest visual claim of legitimacy in the
product, so a stand-in shown for one that could not be verified is precisely how a forged provider comes
to look real. The three states are:

| State | What renders |
|---|---|
| **verified** | the image |
| **unverified** | no image, plus a visible reason in a warning tone |
| **not published** | no image, plus a quiet neutral line |

The last two look identical without that tone difference, which is why they are told apart: an ordinary
provider who published no logo is a **fact** and renders neutral, while a logo whose bytes did not match
their content address is a **failure** and warns.

**What you can check today:**

```
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:46001/v1/content/0x00     # 404 - route is live, nothing stored
curl -s -X PUT -H 'Authorization: Bearer x' --data-binary hi \
     -o /dev/null -w '%{http_code}\n' http://localhost:46001/v1/content/0x1234      # 503 or 401, see below
```

The write gate is a **dedicated** `MIRROR_INGEST_TOKEN`, never an oversight scope token - those carry read
authority this route must never accept, and it refuses them with a message saying so. Which code you get
depends on the stack, per "Which stack am I on?":

- **`demo-up.sh` (simulated indexer):** the demo token `dogtag-indexer-mirror-ingest-demo-token` is
  wired in the indexer and handed to both portals, so uploads work.
- **hand-booted live indexer:** `MIRROR_INGEST_TOKEN` is usually unset, and the route answers **503**
  *"this mirror accepts no content: MIRROR_INGEST_TOKEN is not configured"*. Walked: 503.

> **The verified-logo state could not be walked, and could not be walked by anyone right now.** Rendering
> a verified logo needs a published profile anchor to name it, which needs the typed directory resolver to
> be approved and a provider to be registered - the same three blockers as §N. The mirror's read and write
> routes were walked; the rendering was not.
> The store is in-memory, so a restart empties it. That is honest rather than convenient: content
> addressing makes a lost object re-publishable byte for byte, and a missing address answers 404 rather
> than a wrong answer.

---

## P. Choosing your own blockchain endpoint - every portal → Settings

**Settings** is a new nav item in the admin, vet and groomer portals.
Under **Blockchain endpoint** you can point the browser's direct ROAX reads at a JSON-RPC peer of your
choosing, which helps with availability and with an endpoint that censors requests.

Walk the guard, because it is the interesting part:

1. Open **Settings** in the vet or groomer portal.
2. Into **ROAX JSON-RPC URL**, type an endpoint on a **different chain** - `https://cloudflare-eth.com`
   (Ethereum mainnet, chainId 1) is a convenient one.
3. Click **Check and save**.

Walked result: the save is **rejected**, and it says exactly why:

> The endpoint reports chain 1; DogTag's bundled contracts are for chain 135. The custom endpoint was
> removed; blockchain reads use the bundled default.

Three properties in that one sentence, all worth confirming:

- Every address-bound read is preceded by an `eth_chainId` probe, so a peer on the wrong chain **never
  receives an address-bound request**.
- The rejection **reports itself**. A settings screen that saves and stays silent is indistinguishable
  from one that silently failed to save, so both the success and the rejection render a verdict.
- The rejection also says what it fell back to. It cleared the custom endpoint rather than leaving you on
  a peer that cannot answer.

Then click **Restore default** (or save a good URL) to get back to `https://devrpc.roax.net/`.

**Read the panel's own disclosure, because it is the honest bit:**

> Endpoint choice is not a trust upgrade. This is plain JSON-RPC, not a light client. Any endpoint can
> fabricate `isValid`, `rootIssuer`, `profileRoot`, logs, or transaction data. The chain guard only
> prevents DogTag from using its bundled contract addresses on a different chain.

And note what the setting deliberately does **not** move: the centralized app APIs and the provider
directory/indexer stay fixed, and transactions sent through an injected or WalletConnect wallet still use
that wallet's own provider.

> The same guard is what makes the catalogue's "The endpoint is a different chain" scenario in §G produce
> **no verdict** rather than a refusal. This is that scenario, driven by hand.

---

## Groomer variant (groomer portal :43617)

Same as B + F, but the groomer onboards as a **verifier** via apply→approve:
1. Groomer Setup → genesis/unlock → **Whitelist → Fill demo data** (groomer preset): this fills the
   **verify purposes** field (`grooming_intake/boarding_intake/daycare_access`) → **Submit application**.
2. Fund the relayer signer: `scripts/demo-bootstrap.sh 0x<groomerSignerAddress>` (a groomer is a
   verifier/relayer - no `ISSUER_ROLE` needed; the script funds PLASMA + re-grants the VERIFY whitelist).
3. Admin portal → step A.3 click **Groomer preset** → **Approve** → this whitelists each
   `VERIFY:<purpose>` on-chain (`key = keccak256(abi.encode("VERIFY:", keccak256(label) mod r))`,
   `whitelistFor(verifyKey, groomerRelayer)`).

The groomer is then an authorized verifier for those purposes (gated separately from issuer roles), and
the §F owner-hidden consent-proof flow works against it.

The groomer nav is **Dashboard · Calendar · Appointments · Clients · Pets · All verifications · Groomers ·
Reports · Marketing · Import from user · Ad-hoc verification · Setup · Provider self-service · Settings**.

For the shop-application demo, create a **Client** (with a pet) → book an **Appointment** → run §F
**from that appointment**, then find the result under **All verifications**, filtered by that client
or appointment. The groomer's own book is §I (pets) and §J (calendar), neither of which needs custody
unlocked. The groomer portal has no issuance surface at all: no "Issue a record" entry, no
Records page, and `BUSINESS_TYPE=groomer` does not mount the issuance routes on its backend either.

### The direct credential check, on the groomer's own Verify page

Beside the owner-consent flow, both the vet and groomer **Verification** pages carry a paste-a-record
panel. It runs the same checks §E shows you, and the same issuer pillar is mandatory there: a record
whose `issuer.documentStore` has been repointed comes back **not valid** with the issuer contract
mismatch named, distinctly from a record whose signer simply is not whitelisted - two different
accusations with two different remedies.
That pillar needs `FACTORY_ADDR` configured, which `demo-up.sh` passes to every verifier it starts.

---

## What this guide does not walk you through

Everything below needs hardware, a build, or an on-chain step that has not happened. It is listed so its
absence is not mistaken for it being missing from the product.

**Needs a phone or an arm64 simulator running a rebuilt app** (`docs/MOBILE_BUILD.md`):

- **§A0, §A1 steps 3 to 5, §D, §F step 4 and all of §H.** The apps carry a compiled Rust core and bundled
  proving artifacts and are not rebuilt by `demo-up.sh`.
- **Android on-device consent proving.** Needs an **arm64** device or emulator (the prover ships
  arm64-only), native libraries built with `cargo ndk`, and artifacts vendored with
  `make vendor-mobile-artifacts`. There is a one-command self-check: the debug build's Profile screen has
  a ZK self-test that proves and checks all seven public signals on-device.
- **Nearby provider search.** The owner app asks for coarse location only after a tap, sends the fix in a
  request body, and the **server** computes distance and ranking; the phone does not scan a downloaded
  directory. Beside the permission action the app states, word for word: *"Your location is sent to DogTag
  to find nearby vets and groomers. It is not stored."*
- **Offline Nearby.** The phone keeps a capped, name-ordered cache of provider **records** (never the
  ranking, and no distance key at all), labelled as stored and with its age shown. A remembered set may
  only stand in when the live read was unavailable, never over an empty one.
- **Directions handoff.** Available on nearby rows and on offline stored rows; it carries the provider's
  public destination and never the owner's position, and offers nothing at all for a provider with no
  published location. There is deliberately no in-app map and no place-search field.
- The rendered provider-search **result rows** cannot be verified on a dev machine at all: the directory
  host is a fixed production constant with no debug override, so a dev machine fails closed to an honest
  "could not be reached" state. Treat those rows as deployment/manual-QA coverage.

**Needs an on-chain step that has not been taken:**

- **The provider self-service send paths (§N)** - no registered provider, the typed resolver is not
  approved, and generation-1 clones cannot be attached.
- **The verified-logo state (§O)** - needs a published profile anchor, which needs the above.
- **The issuer domain binding's passing state (§L)** - needs a domain bound; `boundCloneCount()` is 0.
- **The real-chain delisting pair (§G)** - possible, but it delists a live signer. Use the scripted
  scenarios instead.

**Needs data this guide does not create:**

- **The microchip match and mismatch outcomes (§I1)** - need the shop to hold a credential for the tag,
  which arrives through the customer-app pull.
- **The "present but not addressable" explorer state (§K2)** - needs a value that failed to reach the
  chain, such as a `proposed` grant.

---

## Evidence: what was walked, and when

Revised **2026-08-02** against a live stack on ROAX chainId 135. Every result quoted above as "walked" was
observed in a browser or over `curl`/`cast` on that date, not inferred from source.

**Walked end to end:** the custody locked banner and the in-place unlock prompt; the vet Issue form
(demo fill leaves `dogTagId` blank, fills microchip `985112636323787`) through **Sign & Issue** to a real
anchored root with `isValid = true`; **Records** and its **QR** minting a fresh one-time share URL; the
government **Issue** flow for `TRAVEL_CLEARANCE` through **Issue + anchor** to **✓ anchored on-chain**,
independently confirmed with `cast` (`isValid` true, `rootIssuer` equal to the TRAVEL_CLEARANCE clone);
the bench on that genuine record (verdict **valid**, all eleven rows, block-pinned, with the **Reads
made** table); the **attack catalogue** run in full, all eleven scenarios matching their declarations with
no divergence; admin **Activity** reporting the indexer unconfigured; government **Oversight** reporting
`indexer returned 401`; the groomer **Clients → New client** form (including its Microchip field) and
**Pets** with custody locked throughout; linking a DogTag with a blank microchip and getting the neutral
**not compared** outcome; **Provider self-service** refusing with its four named variables; and
**Settings → Blockchain endpoint** rejecting a chain-1 endpoint with a rendered verdict.

**Checked directly on chain:** `CloneProvenanceRouter.generationCount() == 2`,
`ProviderDirectory.resolverApproved() == false`, `IssuerDomainRegistry.boundCloneCount() == 0`.

**Not walked, with the reason stated at each step:** everything in the list above.

> **A note on repeatability.** dogtag has **no CI for these paths** - the two mobile workflows are
> dispatch-only and no workflow runs `cargo test` - so a local run is the only evidence any of this
> works. If you change a portal, re-walk the section rather than assuming.
> And do not run the Playwright suites unfiltered: several are unmocked live-portal drivers that write
> real records and anchor on chain.
