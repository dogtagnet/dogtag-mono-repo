# DogTag Deployment — Start Here (Index)

**Goal / you'll end with:** the right deployment tier picked, a one-picture mental model of the whole
system, and the canonical references (addresses, ports, chain facts) every other doc links back to.

**Audience:** an AI agent runs the fenced blocks top-to-bottom; a human follows the same steps. Tone is
tight and imperative — no marketing.

This is the **router**. It owns three canonical tables (§3): the **Address Book**, the **Service + Port**
tables (LOCAL + REMOTE), and the **tier-comparison** table (§1). Other docs link here instead of copying.

---

## 1. Start here — pick your tier

There are exactly **three tiers**. The single switch between them is the portal env var **`VITE_DEMO_MODE`**:
LOCAL sets it **inline** (`=1`) on the vite dev processes (`demo-up.sh`) → autofill + demo buttons — there is
**no LOCAL `.env` to edit** to flip it. REMOTE/PROD leave it **unset**; `remote-up.sh` **aborts** if it finds
`VITE_DEMO_MODE=1` in any stack `.env`.

| Your goal | Tier | Run | Next doc |
|---|---|---|---|
| Demo or develop on my laptop, all-in-one | **LOCAL** | `scripts/demo-up.sh` | [LOCAL — one Mac](./LOCAL_DEPLOYMENT.md) |
| Host on my own server, still on the ROAX **testnet** | **REMOTE** | `scripts/remote-up.sh` | [REMOTE — self-host](./REMOTE_DEPLOYMENT.md) |
| Real launch on a **production chain** (go-live hardening) | **PRODUCTION** | REMOTE **+** go-live deltas | [PRODUCTION — go-live](./PRODUCTION_DEPLOYMENT.md) |

**What each tier gives you / does NOT give you:**

| Tier | Gives you | Does NOT give you |
|---|---|---|
| **LOCAL** | One-command bring-up of the backends + portals + prover-service on one Mac; demo autofill/buttons; on-chain on ROAX testnet | Persistence (store = MemStore; only custody is sealed to `.demo/`), TLS, multi-host, a production chain |
| **REMOTE** | Docker stacks (admin/vet/groomer) with persistent Mongo + Caddy TLS on your server; still ROAX testnet | A prover-service (you stand one up yourself), demo buttons, a production chain, the ceremony/timelock |
| **PRODUCTION** | REMOTE **plus**: chain swap, real ZK trusted-setup ceremony, 2-day verifier timelock, hardened secrets, owner-trusted prover | Anything automatic — every delta is a deliberate, documented step |

PRODUCTION is a **delta over REMOTE**, not a separate stack: do REMOTE first, then apply the go-live deltas.

---

## 2. The system in one picture

```
                                  ROAX testnet chain (chainId 135, RPC https://devrpc.roax.net)
                                  ProviderRegistry · VerificationRegistryConsent · DogTagSBTConsent
                                  Groth16VerifierConsent · ProtocolRegistry · IssuerFactory/Impl
                                  ProviderDirectory · ServiceDomainResolver
                                                          ▲   ▲   ▲
                  on-chain reads/writes (--legacy gas)    │   │   │   on-chain reads (verify nullifier, etc.)
        ┌─────────────────────────────────────────────────┘   │   └─────────────────────────────┐
        │                                                      │                                  │
  ┌─────┴───────┐      ┌──────────────┐      ┌────────────────┴┐                        ┌─────────┴─────────┐
  │ admin-api   │◄────►│ vet-api      │      │ groomer          │                        │  iOS / Android    │
  │ (central)   │ HMAC │ (vet stack)  │      │ (= vet-api +     │                        │  phone apps       │
  │ :39742      │      │ :41874       │      │  BUSINESS_TYPE)  │                        │  bundle roax.json,│
  │ admin signer│      │              │      │  :43618          │                        │  zkey, graph; RPC │
  └─────┬───────┘      └──────┬───────┘      └────────┬─────────┘                        │  guarded choice   │
        │                     │                       │                                  └───┬──────┬────────┘
  ┌─────┴───────┐      ┌──────┴───────┐      ┌────────┴─────────┐                            │      │
  │ admin portal│      │ vet portal   │      │ groomer portal   │                  scan QR    │      │ 32-bit
  │ :39741      │      │ :41873       │      │ :43617           │             /p/ /x/ /r/ ────┘      │ Android
  └─────────────┘      └──────────────┘      └──────────────────┘             (one-time token)      │ only
                                                                                                    ▼
                   ┌────────── tunnels (LOCAL/phones) ──────────┐                       ┌───────────────────┐
                   │ VET_PUBLIC_URL → vet :41874  (in QR)        │                       │ prover-service    │
                   │ GROOMER_PUBLIC_URL → groomer :43618 (in QR) │◄──────────────────────│ vet-api           │
                   │ PROVER_PUBLIC_URL → prover :41875 (NOT QR)  │   POST /prove-        │ --features prover │
                   └────────────────────────────────────────────┘   consent             │ :41875 (sees      │
                                                                                          │ witness; OWNER's  │
   Custody: LOCAL = sealed JSON in .demo/{vet,groomer,prover}-custody.json (CUSTODY_SEAL_PATH)│ trusted prover) │
            REMOTE/PROD = CustodyBlob in Mongo; re-/admin/unlock every restart              └───────────────────┘
```

