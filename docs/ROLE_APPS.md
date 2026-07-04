# DogTag Role Applications — vet · groomer · government

> Status: v1 (this PR).
> Scope: architect the three **role applications** — **vet**, **groomer**, and **government** — as **separately deployable** stacks, each with its own on-chain wiring **and** its own centralized (off-chain) database, so every role is demoable end-to-end.
> Chain: **ROAX testnet** (EVM, chainId **135**, gas token PLASMA) — but every stack runs exactly as it would on mainnet: separate `docker-compose`, own Mongo, own signer / on-chain wiring.
> Companion docs: [`architecture.md`](./architecture.md) (§1.2 two-backend model, §3.6 record types, §4 contracts, §5 verification), [`implementation.md`](./implementation.md) (§7 Docker), and the live address book in [`../contracts/deployments/roax.json`](../contracts/deployments/roax.json).

---

## 1. The captain's model — real separation, one operator

Each participant (vet / groomer / government) is a **self-contained deployable**: its own `docker-compose` project, its own MongoDB (internal to that compose network, never published to the host), its own signer, and its own on-chain wiring against the shared ROAX contracts.
This mirrors mainnet exactly — in production each business runs its own stack behind its own TLS domain.
For the demo, **the captain operates all of them**, but nothing about the code or compose topology assumes that: the stacks share **no** process, database, or key material.

The shared, deployed-once substrate stays untouched by this work:

- **Contracts** (ROAX) — `DogTagSBT`, `IssuerRegistry`, `DogTagIssuer` clones + factory, `VerificationRegistry`, `ConsentKeyRegistry`, `Groth16Verifier`. Addresses in `contracts/deployments/roax.json`.
- **The open standard** — `crates/dogtag-standard-rs` / `packages/dogtag-standard-ts`: canonicalization + salted-leaf Poseidon-Merkle root `R` + verify. Every role stack builds and verifies credentials through this one SDK, so a credential issued by any role verifies identically everywhere.

The three roles differ only in **which record types they issue**, **what off-chain data they are the custodian of**, and **which on-chain capability (issue vs verify) they exercise**.

---

## 2. Side-by-side: the three role stacks

| | **vet** (`stacks/vet`) | **groomer** (`stacks/groomer`) | **government** (`stacks/government`) — NET-NEW |
|---|---|---|---|
| Deployable | separate `docker-compose` | separate `docker-compose` | separate `docker-compose` |
| API binary | `vet-api` (own crate) | **`vet-api`** run with `BUSINESS_TYPE=groomer` | **`government-api`** (own crate — genuinely separate) |
| Ports (web / api) | 41873 / 41874 | 43617 / 43618 | **44831 / 44832** |
| Database | own Mongo (`vetdata`) | own Mongo (`groomerdata`) | own Mongo (`governmentdata`) |
| On-chain **issue** | `DOG_PROFILE` (mints SBT), `VACCINATION`, `SERVICE_ATTESTATION` | — (verifier only) | **`TRAVEL_CLEARANCE`, `EU_HEALTH_CERT`** |
| On-chain **verify** | `VET_INTAKE` presentations | `GROOMING_INTAKE` presentations (the canonical verifier) | **government-grade credential verification** (integrity + status + issuer identity) |
| On-chain **govern** | — | — | — (governance stays with `stacks/admin` — see §6) |
| Trust tier issued | `licensed_vet` | n/a | **`accredited_authority`** (authority-endorsement) |

The vet and groomer stacks are **already real and already separately deployable** (see §5); the government stack is **net-new** and is what this PR begins building (§4).

> **Not a role stack: the holder.** This doc covers the **issuer/verifier** roles. The **pet-owner (holder)** side (the counterpart that receives, holds, and *presents* these credentials) is a separate component with no backend and no database: the native `apps/android`/`apps/ios` wallets and their web mirror `stacks/owner/web` (`@dogtag/owner-web`, port **45931**). See [`../stacks/owner/web/README.md`](../stacks/owner/web/README.md).

---

## 3. Per-role design

### 3.1 Vet — issuer + verifier (licensed_vet)

**On-chain responsibilities.**
The vet is the primary **issuer**: it mints the pet-identity SBT (`DogTagSBT.mint` under `DOG_PROFILE`, requires `ISSUER_ROLE`) and anchors `VACCINATION` / `SERVICE_ATTESTATION` roots on its `DogTagIssuer` clone (gated by `IssuerRegistry.isWhitelistedFor(recordType, signer)`).
It is also a **verifier** for `VET_INTAKE` presentations (`VerificationRegistry`, normal + ZK paths).

