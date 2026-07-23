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
                                  IssuerRegistry · VerificationRegistryConsent · DogTagSBTConsent
                                  Groth16VerifierConsent · ProtocolRegistry · IssuerFactory/Impl
                                                          ▲   ▲   ▲
                  on-chain reads/writes (--legacy gas)    │   │   │   on-chain reads (verify nullifier, etc.)
        ┌─────────────────────────────────────────────────┘   │   └─────────────────────────────┐
        │                                                      │                                  │
  ┌─────┴───────┐      ┌──────────────┐      ┌────────────────┴┐                        ┌─────────┴─────────┐
  │ admin-api   │◄────►│ vet-api      │      │ groomer          │                        │  iOS / Android    │
  │ (central)   │ HMAC │ (vet stack)  │      │ (= vet-api +     │                        │  phone apps       │
  │ :39742      │      │ :41874       │      │  BUSINESS_TYPE)  │                        │  bundle roax.json,│
  │ admin signer│      │              │      │  :43618          │                        │  zkey, graph; RPC │
  └─────┬───────┘      └──────┬───────┘      └────────┬─────────┘                        │  baked            │
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
| **Apps** (holder) | iOS + Android phone apps **+ the browser holder wallet** (`stacks/owner/web`, :45931, no backend, state in localStorage) | Phone apps: bundled `roax.json` + baked RPC constant + UniFFI lib, rebuilt to change (see [MOBILE](./MOBILE_BUILD.md)). Web wallet: `VITE_OWNER_PROVER_URL` (→ prover :41875); verifier host from the scanned `/x/<token>` link |
| **Tunnels** | 3 public HTTPS tunnels for phones | `VET_PUBLIC_URL` / `GROOMER_PUBLIC_URL` / `PROVER_PUBLIC_URL` on `demo-up.sh` (see [TUNNELING](./TUNNELING.md)) |
| **Custody** | The sealed signer keystore | LOCAL: `.demo/*-custody.json` via `CUSTODY_SEAL_PATH`. REMOTE/PROD: `CustodyBlob` in Mongo |
| **Prover** | `vet-api --features prover`, `POST /prove-consent` (the owner-trusted server-prove fallback) | LOCAL: auto on :41875. REMOTE: run it yourself. PROD: owner-trusted. Needs `CIRCUITS_BUILD_DIR` |

---

## 3. Canonical references (single sources of truth)

### 3.1 Address Book

**The single machine-readable source of truth is `contracts/deployments/roax.json`.** Human-readable
snapshots of the full table are mirrored in this file, `docs/DEPLOY.md` (labelled "snapshot —
authoritative copy is `contracts/deployments/roax.json`"), and `README.md` — and demo runbooks (e.g.
`docs/DEMO.md`) may cite specific demo-clone / verifier addresses. **All of these must mirror
`roax.json`**; don't introduce addresses anywhere else — link here instead.

**The owner-revealing contract line is RETIRED.** Its four `VerificationRegistry` generations, the
`ConsentKeyRegistry`, the `DogTagSBT`, and the non-consent `Groth16Verifier` remain in the ledger for
historical reads only - none of them is a live write target, and all are pending removal at the fresh
testnet redeploy.
The live set is `DogTagSBTConsent` + `VerificationRegistryConsent` + `Groth16VerifierConsent` on top of
the shared base.

