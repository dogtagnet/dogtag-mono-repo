# DogTag — testnet end-to-end demo (live on ROAX)

Click through the whole flow against the **live ROAX deployment** (chainId 135,
`contracts/deployments/roax.json`): admin onboards a vet/groomer → the business issues a credential
(anchored on-chain) → shows a QR → the owner app scans it → imports the raw doc + **polls until it's
verified on-chain** → (optionally) signs a consent that records a proof-of-verification on-chain.

Pre-created on ROAX for the demo: `DogTagIssuer` clones — VACCINATION `0x5c703910111f942EE0f47E02214291b5274cDb53`,
DOG_PROFILE `0xdb8d39eb83DDFAaA7481C4Af4e47D0044116dB25`. ZK verifier is live (`0x138b4330…`),
VerificationRegistry `0x19C1B5f80c41EE864149500bdF998Dd18aec2a43`.

## 0. Boot
```bash
scripts/demo-up.sh        # builds + starts admin/vet/groomer backends + the 3 portals
# portals: admin http://localhost:39741 · vet http://localhost:41873 · groomer http://localhost:43617
# stop with: scripts/demo-down.sh
```
Backends use the in-memory store (no Mongo needed); a restart means re-genesis.

> **Just want the buttons?** See **[DEMO_CLICKS.md](./DEMO_CLICKS.md)** — the exact, literal
> click-through (every form is prefilled by demo buttons; passwords are prefilled; type nothing).
> It also covers the stale-session recovery if you restart a backend mid-demo.

## 1. Stand up the vet's signer (one-time)
1. Open the **vet portal** (:41873) → **Setup wizard**: log in (operator password `operator`), run
   **Genesis** (it shows 24 words → confirm the challenge words → set a passphrase → Unlock). The
   wizard shows the derived **signer address**.
2. Fund + whitelist that signer on-chain (PLASMA gas + issuer whitelist):
   ```bash
   scripts/demo-bootstrap.sh 0x<vetSignerAddress>
   ```
   (This funds 0.5 PLASMA and `whitelistFor` VACCINATION/DOG_PROFILE/SERVICE_ATTESTATION using the
   deployer/admin key. The **admin portal Approve** flow also whitelists — see step 2 — but only the
   script can fund gas.) Repeat for the groomer signer (:43617) if you want to demo the groomer too.

## 2. Admin onboards the business (admin portal :39741, password `admin`)
Follow the wizard: **Register business** (use the "Fill demo data" button → vet preset) → **Submit
issuer application** (its addresses + record types) → **Approve** → this sends `whitelistFor` txs
on-chain; the **Whitelist viewer** shows the live `isWhitelistedFor` state. (If you already ran
demo-bootstrap, the signer is whitelisted; Approve is idempotent/visual.)

## 3. Vet issues a credential → QR (vet portal :41873)
**Issue** → click **Fill demo data** (a valid rabies certificate) → **Sign & Issue**: the backend
builds the doc, anchors the Merkle root with `issue(root)` on ROAX, and re-verifies the `RootIssued`
event before marking it issued. Then **Create QR** → renders the QR.

> The QR now carries a SHORT one-time token — `http://<host>/r/<32-hex>` — instead of a long embedded
> EdDSA record-JWT. The tiny payload makes a low-density QR the phone camera can focus on and scan
> instantly. The token maps to the record server-side and is **deleted after the first scan** (one-time;
> expires after 180s), so a second `GET /r/<token>` returns 404 — the same one-time-use guarantee as the
> old JWT.

## 4. Owner app scans → imports → polls on-chain
On the phone (DogTag app), open **Scan** (Home `+` or the Verify tab) and scan the vet's QR:
- It `GET`s the wrapped doc (resolving `/r/<token>` server-side), recomputes the Merkle root via the
  Rust SDK, and reads
  `DogTagIssuer.isValid(root)` on ROAX — showing **Anchoring… → Verified on-chain ✓**.
- The record lands under the pet, grouped by type; filter by dog on the Travel/Documents tabs.

> PHONE NETWORKING: the phone can't reach `localhost`. Set the app's server base to this Mac's LAN IP
> (e.g. `http://192.168.x.x:41874` for the vet, `:39742` for central). Find it: `ipconfig getifaddr en0`.
> The vet/central must be reachable from the phone's Wi-Fi. (The QR's host is the vet `DEPLOYMENT_URL`;
> set `DEPLOYMENT_URL=http://<LAN-IP>:41874` in demo-up.sh's vet env so the QR points at a reachable host.)

## 5. (Optional) proof-of-verification on-chain
In the vet/groomer portal **Verify** tab → pick a purpose (Normal or ZK) → **Start session** → QR.
On the phone, scan it → review → select the record to present → sign consent → it's relayed to central
`/v1/verify/consent` and submitted on-chain. The portal **polls `GET /verify/session/:id`** → shows
**Verified on-chain ✓** with the tx + a `Verified` event (ZK = no credential data on chain).

## Notes
- Backend (server-key) signing is the default — the clinic pays gas; wallet mode (MetaMask) is also wired.
- ZK proving (`dogtag-prover-rs`) takes a few minutes per proof; the normal/ECDSA path is instant.
- This is **testnet**; the ZK trusted setup is a documented single-operator run (docs/CEREMONY_TRANSCRIPT.md).
