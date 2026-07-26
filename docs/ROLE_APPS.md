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

- **Contracts** (ROAX) — `DogTagSBTConsent`, `IssuerRegistry`, `DogTagIssuer` clones + factory, `VerificationRegistryConsent`, `Groth16VerifierConsent`, `ProtocolRegistry`. Addresses in `contracts/deployments/roax.json`.
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
| On-chain **issue** | `DOG_PROFILE` (custodial SBT mint), `VACCINATION`, `SERVICE_ATTESTATION` | — (verifier only) | **`TRAVEL_CLEARANCE`, `EU_HEALTH_CERT`** |
| On-chain **verify** | `VET_INTAKE` presentations | `GROOMING_INTAKE` presentations (the canonical verifier) | **government-grade credential verification** (integrity + status + issuer identity) |
| On-chain **govern** | — | — | — (governance stays with `stacks/admin` — see §6) |
| Trust tier issued | `licensed_vet` | n/a | **`accredited_authority`** (authority-endorsement) |

The vet and groomer stacks are **already real and already separately deployable** (see §5); the government stack is **net-new** and is what this PR begins building (§4).

> **Not a role stack: the holder.** This doc covers the **issuer/verifier** roles. The **pet-owner (holder)** side (the counterpart that receives, holds, *presents*, and *selectively discloses* these credentials) is a separate component with no backend and no database: the native `apps/android`/`apps/ios` wallets and their web mirror `stacks/owner/web` (`@dogtag/owner-web`, port **45931**). See [`../stacks/owner/web/README.md`](../stacks/owner/web/README.md).

---

## 3. Per-role design

### 3.1 Vet — issuer + verifier (licensed_vet)

**On-chain responsibilities.**
The vet is the primary **issuer**: it issues the pet-identity tag owner-hidden - `issue(R)` on the `DOG_PROFILE` clone, then `DogTagSBTConsent.mintCustodial(dogTagId, R)` (no recipient address; requires `ISSUER_ROLE`) - and anchors `VACCINATION` / `SERVICE_ATTESTATION` roots on its `DogTagIssuer` clone (gated by `IssuerRegistry.isWhitelistedFor(recordType, signer)`).
It is also a **verifier** for `VET_INTAKE` presentations (`VerificationRegistryConsent`, the owner-hidden consent-proof path).

**Centralized DB (what it stores off-chain and why).**
The vet backend is the legal **record-custodian**: it holds full credential records (salted cleartext leaves, per-record DEKs), the age-encrypted custody seed (its signer), operator/admin sessions, appointment replicas, and calendar-sync state.
On-chain we anchor **only** the salted root `R` — all PII stays in the vet's Mongo, erasable per the DPIA.

**API + web surface.**
`vet-api` (Axum): issue → prepare/confirm, share, third-party verify, export-session (owner→verifier ZK consent), dog-tag issuance (`/profiles/issue/*` — the owner-hidden `custodial-bind` session flow, the sole bind path), calendar sync, custody genesis/unlock; records management (`GET /records` operator-gated list, `PATCH /records/:id` off-chain metadata only — on-chain-derived fields rejected, `POST /records/:id/revoke` soft-invalidation) — each record bundles its immutable on-chain proof (tx hash, block number, issuer clone, explorer link).
`vet-web` (React+Vite+`@dogtag/ui`): issue wizards (dog-tag + vaccination), records (DB-backed list + edit/expire/revoke), verify, settings.

**Deployment.** `stacks/vet/docker-compose.yml`: `caddy` (TLS) + `web` (nginx) + `api` (`vet-api`, `--features mongo`) + `mongo` (internal). Host `41873`/`41874`.

### 3.2 Groomer — verifier (the same binary, a separate deployable)

**On-chain responsibilities.**
The groomer is a **verifier only**: it records `GROOMING_INTAKE` presentations on `VerificationRegistryConsent` with the owner's ZK consent proof.
The proof is generated **on-device**, so the groomer never receives the underlying record or the owner's identity - there is no other verify mode.
It holds **no** issuer role; verifier capability is granted via the separate `VERIFY:<purpose>` whitelist namespace (architecture §4.3).

**Centralized DB.**
Its own Mongo (`groomerdata`): verification sessions/records, operator sessions, its own custody seed (the relayer wallet that pays gas for the on-chain `recordVerificationZK`), appointment replicas.