**Centralized DB (what it stores off-chain and why).**
The vet backend is the legal **record-custodian**: it holds full credential records (salted cleartext leaves, per-record DEKs), the age-encrypted custody seed (its signer), operator/admin sessions, appointment replicas, and calendar-sync state.
On-chain we anchor **only** the salted root `R` — all PII stays in the vet's Mongo, erasable per the DPIA.

**API + web surface.**
`vet-api` (Axum): issue → prepare/confirm, share, third-party verify, export-session (owner→verifier ZK consent), calendar sync, custody genesis/unlock; records management (`GET /records` operator-gated list, `PATCH /records/:id` off-chain metadata only — on-chain-derived fields rejected, `POST /records/:id/revoke` soft-invalidation) — each record bundles its immutable on-chain proof (tx hash, block number, issuer clone, explorer link).
`vet-web` (React+Vite+`@dogtag/ui`): issue wizards (dog-tag + vaccination), records (DB-backed list + edit/expire/revoke), verify, settings.

**Deployment.** `stacks/vet/docker-compose.yml`: `caddy` (TLS) + `web` (nginx) + `api` (`vet-api`, `--features mongo`) + `mongo` (internal). Host `41873`/`41874`.

### 3.2 Groomer — verifier (the same binary, a separate deployable)

**On-chain responsibilities.**
The groomer is a **verifier only**: it records `GROOMING_INTAKE` presentations on `VerificationRegistry` with the owner's signed consent (the privacy-maximal ZK path is the default for sensitive purposes — the proof is generated **on-device**, so the groomer never receives the underlying record).
It holds **no** issuer role; verifier capability is granted via the separate `VERIFY:<purpose>` whitelist namespace (architecture §4.3).

**Centralized DB.**
Its own Mongo (`groomerdata`): verification sessions/records, operator sessions, its own custody seed (the relayer wallet that pays gas for the on-chain `recordVerification`), appointment replicas.

**API + web surface.**
Runs the **same `vet-api` binary** with `BUSINESS_TYPE=groomer` + groomer env/port — this is a deliberate reuse (the business-backend surface is identical), but it is still a **separate deployable** (own compose, own DB, own keys, own domain).
`groomer-web`: the groomer portal SPA.

**Deployment.** `stacks/groomer/docker-compose.yml`: `caddy` + `web` + `api` (`vet-api`, `BUSINESS_TYPE=groomer`, `PORT=43618`) + `mongo`. Host `43617`/`43618`.

### 3.3 Government — credential authority (NET-NEW, `accredited_authority`)

The government stack is a **net-new, genuinely separate deployable** (`stacks/government`, crate `government-api`) — **not** a re-run of `vet-api`.
It realizes the architecture's **future-government** notes (§3.6 record-type table: `TRAVEL_CLEARANCE` = "EU competent authority (future)", `EU_HEALTH_CERT` = "USDA APHIS (future)"; §12 roadmap "Government/airline issuer stacks").

**On-chain responsibilities.**

- **Issue** — build an authority-endorsed credential (`TRAVEL_CLEARANCE` is a **CDC-modeled travel receipt** — a nested `credentialSubject` grouping **Section A** importer/consignee person PII (the obfuscatable/private block), **Section B** animal, **Section C** travel, a `validity` window, and a public `receiptId` leaf; `EU_HEALTH_CERT` is an Annex-IV-style health certificate), compute its salted Poseidon root `R` via the shared SDK, and anchor it with `DogTagIssuer.issue(R)` on the record-type's clone.
  A 12-char Crockford-base32 `receiptId` (CSPRNG, ~60 bits) is minted per issue, committed into `R` as a public salted leaf, and kept as the off-chain `/r/:receiptId` lookup handle; the issuance **date** is derived from the on-chain `issuedAt[R]` timestamp, never a leaf.
  The issue endpoint is **gated by the operator bearer** (`GOV_API_TOKEN`) — an authority portal anyone could issue from would undermine the receipt's credibility.
  The government signer must hold that record type's issuance whitelist (`IssuerRegistry.whitelistFor`, granted by the protocol admin) and be funded with PLASMA — exactly the vet issuer model, one trust tier up.
- **Verify** — perform government-grade verification of any DogTag credential: recompute **integrity** offline (salted-leaf root), read **on-chain status** (`DogTagIssuer.isValid(R)`), and read **issuer identity** (`IssuerRegistry.isWhitelistedFor(keccak(recordType), signer)`).
  All three are the authenticity pillars of architecture §5; all chain reads are **gasless**.
