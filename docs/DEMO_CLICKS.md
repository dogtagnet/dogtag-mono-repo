# DogTag - literal click-through (LIVE on ROAX)

The exact buttons to press, in order, against the **live ROAX deployment** (chainId 135; contract addresses come from `contracts/.env` / `contracts/deployments/roax.json`).
The demo buttons fill every form, and all passwords are prefilled, so the operator **types nothing** except the `dogTagId` handle in §C - just click.
(Testnet only.) For the full runbook + phone networking + gotchas see **[DEMO.md](./DEMO.md)**.

Boot first: `scripts/demo-up.sh` (or with a public tunnel on corporate Wi-Fi:
`VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh` - see DEMO.md §6).
This also boots the **prover-service** on **:41875** (`POST /prove-consent`, the trusted server-prove fallback - see §E) and the browser-based **pet-owner (holder) wallet** on **:45931** (a phone-free way to receive/hold/share records; owner-hidden proving stays on the native apps - see [`stacks/owner/web/README.md`](../stacks/owner/web/README.md)).
Automated equivalents: `scripts/e2e-smoke.sh` (credential lifecycle, 7 steps) and `scripts/e2e-zk.sh` (the owner-hidden consent proof, end to end with a real proof).

Portals: **admin** http://localhost:39741 · **vet** http://localhost:41873 · **groomer** http://localhost:43617
Demo passwords (prefilled): operator `operator`, admin `admin`. Record type everywhere: **VACCINATION**.

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

---

## D. Phone (DogTag app) - scan → import → verified on-chain → view fields

> Prereq: the device has a **wallet** (§A0) and the dog-tag was **issued against it by the vet** (§A1) -
> the record's `dogTagId` handle must match a tag the device holds.

1. Open **Scan** (the Home header **Scan** button) → scan the vet's QR.
2. Watch **Anchoring… → Verified on-chain ✓**. The record lands under the pet with the matching handle.
3. **Tap the imported credential** → the detail view decodes **every Merkle leaf** and shows the field
   values + the on-chain root/issuer/verdict (Android + iOS).

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
or appointment. The groomer portal has no issuance surface at all: no "Issue a record" entry, no
Records page, and `BUSINESS_TYPE=groomer` does not mount the issuance routes on its backend either.
