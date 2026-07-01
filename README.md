# DogTag — Pet Credentialing Ecosystem (monorepo)

Verifiable, on-chain pet credentials (identity, vaccination, service, travel) anchored as
**salted-Merkle Poseidon roots** on the **ROAX** EVM chain (chainId **135**, gas token **PLASMA**),
verified three ways — cryptographic **integrity** + on-chain **status** + DNS-bound issuer
**identity** — plus a contextual **ownership** fragment for the owner's own self-import. An
OpenAttestation-style design, **implemented from scratch** with a JSON-free, language-agnostic
(circom/TS/Rust/Solidity) canonicalization on one pinned circomlib BN254 Poseidon.

## Status: LIVE on ROAX (chainId 135)
The full system is **built and DEPLOYED LIVE** to the ROAX testnet, and the **end-to-end demo runs on
a real Android device**. The proper onboarding flow is **vets-issue-dog-tags**: the **admin portal only
approves + whitelists vet/groomer wallet addresses** — **both issuers and verifiers onboard via
apply→approve** (approval whitelists issuance record-types **and** `VERIFY:<purpose>` on-chain) — it does
**not** register devices or mint dog tags (there is **no** `POST /v1/register` and **no** admin
"Registered devices" / "Mint dog-tag" page; the phone has **no** "Central API URL" — every host comes
from a scanned QR). To get a dog tag: the phone creates a self-custodial wallet, the **vet** "Issue dog
tag" wizard (operator enters an `ownerIdentity` + pet fields) starts a session and shows a QR
`/p/<token>`, the device scans it and sends its **signed wallet address**, and the **vet mints the
`DogTagSBT` on-chain** (`mint(walletAddress, dogTagId, root)`) and returns the credential, which the
device verifies against the SBT and imports. This requires the vet signer to hold
`DogTagSBT.ISSUER_ROLE` (granted by the protocol admin — a trust escalation, so prod grants it only to
accredited vets). Then: the vet issues a vaccination → QR → scan → import → verify on-chain → view
decoded fields. The ZK proof-of-verification path is **live** (Groth16Verifier wired into the
VerificationRegistry); the proof is generated **on-device on 64-bit phones** (mopro `circom-prover` +
`circom-witnesscalc` graph witness) and via a **backend prover-service** (`POST /prove-verification`)
on **32-bit-only Android** that cannot prove locally, and the device stays **gasless** throughout
(the consent-key bind is relayer-sponsored). Live contract addresses are in
**[`contracts/deployments/roax.json`](contracts/deployments/roax.json)** — see the table below.

**Two deployment modes.** A single `VITE_DEMO_MODE` flag (set = demo, **unset = production**) switches
between them:
- **LOCAL** — **[`docs/LOCAL_DEPLOYMENT.md`](docs/LOCAL_DEPLOYMENT.md)**: the click-through demo (forms
  auto-filled, demo buttons, ephemeral MemStore, LAN/tunnel). Automated verification: `scripts/e2e-smoke.sh`
  (7 steps, all PASS on ROAX).
- **REMOTE** — **[`docs/REMOTE_DEPLOYMENT.md`](docs/REMOTE_DEPLOYMENT.md)**: hardened, self-hosted-per-business,
  persistent Mongo, real domain + TLS, real DNS-TXT legitimacy, operators type everything (**no demo buttons**).

Demo runbook + literal click-through: **[`docs/DEMO.md`](docs/DEMO.md)** + **[`docs/DEMO_CLICKS.md`](docs/DEMO_CLICKS.md)**.

## Live ROAX addresses (chainId 135)
Source of truth: [`contracts/deployments/roax.json`](contracts/deployments/roax.json).

| Contract | Address |
|---|---|
| IssuerRegistry | `0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c` |
| DogTagSBT | `0x1FB8986573Ac36d532cF7d5a5352202B094D4233` |
| DogTagIssuerFactory | `0xd3179AbBfb0274D0a5F7017d76015A93C159511D` |
| DogTagIssuerImpl (clone impl) | `0x16671686a5926606aB05f5e167fC65B0f8825B85` |
| ConsentKeyRegistry (gasless `bindConsentKeyFor`) | `0xA74DDe4a9b5b5b9045D9244907dE5d84C75BD671` |
| Poseidon6 | `0x58091F2320c78ed6c6D1C02CB7E5c7578f1349db` |
| **VerificationRegistry** (ZK-wired) | `0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1` |
| Groth16Verifier | `0x138b433071Ad806E841B5AD53623290a9bf21761` |
| admin / deployer | `0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96` |
| demo clone — VACCINATION | `0x5c703910111f942EE0f47E02214291b5274cDb53` |
| demo clone — DOG_PROFILE | `0xdb8d39eb83DDFAaA7481C4Af4e47D0044116dB25` |

