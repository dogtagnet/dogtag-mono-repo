# DogTag — LOCAL deployment (Tier 1: the whole system on one Mac)

**Goal / you'll end with:** the entire DogTag stack running on a single Mac - the backends
(admin/central + vet + groomer + the ZK **prover-service**, among others) **+ the web portals** + the
**browser-based pet-owner (holder) wallet** (`stacks/owner/web`, http://localhost:45931, no backend of
its own) + a **real phone scanning a live QR** and (optionally) generating an **owner-hidden
zero-knowledge consent proof** against the **live ROAX testnet** (chainId **135**, addresses in
[`../contracts/deployments/roax.json`](../contracts/deployments/roax.json)).

**Audience:** an AI agent runs the fenced blocks top-to-bottom; a human follows the same steps. In demo
mode every portal form is pre-filled, demo buttons are shown, and passwords/passphrases auto-fill — **you
type almost nothing** (the few values you do type are flagged with a `Replace:` note).

> **Which tier am I on?** This is **Tier 1 — LOCAL** (everything on one Mac, for a demo or dev loop). For
> a hardened, self-hosted, persistent, operators-type-everything deployment on your own server (still ROAX
> testnet) see **[REMOTE — self-host on your server](./REMOTE_DEPLOYMENT.md)**; for go-live hardening on
> top of that see **[PRODUCTION — go-live deltas](./PRODUCTION_DEPLOYMENT.md)**. The single switch between
> demo and the others is the `VITE_DEMO_MODE` flag — set here by `demo-up.sh`, **unset** elsewhere.

> **Related docs.** This is the bring-up **runbook**. For the literal, button-by-button in-portal click
> sequence see **[DEMO_CLICKS.md](./DEMO_CLICKS.md)**; for the narrated walkthrough see
> **[DEMO.md](./DEMO.md)**. This page links them rather than repeating the clicks.

---

## 0. Goal / you'll end with

By the end of this runbook you will have, all on one Mac:

- **admin/central API** + portal - onboards businesses, broadcasts on-chain whitelists as the wired
  admin signer.
- **vet API** + portal - issues dog-tag profiles and vaccination credentials; seals the
  device-computed profile root via `mintCustodial` (no owner address in the calldata).
- **groomer API** + portal - the **same `vet-api` binary** run with `BUSINESS_TYPE=groomer`; relays and
  records owner-hidden consent proofs.
- **prover-service** - a `vet-api` built `--features prover` exposing `POST /prove-consent`; the
  owner-trusted server-prove fallback for a device that cannot prove locally (64-bit iOS and arm64
  Android prove on-device).
- a **real phone** that scans a live QR, imports a credential, and (optionally) presents an
  owner-hidden consent proof.

It all runs against the **live ROAX testnet** (chainId **135**, gas token **PLASMA**, **legacy gas** — all
`cast`/`forge` use `--legacy`). No contracts are redeployed.

---

## 1. Prerequisites for LOCAL