**API + web surface.**
Runs the **same `vet-api` binary** with `BUSINESS_TYPE=groomer` + groomer env/port — a deliberate reuse, and still a **separate deployable** (own compose, own DB, own keys, own domain).
`BUSINESS_TYPE` is the ROLE, and the role decides which surfaces exist: a groomer verifies and does not issue, so `public_router` does **not** mount the issuance routes (`/credentials/*`, `/records/*`, `/r/{token}`, `/profiles/issue/*`, `/p/{token}`) — they do not exist rather than existing-and-refusing. It fails open, so anything that is not the literal `groomer` keeps the full issuing surface. Everything else (genesis/custody, import/pull, `/verify/*`, `/trace/*`, settings, and the shop CRM `/clients` · `/appointments` · `/verifications`) is mounted for every role.
`groomer-web`: the groomer portal SPA — the shop's working application (calendar, appointments, clients, and the verification history, with verification started FROM an appointment); see [`stacks/groomer/web/README.md`](../stacks/groomer/web/README.md).

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
| `GET /health` | liveness | status + the honest chain-backend report (`backend`/`simulated`/`chainId`/`canSign`/`signer`/`simulatedSigner`, see "Chain-client selection" below) + `demo` (store only) + configured issuer clones |
| `POST /v1/travel-clearance/issue` | **issuer** 🔒 | build `TRAVEL_CLEARANCE`/`EU_HEALTH_CERT` VC (+ mint public `receiptId`) → root `R` → anchor `DogTagIssuer.issue(R)` (unless `dry_run` / no signer) → persist |
| `POST /v1/verify` | **verifier** | integrity + `isValid` + `isWhitelistedFor` → verdict → persist audit record |
| `GET /v1/records`, `GET /v1/records/:root` | custodian 🔒 | list / fetch issued credentials (off-chain DB, incl. the on-chain proof + explorer links + derived `effectiveStatus`) |
| `PATCH /v1/records/:root` | custodian 🔒 | update **off-chain metadata only** (`label`/`notes`, `status` → `expired`); any on-chain-derived field is rejected 400 |
| `POST /v1/records/:root/revoke` | **issuer** 🔒 | on-chain `DogTagIssuer.revoke(R)` → soft-invalidate (row + issuance proof kept, revoke-tx proof added) |
| `GET /v1/verifications` | audit | the verification audit log |
| `GET /v1/receipts/:receiptId/status` | **public** | PII-free JSON status via a LIVE `isValid(R)` read (verdict + validity window + provenance + `checkedAt`), plus `chainId` and an explicit `simulated` flag — see "Chain-client selection" below |
| `GET /r/:receiptId` | **public** | server-rendered, PII-free HTML status page (status-only — no Section A/B/C content); its provenance row states outright when the backend is simulated |