**What exists and where it's configured** (the 7 moving parts):

| Thing | What it is | Configured in / set by |
|---|---|---|
| **Backends** | `admin-api` (central) + `vet-api` (vet) + `vet-api`+`BUSINESS_TYPE=groomer` (groomer) | LOCAL: inline env in `scripts/demo-up.sh`. REMOTE/PROD: `stacks/{admin,vet,groomer}/.env` (see [REMOTE](./REMOTE_DEPLOYMENT.md)) |
| **Portals** | 3 Vite web apps (admin/vet/groomer) | LOCAL: `VITE_DEMO_MODE=1` inline. REMOTE/PROD: `stacks/<x>/web/.env` (`VITE_*`) |
| **Chain** | ROAX testnet contract set | `contracts/deployments/roax.json` (source of truth); backend `*_ADDR` + portal `VITE_*_ADDR` reference it |
| **Apps** (holder) | iOS + Android phone apps **+ the browser holder wallet** (`stacks/owner/web`, :45931, no backend, state in localStorage) | Phone apps: bundled `roax.json` + guarded default RPC + UniFFI lib; users can persist a same-chain custom RPC in Profile (see [MOBILE](./MOBILE_BUILD.md)). Web wallet: Settings persists a browser-local custom RPC over its guarded default in `src/lib/config.ts`; it runs no prover |
| **Tunnels** | 3 public HTTPS tunnels for phones | `VET_PUBLIC_URL` / `GROOMER_PUBLIC_URL` / `PROVER_PUBLIC_URL` on `demo-up.sh` (see [TUNNELING](./TUNNELING.md)) |
| **Custody** | The sealed signer keystore | LOCAL: `.demo/*-custody.json` via `CUSTODY_SEAL_PATH`. REMOTE/PROD: `CustodyBlob` in Mongo |
| **Prover** | `vet-api --features prover`, `POST /prove-consent` (the owner-trusted server-prove fallback) | LOCAL: auto on :41875. REMOTE: run it yourself. PROD: owner-trusted. Needs `CIRCUITS_BUILD_DIR` |

The direct-chain admin, vet, groomer, and owner web clients expose the same browser-local RPC
choice. Each custom endpoint must report the contract bundle's chain id via `eth_chainId`; otherwise
only the independently guarded bundled default is used. This is a liveness/censorship remedy, not a
light client or trust upgrade—a JSON-RPC peer can fabricate contract reads. Central app APIs,
provider-directory/indexer endpoints, and QR-discovered service hosts are not user-configurable.
Transactions initiated through an injected or WalletConnect wallet still use that wallet's provider.

---

## 3. Canonical references (single sources of truth)

### 3.1 Address Book

**The single source of truth is `contracts/deployments/roax.json`, and it is now the ONLY place
addresses live.**
This file used to mirror the full table, and `docs/DEPLOY.md` and `README.md` mirrored it in turn.
That is exactly how all three came to state a dead address book at once: a mirror is only as good as
the discipline updating it, and a stale address reads as a checked fact.
So the mirrors are gone rather than refreshed - do not reintroduce one here or anywhere else.

There is one set and one owner-hidden model.
The ledger carries no retired generation and no superseded instance; every contract in
`contracts/src` appears in it exactly once, deployed by a single run of `contracts/script/Deploy.s.sol`:

| contract | role |
|---|---|
| `ProviderRegistry` | provider identity, standing, service attachment, and every authority predicate |
| `DogTagIssuer` | the clone implementation: `issue`/`revoke`/`isValid`, owned by its provider |
| `DogTagIssuerFactory` | self-service clone deployment AND the write-once `rootIssuer` root index |
| `DogTagSBTConsent` | the tag; write-once `profileRoot`, minted only to a neutral custodian |
| `Groth16VerifierConsent` | the frozen consent ceremony VK, on chain |
| `VerificationRegistryConsent` | owner-blind verify |
| `ProviderDirectory` / `ServiceDomainResolver` | the typed resolvers, selected per provider/service |
| `ProtocolRegistry` | the discovery trust anchor: two axes, one binding, timelocked writes |

Read these four ledger notes before pointing anything at that set:

- `_roles` - which key holds what, and which key must broadcast the deploy.
- `_root_index` - `VerificationRegistryConsent.rootIndex` IS the factory, and it is immutable, so
  replacing the factory means replacing the registry too and repointing every client.
