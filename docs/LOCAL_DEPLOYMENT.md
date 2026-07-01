# DogTag — LOCAL deployment (Tier 1: the whole system on one Mac)

**Goal / you'll end with:** the entire DogTag stack running on a single Mac — **4 backends**
(admin/central + vet + groomer + a ZK **prover-service**) **+ 3 web portals** + the **browser-based
pet-owner (holder) wallet** (`stacks/owner/web`, http://localhost:45931, no backend of its own) + a
**real phone scanning a live QR** and (optionally) generating a zero-knowledge proof against the **live
ROAX testnet** (chainId **135**, addresses in [`../contracts/deployments/roax.json`](../contracts/deployments/roax.json)).

**Audience:** an AI agent runs the fenced blocks top-to-bottom; a human follows the same steps. In demo
mode every portal form is pre-filled, demo buttons are shown, and passwords/passphrases auto-fill — **you
type almost nothing** (the few values you do type are flagged with a `Replace:` note).

> **Which tier am I on?** This is **Tier 1 — LOCAL** (everything on one Mac, for a demo or dev loop). For
> a hardened, self-hosted, persistent, operators-type-everything deployment on your own server (still ROAX
> testnet) see **[REMOTE — self-host on your server](./REMOTE_DEPLOYMENT.md)**; for go-live hardening on
> top of that see **[PRODUCTION — go-live deltas](./PRODUCTION_DEPLOYMENT.md)**. The single switch between
> demo and the others is the `VITE_DEMO_MODE` flag — set here by `demo-up.sh`, **unset** elsewhere.

> **Related docs.** This is the bring-up **runbook**. For the literal, button-by-button in-portal click
> sequence see **[DEMO_CLICKS.md](./DEMO_CLICKS.md)** and (for the ZK export) **[GROOMER_ZK_DEMO.md](./GROOMER_ZK_DEMO.md)**;
> for the narrated walkthrough see **[DEMO.md](./DEMO.md)**. This page links them rather than repeating the
> clicks.

---

## 0. Goal / you'll end with

By the end of this runbook you will have, all on one Mac:

- **admin/central API** + portal — onboards businesses, broadcasts on-chain whitelists as the deployer.
- **vet API** + portal — issues dog-tag profiles and vaccination credentials, mints the SBT.
- **groomer API** + portal — the **same `vet-api` binary** run with `BUSINESS_TYPE=groomer`; verifies
  proofs of vaccination.
- **prover-service** — a `vet-api` built `--features prover` exposing `POST /prove-verification`; only a
  **32-bit-only Android** phone needs it (64-bit iOS and arm64 Android prove on-device).
- a **real phone** that scans a live QR, imports a credential, and (optionally) exports a ZK proof.

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
> - **a funded `contracts/.env`** — `DEPLOYER_PRIVATE_KEY` + `DEPLOYER_ADDRESS` for a **funded ROAX EOA**
>   (and `ROAX_RPC`). `demo-up.sh` wires this as the central stack's signer; `demo-bootstrap.sh` and
>   `demo-prepare-phone.sh` also use it to pay gas. **`contracts/.env` is LOCAL-only.**
> - **a populated `circuits/build/`** — must contain `verification_final.zkey` **and** `verification.graph`
>   so the prover-service loads the real **ArkProver**. `demo-up.sh` sets `CIRCUITS_BUILD_DIR=circuits/build`
>   on the prover, so if those files are missing the prover-service is **fail-closed** and **exits on boot**
>   (it never degrades to the chain-invalid StubProver). Build them first (§2.2 in PREREQUISITES).

Run this single block to confirm the toolchain and inputs are present.

```bash
set -e
command -v cargo cast jq python3 node pnpm >/dev/null && echo "tools: ok"   # cloudflared only if §4.2
cast chain-id --rpc-url https://devrpc.roax.net                              # expect: 135
test -f contracts/.env && grep -q DEPLOYER_PRIVATE_KEY contracts/.env && echo "contracts/.env: ok"
# the deployer EOA must be funded with PLASMA (it pays for bootstrap + central whitelists):
cast balance "$(grep -E '^DEPLOYER_ADDRESS=' contracts/.env | cut -d= -f2)" --rpc-url https://devrpc.roax.net
test -f circuits/build/verification_final.zkey && test -f circuits/build/verification.graph && echo "circuits/build: ok"
```

**Verify.** You see `tools: ok`, `135`, `contracts/.env: ok`, a **non-zero** balance, and `circuits/build: ok`.

**STOP if…**
- *a `command -v` line is empty / `cast chain-id` errors* → a tool is missing or RPC is unreachable →
  install it via **[PREREQUISITES.md](./PREREQUISITES.md)** and confirm network access, then re-run.
- *the balance is `0`* → the deployer EOA is unfunded → fund it with PLASMA before continuing (bootstrap
  and central whitelists will fail otherwise). See [PREREQUISITES.md](./PREREQUISITES.md).
- *`circuits/build` files are missing* → the `verification_final.zkey` is **gitignored** and must be
  vendored → populate `circuits/build/` before boot, or the prover will serve non-valid stub proofs.

---

## 2. Boot the stack

Boot everything with one script.

```bash
scripts/demo-up.sh        # builds + starts the 4 backends + the 3 portals (vite dev)
# stop later with: scripts/demo-down.sh
```

`demo-up.sh` **builds from source**:

- `cargo build -q --release -p admin-api -p vet-api` → the `admin-api` and `vet-api` release binaries
  (the **groomer reuses the same `vet-api` binary** with `BUSINESS_TYPE=groomer`).
- **the prover**: `cargo build -q --release -p vet-api --features prover --target-dir target/prover` →
  `target/prover/release/vet-api` (the cargo `prover` feature mounts `POST /prove-verification`; it is
  unrelated to the `FEATURES=mongo` docker build-arg used in REMOTE).

It then runs the 4 backends + 3 portals, setting their `.env` values **inline** (so there is **no `.env`
file to edit for LOCAL** — see the [Environment knobs](#environment-knobs-local) table). Key inline
settings: `VITE_DEMO_MODE=1` (autofill + demo buttons), `DNS_CHECK=skip` (no real domain to bind),
`CONFIRMATIONS=1`, custody sealed to `.demo/{vet,groomer,prover}-custody.json` via `CUSTODY_SEAL_PATH`, the
QR host set to `LAN_IP` (or the `*_PUBLIC_URL` tunnels), and `CIRCUITS_BUILD_DIR=circuits/build` on the
prover. The store is **MemStore** (records/sessions are ephemeral; only custody is sealed to disk).

LOCAL service + port map:

| Service | Portal (web) | API (host) | Binary / command | Notes |
|---|---|---|---|---|
| **admin** / central | http://localhost:39741 | http://localhost:39742 | `target/release/admin-api`, `PORT=39742` | wires the deployer key as the on-chain admin signer |
| **vet** | http://localhost:41873 | http://localhost:41874 | `target/release/vet-api`, `PORT=41874` | issues profiles + vaccination credentials |
| **groomer** | http://localhost:43617 | http://localhost:43618 | `target/release/vet-api` + `BUSINESS_TYPE=groomer`, `PORT=43618` | **same binary as vet** |
| **prover-service** | — | http://localhost:41875 | `target/prover/release/vet-api` (`--features prover`) + `CIRCUITS_BUILD_DIR=circuits/build`, `PORT=41875` | `POST /prove-verification`; 32-bit-Android ZK fallback |
| **owner-wallet** (holder) | http://localhost:45931 | — (no backend) | `pnpm --filter @dogtag/owner-web dev` (Vite) | browser-only holder wallet; state in localStorage; `VITE_OWNER_PROVER_URL`→prover :41875; verifier host comes from the scanned `/x/<token>` link |

**Verify.** Health-check each backend (admin, vet, groomer, prover):

```bash
for p in 39742 41874 43618 41875; do echo -n "$p "; curl -fsS "http://localhost:$p/health"; echo; done
# expect each line to end with: {"status":"ok"}
```

**STOP if…** *a port is silent / `curl` fails for one* → that service didn't come up → read its log under
`.demo/<svc>.log` (e.g. `.demo/vet.log`, `.demo/prover.log`) for the cause (common: a build error, the
deployer key missing, or — for the prover — `circuits/build` not populated). Fix and re-run `demo-up.sh`.

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

   This funds **0.5 PLASMA**, `whitelistFor` **VACCINATION / DOG_PROFILE / SERVICE_ATTESTATION**, and
   grants `DogTagSBT.ISSUER_ROLE` (so the vet can mint dog tags) — all paid by the deployer key with
   **legacy gas**.

   **Verify.** The output includes lines like:

   ```text
   Funding <SIGNER> with 0.5 PLASMA for gas…
   whitelistFor(VACCINATION, <SIGNER>)…
   grantRole(ISSUER, <SIGNER>) on DogTagSBT…
     hasRole(ISSUER): true
   Done. <SIGNER> is funded + whitelisted. Balance: <n> PLASMA
   isWhitelistedFor(VACCINATION): true
   ```

   (On a re-run the ISSUER line instead reads `grantRole(ISSUER, <SIGNER>) on DogTagSBT: already granted — skipping`.)

   **STOP if…** *`isWhitelistedFor(VACCINATION): false` or the tx reverts* → the deployer EOA is unfunded
   or not the whitelist admin, or RPC is flaky → re-check the §1 balance and `contracts/.env`, then re-run.

   **Overrides.** Whitelist a custom purpose set with `VERIFY_PURPOSES="a b c"`; also fund a separate owner
   wallet with `OWNER_WALLET=0x…` (see the [Environment knobs](#environment-knobs-local) table).

3. **Repeat for the groomer signer.** Genesis the groomer signer in the groomer portal (:43617), then run
   `scripts/demo-bootstrap.sh 0x<GROOMER_SIGNER>` (`Replace: <GROOMER_SIGNER>` = the address the groomer
   Setup wizard shows). A groomer is a verifier — it gets funded + the `VERIFY:<purpose>` whitelists.

> The admin portal (:39741, password `admin` prefilled) is where you **Register business → Submit issuer
> application → Approve** (Approve broadcasts the on-chain whitelists as the wired admin signer). Those
> clicks are in [DEMO_CLICKS.md](./DEMO_CLICKS.md).

---

## 6. Prepare a real phone to test EXPORT

To exercise the ZK **EXPORT** flow you need a dog-tag profile that the phone's wallet owns. This script
mints a `DOG_PROFILE` SBT to the phone's wallet and prints the handle to use.

```bash
scripts/demo-prepare-phone.sh <PHONE_WALLET> [<GROOMER_SIGNER>]
# Replace: <PHONE_WALLET> = the secp256k1 wallet the phone app shows (Profile → Create embedded wallet)
# Replace: <GROOMER_SIGNER> = optional; the groomer signer from §5 if you also want it funded here
```

> **First run compiles a helper.** On its first run this script builds the `field-hash` helper
> (`cargo build -p dogtag-standard-rs --bin field-hash`) — a **separate Rust build** beyond the §2 backends
> that needs the **LOCAL Rust toolchain** (incl. `cmake` + a C toolchain — already in §1's prereqs). Expect a
> one-time compile pause here.

**Verify.** The output prints `ownerOf(...) = <phone wallet>` and ends with a line like:

```text
DOG_TAG_ID=42
```

That `DOG_TAG_ID` is the **handle** to type into the **vet Issue form's `dogTagId` field** in §7 (the
on-chain key is `field_of_value(handle)`; the export later checks `ownerOf(field_of_value(dogTagId)) ==
phone`). Type that **exact** value.

**STOP if…**
- *`cargo` errors during the `field-hash` build* → your Rust toolchain can't build the workspace (missing
  `cmake` / C toolchain) → see **[PREREQUISITES.md](./PREREQUISITES.md)** and re-run.
- *no `DOG_TAG_ID=` line is printed / the mint reverts* → the deployer EOA is unfunded or the phone wallet
  is malformed → re-check §1 funding and the wallet address, then re-run.

**Build + install the app on the phone.** Follow **[MOBILE_BUILD.md](./MOBILE_BUILD.md)** (vendor the
`verification_final.zkey`, build, install on iOS or Android). **If it's a 32-bit-only Android** (no arm64
ABI), it cannot prove on-device — set the in-app **`prover_api`** to the prover URL (the prover tunnel from
§4.2, or the LAN-IP `:41875` from §4.1). The baked default `prover_api` is a dead tunnel, so this override
is mandatory on such devices — see **[TUNNELING.md](./TUNNELING.md)**. 64-bit devices ignore `prover_api`.

---

## 7. Run the flow

With the signer bootstrapped (§5) and — for a phone — the host reachable (§4) and the phone prepared (§6),
run the end-to-end flow:

1. **Vet issues** a vaccination credential — Issue form → set `dogTagId` = the `DOG_TAG_ID` handle from §6
   → **Sign & Issue** (anchors `issue(root)` on ROAX).
2. **Create QR** → the IMPORT QR (`/r/<token>`, one-time).
3. **Phone scans / imports** → verifies `isValid(root)` on-chain → tap to view decoded fields.
4. **(Optional) EXPORT (ZK)** → in the vet/groomer Export tab, start a ZK session → EXPORT QR
   (`/x/<token>?a=<groomerAddr>`) → the phone proves (on-device, or via the prover-service on 32-bit
   Android) and POSTs only the proof — the groomer never sees the record.

The literal, button-by-button steps are in **[DEMO_CLICKS.md](./DEMO_CLICKS.md)** (normal path) and
**[GROOMER_ZK_DEMO.md](./GROOMER_ZK_DEMO.md)** (ZK export).

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
scripts/demo-up.sh && scripts/e2e-smoke.sh     # 7-step normal path (admin :39742, vet :41874)
scripts/e2e-zk.sh                              # the ZK export path via the prover-service backend-relay
```

`e2e-smoke.sh` covers: admin login → register business → issuer-application → **approve whitelists
`keccak256(VACCINATION)` on-chain**; vet genesis + unlock (fresh signer each run); fund + whitelist the
signer; prepare → `issue(root)` anchored (`isValid=true`); share → one-time `/r/<token>` (second GET =
404); NORMAL-path EXPORT (`/x/<token>` → consent → recorded on-chain); session status `recorded` + txHash.
`e2e-zk.sh` exercises the same flow on the ZK backend-relay path.

> **Safe to run mid-demo.** `e2e-smoke.sh` stands up and funds its **own** ephemeral signer (a fresh genesis
> each run) and does **not** disturb the signer you bootstrapped in §5 — so running it during a demo is safe.

---

## 10. Troubleshooting (LOCAL subset)

| Symptom | Likely cause | Fix |
|---|---|---|
| Phone loads the QR host fine, then fails after the next boot | **Ephemeral tunnel** — the `trycloudflare` URL changed | Re-run the tunnel, re-boot `demo-up.sh` with the new URL(s), re-set the phone's `prover_api` if the prover URL changed (§4.2, [TUNNELING.md](./TUNNELING.md)) |
| Phone can't reach the Mac at all on same Wi-Fi | **Wrong LAN IP**, or **client isolation** | Re-check `ipconfig getifaddr en0` and re-boot with `LAN_IP=` (§4.1); if still unreachable the network isolates clients → use tunnels (§4.2) |
| QR resolves once but a re-scan 404s | **Stale QR / consumed one-time token** — `/r/` and `/x/` tokens are deleted after first scan (180s TTL) | Create a fresh QR in the portal and scan that |
| 32-bit Android export fails / proof never posts | **`prover_api` not set** (baked default is a dead tunnel) | Set the in-app `prover_api` to the live prover URL (§6, [TUNNELING.md](./TUNNELING.md)) |
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
| `CIRCUITS_BUILD_DIR` | dir holding `verification_final.zkey` + `verification.graph`; makes the prover load the real **ArkProver** (else StubProver, not chain-valid) | `circuits/build` (set on the prover by `demo-up.sh`) |
| `VERIFY_PURPOSES` | `demo-bootstrap.sh` — override the `VERIFY:<purpose>` set whitelisted for a groomer | the built-in `grooming_intake boarding_intake daycare_access` |
| `OWNER_WALLET` | `demo-bootstrap.sh` — also fund this owner wallet with PLASMA | unset |

---

## See also

- **[DEMO_CLICKS.md](./DEMO_CLICKS.md)** — literal, type-nothing in-portal click sequence (normal path).
- **[GROOMER_ZK_DEMO.md](./GROOMER_ZK_DEMO.md)** — the ZK export click sequence.
- **[DEMO.md](./DEMO.md)** — narrated walkthrough.
- **[TUNNELING.md](./TUNNELING.md)** — the 3-tunnel reference + phone networking + ephemerality.
- **[MOBILE_BUILD.md](./MOBILE_BUILD.md)** — build + install the iOS/Android app on a real phone.
- **[PREREQUISITES.md](./PREREQUISITES.md)** — install matrix for every tool above.
- **[REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md)** — Tier 2: self-host on your server (persistent, hardened).
- **[PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md)** — Tier 3: go-live hardening deltas.
- **[DEPLOY.md](./DEPLOY.md)** — ROAX contract-deploy runbook.
