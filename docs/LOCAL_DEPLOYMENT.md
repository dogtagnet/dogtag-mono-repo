# DogTag — LOCAL deployment (the click-through demo)

The authoritative runbook for running DogTag **locally as a demo**: every portal form is
pre-filled, demo buttons are shown, passwords/passphrases are auto-filled, and custody is ephemeral.
**You type nothing — just click.** It runs against the **live ROAX testnet** (chainId 135, addresses
in [`../contracts/deployments/roax.json`](../contracts/deployments/roax.json)).

> **LOCAL vs REMOTE.** This is the demo. For a hardened, self-hosted, persistent, operators-type-
> everything deployment, see **[REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md)**. The single switch
> between the two is the `VITE_DEMO_MODE` flag (set here, **unset** in production). The two modes are
> compared in a table in [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md#local-vs-remote-at-a-glance).

This page is the runbook. For the literal, button-by-button click-through see
**[DEMO_CLICKS.md](./DEMO_CLICKS.md)**; for the narrated walkthrough + phone-networking gotchas see
**[DEMO.md](./DEMO.md)**. This doc cross-links them rather than repeating them.

---

## What "demo mode" actually means

`scripts/demo-up.sh` launches the three Vite portals with **`VITE_DEMO_MODE=1`**. With that flag set:

- forms are **pre-filled** and **"Fill demo data" / "Demo (vet/groomer)"** buttons are shown;
- the operator password (`operator`) and admin password (`admin`) are **auto-filled**;
- the genesis **passphrase** is auto-filled, and the vet Setup wizard briefly stashes the genesis seed
  in `sessionStorage` so the **confirm challenge words auto-fill** (so genesis is zero-typing);
- backends run the **in-memory store (MemStore)** — restart wipes custody (re-genesis needed).

With `VITE_DEMO_MODE` **unset** (the default, i.e. production), every one of those affordances is off:
forms start empty, demo buttons are hidden, no password/passphrase autofill, and the seed is never
stashed. That is the REMOTE path — do not use it here.

---

## 0. Prerequisites

- **Docker / Docker Compose** is *not* required for the demo (backends run from source via the script).
- **Rust toolchain** (the backends are `cargo run`), **Node + pnpm** (the portals are `vite dev`).
- **foundry** (`cast`) + **jq** + **python3** — for `scripts/demo-bootstrap.sh` and `scripts/e2e-smoke.sh`.
- **`contracts/.env`** with `DEPLOYER_PRIVATE_KEY` / `DEPLOYER_ADDRESS` — a funded ROAX EOA. The script
  wires this as the central stack's `ADMIN_PRIVATE_KEY`/`ADMIN_ADDRESS` so it can broadcast
  `whitelistFor` / `mint`, and `demo-bootstrap.sh` uses it to fund gas (PLASMA).
- ROAX RPC reachable: `https://devrpc.roax.net` (chainId **135**, gas token **PLASMA**, **legacy gas**).

---

## 1. Boot

```bash
scripts/demo-up.sh        # builds + starts admin/vet/groomer backends + the 3 portals (vite dev)
# stop with: scripts/demo-down.sh
```

`demo-up.sh` also: sets `VITE_DEMO_MODE=1` on all three portal launches, wires the deployer/admin key,
sets `DNS_CHECK=skip` (bypasses DNS-TXT for the `.local` demo domains, since there is no real domain),
and points the QR host at the Mac LAN IP. Because the QR host is a LAN IP/`.local` (not a real domain),
the phone-side **EXPORT** groomer DNS-verify (`dogtag-verify=<groomerAddr>`, §EXPORT) is **skipped
locally** too — see [REMOTE_DEPLOYMENT.md §4](./REMOTE_DEPLOYMENT.md#4-dns-txt-issuer--groomer-legitimacy-verbatim)
for the production check.

| Stack | Portal (web) | API |
|---|---|---|
| **admin** (central) | http://localhost:39741 | http://localhost:39742 |
| **vet** | http://localhost:41873 | http://localhost:41874 |
| **groomer** | http://localhost:43617 | http://localhost:43618 |

For corporate/VPN Wi-Fi where the phone can't reach the Mac's LAN IP, boot behind a public tunnel:

```bash
cloudflared tunnel --url http://localhost:41874     # prints https://<sub>.trycloudflare.com
VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh
```

The tunnel URL is **ephemeral** (changes each run); re-boot `demo-up.sh` with the new URL. See
[DEMO.md §6](./DEMO.md#6-phone-networking-real-gotchas) for the full LAN-IP vs cloudflared discussion.

---

## 2. The flow (everything auto-filled — you type nothing)

The literal buttons are in **[DEMO_CLICKS.md](./DEMO_CLICKS.md)**. In short:

1. **Vet custody genesis (one-time).** Vet portal (:41873) → **Setup**: operator password is prefilled →
   run **Genesis** (24 words shown, challenge words auto-fill, passphrase auto-fills) → **Unlock**. The
   derived **signer address** is auto-carried — you never copy/paste it.
2. **Fund + whitelist that signer on-chain:**
   ```bash
   scripts/demo-bootstrap.sh 0x<vetSignerAddress>
   ```
   Funds 0.5 PLASMA and `whitelistFor` VACCINATION/DOG_PROFILE/SERVICE_ATTESTATION via the deployer key
   (legacy gas). Repeat with the groomer signer (:43617) to demo the groomer too.
3. **Admin onboards the business.** Admin portal (:39741, password `admin` prefilled) → **Register
   business** ("Fill demo data" → vet preset) → **Submit issuer application** → **Approve** (broadcasts
   `whitelistFor`). With `DNS_CHECK=skip` the DNS-TXT check is bypassed for `.local`.
4. **Vet issues a credential → QR.** Vet portal → **Issue** → **Fill demo data** → **Sign & Issue**
   (anchors the Merkle root with `issue(root)` on ROAX, waits for the `RootIssued` receipt) → **Create
   QR**. The QR carries a **short one-time** `http://<host>/r/<32-hex>` token (deleted after first scan).
5. **Phone scans IMPORT QR → imports → polls on-chain → taps to view decoded fields.** See
   [DEMO.md §4–5](./DEMO.md#4-owner-app-scans--imports--polls-on-chain--taps-to-view-decoded-fields).
6. **(Optional) EXPORT.** In the vet/groomer **Export** tab, start a session → the **EXPORT QR**
   (`http://<host>/x/<token>?a=<groomerAddr>`, a one-time token carrying the groomer's wallet address).
   The phone resolves `GET /x/<token>`, checks the groomer is whitelisted on-chain (the groomer
   DNS-verify is **skipped** locally), generates the ZK proof **on-device** (~1–2 s), and POSTs only the
   proof — the groomer never sees the record. See [DEMO.md §5](./DEMO.md#5-optional-export--proof-of-verification-on-chain).

---

## 3. Ephemerality — restart = re-genesis

Demo backends use **MemStore (in-memory)**, so **a backend restart wipes custody and all records**.
After any restart of `demo-up.sh` you must re-run genesis (step 1) and re-run `demo-bootstrap.sh` for
the new signer. This is expected for the demo. (`e2e-smoke.sh` genesis-es a fresh signer each run for
exactly this reason.)

The persistent (Mongo) store, volume backups, and re-unlock-after-restart are a **REMOTE** concern —
see [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md#3-persistent-storage-mongo).

If you restart a backend mid-demo and a portal holds a stale session, see the recovery note in
[DEMO_CLICKS.md](./DEMO_CLICKS.md).

---

## 4. Automated verification (`scripts/e2e-smoke.sh`)

The click-through ground truth: drives the **live running backends** (admin :39742, vet :41874) with
`curl` + `cast` through the full flow and asserts every on-chain effect — **7 steps, all PASS on ROAX**.

```bash
scripts/demo-up.sh && scripts/e2e-smoke.sh
```

It covers: admin login → register business → issuer-application → **approve whitelists
`keccak256(VACCINATION)` on-chain**; vet genesis + unlock; fund + whitelist the signer; prepare → `issue(root)`
anchored (`isValid=true`); share → one-time IMPORT `/r/<token>` (second GET = 404); NORMAL-path **EXPORT**
session (`/x/<token>` resolve → consent → recorded on-chain); session status `recorded` + txHash. See
[DEMO.md §7](./DEMO.md#7-automated-verification-e2e-smokesh).

---

## See also

- **[DEMO.md](./DEMO.md)** — narrated walkthrough + phone networking + live-bring-up gotchas.
- **[DEMO_CLICKS.md](./DEMO_CLICKS.md)** — literal, type-nothing click-through.
- **[REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md)** — the hardened, persistent, production-shaped guide.
- **[DEPLOY.md](./DEPLOY.md)** — ROAX deploy runbook (contracts, ceremony, Docker bring-up).