> **You need all of these before you boot.** Install commands (macOS + Linux) live in
> **[PREREQUISITES.md](./PREREQUISITES.md)** — this section only lists what LOCAL requires and verifies it.
>
> - **Rust toolchain** — the 4 backends are built with `cargo`.
> - **Node + pnpm** — the 3 portals run as `vite dev`.
> - **foundry** (`cast`) — the bootstrap scripts read/write the chain.
> - **`jq`** and **`python3`** — used by the bootstrap + smoke scripts.
> - **`cloudflared`** — only if a **real phone on corporate/cellular/guest Wi-Fi** is involved (§4.2).
> - **git + `cmake` + a C toolchain** — needed to build the prover-service (`--features prover`).
> - **a funded `contracts/.env`** - `GOVERNANCE_PRIVATE_KEY` + `GOVERNANCE_ADDRESS` for the **funded
>   governance signer-1 EOA** (`0x8E27…F4A2`), plus `ROAX_RPC`. Since Governance Phase-2 (2026-07-05, block
>   123835) this signer-1 is the admin authority. The old deployer EOA `0x119F…` lost governance/admin
>   authority but retains legacy issuer/whitelist capabilities, so it is not a neutral key.
>   `demo-up.sh` wires it as the central stack's signer; `demo-bootstrap.sh` also uses it to
>   whitelist/grant and pay gas. (`DEPLOYER_*` stays only for `forge` deploys.)
>   **`contracts/.env` is LOCAL-only.**
> - **the owner-hidden contract addresses in env** - `demo-up.sh` (and `demo-bootstrap.sh` /
>   `e2e-zk.sh`) take `ISSUER_REGISTRY_ADDR`, `VERIFICATION_REGISTRY_CONSENT_ADDR`, `SBT_CONSENT_ADDR`,
>   `PROFILE_ISSUER_ADDR`, and `VACCINATION_ISSUER_ADDR` from the environment (`contracts/.env` is
>   sourced, so they can live there) and **fail fast with a clear `set …` message if any is unset**.
>   The addresses are deliberately env-only: a redeploy must repoint the tooling instead of silently
>   falling back to a retired deployment. Deployed instances are recorded in
>   [`../contracts/deployments/roax.json`](../contracts/deployments/roax.json); the fresh unified
>   testnet redeploy is a separate upcoming step.
> - **a populated `circuits/build/`** - the prover-service loads the committed `consent_final.zkey` +
>   `consent.r1cs` + `consent_js/consent.wasm`. `demo-up.sh` sets
>   `CIRCUITS_BUILD_DIR=circuits/build` on the prover; if those files are missing, consent proving is
>   **fail-closed** - `POST /prove-consent` returns unavailable rather than serving a non-chain-valid
>   proof. `consent.graph` is **not** read by the prover-service - it is the on-device witness
>   backend vendored into each app bundle, so the phone flows need it in `circuits/build/` too.
>   Build it first (§2.2 in PREREQUISITES).

Run this single block to confirm the toolchain and inputs are present.

```bash
set -e
command -v cargo cast jq python3 node pnpm >/dev/null && echo "tools: ok"   # cloudflared only if §4.2
cast chain-id --rpc-url https://devrpc.roax.net                              # expect: 135
test -f contracts/.env && grep -q GOVERNANCE_PRIVATE_KEY contracts/.env && echo "contracts/.env: ok"
# governance signer-1 must be funded with PLASMA (it pays for bootstrap + central whitelists/mints):
cast balance "$(grep -E '^GOVERNANCE_ADDRESS=' contracts/.env | cut -d= -f2)" --rpc-url https://devrpc.roax.net
test -f circuits/build/consent_final.zkey && test -f circuits/build/consent.graph && echo "circuits/build: ok"
```

**Verify.** You see `tools: ok`, `135`, `contracts/.env: ok`, a **non-zero** balance, and `circuits/build: ok`.

**STOP if…**
- *a `command -v` line is empty / `cast chain-id` errors* → a tool is missing or RPC is unreachable →
  install it via **[PREREQUISITES.md](./PREREQUISITES.md)** and confirm network access, then re-run.
- *the balance is `0`* → the governance signer is unfunded → fund it with PLASMA before continuing
  (bootstrap and central whitelists will fail otherwise). See [PREREQUISITES.md](./PREREQUISITES.md).
- *`circuits/build` files are missing* → the committed `consent_final.zkey` should be present after
  checkout (missing → server consent proving is unavailable, the prover fails closed), while
  `consent.graph` is **not** committed and must be built out-of-band (missing → the phone apps have
  no on-device witness backend to vendor) → populate `circuits/build/` before boot.

---

## 2. Boot the stack

Boot everything with one script.
It reads the contract addresses from the environment (§1) and **fails fast** with a clear `set …`
message if any required `*_ADDR` is missing.

```bash
scripts/demo-up.sh        # builds + starts the backends + the portals (vite dev)
# stop later with: scripts/demo-down.sh
```

`demo-up.sh` **builds from source**:

- `cargo build -q --release -p admin-api -p vet-api -p government-api -p indexer-api` → the backend
  release binaries (the **groomer reuses the same `vet-api` binary** with `BUSINESS_TYPE=groomer`).
- **the prover**: `cargo build -q --release -p vet-api --features prover --target-dir target/prover` →
  `target/prover/release/vet-api` (the cargo `prover` feature mounts `POST /prove-consent`; it is
  unrelated to the `FEATURES=mongo` docker build-arg used in REMOTE).

