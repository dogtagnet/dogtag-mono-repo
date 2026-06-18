# DogTag — testnet end-to-end demo (LIVE on ROAX)

Click through the whole flow against the **live ROAX deployment** (chainId 135,
`contracts/deployments/roax.json`): admin onboards a vet/groomer → the business issues a credential
(anchored on-chain) → shows a QR → the owner app scans it → imports the raw doc + **polls until it's
verified on-chain** → taps the credential to **view every decoded Merkle leaf field** → (optionally)
signs a consent that records a proof-of-verification on-chain.

This flow is **verified working end-to-end on a real Android device** and by the automated
`scripts/e2e-smoke.sh` (see [§7](#7-automated-verification-e2e-smokesh)).

> **Start here:** this page is the narrated **LOCAL/demo** walkthrough. The authoritative LOCAL runbook
> is **[LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md)**; `scripts/demo-up.sh` sets **`VITE_DEMO_MODE=1`**
> (auto-fill + demo buttons + ephemeral MemStore). For a hardened, persistent, self-hosted, **type-
> everything** deployment (flag **unset**), see **[REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md)**.

Pre-created on ROAX for the demo: `DogTagIssuer` clones — VACCINATION `0x5c703910111f942EE0f47E02214291b5274cDb53`,
DOG_PROFILE `0xdb8d39eb83DDFAaA7481C4Af4e47D0044116dB25`. ZK verifier is live (`0x138b4330…`),
VerificationRegistry `0x19C1B5f80c41EE864149500bdF998Dd18aec2a43` (ZK-wired).

## 0. Boot
```bash
scripts/demo-up.sh        # builds + starts admin/vet/groomer backends + the 3 portals (vite dev)
# portals: admin http://localhost:39741 · vet http://localhost:41873 · groomer http://localhost:43617
# backends: admin :39742 · vet :41874 · groomer :43618   (ROAX chainId 135)
# stop with: scripts/demo-down.sh
```
Backends use the in-memory store (no Mongo needed); a restart means re-genesis. `demo-up.sh` wires the
deployer/admin key (`contracts/.env` → `ADMIN_PRIVATE_KEY`) into the central stack so it can broadcast
`whitelistFor`/`mint`, sets `DNS_CHECK=skip` (bypasses DNS-TXT for the `.local` demo domains), and sets
the QR host to the Mac LAN IP (see [§6 phone networking](#6-phone-networking-real-gotchas)).

For corporate/VPN Wi-Fi, boot with a public tunnel so the phone can reach the vet from any network:
```bash
VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh   # see §6
```

> **Just want the buttons?** See **[DEMO_CLICKS.md](./DEMO_CLICKS.md)** — the exact, literal
> click-through (every form is prefilled by demo buttons; passwords are prefilled; type nothing).
> It also covers the stale-session recovery if you restart a backend mid-demo.

## 1. Stand up the vet's signer (one-time)
1. Open the **vet portal** (:41873) → **Setup wizard**: log in (operator password `operator`,
   prefilled), run **Genesis** (it shows 24 words → confirm the challenge words → set a passphrase →
   Unlock). The wizard shows the derived **signer address** (auto-carried — you never copy/paste it).
2. Fund + whitelist that signer on-chain (PLASMA gas + issuer whitelist):
   ```bash
   scripts/demo-bootstrap.sh 0x<vetSignerAddress>
   ```
   (Funds 0.5 PLASMA and `whitelistFor` VACCINATION/DOG_PROFILE/SERVICE_ATTESTATION using the
   deployer/admin key — **legacy gas**, see [§8 notes](#8-notesgotchas-from-live-bring-up). The admin
   portal **Approve** flow also whitelists — see step 2 — but only the script can fund gas.) Repeat for
   the groomer signer (:43617) to demo the groomer too.

## 2. Admin onboards the business (admin portal :39741, password `admin`, prefilled)
Follow the wizard: **Register business** (use the "Fill demo data" button → vet preset) → **Submit
issuer application** (its addresses + record types) → **Approve** → this sends `whitelistFor` txs
on-chain (central broadcasts as the wired admin signer); the **Whitelist viewer** shows the live
`isWhitelistedFor` state. (If you already ran demo-bootstrap, the signer is whitelisted; Approve is
idempotent/visual.)

## 3. Vet issues a credential → QR (vet portal :41873)
**Issue** → click **Fill demo data** (a valid rabies certificate) → **Sign & Issue**: the backend
builds the doc, anchors the Merkle root with `issue(root)` on ROAX, and re-verifies the `RootIssued`
event (waits for the receipt) before marking it issued. Then **Create QR** → renders the QR.

> The QR carries a SHORT one-time token — `http://<host>/r/<32-hex>` — instead of a long embedded
> EdDSA record-JWT. The tiny payload makes a low-density QR the phone camera can focus on and scan
> instantly. The token maps to the record server-side and is **deleted after the first scan** (one-time;
> expires after 180s), so a second `GET /r/<token>` returns 404 — the same one-time-use guarantee as the
> old JWT. (`<host>` is the vet's `DEPLOYMENT_URL` — the LAN IP or the cloudflared tunnel; see §6.)

## 4. Owner app scans → imports → polls on-chain → taps to view decoded fields
On the phone (DogTag app), open **Scan** (Home `+` or the Verify tab) and scan the vet's QR:
- It `GET`s the wrapped doc (resolving `/r/<token>` server-side), recomputes the Merkle root via the
  Rust SDK, and reads `DogTagIssuer.isValid(root)` on ROAX — showing **Anchoring… → Verified on-chain ✓**.
- The record lands under the pet, grouped by type; filter by dog on the Travel/Documents tabs.
- **Tap the imported credential** to open the detail view: it decodes **every Merkle leaf**
  (`data` `salt:tag:value` → field values) and shows the field values alongside the **on-chain
  root / issuer / verdict** (works on both Android and iOS).

See [§6](#6-phone-networking-real-gotchas) for getting the phone to actually reach the backend.

## 5. (Optional) proof-of-verification on-chain
In the vet/groomer portal **Verify** tab → pick a purpose (**Normal** or **ZK**) → **Start session** → QR.
On the phone, scan it → review → select the record to present → sign consent (EdDSA-BabyJubjub) → it's
relayed to central `/v1/verify/consent` and submitted on-chain. The portal **polls `GET /verify/session/:id`**
→ shows **Verified on-chain ✓** with the tx + a `Verified` event.
- **Normal** path commits `credentialRoot` on-chain (ECDSA consent; instant).
- **ZK** path (live, Groth16Verifier `0x138b4330…`) keeps `recordType`/`credentialRoot` **off chain**;
  proving via `dogtag-prover-rs` takes a few minutes per proof.

## 6. Phone networking (real gotchas)
The phone is **not** the Mac — `localhost` on the phone is the phone itself. Two cases:

- **Same Wi-Fi (no client isolation):** set the app's server base to this Mac's **LAN IP**
  (`ipconfig getifaddr en0`), e.g. `http://192.168.x.x:41874` for the vet (`:39742` for central).
  `demo-up.sh` already sets the vet/groomer `DEPLOYMENT_URL` to the LAN IP so the **QR host is
  reachable** from the phone (override with `LAN_IP=192.168.x.x scripts/demo-up.sh`). Android allows
  cleartext HTTP for the demo (`usesCleartextTraffic=true`).
- **Corporate / VPN Wi-Fi (client isolation — phone can't see the Mac's LAN IP):** boot with a
  **cloudflared public HTTPS tunnel** so the phone reaches the vet from any network:
  ```bash
  cloudflared tunnel --url http://localhost:41874        # prints https://<sub>.trycloudflare.com
  VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh
  ```
  `VET_PUBLIC_URL` overrides the vet's `DEPLOYMENT_URL`, so the QR host becomes the public tunnel URL.
  **The tunnel URL is ephemeral** — it changes each run; re-boot `demo-up.sh` with the new URL.

The camera scanner was upgraded for reliable scanning (1280×720 + tap-to-focus).

## 7. Automated verification (e2e-smoke.sh)
`scripts/e2e-smoke.sh` is the click-through ground truth — it drives the **live running backends**
(admin :39742, vet :41874) through the full flow and asserts every on-chain effect, in **7 steps, all
PASS on ROAX**:
1. admin login → register business → issuer-application → **approve whitelists `keccak256(VACCINATION)`
   on-chain**;
2. vet custody genesis + unlock (fresh signer each run);
3. fund + whitelist the genesis signer (`demo-bootstrap.sh`);
4. prepare VACCINATION → **`issue(root)` anchored on the clone** (`isValid(root)=true`);
5. share → **short one-time `/r/<token>`** QR → `GET` returns the doc → **second GET = 404** (one-time);
6. NORMAL-path verify-session → subject SBT mint + consent-key bind → ECDSA consent → **recorded on-chain**;
7. `GET /verify/session/{id}` shows `recorded` + the txHash.
```bash
scripts/demo-up.sh && scripts/e2e-smoke.sh
```

## 8. Notes/gotchas (from live bring-up)
Five backend issues fixed while bringing the system up live on ROAX — worth knowing:
- **ROAX needs legacy gas.** EIP-1559 txs are accepted but never mined; all broadcasts use `--legacy`
  (the backend chain client falls back to `gas_price`).
- **The central stack needs its admin signer wired** (`ADMIN_PRIVATE_KEY`/`ADMIN_ADDRESS` — set by
  `demo-up.sh` from `contracts/.env`) to broadcast `whitelistFor`/`mint`. Without it, Approve/mint no-op.
- **`sign_and_send` waits for the receipt** before reporting success (so issue/verify reflect the real
  on-chain state, not just a submitted tx hash).
- **The `VERIFY:` whitelist key** = `keccak256(abi.encode("VERIFY:", keccak256(label) mod r))` — the
  purpose is reduced mod BN254 `r` before keying (the registry stores/nullifies the same reduced value).
- **`DNS_CHECK=skip`** bypasses DNS-TXT issuer verification for the `.local` demo domains; production
  uses DNS-over-HTTPS (DoH).
- The vet **wrap now types ALL scalar leaves** (fixes "non-typed leaf at authorizedVet").

General:
- Backend (server-key) signing is the default — the clinic pays gas; wallet mode (MetaMask) is also wired.
- CORS is enabled on the backends; the groomer api is the **same `vet-api` binary** with
  `BUSINESS_TYPE=groomer` and `PORT` from env (`43618`).
- This is **testnet**; the ZK trusted setup is a documented single-operator run
  (`docs/CEREMONY_TRANSCRIPT.md`). Mainnet requires the multi-party ceremony (`docs/CEREMONY.md`).
