# DogTag — Pet Credentialing Ecosystem (monorepo)

Verifiable, on-chain pet credentials (identity, vaccination, service, travel) anchored as
**salted-Merkle Poseidon roots** on the **ROAX** EVM chain (chainId **135**, gas token **PLASMA**),
verified three ways — cryptographic **integrity** + on-chain **status** + DNS-bound issuer
**identity** — plus a contextual **ownership** fragment for the owner's own self-import. An
OpenAttestation-style design, **implemented from scratch** with a JSON-free, language-agnostic
(circom/TS/Rust/Solidity) canonicalization on one pinned circomlib BN254 Poseidon.

## Status: deployed on ROAX (chainId 135)
The full system is **built and deployed to the ROAX testnet**, and the protocol has **one model**: the owner is a **hidden leaf** of the tag's Merkle tree, and every verification is **owner-hidden**.
A dog tag is a salted Merkle tree with a single Poseidon root `R`; three reserved owner leaves (owner-address, consent-key, owner-secret) fold into `R` **on the owner's device**, next to the disclosable pet-attribute leaves, so the issuer never learns them.
A **Merkle proof is issuance**: verifying a credential and selectively disclosing fields is an inclusion check against `R` (or a record root).
A **ZK proof is consent**: a Groth16 proof over `circuits/consent.circom` (`DogTagConsent(6)`) proves - without revealing the owner - that the owner of tag `dogTagId` consented to a verification for `purpose` bound to the verifying operator, and `VerificationRegistryConsent` checks it on-chain and emits a **subject-less** `Verified` event.

The proper onboarding flow is **vets-issue-dog-tags**: the **admin portal only approves + whitelists vet/groomer wallet addresses** - **both issuers and verifiers onboard via apply→approve** (approval whitelists issuance record-types **and** `VERIFY:<purpose>` on-chain) - it does **not** register devices or mint dog tags (there is **no** `POST /v1/register` and **no** admin "Registered devices" / "Mint dog-tag" page; the phone has **no** "Central API URL" - every host comes from a scanned QR).
To get a dog tag: the phone creates a self-custodial wallet, the **vet** "Register pet" wizard (operator enters an `ownerIdentity` + pet fields) starts an issuance session and shows a one-time QR `/p/<token>`, the device scans it, folds its owner leaves into the profile root `R` **on-device**, and submits only `{token, root}` to `POST /profiles/issue/custodial-bind`.
The vet signer then seals `R` on-chain via `DogTagSBTConsent.mintCustodial(dogTagId, R)` (write-once `profileRoot`, minted to a neutral custodian) and returns the credential, which the device verifies and imports - **no owner wallet address is ever expressible in the calldata**.
This requires the vet signer to hold `DogTagSBTConsent.ISSUER_ROLE` (granted by the protocol admin - a trust escalation, so prod grants it only to accredited vets).
Then: the vet issues a vaccination → QR → scan → import → verify on-chain → view decoded fields.
Verification is the owner-hidden consent path: the phone builds the `consent.circom` Groth16 proof **on-device** (mopro `circom-prover` + `circom-witnesscalc` graph witness) and hands it to the verifying operator, who relays it to `VerificationRegistryConsent` - the device stays **gasless** throughout.
A backend prover-service (`POST /prove-consent`) exists as the **owner-trusted server-prove fallback** for a device that cannot prove locally: it sees the witness, but the verifier, relayer, and chain still never learn the owner.
The earlier owner-revealing protocol path (its SBT, registry, key registry, verifier, and circuit) is **retired**: its sources are deleted from the repo, and its already-deployed instances remain on-chain only as historical records pending the fresh testnet redeploy.
Contract addresses are in **[`contracts/deployments/roax.json`](contracts/deployments/roax.json)** - see the table below.

**Two deployment modes.** A single `VITE_DEMO_MODE` flag (set = demo, **unset = production**) switches
between them:
- **LOCAL** — **[`docs/LOCAL_DEPLOYMENT.md`](docs/LOCAL_DEPLOYMENT.md)**: the click-through demo (forms
  auto-filled, demo buttons, ephemeral MemStore, LAN/tunnel). Automated verification: `scripts/e2e-smoke.sh`
  (generic credential lifecycle) + `scripts/e2e-zk.sh` (the owner-hidden consent path).
- **REMOTE** — **[`docs/REMOTE_DEPLOYMENT.md`](docs/REMOTE_DEPLOYMENT.md)**: hardened, self-hosted-per-business,
  persistent Mongo, real domain + TLS, real DNS-TXT legitimacy, operators type everything (**no demo buttons**).

Demo runbook + literal click-through: **[`docs/DEMO.md`](docs/DEMO.md)** + **[`docs/DEMO_CLICKS.md`](docs/DEMO_CLICKS.md)**.

## ROAX addresses (chainId 135)

