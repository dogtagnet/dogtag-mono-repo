# DogTag - testnet end-to-end demo (LIVE on ROAX)

> **This is the narrative overview, not the guide to follow.**
> If you want to *do* the walk - from an empty machine through every use case, with each step actually
> performed on a live stack - read **[DEMO_CLICKS.md](./DEMO_CLICKS.md)** instead. That is the single
> entry point.
>
> This file is kept because `scripts/demo-up.sh` and `scripts/e2e-roles.sh` point at it as the prose
> companion to the e2e scripts, and because it indexes what `e2e-smoke.sh` / `e2e-zk.sh` assert. It
> describes the same flow in summary form and is **not** maintained as a click-through: where the two
> disagree, DEMO_CLICKS.md is the one that was walked.

Click through the whole flow against the **live ROAX deployment** (chainId 135, addresses from `contracts/.env` / `contracts/deployments/roax.json`):
**admin onboards a vet/groomer** via **apply→approve** (issuers get issuance whitelists - and a `DOG_PROFILE` issuer also gets `DogTagSBTConsent.ISSUER_ROLE` - while **verifiers get `VERIFY:<purpose>` whitelisted** the same way; the admin portal **only approves + whitelists wallet addresses**)
→ the **phone creates a self-custodial wallet** (the seed is what the owner leaves are derived from)
→ the **vet issues the dog tag owner-hidden** via a session + QR (`/p/<token>`): the device scans, folds the owner-hidden profile tree to the root `R` **on the device**, and POSTs only `{token, root}` - **no wallet address, no signature** - and the **vet anchors `issue(R)` + seals `mintCustodial(dogTagId, R)` on-chain** (no recipient address in the calldata; the tag goes to the contract's neutral custodian) while the phone polls `profileRoot(dogTagId)` until the tag is forged
→ the vet issues a vaccination credential (anchored on-chain) → shows a QR → the owner app scans it → imports the raw doc + **polls the issuer contract until `isValid(root)` is true** → taps the credential to **view every decoded Merkle leaf field**
→ (optionally) the owner **proves consent** to a verifier: a Groth16 proof over `circuits/consent.circom`, generated on-device, records an **owner-blind** proof-of-verification on-chain (`VerificationRegistryConsent`; the `Verified` event names no owner).
There is **no central `/v1/register`**, **no admin "Registered devices" / "Mint dog-tag" page**, and **no verify-mode choice anywhere** - owner-hidden is the only flow.

The credential lifecycle half is asserted by the automated `scripts/e2e-smoke.sh` (see [§7](#7-automated-verification-e2e-smokesh)); the owner-hidden consent half is asserted by `scripts/e2e-zk.sh`, which drives a REAL `consent.circom` proof through the live registry.

> **No phone? Use the browser holder wallet.** The receive/hold/display side of the pet-owner (holder) role is also available as a backend-less web app at **http://localhost:45931** (`scripts/demo-up.sh` boots it).
> Paste an issuer's "Copy wrapped document" output to **Receive**, then inspect receipts or share a selectively-redacted copy.
> Owner-hidden consent **proving stays on the native apps** - they hold the private profile-tree witness; the browser wallet deliberately has no prover wiring.
> See [`stacks/owner/web/README.md`](../stacks/owner/web/README.md).

> **Start here:** this page is the narrated **LOCAL/demo** walkthrough. The authoritative LOCAL runbook
> is **[LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md)**; `scripts/demo-up.sh` sets **`VITE_DEMO_MODE=1`**
> (auto-fill + demo buttons + ephemeral MemStore). For a hardened, persistent, self-hosted, **type-
> everything** deployment (flag **unset**), see **[REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md)**.

The demo consumes the fresh owner-hidden deployment through env only - `demo-up.sh` refuses to boot without `ISSUER_REGISTRY_ADDR`, `VERIFICATION_REGISTRY_CONSENT_ADDR`, `SBT_CONSENT_ADDR`, `PROFILE_ISSUER_ADDR` (the `DOG_PROFILE` clone) and `VACCINATION_ISSUER_ADDR` in `contracts/.env`.
That is deliberate: the ROAX testnet is disposable and gets wiped + redeployed fresh, so a redeploy repoints the demo instead of silently hitting a stale deployment.

## 0. Boot
```bash
scripts/demo-up.sh        # builds + starts admin/vet/groomer/government backends + the role portals + owner wallet (vite dev)
# portals: admin http://localhost:39741 · vet http://localhost:41873 · groomer http://localhost:43617 · government http://localhost:44831
# owner wallet (holder, no backend): http://localhost:45931   (Receive a wrapped doc → inspect receipts / share selected fields)
# backends: admin :39742 · vet :41874 · groomer :43618 · government :44832   (ROAX chainId 135)
# also boots the prover-service :41875 (POST /prove-consent - the trusted server-prove fallback; see §5)
# also boots the oversight indexer :46001 (demo mode) - the data layer for the vet Traceability + government Oversight pages
# stop with: scripts/demo-down.sh
```
Backends keep records/sessions in an in-memory store (no Mongo needed) - those are **lost on restart** -
but **custody is sealed to `.demo/{vet,groomer,prover}-custody.json`**, so a **restart = re-UNLOCK with the
same passphrase (same signer), NOT re-genesis** (records/sessions are simply re-created, and the signer is
still funded + whitelisted on-chain, so no re-bootstrap is needed). A full re-genesis is required only after
`rm -rf .demo`.
That re-unlock is **not** buried in Setup.
The first action that needs custody raises an **unlock prompt in place**, over the page you are already on, and the refused request is **replayed** once the seal opens - nothing navigates and nothing you typed is lost.
Arriving at an already-locked backend shows a **non-blocking banner** with an Unlock button instead; read-only pages stay reachable.
The dedicated **`/unlock`** page is kept as the fallback surface and as a direct link (it restores `?next=` when it carries one).
Setup keeps genesis only. Both fields prefill in demo mode - see
[DEMO_CLICKS.md](./DEMO_CLICKS.md) for the literal clicks.
`demo-up.sh` wires the governance signer-1 admin key (`contracts/.env` → `GOVERNANCE_PRIVATE_KEY`, passed to the central stack as `ADMIN_PRIVATE_KEY`) so it can broadcast `whitelistFor` - since Governance Phase-2 (2026-07-05, block 123835) this admin authority is signer-1 `0x8E27…F4A2`, NOT the old deployer EOA `0x119F…` (now zero governance roles).
(The dog-tag `mintCustodial` is broadcast by the **vet** signer, not central.)
It sets the QR host to the Mac LAN IP (see [§6 phone networking](#6-phone-networking-real-gotchas)). There is no DNS bypass: the issuer DNS lookup is always REAL, and because a `.local` domain can never publish the TXT record, approve answers `409 dnsConfirmationRequired` and the portal offers a deliberate "Whitelist anyway" - see [ISSUER_DOMAIN_BINDING.md](./ISSUER_DOMAIN_BINDING.md).

For corporate/VPN Wi-Fi, boot with a public tunnel so the phone can reach the vet from any network:
```bash
VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh   # see §6
```

> **Just want the buttons?** See **[DEMO_CLICKS.md](./DEMO_CLICKS.md)** - the exact, literal
> click-through (every form is prefilled by demo buttons; passwords are prefilled; type nothing).
> It also covers the stale-session recovery if you restart a backend mid-demo.

## 0.5 Device creates a wallet + the vet issues the dog tag
The phone just creates a self-custodial wallet; the **vet** issues the dog tag via a session + QR (there is no central registration and no admin mint page).
The phone has **no** "Central API URL" setting - every host it talks to comes from a scanned QR:
1. **Phone**: **Profile → "Create embedded wallet"** → 24-word seed.
   The wallet seed is the root of the owner-hidden identity: the three reserved owner leaves (owner-address, consent-key, owner-secret) and their salts are all derived from it, per tag.
2. **Vet** portal (:41873) → **Register pet** → **Fill demo data**: the operator enters the
   **`ownerIdentity`** block (demo-prefilled: country `GB`, identification `P1234567`, name `Alex Doe`) plus the pet fields - and **no wallet address, ever** (nothing owner-linkable goes on-chain; the issuing vet legitimately knows who it issues to, see `docs/DPIA.md`).
   → **Start** (`POST /profiles/issue/session/start`) → allocates a **`dogTagId` handle** (skipping any id already sealed on-chain) and renders a one-time QR `<vetHost>/p/<token>` (32-hex token, 180s TTL).
   **Note this handle** - you set it on the vaccination cert in §3.
3. **Phone**: scan `/p/<token>` → the app resolves the session, derives the per-tag owner leaves from its wallet seed + the `dogTagId`, folds the **owner-hidden profile tree** to the root `R` **on the device**, and POSTs `<vetHost>/profiles/issue/custodial-bind { token, root }`.
   That is the whole payload: whoever redeems the one-time token defines ownership through the owner secret sealed inside `R`.
4. The **vet backend mints on-chain**: it anchors **`issue(R)`** on the `DOG_PROFILE` clone, then seals **`DogTagSBTConsent.mintCustodial(dogTagId, R)`** (write-once `profileRoot`; the ERC-721 owner is the contract's neutral custodian, never the pet owner) - needs the vet signer's `ISSUER_ROLE` (§1) - and responds `status:"minting"`.
   The phone polls `profileRoot(dogTagId)` on ROAX until it equals its own `R` (the "forging" animation), then stores the tag.
   **Gasless for the device**, and the vet, the chain, and every later verifier learn only `R`.

> The vet must be set up + whitelisted + hold `ISSUER_ROLE` first - do §1 + §2 before step 2 above.

## 1. Stand up the vet's signer (one-time)
1. Open the **vet portal** (:41873) → **Setup wizard**: log in (operator password `operator`,
   prefilled), run **Genesis** (it shows 24 words → confirm the challenge words → set a passphrase →
   Unlock). The wizard shows the derived **signer address** (auto-carried - you never copy/paste it).
   For a **groomer/verifier**, the Setup **Whitelist** step also fills the **verify purposes** field
   (`grooming_intake/boarding_intake/daycare_access`), carried on the application as `verifyPurposes`.
2. **Fund** that signer on-chain (the **only** thing the admin portal cannot do; whitelisting + the
   `ISSUER_ROLE` grant are the admin Approve in step 2, and this script re-grants them idempotently):
   ```bash
   scripts/demo-bootstrap.sh 0x<vetSignerAddress>
   ```
   (Funds 0.5 PLASMA, whitelists `VACCINATION` + `DOG_PROFILE` issuance, grants `DogTagSBTConsent.ISSUER_ROLE` so the vet can seal tags, and whitelists the default `VERIFY:<purpose>` keys - all using the governance admin key, **legacy gas**, see [§8 notes](#8-notesgotchas-from-live-bring-up).)
   Repeat for the groomer signer (:43617) to demo the groomer too (a groomer is a verifier/relayer - funding + `VERIFY` whitelist, no `ISSUER_ROLE`).
   > **Prod note:** `ISSUER_ROLE` is a trust escalation (a holder can seal any unminted `dogTagId` to any root) - grant
   > only to accredited vets.

## 2. Admin onboards the business (admin portal :39741, password `admin`, prefilled)
Follow the wizard: **Register business** (use the **Fill demo data (vet)** button to fill the demo data) → **Submit
issuer application** (its addresses + record types) → **Approve** → this sends `whitelistFor` txs
on-chain (central broadcasts as the wired admin signer); the **Whitelist viewer** shows the live
`isWhitelistedFor` state. Both **issuers and verifiers** onboard this way:
- an **issuer** application whitelists the issuance record-types per address - and when the record
  types include `DOG_PROFILE`, Approve **also grants `DogTagSBTConsent.ISSUER_ROLE`** to the signer
  (the mint-capability grant is part of onboarding, not a script);
- a **verifier** (groomer) application carries `verifyPurposes`, and **Approve** whitelists each
  `VERIFY:<purpose>` on-chain - `key = keccak256(abi.encode("VERIFY:", keccak256(label) mod r))`,
  `whitelistFor(verifyKey, relayer)` - gated separately from issuer roles.

## 3. Vet issues a vaccination credential → QR (vet portal :41873)
**Issue a record** → click **Fill demo data** (a valid rabies certificate; it fills the cert fields but **leaves
`dogTagId` untouched**) → **set `dogTagId` = the dog tag's handle from §0.5** (the numeric handle the
Register-pet wizard allocated). This must match: the credential's `dogTagId` leaf is what ties the
record to the sealed tag (on-chain the tag key is `field_of_value(handle)`), and the phone attaches an
imported record to a pet by that handle - a mismatch leaves the record orphaned from the tag, so it
cannot be presented for that pet later. → **Sign &
Issue**: the backend builds the doc, anchors the Merkle root with `issue(root)` on ROAX, and
re-verifies the `RootIssued` event (waits for the receipt) before marking it issued. Then **Create
QR** → renders the QR.

> **dogTagId = the credential handle.** The numeric `dogTagId` is just the operator/credential id; the
> on-chain SBT key is `field_of_value(handle)`. The operator only needs the **same handle** in §0.5
> (dog-tag issuance) and here (vaccination). The demo-fill **no longer clobbers** the `dogTagId` field (a
> fixed footgun), but you must still type the matching handle.

> This is the **IMPORT** QR (device ← vet). It carries a SHORT one-time token, `http://<host>/r/<32-hex>`,
> instead of a long embedded record payload. The tiny payload makes a low-density QR the phone camera
> can focus on and scan instantly. The token maps to the record server-side and is **deleted after the
> first scan** (one-time; expires after 180s), so a second `GET /r/<token>` returns 404. (`<host>` is
> the vet's `DEPLOYMENT_URL` - the LAN IP or the cloudflared tunnel; see §6.)
>
> The symmetric **verification** QR (device → groomer, §5) is also a one-time token, but carries the
> groomer's wallet address too: `http://<host>/x/<token>?a=<groomerAddr>`. The phone resolves it via
> `GET /x/<token>`, (prod/remote) DNS-verifies the groomer, proves **on-device**, and POSTs the proof - see §5.

## 4. Owner app scans → imports → polls on-chain → taps to view decoded fields
On the phone (DogTag app), open **Scan** (the Home header **Scan** button) and scan the vet's QR:
- It `GET`s the wrapped doc (resolving `/r/<token>` server-side), recomputes the Merkle root via the
  Rust SDK, and reads `DogTagIssuer.isValid(root)` on ROAX - showing **Anchoring… → Verified on-chain ✓**.
- The record lands under the pet whose tag carries the same `dogTagId` handle; filter by dog on the
  Travel/Documents tabs.
- **Tap the imported credential** to open the detail view: it decodes **every Merkle leaf**
  (`data` `salt:tag:value` → field values) and shows the field values alongside the **on-chain
  root / issuer / verdict** (works on both Android and iOS).

See [§6](#6-phone-networking-real-gotchas) for getting the phone to actually reach the backend.

## 5. (Optional) EXPORT - proof-of-verification on-chain
The owner proves consent to the groomer, revealing nothing about themselves (the symmetric counterpart of the §3–4 import).
The verify surface is the **Verification** tab in the vet portal; in the **groomer** portal the primary path is an appointment (**Appointments** → open the booking → **Start verification**, which files the result against that visit and its client), with **Ad-hoc verification** for a walk-in with no booking.
Either way you land on the same flow: pick a purpose → **Start export** → QR.
There is **no mode picker** - owner-hidden ZK consent is the only path.
The QR is a one-time token carrying the groomer's wallet address + host (`http://<host>/x/<token>?a=<groomerAddr>`).
On the phone, scan it → the app resolves `GET /x/<token>` (purpose, recordType, relayer), asserts the groomer is whitelisted on-chain for that `VERIFY:<purpose>` (and, on prod/remote, DNS-verifies the groomer - skipped for the `.local` demo) → review → the app signs the consent with the tag's **in-tree consent key** and **generates the `DogTagConsent` Groth16 proof ON-DEVICE** → POSTs `{ exportToken, proof: {a, b, c, pubSignals} }` to `/v1/verify/consent`.
The groomer's backend relays `recordVerificationZK(a, b, c, pub[7])` as the proof-bound relayer; `VerificationRegistryConsent` checks the proof, `R == profileRoot(dogTagId)`, the deadline, and the nullifier, then emits an **owner-blind `Verified` event** - no subject, no key hash, no owner bytes anywhere in the calldata.
The portal **polls `GET /verify/session/:id`** → shows **Verified on-chain ✓** with the tx.
The groomer never sees the record, the witness, or the owner - it learns only "the owner of tag `dogTagId` consented to `purpose` for me".

> **Who generates the proof.** Phones prove **on-device**, using the frozen consent artifacts the app
> needs (`consent_final.zkey` + the witness graph; the iOS bundle wiring for the consent pair is
> finalized in the mobile-issuance/redeploy slice). For devices that cannot run the on-device prover, the **prover-service**
> (`POST /prove-consent`, booted by `demo-up.sh` on **:41875**) is the independent **server-prove
> fallback**: it is the same `vet-api` binary built `--features prover` with `CIRCUITS_BUILD_DIR` set,
> run as a SEPARATE process so the vet/groomer instances cannot accept a proving witness. Trust model:
> the prover-service sees the witness, so in production it must be a prover the OWNER trusts - the
> verifier/chain still never learn the owner, and the device submits the returned proof to the groomer
> itself. The demo runs it as a platform service; tunnel it with
> `cloudflared tunnel --url http://localhost:41875` →
> `PROVER_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh`.
> (`scripts/e2e-zk.sh` exercises the full consent path headlessly with a real proof.)

## 6. Phone networking (real gotchas)
The phone is **not** the Mac - `localhost` on the phone is the phone itself. Two cases:

- **Same Wi-Fi (no client isolation):** set the app's server base to this Mac's **LAN IP**
  (`ipconfig getifaddr en0`), e.g. `http://192.168.x.x:41874` for the vet.
  `demo-up.sh` already sets the vet/groomer `DEPLOYMENT_URL` to the LAN IP so the **QR host is
  reachable** from the phone (override with `LAN_IP=192.168.x.x scripts/demo-up.sh`). Android allows
  cleartext HTTP for the demo (`usesCleartextTraffic=true`).
- **Corporate / VPN Wi-Fi (client isolation - phone can't see the Mac's LAN IP):** boot with a
  **cloudflared public HTTPS tunnel** so the phone reaches the vet from any network:
  ```bash
  cloudflared tunnel --url http://localhost:41874        # prints https://<sub>.trycloudflare.com
  VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh
  ```
  `VET_PUBLIC_URL` overrides the vet's `DEPLOYMENT_URL`, so the QR host becomes the public tunnel URL.
  **The tunnel URL is ephemeral** - it changes each run; re-boot `demo-up.sh` with the new URL.

The camera scanner was upgraded for reliable scanning (1280×720 + tap-to-focus).

## 7. Automated verification (e2e-smoke.sh)
`scripts/e2e-smoke.sh` is the click-through ground truth for the credential lifecycle - it drives the
**live running backends** (admin :39742, vet :41874) and asserts every on-chain effect, in **7 steps**:
1. admin login → register business → issuer-application → **approve whitelists `keccak256(VACCINATION)`
   on-chain**;
2. vet custody genesis + unlock (fresh signer each run);
3. fund + whitelist the genesis signer on-chain;
4. prepare VACCINATION → **`issue(root)` anchored on the clone** (`isValid(root)=true`, confirm
   re-verified `RootIssued`);
5. share → **short one-time `/r/<token>`** QR → `GET` returns the doc → **second GET = 404** (one-time);
6. direct credential check → integrity + anchor + issuer whitelist all **valid**;
7. revoke → `isValid(root)=false` on-chain → the direct check reports **revoked**.
```bash
scripts/demo-up.sh && scripts/e2e-smoke.sh
```
The **owner-hidden consent path** needs a real holder witness + `DogTagConsent` proof, which the smoke
harness does not fabricate - `scripts/e2e-zk.sh` is its ground truth: it generates a REAL proof from
`consent.circom`, anchors `R` + seals a custodial tag, submits through the verifier backend, and then
asserts nullifier consumption, the owner-blind `Verified` event shape, and that **no owner bytes appear
in the calldata or logs**.

## 8. Notes/gotchas (from live bring-up)
Backend issues fixed while bringing the system up live on ROAX - worth knowing:
- **ROAX needs legacy gas.** EIP-1559 txs are accepted but never mined; all broadcasts use `--legacy`
  (the backend chain client falls back to `gas_price`).
- **The central stack needs its admin signer wired** (`ADMIN_PRIVATE_KEY`/`ADMIN_ADDRESS` - set by
  `demo-up.sh` from `contracts/.env`'s `GOVERNANCE_PRIVATE_KEY`/`GOVERNANCE_ADDRESS`, i.e. governance
  signer-1 `0x8E27…F4A2` since Phase-2; the old deployer EOA `0x119F…` no longer has governance/admin
  authority, though it does still hold a residual legacy issuer capability - `ISSUER_ROLE` +
  record-type whitelists from the pre-unification era - so it is **not a neutral key**) to broadcast
  `whitelistFor`.
  Wired with a key that lacks `WHITELIST_ADMIN`, the two failure shapes differ and it is worth knowing
  which one you are looking at.
  The issuer-application **Approve** button (`POST /v1/issuer-applications/:id/approve`) calls
  `whitelistFor` **directly**, not through the `GovernanceAction` dispatcher, so it BROADCASTS a
  reverting tx; `sign_and_send` checks the receipt status, so the portal surfaces a **502**, not a
  proposal - no unsigned calldata is returned on that path.
  The tri-state `outcome`/`warning` that tells a designed proposal apart from a wrong-key one covers
  the standalone whitelist console (`POST /v1/admin/whitelist/{grant,revoke}`), which does route through
  the dispatcher and comes back `disposition:"proposed"` rather than broadcasting.
  Either way `demo-up.sh` now refuses to boot on such a key unless you declare `ADMIN_PROPOSE_ONLY=1`
  (out-of-band signing) ([LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md) §2).
  (The dog-tag `mintCustodial` is broadcast by the **vet** signer, which must hold
  `DogTagSBTConsent.ISSUER_ROLE`.)
- **`sign_and_send` waits for the receipt** before reporting success (so issue/verify reflect the real
  on-chain state, not just a submitted tx hash).
- **The `VERIFY:` whitelist key** = `keccak256(abi.encode("VERIFY:", keccak256(label) mod r))` - the
  purpose is reduced mod BN254 `r` before keying (the registry stores/nullifies the same reduced value).
- **The issuer DNS check is ADVISORY and never faked.** `DNS_CHECK` is retired - it used to install a
  checker that returned an unconditional pass, fabricating a result on the very gate that decides
  whether an organisation is legitimate enough to whitelist. Every deployment now performs a real
  DNS-over-HTTPS lookup; a non-verified observation does not block approval but requires the admin's
  explicit `proceedWithoutDns`, and both the observation and the override are persisted
  (`dnsStateAtApproval` / `dnsProceededUnverified`). `.local` demo domains always take that path.
- The vet **wrap types ALL scalar leaves** (fixes "non-typed leaf at authorizedVet").

General:
- Backend (server-key) signing is the default - the clinic pays gas; wallet mode (MetaMask) is also wired.
- CORS is enabled on the backends; the groomer api is the **same `vet-api` binary** with
  `BUSINESS_TYPE=groomer` and `PORT` from env (`43618`).
- This is **testnet**; the ZK trusted setup securing the consent circuit is a documented
  single-operator run (`docs/CEREMONY_TRANSCRIPT.consent.md`). Mainnet requires the multi-party
  ceremony (`docs/CEREMONY_RUNBOOK.md`).
