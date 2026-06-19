# DogTag — literal click-through (LIVE on ROAX)

The exact buttons to press, in order, against the **live ROAX deployment** (chainId 135). The demo
buttons fill every form, and all passwords are prefilled, so the operator **types nothing** — just
click. (Testnet only.) For the full runbook + phone networking + gotchas see **[DEMO.md](./DEMO.md)**.

Boot first: `scripts/demo-up.sh` (or with a public tunnel on corporate Wi-Fi:
`VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh` — see DEMO.md §6).
Automated equivalent of the whole flow: `scripts/e2e-smoke.sh` (7 steps, all PASS on ROAX).

Portals: **admin** http://localhost:39741 · **vet** http://localhost:41873 · **groomer** http://localhost:43617
Demo passwords (prefilled): operator `operator`, admin `admin`. Record type everywhere: **VACCINATION**.

> Stale session? Backends keep sessions in memory, so a backend restart invalidates the saved token.
> The portal now detects the 401, shows **"Session expired — please log in again"**, clears the token,
> and routes you back to login (vet/groomer Setup re-shows the Custody admin login). Just click Sign in
> again (password stays prefilled).

---

## A. Admin onboards the business — admin portal (:39741)

1. **Sign in** (admin password prefilled).
2. Go to **Onboard issuer** (the wizard).
3. Click **Vet preset** (top-right) — fills all three steps with the vet demo data
   (recordType `VACCINATION`, documentStore `0x5c70…Db53`).
4. Step 1 **Register business** → **Register business**.
5. Step 2 **Submit issuer application** → **Submit application**.
6. Step 3 **Approve (whitelists on-chain)** → **Approve & whitelist** → done (tx hashes shown).

> Funding gas: the on-chain signer still needs PLASMA + whitelist. If not already done, run
> `scripts/demo-bootstrap.sh 0x<vetSignerAddress>` once (the address is the genesis signer from step B).

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
8. **Whitelist** → **Fill demo data** (signer address is already auto-filled from genesis) →
   **Submit application** → **Continue**.
9. **DNS** → **Done**.

> The signer address is auto-carried — you never copy/paste it. After this, approve in the admin
> portal (section A) if you haven't, and fund/whitelist via demo-bootstrap.

---

## C. Vet issues a credential → IMPORT QR — vet portal (:41873)

1. Go to **Issue credential**.
2. Click **Fill demo data** (valid rabies cert; recordType `VACCINATION`).
3. **Sign & Issue**.
4. **Create QR** → the **IMPORT** QR (device ← vet) renders. It carries a SHORT one-time token
   (`http://<host>/r/<32-hex>`), NOT a long embedded JWT — a low-density QR the camera focuses on
   instantly. The token is **deleted after the first scan** (one-time; 180s expiry), so re-scanning the
   same QR yields a 404.

---

## D. Phone (DogTag app) — scan → import → verified on-chain → view fields

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
   sign consent → **generate the proof ON-DEVICE** (~1–2 s) → POST `{proof, pubSignals, consent, bind}`.
5. The portal polls and flips to **Verified on-chain ✓** (ZK = no credential data on chain **and** the
   groomer never sees the record).

---

## Groomer variant (groomer portal :43617)

Same as B + C + E, but in admin step A.3 click **Groomer preset**, and in the groomer Setup
Whitelist step click **Fill demo data** (groomer preset, recordType `VACCINATION`).