Every address, transaction hash and block is in
[`contracts/deployments/roax.json`](contracts/deployments/roax.json), which is the only place they
belong.
They are deliberately not transcribed here: this section used to carry the full table, and every
entry in it went stale at once when the launch set was deployed - a reader trusting the README would
have pointed a client at contracts that no longer decide anything.

What is deployed is one set of ten contracts, deployed by a single run of
`contracts/script/Deploy.s.sol` and each represented exactly once in the ledger:

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

Two facts about that set are easy to get wrong and are recorded in the ledger rather than here:
`VerificationRegistryConsent.rootIndex` IS the factory and is immutable (`_root_index`), and the
consent verifier was redeployed with byte-identical runtime, so no zero-knowledge artifact rotated
(`_frozen_verifier`).

No provider is onboarded yet - `providerCount` is 0, so no clone exists and nothing has been issued.
Nothing under `stacks/`, `apps/`, `packages/` or `crates/` resolves these addresses yet, so the
running demo stack and both mobile bundles still name superseded contracts.

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
- **[`docs/CEREMONY_RUNBOOK.md`](docs/CEREMONY_RUNBOOK.md)** / **[`docs/CEREMONY.md`](docs/CEREMONY.md)** / **[`docs/CEREMONY_TRANSCRIPT.consent.md`](docs/CEREMONY_TRANSCRIPT.consent.md)** - ZK trusted-setup (the captain-fill-in production runbook + concise version + the live consent-circuit testnet transcript; the retired circuit's transcript stays at [`docs/CEREMONY_TRANSCRIPT.md`](docs/CEREMONY_TRANSCRIPT.md)).
- **[`docs/DPIA.md`](docs/DPIA.md)** — mandatory Data Protection Impact Assessment.
- **[`docs/research/`](docs/research)** — research briefs + security audits behind every decision.

## Components
| Path | What | Runs where |
|---|---|---|
| `apps/android`, `apps/ios` | Pet-owner apps (Kotlin/Compose, Swift/SwiftUI), 7 themes, self-custodial MPC wallet | User devices |
| `stacks/owner/web` | Pet-owner (**holder**) wallet - the web mirror of the native apps: **no backend**, receives/holds a wrapped credential, displays it (incl. printable travel/health **receipts**), and shares a **selectively-redacted copy** via local `obfuscate` (root unchanged). It runs no ZK prover - consent proofs are generated on-device by the native apps (backend `POST /prove-consent` is the fallback) - see [`stacks/owner/web/README.md`](stacks/owner/web/README.md) | Owner's browser |
| `stacks/vet` | Self-hosted vet stack — React+Vite SPA + Rust `vet-api` + Mongo (issue/share/verify/calendar) | Each vet |
| `stacks/groomer` | Self-hosted groomer stack — SPA + **the same `vet-api` binary** (`BUSINESS_TYPE=groomer`) + Mongo | Each groomer |
| `stacks/government` | **Net-new** government credential-authority stack — SPA + **its own `government-api` binary** + Mongo (issue TRAVEL_CLEARANCE/EU_HEALTH_CERT + government-grade verify) — see [`docs/ROLE_APPS.md`](docs/ROLE_APPS.md) | Each competent authority |
| `stacks/admin` | Central registry, issuer whitelisting, mobile API, appointment source-of-truth, erasure | We host |
| `contracts` | the ten launch contracts, one set: `ProviderRegistry` (provider identity + every authority predicate) · `DogTagIssuer` clones + `DogTagIssuerFactory` (self-service deployment + the write-once root index) · `DogTagSBTConsent` (write-once `profileRoot`, neutral custodial sink) · `Groth16VerifierConsent` · `VerificationRegistryConsent` (owner-blind `Verified`) · `ProviderDirectory` / `ServiceDomainResolver` (the typed resolvers) · `ProtocolRegistry` (discovery anchor) · `IERC5192`. No superseded generation exists in source | ROAX |
| `circuits` | Groth16 owner-hidden consent circuit `DogTagConsent(6)` (reserved-owner-leaf Merkle membership + EdDSA consent + hidden-owner nullifier; depth 6, ≤64 leaves); the retired owner-revealing `verification.circom` is gone from source, its frozen build products kept as provenance | Prover image |
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
| **owner-wallet** (holder) | **45931** | — (no backend) | — (localStorage) |

Each **role** stack is `web` (nginx serving the Vite build) + `api` (Rust binary, multi-stage build) +
`mongo` (compose-network-internal). The groomer `api` runs the **`vet-api`** binary with
`BUSINESS_TYPE=groomer` (host `43618` → container `43618`). The **government** `api` runs its **own**
`government-api` binary (a genuinely separate deployable — not a `vet-api` re-run); see
[`docs/ROLE_APPS.md`](docs/ROLE_APPS.md) for the three-role separation design.
The **owner-wallet** is the odd one out: the pet-owner (holder) front has **no backend and no Mongo**
(state lives in the browser's localStorage), so `scripts/demo-up.sh` runs it as a plain Vite dev server
on `45931`; its only network dependency is the public ROAX RPC for on-chain validity reads - it runs
no prover and calls no prover service.

## Build & test

**Everything (root targets):**
```bash
make build     # SDKs (TS + Rust) + contracts
make test      # Poseidon 4-language parity gate + TS/Rust SDK + Foundry
make parity    # the NORMATIVE Poseidon anchor gate (t=2/3/6/7) — blocks downstream

make test-consent-parity   # consent prove <-> frozen VK agreement — LOUD gate, deliberately NOT in
                           # `make test` (real Groth16, slow). Both proving artifacts are committed,
                           # so a plain checkout runs it; it fails closed if either is absent.

make vendor-mobile-artifacts  # copy the consent zkey+graph into both app bundles (needed before
                              # either mobile app can prove — see docs/MOBILE_BUILD.md §4)
```

**End-to-end — one command per surface, from a fresh worktree, nothing served first:**
```bash
make e2e-web              # every portal: starts them + a hermetic government backend, then tears down
make e2e-web ONLY=vet     # one portal
make e2e-ios              # builds the app, boots a simulator, runs the Maestro flows
make e2e-android          # builds the apk, installs on a connected arm64 device, runs Maestro
make e2e                  # all three
```
Each exits **0** passed, **1** failed, and **78** when a prerequisite is genuinely missing — a loud
"THE SUITE DID NOT RUN" naming the specific thing, never a silent skip and never a green.
A suite that executed zero tests, or skipped any, is treated as a FAILURE.
Full rationale, and what each surface refuses on: **[`docs/E2E_TESTING.md`](docs/E2E_TESTING.md)**.

These are deliberately NOT in `make test`: they serve portals and need a device.

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
# Pet-owner (holder) wallet - the backend-less web mirror of the native apps:
pnpm --filter @dogtag/owner-web build
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
- **Erasure-unlinkability** (`stacks/admin/api/tests/gate_erasure_unlinkability.rs`) — after `erase()`, the per-record DEK is destroyed and salts/PII (incl. `verification_records`) **decrypt fails** → on-chain commitment unlinkable.
- The gates that policed the retired owner-revealing EXPORT path were deleted with it.
  Their properties now hold by construction: the consent `Verified` event carries **no subject at all**, nothing personal is ever on-chain, and the on-chain `dogTagId` is the field-hash of the human-typed handle, never any hash of the microchip.

## Status (Phases 0–8)
| Phase | Scope | Status |
|---|---|---|
| 0 | Monorepo scaffold (pnpm + Cargo + Foundry workspaces, Makefile) | ✅ Done |
| 1 | Shared Poseidon standard SDKs (4-language bit-identical parity) | ✅ Done |
| 2 | Smart contracts (SBT, provider authority core, DogTagIssuer clones, factory) | ✅ Done |
| 2.5 | ZK verification subsystem (consent circuit, verifier, verification registry) | ✅ Done |
| 3 | Vet business backend (Rust): issue→share→verify, dual signing, custody | ✅ Done |
| 4 | Central/admin backend: discovery, whitelisting, appointments, erasure | ✅ Done |
| 5 | Web portals (vet/groomer/admin; light/dark, wallet-connect, Export UI) | ✅ Done |
| 6 | Mobile apps (Android + iOS): import, on-device-proof export, wallet, consent signing | ✅ Done |
| 7 | Calendar sync + cross-backend appointments | ✅ Done |
| 8 | Hardening: per-stack Docker, privacy/parity gates, DEPLOY + DPIA docs | ✅ Done |
| — | **DEPLOYED on ROAX (chainId 135)** - contract set on-chain; the owner-hidden consent set is the live protocol surface | ✅ Deployed |

> **Deployment note:** the contract set is **deployed on ROAX** (`contracts/deployments/roax.json`);
> the live protocol surface is the owner-hidden consent set
> (`DogTagSBTConsent` + `VerificationRegistryConsent` + `Groth16VerifierConsent`).
> The phases table above is history: the owner-revealing subsystem built in Phase 2/2.5 was later retired
> in favor of the consent set, and its sources are gone from the repo.
> The ROAX testnet is disposable: the fresh redeploy of the unified deployment executed on 2026-07-23
> (`_r8_fresh_redeploy` in `roax.json`) - the live set starts empty-fresh, no pre-unification records
> carry over, and the retired-generation instances remain on-chain as deployment history only.
> The demo backends run from the in-memory store via `scripts/demo-up.sh` (Docker compose files are also
> present and validated by syntax). The shipped consent trusted setup is a **single-operator testnet** run
> (`docs/CEREMONY_TRANSCRIPT.consent.md`); production requires the multi-party ceremony in
> `docs/CEREMONY.md` / `docs/CEREMONY_RUNBOOK.md`.
