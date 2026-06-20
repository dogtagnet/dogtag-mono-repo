# DogTag — literal click-through (LIVE on ROAX)

The exact buttons to press, in order, against the **live ROAX deployment** (chainId 135). The demo
buttons fill every form, and all passwords are prefilled, so the operator **types nothing** — just
click. (Testnet only.) For the full runbook + phone networking + gotchas see **[DEMO.md](./DEMO.md)**.

Boot first: `scripts/demo-up.sh` (or with a public tunnel on corporate Wi-Fi:
`VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh` — see DEMO.md §6). This also boots the
**prover-service** on **:41875** (the 32-bit-Android ZK fallback — see §E).
Automated equivalent of the whole flow: `scripts/e2e-smoke.sh` (7 steps, all PASS on ROAX).

Portals: **admin** http://localhost:39741 · **vet** http://localhost:41873 · **groomer** http://localhost:43617
Demo passwords (prefilled): operator `operator`, admin `admin`. Record type everywhere: **VACCINATION**.

> Stale session? Backends keep sessions in memory, so a backend restart invalidates the saved token.
> The portal now detects the 401, shows **"Session expired — please log in again"**, clears the token,
> and routes you back to login (vet/groomer Setup re-shows the Custody admin login). Just click Sign in
> again (password stays prefilled).

---

## A0. Device creates a self-custodial wallet — DogTag app

The phone just needs a wallet — the **vet** issues the dog tag into it (§A1). There is **no** central
registration and **no** "Central API URL" setting; every host the device talks to comes from a scanned QR.

1. **Profile → "Create embedded wallet"** → the app shows a **24-word seed** → confirm → it derives the
   **secp256k1 walletAddress**.
2. Profile shows the address. That's it — the device is ready to scan the vet's dog-tag QR (§A1).

---

## A. Admin onboards the business — admin portal (:39741)

1. **Sign in** (admin password prefilled).
2. Go to **Onboard issuer** (the wizard).
3. Click **Vet preset** (top-right) — fills all three steps with the vet demo data
   (recordType `VACCINATION`, documentStore `0x5c70…Db53`).
4. Step 1 **Register business** → **Register business**.
5. Step 2 **Submit issuer application** → **Submit application**.
6. Step 3 **Approve (whitelists on-chain)** → **Approve & whitelist** → done (tx hashes shown).
   - **Issuer** approval whitelists the issuance record-types per address.
   - For a **groomer/verifier** application (Groomer preset), Approve also whitelists each
     `VERIFY:<purpose>` on-chain (from the application's **verify purposes** field) — see §B and the
     Groomer variant.

> Funding gas + ISSUER_ROLE: the on-chain signer still needs PLASMA, and a **vet** signer additionally
> needs `DogTagSBT.ISSUER_ROLE` to mint dog tags. If not already done, run
> `scripts/demo-bootstrap.sh 0x<signerAddress>` once (the address is the genesis signer from step B) — it
> funds PLASMA, whitelists the issuance record-types, **and grants `ISSUER_ROLE`** (idempotent).
> **Whitelisting is the admin Approve above — not the script.**
> Note for prod: `ISSUER_ROLE` is a trust escalation (a holder can mint any id to any address) — grant
> only to accredited vets.

---

## A1. Vet issues the dog tag — vet portal (:41873) + DogTag app

The dog-tag is issued by the **vet**, not by an admin page (there is no admin "Registered devices" /
"Mint dog-tag" page). Prereq: the vet signer is set up (§B), funded + whitelisted, and holds
`ISSUER_ROLE` (the funding note above).

1. Vet portal → **Issue dog tag** → **Fill demo data**. The form collects the `DOG_PROFILE`
   **`ownerIdentity`** (demo-prefilled): **countryOfIdentification** `GB`, **identification** `P1234567`,
   **name** `Alex Doe`, plus the pet fields.
2. **Start** → `POST /profiles/issue/session/start` → renders a one-time QR `<vetHost>/p/<token>`
   (32-hex token, 180s TTL) and shows the allocated **`dogTagId` handle**. **Note this handle** — you
   type it into the vaccination Issue form in §C (the operator types **no** wallet address anywhere; the
   device sends its own in step 3).
3. On the phone: **Scan** the `/p/<token>` QR. The app `personal_sign`s (EIP-191)
   `DogTag wallet registration: <walletAddress lowercased>` and POSTs
   `<vetHost>/profiles/issue/bind { token, walletAddress, signature }` (proves the device owns the address).
4. The vet backend recovers the signer (`== walletAddress`), builds the `DOG_PROFILE` VC (with
   `ownerIdentity` + `ownerAddress`), and calls **`DogTagSBT.mint(walletAddress, dogTagId, root)`**
   (sets `ownerOf` + `profileRoot`). It returns `{ wrappedDoc, dogTagId, root, txHash }`; the phone
   verifies against the SBT (`profileRoot == root && ownerOf == wallet`) + offline integrity and imports
   its dog tag. **Gasless for the device.**

---

## B. Vet stands up its signer + applies — vet portal (:41873)

Setup is a linear wizard; each step auto-advances on success.

1. **Sign in** (operator password prefilled).
2. Navigate to **Setup**.
3. **Continue** (Custody admin login — admin password prefilled).
4. **Generate 24-word seed** → tick **"I have written down all 24 words."** → **Continue to confirmation**.
5. Confirm screen: (demo) re-type the challenge words shown on the seed screen + any passphrase →
   **Confirm & encrypt**. (The derived signer address is now auto-saved.)
6. **Unlock** (enter the same passphrase) → **Unlock** → **Continue**.
7. **Accounts** → **Continue to whitelist application** (no extra accounts needed).
8. **Whitelist** → **Fill demo data** (signer address auto-filled from genesis). For a **groomer/verifier**
   this also fills the **verify purposes** field (`grooming_intake/boarding_intake/daycare_access`), carried
   on the application as `verifyPurposes`. → **Submit application** → **Continue**.
9. **DNS** → **Done**.

> The signer address is auto-carried — you never copy/paste it. After this, **approve in the admin portal
> (section A)** — that is what whitelists the address on-chain (issuance record-types, and `VERIFY:<purpose>`
> for a verifier). The only script step left is **funding** PLASMA + (for the vet) the **ISSUER_ROLE
> grant**: `scripts/demo-bootstrap.sh 0x<signerAddress>`.

---

## C. Vet issues a vaccination credential → IMPORT QR — vet portal (:41873)

1. Go to **Issue credential**.
2. Click **Fill demo data** (valid rabies cert; recordType `VACCINATION`). This fills the cert fields
   but **leaves `dogTagId` blank** — the demo-fill no longer clobbers it (a fixed footgun).
3. **Set the `dogTagId` field = the dog tag's handle from §A1** (the numeric `dogTagId` the Issue-dog-tag
   wizard allocated). It **must match** — on-chain the SBT key is `field_of_value(handle)`, and the
   owner's §E ZK export checks `ownerOf(field_of_value(dogTagId)) == subject`; a mismatch reverts the
   export with `ERC721NonexistentToken`.