🔒 = gated by `Authorization: Bearer <GOV_API_TOKEN>`. This covers issue, both record **reads** (`GET /v1/records`, `GET /v1/records/:root` — the CDC subject denormalizes Section A person PII, so an unauthenticated read would leak it) and both record **mutations** (PATCH, revoke). Health, verify, the verifications audit log, and the two **public receipt** endpoints (`GET /v1/receipts/:receiptId/status`, `GET /r/:receiptId`) stay open. Missing or wrong token → 401; in demo mode an unset `GOV_API_TOKEN` defaults to `dogtag-gov-demo-token` (the portal's `VITE_GOV_API_TOKEN` falls back to the same value); in live mode with no token configured, the gated routes fail closed with 503.

`government-web` (React+Vite) — built on the shared **`@dogtag/ui` AppShell + Tailwind + tokens** (same stack as vet/groomer/admin; `app/Layout.tsx` + `pages/{Issue,Verify,Records,Receipt}.tsx` + `lib/api.ts`, wrapped in `ThemeProvider`+`ToastProvider` — no `WalletProvider`, since government authenticates with a bearer token, not a wallet): an **Issue** page (`pages/Issue.tsx`, record type + pet + the per-type subject leaves — for `TRAVEL_CLEARANCE` the CDC A/B/C fields grouped into a sectioned A/B/C+validity form via `RECORD_TYPE_SECTIONS`, whose keys map 1:1 onto the nested subject the backend builds → issue + anchor), a **Records** page (`pages/Records.tsx`, DB-backed list with the derived `effectiveStatus` badge + on-chain proof + explorer links, edit label/notes, mark expired, revoke), a **Verify** page (paste a wrapped doc → per-pillar verdict), and a printable CDC-modeled **Receipt** page (`pages/Receipt.tsx` at `/receipt/:root`, letterhead + status chip + Section A/B/C tables + a **QR** to the public PII-free `/r/:receiptId` status page; `@media print` strips the AppShell chrome for a clean "Save as PDF"). All operator calls carry the `VITE_GOV_API_TOKEN` bearer.
The receipt landed in PR-2 (see AGENTS.md "Government receipt UI + portal shell (PR-2)"); §7 lists the remaining parity gaps.

**Deployment.** `stacks/government/docker-compose.yml`: `caddy` + `web` (nginx) + `api` (`government-api`, `--features mongo`) + `mongo` (internal). Host `44831`/`44832`.
`make up-government` brings it up.

**Chain-client selection.**
`government-api` picks its `ChainClient` from `GOV_CHAIN_BACKEND` alone - deliberately NOT from the demo flag:

- **`live` (the default)** → `AlloyChain` against ROAX RPC. Reads (verify) always work; issuance additionally needs `GOV_SIGNER_KEY` (a malformed key fails closed). Legacy gas pricing (read `eth_gasPrice`, send a legacy tx) mirrors `vet-api`'s ROAX quirk.
- **`mem` (explicit opt-in)** → `MemChain`: the full issue/verify flow runs with no node and no gas. The stand-in signer is pre-whitelisted so the issuer-identity pillar is still demoable. Nothing is broadcast and nothing survives the process.

The STORE is a separate axis: `GOV_DEMO_MODE`/`DEMO_MODE` selects the ephemeral `MemStore` (plus the well-known demo API token), and `MONGO_URI` selects `MongoStore`.

Keeping these two axes apart matters. When one `demo` flag drove both, a demo stack that only wanted an ephemeral *store* silently got a simulated *chain* too - and because `/health` echoed the configured `CHAIN_ID`, it reported `chainId:135` with `canSign:true` while running entirely on `MemChain`. Its verify and records surfaces were simulated but indistinguishable from live.

**`/health` states the backend truthfully.** `backend` is `"live"` or `"simulated"`; `simulated` is the same fact as a boolean; `chainId` carries the real id when live and is **`null`** when simulated (never a network the process is not on); `canSign` is true only when a real key would put a real tx on a real chain; a stand-in signer is reported as `simulatedSigner`, never as `signer`. The portal's sidebar and topbar badge read from these, so "SIMULATED CHAIN" vs "LIVE CHAIN" is visible on every page — with a third **unknown** state whenever `/health` has not answered (first paint, a failed poll) or answered without these fields, because collapsing "we don't know" into "live" is the same over-claim in a different place.

**The two public receipt surfaces carry that contract too** — they are what an outside party checks, so they must not assert a chain the process is not on.
`GET /v1/receipts/:receiptId/status` returns `chainId` (`null` when simulated) alongside an explicit `simulated` flag, and `GET /r/:receiptId` swaps its "Anchored on ROAX chainId …" row for a plain statement that the receipt came from a demonstration stack, was never broadcast, and carries no legal effect.
On a simulated backend both surfaces also **suppress the stored `explorer.roax.net` links**, which would otherwise point at transactions that never existed.
Read `simulated`/`chainId` to tell the backends apart, **not** the absence of a link: a live-but-unanchored credential (`dry_run`, no signer) has no links either.
The record's own row and the operator-facing `/v1/records*` surfaces keep their links unchanged; the suppression is on these two public surfaces only.

To provision the government for real on-chain issuance (funded signer + `TRAVEL_CLEARANCE` whitelist + a `DogTagIssuer` clone), run `scripts/demo-provision-government.sh`; it is idempotent and never prints the key.

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
Protocol governance — issuer/verifier whitelisting (`IssuerRegistry.whitelistFor` / `delistFor`), role grants on `DogTagSBTConsent`, the `DEFAULT_ADMIN_ROLE` two-step timelock, and GDPR erasure — stays centralized in **`stacks/admin`** (the protocol registry we host).
The government app is a **credential authority** (a high-trust issuer/verifier), not the protocol operator; conflating the two would put chain-wide admin power in a per-authority deployable.
When a real competent authority onboards, the protocol admin whitelists its signer for `TRAVEL_CLEARANCE`/`EU_HEALTH_CERT` through the **same apply→approve flow** as any vet/groomer.

---

## 7. Concrete gaps to a full three-role end-to-end showcase

Tracked so the next PRs can close them:

1. **`DogTagIssuer` clones for the government record types.** ✅ **DONE (testnet).**
   `TRAVEL_CLEARANCE` is live on the FRESH owner-hidden set at **`0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F`** (`contracts/deployments/roax.json → government_clones`), deployed via `DogTagIssuerFactory.createIssuer(name, keccak256(recordType), business)` on factory `0xED20269E` with `business ==` the governance signer `0x8E27E117…` (the factory's `Ownable` owner, so `createIssuer` is authorised).
   **The clone must be bound to the registry the rest of the stack reads.** The earlier `government_clones` entries (`TRAVEL_CLEARANCE 0x8e276BD4…`, `EU_HEALTH_CERT 0xe30A1739…`) are bound to the RETIRED `IssuerRegistry 0x5d86e4CF…` and are not clones of the fresh factory, so `onlyWhitelisted` reads a registry nobody uses and issuance fails closed even after a correct `whitelistFor`. They are retained only as `government_clones_deadRegistry_legacy`. `demo-up.sh` preflights BOTH sides of this to catch exactly the class: `factory.registry() == ISSUER_REGISTRY_ADDR`, and - whenever `TRAVEL_CLEARANCE_ISSUER_ADDR` is set - the configured clone's own `registry()` plus its `recordType() == keccak256("TRAVEL_CLEARANCE")` (the same pair `scripts/demo-provision-government.sh` asserts after deploying one). **A stale clone HARD-FAILS the whole boot**, deliberately: it otherwise passes every other preflight line and surfaces only as a 502 on the first issuance.
   `EU_HEALTH_CERT` is not deployed on the fresh set; leave `EU_HEALTH_CERT_ISSUER_ADDR` unset so `/issue` reports the issuer as `null` and dry-runs rather than anchoring somewhere wrong.
2. **Government signer onboarding.** ✅ **DONE (testnet).**
   `scripts/demo-provision-government.sh` does all of it, idempotently and without printing the key: it generates a DEDICATED government EOA (not the governance key - the demo is meant to show the government as its own authority that governance granted rights to), funds it, `whitelistFor(keccak256(TRAVEL_CLEARANCE), gov)` on the live registry, and deploys the clone. `DogTagIssuer.issue` is `onlyWhitelisted`, so all three are required; the script verifies each on-chain afterwards.
   Anchoring costs real PLASMA gas. Set `GOV_CHAIN_BACKEND=mem` if you deliberately want to side-step gas - never as a default, and `/health` will say `backend:"simulated"`.
3. **Custody parity.**
   The government stack loads its signer from `GOV_SIGNER_KEY` (env). The vet/groomer age-encrypted custody genesis/unlock flow is richer; a future PR can port it to `government-api` for prod-grade key handling.
4. **Web parity.** ✅ **DONE (PR-2).**
   `government-web` was migrated onto the shared `@dogtag/ui` AppShell + Tailwind + tokens (theming, toasts) at vet/groomer portal parity, and gained the printable CDC-modeled receipt view (`pages/Receipt.tsx`) with a QR to the public PII-free status page. No wallet-connect: government authenticates with the `VITE_GOV_API_TOKEN` bearer, not a wallet.
5. **Verification consent path.**
   Government verify currently checks the three authenticity pillars (integrity + on-chain status + issuer identity) as gasless reads. Recording a **consented owner-hidden `VerificationRegistryConsent` presentation** for a government purpose (e.g. `TRAVEL_PRESENTATION` / `AIRLINE_CHECKIN`) is the natural next increment — the contracts already support it.
6. **Central discovery.**
   The government stack is not yet registered in the `stacks/admin` business directory; adding it lets the mobile app discover a government authority the same way it discovers vets/groomers.
7. **A three-role smoke script.** ✅ **DONE** (§8) — `scripts/e2e-roles.sh` drives the cross-role chain; `scripts/demo-up.sh` now boots government as a 4th separate stack; the `government-api` `cross_role` test codifies "vet ISSUES → government VERIFIES" deterministically.

---

## 8. Three-role showcase — how to run it

The three roles boot as **separate running stacks** and one credential flows across them.

### 8.1 Hermetic (zero deps — no node, no gas, no Mongo)

```bash
scripts/e2e-roles.sh            # boots government-api on GOV_CHAIN_BACKEND=mem, runs ISSUE→VERIFY→audit
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
The **groomer** verify (an owner-hidden `VerificationRegistryConsent` consent proof) is phone-driven in the demo; `scripts/e2e-zk.sh` exercises it headlessly with a real `consent.circom` proof (§7.5 tracks folding a government purpose into the same consent path).