- `_frozen_verifier` - the consent verifier was redeployed with byte-identical runtime, so the
  on-chain VK is unchanged and no zero-knowledge artifact rotated.
- `_provisioning` - no provider is onboarded (`providerCount` is 0), so no clone exists and nothing
  has been issued.

Two non-address constants that do belong in prose, because they are artifact hashes rather than
deployment state:

| constant | value |
|---|---|
| chainId | 135 (ROAX testnet) |
| consent zkey sha256 | `f83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868` - the live committed `circuits/build/consent_final.zkey`; transcript [CEREMONY_TRANSCRIPT.consent.md](./CEREMONY_TRANSCRIPT.consent.md) |

### 3.2 Service + Port tables

#### LOCAL — `scripts/demo-up.sh` (runs from source: `cargo` + `vite dev`)

| Service | Portal (web) | API (host) | Binary / command | Notes |
|---|---|---|---|---|
| admin / central | 39741 | 39742 | `target/release/admin-api`, `PORT=39742` | wires the governance signer-1 key as the on-chain admin signer |
| vet | 41873 | 41874 | `target/release/vet-api`, `PORT=41874` | |
| groomer | 43617 | 43618 | `target/release/vet-api` + `BUSINESS_TYPE=groomer`, `PORT=43618` | **same binary as vet** |
| prover-service | — | 41875 | `target/prover/release/vet-api` (`--features prover`) + `CIRCUITS_BUILD_DIR=circuits/build`, `PORT=41875` | `POST /prove-consent`; the owner-trusted server-prove fallback (e.g. 32-bit-only Android) |
| owner-wallet (holder) | 45931 | — | `pnpm --filter @dogtag/owner-web dev` (Vite; no backend) | browser holder wallet; holds/displays credentials, receipts, redacted sharing; state in localStorage; reads validity from the ROAX RPC directly (no prover) |

#### REMOTE / PROD — `scripts/remote-up.sh` (docker compose; mongo internal-only)

| Stack | Caddy (host) | api (host) | mongo | back-up volume |
|---|---|---|---|---|
| admin | 80, 443 | 39742 | 27017 internal-only | `admindata` |
| vet | 80, 443 | 41874 | 27017 internal-only | `vetdata` |
| groomer | 80, 443 | 43618 (→ container 43618) | 27017 internal-only | `groomerdata` |
| prover-service | (manual; NOT started by `remote-up.sh`) | operator-chosen | n/a | n/a |

- Each stack's `web` (nginx) is `expose: 80` internal-only; Caddy reaches it as `web:80`.
- Mongo is **27017 internal-only on every stack** — never published to the host.
- `/admin/*` binds a SEPARATE `127.0.0.1:ADMIN_PORT` listener (default = **PORT+1**) when
  `ADMIN_LOOPBACK_ONLY=1`. (So vet's admin listener = 41875, which equals the LOCAL prover port — harmless;
  they never co-run.)

### 3.3 Chain facts

- **Network:** ROAX testnet. **RPC:** `https://devrpc.roax.net`. **chainId:** `135`.
- **Gas token:** PLASMA. **Gas mode: LEGACY** — EIP-1559 txs are accepted but never mined, so **all
  `cast`/`forge` use `--legacy`**.
- **BN254 pairing precompiles** (`0x06` add, `0x07` mul, `0x08` pairing) are **required** (Gate-B precheck
  in [DEPLOY.md](./DEPLOY.md)).
- Quick verify: `cast chain-id --rpc-url https://devrpc.roax.net` → `135`.

---

## 4. Glossary