> The original VerificationRegistry was deployed with `zkVerifier = 0`; for the testnet the registry was
> **redeployed** pointing at the live Groth16Verifier (`VerificationRegistry_zk0_legacy`
> `0xb4FbbDb5…` is the retired zk=0 instance), and a later **meta-tx migration** produced the current VR
> `0x8bA836eCe9…` + CKR `0xA74DDe4a9b…` (retiring the `_preMetaTx_legacy` VR `0x19C1B5f8…` and CKR
> `0xFD277b9B…`) — three VR generations in all. In production the verifier is wired via the registry's
> 2-day timelock (`proposeZkVerifier(addr)` → wait ≥ 2 days → `executeZkVerifier()`) instead — see
> [`docs/DEPLOY.md`](docs/DEPLOY.md). The testnet ZK trusted setup (3 contributions + beacon) is recorded
> in [`docs/CEREMONY_TRANSCRIPT.md`](docs/CEREMONY_TRANSCRIPT.md).

## Start here
- **[`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md)** — **deployment index — start here** (tier decision-guide, system model, address book, routing).
- **[`docs/PREREQUISITES.md`](docs/PREREQUISITES.md)** — canonical install/tooling matrix (macOS + Linux, per-tool "needed by").
- **[`docs/PRODUCTION_DEPLOYMENT.md`](docs/PRODUCTION_DEPLOYMENT.md)** — Tier 3 go-live hardening (chain-swap + ceremony/timelock runbook).
- **[`docs/MOBILE_BUILD.md`](docs/MOBILE_BUILD.md)** — build + install the iOS & Android apps on real phones.
- **[`docs/TUNNELING.md`](docs/TUNNELING.md)** — the 3-tunnel reference + phone networking + ephemerality.
- **[`docs/LOCAL_DEPLOYMENT.md`](docs/LOCAL_DEPLOYMENT.md)** — LOCAL/demo runbook (`VITE_DEMO_MODE=1`, auto-filled, ephemeral).
- **[`docs/REMOTE_DEPLOYMENT.md`](docs/REMOTE_DEPLOYMENT.md)** — REMOTE/production runbook (persistent, TLS, DNS-TXT, operators type everything).
- **[`docs/DEMO.md`](docs/DEMO.md)** + **[`docs/DEMO_CLICKS.md`](docs/DEMO_CLICKS.md)** — run the LIVE demo (narrated + literal click-through).
- **[`docs/architecture.md`](docs/architecture.md)** — system + smart-contract architecture (§13 = normative audit remediations).
- **[`docs/implementation.md`](docs/implementation.md)** — per-function pseudocode, contract bodies, endpoints, Docker, deploy (§11 = normative corrected code).
- **[`docs/BUILD_PROMPT.md`](docs/BUILD_PROMPT.md)** — the phased build-out prompt.
- **[`docs/DEPLOY.md`](docs/DEPLOY.md)** — ROAX deploy runbook (already deployed; Gate B prechecks, ceremony, Docker bring-up).
- **[`docs/CEREMONY_RUNBOOK.md`](docs/CEREMONY_RUNBOOK.md)** / **[`docs/CEREMONY.md`](docs/CEREMONY.md)** / **[`docs/CEREMONY_TRANSCRIPT.md`](docs/CEREMONY_TRANSCRIPT.md)** — ZK trusted-setup (the captain-fill-in production runbook + concise version + the testnet transcript).
- **[`docs/DPIA.md`](docs/DPIA.md)** — mandatory Data Protection Impact Assessment.
- **[`docs/research/`](docs/research)** — research briefs + security audits behind every decision.

## Components
| Path | What | Runs where |
|---|---|---|
| `apps/android`, `apps/ios` | Pet-owner apps (Kotlin/Compose, Swift/SwiftUI), 7 themes, self-custodial MPC wallet | User devices |
| `stacks/vet` | Self-hosted vet stack — React+Vite SPA + Rust `vet-api` + Mongo (issue/share/verify/calendar) | Each vet |
| `stacks/groomer` | Self-hosted groomer stack — SPA + **the same `vet-api` binary** (`BUSINESS_TYPE=groomer`) + Mongo | Each groomer |
| `stacks/government` | **Net-new** government credential-authority stack — SPA + **its own `government-api` binary** + Mongo (issue TRAVEL_CLEARANCE/EU_HEALTH_CERT + government-grade verify) — see [`docs/ROLE_APPS.md`](docs/ROLE_APPS.md) | Each competent authority |
| `stacks/admin` | Central registry, issuer whitelisting, mobile API, appointment source-of-truth, erasure | We host |
| `contracts` | `DogTagSBT` (ERC-5192) · `IssuerRegistry` · `DogTagIssuer` (clones) + factory · `VerificationRegistry` · `ConsentKeyRegistry` | ROAX |
| `circuits` | Groth16 Poseidon-Merkle + EdDSA-BabyJubjub consent circuit (N=24, depth 5) | Prover image |
| `crates/dogtag-standard-rs`, `packages/dogtag-standard-ts` | The open data standard: canonicalization + Poseidon-Merkle + verify + consent | Shared (UniFFI → mobile) |
| `crates/dogtag-prover-rs` | ark-circom + ark-groth16 proof builder — **test oracle** for `scripts/e2e-zk.sh` (prod proving is **on-device** via mopro) | test/e2e |
| `packages/ui` | Shared React components + light/dark theme tokens | Portals |

## Ports (uncommon; Mongo internal-only, NEVER published to the host)
| Stack | web (host) | api (host) | mongo |
|---|---|---|---|
| **admin** (central) | **39741** | **39742** | internal only |
| **vet** | **41873** | **41874** | internal only |
| **groomer** | **43617** | **43618** | internal only |
| **government** | **44831** | **44832** | internal only |

Each stack is `web` (nginx serving the Vite build) + `api` (Rust binary, multi-stage build) +
`mongo` (compose-network-internal). The groomer `api` runs the **`vet-api`** binary with
`BUSINESS_TYPE=groomer` (host `43618` → container `43618`). The **government** `api` runs its **own**
`government-api` binary (a genuinely separate deployable — not a `vet-api` re-run); see
[`docs/ROLE_APPS.md`](docs/ROLE_APPS.md) for the three-role separation design.

## Build & test

**Everything (root targets):**
```bash
make build     # SDKs (TS + Rust) + contracts
make test      # Poseidon 4-language parity gate + TS/Rust SDK + Foundry
make parity    # the NORMATIVE Poseidon anchor gate (t=2/3/6/7) — blocks downstream
```

**Per stack:**
```bash
# Rust business backend (vet + groomer share this crate):
cargo test -p vet-api
# Central/admin backend:
cargo test -p admin-api
# Government credential-authority backend (its own crate):
cargo test -p government-api
# Web portals (Vite build):
pnpm --filter @dogtag/vet-web build
pnpm --filter @dogtag/groomer-web build
pnpm --filter @dogtag/admin-web build
pnpm --filter @dogtag/government-web build
# Contracts:
cd contracts && forge test
```

**Run a stack (Docker — Mongo internal-only):**
```bash
cp stacks/vet/.env.example stacks/vet/.env   # fill addrs + secrets
make up-vet        # or up-admin / up-groomer / up-government
```
See **[`docs/DEPLOY.md`](docs/DEPLOY.md)** for the full deploy + ceremony runbook.

## Privacy gates (Phase 8)
Cross-cutting CI guardrails enforce the privacy claims:
- **Dual-signing parity** (`stacks/vet/api/tests/gate_dual_signing_parity.rs`) — wallet vs backend mode yield byte-identical `merkleRoot`/`targetHash`/records (build is server-side in both modes).
- **PII-off-chain negative** (`stacks/admin/api/tests/gate_pii_off_chain.rs`) — `dogTagId` is never `keccak256`/`Poseidon` of the microchip; only the **salted** root is anchored.
- **Erasure-unlinkability** (`stacks/admin/api/tests/gate_erasure_unlinkability.rs`) — after `erase()`, the per-record DEK is destroyed and salts/PII (incl. `verification_records`) **decrypt fails** → on-chain commitment unlinkable.
- **Behavioral-privacy** (`stacks/vet/api/tests/gate_behavioral_privacy.rs`) — the **EXPORT** session (`/verify/session/start`, the owner→groomer proof flow) defaults to **ZK** for sensitive purposes; the **proof is generated on-device** so the groomer never receives the record; fresh-per-pet `subject` bounds linkage to one pet.

## Status (Phases 0–8)
| Phase | Scope | Status |
|---|---|---|
| 0 | Monorepo scaffold (pnpm + Cargo + Foundry workspaces, Makefile) | ✅ Done |
| 1 | Shared Poseidon standard SDKs (4-language bit-identical parity) | ✅ Done |
| 2 | Smart contracts (SBT, IssuerRegistry, DogTagIssuer clones, factory) | ✅ Done |
| 2.5 | ZK verification subsystem (circuit, VerificationRegistry, ConsentKeyRegistry) | ✅ Done |
| 3 | Vet business backend (Rust): issue→share→verify, dual signing, custody | ✅ Done |
| 4 | Central/admin backend: discovery, whitelisting, appointments, erasure | ✅ Done |
| 5 | Web portals (vet/groomer/admin; light/dark, wallet-connect, Export UI) | ✅ Done |
| 6 | Mobile apps (Android + iOS): import, on-device-proof export, wallet, consent signing | ✅ Done |
| 7 | Calendar sync + cross-backend appointments | ✅ Done |
| 8 | Hardening: per-stack Docker, privacy/parity gates, DEPLOY + DPIA docs | ✅ Done |
| — | **DEPLOYED LIVE on ROAX (chainId 135)** — contracts live, ZK path wired, demo verified on a real Android device | ✅ Live |

> **Deployment note:** all contracts are **deployed and live on ROAX** (`contracts/deployments/roax.json`),
> the ZK proof-of-verification path is **wired and live** (Groth16Verifier in the VerificationRegistry),
> and the end-to-end flow was verified by `scripts/e2e-smoke.sh` (7/7 PASS) **and** a manual Android run.
> The demo backends run from the in-memory store via `scripts/demo-up.sh` (Docker compose files are also
> present and validated by syntax). The shipped ZK trusted setup is a **single-operator testnet** run
> (`docs/CEREMONY_TRANSCRIPT.md`); production requires the multi-party ceremony in `docs/CEREMONY.md` /
> `docs/DEPLOY.md` §4.