4. **Sign & Issue**.
5. **Create QR** → the **IMPORT** QR (device ← vet) renders. It carries a SHORT one-time token
   (`http://<host>/r/<32-hex>`), NOT a long embedded JWT — a low-density QR the camera focuses on
   instantly. The token is **deleted after the first scan** (one-time; 180s expiry), so re-scanning the
   same QR yields a 404.

---

## D. Phone (DogTag app) — scan → import → verified on-chain → view fields

> Prereq: the device has a **wallet** (§A0) and the dog-tag was **issued to it by the vet** (§A1) —
> import-as-mine checks `ownerOf(dogTagId) == walletAddress`.

1. Open **Scan** (Home `+` or Export tab) → scan the vet's QR.
2. Watch **Anchoring… → Verified on-chain ✓**. The record lands under the pet.
3. **Tap the imported credential** → the detail view decodes **every Merkle leaf** and shows the field
   values + the on-chain root/issuer/verdict (Android + iOS).

> Phone can't reach `localhost`. Same Wi-Fi: set the app server base to this Mac's LAN IP (demo-up.sh
> already sets the QR host to it via `LAN_IP`/`DEPLOYMENT_URL`). Corporate/VPN Wi-Fi: boot with
> `VET_PUBLIC_URL=https://<sub>.trycloudflare.com`. Full details: docs/DEMO.md §6.

---

## E. (Optional) EXPORT — proof-of-verification on-chain — vet or groomer portal Export tab

The owner **exports** an on-device proof to the groomer (symmetric counterpart of the §C–D import).

1. Go to **Export**.
2. Click **Fill sample** (selects a non-sensitive purpose + Normal mode).
3. **Start export** → the **EXPORT** QR renders. It carries the groomer's wallet address + a one-time
   token + host: `http://<host>/x/<token>?a=<groomerAddr>` (a token, NOT a JWT).
4. On the phone: scan → the app resolves `GET /x/<token>`, confirms the groomer is whitelisted on-chain
   (prod/remote also DNS-verifies the groomer; skipped for the `.local` demo) → review → select record →
   sign consent → **generate the proof** (~1–2 s) → POST `{proof, pubSignals, consent, bind}`.
   - **64-bit phones** (iPhone, modern arm64 Android) prove **on-device**.
   - A **32-bit-only Android** (no arm64 ABI) can't run the on-device prover, so it POSTs
     `{wrappedDoc, consent, eddsaSig}` to the **prover-service** (`/prove-verification`, booted by
     `demo-up.sh` on **:41875**) and submits the returned proof to the groomer itself — the groomer still
     never sees the witness. The phone uses `AppConfig.DEFAULT_PROVER_API` (override via the `prover_api`
     pref); tunnel the service with `cloudflared tunnel --url http://localhost:41875` →
     `PROVER_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh`.
5. The portal polls and flips to **Verified on-chain ✓** (ZK = no credential data on chain **and** the
   groomer never sees the record).

---

## Groomer variant (groomer portal :43617)

Same as B + E, but the groomer onboards as a **verifier** via apply→approve:
1. Groomer Setup → genesis/unlock → **Whitelist → Fill demo data** (groomer preset): this fills the
   **verify purposes** field (`grooming_intake/boarding_intake/daycare_access`) → **Submit application**.
2. Fund the relayer signer: `scripts/demo-bootstrap.sh 0x<groomerSignerAddress>` (a groomer is a
   verifier/relayer — no `ISSUER_ROLE` needed; the script funds PLASMA + VERIFY whitelist).
3. Admin portal → step A.3 click **Groomer preset** → **Approve** → this whitelists each
   `VERIFY:<purpose>` on-chain (`key = keccak256(abi.encode("VERIFY:", keccak256(label) mod r))`,
   `whitelistFor(verifyKey, groomerRelayer)`) — **no demo-bootstrap VERIFY cast**.

The groomer is then an authorized verifier for those purposes (gated separately from issuer roles), and
the §E EXPORT on-device ZK proof flow works against it.