- **Govern** — intentionally **out of scope** for the government app: protocol governance (whitelisting, role grants, timelock) remains centralized in `stacks/admin` (§6). The government app is a credential authority, not the protocol admin.

**Centralized DB (what it stores off-chain and why).**
Its own Mongo (`governmentdata`), two collections:

- `credentials` — every issued government credential, keyed by its anchored root `R`: the full wrapped document (salted cleartext leaves the authority is custodian of — for `TRAVEL_CLEARANCE` the CDC-sectioned subject A/B/C + validity window; for `EU_HEALTH_CERT` the Annex-IV health leaves), the target `DogTagIssuer` clone, and its **immutable on-chain proof** (anchoring tx hash, block number, ready-to-click explorer link), plus off-chain operator metadata (`label`/`notes`) and a status (`issued`/`revoked`/`expired` — soft-invalidation only, never hard-deleted; a revoke adds a revoke-tx proof alongside the issuance proof).
  It also denormalizes the public `receiptId` (with a **unique+sparse** Mongo index backing the `/r/:receiptId` lookup), a cleartext `subject` projection, and `validUntil` — all three mirror content committed in `R`, so they are immutable (in `IMMUTABLE_KEYS`, rejected by PATCH). A read-time `effectiveStatus` (`VALID`/`EXPIRED`/`REVOKED`/`DRAFT`) is derived from the stored lifecycle + `validUntil` on every record surface.
  On-chain holds only `R`; the operational + PII payload stays here.
- `verifications` — an audit log of every verification the authority performed (root, issuer, per-pillar fragment states, folded verdict, timestamp) — the evidentiary trail a border/authority check needs.

**API + web surface.**

`government-api` (Axum) — routes:

| Route | Role | What |
|---|---|---|
| `GET /health` | liveness | status + chainId + demo/live + signer + configured issuer clones |
| `POST /v1/travel-clearance/issue` | **issuer** 🔒 | build `TRAVEL_CLEARANCE`/`EU_HEALTH_CERT` VC (+ mint public `receiptId`) → root `R` → anchor `DogTagIssuer.issue(R)` (unless `dry_run` / no signer) → persist |
| `POST /v1/verify` | **verifier** | integrity + `isValid` + `isWhitelistedFor` → verdict → persist audit record |
| `GET /v1/records`, `GET /v1/records/:root` | custodian 🔒 | list / fetch issued credentials (off-chain DB, incl. the on-chain proof + explorer links + derived `effectiveStatus`) |
| `PATCH /v1/records/:root` | custodian 🔒 | update **off-chain metadata only** (`label`/`notes`, `status` → `expired`); any on-chain-derived field is rejected 400 |
| `POST /v1/records/:root/revoke` | **issuer** 🔒 | on-chain `DogTagIssuer.revoke(R)` → soft-invalidate (row + issuance proof kept, revoke-tx proof added) |
| `GET /v1/verifications` | audit | the verification audit log |
| `GET /v1/receipts/:receiptId/status` | **public** | PII-free JSON status via a LIVE `isValid(R)` read (verdict + validity window + provenance links + `checkedAt`) |
| `GET /r/:receiptId` | **public** | server-rendered, PII-free HTML status page (status-only — no Section A/B/C content) |

