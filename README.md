# DogTag — Pet Credentialing Ecosystem (monorepo)

Verifiable, on-chain pet credentials (identity, vaccination, service, travel) anchored as
**salted-Merkle Poseidon roots** on the **ROAX** EVM chain (chainId **135**, gas token **PLASMA**),
verified three ways — cryptographic **integrity** + on-chain **status** + DNS-bound issuer
**identity** — plus a contextual **ownership** fragment for the owner's own self-import. An
OpenAttestation-style design, **implemented from scratch** with a JSON-free, language-agnostic
(circom/TS/Rust/Solidity) canonicalization on one pinned circomlib BN254 Poseidon.

## Start here
- **[`docs/architecture.md`](docs/architecture.md)** — system + smart-contract architecture (§13 = normative audit remediations).
- **[`docs/implementation.md`](docs/implementation.md)** — per-function pseudocode, contract bodies, endpoints, Docker, deploy (§11 = normative corrected code).
- **[`docs/BUILD_PROMPT.md`](docs/BUILD_PROMPT.md)** — the phased build-out prompt.
- **[`docs/DEPLOY.md`](docs/DEPLOY.md)** — ROAX deploy runbook (Gate B prechecks, ceremony, Docker bring-up).
- **[`docs/DPIA.md`](docs/DPIA.md)** — mandatory Data Protection Impact Assessment.
- **[`docs/research/`](docs/research)** — research briefs + security audits behind every decision.

## Components
| Path | What | Runs where |
|---|---|---|
| `apps/android`, `apps/ios` | Pet-owner apps (Kotlin/Compose, Swift/SwiftUI), 7 themes, self-custodial MPC wallet | User devices |
| `stacks/vet` | Self-hosted vet stack — React+Vite SPA + Rust `vet-api` + Mongo (issue/share/verify/calendar) | Each vet |
| `stacks/groomer` | Self-hosted groomer stack — SPA + **the same `vet-api` binary** (`BUSINESS_TYPE=groomer`) + Mongo | Each groomer |
| `stacks/admin` | Central registry, issuer whitelisting, mobile API, appointment source-of-truth, erasure | We host |
| `contracts` | `DogTagSBT` (ERC-5192) · `IssuerRegistry` · `DogTagIssuer` (clones) + factory · `VerificationRegistry` · `ConsentKeyRegistry` | ROAX |
| `circuits` | Groth16 Poseidon-Merkle + EdDSA-BabyJubjub consent circuit (N=24, depth 5) | Prover image |
| `crates/dogtag-standard-rs`, `packages/dogtag-standard-ts` | The open data standard: canonicalization + Poseidon-Merkle + verify + consent | Shared (UniFFI → mobile) |
| `crates/dogtag-prover-rs` | ark-circom + ark-groth16 witness/proof builder | vet/groomer api |
| `packages/ui` | Shared React components + light/dark theme tokens | Portals |

## Ports (uncommon; Mongo internal-only, NEVER published to the host)
| Stack | web (host) | api (host) | mongo |
|---|---|---|---|
| **admin** (central) | **39741** | **39742** | internal only |
| **vet** | **41873** | **41874** | internal only |
| **groomer** | **43617** | **43618** | internal only |

Each stack is `web` (nginx serving the Vite build) + `api` (Rust binary, multi-stage build) +
`mongo` (compose-network-internal). The groomer `api` runs the **`vet-api`** binary with
`BUSINESS_TYPE=groomer` (host `43618` → container `41874`).

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
# Web portals (Vite build):
pnpm --filter @dogtag/vet-web build
pnpm --filter @dogtag/groomer-web build
pnpm --filter @dogtag/admin-web build
# Contracts:
cd contracts && forge test
```

**Run a stack (Docker — Mongo internal-only):**
```bash
cp stacks/vet/.env.example stacks/vet/.env   # fill addrs + secrets
make up-vet        # or up-admin / up-groomer
```
See **[`docs/DEPLOY.md`](docs/DEPLOY.md)** for the full deploy + ceremony runbook.

## Privacy gates (Phase 8)
Cross-cutting CI guardrails enforce the privacy claims:
- **Dual-signing parity** (`stacks/vet/api/tests/gate_dual_signing_parity.rs`) — wallet vs backend mode yield byte-identical `merkleRoot`/`targetHash`/records (build is server-side in both modes).
- **PII-off-chain negative** (`stacks/admin/api/tests/gate_pii_off_chain.rs`) — `dogTagId` is never `keccak256`/`Poseidon` of the microchip; only the **salted** root is anchored.
- **Erasure-unlinkability** (`stacks/admin/api/tests/gate_erasure_unlinkability.rs`) — after `erase()`, the per-record DEK is destroyed and salts/PII (incl. `verification_records`) **decrypt fails** → on-chain commitment unlinkable.
- **Behavioral-privacy** (`stacks/vet/api/tests/gate_behavioral_privacy.rs`) — `/verify/session/start` defaults to **ZK** for sensitive purposes; fresh-per-pet `subject` bounds linkage to one pet.

## Status (Phases 0–8)
| Phase | Scope | Status |
|---|---|---|
| 0 | Monorepo scaffold (pnpm + Cargo + Foundry workspaces, Makefile) | ✅ Done |
| 1 | Shared Poseidon standard SDKs (4-language bit-identical parity) | ✅ Done |
| 2 | Smart contracts (SBT, IssuerRegistry, DogTagIssuer clones, factory) | ✅ Done |
| 2.5 | ZK verification subsystem (circuit, VerificationRegistry, ConsentKeyRegistry) | ✅ Done |
| 3 | Vet business backend (Rust): issue→share→verify, dual signing, custody | ✅ Done |
| 4 | Central/admin backend: discovery, whitelisting, appointments, erasure | ✅ Done |
| 5 | Web portals (vet/groomer/admin; light/dark, wallet-connect, Verify UI) | ✅ Done |
| 6 | Mobile apps (Android + iOS): verify, wallet, consent signing | ✅ Done |
| 7 | Calendar sync + cross-backend appointments | ✅ Done |
| 8 | Hardening: per-stack Docker, privacy/parity gates, DEPLOY + DPIA docs | ✅ Done |

> **Note:** the Docker build files are validated by **syntax** (valid compose YAML + Dockerfiles);
> they have not been `docker compose up`-run in this environment (the daemon is off). The ZK trusted
> setup shipped for tests is a **dev** setup — production requires the multi-party ceremony in
> `docs/DEPLOY.md` §4.