| Term | Meaning |
|---|---|
| **signer** | The EOA whose key the backend holds in custody; signs on-chain txs (`whitelistFor`, mint, gasless relays). Since Governance Phase-2 (2026-07-05, block 123835) governance actions MUST use signer-1 (`0x8E27…F4A2`); the old deployer EOA `0x119F…` lost governance/admin authority but retains legacy issuer/whitelist capabilities and must not be treated as neutral. LOCAL wires signer-1 from `contracts/.env` (`GOVERNANCE_PRIVATE_KEY`); REMOTE/PROD use the dedicated `ADMIN_PRIVATE_KEY` (also signer-1). |
| **custody** | The sealed keystore holding the signer's key. LOCAL: `.demo/*-custody.json` (`CUSTODY_SEAL_PATH`). REMOTE/PROD: a `CustodyBlob` in Mongo. Lost passphrase = unrecoverable. |
| **genesis** | First-time creation of a stack's custody/signer. Done once. LOCAL re-genesis happens **only** after `rm -rf .demo`. |
| **unlock** | Decrypting custody with the passphrase so the backend can sign. Required after **every** restart (`POST /admin/unlock`). A LOCAL restart = re-unlock with the same passphrase, **not** re-genesis. |
| **clone / documentStore** | A per-recordType issuer contract (cloned from `DogTagIssuerImpl`) that anchors documents for a record type (e.g. VACCINATION). Backend env `*_ISSUER_ADDR` points at it. **Trap:** `PROFILE_ISSUER_ADDR` (which *is* `issue(R)`'d for `DOG_PROFILE` roots) must be a **real factory-deployed clone**, never the SBT - `issue(R)` sent to the SBT reverts. `demo-bootstrap.sh` verifies the clone's `recordType()` before sending any governance tx. |
| **QR token** | A **one-time** token embedded in a deep-link QR scanned by the phone: `/r/` (register), `/x/` (export/verify, groomer), `/p/` (issue dog tag, vet). The QR carries the host the device should call. |
| **witness** | The private inputs to the ZK circuit (the secret behind a proof). Whoever computes the proof sees the witness — which is why the prover-service is the **owner's** trusted prover, never the groomer. |
| **on-device proving vs prover-service** | 64-bit iPhone + modern arm64 Android prove **on-device** (no prover URL). A device that cannot prove locally (e.g. 32-bit-only Android) offloads to the owner-trusted **prover-service** (`POST /prove-consent`). |
| **ephemeral tunnel** | A free `trycloudflare.com` URL: changes every run and drops overnight. After any change, re-boot with the new vet/groomer URLs and re-set the phone's `prover_api`. |
| **field-hashed dogTagId** | The on-chain id is `field_of_value(handle)` - the human-typed handle is hashed into the field element used as the on-chain key (e.g. `profileRoot(field_of_value(dogTagId))`; `ownerOf` on the custodial SBT always returns the neutral custodian). |
| **MemStore vs MongoStore** | MemStore = in-memory, ephemeral (records/sessions/op-sessions lost on restart) — the LOCAL default. MongoStore = persistent, **fail-closed** (api refuses to boot if `MONGO_URI` is set but unreachable) — REMOTE/PROD. |
| **fail-closed boot** | In production (neither `DEMO_MODE` nor `VITE_DEMO_MODE` set) the api binary **refuses to start** on an unset/dev-default secret (`OPERATOR_PASSWORD`/`ADMIN_PASSWORD`/`CENTRAL_HMAC_SECRET`, or `ADMIN_PASSWORD`/`ADMIN_PRIVATE_KEY` on admin) or on an unreachable `MONGO_URI` - it exits with a `FATAL:` log rather than booting degraded. The consent prover is fail-closed **per request**: with `CIRCUITS_BUILD_DIR` unset, or set but with missing/corrupt artifacts **or a zkey whose sha256 ≠ the pinned hash** (set `CONSENT_EXPECTED_ZKEY_SHA256` when shipping a non-testnet ceremony key, audit M4), `POST /prove-consent` returns unavailable - it never serves a non-chain-valid proof. |

---

## 5. Where to go next

Read these in order for your tier; each is self-contained and runnable top-to-bottom.

| Doc | Read this if… |
|---|---|
| [PREREQUISITES.md](./PREREQUISITES.md) | …you need the install/tooling matrix (macOS + Linux) before any tier — Rust, Node/pnpm, foundry, Docker, mobile SDKs, `contracts/.env`. |
| [LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md) | …you're running everything on one Mac (demo/dev) — `demo-up.sh`, bootstrap, prover, tunnels. |
| [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md) | …you're self-hosting on your server (still ROAX testnet) — docker stacks, the backend `.env` + portal `VITE_` tables, TLS. |
| [PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md) | …you're going live on a production chain — the delta over REMOTE: chain swap, ceremony, verifier timelock, hardened secrets. |
| [MOBILE_BUILD.md](./MOBILE_BUILD.md) | …you're building/installing the iOS or Android app on a real phone — endpoint model, 32/64-bit, rebuild-on-chain-swap. |
| [TUNNELING.md](./TUNNELING.md) | …a phone can't reach your Mac — the 3-tunnel reference, phone networking, ephemerality. |
| [DEPLOY.md](./DEPLOY.md) | …you're deploying the contract set itself — the contract-deploy runbook (already live on testnet). |
| [DEMO.md](./DEMO.md) · [DEMO_CLICKS.md](./DEMO_CLICKS.md) | …you're driving a live demo - narrative runbook + click-by-click script (including the owner-hidden consent verify). |
| [CEREMONY_RUNBOOK.md](./CEREMONY_RUNBOOK.md) · [CEREMONY.md](./CEREMONY.md) | …you're running the ZK trusted-setup ceremony for production (≥3 contributors + public beacon) — the expanded captain-fill-in runbook, plus the concise version. |
| [DPIA.md](./DPIA.md) | …you need the data-protection impact assessment (privacy/compliance). |