🔒 = gated by `Authorization: Bearer <GOV_API_TOKEN>`. This covers issue, both record **reads** (`GET /v1/records`, `GET /v1/records/:root` — the CDC subject denormalizes Section A person PII, so an unauthenticated read would leak it) and both record **mutations** (PATCH, revoke). Health, verify, the verifications audit log, and the two **public receipt** endpoints (`GET /v1/receipts/:receiptId/status`, `GET /r/:receiptId`) stay open. Missing or wrong token → 401; in demo mode an unset `GOV_API_TOKEN` defaults to `dogtag-gov-demo-token` (the portal's `VITE_GOV_API_TOKEN` falls back to the same value); in live mode with no token configured, the gated routes fail closed with 503.

`government-web` (React+Vite) — built on the shared **`@dogtag/ui` AppShell + Tailwind + tokens** (same stack as vet/groomer/admin; `app/Layout.tsx` + `pages/{Issue,Verify,Records,Receipt}.tsx` + `lib/api.ts`, wrapped in `ThemeProvider`+`ToastProvider` — no `WalletProvider`, since government authenticates with a bearer token, not a wallet): an **Issue** page (`pages/Issue.tsx`, record type + pet + the per-type subject leaves — for `TRAVEL_CLEARANCE` the CDC A/B/C fields grouped into a sectioned A/B/C+validity form via `RECORD_TYPE_SECTIONS`, whose keys map 1:1 onto the nested subject the backend builds → issue + anchor), a **Records** page (`pages/Records.tsx`, DB-backed list with the derived `effectiveStatus` badge + on-chain proof + explorer links, edit label/notes, mark expired, revoke), a **Verify** page (paste a wrapped doc → per-pillar verdict), and a printable CDC-modeled **Receipt** page (`pages/Receipt.tsx` at `/receipt/:root`, letterhead + status chip + Section A/B/C tables + a **QR** to the public PII-free `/r/:receiptId` status page; `@media print` strips the AppShell chrome for a clean "Save as PDF"). All operator calls carry the `VITE_GOV_API_TOKEN` bearer.
The receipt landed in PR-2 (see AGENTS.md "Government receipt UI + portal shell (PR-2)"); §7 lists the remaining parity gaps.

**Deployment.** `stacks/government/docker-compose.yml`: `caddy` + `web` (nginx) + `api` (`government-api`, `--features mongo`) + `mongo` (internal). Host `44831`/`44832`.
`make up-government` brings it up.

**Chain-client selection (demo vs live).**
`government-api` picks its `ChainClient` by mode:

- **Live / production** (`GOV_DEMO_MODE` unset) → `AlloyChain` against ROAX RPC. Reads (verify) always work; issuance additionally needs `GOV_SIGNER_KEY` (a malformed key fails closed). Legacy gas pricing (read `eth_gasPrice`, send a legacy tx) mirrors `vet-api`'s ROAX quirk.
- **Demo / local / CI** (`GOV_DEMO_MODE=1`) → `MemChain` + `MemStore`: the full issue→verify flow runs with no node, no gas, no Mongo. The demo signer is pre-whitelisted so the issuer-identity pillar is demoable too.

This is the same two-mode split the rest of the system uses (`VITE_DEMO_MODE` set = demo, unset = production).

---

## 4. What this PR builds

1. This design doc.
2. The **net-new government stack** to a runnable skeleton:
   - crate `stacks/government/api` (`government-api`) — `chain.rs` (`ChainClient` + `AlloyChain` + `MemChain`), `store.rs` (`Store` + `MemStore` + `MongoStore`), `app.rs` (config + government VC build/wrap via the shared SDK), `routes.rs` (the routes above), `main.rs` (mode selection + fail-closed secrets/Mongo).
   - `stacks/government/web` — lean React+Vite SPA (Issue + Verify).
   - `Dockerfile` (api + web), `docker-compose.yml`, `.env.example` — mirroring vet/groomer.
   - Registered in the Cargo workspace + pnpm workspace + Makefile (`up-government`) + README ports/components.
   - Tests: unit (build/wrap produces a root, keccak record-type key, calldata selector, MemChain issue→valid) **and** an HTTP end-to-end (`tests/flow_memchain.rs`): `POST /issue` → `POST /verify` → verdict `true` with all three pillars, plus records/audit surfaces; and a negative (unanchored root → on-chain `false` → verdict `false`).

Runnable skeleton acceptance: `cargo test -p government-api` is green (default + `mongo` feature compile), and `POST /v1/verify` performs a **real gasless ROAX read** (`DogTagIssuer.isValid`) in live mode — the government role is demoable end-to-end.

---

## 5. vet + groomer remain separately deployable

Confirmed unchanged and independent:

- `stacks/vet/docker-compose.yml` — network `dogtag-vet`, volumes `vetdata`/`vetseed`, ports `41873`/`41874`, image `dogtag-vet-api`.
- `stacks/groomer/docker-compose.yml` — network `dogtag-groomer`, volumes `groomerdata`/`groomerseed`, ports `43617`/`43618`, image `dogtag-groomer-api` (the `vet-api` binary with `BUSINESS_TYPE=groomer`).

The two share **no** compose project, network, volume, or published port; each has its own Mongo and its own signer.
Adding the government stack touched none of their files (only the shared workspace manifests + README + Makefile), so both remain byte-for-byte deployable exactly as before.

---

## 6. Where governance lives

"Govern" is deliberately **not** a government-app capability.
Protocol governance — issuer/verifier whitelisting (`IssuerRegistry.whitelistFor` / `delistFor`), role grants on `DogTagSBT`, the `DEFAULT_ADMIN_ROLE` two-step timelock, and GDPR erasure — stays centralized in **`stacks/admin`** (the protocol registry we host).
The government app is a **credential authority** (a high-trust issuer/verifier), not the protocol operator; conflating the two would put chain-wide admin power in a per-authority deployable.
When a real competent authority onboards, the protocol admin whitelists its signer for `TRAVEL_CLEARANCE`/`EU_HEALTH_CERT` through the **same apply→approve flow** as any vet/groomer.

---

## 7. Concrete gaps to a full three-role end-to-end showcase

Tracked so the next PRs can close them:

1. **`DogTagIssuer` clones for the government record types.** ✅ **DONE (testnet, OPS-0).**
   The per-record-type clones are deployed on ROAX (chainId 135) via `DogTagIssuerFactory.createIssuer(name, keccak256(recordType), business)` with `business == the protocol admin` (single-authority topology), and their addresses are wired into `contracts/deployments/roax.json → government_clones` and `TRAVEL_CLEARANCE_ISSUER_ADDR` / `EU_HEALTH_CERT_ISSUER_ADDR` (`TRAVEL_CLEARANCE 0x8e276BD4c57740766A7e173D05F4f02013681c6a`, `EU_HEALTH_CERT 0xe30A17396c0fb75D3e8bFc862a49677B3dd568E2`).
   Set `GOV_SIGNER_KEY` to anchor live; unset (or a zero clone address) still runs `dry_run` (build + persist, no anchor).
2. **Government signer onboarding.** ✅ **DONE (testnet, OPS-0).**
   The government signer (the protocol admin address on testnet) is whitelisted on `IssuerRegistry.whitelistFor(keccak256(recordType), signer)`, so `DogTagIssuer.issue` (`onlyWhitelisted`) accepts it. It still needs PLASMA gas to anchor live; demo mode side-steps this via `MemChain`.
3. **Custody parity.**
   The government stack loads its signer from `GOV_SIGNER_KEY` (env). The vet/groomer age-encrypted custody genesis/unlock flow is richer; a future PR can port it to `government-api` for prod-grade key handling.
4. **Web parity.** ✅ **DONE (PR-2).**
   `government-web` was migrated onto the shared `@dogtag/ui` AppShell + Tailwind + tokens (theming, toasts) at vet/groomer portal parity, and gained the printable CDC-modeled receipt view (`pages/Receipt.tsx`) with a QR to the public PII-free status page. No wallet-connect: government authenticates with the `VITE_GOV_API_TOKEN` bearer, not a wallet.
5. **Verification consent path.**
   Government verify currently checks the three authenticity pillars (integrity + on-chain status + issuer identity) as gasless reads. Recording a **consented `VerificationRegistry` presentation** for a government purpose (e.g. `TRAVEL_PRESENTATION` / `AIRLINE_CHECKIN`, owner-signed consent, ZK path) is the natural next increment — the contracts already support it.
6. **Central discovery.**
   The government stack is not yet registered in the `stacks/admin` business directory; adding it lets the mobile app discover a government authority the same way it discovers vets/groomers.
7. **A three-role smoke script.** ✅ **DONE** (§8) — `scripts/e2e-roles.sh` drives the cross-role chain; `scripts/demo-up.sh` now boots government as a 4th separate stack; the `government-api` `cross_role` test codifies "vet ISSUES → government VERIFIES" deterministically.

---

## 8. Three-role showcase — how to run it

The three roles boot as **separate running stacks** and one credential flows across them.

### 8.1 Hermetic (zero deps — no node, no gas, no Mongo)

```bash
scripts/e2e-roles.sh            # boots government-api in GOV_DEMO_MODE and runs ISSUE→VERIFY→audit
cargo test -p government-api    # incl. cross_role: government VERIFIES a vet-issued VACCINATION credential
```

`tests/cross_role.rs` is the codified cross-role guarantee: a credential built through the shared SDK exactly as the **vet** stack builds it (record type `VACCINATION`, anchored on the vet clone — MemChain stands in for the shared chain) is verified, unchanged, by the **government** verifier (integrity + on-chain status + issuer identity), and a tampered copy is rejected.

### 8.2 Live cross-stack (the full showcase, over ROAX)

```bash
scripts/demo-up.sh              # boots admin :39742 · vet :41874 · groomer :43618 · government :44832 (+ portals)
scripts/e2e-roles.sh --live     # vet ISSUES a VACCINATION → government VERIFIES it (gasless ROAX read)
                                #                          → government ISSUES a TRAVEL_CLEARANCE
```

`--live` needs `contracts/.env` (a funded DEPLOYER key) for the vet issue + `cast`/`jq`/`python3`.
The **groomer** verify (an owner-consent `VerificationRegistry` presentation) is wallet/phone-driven and is exercised by `scripts/e2e-smoke.sh` step 6 (§7.5 tracks folding it into `e2e-roles.sh` once a headless consent signer is wired).