| Contract | Address | Legacy / notes |
|---|---|---|
| IssuerRegistry | `0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c` | — |
| VerificationRegistry (RETIRED owner-revealing registry) | `0x4E2f0996e1CB4E24F1053346f3da2186906835E8` | Source deleted; final instance kept for historical reads. Earlier generations: `0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1` = `_4arg_legacy`; `0x19C1B5f80c41EE864149500bdF998Dd18aec2a43` = `_preMetaTx_legacy`; `0xb4FbbDb50D86c5208D9278413ca05c5eE309b1e8` = `_zk0_legacy` |
| VerificationRegistryConsent (**the live owner-hidden registry**) | `0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87` | 4-arg `recordVerificationZK(a,b,c,pub[7])`, `pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`; requires `msg.sender == relayer` and `R == profileRoot(dogTagId)`; owner-blind `Verified` (no `subject`). Wiring verified against `DogTagSBTConsent` `0x96Cba458…`. `0x53F988Ae…` = `_M4_mutableRoot_legacy` (**DEPRECATED / DO NOT USE**; its immutable `sbt` points at the retired mutable-root SBT; never live; zero `Verified`); `0x57A2998…` = `_preErasureGate_legacy` (RETIRED; lacks the erasure gate). Deployment details: `contracts/deployments/roax.json` `_m5_custodial_issuance` |
| ConsentKeyRegistry (RETIRED) | `0xA74DDe4a9b5b5b9045D9244907dE5d84C75BD671` | The consent key now lives inside the tree as the per-tag `owner.consentKey` leaf - there is no on-chain key registry. `0xFD277b9B33a4b299fe0b08dfA19eA0372b70745b` = `_preMetaTx_legacy` |
| DogTagSBT (RETIRED owner-revealing SBT) | `0x1FB8986573Ac36d532cF7d5a5352202B094D4233` | Source deleted; mutable-root instance kept for historical reads only |
| DogTagSBTConsent (**the live owner-hidden SBT**) | `0x96Cba4580D79bc9b8e51Fc1B3a044A29592AfFFc` | Deployed block 202601; write-once `profileRoot`; neutral custodian `0x637A5146…`; verified. Minted via `POST /profiles/issue/custodial-bind` → `mintCustodial(dogTagId, R)` - no owner address in the calldata |
| Groth16Verifier (RETIRED) | `0xEEFCfAF026931b7325472A88fd14Ee780Da13559` | Paired with the retired verification circuit; `0x138b433071Ad806E841B5AD53623290a9bf21761` = `_v1_legacy` |
| Groth16VerifierConsent (**the live consent verifier**) | `0x272be146C0aEd6401000E9Aa8241201F6f0fdF1a` | Wired into `VerificationRegistryConsent` at construction |
| Poseidon6 | `0x58091F2320c78ed6c6D1C02CB7E5c7578f1349db` | circomlib-exact; deployed with the retired owner-revealing set |
| DogTagIssuerFactory | `0xd3179AbBfb0274D0a5F7017d76015A93C159511D` | `onlyOwner createIssuer` |
| DogTagIssuerImpl | `0x16671686a5926606aB05f5e167fC65B0f8825B85` | clone impl |
| deployer EOA (genesis) | `0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96` | Governance/admin authority removed in Phase-2, but the retired owner-revealing SBT's `ISSUER_ROLE` + record-type whitelists remain; **not a neutral custodian** |
| **governance authority / admin** | `0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2` | signer-1 - WHITELIST_ADMIN + DEFAULT_ADMIN + factory owner + SBT ISSUER + PLASMA source (live since Phase-2) |
| demo clone VACCINATION | `0x5c703910111f942EE0f47E02214291b5274cDb53` | `demo_clones` |
| demo clone DOG_PROFILE | `0xdb8d39eb83DDFAaA7481C4Af4e47D0044116dB25` | `demo_clones` |
| chainId | 135 | ROAX testnet |
| consent zkey sha256 (repo) | `f83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868` | the **live** `circuits/build/consent_final.zkey` (committed); ceremony transcript [CEREMONY_TRANSCRIPT.consent.md](./CEREMONY_TRANSCRIPT.consent.md) |
| zkey sha256 (retired circuit) | `9e3636b9c12b57b8662e34505a01e19bfc87a99189c994b0d87bc2e3dcdcd992` | the retired verification circuit's testnet self-run **v2** key (Hermez ptau + drand beacon); kept as provenance - see [CEREMONY_TRANSCRIPT.md](./CEREMONY_TRANSCRIPT.md) |

### 3.2 Service + Port tables

#### LOCAL — `scripts/demo-up.sh` (runs from source: `cargo` + `vite dev`)

| Service | Portal (web) | API (host) | Binary / command | Notes |
|---|---|---|---|---|
| admin / central | 39741 | 39742 | `target/release/admin-api`, `PORT=39742` | wires the governance signer-1 key as the on-chain admin signer |
| vet | 41873 | 41874 | `target/release/vet-api`, `PORT=41874` | |
| groomer | 43617 | 43618 | `target/release/vet-api` + `BUSINESS_TYPE=groomer`, `PORT=43618` | **same binary as vet** |
| prover-service | — | 41875 | `target/prover/release/vet-api` (`--features prover`) + `CIRCUITS_BUILD_DIR=circuits/build`, `PORT=41875` | `POST /prove-consent`; the owner-trusted server-prove fallback (e.g. 32-bit-only Android) |
| owner-wallet (holder) | 45931 | — | `pnpm --filter @dogtag/owner-web dev` (Vite; no backend) | browser holder wallet; `VITE_OWNER_PROVER_URL`→prover :41875; verifier host from scanned `/x/<token>` link; state in localStorage |

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