It then runs the backends + portals, setting their `.env` values **inline** (so there is **no `.env`
file to edit for LOCAL** — see the [Environment knobs](#environment-knobs-local) table). Key inline
settings: `VITE_DEMO_MODE=1` (autofill + demo buttons), `DNS_CHECK=skip` (no real domain to bind),
`CONFIRMATIONS=1`, custody sealed to `.demo/{vet,groomer,prover}-custody.json` via `CUSTODY_SEAL_PATH`, the
QR host set to `LAN_IP` (or the `*_PUBLIC_URL` tunnels), and `CIRCUITS_BUILD_DIR=circuits/build` on the
prover. The store is **MemStore** (records/sessions are ephemeral; only custody is sealed to disk).

LOCAL service + port map:

| Service | Portal (web) | API (host) | Binary / command | Notes |
|---|---|---|---|---|
| **admin** / central | http://localhost:39741 | http://localhost:39742 | `target/release/admin-api`, `PORT=39742` | wires the governance signer-1 key as the on-chain admin signer |
| **vet** | http://localhost:41873 | http://localhost:41874 | `target/release/vet-api`, `PORT=41874` | issues profiles + vaccination credentials |
| **groomer** | http://localhost:43617 | http://localhost:43618 | `target/release/vet-api` + `BUSINESS_TYPE=groomer`, `PORT=43618` | **same binary as vet** |
| **prover-service** | — | http://localhost:41875 | `target/prover/release/vet-api` (`--features prover`) + `CIRCUITS_BUILD_DIR=circuits/build`, `PORT=41875` | `POST /prove-consent`; the owner-trusted server-prove fallback (e.g. 32-bit-only Android) |
| **owner-wallet** (holder) | http://localhost:45931 | — (no backend) | `pnpm --filter @dogtag/owner-web dev` (Vite) | browser-only holder wallet; state in localStorage; `VITE_OWNER_PROVER_URL`→prover :41875; verifier host comes from the scanned `/x/<token>` link |

**Verify.** Health-check each backend (admin, vet, groomer, prover):

```bash
for p in 39742 41874 43618 41875; do echo -n "$p "; curl -fsS "http://localhost:$p/health"; echo; done
# expect each line to end with: {"status":"ok"}
```

**STOP if…** *a port is silent / `curl` fails for one* → that service didn't come up → read its log under
`.demo/<svc>.log` (e.g. `.demo/vet.log`, `.demo/prover.log`) for the cause (common: a build error, the
governance key or an `*_ADDR` missing, or - for the prover - `circuits/build` not populated). Fix and
re-run `demo-up.sh`.

> **Stopping.** `scripts/demo-down.sh` kills the backend/portal PIDs but **leaves the custody seal**
> (`.demo/*-custody.json`) in place — that is what makes a restart a re-unlock, not a re-genesis (§8).

---

## 3. Decision: will a real phone be involved?

- **No real phone** (you'll demo the portals and use the automated checks only) → **skip to §5**.
- **Yes, a real phone** scans the QR → you must make the QR host **reachable from the phone first**. Do
  **§4 (phone networking & tunnels) BEFORE you issue anything** in §5–§7, because the issued QR embeds the
  host the phone will call. → continue to **§4**.

---

## 4. Phone networking & tunnels

The phone is **not** the Mac — `localhost` on the phone is the phone itself. Pick the fork that matches the
network. For the full reference (the 3-tunnel map, ephemerality rules, the stale-baked-prover trap) see
**[TUNNELING.md](./TUNNELING.md)**; this section gives you just enough to boot.

### 4.1 Same Wi-Fi, no client isolation

If the phone and Mac share a Wi-Fi that does **not** isolate clients, the phone can reach the Mac's LAN IP
directly. Find it and re-boot with it set.

```bash
ipconfig getifaddr en0                                   # prints your Mac's LAN IP, e.g. 192.168.1.23
LAN_IP=<LAN_IP> scripts/demo-up.sh                       # Replace: <LAN_IP> = the IP printed above
```

`demo-up.sh` sets the vet/groomer `DEPLOYMENT_URL` to `LAN_IP`, so the **QR host is reachable** from the
phone. Android allows cleartext HTTP for the demo (`usesCleartextTraffic=true`).

**Verify.** From the phone's browser (same Wi-Fi), `http://<LAN_IP>:41874/health` returns `{"status":"ok"}`.

**STOP if…** *the phone can't load that URL* → the network isolates clients (common on corporate/guest
Wi-Fi) → use **§4.2 tunnels** instead.

### 4.2 Corporate / cellular / guest Wi-Fi (client isolation)

When the phone can't reach the Mac's LAN IP, expose public HTTPS URLs with **three** cloudflared tunnels —
one each for vet, groomer, and the prover. Each command prints `https://<sub>.trycloudflare.com`.

```bash
cloudflared tunnel --url http://localhost:41874     # VET   → prints VET_TUNNEL_URL
cloudflared tunnel --url http://localhost:43618     # GROOMER → prints GROOMER_TUNNEL_URL
cloudflared tunnel --url http://localhost:41875     # PROVER  → prints PROVER_TUNNEL_URL
```

Then re-boot the stack passing all three URLs (and the LAN IP, harmless to include) so the QR host and the
prover endpoint point at the tunnels.

```bash
# Replace each <…_TUNNEL_URL> with the https URL its cloudflared command printed; <LAN_IP> from §4.1.
LAN_IP=<LAN_IP> VET_PUBLIC_URL=<VET_TUNNEL_URL> GROOMER_PUBLIC_URL=<GROOMER_TUNNEL_URL> PROVER_PUBLIC_URL=<PROVER_TUNNEL_URL> scripts/demo-up.sh
```

The vet/groomer tunnel URLs become the **QR host** (embedded in the scanned QR). The prover tunnel is
**not** in any QR — a 32-bit Android phone reaches it via the in-app `prover_api` setting (§6).

**Verify.** Each `curl -fsS <…_TUNNEL_URL>/health` returns `{"status":"ok"}`.

**STOP if…** *a tunnel URL 404s or times out* → free `trycloudflare` URLs are **ephemeral**: they change
on every `cloudflared` run and drop overnight → re-run that tunnel, re-boot `demo-up.sh` with the **new**
URL(s), and re-set the phone's `prover_api` if the prover URL changed. Full details: [TUNNELING.md](./TUNNELING.md).

---

## 5. On-chain bootstrap

Each business signs on-chain with a custody signer it generates in its portal. You **genesis** the signer
in the portal, then **fund + whitelist** it on-chain with one script.

1. **Genesis the vet signer in the portal.** Open the vet portal (:41873) → **Setup** → run Genesis →
   Unlock. In demo mode the operator password, passphrase, and challenge words all auto-fill. The Setup
   wizard shows the derived **signer address** — copy it. (The literal clicks are in
   [DEMO_CLICKS.md](./DEMO_CLICKS.md).)

2. **Fund + whitelist that signer on-chain.**

   ```bash
   scripts/demo-bootstrap.sh 0x<SIGNER>
   # Replace: <SIGNER> = the signer address the vet Setup wizard shows (without re-typing — copy it)
   ```

   This first verifies the configured owner-hidden contract set is deployed and wired together (it
   fails before sending any tx otherwise), then funds **0.5 PLASMA**, `whitelistFor`s
   **VACCINATION / DOG_PROFILE**, grants `DogTagSBTConsent.ISSUER_ROLE` (so the vet signer can
   `mintCustodial` dog tags; the canonical grant is the admin portal's **Approve** flow - this script is
   the idempotent fallback that additionally funds gas), and whitelists the `VERIFY:<purpose>` keys -
   all paid by the governance signer with **legacy gas**.

   **Verify.** The output includes lines like:

   ```text
   Funding <SIGNER> with 0.5 PLASMA for gas…
   whitelistFor(VACCINATION, <SIGNER>)…
   whitelistFor(DOG_PROFILE, <SIGNER>)…
   grantRole(ISSUER, <SIGNER>) on DogTagSBTConsent…
     hasRole(ISSUER): true
   whitelistFor(VERIFY:grooming_intake, <SIGNER>)…
   Done. <SIGNER> is funded for owner-hidden issuance + verification. Balance: <n> PLASMA
   isWhitelistedFor(VACCINATION): true
   ```

   (On a re-run the ISSUER line instead reports the role is already granted and skips it.)

   **STOP if…** *`isWhitelistedFor(VACCINATION): false`, an "ERROR: … has no code at …" / wiring line,
   or a reverted tx* → an `*_ADDR` points at the wrong deployment, the governance signer is unfunded or
   not the whitelist admin, or RPC is flaky → re-check the §1 address env + balance and
   `contracts/.env`, then re-run.

   **Overrides.** Whitelist a custom purpose set with `VERIFY_PURPOSES="a b c"` (see the
   [Environment knobs](#environment-knobs-local) table). The phone itself needs no gas - the device
   stays gasless throughout.

3. **Repeat for the groomer signer.** Genesis the groomer signer in the groomer portal (:43617), then run
   `scripts/demo-bootstrap.sh 0x<GROOMER_SIGNER>` (`Replace: <GROOMER_SIGNER>` = the address the groomer
   Setup wizard shows). A groomer is a verifier — it gets funded + the `VERIFY:<purpose>` whitelists.

> The admin portal (:39741, password `admin` prefilled) is where you **Register business → Submit issuer
> application → Approve** (Approve broadcasts the on-chain whitelists as the wired admin signer). Those
> clicks are in [DEMO_CLICKS.md](./DEMO_CLICKS.md).

---

## 6. Prepare a real phone to test the owner-hidden flow

There is deliberately **no scripted pre-mint**: the phone derives the profile root `R` from its owner
secret, and the vet learns only `R` when the phone redeems the one-time issuance QR.
The issuance session allocates the `dogTagId`, so neither a phone wallet address nor a preselected id
belongs here - the device-owned root cannot be precomputed by a script, and that is the privacy
boundary.

```bash
scripts/demo-prepare-phone.sh                                        # prints the phone-preparation steps
scripts/demo-prepare-phone.sh --groomer-relayer 0x<GROOMER_SIGNER>   # …and also bootstraps the groomer relayer
# Replace: <GROOMER_SIGNER> = the signer address the groomer Setup wizard shows (§5)
```

**Verify.** The script prints the `PHONE PREPARATION` step list (create/restore the owner wallet on the
phone → vet **Register pet** → scan the `/p/<token>` QR → the phone submits `{token, root}` → issue the
VACCINATION record → scan the groomer's `/x/<token>` consent QR); with `--groomer-relayer` it first runs
`demo-bootstrap.sh` for that signer (§5's Verify applies).

**Build + install the app on the phone.** Follow **[MOBILE_BUILD.md](./MOBILE_BUILD.md)** (vendor the
consent proving assets, build, install on iOS or Android). A device that cannot prove on-device (e.g. a
**32-bit-only Android** with no arm64 ABI) relies on the owner-trusted `POST /prove-consent`
prover-service - expose it via the prover tunnel from §4.2 (or the LAN-IP `:41875` from §4.1) and point
the phone's **`prover_api`** setting at that URL - see **[TUNNELING.md](./TUNNELING.md)**. 64-bit
devices prove on-device and ignore `prover_api`.

---

## 7. Run the flow

With the signer bootstrapped (§5) and — for a phone — the host reachable (§4) and the phone prepared (§6),
run the end-to-end flow:

1. **Vet registers the pet** - **Register pet** → enter the owner identity + pet details → **Start**;
   the backend allocates a fresh `dogTagId` and shows a one-time `/p/<token>` QR.
2. **Phone scans the issuance QR** → folds its owner secret into the profile root `R` **on-device** and
   submits only `{token, root}` to `POST /profiles/issue/custodial-bind` - no owner address enters the
   request or the chain; the vet signer seals `R` via `mintCustodial(dogTagId, R)`. Keep the portal
   open until the session reports **bound** and shows its transaction proof.
3. **Vet issues** the pet's VACCINATION credential for that session's `dogTagId` (anchors `issue(root)`
   on ROAX) → **Create QR** → the IMPORT QR (`/r/<token>`, one-time).
4. **Phone scans / imports** → verifies the anchor on-chain → tap to view decoded fields.
5. **(Optional) owner-hidden verify** - in the groomer portal, start a consent verification → the
   `/x/<token>` QR → the phone builds the `DogTagConsent(6)` proof (on-device, or via the owner-trusted
   prover-service) and the groomer relays it to `VerificationRegistryConsent` - the on-chain `Verified`
   event carries no owner, and the groomer never sees the record.

The literal, button-by-button steps are in **[DEMO_CLICKS.md](./DEMO_CLICKS.md)**.

---

## 8. Lifecycle: stop / restart

Stop the stack with:

```bash
scripts/demo-down.sh        # kills backend/portal PIDs; LEAVES the custody seal (.demo/*-custody.json)
```

**Restart semantics (read this — the old docs were wrong).** A plain restart is a **re-UNLOCK with the
same passphrase (the same signer)**, **not** a re-genesis:

- The store is **MemStore**, so **records / sessions / op-sessions are wiped** on restart.
- But the signer is **sealed** to `.demo/{vet,groomer,prover}-custody.json` (`demo-down.sh` leaves it), so
  the **same signer comes back** — it is still funded and whitelisted on-chain.
- Therefore **do NOT re-run `demo-bootstrap.sh` after a plain restart** — the signer is unchanged. Just
  boot again and unlock (auto-filled in demo mode).
- A **full re-genesis** (new signer, so you must re-bootstrap) is required **only after you delete the
  seal**: `rm -rf .demo`.

```bash
scripts/demo-up.sh          # same signer returns; re-issue records as needed; NO re-bootstrap
# full reset (new signer — THEN re-run demo-bootstrap.sh):  rm -rf .demo && scripts/demo-up.sh
```

> The persistent (Mongo) store, volume backups, and the re-`/admin/unlock`-every-restart model are a
> **REMOTE** concern — see **[REMOTE — persistent storage](./REMOTE_DEPLOYMENT.md)**.

---

## 9. Automated verification

Two scripts drive the **live running backends** end-to-end and assert every on-chain effect — both should
print **PASS**.

```bash
scripts/demo-up.sh && scripts/e2e-smoke.sh     # generic credential lifecycle (admin :39742, vet :41874)
scripts/e2e-zk.sh                              # the owner-hidden consent path (real Groth16 proof)
```

`e2e-smoke.sh` covers: admin login → register business → issuer-application → **approve whitelists
`keccak256(VACCINATION)` on-chain**; vet genesis + unlock (fresh signer each run); fund + whitelist the
signer; issue → `issue(root)` anchored (`isValid=true`); share → one-time `/r/<token>` (second GET =
404); direct credential checks; revoke.
Owner-hidden consent verification needs a real holder witness + a `DogTagConsent` Groth16 proof, which
`e2e-smoke.sh` does not have - `e2e-zk.sh` exercises that canonical path end-to-end (it needs the
owner-hidden address env from §1; see the header of `scripts/e2e-zk.sh`).

> **Safe to run mid-demo.** `e2e-smoke.sh` stands up and funds its **own** ephemeral signer (a fresh genesis
> each run) and does **not** disturb the signer you bootstrapped in §5 — so running it during a demo is safe.

---

## 10. Troubleshooting (LOCAL subset)

| Symptom | Likely cause | Fix |
|---|---|---|
| Phone loads the QR host fine, then fails after the next boot | **Ephemeral tunnel** — the `trycloudflare` URL changed | Re-run the tunnel, re-boot `demo-up.sh` with the new URL(s), re-set the phone's `prover_api` if the prover URL changed (§4.2, [TUNNELING.md](./TUNNELING.md)) |
| Phone can't reach the Mac at all on same Wi-Fi | **Wrong LAN IP**, or **client isolation** | Re-check `ipconfig getifaddr en0` and re-boot with `LAN_IP=` (§4.1); if still unreachable the network isolates clients → use tunnels (§4.2) |
| QR resolves once but a re-scan 404s | **Stale QR / consumed one-time token** — `/r/` and `/x/` tokens are deleted after first scan (180s TTL) | Create a fresh QR in the portal and scan that |
| 32-bit Android consent proof fails / never posts | **`prover_api` not set** (baked default is a dead tunnel) | Set the in-app `prover_api` to the live prover URL (§6, [TUNNELING.md](./TUNNELING.md)) |
| A `/health` is silent on boot | **Port silent** — that service crashed during boot | Read `.demo/<svc>.log` for the error (build failure, missing key, prover missing `circuits/build`); fix and re-run `demo-up.sh` (§2) |
| After a restart, on-chain calls fail / I re-ran genesis | **Restart confusion** — restart is a re-unlock, not re-genesis | Don't re-genesis on a plain restart; the sealed signer returns and is already funded — do NOT re-run `demo-bootstrap.sh` (§8). Full reset is `rm -rf .demo` |

---

## Environment knobs (LOCAL)

`demo-up.sh` sets the backend `.env` values **inline**, so for LOCAL there is **no `.env` file to edit** —
you only ever override these via environment variables on the `demo-up.sh` / bootstrap command line.

| Key | Effect | Default |
|---|---|---|
| `LAN_IP` | Mac LAN IP used as the vet/groomer `DEPLOYMENT_URL` (the QR host) on same-Wi-Fi setups | a baked LAN IP (`172.24.230.152`) — override with `ipconfig getifaddr en0` |
| `VET_PUBLIC_URL` | overrides the **vet** `DEPLOYMENT_URL` → the QR host becomes this tunnel URL | unset (uses `LAN_IP`) |
| `GROOMER_PUBLIC_URL` | overrides the **groomer** `DEPLOYMENT_URL` → the groomer QR host becomes this tunnel URL | unset (uses `LAN_IP`) |
| `PROVER_PUBLIC_URL` | public URL for the prover-service (the phone's `prover_api` target; **not** in any QR) | unset (LAN-IP `:41875`) |
| `CUSTODY_SEAL_PATH` | where each signer's custody seal is written/read (`.demo/{vet,groomer,prover}-custody.json`) | `.demo/` (set by `demo-up.sh`) |
| `CIRCUITS_BUILD_DIR` | dir holding `consent_final.zkey` + `consent.r1cs` + `consent_js/consent.wasm`; the prover-service loads the consent prover from it (else `POST /prove-consent` is unavailable - fail-closed, never a non-chain-valid proof). `consent.graph` lives beside them but is the on-device backend the app bundles vendor, not a prover-service input | `circuits/build` (set on the prover by `demo-up.sh`) |
| `VERIFY_PURPOSES` | `demo-bootstrap.sh` - override the `VERIFY:<purpose>` set whitelisted for a verifier's relayer | the built-in `grooming_intake boarding_intake daycare_access` |
| `ISSUER_REGISTRY_ADDR` | **REQUIRED** - the shared `IssuerRegistry` of the deployment the demo runs against | none - `demo-up.sh` fails fast if unset (set it in `contracts/.env` or inline) |
| `SBT_CONSENT_ADDR` / `PROFILE_ISSUER_ADDR` | **REQUIRED** - the owner-hidden `DogTagSBTConsent` + the `DOG_PROFILE` issuer clone behind `POST /profiles/issue/custodial-bind`. `PROFILE_ISSUER_ADDR` must be a **real factory-deployed clone**, never the SBT (`issue(R)` sent to the SBT reverts) - `demo-bootstrap.sh` verifies the wiring before sending any tx | none - `demo-up.sh` fails fast if unset |
| `VERIFICATION_REGISTRY_CONSENT_ADDR` | **REQUIRED** - the owner-hidden `VerificationRegistryConsent` that consent proofs are relayed to; the relayer (custody account 0) must be whitelisted for the purpose (§5) | none - `demo-up.sh` fails fast if unset |
| `VACCINATION_ISSUER_ADDR` | **REQUIRED** - the `VACCINATION` issuer clone credentials are anchored on | none - `demo-up.sh` fails fast if unset |

---

## See also

- **[DEMO_CLICKS.md](./DEMO_CLICKS.md)** - literal, type-nothing in-portal click sequence.
- **[DEMO.md](./DEMO.md)** — narrated walkthrough.
- **[TUNNELING.md](./TUNNELING.md)** — the 3-tunnel reference + phone networking + ephemerality.
- **[MOBILE_BUILD.md](./MOBILE_BUILD.md)** — build + install the iOS/Android app on a real phone.
- **[PREREQUISITES.md](./PREREQUISITES.md)** — install matrix for every tool above.
- **[REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md)** — Tier 2: self-host on your server (persistent, hardened).
- **[PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md)** — Tier 3: go-live hardening deltas.
- **[DEPLOY.md](./DEPLOY.md)** — ROAX contract-deploy runbook.
