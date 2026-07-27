# Project agent memory

This file is the project's committed home for project-intrinsic agent knowledge: build, test, release, architecture, and sharp-edge notes that should travel with the code.

## No-mistakes Document safety (high priority, conditional)

Start every new Dogtag run with `no-mistakes axi run --skip=document --intent "<intent>"` until upstream provides an enforced step/file budget and this policy is deliberately revised. Do not use bare `axi run` or `no-mistakes rerun`, which cannot preserve this skip. The config instructions and post-stage commit guard are prompt/supervision defense-in-depth for accidental raw runs, not a runtime cap: update only documentation made stale directly by the submitted branch; never run write-mode formatters, generators, codegen, or UniFFI/binding synchronization; and never edit functional source, tests, workflows, circuits, contracts, or generated bindings. If the work would require more than 10 files, any non-documentation file, or cross-slice reconciliation, make no such edits and return an ask-user finding.

## No-mistakes Test safety (high priority, conditional)

When acting as the no-mistakes Test or evidence agent, use the configured targeted command plus at most the smallest checks directly relevant to the submitted diff. Never run `cargo test --workspace` or another full monorepo suite locally, and do not expand into browsers or screenshots unless the diff changes that UI. Treat 15 minutes as a prompt/supervision budget, not a hard enforced timeout; park with a finding instead of broadening beyond it.

## Product model (non-negotiable)

**dogtag is ONE owner-hidden model. There is no Level-A/Level-B split, mode, or vocabulary in the product.**

The whole model is exactly two primitives:

1. **MERKLE PROOF = CREDENTIAL ISSUANCE.** The credential is a Merkle tree/root; the owner is a hidden leaf.
2. **ZK PROOF = THE OWNER'S CONSENT FOR THAT MERKLE PROOF.** Consent is proven in zero knowledge; the owner is never revealed.

That is it. Vets issue credentials; owners hold credentials and give consent. Vets and owners must never be shown or asked to choose or understand "Level-A" versus "Level-B", a mode, an opt-in, or a toggle.

All live and forward app/server/web code, config, API design, and docs must implement this single owner-hidden model. Do **not** build or preserve a Level-A path, a dual-mode gate, an "available-not-default" opt-in, or A/B coexistence. The owner-revealing code path has been fully retired from the repo (contracts PR #69, backend PR #72, mobile PR #71, SDK/FFI/docs in the final cleanup slice); the retired-generation contracts remain on the disposable testnet as deployment history only, superseded in place by the fresh owner-hidden set deployed 2026-07-23 (r8), and the old level names survive ONLY as internal version-key strings (`dogtag-levelb/1`) and internal identifiers (`public_signals::level_b`), never as product vocabulary. The old level names are internal migration history, never product vocabulary or a user choice; later migration-era statements may accurately describe today's transition state, but every one that calls Level-A live/default or Level-B optional/additive is superseded by this rule for new work and can neither guide nor justify new A/B product design.

## Build & test (what actually runs offline)

Toolchain: Rust (cargo workspace), Foundry (`forge`/`cast`), Node 22 + pnpm 10, circom 2.1.9 + snarkjs 0.7.6, Docker.

- `cargo check --workspace` / `cargo build` — Rust workspace: `dogtag-standard-rs`, `dogtag-prover-rs`, `vet-api`, `admin-api`, `government-api`, `indexer-api`.
- `cargo test -p indexer-api` — the oversight indexer (scope + store unit tests + `tests/query_api.rs` end-to-end over MemLogSource + MemStore). Hermetic, fast (no node/Mongo). See the "Oversight indexer (PR-4)" section.
- `cargo test -p dogtag-standard-rs` — trust-core crypto + cross-language parity vectors.
- `make test-consent-parity` (wrapper: `scripts/test-consent-parity.sh`) - the LOUD entry point for the
  repo's ONLY empirical proof that the on-device consent prover agrees with the frozen consent VK. Use
  it instead of the bare cargo invocation, because that invocation can report green in TWO ways without
  running the check, neither of them visible: the test is `#![cfg(feature = "prover")]`, so a plain
  `cargo test -p dogtag-standard-rs` compiles it away and prints `running 0 tests`; and even with the
  feature it used to self-skip when `circuits/build/consent.graph` was absent. **`consent.graph` is now
  COMMITTED** (alongside `consent_final.zkey`), so that skip is gone: an absent artifact means an
  incomplete checkout and the test panics. The wrapper still closes the feature-flag hole: it always
  passes `--features prover`, and it checks the artifacts from the SHELL - where a `::error::` line is
  actually parsed and a non-zero exit is a real failure - naming the missing artifact. An annotation
  printed from inside the test could not work: libtest captures stdout for PASSING tests.
  It is deliberately **not** in `make test` (like `test-consent`) because it is slow (real Groth16),
  no longer because a normal checkout lacks artifacts. `DOGTAG_REQUIRE_ZK_ARTIFACTS` is retired - it
  existed only to turn the skip into a panic, and the skip no longer exists. Note no GitHub workflow
  runs `cargo test` today, so this gate is operator-invoked; a captain-gated Rust CI job is a separate
  follow-up.
- `cargo test -p vet-api -p admin-api` — backends. (One vet-api suite, `gate_dual_signing_parity`, is slow — ~5 min — it runs the real prover/signing; this is expected, not a hang.)
- `cd contracts && forge test` - 83 tests over the owner-hidden contract set. `CustodialIssuance.t.sol`
  and `ConsentRegistry.t.sol` verify real owner-hidden issuance/proofs; `DeployProtocolRegistry.t.sol`
  exercises the real env-driven deploy→propose→execute path for the single `dogtag-levelb/1`
  protocol version (an internal version key, not a product label) on both registry axes; `OwnerHiddenSurface.t.sol` rejects a recipient-bearing `mint` or a
  subject-bearing `Verified` ABI. Use `forge test`, **not** bare `forge build`: a bare full build tries
  to compile the OZ submodule's `certora/harnesses/*` which import generated `../patched/*` files that
  aren't present, so it fails with "File not found" - a vendored-submodule artifact, NOT a project
  error. `forge test` only compiles the real dependency closure and is green.
- `cd circuits && pnpm test-consent` — generates real `DogTagConsent` Groth16 proofs across multiple
  tree sizes, asserts the frozen seven-signal order and SDK root parity, and runs the negative tests.
  Needs the TS SDK built first (`pnpm --filter @dogtag/standard build`) and `pnpm install`.
- `make parity` — the Poseidon anchor gate; `make test` — parity + TS + Rust + contracts.
- `cd apps/android && gradle test` - the JVM unit suites (`RoaxRpcSelectorTest`, `QrPayloadTest`,
  `PublicSignalIndexTest`, `ZkeyAssetTest`, `ProfileTreeParityTest`, `OwnerSecretRecordsTest`,
  `OwnerSecretCodecTest`, `OwnerSecretRecoveryJourneyTest`).
  Needs `apps/android/local.properties` with `sdk.dir=…` (gitignored; the CI job writes it).
  **`ProfileTreeParityTest` calls the REAL Rust core from the host JVM.**
  That needs two things the rest of the module does not: the desktop `net.java.dev.jna:jna` jar, since the `@aar` variant ships `libjnidispatch` for Android ABIs only, and a HOST build of `dogtag-standard-rs`, since `jniLibs/`'s `.so` files are Android-ABI-only and gitignored and so can never load on a dev machine.
  `app/build.gradle.kts` handles both: every `Test` task `dependsOn` a `cargo build … --lib` and gets `jna.library.path` + `dogtag.repoRoot` system properties.
  Use `dogtag.repoRoot` to reach repo fixtures - the unit-test working directory is the MODULE dir `apps/android/app`, so tests must not hard-code `../..`.
  That cargo build MUST pass `--features prover`: the checked-in Kotlin bindings were generated from the prover-enabled crate and UniFFI checksum-verifies EVERY exported function at library load, so a default-feature build dies with `UnsatisfiedLinkError: …checksum_func_prove_consent` before any test body runs.
  The dependency is deliberately hard rather than a soft skip - a self-skipping parity test reports green in exactly the case it exists to catch.
  Note `org.json` is Android's "not mocked" STUB on the unit-test classpath: pure-JVM tests must parse fixtures without it, and anything exercising the real codec needs Robolectric (`OwnerSecretCodecTest`, `OwnerSecretRecoveryJourneyTest`), which costs the network on a cold cache.
  `OwnerSecretRecoveryJourneyTest` is the one suite that needs BOTH rungs at once - Robolectric for the real `org.json` codec and the host Rust core over JNA - because it joins them: it asserts the phrase plus the backup file rebuild the SAME `R` on a replacement device, and that the backup file ALONE does not.
  **Exception to this section's "runs offline" framing:**
  `QrPayloadTest` uses Robolectric, which resolves its `android-all-instrumented` runtime jars from
  Maven on the FIRST run, so a cold Gradle cache needs network (warm cache is offline). The tradeoff
  was taken deliberately: `QrPayload.parse` is built on the real `android.net.Uri` and QR content is
  fully attacker-controlled, so a hand-rolled stand-in would test the stand-in, not the parser that
  actually runs - the very trap that let a duplicate-query-key QR crash the iOS scanner.
- **Known gap: neither mobile unit suite runs in CI.** The only two workflows are the
  `workflow_dispatch`-only mobile e2e jobs, so `apps/android`'s JVM tests and the iOS `DogTagTests`
  scheme are guarded by LOCAL runs only - including the QR-trap regression they were written for. Run
  them by hand before shipping mobile changes. (Dispatch-only CI is a standing decision, not an
  oversight; broadening it is captain-gated.)

### Sharp edges learned
- **The parity gate is `circuits/scripts/gen-vectors.mjs`.** It is the source of truth: it computes the circom witness (reference-of-record) and cross-checks `poseidon-lite` (TS) and `circomlibjs`, then writes `circuits/poseidon-vectors.json` which Rust (`sdk_parity.rs`/`poseidon_parity.rs`) asserts. The parity gate is now the union of `make parity` + `test-rs` (the Solidity `PoseidonParity.t.sol` leg was retired with the owner-revealing layer; the owner-hidden `VerificationRegistryConsent` computes no on-chain Poseidon). (`circuits/scripts/check-ts.mjs` was referenced by `package.json` but never existed; it was removed — `gen-vectors.mjs` already covers TS↔circom.)
- `gen-vectors.mjs` rewrites `poseidon-vectors.json` deterministically, so running `make parity` leaves the tree clean (no spurious diff).
- `rust-analyzer` in this worktree can't find the proc-macro server and emits false `E0308`/`tokio::test` errors; trust `cargo`, not the IDE diagnostics.
- Pre-existing harmless warning: unused import `BigInteger` in `crates/dogtag-standard-rs/src/bin/field-hash.rs`.
- **Mobile `eth_call` selectors must be DERIVED from the signature, never hard-coded.** `apps/*` hand-encode selectors in `RoaxRpc.kt` / `Net.swift` (no ABI lib). `isValid`'s was once the stale literal `0x6d04f0bc` (its comment *claimed* to be the keccak but wasn't) - that selector REVERTS on the deployed ROAX `DogTagIssuer` clone, so every mobile validity read silently fell through to `Unknown`/accept-with-caveat and a revoked credential never showed as revoked. The canonical selector is `keccak256("isValid(bytes32)")[:4] = 0x6a938567` (what viem, the alloy `sol!` ABI, vet-api `verify_credential`, and the web direct-RPC path in `packages/ui` all bind). It is now derived on-device via `Keccak256` (`RoaxRpc.functionSelector` / `Net.swift` `functionSelector`); `apps/android/app/src/test/.../RoaxRpcSelectorTest.kt` pins it. **Android derives ALL of its selectors** (`isValid`, `isWhitelistedFor`, `consumed`, `profileRoot`, plus the ProtocolRegistry reads; the retired `bindNonce`/`keyOf`/`ownerOf` reads went with the owner-revealing layer) - `RoaxRpc.kt` holds no selector literals. **iOS `Net.swift` still hard-codes the non-`isValid` read selectors** (`isWhitelistedFor`, `consumed`, `profileRoot`): each was reconfirmed correct via `cast sig`, so this is latent drift risk rather than a live bug, and it stays open only because `Net.swift` is not yet covered by the iOS unit-test target (which exists now - see "iOS unit tests" - but is host-less/FFI-free, and `Net.swift` would need the selector helpers extracted into an FFI-free source before it can be pinned the way `RoaxRpcSelectorTest.kt` pins Android's). Verify any new mobile selector against the chain before shipping: `eth_call` a real clone (VACCINATION `0x1456f93f7376789c46408CC4616751eB853edD9A` on `https://devrpc.roax.net`) - the correct selector returns a 32-byte word, a wrong one returns `execution reverted`. Note mobile has only the single `isValid` bool (no `issuedAt`/`isRevoked` decomposition like web), so it renders revoked and never-anchored identically as "REVOKED / not anchored"; that is intentional, not a bug.

## Architecture quick map
- `crates/dogtag-standard-rs` — trust core: canonicalization, field/type-tag encoding, circom-compatible Poseidon (`light-poseidon`), salted Merkle, verify, EdDSA-BabyJubjub signer, BLAKE-512 (circomlibjs parity), UniFFI → mobile.
- `crates/dogtag-prover-rs` — real ark-circom/ark-groth16 prover (self-verifies). Test oracle + backend prover-service. Its artifacts are **version-keyed** (`src/artifact.rs`) — see "Version-keyed proving artifacts".
- `circuits` — the active source is Groth16 `DogTagConsent(6)`: reserved-owner-leaf Merkle membership,
  EdDSA consent, proof-bound relayer/purpose, and a hidden-owner nullifier. Its seven public signals are
  `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`. `verification.circom` and its
  fixture generators are retired; its retained build products and ceremony transcript remain
  historical provenance and are not inputs to new builds. The consent VK/zkey remain active and frozen.
- `contracts` — live source consists of the shared `IssuerRegistry`, `DogTagIssuer` implementation +
  factory/root index, `ProtocolRegistry`, and `IERC5192`, plus `Groth16VerifierConsent`,
  `DogTagSBTConsent` (write-once `profileRoot`, neutral custodial sink), and
  `VerificationRegistryConsent`. `Deploy.s.sol` deploys only the shared base;
  `DeployCustodialIssuance.s.sol` explicitly deploys the frozen-ceremony verifier before the SBT and
  registry and repoints only the canonical ledger keys. The retired owner-revealing contract sources
  and deploy scripts are gone; their already-deployed addresses remain solely in the deployment ledger
  for historical reads. Protocol publication keeps the exact compatibility key `dogtag-levelb/1` and
  publishes one contract set plus one independently rotatable artifact set and their binding.
- `stacks/vet` + `stacks/groomer` — same `vet-api` binary (`BUSINESS_TYPE` switch) + SPA + Mongo. `stacks/admin` — central registry/admin-api.
- `stacks/government` — **net-new, separately-deployable** role stack running its **own** `government-api` crate (NOT vet-api): a government credential authority that issues authority-endorsed `TRAVEL_CLEARANCE`/`EU_HEALTH_CERT` (anchors root via `DogTagIssuer.issue`) and does government-grade verify (integrity + `isValid` + `isWhitelistedFor`, all gasless reads). Own Mongo (`governmentdata`), ports 44831/44832, `make up-government`. **CHAIN and STORE are separate axes, deliberately:** `GOV_CHAIN_BACKEND` picks the chain - `live` (DEFAULT, `AlloyChain` on ROAX; `GOV_SIGNER_KEY` to anchor) or `mem` (explicit opt-in `MemChain`, used by `tests/flow_memchain.rs` and `e2e-roles.sh`) - while `GOV_DEMO_MODE=1` only picks the ephemeral `MemStore` + demo API token. They used to be one flag, which silently ran demo stacks' verify/records on a simulated chain while `/health` still echoed `CHAIN_ID` as `chainId:135, canSign:true`. `/health` now reports `backend`/`simulated`, `chainId:null` when simulated, `canSign` only for real broadcast, and `simulatedSigner` for a stand-in; the portal badge shows LIVE vs SIMULATED CHAIN. Provision real on-chain issuance with `scripts/demo-provision-government.sh` (funded signer + `TRAVEL_CLEARANCE` whitelist + `DogTagIssuer` clone; idempotent, never prints the key). It reuses the shared `dogtag-standard-rs` SDK for credential build/wrap but has its own trimmed `chain.rs`. Design: `docs/ROLE_APPS.md`.
- **Three-role showcase**: `scripts/demo-up.sh` boots all role stacks as separate services (admin/vet/groomer/government + portals). `scripts/e2e-roles.sh` (default = hermetic government ISSUE→VERIFY on `GOV_CHAIN_BACKEND=mem`, no deps; `--live` = vet ISSUES → government VERIFIES → government ISSUES across the running stacks over ROAX, needs `contracts/.env`). `government-api tests/cross_role.rs` codifies "vet ISSUES → government VERIFIES" deterministically over MemChain. See `docs/ROLE_APPS.md` §8.
  `demo-up.sh` PREFLIGHTS before booting anything and refuses to start a stack that would look healthy while doing nothing: it asserts the RPC's chainId, that the factory's `registry()` matches `ISSUER_REGISTRY_ADDR` (a stale ledger entry otherwise deploys clones no verifier reads), and that the hosted admin signer holds `WHITELIST_ADMIN`. That last one is the trap: `ADMIN_PK` falls back to `DEPLOYER_PRIVATE_KEY`, and the retired deployer EOA **lost `WHITELIST_ADMIN` in governance Phase-2**, so booting on it made every portal grant return `disposition:"proposed"` with unsigned calldata and nothing reached the chain. Override with `ADMIN_PROPOSE_ONLY=1` (or its alias `ALLOW_UNAUTHORIZED_ADMIN_SIGNER=1`) only for a genuine propose-for-external-signing setup - the script resolves either name into the one declaration it both gates on and forwards to admin-api. `FACTORY_ADDR` is resolved from env then `contracts/deployments/roax.json` (`DogTagIssuerFactory`) and passed to admin-api - without it the Issuers/Factory UI answered `FACTORY_ADDR not configured` and `governance/authority` reported `factoryOwner.target` as the zero address. admin-api repeats the authority check at boot (`ADMIN_REQUIRE_AUTHORITY=1` to refuse to boot).
  **`ADMIN_REQUIRE_AUTHORITY` is best-effort, not a hard gate.** The boot preflight is wrapped in a 5s `tokio::time::timeout` because it runs before `axum::serve` binds and a hung-but-accepting RPC would otherwise mean `/health` never comes up - and each authority read in `chain.rs` builds its own provider, so the preflight is THREE connect+call round trips and can exceed that on a slow link. On elapse the verdict is `Unknown`/UNRESOLVED: the loud wrong-key error does not print and `ADMIN_REQUIRE_AUTHORITY=1` does **not** refuse to boot. Deliberate - the diagnostic must never gate liveness - but do not rely on it as the only thing standing between you and a wrong-key deployment.
  **The propose-only DECLARATION (`ADMIN_PROPOSE_ONLY`, alias `ALLOW_UNAUTHORIZED_ADMIN_SIGNER`) is what makes a proposal legible**, because `disposition:"proposed"` is BOTH the designed out-of-band-signing flow and what a retired key produces. `demo-up.sh` forwards the same flag it gates its own boot on to admin-api, which reports the two cases apart - see the tri-state `outcome` under "Admin whitelist management console (PR-E)".
- **Government per-record-type fields**: each credential type has its OWN field set — backend `credentialSubject` is built per type in `government/api/src/app.rs::build_gov_vc` (`TRAVEL_CLEARANCE` = the CDC-sectioned nested subject: Section A importer/consignee + B animal + C travel + validity + public `receiptId` — see the "Government travel receipt" section above; `EU_HEALTH_CERT` = species/microchip/rabies/examining-vet/health-status), and the web Issue form (`government/web/src/pages/Issue.tsx`, `RECORD_TYPE_SECTIONS`) mirrors those leaves as a **sectioned** A/B/C+validity form. Keep the two in sync (a form field `key` must equal the flat input key `build_gov_vc` reads via `get(...)`; for `EU_HEALTH_CERT` that key equals the leaf name, while `TRAVEL_CLEARANCE` maps flat keys onto nested leaves, e.g. `importerLastName` → `importer.lastName`, `animalName` → `animal.name`). **e2e-locked field keys:** `government.spec.ts` asserts TRAVEL_CLEARANCE has `field-animalName` and NOT `field-microchipNumber`, and EU_HEALTH_CERT the reverse — do NOT add a `microchipNumber` input to the TRAVEL form (the backend defaults it under `animal`), or the per-type field test breaks. After a successful issue the portal shows the wrapped doc with a one-click **Copy** button to paste into Verify + a link to the printable receipt. The whitelist pillar is exercised because the Verify page pre-fills the signer from `/health`.
- **Government web e2e (Playwright)**: `stacks/government/web/e2e/government.spec.ts` (config `playwright.config.ts`) drives issue→copy→verify for both record types against a LIVE portal. It is NOT in `pnpm test`/CI (needs a running portal + browsers); run it against a served instance: `GOV_URL=<portal-url> pnpm --filter @dogtag/government-web test:e2e` (one-off `pnpm exec playwright install chromium`). It asserts all three pillars green for BOTH record types, so the serve must be able to anchor each one: on the fresh set that means a **simulated** serve (`GOV_DEMO_MODE=1 GOV_CHAIN_BACKEND=mem`, which pre-whitelists the stand-in signer for both types), because live only `TRAVEL_CLEARANCE` has a clone + a whitelisted signer (both provisioned by `scripts/demo-provision-government.sh`; `EU_HEALTH_CERT_ISSUER_ADDR` stays unset until one is deployed). Do NOT point both `*_ISSUER_ADDR` at the TRAVEL clone or reuse `DEPLOYER_PRIVATE_KEY` as `GOV_SIGNER_KEY` — the clone's `recordType()` is `keccak256(TRAVEL_CLEARANCE)`, and the whitelisted government signer is the dedicated EOA the provisioning script mints.
- `stacks/owner/web` (`@dogtag/owner-web`, port **45931**) - the **pet-owner (holder) wallet**, the consumer front. Web mirror of the native `apps/android`+`apps/ios` holder: a self-custodial wallet that **receives** an issued wrapped doc (integrity-checked offline via `@dogtag/standard checkIntegrity`, held in localStorage) and **displays** it (decoded leaves + `DogTagIssuer.isValid` read). It has **no backend** and no ZK path: the browser present/prove surface was retired with the owner-revealing layer (`src/lib/present.ts`, the `/present` route, and the `VITE_OWNER_PROVER_URL` prover hookup are gone - the e2e now asserts `/present` falls back to the wallet). ZK consent presentation is the native holder apps' on-device flow; the backend `/prove-consent` route is the server-prove fallback concept for devices that cannot prove locally. Wired into `scripts/demo-up.sh`.
  - **Sharp edge (browser Buffer)**: `@dogtag/standard`'s EdDSA path pulls in `circomlibjs`, which needs Node `Buffer`/`global` at runtime. The vite **build** tree-shakes past it but the **dev server crashes** ("Buffer is not defined") without a shim. `src/polyfills.ts` (imported first in `main.tsx`, `buffer` npm dep) provides them. Any new web app that signs consent client-side needs the same shim.
  - **Owner-web receipt renderer (`src/pages/Receipt.tsx`, `/receipt/:root`; index `/receipts`)** - govarch PR-6 holder-side receipt for `TRAVEL_CLEARANCE` and `EU_HEALTH_CERT`, derived entirely from the locally held `WrappedDoc` plus a live `DogTagIssuer.isValid(root)` read. It mirrors the government/mobile receipt anatomy: fixed-light printable sheet, Receipt ID, issuance/validity, Section A/B/C or Annex-IV rows, QR to `https://<issuer.domain>/r/<receiptId>`, root/provenance, and holder-redaction awareness (`privacy.obfuscated[]` count; redacted copies only render leaves still present). Status derivation: `isValid=false` → `REVOKED / not anchored`, else lapsed ISO `validity.validUntil`/`rabiesValidUntil` → `EXPIRED`, else `VALID`; wallet cards/detail reuse this so revoked/expired receipts are not mislabeled as merely "not anchored". No backend, no new PII, no ZK on this path.
  - **Selective disclosure / "Share a redacted copy" (`src/pages/Share.tsx` at `/share/:id`, logic in `src/lib/redact.ts`)** - the Merkle counterpart to the ZK Present flow, and the web mirror of the native apps' "Share redacted" (mobile FFI `obfuscateDocumentJson`). The holder toggles which leaves to reveal; withheld leaves run through `@dogtag/standard`'s `obfuscate` (leaf hash → `privacy.obfuscated[]`, cleartext dropped, **Merkle root R unchanged**), so the recipient still `checkIntegrity`-verifies the SAME authentic credential + can read `isValid` on-chain, seeing only revealed fields. Default = reveal-all (the holder explicitly withholds; no fragile PII classifier). `credentialSubject.dogTagId` is **locked-on** (`NON_OBFUSCATABLE_PATHS`, mirrors verify's `NON_OBFUSCATABLE` - withholding it would fail integrity), and `recordType` is **locked as public** (`PUBLIC_PATHS` - its value is also carried in the always-revealed `issuer` block, so a toggle to "withhold" it would be a lie). Output is copy-JSON + download (same paste-JSON idiom as Receive / the issuers' "Copy wrapped document"); NO ZK on this path, NO backend, no store mutation (the held full credential is untouched). Reached via a "Share a redacted copy →" button on `CredentialDetail`.
- **Owner web e2e (Playwright)**: `stacks/owner/web/e2e/owner.spec.ts` drives the holder loop (receive → hold → display, plus an assertion that the retired `/present` browser proof route falls back safely to the wallet) + a tamper-rejection test + a **receipt test** (receive the CDC-modeled travel sample → `/receipts` → `/receipt/:root` renders Receipt ID, Section A/B/C, QR/public URL, derived status/provenance) + a **selective-disclosure test** (open Share → withhold a field → the redacted copy still `checkIntegrity`-verifies with the SAME `merkleRoot` + the withheld cleartext gone + `privacy.obfuscated` grown; re-importing that redacted copy makes the receipt omit the withheld value and show the obfuscated-count notice). Like the government e2e it is NOT in `pnpm test`/CI. It starts its OWN vite dev server and **mocks the ROAX RPC** at the network layer (deterministic), but runs the REAL client-side crypto. `pnpm --filter @dogtag/owner-web test:e2e`; `OWNER_URL=<url>` runs it against a live wallet instead (no self-server).

### Per-role records DB + CRUD (management layer)
Each role platform persists the records it issues into its OWN store (separate Mongo per running instance; `MemStore` for demo/tests), bundling the credential data with its **immutable on-chain proof**: tx hash, block number, contract (DogTagIssuer clone) address, and a ready-to-click explorer link `https://explorer.roax.net/tx/<hash>`.
- **vet-api** (the vet role — these records routes are ISSUANCE surfaces, so they are not mounted for `BUSINESS_TYPE=groomer`; see "The groomer IS the vet binary" below — one DB per instance): `store::Record` gained `block_number`/`explorer_url`/`created_at`/`updated_at`/`label`/`notes`/`revoked_*`/`invalidated_at`/`invalidation_reason` + `RecordStatus::Expired`; `Store::list_records` (Mem + Mongo, most-recent first). Routes: `GET /records` (operator-gated list, surfaces explorer links), `PATCH /records/:id` (off-chain metadata only), plus the existing soft-invalidating `POST /records/:id/revoke`. `block_number` is captured in `confirm_inner` from `TxView.block_number`; the revoke path reads the revoke tx's block via `get_tx_view`.
- **government-api** (own DB): `store::IssuedCredential` gained the same proof + metadata fields + a `CredentialStatus` enum; routes `PATCH /v1/records/:root` and `POST /v1/records/:root/revoke` (adds `ChainClient::revoke` + `revoke_calldata`; `SentTx` now carries `block_number`).
  These routes are gated by `Authorization: Bearer <GOV_API_TOKEN>` — as are issue and the operator record reads (`GET /v1/records`, `GET /v1/records/:root`, which leak Section A person PII if open); health, verify, the verifications audit log, and the public PII-free receipt endpoints stay open (see the "Government travel receipt" section above for the full gating rationale). Missing/wrong token → 401; in demo mode (`GOV_DEMO_MODE` et al) an unset `GOV_API_TOKEN` defaults to `dogtag-gov-demo-token` (the portal's `VITE_GOV_API_TOKEN` falls back to the same value); in non-demo mode with no token configured, the gated routes fail closed with 503.
- **Immutability**: `PATCH` accepts ONLY off-chain fields (`label`/`notes`, and `status` → `expired`); any on-chain-derived key in the body (tx hash, block, contract/issuer addr, root, wrapped doc, explorer url) is **rejected 400** ("… is on-chain-derived and immutable"). See the `IMMUTABLE_KEYS` list in each `routes.rs`.
- **Soft-invalidation, never hard delete**: revoke flips status to `revoked` on-chain (isValid → false) but keeps the row + its original issuance proof AND adds a revoke-tx proof; `expired` is an off-chain-only status transition (anchor untouched). Both stay listed + explorer-verifiable. There is NO delete endpoint by design. State machine: revoke accepts `issued` OR `expired` records (a compromised-but-expired credential can still be invalidated on-chain); expire accepts ONLY `issued` — anything else, incl. a revoked record, is rejected 409 (an off-chain `expired` must never mask an on-chain revocation; `revoked` is terminal).
- **Web**: the VET portal's `stacks/vet/web/src/pages/Records.tsx` reads `api.listRecords()` from the backend DB (NOT the old localStorage `recordsStore`) and offers edit/expire/revoke via the shared `@dogtag/ui` client (`listRecords`/`updateRecord` in `packages/ui/src/api/client.ts`). The GROOMER has no Records page: `BUSINESS_TYPE=groomer` does not mount the records routes at all (see the role-gate note above). The government portal has its own `Records` page (`stacks/government/web/src/pages/Records.tsx`) using the `@dogtag/ui` `Table`/`Badge`.
- **Tests**: hermetic Rust integration tests (`stacks/{vet,government}/api/tests/records_crud.rs`, MemChain+MemStore) prove issue→persist-proof→list→patch(reject on-chain)→revoke(soft)→expire. Playwright: `government/web/e2e/records-crud.spec.ts` runs full-stack against a demo backend started with **`GOV_DEMO_MODE=1 GOV_CHAIN_BACKEND=mem`** (real store + mem chain) — `GOV_DEMO_MODE` alone selects only the store, so without `GOV_CHAIN_BACKEND=mem` the backend runs LIVE and, with no `GOV_SIGNER_KEY`, `/issue` dry-runs and the spec's explorer-tx-link assertion fails; `stacks/vet/web/e2e/records.spec.ts` drives the Records UI against a **mocked** backend (route regex `^https?://[^/]+/api/` — a `**/api/**` glob wrongly swallows `@dogtag/ui`'s `src/api/*.ts` module scripts and breaks the mount). None are in CI (need a served portal + browsers).

### Government travel receipt (CDC-modeled TRAVEL_CLEARANCE)
The government `TRAVEL_CLEARANCE` credential is a CDC-modeled travel receipt (research `dogtag-govreceipt-r7` §2.1 + arch `dogtag-govarch-r8`). Grounding rules that are easy to get wrong:
- **Nested CDC subject.** `build_gov_vc` (`stacks/government/api/src/app.rs`) builds a nested `credentialSubject`: `importer`/`consignee` (**Section A** — person PII, the private/obfuscatable block), `animal` (**Section B**), `travel` (**Section C**), a `validity` block, plus top-level `receiptId`. Nesting flattens to leaf key-paths automatically (`credentialSubject.importer.firstName`, …). B/C + validity + receiptId are PUBLIC (revealed leaves); A is obfuscated by the holder. `dogTagId` stays mandatory + non-obfuscatable. The envelope (attestationType/trustTier/legalEffect/legalBasisVersion/jurisdiction) is UNCHANGED. The web Issue form (`stacks/government/web/src/pages/Issue.tsx` `RECORD_TYPE_SECTIONS`) sends a flat subset whose keys map 1:1 onto these leaves (`importerLastName`, `animalName`, `travelType`, …), grouped into the CDC sections; the sectioned form + printable receipt view landed in **PR-2** (see "Government receipt UI" below).
- **Receipt ID = public salted leaf + off-chain lookup handle — NOT the nullifier.** 12-char Crockford-base32 from a CSPRNG (~60 bits), minted in `routes::issue` (`gen_receipt_id`, uniqueness-retried), committed into `R` as a leaf AND stored on `IssuedCredential.receipt_id` (Mongo unique+sparse index on `receiptId`; `Store::get_credential_by_receipt_id`). Equating it to the ZK nullifier was ruled unsound (nullifier is per-verification, consumed once, unlinkable). `IssuedCredential` also denormalizes a cleartext `subject` projection + `valid_until`; all three (`receiptId`/`subject`/`validUntil`) are in `IMMUTABLE_KEYS` (mirror content committed in R).
- **Issuance date is DERIVED from the chain, never a leaf** (arch DP-2): read `DogTagIssuer.issuedAt[R]` (the anchoring block timestamp). `validUntil` DOES stay a public salted leaf (policy-variable window).
- **Derived `effectiveStatus`** computed at read time everywhere a record renders: `revoked ? REVOKED : (status==expired || today > validUntil) ? EXPIRED : VALID` (a never-anchored draft → `DRAFT`). `routes.rs` has `derive_effective_status` (pure, for list/detail) and folds it against a LIVE `isValid(R)` read in `resolve_receipt_status` (public endpoints). Date math uses a self-contained civil-from-days helper (no chrono/time dep); ISO dates compare as strings.
- **Public, PII-free endpoints (no auth):** `GET /v1/receipts/:receiptId/status` (JSON: effectiveStatus, recordType, receiptId, validUntil, issuanceDate, root, issuerAddr, explorer links, checkedAt — via a LIVE `isValid(R)` read, not a DB echo) and `GET /r/:receiptId` (server-rendered HTML status page, status-only by default per arch DP-5 — NO Section A/B/C content).
- **Issue AND the operator record reads are now GATED** behind the `require_api_token` bearer: `/v1/travel-clearance/issue` (arch DP-6; was open) plus `GET /v1/records` and `GET /v1/records/:root`, which are gated because the CDC subject denormalizes Section A person PII (idNumber, dateOfBirth, email, phone, name) into the record — an unauthenticated read would leak it. Verify, health, the verifications audit log, and the PUBLIC PII-free receipt endpoints (`GET /v1/receipts/:receiptId/status`, `GET /r/:receiptId`) stay open; demo keeps the baked `dogtag-gov-demo-token`. Callers must send the bearer — the web app (`apiGet`/`apiPost(..., {auth:true})`), `scripts/e2e-roles.sh` (`$GTOK`), and the Rust integration tests were updated accordingly.
- **OPS-0 (on-chain prereq, live on ROAX chainId 135).** The per-record-type `DogTagIssuer` clones are deployed via `DogTagIssuerFactory.createIssuer(name, keccak256(recordType), business)`, and the government signer must be whitelisted on `IssuerRegistry.whitelistFor(keccak256(recordType), signer)` — `DogTagIssuer.issue` is `onlyWhitelisted`. **A clone is gated by its OWN `registry()`, so it must be bound to the same `IssuerRegistry` the stack reads** (`ISSUER_REGISTRY_ADDR`); a clone on a superseded registry fails closed with `NotWhitelisted` even after a correct `whitelistFor`, and nothing before the first issuance reveals it (`scripts/demo-up.sh` now preflights both the factory's and the clone's `registry()` to catch exactly this). Canonical addresses live in `contracts/deployments/roax.json` → `government_clones` (mirrored in `stacks/government/.env.example`): **TRAVEL_CLEARANCE `0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F`** (factory `0xED20269E`, `business ==` the governance signer `0x8E27E117…` which is the factory `Ownable` owner; verified `isClone` true, `registry() == 0xAEE54035…`), and **EU_HEALTH_CERT is NOT deployed on the fresh set** — leave `EU_HEALTH_CERT_ISSUER_ADDR` unset (the API reports the issuer as null and `/issue` dry-runs) until `scripts/demo-provision-government.sh` deploys one. **Do NOT wire the earlier `0x8e276BD4…` / `0xe30A1739…` pair:** they are bound to the RETIRED `IssuerRegistry 0x5d86e4CF…`, are not clones of the fresh factory, and are quarantined in the ledger as `government_clones_deadRegistry_legacy`. Governance Phase-2 moved factory ownership + `WHITELIST_ADMIN` to the governance signer, so every NEW clone deploy / whitelist grant flows through that holder (already-deployed clones are immutable and unaffected).

### Government receipt UI + portal shell (PR-2)
The government web portal (`stacks/government/web`) was migrated from the hand-rolled dark SPA onto the shared **`@dogtag/ui` AppShell + Tailwind + tokens** (same stack as vet/groomer/admin) and gained the printable CDC-modeled receipt view. Structure + sharp edges:
- **Build wiring (was a lean SPA, now a `@dogtag/ui` consumer):** added `tailwind.config.ts` (scans `../../../packages/ui/src/**` so the shared components' token classes are emitted), `postcss.config.js`, deps (`@dogtag/ui`/`@dogtag/standard`/`lucide-react` + tailwind/postcss/autoprefixer), `index.css` = `@import "@dogtag/ui/tokens.css"` + `@tailwind` layers, and `vite.config.ts` `optimizeDeps.exclude` for the workspace-source packages. `main.tsx` wraps in `ThemeProvider`(default light)+`ToastProvider` (NO WalletProvider — government auths with a bearer token, not a wallet). App split into `app/Layout.tsx` (AppShell) + `pages/{Issue,Verify,Records,Receipt}.tsx` + `lib/api.ts`. The Dockerfile already `COPY packages` so the shared UI builds in-image.
- **Receipt view** `pages/Receipt.tsx` at route `/receipt/:root`: the authenticated, CDC-anatomy receipt. Fetches `GET /v1/records/:root` (auth, carries the Section A/B/C PII `subject`) for content + `GET /v1/receipts/:receiptId/status` (public) for the LIVE `effectiveStatus` + chain-derived `issuanceDate`. Renders letterhead + status chip + Receipt ID / issuance / validity block + legal preamble + Section A/B/C tables + a Verification block with a **QR** to the public status page. The receipt sheet uses a FIXED light palette (the `.receipt-sheet` CSS in `index.css`, NOT the theme tokens) so it always looks like the official paper in dark theme AND when printed. `@media print` strips the AppShell chrome (`aside`,`header`,`.no-print`) so browser "Print → Save as PDF" yields the clean document.
- **QR target = same-origin `/r/:receiptId`** (`publicReceiptUrl` in `lib/api.ts`), the PII-free server-rendered page PR-1 built on the BACKEND. Because the SPA history-fallback would otherwise swallow `/r/*`, both the vite dev proxy (`vite.config.ts`) AND nginx (`nginx.conf`) proxy `/r/` straight to the api service (NOT prefix-stripped) so the QR resolves on the portal's own origin. If you add more public backend-owned paths, proxy them the same way.
- **Derived-status rendering:** the Records table shows the backend's derived `effectiveStatus` (VALID/EXPIRED/REVOKED) as the colored `Badge`, plus an amber "expires ≤30d" chip; a separate `data-testid="record-status"` span keeps the RAW lifecycle status text (`issued`/`revoked`/`expired`) that `records-crud.spec.ts` asserts exact-match on. Don't merge the two — the e2e needs the raw word.
- **e2e test-id contract (do not rename):** the migration preserved every selector the two Playwright specs use — `record-type` MUST stay a native `<select>` (Playwright `selectOption` can't drive a radix Select); the dogTagId field MUST stay the FIRST `<input>` on `/issue`; the dogTag cell keeps class `mono` (`td.mono`); the Verify verdict keeps a literal `ok`/`bad` class token; and `issue-submit`/`wrapped-doc`/`copy-wrapped`/`verify-*`/`pillar-*`/`record-row`/`edit-*`/`expire`/`revoke`/`explorer-link`/`revoke-explorer-link`/`records-refresh` are all retained. `receipt.spec.ts` adds the new flow (issue → open receipt → sections + QR render → public `/r/:id` page shows the verdict). None of these are in CI (need a served portal + browsers).

### Government phone handoff — record-share QR + owner-hidden verify QR
The government stack gained the two QR flows the vet/groomer stacks already had. It is its OWN binary,
so the endpoints are implemented in `government-api`, but the SEMANTICS are mirrored from vet-api
deliberately: both mobile apps hard-code these paths and must need no government branch.
- **The four phone-facing paths are a hard contract.** `GET /r/<token>` (import),
  `GET /x/<token>` (verify-session resolve), `POST /v1/verify/consent`, `GET /verify/session/:id?token=`.
  They are hard-coded in `apps/android/.../net/CentralApi.kt` and `apps/ios/DogTag/Net.swift` against
  whatever host the QR names. Never rename them, never add a `/v1` prefix to the two that lack one, and
  never make the operator-facing start route the one the phone hits. Token shape is 32 hex (16 CSPRNG
  bytes) because `QrPayload.parse` keys on the URL SHAPE; TTLs are share **180s** / export **600s**
  (export is longer because on-device Groth16 proving takes tens of seconds). Share tokens are consumed
  on first read; export tokens are peeked, with the session `status` as the replay guard.
- **`GET /r/:id` on government is OVERLOADED and must stay dispatched by SHAPE.** A 32-hex segment is a
  record-share token (JSON, consumed); anything else is a public receiptId (the PII-free HTML status
  page that shipped in PR-1). The shapes cannot collide — 32 hex vs the 12-char Crockford-base32
  `gen_receipt_id`. Once the segment is known to be a share token the handler MUST stay in the JSON
  world including its 404, or the phone reports "bad wrapped doc" instead of "expired".
- **`unverifiedClaims` on `/x/<token>` is MANDATORY, not decorative.** `ScanScreen` refuses the whole
  owner-hidden flow when the block is absent, and `validateDiscovery` refuses when the claimed
  `verificationRegistry` disagrees with the on-chain `ProtocolRegistry` anchor. Government carries ONE
  registry address (`VERIFICATION_REGISTRY_ADDR`, aliased `VERIFICATION_REGISTRY_CONSENT_ADDR`) which must
  be the same unified consent registry the siblings use; `GET /health` echoes it (plus `issuerRegistry`
  and `deploymentUrl`) so a mismatch is eyeball-able rather than an opaque phone-side refusal.
- **The authority verifies through the same ZK consent path as anyone else** (`verify.rs`, a mirror of
  vet-api's `consent_submit_levelb` with the relayer resolved from `GOV_SIGNER_KEY` instead of a custody
  vault). Capability comes from the `VERIFY:<purpose>` namespace, which is SEPARATE from the issuer
  record-type roles the authority already holds — an issuer is not implicitly a verifier. Portal purpose:
  `travel_check` (`app::VERIFY_PURPOSES`), an existing label so owner consent receipts render it by name.
  Without the grant `POST /verify/session/start` refuses 403 `relayer not whitelisted for this purpose`
  BEFORE any QR is shown, and the portal names the signer + registry + how to fix it.
- **`profileDisclosure` runs checks (1) pure fold + (2) binding-to-this-proof only.** vet-api also
  preflights `R == profileRoot(dogTagId)` and `isValid(R)`, both gated there on addresses government does
  not hold (it is a VERIFY-only deployment for `DOG_PROFILE`), so this takes exactly the branch vet takes
  when they are unset. `VerificationRegistryConsent` re-runs both on-chain, and check (2) is what extends
  that enforcement to the disclosure.
- **The QR host comes from `DEPLOYMENT_URL`, server-side.** Never `window.location.origin` — the portal's
  origin is routinely a laptop `localhost` no phone can reach. `/x/` is now proxied alongside `/r/` in
  both `vite.config.ts` and `nginx.conf` so a portal-origin `DEPLOYMENT_URL` also resolves.
- **Countdowns count from `ttlSecs` at response receipt**, not `expiresAt - Date.now()`: the two clocks
  are the backend's and the browser's, and skew would show an expired timer over a perfectly good QR.
- Tests: `government/api/tests/share_qr.rs` (mint/resolve/one-time/refresh/auth + the `/r/` overload) and
  `tests/verify_session.rs` (not-whitelisted 403, QR shape, mandatory claims, dual-gated poll, the
  preflight rejects, and that the paste-JSON fallback still works). MemChain cannot verify Groth16, so
  the ZK soundness of this path stays the registry's — pinned in `contracts/test`, not here.

### Mobile travel receipt + `obfuscate()` FFI (PR-3)
The pet-owner HOLDER apps (iOS `apps/ios/DogTag`, Android `apps/android`) render a held `TRAVEL_CLEARANCE` credential as the same CDC receipt the web portal shows, produced LOCALLY from the stored `wrappedDocJson`. Structure + sharp edges:
- **`obfuscate()` is now in the mobile FFI.** `crates/dogtag-standard-rs/src/ffi.rs` exposes `obfuscate_document_json(wrapped_doc_json, key_paths) -> String` (UniFFI → Swift `obfuscateDocumentJson(wrappedDocJson:keyPaths:)`, Kotlin `obfuscateDocumentJson(wrappedDocJson, keyPaths)`). It wraps `wrap::obfuscate` (already existed, just wasn't surfaced): moves each named leaf's hash into `privacy.obfuscated[]` and drops the cleartext, leaving the Merkle root == on-chain root R UNCHANGED. So the phone builds a PII-free presentation copy with ZERO new ceremony — it's the merkle selective-disclosure proof, NOT a ZK proof. `credentialSubject.dogTagId` must never be obfuscated (`verify.rs` rejects it). Key paths are the FULL dotted path incl. the `credentialSubject.` prefix.
- **Regenerating the bindings is MANDATORY after any FFI change.** The committed `apps/ios/DogTag/dogtag_standard.swift` and `apps/android/app/src/main/java/uniffi/dogtag_standard/dogtag_standard.kt` carry UniFFI contract CHECKSUMS; if they don't match the freshly-built `.so`/`.a` the app traps at the first FFI call. Android CI rebuilds only the `.so` (cargo-ndk) and bundles the committed `.kt` as-is — it does NOT regenerate it — so you MUST regenerate + commit both. Build the host dylib WITH `--features prover` (else the consent prover surface - `proveConsent`/`ProofFfi` - drops out and the ABI shifts), then `cargo run --features prover,uniffi/cli --release --bin uniffi-bindgen -- generate --library target/release/libdogtag_standard.dylib --language {swift,kotlin} --out-dir <tmp>` and copy both outputs over the committed files (the generator output matches the committed style; the diff is additive).
- **`TravelReceiptView.swift` / `TravelReceiptScreen.kt`** mirror `stacks/government/web/src/pages/Receipt.tsx` 1:1 (Section A/B/C labels, sex+neutered combine, humanize, empty-row omission). Reached from `CredentialDetailScreen` via a "Show travel receipt" button gated on `group == .travel`. They decode `credentialSubject` leaves into a dotted-path→value map from `WrappedDoc.decodedFields()` (strip the `credentialSubject.` prefix), render the effectiveStatus banner (live `RoaxRpc.isValid` → REVOKED wins, then lapsed `validity.validUntil` → EXPIRED, else VALID; chain-unreachable falls back to the stored integrity verdict), and a Verification block with a QR.
- **The QR is PII-free and points at `https://<issuer.domain>/r/<receiptId>`** — the public status page PR-1 built. This is a NEW, deliberate exception to the "QR generation removed" rule in `QR.swift` (that removal was for the one-time verification-JWT presentation QR; a status-page URL leaks nothing). iOS draws it with CoreImage `CIFilter.qrCodeGenerator` (no dep); Android with `com.google.zxing:core` (added to `app/build.gradle.kts` — ML Kit only SCANS, it can't ENCODE). `receiptId` and `issuer.domain` come from the credential itself (the `receiptId` leaf + `issuer.domain`); if the gov web app is hosted somewhere other than its `did:web` domain the URL would need another source, but the receipt also prints the id as text.
- **Selective disclosure is holder-controlled.** Section-A person-PII leaves default to WITHHELD; per-field reveal toggles flip them; `dogTagId` + Section B/C default visible. "Share redacted" runs `obfuscateDocumentJson` over the withheld leaves and hands the redacted `wrappedDoc` to the OS share sheet (iOS `UIActivityViewController`, Android `ACTION_SEND`). Withheld rows render as "— withheld by holder —". NO ZK on this path; the on-device Groth16 prover stays reserved for the separate anonymous verification-record flow.
- **Issuance date** comes from the `validity.issuedOn` leaf (the phone can't read on-chain `issuedAt[R]`); falls back to the imported record's `issuedOn`.

### Self-custody export UX (iOS, `apps/ios/DogTag`)
Holder-side backup/migration rights: copy the phrase at creation, re-export it later, and export held credentials. Single account only (no HD multi-account/derivation switcher - deliberately scoped out).
- **The embedded wallet now persists the 32-byte BIP-39 *entropy*** (`Wallet.swift`, Keychain account `dogtag_wallet_entropy`, SAME protection as the seed: `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, no `kSecAttrSynchronizable`, so never iCloud-synced). This is a **deliberate change to the prior "mnemonic never persisted" property**: BIP-39 seed to mnemonic is one-way, so without the entropy the 24 words are unrecoverable after genesis, and "export your recovery phrase later" is impossible. `Wallet.revealMnemonic()` re-derives the exact phrase via `Bip39.entropyToMnemonic`; it returns `nil` for wallets created before this change (seed-only), whose phrase is genuinely gone. **A `nil` is AMBIGUOUS, though** - `revealMnemonic()` also returns `nil` on a Keychain READ failure - so only the presence-only `Wallet.hasExportablePhrase()` may diagnose a wallet as legacy; wiring the destructive remedy to the weaker `mnemonic == nil` test would offer to destroy a healthy, phrase-backed wallet (see "Legacy-wallet rescue + the Danger zone"). The entropy is no more sensitive than the already-stored seed (both fully control the wallet). Callers gate `revealMnemonic()` behind a fresh `Biometric.authenticate` and never log/transmit it.
- **`ProfileScreen.swift`** owns the wallet/Profile screen (a *separate* crew owns the rest of the app UI). At creation the recovery-phrase card has a **Copy phrase** action (auto-expiring pasteboard via `SecureClipboard.copySecret`, 90s TTL) + an "I've saved it" acknowledgment that hides it. A biometric-gated **Export account keys** button opens `ExportAccountSheet` (hard security warning + numbered 24-word grid + copy). Every displayed value is **tap-to-copy with a "Copied" flash** via the reusable `CopyRow` (wallet address, Consent Ax, keyHash, dog-tag ids; copies the FULL value, not the truncated preview). **Sharp edge:** the export sheet MUST be presented with `.sheet(item:)` (Profile's single `$sheet` route binding, whose `.export` case carries the revealed secrets as an `Identifiable` payload), NOT `.sheet(isPresented:)` reading sibling `@State` set in the same handler - SwiftUI evaluates that sibling state as still-nil on the first present, so the phrase/key silently render as "unavailable" even though `revealMnemonic()`/`revealPrivateKeyHex()` returned real values. Dismissing the `.sheet(item:)` nils the binding, which also releases the secrets from memory.
- **Document export** (`DocumentsScreen.swift` + `CredentialDetailScreen.swift`) uses the app's existing `WrappedDoc` JSON as the portable form, **no new format**. `ExportedDocument: Transferable` + native SwiftUI `ShareLink`/`SharePreview`; single credential = its `wrappedDocJson` verbatim, list export = a JSON array of the shown docs (respects the pet filter). No `UIActivityViewController` bridge added; `TravelReceiptView`'s existing redacted-share `ShareSheet` is left untouched. **Sharp edge:** use `FileRepresentation(exportedContentType: .json)` + `SentTransferredFile` (both iOS 16), which carry the filename via the written temp-file URL. Do NOT use `DataRepresentation(...).suggestedFileName {...}` - `suggestedFileName` is iOS 17+ and the deployment target is 16.0, so it fails the build.
- **Raw secp256k1 private-key export is included** (captain-approved) alongside the phrase in the same biometric-gated `ExportAccountSheet`: `Wallet.revealPrivateKeyHex()` returns the 0x-hex 32-byte key, shown with its own hard warning and tap-to-copy (auto-expiring clipboard), never logged/transmitted. It matters BECAUSE the private key is NOT a subset of the phrase for migration: this wallet derives the secp key as the **raw BIP-32 master key** (`Bip39.seedToSecp256k1Priv` = HMAC-SHA512("Bitcoin seed", seed)[:32]), **not** `m/44'/60'/0'/0/0`, so the mnemonic imported into a standard EVM wallet yields a DIFFERENT address; only the raw private key reproduces the on-chain `userWallet` elsewhere. Available even for legacy seed-only wallets (needs only the seed, not the entropy).

### Oversight indexer (PR-4)
The **net-new, separately-deployable** `stacks/indexer/api` crate (`indexer-api`, port **46001**, own Mongo `indexerdata`, `stacks/indexer/docker-compose.yml`, `make`-free — run via compose) is the on-chain oversight feed the arch calls for (`dogtag-govarch-r8` Part 4; the admin portal `dogtag-adminportal-a3` is its later UNSCOPED consumer).
It scans the ROAX (chainId 135) contract event logs into a **non-PII** queryable index and serves a **scope-enforced** query API. It is a backend service only — **no web UI in this PR** (the admin/government/vet portals are the later consumers). Design + sharp edges:
- **What it watches (all non-PII, arch §4.3):** `DogTagIssuerFactory` `IssuerCreated(clone,recordType,name)` + `RootRegistered(root,clone)`; `IssuerRegistry` `Whitelisted`/`Delisted(recordType,signer)`; every `DogTagIssuer` clone `RootIssued`/`RootRevoked(root,by,ts)`; and the `Verified` event (M8 shipped an additive dual-decode of both the retired subject-bearing shape and the owner-hidden shape during the migration; since collapsed - the indexer now decodes ONLY the subject-less owner-hidden `VerificationRegistryConsent` `Verified(dogTagId,relayer,purpose,nullifier,deadline,ts)` shape, mapping to `EventType::Verified` with a `deadline` and no `subject` field at all). Each log is flattened into a uniform `IndexedEvent` (`src/events.rs`) keyed by `id = txHash:logIndex` (the idempotency key — re-scans upsert, never duplicate). Roots are salted commitments, `dogTagId` is the non-personal SBT id, addresses are public signers — **no PII in the index** (doctrine).
- **Scan / decode (`src/chain.rs`, `LogSource` trait).** `AlloyLogSource` = real `eth_getLogs` filtered by event *signature* (topic0) with **no address filter**, so it catches every clone's `RootIssued`/`RootRevoked` regardless of when the clone was deployed. Each decoded log is then **anti-spoof-gated by emitting address**: factory events must come from the factory, registry events from the registry, a `Verified` only from `DEFAULT_VREG_CONSENT` (the owner-hidden `VerificationRegistryConsent`; the retired registry's `DEFAULT_VREG` gate went with the dual-decode) — and a `RootIssued`/`RootRevoked` only from a **known clone** (seeded from `roax.json` government + demo clones via `SEED_CLONES`, extended at runtime by `IssuerCreated`). Logs are processed in `(block,logIndex)` order so an `IssuerCreated` folds its clone into the known set before that clone's first issuance in the same range. `MemLogSource` is a scriptable in-memory source (with `chain::emit::*` alloy-encoding helpers) so the whole scan→index→query flow is testable with no node — the SAME `decode_log` runs both paths.
- **Finality-aware ingest loop / resume (`src/indexer.rs`) — captain-directed model.** ROAX is an EVM/PoS chain **with block finality** (verified live: `devrpc.roax.net` exposes the `finalized` AND `safe` block tags — `finalized` sits ~80 blocks behind `latest`). A finalized block can never reorg, so every indexed event carries a `Finality` lifecycle (`src/events.rs`): **finalized** (block ≤ the finalized watermark — immutable, never rewound/re-scanned) vs **pending** (block > watermark — still reorg-able, the ONLY range reorg logic touches). This matters for a *government oversight* feed: it must never present a pre-finality, reorg-able issuance as authoritative. Each tick: read `head` + the `finalized` tag (`LogSource::finalized_block()`; **fallback** to a `head - CONFIRMATIONS` watermark, logged as `confirmations-fallback`, if a node ever lacks the tag); scan `[last_finalized+1 .. head]` into a buffer (stamping each event finalized/pending from the watermark), then **atomically swap** the pending range — `delete_pending()` + upsert the re-derived set — only after the whole fallible scan succeeds, so a transient RPC error on any chunk leaves the prior pending rows intact instead of blanking the feed. A pending event orphaned by a reorg simply disappears (absent from the re-derived set) and finalized rows are untouched (no rewind needed). Promotion pending→finalized happens naturally as the watermark advances and the range is re-derived. The resume cursor persists the **finalized watermark** (`last_finalized` + its hash); a defensive hash-divergence guard at the watermark only ever fires under the confirmations fallback (a deeper-than-N reorg), rewinding via `delete_from_block`. `rebuild_known_clones()` on startup re-derives the clone set from previously-indexed `IssuerCreated` rows.
- **Finality on the query surface.** Every event JSON carries `finality`; `?finality=finalized|pending` filters; `/v1/stats` reports `finalized`/`pending` counts; `/v1/status` reports `finalizedBlock` + `finalitySource` (`finalized-tag` vs `confirmations-fallback`) + `lastFinalizedIndexed` + `lag`. The feed returns ALL events clearly annotated (not hidden), so an oversight consumer can default its authoritative view to `finality=finalized` while still seeing in-flight activity. Scope enforcement is unchanged.
- **Scoping is server-side (`src/scope.rs`) — the load-bearing doctrine.** A bearer token resolves (via `INDEXER_SCOPES` JSON) to a `Scope`: `Unscoped` (government oversight — every event) or `Signers{signers,clones}` (a business sees ONLY events whose acting signer ∈ its signers OR whose clone ∈ its clones). `Store::query_events(&q, &scope)` enforces admission; client filters (`type`/`signer`/`issuer`/`recordType`/`root`/`dogTagId`/`since`/`until`) only ever **narrow within** the token's ceiling — a scoped token can never reach another issuer via a query param (there is an integration test for exactly this). Empty registry + not demo ⇒ every query 401s (fail-closed, mirrors the government stack).
- **Query API (`src/routes.rs`, all bearer-gated except `/health`):** `GET /v1/events` (the feed — filters + newest-first + pagination), `GET /v1/stats` (in-scope counters: issued/revoked/active/verifications/clones/signers), `GET /v1/issuers` (deployed clones + per-clone issued/revoked counts), `GET /v1/status` (head/lastIndexedBlock/lag/scope). Every event is joined to the **signer→business directory** (`src/directory.rs`) to add `actorName`/`cloneName` where possible, plus a `txUrl` explorer link. `?recordType=` accepts a human label (keccak'd server-side) or a raw `0x` key.
- **Directory join (`src/directory.rs`) — doctrine-safe naming.** Two layers: operator-authoritative static seeds (`INDEXER_DIRECTORY` JSON `{addr:name}`), and optional admin-API enrichment (`ADMIN_API_BASE`/`ADMIN_API_TOKEN`) that periodically reads the admin `/v1/businesses` (public) + `/v1/issuer-applications` (admin-token) and joins signer addresses → business names on the shared `domain`. Reads **business identity only — never any role's PII Mongo**; any admin-API failure degrades to static-only.
- **Store (`src/store.rs`, `Store` trait).** `MemStore` (demo/tests) + `MongoStore` (feature `mongo`, `src/mongo.rs`; `events` keyed by `id` unique, `cursor` single doc). The Mongo query pushes the high-selectivity equality/range predicates then re-applies scope + `EventQuery::matches` + pagination in Rust (identical semantics to MemStore). The index is fully rebuildable from the chain, so a lost volume just triggers a re-scan from `START_BLOCK`.
- **Demo mode** (`INDEXER_DEMO_MODE=1`): scripted `MemLogSource` history (deploy → whitelist → issue×2 → verify → revoke on the gov clone, plus a demo-groomer issuance on the DOG_PROFILE clone) + `MemStore`, and two well-known tokens — `dogtag-indexer-oversight-demo-token` (unscoped) and `dogtag-indexer-vet-demo-token` (scoped to the DOG_PROFILE clone + demo-groomer signer). The demo sets the finalized watermark to block 6, so the gov flow shows as **finalized** and the newer demo-groomer DOG_PROFILE events show as **pending** — the feed demonstrates the finality lifecycle with no node/Mongo. `MemLogSource::set_finalized(h)` scripts the `finalized` tag; tests use it (+ `reorg_from`) to drive finality/promotion/reorg cases.
- **Simulated-source disclosure (`LogSource::backend()`) - a fleet-wide convention, not an indexer detail.** A simulated backend must be structurally impossible to mistake for a live one, so **which surface a source reads is a property of the source object, never of a `*_DEMO_MODE` config flag**. `LogSource::backend() -> SourceBackend::{Live,Simulated}` is **required with no default body**: a new source will not compile until it declares, so it can never silently inherit "live". `/health` and `/v1/status` derive `simulated`/`backend` from a SINGLE `backend()` read, and take `chainId` **from the source object rather than from `Config`** (which no longer carries one) - `null` when simulated (`chain::SIMULATED_CHAIN_ID`), because a scripted source is on no network at all. Read that claim precisely: sourcing the id from the object is what makes the SIMULATED case structural (a scripted source has no id to give, so it cannot echo a real network however `CHAIN_ID` is set), but a LIVE id is still operator-asserted - `main.rs` builds `AlloyLogSource::with_chain_id` from the `CHAIN_ID` env var and nothing checks it against the node, so pointing `ROAX_RPC` at another network while leaving `CHAIN_ID=135` still reports `135`. Both keys are emitted on BOTH paths: a flag present only when simulated makes its absence ambiguous between "live" and "a build too old to tell you", i.e. *could not check* rendering as a neighbour. This is the same defect, remedy and JSON shape as `government-api`'s `ChainClient::backend()`/`chain::ChainBackend` (`stacks/government/api/src/chain.rs`) - **two services, one convention; port it rather than reinventing it for any third.** What demo mode DOES is unchanged: `/v1/status` still reports the scripted `headBlock: 8`, now labelled rather than corrected.
  - **Known gap (separate axis, undisclosed).** `build_store(demo)` returns `MemStore` when `demo || MONGO_URI` is empty, so a NON-demo indexer with no `MONGO_URI` gets an ephemeral store on a live chain while `/health` correctly says `simulated:false` (that is the CHAIN axis) and says nothing about the store. Government keeps the two axes visible side by side (`backend` + `demo`); the indexer's `Config`/`AppState` carry no store-mode field yet. If closed, do it symmetrically via a `Store::backend()` - **never** by threading a `demo` bool into `Config`, which is the collapse that produced this class of bug in the first place.
- **Tests:** unit (`scope`/`store` modules, incl. `delete_pending` keeps finalized + finality filter) + `tests/simulated_disclosure.rs` (the disclosure regression guard - it defines its OWN `LogSource` impls rather than using `MemLogSource`, so the flag cannot be satisfied by special-casing a concrete type; it fails if disclosure is dropped, hardcoded, or read from config) + `tests/query_api.rs` drives the real ingest loop + HTTP router end-to-end (unscoped vs scoped counts, scope-cannot-be-widened, filters/stats/issuers, 401 auth, idempotent re-scan; **finalized events survive a pending-range reorg while orphaned pending events are dropped**; **promotion pending→finalized at the watermark**; **the owner-hidden `Verified` shape ingests with a `deadline` and no `subject` field, and a stranger-emitted `Verified` log is dropped by the anti-spoof gate** (the M8 dual-decode of the retired subject-bearing shape has since been collapsed)). All hermetic (MemLogSource + MemStore), in `cargo test -p indexer-api`.

### Governance / admin (audit H-3)
- Governed contracts split admin two ways: `IssuerRegistry` (3-day), `VerificationRegistry` (2-day, since retired from the repo), and `DogTagSBT` (3-day, since retired from the repo) use OZ `AccessControlDefaultAdminRules` (two-step `begin`/`acceptDefaultAdminTransfer` + timelock); `DogTagIssuerFactory` uses `Ownable2Step`. `DogTagIssuer` clones have no own admin — they read `IssuerRegistry.hasRole(0x00)`. `ConsentKeyRegistry` (since retired)/`Groth16Verifier` (since retired)/`Poseidon6` have no admin. (The retired-generation sources are gone from `contracts/`; their deployed instances remain on the disposable testnet as deployment history only, superseded in place by the 2026-07-23 r8 fresh redeploy.)
- `DogTagSBT` inherits BOTH `AccessControlEnumerable` and `AccessControlDefaultAdminRules`, so it must explicitly override `grantRole`/`revokeRole`/`renounceRole`/`_setRoleAdmin` (`override(AccessControl, IAccessControl, AccessControlDefaultAdminRules)`) plus `_grantRole`/`_revokeRole`/`supportsInterface` — `super` resolves to the ACDAR rules first, then chains the enumerable bookkeeping. Do NOT `_grantRole(DEFAULT_ADMIN_ROLE,...)` in the constructor; the `AccessControlDefaultAdminRules(delay, admin)` base already does, and a second grant reverts (`AccessControlEnforcedDefaultAdminRules`).
- **Governance handover is DONE on ROAX (Phase-2 executed).** The governance signer **signer-1 `0x8E27E117…`** now holds the registry `DEFAULT_ADMIN_ROLE` + `WHITELIST_ADMIN` AND is the `DogTagIssuerFactory` `Ownable2Step` owner; the old deployer EOA `0x119F8c7F…` (`roax.json:admin`, kept as the historical deploy record) lost those governance/admin authorities. **Do not call it role-free or neutral:** the M5 deployment preflight re-verified on 2026-07-16 that it still holds the retired-generation SBT's (still deployed) `ISSUER_ROLE` and remains whitelisted for the four known record types. Consequence for tooling: the demo/relayer/admin `ADMIN_PRIVATE_KEY` (the control-plane / GovernanceAction signer) **must now be signer-1 `0x8E27E117…`** — with the old EOA any governance write (`createIssuer`/`whitelistFor`/`adminRevoke`) correctly downgrades to a `Disposition::Proposed` payload instead of broadcasting. The key value itself is captain-managed env, never committed. The EOA→governance migration is shipped as reviewable code (`contracts/script/GovernanceMigration.sol` library + `MigrateGovernance.s.sol` two-phase Begin/Accept scripts + `GovernanceMigration.t.sol`) and lives on mainline (merged via PR #8) — see `docs/GOVERNANCE_MIGRATION.md`. The **deployed retired-generation** `DogTagSBT` (`0x1FB8…`) predates the two-step upgrade and is still plain `AccessControlEnumerable`; it can't be retrofitted without a state-orphaning redeploy (it remains on-chain as deployment history only, superseded in place by the 2026-07-23 r8 fresh redeploy). Never re-run the migration on live testnet without explicit captain approval.
- Removed dead governance surface: `IssuerRegistry.PROFILE_ISSUER_ROLE` and `DogTagSBT.UPDATER_ROLE` were declared but never enforced (SBT mint = `ISSUER_ROLE`; `setProfileRoot` = originator-or-`AUTHORITY_ROLE`). Don't re-add them.

### Admin control-plane foundation (PR-A: `GovernanceAction` + factory bindings)
The admin portal is the protocol control plane — it **extends** the existing `stacks/admin/web` (shared `@dogtag/ui` `AppShell`) + its `stacks/admin/api` AlloyChain signer; it is NOT a greenfield build (scout `dogtag-adminportal-a3`). "See on-chain activity" = the UNSCOPED consumer of the PR-4 indexer above (that UI is PR-B/PR-D). This PR-A landed only the backend **foundation**: the governance-action abstraction + factory bindings (no new web pages).
- **Three distinct on-chain authorities (`chain.rs`, plan Part 2).** Every privileged write is gated by ONE of: the **factory `Ownable2Step` owner** (`createIssuer`), the registry **`WHITELIST_ADMIN` role** (`whitelistFor`/`delistFor`), or the registry **`DEFAULT_ADMIN_ROLE`** (`adminRevoke`/role-admin/verifier+consent-key swaps, behind the 2–3 day ACDAR timelock). Governance Phase-2 has **executed**: all three now rest with the governance signer `0x8E27E117…` (the deployer EOA `0x119F8c7F…` was stripped of these three governance authorities, but NOT of the retired-generation SBT's (still deployed) `ISSUER_ROLE` + record-type whitelists, so it is not role-free; see "Governance / admin" above). **Do NOT hardcode any EOA as the authority** — the dispatcher reads the holder live, so the control plane keeps working (executing when the hosted key IS the holder, else proposing) across the handover.
- **`GovernanceAction` (`src/governance.rs`) — the key-holder-agnostic abstraction.** A privileged write is a value `{target, calldata, authority, summary}` where `authority` is `Owner{owner_target}` or `Role{role_target, role, default_admin}`. `governance::dispatch(chain, signer_index, &action)` asks the chain WHO holds the authority (factory `owner()` / registry `hasRole()` / `defaultAdmin()`): if the hosted signer holds it → `send_action` (sign-and-broadcast, the existing legacy-gas path) returning `Disposition::Executed{txHash,holder}`; else → `Disposition::Proposed{holder,target,calldata,…}` for the governance signer / Safe to execute out-of-band. This survives the Phase-2 split BY CONSTRUCTION: an action silently flips executed→proposed the moment its role leaves the hosted key — no code path assumes which key holds which role.
- **Factory bindings added to `chain.rs` (`ChainClient` trait, both `AlloyChain` + `MemChain`):** `predict_issuer` (deterministic clone preview, `salt = keccak256(recordType, business)` — exact BEFORE deploy), `create_issuer_calldata`, `is_clone`, `root_issuer`, plus the authority reads `ownable_owner`/`ownable_pending_owner`, `has_role`, `default_admin`, `pending_default_admin` (the Phase-2 handover surfaces here), and `signer_address(index)` (Alloy derives it from the key) so the dispatcher can test hosted-key holdership. `MemChain` gains seed setters (`set_factory_owner`, `set_role`, `set_default_admin`, `set_pending_default_admin`) + a deterministic (non-CREATE2) clone preview for hermetic tests.
- **Endpoints (`admin_router`, admin-session gated):** `POST /v1/admin/factory/predict` (address preview), `POST /v1/admin/factory/issuers` (deploy via `GovernanceAction`; `business` defaults to the hosted signer = single-authority topology, matching the deployed government clones; returns predicted address + `Disposition`), `GET /v1/admin/governance/authority` (the live authority map: factory owner + pending, WHITELIST_ADMIN/DEFAULT_ADMIN holders, `heldByHosted` per authority, pending DEFAULT_ADMIN transfer + ETA — best-effort, unreachable target → `null`). `recordType` accepts a human label (keccak'd server-side via `record_type_key`) or a raw `0x`+64-hex key.
- **Config:** `FACTORY_ADDR` (the `DogTagIssuerFactory` — address owned by roax.json, never transcribed here; `scripts/demo-up.sh` resolves it from env then that ledger, since without it every factory route answers `FACTORY_ADDR not configured`) + `ADMIN_SIGNER_INDEX` (now HONORED — `main.rs` previously hardcoded index 0 and ignored the env). Doctrine holds: the control plane reads the chain + the admin business directory only — never another role's PII Mongo.

### Admin indexer consumption + signer→business directory (PR-B: `admin-indexer-consume`)
The "see on-chain activity" **data layer** — the admin/central backend becomes the UNSCOPED consumer of the PR-4 oversight indexer, plus the authoritative signer→business directory that NAMES on-chain signers. Backend only; the Activity/Dashboard UI is PR-D. Doctrine: read the CHAIN (via the indexer) + the admin business directory; NEVER another role's PII Mongo; no PII in these aggregates.
- **`src/indexer.rs` — the `OversightFeed` client (unscoped consumer).** A trait (`events`/`stats`/`issuers`/`status`) with a real `HttpOversightFeed` (reqwest → the indexer on `:46001`, presenting the `unscoped:true` bearer so it sees EVERY issuer's events — no client filter can widen the token's server-side ceiling), a `DisabledFeed` (unset `INDEXER_API_BASE` → every call `NotConfigured` → 503, fail-closed, rest of the backend unaffected), and a `MemFeed` (canned payloads for hermetic tests). Mirrors the `business.rs` outbound-HTTP + `MemChain` mock patterns. Injected as `AppState.feed: Arc<dyn OversightFeed>`. The admin does NO `eth_getLogs` itself — the indexer is the sole event source.
- **`src/directory.rs` — `SignerDirectory` (the naming join, plan §3.5).** Built live from the store (`all_businesses()` + `all_applications()`) into a `HashMap<signer_addr, DirectoryEntry{business, businessId, entity, recordTypes, verifyPurposes, domain, status}>`. Join key = `IssuerApplication.addresses[]`; business name resolved by `issuerEntityId → Business.business_id`, else `domain → Business.domain`, else the bare application domain. Approved applications win over pending on a signer collision. This is the AUTHORITATIVE source; the indexer's own `directory.rs` is a best-effort copy it pulls from this same admin API. It carries zero client PII (business + signer identity only) and needs NO indexer (store-derived, always live). Promotes the old O(n) `verify_relay.rs` relayer→business scan to a proper indexed lookup.
- **Endpoints (`admin_router`, admin-session gated):** `GET /v1/admin/activity` (unscoped cross-issuer feed; pass-through narrowing filters `type`/`signer`/`issuer`/`recordType`/`root`/`dogTagId`/`finality`/`since`/`until`/`limit`/`offset`; each event re-enriched with the admin directory's authoritative `actorName`/`cloneName`, overriding the indexer's copy), `GET /v1/admin/activity/stats` (cross-issuer counts: active vs revoked credentials, verifications, whitelisted/delisted, distinct clones/signers, finalized/pending — the aggregates PR-D renders), `GET /v1/admin/activity/issuers` (per-clone issued/revoked/active, name-enriched), `GET /v1/admin/directory` (full signer→business listing), `GET /v1/admin/directory/signer/:addr` (one signer → `{business, entity, recordTypes, …}`, 404 if unknown). An unconfigured indexer 503s the `activity*` surfaces but the `directory*` surfaces keep working.
- **Config:** `INDEXER_API_BASE` (+ alias `ADMIN_INDEXER_BASE`) + `INDEXER_OVERSIGHT_TOKEN` (+ alias `ADMIN_INDEXER_TOKEN`) — the indexer root + its `unscoped:true` bearer. Tests: `tests/indexer_consume.rs` (7 end-to-end over the real router with a seeded `MemFeed` + store directory) + `directory.rs`/`indexer.rs` unit tests. `tests/common/mod.rs` gained `hermetic_state_with_feed(feed)` (the base `hermetic_state()` wires an empty `MemFeed`).

### Role-traceability portals (govarch PR-5: `dogtag-trace-w6`)
The per-role **consumers** of the PR-4 oversight indexer, one tier up from the admin PR-B consumer: each role sees the on-chain credential activity relevant to IT, joined to its own off-chain DB records. Three views, one doctrine — **government is UNSCOPED (every issuer), vet/groomer are SCOPED (own signer/clone only)**. No new PII; the feed is the non-PII chain layer (the join projection deliberately EXCLUDES the government TRAVEL_CLEARANCE `subject`/importer PII block).
- **`OversightFeed` client, ported per stack.** `stacks/vet/api/src/oversight.rs` (serves vet + groomer — same binary) and `stacks/government/api/src/oversight.rs` each carry the same trait (`events`/`stats`/`issuers`/`status`) + `HttpOversightFeed` (reqwest → indexer `:46001`) + `DisabledFeed` (unset `INDEXER_API_BASE` → `NotConfigured` → 503) + `MemFeed` (hermetic tests), mirroring the admin `src/indexer.rs`. Injected as `AppState.feed: Arc<dyn OversightFeed>`. Government had no `reqwest` dep — added inline (`0.12`, `rustls-tls`).
- **Two-layer server-side scoping (the load-bearing property).** (1) The INDEXER scopes by bearer token (a vet/groomer presents a SCOPED `INDEXER_SCOPES` token → `Scope::Signers`; the government presents the `unscoped:true` token). (2) The role backend RE-CHECKS every returned event against a **local scope gate** (`crate::trace::LocalScope::admits` — `actor ∈ own signers OR clone ∈ own clones`, the same rule as the indexer's `scope.rs`), built from the operator's own config issuer-clones + custody signer accounts + the signer/clone/relayer addresses on its own records+sessions (zero address never widens scope). So even a mis-scoped indexer token can never leak another operator's event into a vet's view. The government passes `scope = None` (admits everything, unscoped). This defense-in-depth makes "a vet cannot fetch another vet's activity" testable at the role layer without a live indexer.
- **The DB-record join (`crate::trace`).** Each on-chain event is matched to the operator's own record: vet/groomer by anchored `root` (issuances/revocations) or verification `nullifier` / tx (verifications) → `Record`/`VerifySession`; government by `root` / tx → `IssuedCredential`/`VerificationRecord`. The matched record's non-PII summary is attached as the event's `local` field (`null` when on-chain activity has no local record — a drift signal that is still shown in-scope).
- **Endpoints.** Vet/groomer (operator-session gated, `public_router`): `GET /trace/activity` (scoped + gated + joined; envelope adds `inScope`/`matched`/`droppedOutOfScope`/`localScope`), `GET /trace/stats` (indexer scoped counters + own record/session counts). Government (`GOV_API_TOKEN` gated): `GET /v1/oversight/activity` (unscoped + joined; `matched` = how many cross-issuer events are the authority's own), `GET /v1/oversight/stats`, `GET /v1/oversight/issuers`. Unconfigured indexer → 503 `{indexer:"not-configured"}` (rest of backend unaffected).
- **Web.** New nav+route+page per app (Waypoints icon): the vet's `pages/Traceability.tsx` ("Traceability") uses the `@dogtag/ui` client — added `traceActivity`/`traceStats` + `Trace*` types to `packages/ui/src/api/{client,types}.ts`; government `pages/Oversight.tsx` ("Oversight") uses its local `lib/api.ts` (`apiGetResult` surfaces the 503 for a first-class "indexer not connected" state) + `VITE_GOV_API_TOKEN`. Each renders the joined feed with the local record highlighted, finality badges, and explorer links.
- **Config.** `INDEXER_API_BASE` for all three; vet/groomer add `INDEXER_SCOPED_TOKEN` (alias `INDEXER_TOKEN`), government adds `INDEXER_OVERSIGHT_TOKEN` (alias `GOV_INDEXER_TOKEN`). `scripts/demo-up.sh` starts the indexer (`INDEXER_DEMO_MODE=1`, `:46001`) and wires all three with the two well-known demo tokens. DEMO CAVEAT: the indexer's scoped demo token is bound to a FIXED stand-in signer/clone, so a freshly-genesis'd vet/groomer sees "0 in scope" (its local gate correctly rejects the demo-groomer's events) until its real signer is added to `INDEXER_SCOPES`; the government unscoped view always shows the full scripted cross-issuer feed.
- **Tests.** `stacks/vet/api/tests/trace.rs` (scoping: foreign vet excluded; join; auth; 503) + `crate::trace`/`crate::oversight` unit tests; `stacks/government/api/tests/oversight.rs` (unscoped feed sees all issuers, own highlighted, non-PII; auth; 503). Web e2e mirror the mocked-`/api/`-regex style: `stacks/vet/web/e2e/traceability.spec.ts`, `stacks/government/web/e2e/oversight.spec.ts` (not in CI — need a served portal). All 4 test constructors in vet `tests/common/mod.rs` + the 3 gov test `AppState` literals gained a `feed` field (default `DisabledFeed`; trace tests override `state.feed` with a seeded `MemFeed`).
- **Post-unification alignment (the PR-5 remainder, after the owner-hidden collapse).** Three joins the original PR-5 predates:
  (1) **Dog-tag mints** - the owner-hidden custodial issuance (`issue(R)` on `cfg.profile_issuer_addr` + `mintCustodial`) joins the vet's own `ProfileIssueSession` by anchored profile root / mint tx (`kind:"mint"`, via `Store::list_profile_sessions`); the profile clone is part of the local scope; `/trace/stats` adds `local.dogTagsMinted` (bound sessions only - an errored bind stores error TEXT in `tx_hash` and must never enter the tx join).
  (2) **Owner-blind `verified` payload** - `dogTagId` (field-hashed decimal), hashed `purpose`, proof-bound `deadline` are rendered on all three portals (`TraceEvent`/`OversightEvent` types); there is NO subject anywhere downstream, `deadline` replaced it.
  (3) **Government tag-granular join** - `verified` events carry no root, so gov credentials also index under `trace::onchain_dog_tag_id_decimal(handle)` (= decimal `U256` of `field_of_value(Integer(handle))`, matching the indexer's `U256::to_string()` rendering of the event topic; the stored `dog_tag_id` is the RAW operator-entered handle). A dogTagId hit is stamped `joinedBy:"dogTagId"` and labeled **"tag we credentialed"** - never "our credential was verified": the event binds no root/recordType, so which credential was verified is unknowable; `purpose` is the only disambiguator. Newest credential (by `created_at`) wins a tag's key. The join summary stays subject-less (no `owner_identity`/pet block on mint joins, no Section-A block on gov joins).

### Issuers / Factory deploy UI (PR-C: `admin-factory-ui`)
The web surface for the captain's "deploy contracts from our factory". A new **Issuers / Factory** nav item on the shared `@dogtag/ui` `AppShell` (`stacks/admin/web/src/pages/Issuers.tsx`, nav in `app/Layout.tsx`, route in `App.tsx`) — the first web page consuming the PR-A backend. No new backend; it is pure UI over the PR-A/PR-B endpoints.
- **Live deterministic address preview.** The Deploy dialog debounces (recordType, business) and calls `POST /v1/admin/factory/predict` → shows the exact CREATE2 clone address BEFORE committing (salt = `keccak256(recordType, business)`). `business` is optional; blank = the single-authority topology (backend defaults it to the hosted signer). A stale-response guard (`seq` ref) drops superseded keystrokes so the preview never flickers to an old address.
- **Deploy routes through the GovernanceAction layer — the web NEVER assumes the old EOA.** Submit calls `POST /v1/admin/factory/issuers`; the response `result.disposition` is either `executed` (hosted key IS the factory owner → real ROAX tx, shown with an `explorer.roax.net/tx/…` link) or `proposed` (ownership sits with the governance signer post Phase-2 → the `{target, calldata, holder}` payload is rendered for out-of-band execution, nothing broadcast). An **authority banner** at the top reads `GET /v1/admin/governance/authority` and tells the operator up-front which path a deploy will take ("Hosted key deploys directly" vs "Deploys route to governance as proposals"). This is why the tooling `ADMIN_PRIVATE_KEY` must be signer-1 `0x8E27E117…` post-handover — otherwise every deploy comes back `proposed` rather than executed.
- **Clone list is best-effort.** The table reads `GET /v1/admin/activity/issuers` (needs the oversight indexer); a 503/unwired indexer degrades to an inline "activity unavailable" note WITHOUT breaking deploys or the preview (those need only the chain). Client types + `predictIssuer`/`createIssuer`/`governanceAuthority`/`listIssuers` methods live in `packages/ui/src/api/{types,central}.ts`. Web has no unit suite — `tsc --noEmit` + `vite build` are the gates.

### Admin whitelist management console (PR-E: `admin-whitelist-mgmt`)
Promotes the read-only `stacks/admin/web` Whitelist viewer to a **direct grant/revoke management console** — the whitelisting machinery `approve_application` runs, exposed as a standalone control-plane action decoupled from the issuer-application queue (key rotation, ad-hoc grants, incident response). Web + backend; builds on PR-A's `GovernanceAction`.
- **Two new admin-gated endpoints (`routes.rs`, admin-session):** `POST /v1/admin/whitelist/grant` and `POST /v1/admin/whitelist/revoke`. Body `{ signer, recordType?, verifyPurposes? }` (at least one of `recordType`/`verifyPurposes` required — else 400; malformed signer → 400). **Grant** builds a `whitelistFor` `GovernanceAction` per capability (the `recordType` key via `to_record_type_key` + each `verify_key(purpose)`) and, for a `DOG_PROFILE` recordType, ALSO a `grantRole(ISSUER)` action on the SBT (idempotent: `has_issuer_role` pre-check → `{status:"alreadyHeld"}` when already held). **Revoke** builds `delistFor` per capability; it does NOT revoke `ISSUER_ROLE` or on-chain roots (that is a DEFAULT_ADMIN `adminRevoke`, a PR-F Governance action) — mirrors `delist_application` (delistFor only).
- **Everything routes through `governance::dispatch` (never the direct `whitelist_for`/`delist_for` path).** The whitelist capabilities are gated by `Authority::Role{registry, whitelist_admin_role(), default_admin:false}`; the DOG_PROFILE ISSUER grant by `Authority::Role{sbt, default_admin_role(), default_admin:true}` (the SBT is `AccessControlDefaultAdminRules`, so `defaultAdmin()` resolves the holder). Response: `{ signer, recordType, actions: [Disposition…], issuerRole?, outcome, executed, warning }` — each `Disposition` is `executed{txHash,holder}` (hosted key holds the role) or `proposed{holder,hostedSigner,target,calldata,authority}` (role moved to governance; `hostedSigner` names the key that was checked, so a wrong-key proposal is distinguishable from a designed one). So a grant/revoke flips executed→proposed by construction the moment WHITELIST_ADMIN leaves the hosted key (Phase-2), exactly like the factory deploy.
- **`outcome` is the request-level verdict, and it is TRI-state because "nothing was broadcast" has two meanings.** `governance::DispatchOutcome::classify` folds EVERY dispatched action (the whitelist capabilities **and** the separate ISSUER_ROLE action, so the not-one-tx claim can only be made when none landed) into `executed` (≥1 broadcast), `proposed_by_design` (nothing broadcast **and** the deployment declared propose-only via `ADMIN_PROPOSE_ONLY`/`ALLOW_UNAUTHORIZED_ADMIN_SIGNER` — a correct outcome, calm `warning`, never says the key is wrong), or `proposed_unauthorized` (nothing broadcast, not declared — the loud wrong-key `warning`). The boolean `executed` is retained for back-compat and `warning` is `null` only for `executed`. The declaration is a REPORTING input only: it never changes what is dispatched, and holdership is always read live from the chain.
- **Web (`pages/Whitelist.tsx`, now "Whitelist management"):** the derived + live-`isWhitelistedFor` state view is kept; each (recordType,address) row gains **Grant**/**Revoke** buttons behind a confirm dialog, plus a header **"Grant capability"** dialog for an arbitrary (signer, recordType, verifyPurposes) pair. Each dispatched capability renders inline as a `Disposition`: executed → `explorerTxUrl` link, proposed → holder + authority + truncated calldata. After an action the affected row re-reads on-chain. The result toast is driven off the response's `outcome` (green / amber / red) and never re-derives the case from `actions`. Client: `central.whitelistGrant`/`whitelistRevoke` (`packages/ui/src/api/central.ts`) + `GovernanceDisposition`/`WhitelistActionReq`/`WhitelistGrantResp`/`WhitelistRevokeResp` types. The nav label `Layout.tsx` changed `Whitelist viewer` → `Whitelist`.
- **Tests:** `tests/control_plane.rs` (7 new, MemChain): grant proposes when hosted lacks WHITELIST_ADMIN / executes all capabilities (recordType + 2 verify purposes) when it holds it; DOG_PROFILE grant also executes the ISSUER-role grant; revoke executes; requires-admin (401); missing-capability + bad-signer (400). Set the hosted role via `chain.set_role(REGISTRY, &whitelist_admin_role(), HOSTED)` / `set_role(SBT, &default_admin_role(), HOSTED)`. The tri-state `outcome` has its own cases in the same file: a landed tx outranks the declaration, a declared propose-only grant/revoke is `proposed_by_design`, and the "nothing was broadcast" warning fires only when neither a whitelist action nor the ISSUER_ROLE action executed.

### Vet/groomer verification audit history (verify2-s4)
The shared vet/groomer verifier flow now keeps a durable operator-visible audit history for owner-consent verification sessions, using the existing `VerifySession` rows instead of a parallel table. `VerifySession` carries `created_at`/`updated_at`; the status lifecycle is `pending` -> `recording` -> `recorded` or `error`. `GET /verify/history` is operator-gated and returns most-recent-first rows with purpose, recordType, relayer, status, txHash, explorerUrl, nullifier, and timestamps (the migration-era `mode` field is gone - there is one owner-hidden flow). It intentionally stores verifier operational proof metadata only, not credential PII. `packages/ui` exposes `verificationHistory()` plus `VerificationHistoryPanel`; both `stacks/vet/web/src/pages/Verify.tsx` and `stacks/groomer/web/src/pages/Verify.tsx` render it under the QR export flow. Hermetic coverage lives in `stacks/vet/api/tests/flow_memchain.rs::verify_session_status_polls_pending_to_recorded` and checks auth gating plus pending -> recorded history rows.

### Verifier direct credential status (issuer-c3)
The vet/groomer verifier product now has a direct, operator-facing **pasted credential check** in addition to the existing owner-consent proof-export flow. It is intentionally NON-admin-nav work.
- **Backend (`stacks/vet/api`)**: `POST /verify/credential` (plus `/v1/verify/credential` alias) is operator-session gated and non-persistent. Body `{ wrappedDoc, issuerAddr?, signerAddr? }`; it recomputes wrapped-doc integrity with `dogtag_standard::verify::check_integrity`, defaults the issuer clone from `wrappedDoc.issuer.documentStore`, reads `DogTagIssuer.issuedAt(root)`, `isValid(root)`, and `isRevoked(root)`, and optionally checks `IssuerRegistry.isWhitelistedFor(keccak256(recordType), signerAddr)`. Response includes `{verdict,status,recordType,root,recomputedRoot,issuerAddr,issuedAt,fragments}` where `status` is `valid|revoked|not_issued|integrity_failed|invalid`. The handler stores no pasted credential data, so no new PII store is introduced.
- **Chain surface**: `ChainClient` gained `is_revoked`; `AlloyChain` binds `DogTagIssuer.isRevoked(bytes32)` and `MemChain` reads the existing in-memory revoked map.
- **Web (`stacks/vet/web`, `stacks/groomer/web`)**: the shared `@dogtag/ui` `CredentialVerifyPanel` is mounted on each Verify page above `VerifyFlow`. It accepts wrappedDoc JSON, optional issuer signer, and renders pass/fail plus integrity/on-chain/issued/revoked/whitelist pillars with issuer/root details. **As of `webverify-n3` the panel no longer calls `POST /verify/credential`** - see the next section.
- **Tests/builds**: `stacks/vet/api/tests/flow_memchain.rs::full_issuance_share_revoke_flow` now proves issue -> direct verify valid -> revoke -> direct verify revoked over `MemChain`.

### Public-signal indices: ALWAYS via the named constants, never a literal (e9 E-1)

There is ONE live public-signal order - the frozen seven-signal consent vector:

`[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`

The named constants live in three mirrored files, one per language:
Rust `crates/dogtag-standard-rs/src/public_signals.rs` (module `public_signals::level_b` - the module name is kept because it mirrors the internal version key `dogtag-levelb/1` and the on-chain `P_*` constants; an internal identifier, not a product label),
Swift `apps/ios/DogTag/PublicSignalIndex.swift` (`PublicSignalIndex.ownerHidden`),
and Kotlin `apps/android/app/src/main/java/io/liberalize/dogtag/zk/PublicSignalIndex.kt` (a flat `PublicSignalIndex` object).
Rust (`public_signals::tests`), iOS (`PublicSignalIndexTests`) and Android (`PublicSignalIndexTest`) each guard their own constants against accidental drift.
The values were transcribed from `VerificationRegistryConsent.sol`'s `P_*` constants, which stay
the authority - but every one of those tests asserts LITERALS and never reads the Solidity, so a
contract-side change would not fail them; the two sides must be moved together by hand.

Historical note - why reads go through named constants at all: the retired owner-revealing circuit
emitted a same-width `[String; 7]` vector whose order DIVERGED from index 3 on (`subject`, then
`nullifier`, `keyHash`, `R`), so a literal-index mix-up was invisible to every compiler and produced a
plausible-looking field element instead of an error.
The canonical failure: reading `pubSignals[4]` as the nullifier under the consent order actually
yields `R`, so the phone polls `consumed(R)` - never set - and a verification that **succeeded
on-chain** hangs until timeout.
The divergent order is gone with its circuit (the retired `level_a` constant sets have been deleted
from all three files, and both apps bundle and resolve only the consent artifact set), but the
constants remain the only sanctioned way to read a signal.

### Superseded M-4 dual-route migration guard (historical only)

The route split below records transitional safety behavior; it is not product architecture to
preserve. The routes it describes have since been deleted: the sole submit route today is
`POST /v1/verify/consent` (one owner-hidden consent flow, no user-visible mode).

A verify session's `mode` (`store::VerifySession`, field since deleted) was `"normal" | "zk" | "levelb"`, validated at
`POST /verify/session/start` — an unrecognised mode was a 400, because it previously fell through the
`if mode == "normal" { .. } else { <Level-A ZK> }` dispatch into the Level-A branch, so a typo like
`"level-b"` silently produced a Level-A session.

- `POST /verify/consent/submit` + `/v1/verify/consent` — Level-A. **Refuses a `levelb` session.** The
  refusal reads the session's **STORED** `mode`, deliberately NOT the request body's `mode` override:
  the override legitimately picks normal vs zk WITHIN a Level-A session, but testing it would let a
  caller holding a `levelb` session's export token pass `"mode":"zk"` and reach the Level-A ZK branch
  anyway. `mode_override` must never be able to change WHICH LEVEL is served.
- `POST /verify/consent/levelb` — Level-B, operator-gated, entered cold (mints its own owner-blind
  audit row). `POST /v1/verify/consent/levelb` is the phone twin: same
  `require_operator_or_export_token` gate as the Level-A alias, same PEEK (`consume=false`) so a
  failed verification does not burn the owner's one-time token. **Refuses a non-`levelb` session**,
  and binds the proof's `purpose`/`relayer`/`recordType` to the session — the export token is a
  capability to spend the relayer's gas, so it must not fund an unrelated submission. A session-scoped
  call drives the session's OWN row rather than minting a second one.
  - **`recordType` is bound with the REDUCED keccak (`purpose_key`), not Level-A's raw `rt_key`.**
    `pub[5]` is a circuit output and so always `< r`, while a raw keccak may EXCEED r — comparing
    against `rt_key` would be a guard that can never fire, the same trap as the art9 constant. It is
    bound at all because `recordType` is prover-asserted rather than consent-signed, so nothing else
    pins it: unbound, the phone could prove a record type the operator's session never named, and the
    audit row would record the requested one instead of the proved one.
  - **The phone twin never consumes the token** (Level-A does). It is peeked and stays live until its
    600s TTL; replay is blocked instead by the **session status guard** — the row is persisted as
    `"recording"` before the broadcast is spawned, so a second submit against a settled session is
    refused. Sound only because a session OUTLIVES its token (sessions are never deleted; the store
    has no delete/GC path). Hence a named session that does not resolve **fails closed** rather than
    falling through to the cold path — `MongoStore::get_session` maps driver errors to `None`, so
    without that a DB blip would silently strip this route of every session-scoped guard, replay
    protection included.

Both refusals exist because the two routes read the SAME export token; without them a session's
token would be accepted by both, and an owner-hidden proof read with Level-A indices is exactly E-1.

### Superseded M-4 opt-in snapshot (historical only)

The "available, not default" design below is forbidden by the standing product model. Never preserve
or reintroduce it as a default, opt-in, mode, toggle, or user choice. The code it describes has since
been deleted: `convenience_claims_for_mode` collapsed into the single mode-free `app::convenience_claims`
builder, pinned to the unified internal version key (see "Discovery API + app anchor-validation").

`app::convenience_claims_for_mode` stamped `dogtag-levelb/1` + the owner-hidden registry **only** for a
session explicitly started with `mode="levelb"`. Every other mode, and every flow with no session,
still advertised the since-deleted `LEVEL_A_VERSION` + the retired registry. Flipping the DEFAULT was
the P-3 version stamp flip (M-5) and was deliberately not done in M-4. The two fields move together:
advertising `dogtag-levelb/1` beside the retired registry address made every validating app fail
closed with `RegistryMismatch`. (The flip has since completed and only the unified pair exists.)
- The migration-era rule "never add a third, level-neutral constant set" is moot now that the retired
  set is deleted: exactly one public-signal order exists (see "Public-signal indices" above).
- `crates/dogtag-prover-rs` cannot import the standard crate's constants (it pins ark 0.6 vs
  `dogtag-standard-rs`'s 0.5 and the two coexist only because ark types never cross the boundary -
  see its `Cargo.toml`). Its `Groth16Output` doc lists the order and must be kept in step by hand.

### Web credential verify is permissionless direct-to-RPC (webverify-n3)
Credential verification is permissionless + on-chain, so the web `CredentialVerifyPanel` reads the chain itself instead of the operator-gated `POST /verify/credential`. The server endpoint is retained (it may serve other callers) but the web panel no longer depends on it.
- **Where**: `packages/ui/src/wallet/verifyCredential.ts` `verifyCredentialOnchain(...)` is a byte-for-byte TS port of the Rust `verify_credential` handler's classification. It runs `@dogtag/standard` `checkIntegrity` (pure offline recompute) then reads `DogTagIssuer.issuedAt/isValid/isRevoked` (and optional `IssuerRegistry.isWhitelistedFor`) via viem `eth_call` over the public ROAX RPC (`roax` chain def, chainId 135, `https://devrpc.roax.net`). All chain reads use the **claimed** root (`signature.merkleRoot`); the recomputed root only populates the `recomputedRoot` display field. Returns the identical `VerifyCredentialResp` shape, so the result renderer is unchanged.
- **Reader injection**: reads go through an `IssuerChainReader` interface (default `roaxIssuerChainReader`); tests inject a fake. Hermetic coverage: `packages/ui/test/verifyCredential.test.ts` (needs the `vitest` devDep added to `packages/ui`; picked up by root `pnpm -r --filter "./packages/**" test`).
- **Fail-closed**: an RPC read error rejects (panel shows a toast); it is never silently treated as valid. This is deliberately stricter than mobile, which accepts-with-caveat when the chain is unreachable.
- **Selector gotcha (verified on live chain)**: the deployed `DogTagIssuer.isValid(bytes32)` selector is `0x6a938567` (keccak of the canonical sig; what viem/the Rust ABI bind). The web path uses this selector (matching the server), so it is the faithful on-chain check. Mobile once hard-coded a stale `0x6d04f0bc` in `apps/*/.../RoaxRpc` / `Net.swift`, which **reverts** on the deployment -> its on-chain isValid resolved Unknown and fell back to accept-with-caveat; as of `dogtag-mobilefix-s7` mobile DERIVES the selector on-device (`Keccak256`) instead, so both paths now bind `0x6a938567` - see the mobile-selector note at the top of this file.

### Owner-web consent-receipt renderer (govarch PR-6, `dogtag-receipts-r6`)
The pet-owner wallet's **Consents** surface (`stacks/owner/web` `/consents`, `/consents/:nullifier`) renders the owner's own consent history entirely on-device: held-credential `dogTagId` handles are field-hashed (`fieldOfValue(Integer(handle))`, see "dogTagId encoding" below) into topic filters for a direct `eth_getLogs` over `VerificationRegistryConsent`'s owner-blind `Verified` events - no backend, no server-side owner state, matching the wallet's zero-backend doctrine.
- **`recordType` is NOT in the `Verified` event** (owner-blind shape). Consumers that need it must read it back from the verifying tx's calldata: decode `recordVerificationZK(a,b,c,pub[7])` and take `pub[5]` (`stacks/owner/web/src/lib/chain.ts::fetchConsentRecordType`). Degrade a single unreadable tx to "unavailable", never fail the whole history.
- **Purpose/recordType labels**: the chain carries `keccak256(label) % r` (backend `verify.rs::purpose_key`); the renderer reverse-maps a known-label list (`src/lib/consents.ts`) and falls back to the hex. New portal purposes should be added to that list.
- **No revoke, ever**: consent is a point-in-time grant (captain 2026-07-21). The UI shows "Window open/closed" as history metadata; never add a cancel/revoke affordance, and print is the only export (private owner surface - no share path).
- **E2E mock discipline**: the chain has no `Verified` events yet, so `e2e/consentFixture.ts` is the functional oracle. Its bytes are real ABI encodings (generated via viem `encodeEventTopics`/`encodeAbiParameters`/`encodeFunctionData`, round-tripped through the decoders) - if the event or calldata shape ever changes, regenerate; do not hand-edit hex.

### Custody unlock happens AT THE POINT OF NEED (`dogtag-unlockpage-u7`)
A custody-gated action refused with `not unlocked` raises an unlock prompt IN PLACE over the page the operator is already on; the shared api client then REPLAYS the refused request, so a half-filled form is never discarded. The dedicated `/unlock` route remains as the fallback for arriving at an already-locked backend and as a direct link. Genesis stays in Setup.
Shared logic is in `packages/ui/src/custody/lock.ts` (pure, unit-tested in `packages/ui/test/custodyLock.test.ts`) plus `domain/CustodyUnlock.tsx` (`CustodyUnlockForm` + the `CustodyUnlockDialog` / `CustodyUnlockPanel` / `CustodyLockedBanner` wrappers, all router-free).

- **Replay is safe because of WHERE the backend checks.** Every handler tests `is_unlocked()` immediately after its auth check and BEFORE any store or chain access (`routes.rs` 374/464/718/1172/1587/1958, `verify.rs:260`), so a `not unlocked` refusal means nothing happened and re-sending cannot double-submit. Verify that placement before extending the retry to a new route.
- **Do NOT gate routes on the lock.** An earlier iteration redirected every non-exempt route and had to be withdrawn: the operator password and the custody-admin password are SEPARATE credentials (`OPERATOR_PASSWORD` vs `ADMIN_PASSWORD`), so a front-desk operator who lacks the custody passphrase was shut out of read-only work (records, traceability, verification history) by a lock they cannot clear. Read-only pages stay reachable; `CustodyLockedBanner` is the arrival-time signal.
- **`/admin/login` WIPES the rate limiter, so authenticate once per prompt, not once per attempt.** It ends in `record_success(&ip)` = `map.remove(ip)` (`auth.rs:274`), on the same limiter that guards `/admin/unlock`. Logging in before each passphrase guess clears the failure tally between guesses and the per-IP lockout can never trip. `CustodyUnlockForm` therefore reuses the held admin session and re-authenticates only on a genuine dead-session 401.

- **Setup owns genesis, not unlock.** `Setup.tsx`'s admin-login step routes a sealed-but-locked instance to `navigate(buildUnlockPath("/setup"))` instead of its old `setStep("unlock")` — that branch WAS the buried unlock the operator had to hunt for after every restart. The wizard's `confirm -> unlock` step is deliberately RETAINED: post-genesis the operator just chose the passphrase and the wizard needs custody open to derive accounts, so that one is genesis continuation, not an entry point. Both branches are easy to conflate; changing the wrong one either re-buries unlock or breaks first-time setup.
- **Only vet + groomer have a custody seal.** The government backend signs from an env-configured `GOV_SIGNER_KEY` (no genesis, no `/admin/unlock`) and the admin portal talks to the central client, so neither has anything to lock. Do not add an unlock route to those portals; check `grep -rn "admin/unlock" stacks/*/api/src` before assuming a portal is in scope.
- **`/admin/unlock` answers a WRONG PASSPHRASE with 401**, which collides with the shared client's stale-session hook: a typo would clear the admin token and show "Session expired" instead of "wrong passphrase". The exemption is keyed on the MESSAGE, not the path - `isWrongPassphraseError` (`packages/ui/src/custody/lock.ts`, consumed by `isCredentialRejection` in `api/client.ts`) - because that same route's admin gate raises `missing admin session` / `invalid admin session` with the same 401, and those ARE dead sessions that must still fire `onUnauthorized`. A future password-checking endpoint joins by having its rejection message recognised, not by adding a path.
- **Detect a lock by the MESSAGE `not unlocked`, never by status.** 409 is overloaded across the custody routes (`already initialized`, `not initialized`, `no pending genesis`), and the same "not unlocked" text also arrives as a 500 from `CustodyError::Locked`. Keying on 409 alone would read `/admin/unlock`'s own `not initialized` as "locked" and loop the operator back to `/unlock`.
- **The on-load probe is the existing `GET /issuer/signers`** — operator-gated and read-only, so no new endpoint. Locked (or seal-less) short-circuits to `{signers: []}`; unlocked returns `{activeSigner, matrix}`. Recognise BOTH shapes positively and leave anything else `unknown` - reading "no `activeSigner` key" as locked classifies every unrecognised payload as a lock, which is how the six `route.fulfill({ json: {} })` catch-all Playwright mocks in `stacks/{vet,groomer}/web/e2e/` end up redirected to `/unlock` instead of the page under test. Key `activeSigner` on PRESENCE, never truthiness: `active_address()` can return `""`, and a false "locked" traps the operator in a redirect loop while a false "unlocked" self-corrects on the first real request.
- **Never announce a lock on `unknown`.** `CustodyState` is tri-state and a failed probe (backend down) must stay `unknown` — a passphrase cannot fix an unreachable backend.
- **No operator-gated way to tell "no seal" from "locked"** before authenticating: only `POST /admin/login` reports `{initialized, unlocked}`. That is why the unlock page submits the custody-admin password and the passphrase together, and why it opens on neutral "Unlock custody" copy, claiming "Custody is locked" or "Custody not set up" only once the backend has said so.
- **With the Mongo store, op sessions SURVIVE a restart while custody re-locks** (`mongo.rs` persists `op_sessions`; `MemStore` does not). So in production a live action really does hit the "not unlocked" 409 mid-session — which is exactly the case the in-place prompt serves. Under the in-memory demo store a restart kills the session too, so the operator re-logs-in first and the banner comes from the post-login probe.
- **`sanitizeNextPath` must NOT decode.** Both call sites already hand it a decoded string (`buildUnlockPath` takes a live `pathname + search`; the unlock page reads the param through `URLSearchParams`). A second decode turns a destination's own escapes into structure — `%2B` became a literal `+`, `%26` split off a second query parameter.

### Calendar grids: never enumerate days by `+ 86_400` (`dogtag-calendar-c6`)
A local day is 23h or 25h across a DST transition, so `start + i * DAY_SECS` DRIFTS OFF LOCAL MIDNIGHT from the transition onwards. That silently breaks the calendar's day-bucketing, because the bucket KEY is `startOfDay(a.startAt)` (a true local midnight) while the seeded cell is the drifted value - `Map.get` misses and the booking vanishes from the grid with no warning. Measured: in the March 2026 month grid for `Europe/London`, 7 of the 42 cells land at 01:00 and a booking on Mon 30 Mar is dropped entirely. Use `lib/time.ts`'s `addDays` (steps the `Date` day component, which re-resolves the offset per day) for every grid cell AND for the window's `to` bound; `startOfWeek`/`monthGrid`/`addMonths` are all built on it. `addMonths` anchors on the 1st first, because `setMonth` on the 31st overflows and would skip February.

### Groomer portal page widths: `Layout wide` means "working surface" (`dogtag-calendar-c6`)
`stacks/groomer/web/src/app/Layout.tsx` takes `wide`, and it is NOT a bigger max-width - `wide` renders `w-full` (the shell's content column, whatever width that is) and the default renders `mx-auto max-w-5xl`. The split is by page KIND: a form or detail view is READ, so it keeps a reading measure; a booking list or calendar is WORKED IN, so it takes the width it has. Routes marked `wide`: `/calendar`, `/appointments`, `/clients`, `/verifications`. A form hosted ON a wide page (`ClientForm` inside `Clients`) caps itself with `max-w-3xl` rather than inheriting the list's width. Filter rows use `FilterBar`/`FilterField` from `app/crm.tsx` (a 12-column `xl` track); the point is that each date input gets its OWN column - an equal 4-up grid gave `From`+`To` a quarter-row between them, which is what "squeezed together" meant.

### `.ics` calendar interop: feed out, import in (`dogtag-calendar-c6`)
Full rationale in `docs/CALENDAR_SYNC.md`. The non-obvious parts:
- **The parser runs in the BROWSER, on purpose** (`packages/ui/src/calendar/ics.ts`). vet-api has no timezone database (no `chrono-tz`), and `TZID=Europe/London` cannot be resolved to an instant without one. `Intl.DateTimeFormat` resolves any IANA zone exactly, so the browser POSTs unix seconds and `POST /calendar/import` does NO timezone interpretation. Do not "move the parser to the backend for symmetry" - that is how you book appointments an hour off twice a year. The Rust side (`ics.rs`) only WRITES iCalendar, which is UTC-only and needs no database.
- **`Appointment.client_id` may legitimately be `""`** — an imported booking is UNASSIGNED, because a calendar invite names an event, not a DogTag client, and the import refuses to fabricate directory rows. Consequences: render it through `AppointmentClient` (`app/crm.tsx`), never `/clients/${clientId}`; `resolve_session_context` maps `""` to `None`; and `resolve_appointment_target` still 400s on an empty `clientId`, so an unassigned booking CANNOT be saved until a client is picked (`AppointmentForm` says so inline rather than showing a dead submit button).
- **Import dedup is by the source `UID`** (`Appointment.external_uid`, unique-partial-indexed in Mongo). Re-importing the same file updates rather than duplicates. `update_appointment` MUST carry `source`/`external_uid` through from the existing row, or an edit orphans the booking from its source event and the next re-import duplicates it. The merge rule: the file owns WHEN (start/end/label), the portal owns WHO and HOW FAR ALONG (client/pet/groomer/status).
- **The feed URL is a credential**: `GET /calendar/feed/{token}.ics` is UNAUTHENTICATED (a calendar client cannot present a bearer), so the 32-byte CSPRNG secret in the path is the whole gate — constant-time compared, 404 on any miss (never 401/403, which would confirm a feed exists). Revocable and rotatable from the portal.
- **`put_settings` is a whole-document replace.** `IssuerSettings` now carries `ics_feed_token` beside `signing_mode`, so any settings write MUST read-modify-write — constructing a fresh `IssuerSettings` revokes the shop's published feed as a side effect. Covered by `tests/calendar_ics.rs::switching_signing_mode_does_not_revoke_the_published_feed`.
- **Google two-way sync scaffolding already exists and does not work** (`calendar.rs` + `sync.rs` + `/calendar/google/*`): never exercised against real Google, and it mirrors `ApptReplica` (the central-pushed replica) rather than the `crm_appointments` the portal books into. `docs/CALENDAR_SYNC.md` §2 has the full list before anyone scopes it.

### dogTagId encoding (easy to get wrong)
The operator-facing **handle** is a small integer. The **on-chain** dogTagId minted into `DogTagSBT` and emitted as the circuit's `pub[0]` is the Poseidon **field-hash** of that handle: `routes::onchain_dog_tag_id(handle)` = `to_hex32(field_of_value(Integer(handle)))` (mirrors the `dog_tag_id_field_hex` FFI / `field-hash` bin). The SBT is keyed by the field element, NOT the raw handle — `ownerOf`/`profileRoot` lookups (and tests) must field-hash first.

## Deployment / production guards (fail-closed)
- Demo vs prod is gated by `DEMO_MODE` / `VITE_DEMO_MODE` (set = demo/local, unset = production).
- Both backends call `startup::validate_production_secrets(...)` at boot: in production they **refuse to start** if `OPERATOR_PASSWORD`/`ADMIN_PASSWORD`/`CENTRAL_HMAC_SECRET` (vet) or `ADMIN_PASSWORD`/`ADMIN_PRIVATE_KEY` (admin) are unset or equal to the known dev defaults. Set `DEMO_MODE=1` to keep the convenient demo defaults.
- vet-api: the consent prover behind `POST /prove-consent` is loaded **lazily on the first request** from `CIRCUITS_BUILD_DIR` and **fails closed per request** (503 on an unset dir, a missing artifact, or a hash mismatch); it never degrades to a stub or emits an unverifiable proof. (The old eager-boot `ArkProver`/`StubProver` pair was deleted with the retired prove path.)
- The prover **enforces a pinned zkey sha256** (the consent descriptor's pin, `f83a111f…` - see "Version-keyed proving artifacts"): loading rejects any zkey whose hash differs, so a swapped/corrupt key fails closed instead of proving against the wrong key (audit M4). The r1cs/wasm are pinned + verified the same way (before parse). A deployment shipping a **different** zkey (a production ceremony output) sets the `CONSENT_EXPECTED_ZKEY_SHA256` env var on vet-api (→ `load_versioned_with_expected_zkey`) - a config swap, not a code change. Leave it unset to enforce the bundled testnet hash.
- **Shared JWT signing key** (`SHARE_JWT_SIGNING_KEY`, 32-byte hex; vet + admin): the Ed25519 share/record JWT key. MUST be identical across restarts and horizontally-scaled instances or tokens break (audit L4). `load_jwt_keys` requires it (fail-closed) in production (same `DEMO_MODE` signal as the secret guard above), and uses an ephemeral key + warning in demo. `JwtKeys::generate()` alone is per-process/ephemeral — never the production path.
- **Admin password hashing** (`ADMIN_PASSWORD_HASH`, `"<salt_hex>$<hash_hex>"` from `auth::hash_password`; admin): the stored hash `admin_login` verifies against with `auth::verify_password` (audit L4 — replaces the old cosmetic plaintext compare). Optional; unset → the H2-required `ADMIN_PASSWORD` plaintext is hashed once at startup.

## Version-keyed proving artifacts (M7 brick 1)

Which files a prover loads, and the hashes it pins them to, come from a **version-keyed table** rather than hard-coded filenames.
This is the structure M7's fully-dynamic proving (lock C) plugs fetch into; the brick itself is additive and **fetches nothing**.

- **The model** (`crates/dogtag-prover-rs/src/artifact.rs`): a version key → `ArtifactDescriptor { version, circuit_id, num_public, public_signal_layout, zkey, r1cs, wasm, witness_graph, vk }`.
  `REGISTRY` holds every version this build can prove; `artifact::resolve(Option<&str>)` looks one up.
- **One entry**: `LEVEL_B_V1` (`"dogtag-levelb/1"` - the internal protocol version key, an internal identifier rather than a product label) - the owner-hidden consent set. Adding a version = adding a const + a `REGISTRY` line. (The retired `LEVEL_A_V1` entry and its `max_leaves` fixed-leaf-array machinery were deleted with the owner-revealing layer; consent folds depth-6 inclusion PATHS, so no leaf-width load guard applies.)
- **`resolve(None)` / `current()` ⇒ the consent set** - the sole registered version is the default for every caller naming no version.
  **A named-but-unknown version FAILS CLOSED** (including the retired `dogtag-levela/1` key, which this build no longer serves) - never fall back to the current artifacts, since a proof built with the wrong key is rejected by that version's verifier (a confusing failure far from the cause).
- **`zkey.sha256` is NOT the VK hash.** Two different things, deliberately two fields (M7 §3.2; the ZK cross-check calls out the conflation):
  `ZkeyArtifact::sha256` = the **fetch/integrity pin** of the proving-key file (hashed BEFORE parse, fail-closed, audit M4);
  `VerifyingKeyIdentity` = **which VK the proof verifies against** (authoritatively the on-chain `Groth16Verifier`, identified by address; `verification_key.json`'s hash is its off-chain identity). The prover never reads that file — the VK it proves with is inside the zkey.
- **Pins are checked facts, not decorative strings**: the consent-prove tests (`stacks/vet/api/tests/consent_prove.rs`) load the committed `circuits/build` artifacts through the fail-closed loader and assert the descriptor's pins against the real files, so a pin that rots fails a test rather than production.
  The zkey pin is mandatory (the type makes an unpinned zkey unrepresentable; the consent pin is `f83a111f…`); r1cs/wasm are pinned + verified before parse too. `witness_graph` is deliberately **unpinned** (`sha256: None`) — unlike the others it is NOT committed; CI fetches it from `DOGTAG_ARTIFACTS_URL`, so there is no byte-stable in-tree hash.
- **Loader shape**: `Prover::load_versioned(build_dir, descriptor)` is the real path, and `load_versioned_with_expected_zkey` overrides just the zkey hash (the `CONSENT_EXPECTED_ZKEY_SHA256` config swap). Both compose from one `load_inner`, so every entry point shares the fail-closed check.
  `load_inner` also **width-guards** the descriptor before any file I/O: this build formats a fixed `NUM_PUBLIC`-wide `pub` vector, so a version whose `num_public` differs is refused at load rather than surfacing as a truncated `pub` or an obscure witness failure. (The old convenience `Prover::load`/`load_with_expected_zkey` wrappers and the `max_leaves` leaf-width guard went with the retired fixed-leaf-array circuit.)
- **Service** (`stacks/vet/api`): `POST /prove-consent` is the ONLY prover route (`/prove-verification` was deleted with the retired circuit). The consent prover is loaded **lazily per request** from `CIRCUITS_BUILD_DIR`, cached (`version -> Arc<Prover>`), and **fails closed per REQUEST** (503 on missing dir/artifact/hash-mismatch) - see "Consent proving path (M7 P0)".
- **Mobile** (`apps/android/.../data/ZkeyAsset.kt`, `apps/ios/DogTag/ZkeyAsset.swift`): the same version-keyed resolver over the SAME bundled assets — Android copies from APK assets into `filesDir` (size-matched), iOS resolves from `Bundle.main`. Both registries hold exactly one descriptor: the consent set (`consent_final.zkey` + `consent.graph`), and `current()`/a nil version resolve to it; an unknown version fails closed. Bundled artifacts carry no hash: their integrity is the package signature's, not a runtime check.
- **The version key `"dogtag-levelb/1"` is declared three times** (Rust `artifact::LEVEL_B_V1`, Kotlin `ZkeyAsset.OWNER_HIDDEN_V1`, Swift `ZkeyAsset.ownerHiddenV1`) — one internal protocol constant, three languages, no shared source. A typo is a runtime rejection, not a compile error; `ZkeyAssetTest.ownerHiddenConsentArtifactsAreTheOnlyCurrentSet` pins the Kotlin side.
- **NOT here (deliberately)**: no network fetch. The consent descriptor is a real `REGISTRY` entry built from `circuits/consent.circom` with its own zkey + VK pins. The remaining gap is the fetch: every descriptor still resolves to a locally-present artifact.

## Consent proving path (M7 P0)

The code path that generates a **consent** ZK proof against the frozen `consent.circom` (`DogTagConsent(6)`).
Before this brick `prove_consent` existed **nowhere** — only the since-retired `prove_verification`.
It touches NO circuit/VK/ceremony (all frozen) and NO contract; it is a prover-path build to the existing key.

- **The assembler** (`crates/dogtag-standard-rs/src/consent_assemble.rs`, `assemble` feature): the ONLY circuit-input assembler in the crate (the retired `prover_assemble.rs` and the stale pre-consent `consent.rs` were both deleted with the owner-revealing layer; `consent_assemble` carries its own `fe_to_dec`). `assemble_consent(&ConsentWitness)` builds the per-tag tree (`build_profile_tree`), EdDSA-signs `M = Poseidon5(dogTagId, purpose, relayer, deadline, consentNonce)` with that tag's OWN per-tag consent key (derived from `(seed, dogTagId)` — see "M5 app-side" below), front-packs the three reserved-leaf inclusion paths into `siblings[6] + pathLen`, and emits the named inputs as `consent_input_map` (circom-prover / FFI) + `consent_circuit_input_value` (server JSON). Built from `consent.circom`.
- **THE canonical `dogTagId` field (load-bearing, ZK cross-check §2).** `assemble_consent` computes `field_of_value(Integer(handle))` **once** and uses that identical field element for BOTH (a) the circuit `dogTagId` input and (b) the `build_profile_tree` KDF binding that yields `R`; the on-chain `mintCustodial(id, R)` MUST use the SAME field as `id` (`ConsentAssembledInputs::dog_tag_id_field`). A mismatch fails closed at `R != profileRoot(dogTagId)` — a maddening liveness bug, never a safety hole. **Do NOT copy the fixture/`DeviceRootFixtureWitness` raw-`424242n` shortcut into issuance** (`profile_tree.rs:279-287`); the round-trip test `canonical_field_is_used_across_circuit_input_kdf_and_mint_id` + the fail-closed `raw_handle_shortcut_breaks_the_r_binding_fail_closed` pin this.
- **`nullifier` (`pub[3]`) and `R` (`pub[4]`) are circuit OUTPUTS** — the assembler recomputes them only for on-chain wiring / test assertions; it never feeds them in. `ownerAddress` is the **raw** reserved-leaf field (`field_from_scalar_bytes(addr)`), not `field_of_value`.
- **FFI** (`prover_ffi::prove_consent`, `prover` feature): the sole proving export (the retired `prove_verification` is gone), on the **circom-witnesscalc GRAPH backend** (kept over rust-witness/wasm2c, which miscompiles i64 field math on 32-bit ARM). Takes the owner seed + disclosed params + `zkey`/`graph` paths; `NUM_PUBLIC_CONSENT = 7`; returns `pub` in the FROZEN OUTPUT order `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`. Param parsing is split into `parse_consent_ffi_inputs` (hermetically unit-tested).
- **Backend** (`crates/dogtag-prover-rs`): `LEVEL_B_V1_DESCRIPTOR` in the version-keyed `REGISTRY`; `ConsentProveInputs` + `Prover::prove_consent_inputs` push the consent signal names by name (`push_consent_inputs`) and self-verify + format the output. The self-verify against the zkey's embedded VK IS a verify against the frozen consent VK (zkey sha256 `f83a111f…`, its exported VK json `27879dd7…`). (The retired `ProveInputs`/`Prover::prove` fixed-leaf-array path and the `prove-stdin` bin were deleted with the owner-revealing circuit.)
- **Service** (`stacks/vet/api`): `POST /prove-consent` - the ONLY prover route - selects the version-keyed consent artifact via a **lazy per-request** `ConsentProver` (`prover.rs`): loaded on first request from `CIRCUITS_BUILD_DIR`, cached (`version -> Arc<Prover>`), **fail-closed per REQUEST not boot** (503 on missing/hash-mismatch) so a misconfigured prover never blocks the rest of the backend (M7 §3.5). The device assembles the `circuitInput` **on-device** (cheap field math) and POSTs it; only the heavy Groth16 prove runs server-side. **State the threat model when describing this route's privacy:** the wallet SEED never reaches the server (so the operator cannot reach the owner's other tags or forge future consents), and owner-unlinkability holds against a chain observer and against the relayer - but the POSTed `circuitInput` carries `ownerSecret` AND `ownerAddress` (`consent_assemble.rs:245-246`), so it does **NOT** hold against the prover operator, which can name the owner and link that tag's entire verification history. `docs/MOBILE_OWNER_SECRET.md` marks `ownerSecret` "Never transmit"; this route is the one deliberate exception, kept for devices that cannot prove locally. On-device proving leaks none of it. (The bare claim "preserves owner-unlinkability" was wrong here and in `prover.rs` - e9 E-2.)
- **The ground truth** is `stacks/vet/api/tests/consent_prove.rs` (`--features prover`): the REAL Rust assembler → `prove_consent_inputs` → verify vs frozen VK → assert the 7 signals in frozen order + `pub[0]==canonical dogTagId` + `pub[4]==R`. It runs against the committed `consent_final.zkey`/`consent.r1cs`/`consent.wasm` (~2 min real prove; self-skips if absent). `consent_prove_parity.rs` verifies the FFI GRAPH proof vs the frozen VK json — run it through `make test-consent-parity`, never the bare cargo command, and note it is **operator-invoked, not CI-run**: no workflow fetches `consent.graph` (see the entry under "Build & test"). `contracts/test/consent-fixture.json` is regenerated to bind the CANONICAL field (`gen-consent-fixture.mjs` no longer uses raw `424242n`); `forge test --match-contract ConsentRegistry` (16 tests) verifies it on-chain.

## Record provenance block (M7 brick 2 / P2)

Every credential record is stamped with which protocol/contract it was created on AND who issued it, carried **BESIDE `R`, never inside `R` or the ZK proof** (M7 §4.2). Additive + back-compat: pre-M7 records have no block and stay verifiable via a defaulted provenance. This touches NO circuit/VK/ceremony and does NOT change `R` or `profile_tree.rs`.

- **The type** (`crates/dogtag-standard-rs/src/wrap.rs`, mirror `packages/dogtag-standard-ts/src/types.ts`): `ProtocolMeta { chainId, version, verificationRegistry, issuerClone, issuerSigner }`, added as `WrappedDoc.protocol: Option<ProtocolMeta>` (`#[serde(default, skip_serializing_if)]` / TS `protocol?`). Absent-by-default so existing docs/fixtures load unchanged; there is no golden WrappedDoc JSON fixture, so an optional field breaks nothing.
- **TWO `version` strings, do not conflate**: the block's `version` is the internal protocol version key `"dogtag-levelb/1"` (`LEVEL_B_VERSION` const in `dogtag-standard-rs` - the SOLE version constant since the retired `LEVEL_A_VERSION` was deleted; the TS `ProtocolMeta.version` string carries the same key); the envelope `WrappedDoc.version` is `"dogtag/1.0"`. Do NOT source the block version from `dogtag-prover-rs`'s `LEVEL_B_V1` (that's the artifact key - same string, different concern). Vet and government stamp `LEVEL_B_VERSION` at issuance; the admin import path assigns the same single version when projecting an unstamped doc into columns.
- **Provenance is a routing HINT, never authority.** `issuerSigner` is the envelope's *claim* of who issued; it is validated against the on-chain `DogTagIssuer.issuedBy[R]` (`mapping(bytes32=>address) public issuedBy`, set to `msg.sender` in `issue()`). The SDK `verify()` (Rust + TS, kept mirror-symmetric) adds an issuer-signer check via an **optional/default adapter method** (`RpcAdapter::issued_by` returns `Err` by default; TS `issuedBy?`): a present block whose claim != on-chain `issuedBy[R]` -> issuance `INVALID`; **skipped when the block is absent OR the adapter is unwired** (base validity governs). It can only make verification STRICTER - validation ALWAYS re-derives against `doc.issuer.document_store`, NEVER the untrusted block, so a forged block can neither reroute validation nor make an invalid record verify. Property pinned by `verify::tests::provenance_*` (Rust) + the TS `verify() M7 provenance` cases.
- **Populated at issuance, mirrored to queryable columns** (persist, don't just transmit): vet `Record`, gov `IssuedCredential`, and admin `Pet`+`Credential` gain `chain_id`/`protocol_version`/`verification_registry`/`issuer_signer` (all `Option`, `#[serde(default)]`; whole-struct serde/BSON makes them a transparent Mongo migration). **Admin also gains `issuer_addr`** (the issuerClone) - it previously carried provenance ONLY inside the encrypted `sealed_doc`; this closes that gap.
- **Where `issuerSigner` comes from per stack** (the honest claim, sourced from the issuer's OWN signer knowledge - never by reading `issuedBy[R]` and copying it, which would make the claim un-falsifiable): gov/admin backend-sign, so it's `chain.signer_address(...)` known at issue. **Vet's is POST-CONFIRM**: at prepare the block's `issuerSigner` is left `""` (wallet mode never learns the signer at build time); at confirm vet derives it from the `RootIssued(root, by)` log (the value it already stores as `signer_address`) and patches both the `issuer_signer` column AND `wrapped_doc.protocol.issuerSigner` (the block sits outside `R`, so patching never perturbs the root).
- **Back-compat default (§4.4) - DELETED (decision D5).** `WrappedDoc::resolved_protocol` / TS `resolvedProtocol` defaulted an absent block to the retired protocol generation; both were deleted in the final cleanup slice because the testnet is disposable and was redeployed fresh (2026-07-23 r8), so no pre-unification record survives to need the default. A stamped block is read as-is; the admin import path assigns an unstamped doc the single owner-hidden version/registry inline (`stacks/admin/api/src/routes.rs` - there is no retired generation left to route to).
- **Config**: gov + admin read `verification_registry_addr` (env `VERIFICATION_REGISTRY_ADDR`, default = the owner-hidden `VerificationRegistryConsent` `0xaBFd6f6E31780EBcB7ABd28A2a9bCfc9C8e6A77B`) for unstamped imported-document metadata; vet's routing key is `VERIFICATION_REGISTRY_CONSENT_ADDR`. Admin also gained a `chain_id` Config field (mirrors the chain client's id).
- **Deferred to M7 P5 (noted, not built here)**: live per-backend `issuedBy` eth-calls (vet ABI/`ChainRpcAdapter`, gov/admin `chain.rs`) so the backends enforce the signer check end-to-end, and the full §4.3 resolution loop (recognized-trio validation of `verificationRegistry`/`issuerClone` against a discovery anchor - that's what makes a forged *registry/clone* safe; P3/P4). This brick ships the envelope block + columns + default + the SDK-enforced signer property only.

## ProtocolRegistry discovery anchor + signed-manifest fallback (M7 P3)

The dogtag-governed discovery TRUST ANCHOR (M7 §5.1, lock B): a small read-mostly contract recording, per protocol version, the trio + verifier and the proving-artifact fetch-pins, so an app can validate a platform's version CLAIM against a dogtag-owned root of truth. **Additive** - it references the existing trio/verifier addresses but deploys/changes nothing else; NO circuit/VK/ceremony change. The resolve-GET extension + app-side validation are **P4** (not here); federation contracts are **P5**.

- **TWO INDEPENDENT VERSION AXES (R-5) - the load-bearing shape of this contract.** The registry keys the on-chain contract set and the off-chain proving artifacts on SEPARATE, separately-rotatable axes. This was decided before the first `versionId` was ever published, precisely because adding a second axis afterwards would mean re-keying every published record.
  - **On-chain axis** - `struct ContractSet`, keyed by `contractSetId` (== `keccak256(bytes("dogtag-levelb/1"))`): `factory`/`verificationRegistry`/`sbt` (trio), `verifier`, `circuitId`, `publishedAt`, `active`. Mapping `contractSets` + enumerable `contractSetList`.
  - **Artifact axis** - `struct ArtifactSet`, keyed by `artifactSetId` (== `keccak256(bytes("dogtag-levelb-artifacts/1"))`, a DELIBERATELY different namespace so the two axes never collide): the four `*Sha256` fetch-pins, `artifactBaseUrl`, `minAppVersion`, `publishedAt`, `active`. Mapping `artifactSets` + enumerable `artifactSetList`.
  - **The binding** - `activeArtifactSetOf[contractSetId] -> artifactSetId` is the ONLY link; neither struct references the other. Rotating a zkey is `proposeArtifactSet` + `proposeArtifactBinding` (then the executes): NO `ContractSet` is written, so no trio address moves and nothing is redeployed. Rotating the trio is one `ContractSet` publish and leaves every artifact record and the binding untouched.
  - **`minAppVersion` sits on the ARTIFACT axis** because the app gate is a property of the proving artifacts an app must be new enough to LOAD, not of the deployed contracts.
  - The registry stores data and never asserts a zkey proves against a given `verifier` (pins are byte-integrity, the verifier is VK identity) - that compatibility is a governance judgement, which is exactly why the binding is timelocked.
  - Independence is enforced by tests, not just convention: `test_artifact_rotation_leaves_the_contract_set_untouched` / `test_contract_rotation_leaves_the_artifact_set_untouched` / `test_the_two_axes_do_not_share_a_keyspace` in `ProtocolRegistry.t.sol`, mirrored in Rust by `an_artifact_rotation_conflicts_only_on_the_artifact_axis` / `a_trio_rotation_conflicts_only_on_the_contract_axis` (`manifest.rs`) and `an_artifact_rotation_leaves_the_onchain_axis_of_the_anchor_untouched` (`stacks/vet/api/tests/discovery_validation.rs`).
- **On-chain VK identity vs fetch pins - DO NOT conflate (§3.2).** `verifier` (an ADDRESS) is the on-chain VK identity; `zkeySha256`/`witness*Sha256` are byte-integrity FETCH pins. The `verification_key.json` file hash (`27879dd7…` for the consent set; the retired circuit's VK json is gone from the tree) is the OFF-CHAIN VK identity and is DELIBERATELY NOT an on-chain field - it lives only in the signed manifest. `witnessMobileSha256` (the `.graph`) is published as `0`/unpinned, matching `artifact.rs`'s `witness_graph.sha256 = None`. The two are in DELIBERATE LOCKSTEP: the descriptor field feeds `Manifest.witness_mobile_sha256`, and `manifest::reconcile`'s `cmp_opt` treats manifest-`Some` against on-chain-`None` as a CONFLICT, so flipping one side alone makes every reconcile report a phantom disagreement. The graph IS now committed and its bytes are attested in-repo by `artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256`; publishing that hash on-chain is a single atomic step (descriptor + chain together) documented in `docs/ARTIFACT_PIN_RUNBOOK.md` and is the operator's to run.
- **Timelocked publish, immediate deprecate - on BOTH axes and on the binding.** Each write is `propose…` → (immutable, deploy-time `PUBLISH_TIMELOCK`) → `execute…`, MIRRORING `VerificationRegistryConsent.proposeZkVerifier`/`executeZkVerifier` (a fresh propose resets the ETA; execute stamps `publishedAt=block.timestamp`+`active=true` and appends to the list only on FIRST publish so a swap-republish never dups): `proposeContractSet`/`executeContractSet`, `proposeArtifactSet`/`executeArtifactSet`, and `proposeArtifactBinding`/`executeArtifactBinding`. `DEFAULT_PUBLISH_TIMELOCK` remains 2 days. `DeployProtocolRegistry.s.sol` uses that default and rejects any other value unless `TESTNET_DEPLOY=true`; ROAX deliberately pairs that opt-in with `PUBLISH_TIMELOCK_SECS=0`, while mainnet must leave the opt-in unset and use exactly 2 days. The binding is timelocked because it is the pointer an app follows to decide which bytes to fetch - a one-transaction repoint is exactly the attack the window exists to catch on production.
- **The binding's published-ness AND active-ness are checked at EXECUTE, not propose.** Both axes must be published and `active` at the moment of execute (`unknown`/`inactive` are distinct revert reasons), which is both stricter (a set deprecated during the window cannot slip through - deprecate is the emergency lever, so a stale proposal must not be able to bind a just-retired set) and what lets the FIRST rollout run all three timelocks CONCURRENTLY - propose the sets and the binding together, wait once, then execute sets-then-binding. So publishing is still a two-phase script.
- `deprecateContractSet`/`deprecateArtifactSet` flip `active=false` immediately (a safety lever), NEVER delete the published record - history stays pinned so old records self-route (§7.3). Each axis is an independent lever: retiring a compromised artifact set does not touch the trio, and the app still stops (both `active` bits must hold - they are carried separately to the app and required jointly by `validate`). **Deprecate ALSO cancels any in-flight proposal for that id** (`delete _pending*[id]` + the ETA), which is what makes it a true EMERGENCY HALT: without it, a swap proposed before the compromise was found - and whose configured timelock had already elapsed - could be executed afterwards and flip `active` straight back with no fresh review window. That bites hardest on the artifact axis, where `activeArtifactSetOf` still points at the retired set, so a re-activation would instantly restore compromised artifacts as live for every bound contract set. Re-publishing after a deprecate therefore costs a fresh propose + the full configured timelock. Cancelling a PENDING proposal is **not** deleting history - the published record and its `*List` entry are untouched. Pinned by `test_deprecate_{contract,artifact}_set_cancels_an_in_flight_proposal` + `test_republish_after_deprecate_requires_a_fresh_timelock`.
- **Resolvers read the axis they belong to.** `getContractSet(id)` is for anything checking trio addresses / verifier / circuitId; `getArtifactSet(id)` and `getActiveArtifactSet(contractSetId)` (which follows the binding) are for anything fetching a zkey or enforcing `minAppVersion`; `resolve(contractSetId)` returns both halves in one call. All fail closed on an unknown id, and artifact resolution fails closed with `no artifact binding` when a contract set has no artifacts bound yet (a valid intermediate rollout state).
- **Pins are single-sourced from `crates/dogtag-prover-rs/src/artifact.rs`** (the consent descriptor; file-verified against `circuits/build/*` by the fail-closed loader plus the consent-prove tests, which stream-hash even the ~25 MB zkey). `contracts/script/ProtocolVersions.sol` reuses the SAME hex (the DRY source both the publish script AND `ProtocolRegistry.t.sol` import). The Solidity test can only re-hash the ~4 MB `.wasm` in-EVM (`vm.readFileBinary` MemoryOOGs at the 11+ MB r1cs / ~25 MB zkey), so the big-file pins are verified by the Rust tests - that split is intentional, not a gap. `foundry.toml` gained a `../circuits/build` read permission for the wasm hash.
- **Signed-manifest fallback (1B)** = `crates/dogtag-prover-rs/src/manifest.rs`: an **ed25519** dogtag-key-signed JSON of the SAME version content (§5.2 TRUST tier), built DRY from the artifact descriptor + `VersionDeployment` (the on-chain axis: trio addresses) + `ArtifactRelease` (the artifact axis: artifact-set name, base URL, `minAppVersion`; `artifact_release_for(version)` is the off-chain mirror of `activeArtifactSetOf`). The manifest carries BOTH axis identities (`version_id` and `artifact_set_id`). It is a CACHE/FALLBACK, never a second authority: `reconcile(signed, pinned_pubkey, &OnchainContractSet, &OnchainArtifactSet)` takes the two axes SEPARATELY (they are read separately on-chain, and a caller must be able to reconcile against a freshly-rotated artifact set without re-reading the contract set), verifies the sig, then returns the ALWAYS-on-chain authoritative fields of both + any `FieldConflict`s - **on conflict, on-chain wins**. `verify()` checks against the PINNED pubkey (not the envelope's advertised key), so a wrong-signer/tampered manifest fails. Real anchor is a compile-pinned `DOGTAG_MANIFEST_PUBKEY` (`None` until go-live). Served by vet-api `GET /protocol/manifest?version=…` (`stacks/vet/api/src/protocol.rs`; `version` is a query param because it contains `/`; key from env `DOGTAG_MANIFEST_SIGNING_KEY`, unset ⇒ 503; a NEW route that does NOT touch the resolve GET).
- **Publish scripts** (`DeployProtocolRegistry.s.sol` + `PublishProtocolVersions.s.sol`): deploy is admin/publisher = governance `0x8E27E117…`; `PUBLISH_TIMELOCK_SECS` defaults to 2 days and `TESTNET_DEPLOY=true` is the mandatory loud opt-in for any non-default delay. Publish is TWO phases (`…Propose`, then `…Execute` at/after the printed ETAs), reading the registry address from `PROTOCOL_REGISTRY`. `ProtocolVersions.sol` now authors the single owner-hidden `dogtag-levelb/1` version as THREE values - `levelBContracts()`, `levelBArtifacts()`, and the binding - so the FIRST publication uses the two-axis scheme from the start (there is nothing to migrate). Only `dogtag-levelb/1` publishes (the retired owner-revealing `dogtag-levela/1` is no longer authored here), with the confirmed roax.json trio addresses + the frozen pins. The registry is deployed + published on ROAX as of the 2026-07-23 r8 fresh redeploy: `ProtocolRegistry` `0xf5492A671E69b1A13f7Fd123C021830eB1ea8081` (`deployments/roax.json`), zero-timelock testnet opt-in, with the one version + artifact set + binding executed and `active` on both axes - see `docs/PROTOCOL_REGISTRY_RUNBOOK.md`.

## Discovery API + app anchor-validation (M7 P4)

The client TRUST gate on top of P3 (M7 §5.2 / §5.3): the resolve GET now also returns the CONVENIENCE tier (platform CLAIMS), and the app validates those claims against the P3 anchor (ProtocolRegistry / signed manifest) before trusting any platform-supplied version/registry. **Additive** - no circuit/VK/ceremony/contract change; P3's `ProtocolRegistry`/`manifest.rs`/`protocol.rs` are untouched.

- **The validator lives in the STANDARD crate, not the prover crate** (`crates/dogtag-standard-rs/src/discovery.rs`, `pub fn validate(claims, anchor, ctx)`). This placement is deliberate and load-bearing: the mobile app links `dogtag-standard-rs` (the UniFFI surface) but NOT the ark-heavy `dogtag-prover-rs` (server-only; deliberate ark 0.5/0.6 isolation), so the validator "the app path calls" MUST be here. It is PURE (string / int / dotted-numeric-semver compare, no ZK, no chain I/O, no signature check), so it is unit-testable independent of the mobile UI and reused by the server. RESOLVING the anchor (the `getContractSet`+`getActiveArtifactSet` eth-calls, or the ed25519 manifest verify + reconcile in P3 `dogtag_prover::manifest`) is the CALLER's job; `validate` checks the ALREADY-resolved anchor.
- **Types**: `ConvenienceClaims` (serde camelCase - the exact nested `unverifiedClaims` block the resolve GET emits AND the struct the app deserializes; also a `uniffi::Record`), `TrustedAnchor` (the resolved trust tier, built from plain fields so the app maps it from a `getContractSet`+`getActiveArtifactSet` result / parsed manifest and the server maps it from prover types; it carries BOTH axes - `version`/`version_id` and `artifact_set`/`artifact_set_id`), `ClientContext { app_version, expected_purpose }` (a borrow struct - never crosses FFI), `ValidatedVersion` (carries `version` + `circuit_id` for §5.3-step-6 artifact selection via `dogtag_prover::artifact::resolve`, plus the validated `artifact_set` - the artifact axis is returned separately because it moves separately). Thin FFI wrapper `validate_discovery(...)` flattens `DiscoveryError` into the crate's single `FfiError`. The committed UniFFI bindings (`apps/ios/DogTag/dogtag_standard.swift`, `apps/android/app/src/main/java/uniffi/dogtag_standard/dogtag_standard.kt`) WERE regenerated for these exports, so the app can call `validateDiscovery` - mandatory, because Android CI bundles the committed `.kt` as-is and never regenerates it (a stale binding makes the validator silently uncallable).
- **Fail-closed checks** (each returns `Err`, the caller aborts): version-coherence, artifact-axis coherence (`ArtifactSetIncoherent` - the anchor's `artifact_set` must hash to its `artifact_set_id`; the platform never claims the artifact axis, so this is the same caller-integrity guard applied independently to the second axis), BOTH `active` bits (deprecating either axis refuses the version - anti-downgrade §8.4), `chainId`, `verificationRegistry` (case-insensitive address - THE anti-redirect trip), `purpose`, and `minAppVersion`. The two `0x`-hex FIELDS (`versionId`, `verificationRegistry`) compare **case-insensitively** so an app formatting an `eth_call` bytes32/address as uppercase hex is not hard-failed; the `protocolVersion` and `purpose` strings are semantic, not hex, so they stay exact compares. **`minAppVersion` is compared NUMERICALLY, not lexically** (`1.10.0` > `1.9.0`; hand-rolled `[u64;3]` component compare because the workspace has no semver crate), fail-closed on malformed input.
- **`purpose` is checked against the app's OUT-OF-BAND intent (`ClientContext::expected_purpose`), NOT a chain field.** Neither on-chain struct carries a purpose (purpose is per-verification, not per-version), so there is nothing chain-derived to compare against - the anchor cannot supply it. The expected purpose is the user/app's own intent for the scan and MUST come from a source independent of the platform's claim (comparing the platform's claim against the platform's own session data would be vacuous). This is a consent-integrity check complementary to the registry/chain anti-redirect checks.
- **The convenience tier is on BOTH resolve GETs** (`stacks/vet/api/src/routes.rs` `export_session_resolve` `/x/` + `profile_bind_resolve` `/p/`), as an additive nested `unverifiedClaims` block (the pre-existing top-level fields were unchanged - back-compat; `/p/` has since additionally gained the `pet` + `ownerIdentity` containers of the mobile issuance contract - see `profile_bind_resolve` and implementation §3.11). Built by `app::convenience_claims`. `/x/` (verify) uses `VerifySession.purpose`; `/p/` (issuance) has no stored purpose so it uses the record type `DOG_PROFILE` as the app-knowable namespace (not fabricated).
- **The version-coherence check is a CALLER-INTEGRITY guard, NOT the downgrade defense.** The caller resolves the anchor BY `claims.protocolVersion`, so a platform that merely CLAIMS an older version resolves that version's own legitimate record and coheres with it; the check only catches "the caller resolved the wrong anchor". The app is version-AGNOSTIC **by design** (lock C: nothing bundled - it discovers the version), so there is deliberately NO `expected_version` to pin against - adding one would contradict the architecture. The version-DOWNGRADE defense is therefore **OPERATIONAL**: dogtag MUST `deprecateContractSet` superseded versions in the `ProtocolRegistry` so the validator's `active` check rejects them, backed by `minAppVersion` enforcement (moot for the retired generation - its `dogtag-levela/1` key was never published and is no longer authored, so only `dogtag-levelb/1` will ever be on the registry at go-live; the rule binds any FUTURE superseding version). Publishing a superseded version as still-`active` silently permits a downgrade onto it.
That operational lever is now **WIRED, not merely documented**: `active` is a real member of BOTH `dogtag_prover::manifest::OnchainContractSet` and `OnchainArtifactSet` (mirroring the two Solidity `active` bits) and both ride through `reconcile` into `Reconciliation::contract_set`/`::artifact_set`, from which `anchor_from_reconciliation` SOURCES them into the two SEPARATE `TrustedAnchor` bits `contract_set_active`/`artifact_set_active` - passed through, NOT AND-ed there; `validate` requires both, so deprecating EITHER axis fails closed with `DeprecatedVersion` naming the axis that fired.
`reconcile` deliberately does NOT compare `active` against the manifest (the signed manifest carries no lifecycle bit, so a disagreement is impossible by construction, not suppressed); it is a pass-through whose only attestation is the chain.
P4 still does NOT add a live Rust `ProtocolRegistry` eth-call reader (P3 deferred that) - the app reads `getContractSet`/`getActiveArtifactSet` natively into the **two separate** `TrustedAnchor` bits `contract_set_active` (Swift/Kotlin `contractSetActive`) and `artifact_set_active` (`artifactSetActive`) - but the bits now flow end-to-end through the reconcile/mapping path, so the defense is enforceable from both the app path and the server path.
**Wire BOTH, never one, and never pre-AND them into a single field.** `validate` requires both and is the only place the pair is enforced, so an implementer who sets `contractSetActive = contractSet.active` and leaves the artifact bit `true` silently discards the artifact-axis kill switch - `deprecateArtifactSet` on a compromised zkey would then not stop the app, defeating the headline R-5 property that each axis is an independent lever. The refusal names which lever fired (`DiscoveryError::DeprecatedVersion { axis: DeprecatedAxis::ContractSet | ArtifactSet }`, rendered into the message so it survives the FFI's flattening to a string), because the remedy differs: a retired contract set means the whole version is gone, a retired artifact set means only the artifacts were pulled and a newer set may already be published for the same version.
End-to-end coverage: `onchain_deprecation_flows_through_reconciliation_and_fails_closed` in `stacks/vet/api/tests/discovery_validation.rs`.
- **`convenience_claims`' pinned `protocolVersion` and the config `verificationRegistry` MUST move together.** `app::convenience_claims` (`stacks/vet/api/src/app.rs:123`) is a single mode-free builder pinned to the unified internal version key (`LEVEL_B_VERSION`) while `verificationRegistry` comes from `VERIFICATION_REGISTRY_CONSENT_ADDR` (mirroring the `protocol_meta` pattern). Pointing the env at a different registry WITHOUT moving the version constant emits an internally incoherent claim pair, and every validating client trips `RegistryMismatch`. Fail-closed (safe), but it breaks the flow - flip both in the same change.
- **`minAppVersion` tolerates a prerelease/build suffix** (`1.4.0-rc1`, `1.4.0+build.5`): `parse_semver` strips everything from the first `-`/`+` and compares the numeric `major.minor.patch` core, so real mobile builds are not locked out of verifying. Still fail-closed `BadSemver` on a non-numeric/empty core (`1.x.0`, `-rc1`, `1..0`, `1.4.0.1`). Prerelease ORDERING is deliberately not modelled - a `-rc` build counts as its release core.
- **Server-side trust-tier mapping** (`stacks/vet/api/src/discovery.rs`): `anchor_from_manifest(&Manifest, contract_set_active, artifact_set_active)` / `anchor_from_reconciliation(&Reconciliation)` map the prover-crate manifest types into `TrustedAnchor`. Note the asymmetry: the ONLINE reconcile path takes NO lifecycle params (it sources both bits from the on-chain records and passes them through SEPARATELY - it does not AND them; `validate` owns the enforcement - see the downgrade-defense note above), while the OFFLINE manifest-only path still takes them and its callers pass `true`/`true`, since a served manifest carries no lifecycle bit and is presumed active. The manifest is the source of the READABLE fields (on-chain the contract set returns `circuitId` as a keccak hash and carries no version string / no chain id); on-chain precedence is enforced by P3 `reconcile` BEFORE a manifest may feed the validator (`anchor_from_reconciliation` returns the conflicts if the manifest disagrees). This mapping lives in vet-api (which links both crates), keeping standard prover-free. The signed-manifest-fallback path is covered by `stacks/vet/api/tests/discovery_validation.rs`.

## Delegation - separate circuit (captain decision 2026-07-20, implementation deferred post-v1)

Delegation = an owner authorizing a **non-owner** (caretaker at the groomer) for **scoped consent, without transferring ownership**. Full decision record: `docs/DELEGATION.md`. There is no delegation code in the repo yet; this entry exists so nobody builds against the wrong assumption.

- **The decision: a SEPARATE delegate circuit, NOT a change to `consent.circom`.** The delegate is authorized by an **owner-signed delegation message** and is therefore **never committed in `R`**. The owner circuit, its reserved-leaf schema, and its `pub[7]` stay frozen; delegation arrives as a **new protocol version** under its own `contractSetId` via the two-axis `ProtocolRegistry` (R-5). This is what makes delegation **fully deferrable** - it gates neither Android parity, nor the submission path, nor app-release, and deferring costs one additional ceremony later, **not a redo of the owner ceremony**.
- **Do NOT "add delegation" by committing a second reserved triple into `R`.** It looks nearly free (the frozen circuit would accept it - see below), and it is a trap: `profileRoot` is write-once, so delegates would be fixed at mint, permanently unrevocable, unscoped, and indistinguishable from the owner on every public signal. Rejected deliberately; `docs/DELEGATION.md` §3.2 has the full reasoning.
- **NORMATIVE INVARIANT (P-e): exactly ONE `(owner.address, owner.consentKey, owner.secret)` reserved triple per `R`, always.** `consent.circom` verifies **three independent leaf inclusions** (`:155-157`) and its soundness argument ("pinning keyPath forces the unique real leaf") is an **assumption about the tree, not a property the circuit enforces**. It holds today because `build_profile_tree` derives all three reserved leaves internally and rejects any *attribute* at a reserved keyPath (`profile_tree.rs`, compared on the derived keyPath field). **Any future issuance entry point** - delegate-issuance API, externally-supplied leaf commitments, import/batch paths - **MUST preserve this**. A proposal needing a second triple is not a guard tweak; it changes the circuit's soundness argument.
- **`DEPTH` is now the ONLY remaining ceremony-gated decision** before the mainnet consent re-run. `DogTagConsent(6)` caps a tree at 64 leaves and is baked into the version identity (`circuitId == keccak256("consent.circom/DogTagConsent(6)")`). Triple coherence is **not** ceremony-gated - the P-e invariant covers it.
- **Delegation is not owner change.** Recovery is a fresh custodial issuance under a new `dogTagId` + new `R` (see "M6 app-side - recovery is re-issue"). Never implement delegation as a partial owner rebind, or recovery as a delegation.

## ZK trusted-setup ceremony

- This section is the **Level-A `verification.circom`** ceremony. **RETIRED / HISTORICAL:** the Level-A circuit and its ceremony scripts (`scripts/setup.sh`, `scripts/ceremony.sh`) were removed with the owner-revealing layer, so every ceremony command in this section is non-runnable provenance for the already-deployed retired-generation verifier - only the frozen artifacts remain in-tree, and the deployed addresses remain on the disposable testnet as deployment history only, superseded in place by the 2026-07-23 r8 fresh redeploy. The **Level-B `consent.circom`** circuit has its OWN M3 ceremony - see "M3 trusted-setup ceremony" under "Level-B `DogTagConsent` circuit (M2)" and `docs/CEREMONY_TRANSCRIPT.consent.md`. The surviving ceremony scripts are the consent ones: `scripts/setup-consent.sh` (DEV consent) + `scripts/ceremony-consent.sh` (consent, testnet single-contributor).
- Two now-removed verification scripts (historical): `circuits/scripts/setup.sh` was the **DEV/TEST** single-contributor setup (self-generated ptau, throwaway beacon) and must never have secured production; `circuits/scripts/ceremony.sh` was the **production** multi-party ceremony (public Hermez phase-1 ptau + ≥3 independent contributors + public beacon). Subcommands were: `init` → `contribute IN OUT "name"` (×N) → `beacon LAST 0x<hex> "note"` → `finalize`.
- Security model is **1-of-N honest, NOT majority/multisig**: the setup is sound if *any one* contributor destroys their toxic waste (entropy); broken only if *all* collude. So maximize diverse, independent contributors — adding more can only help. Do not describe it as a threshold/quorum scheme.
- The testnet key currently on-chain is a **single-operator self-run** (`docs/CEREMONY_TRANSCRIPT.md`, audit Finding H3) → forgeable; a production Level-A key would have required re-running the now-removed `ceremony.sh` (historical runbook `docs/CEREMONY_RUNBOOK.md`). The ceremony gates only the ZK path (`recordVerificationZK`); the ECDSA path and three-pillar trust model are unaffected.
- Circuit `DogTagVerification(24,5)` = 94,459 constraints → needs **2^17** powers of tau (`PTAU_POW=17`).
- Final artifacts: `circuits/build/verification_final.zkey` (proving key the Rust prover loads + pins SHA-256, impl §11.8(f)), `circuits/Groth16Verifier.sol` (vkey compiled in → deployed), `circuits/build/verification_key.json` (for `snarkjs groth16 verify`). `finalize` exports all three; verify with `snarkjs zkey verify r1cs ptau zkey` → `ZKey Ok!`.
- On-chain verifier swap has **no single-call setter**: `VerificationRegistry.proposeZkVerifier(addr)` → wait `ZK_TIMELOCK = 2 days` → `executeZkVerifier()`; confirm with `zkVerifier()`. Live registry `0x4E2f0996e1CB4E24F1053346f3da2186906835E8` (`contracts/deployments/roax.json`; the prior `0x8bA836eCe9…` is `VerificationRegistry_4arg_legacy`).
- **The retired-generation `VerificationRegistry` address (still on-chain as deployment history only, superseded by the r8 fresh redeploy) is baked into MANY committed consumers that must move together on any redeploy** (the 4-arg→6-arg fix split-brained precisely because only 2 of them were updated). The full set: `contracts/deployments/roax.json` (canonical; keep the old address as `VerificationRegistry_4arg_legacy`), the two compile-time mobile bundles `apps/ios/DogTag/roax.json` + `apps/android/app/src/main/assets/roax.json` (rebuild+reinstall both), the web/shared config `packages/ui/src/wallet/contracts.ts` + `stacks/owner/web/src/lib/config.ts`, the oversight indexer's registry gate in `stacks/indexer/api/src/main.rs` (its anti-spoof gate drops `Verified` logs from any address it does not recognize; the M8 dual-decode has since been collapsed and it now recognizes only `DEFAULT_VREG_CONSENT`) + `stacks/indexer/.env.example`, the demo/e2e scripts `scripts/{e2e-zk,demo-up,e2e-smoke}.sh` (`VR=`), the `stacks/{vet,groomer,admin,indexer}/**/.env.example` files, and the live-address tables in `README.md` + `AGENTS.md` + `docs/{DEPLOY,DEPLOYMENT,DEMO,REMOTE_DEPLOYMENT,CEREMONY_RUNBOOK}.md`. (The retired `RedeployVerificationRegistry.s.sol` that printed this checklist post-deploy was removed with the owner-revealing layer, and `docs/GROOMER_ZK_DEMO.md` has been deleted.) Do NOT rewrite the historical records (`roax.json` `_4arg_legacy`/`_zk_verifier_swap`/`_verification_registry_redeploy` fields, `docs/CEREMONY_TRANSCRIPT.md`) — those intentionally pin the old address.
- The **v2 ceremony verifier `0xEEFCfAF026931b7325472A88fd14Ee780Da13559` is the verifier the deployed retired-generation registry still points at** since the 2026-07-02 `executeZkVerifier()` cutover (tx `0xe2e3270f…40e70`, block 103419); the v1 verifier `0x138b4330…1761` was retired first and rejects v2-key proofs (and vice versa). Its address was baked into several committed consumers that had to move together on a swap: `contracts/deployments/roax.json`, `README.md` (Live ROAX addresses table), `stacks/owner/web/src/lib/config.ts`, `packages/ui/src/wallet/contracts.ts`, `scripts/e2e-zk.sh` (`ZKV=`), the live-chain parity tests (`crates/dogtag-standard-rs/tests/prove_parity.rs`, `stacks/vet/api/tests/prove_verification.rs` - both since deleted with the retired circuit), and the docs that quote the address (`docs/DEPLOY.md`, `docs/DEPLOYMENT.md`, `docs/DEMO.md`, `docs/CEREMONY_RUNBOOK.md`). The **mobile apps also carry the coupling** - each bundles its verifier's paired zkey/graph plus `roax.json` addresses and must be rebuilt + reinstalled on any swap (see "Building the mobile (iOS) holder app"); that coupling now binds the consent pair to `Groth16VerifierConsent`.

## Mobile end-to-end testing (Android, on-device ZK proof)

The Android app's on-device Groth16 proving flow has a real device/emulator e2e driven by
[Maestro](https://maestro.mobile.dev): `apps/android/maestro/zk_e2e.yaml`. It exercises the SAME
native code path the privacy-preserving groomer export uses — UniFFI → Rust SDK + circom-prover
(graph witness calculator) + the bundled proving key — with no camera, biometric, or network.

### How the e2e works (and why it's shaped this way)

The production scan→prove path is entangled with the camera QR scan, a biometric prompt, live
ROAX-chain RPC calls and a verifier host — none reliably automatable on an emulator. So instead of
faking all of that, the e2e drives a **debug-only ZK self-test** on the Profile screen
(`ui/screens/ZkSelfTest.kt`, gated by `BuildConfig.DEBUG` — never in release). It runs, on-device:

1. `proveConsent` - the REAL on-device owner-hidden Groth16 consent proof (graph witnesscalc +
   bundled `consent_final.zkey`), over a fixed seed/attribute vector.
2. public-signal check - the proof's 7 `pubSignals` must equal the Rust consent parity vector
   exactly, plus a non-zero nullifier (`pub[3]`) guard.

It renders the stable text `ZK-SELFTEST: PASS` / `ZK-SELFTEST: FAIL` that the Maestro flow asserts on.
The Maestro flow also asserts the Verify tab's `mobile root == server root: PASS` (the import/issuance
trust core through the native `.so`).

The fixed input vector and the seven expected signals are embedded in `ZkSelfTest.kt` and
byte-for-byte mirror the fixture in
`crates/dogtag-standard-rs/tests/consent_prove_parity.rs`, so the device proof MUST reproduce the
same public signals the host SDK computes.
Keep the two in step by hand after any change to that test (do not invent a new local vector).

### Running the e2e locally

A 64-bit (**arm64**) runtime is required — the prover ships only as `arm64-v8a` / `armeabi-v7a`
native libs, so an x86_64 emulator cannot load them. On this machine the SDK is at
`~/Library/Android/sdk` and the `roax_test` AVD is already `arm64-v8a` / android-34.

```bash
export ANDROID_HOME=~/Library/Android/sdk
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/27.0.12077973

# 1. Vendor the consent proving artifacts into the app bundles (see docs/MOBILE_BUILD.md §4). Both
#    sources are committed; the bundle copies are gitignored. Verifies the graph's attested hash.
make vendor-mobile-artifacts

# 2. Build the native prover libs into jniLibs (gitignored; Gradle does NOT run cargo-ndk).
cargo ndk -t arm64-v8a -t armeabi-v7a -o apps/android/app/src/main/jniLibs \
  build --release -p dogtag-standard-rs --features prover

# 3. Build + install the debug APK (system Gradle 9.5.1 == the wrapper version; the wrapper jar is
#    gitignored by a global *.jar rule, so `./gradlew` may be unavailable on a fresh clone).
echo "sdk.dir=$ANDROID_HOME" > apps/android/local.properties
( cd apps/android && gradle :app:assembleDebug )
adb install -r apps/android/app/build/outputs/apk/debug/app-debug.apk

# 4. Run the flow (Groth16 proving on an emulator is slow; the flow waits up to 180s for PASS).
maestro test apps/android/maestro/zk_e2e.yaml
```

### Sharp edges / gotchas

- **Witness graph IS committed now; it is not rebuildable by the published crate.**
  `circuits/build/consent.graph` (`wtns.graph.001` format, consumed by `circom_witnesscalc::
  calc_witness`) is **committed** (force-added past the `circuits/build/` ignore, like the zkey), so a
  fresh clone has it and the consent parity gate RUNS - it no longer skips, and an absent artifact is
  an incomplete checkout rather than an unbuilt one.
  It is committed precisely BECAUSE it cannot be reproduced on demand: the published
  `circom-witnesscalc` 0.2.1 crate ships no `build-circuit` binary (only `calc-witness`/`cvm-compile`),
  iden3's `build-circuit` must be installed out-of-band, and it is **not byte-deterministic** - so a
  per-machine rebuild made "which graph did this app prove with?" unanswerable (audit M9 rec 10).
  The committed bytes are attested by `artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256` and checked by
  `graph_file_matches_attested_sha256`; `scripts/vendor-mobile-artifacts.sh` re-verifies before
  copying into a bundle.
  A deliberate rebuild is a **rotation**: the attested constant and the on-chain `witnessMobileSha256`
  move together - see `docs/ARTIFACT_PIN_RUNBOOK.md`. Validate any graph against the zkey with
  `make test-consent-parity`.
  (The retired circuit's `verification.graph` can no longer be rebuilt at all - its circom source is
  gone; nothing consumes it anymore, so the mobile workflows no longer vendor it and the iOS `pbxproj`
  references only the consent pair.)
- **arm64 emulator only** — see above. `Build.SUPPORTED_64_BIT_ABIS` being empty (32-bit-only) has no
  on-device prover; the retired remote `/prove-verification` fallback is gone, and the consent
  server-prove fallback (`POST /prove-consent`) is the replacement concept - the backend route
  exists, but the mobile wiring lands in a later slice, so the self-test covers 64-bit devices only.
- **Gradle wrapper jar gitignored** — a global `*.jar` ignore drops `gradle-wrapper.jar`. Use system
  Gradle 9.5.1, or `gradle wrapper` to regenerate it.
- **`buildConfig = true`** is enabled in `app/build.gradle.kts` so `BuildConfig.DEBUG` gates the
  self-test card.
- **JNA can SIGSEGV on individual UniFFI exports** - the (since-deleted) `verifyConsentEddsa` export
  crashed natively on the arm64 emulator when called from Kotlin while every sibling export worked.
  If a new export crashes the same way, suspect the JNA binding for that specific function before
  suspecting the Rust.

### CI

`.github/workflows/android-mobile-e2e.yml` builds the app and runs this Maestro flow, but is
**`workflow_dispatch`-only** and targets a **self-hosted arm64 runner**: GitHub-hosted runners cannot
provide a hardware-accelerated arm64 Android emulator (the x86_64 emulators they accelerate can't load
the ARM-only prover `.so`), and the proving artifacts are gitignored. Wiring it to push/PR would make a
perpetually-red check. The validated signal is the local run above.

## Mobile end-to-end testing (iOS, on-device ZK proof)

The iOS app mirrors the Android e2e exactly: a Maestro flow `apps/ios/maestro/zk_e2e.yaml` drives the
SAME native code path the privacy-preserving groomer export uses — UniFFI → Swift bindings →
`DogTagFFI.xcframework` (Rust SDK + circom-prover graph witness calculator + the bundled proving key)
— with no camera, biometric, or network. It asserts the Verify tab's `mobile root == server root:
PASS` (import/issuance trust core) and the Profile screen's `ZK-SELFTEST: PASS`.

A second flow, `apps/ios/maestro/wallet_reset.yaml`, covers Profile's **Danger zone** typed-confirmation
gate (near-miss and cross-action phrases must both leave the destructive button inert). It stops at the
biometric gate by necessity — see "No biometric-gated flow is verifiable on a simulator" under
"Sharp edges / gotchas (iOS)".

### The iOS ZK self-test

`apps/ios/DogTag/ZkSelfTestScreen.swift` (`ZkSelfTestCard`) is the Swift port of Android
`ui/screens/ZkSelfTest.kt`, wrapped in `#if DEBUG` so it never ships in a release build. It runs, on
the device's own arm64 code: `proveConsent` (the REAL on-device owner-hidden Groth16 consent proof)
→ public-signal check (7/7 == the Rust consent parity vector, plus a non-zero nullifier guard).
The fixed inputs and the seven expected signals are embedded in the screen and mirror the fixture in
`crates/dogtag-standard-rs/tests/consent_prove_parity.rs` exactly, matching the Android
`ZkSelfTest.kt` vector.
Keep all three in step by hand after any change to that test; do not replace the vector with a
locally invented one.

### Building the on-device prover xcframework + running the e2e locally

`DogTagFFI.xcframework` is gitignored and is NOT produced by a plain Xcode build — build it from the
Rust crate (`--features prover`) for the iOS Simulator, regenerate the Swift bindings (keeping the
committed `apps/ios/DogTag/dogtag_standard.swift` ABI-consistent), then assemble it. On an
Apple-Silicon Mac:

```bash
# 1. Vendor the CONSENT proving artifacts into the app bundles (docs/MOBILE_BUILD.md §4) - these are
#    what the app actually proves with. Both sources are committed; bundle copies are gitignored.
#    MUST run before xcodegen below, which sweeps DogTag/.
make vendor-mobile-artifacts
# The committed project.pbxproj lists exactly this consent pair as bundle resources (the retired
# verification pair is gone from the wiring), so the two copies above are also what makes a plain
# xcodebuild link: it fails loudly on a checkout that has not vendored them.

# 2. Build the prover static lib for the arm64 iOS Simulator + a host build for bindgen.
rustup target add aarch64-apple-ios-sim
cargo build -p dogtag-standard-rs --features prover --release --target aarch64-apple-ios-sim --lib
cargo build -p dogtag-standard-rs --features prover --release --lib

# 3. Regenerate Swift bindings (header + modulemap + the committed .swift, all checksum-consistent).
gen=$(mktemp -d); cargo run --features uniffi/cli --release --bin uniffi-bindgen -- \
  generate --library target/release/libdogtag_standard.dylib --language swift --out-dir "$gen"
cp "$gen/dogtag_standard.swift" apps/ios/DogTag/dogtag_standard.swift

# 4. Assemble the xcframework (simulator slice). The headers dir needs the .h + a `module.modulemap`.
hdr=$(mktemp -d); cp "$gen/dogtag_standardFFI.h" "$hdr/"; cp "$gen/dogtag_standardFFI.modulemap" "$hdr/module.modulemap"
rm -rf apps/ios/DogTagFFI.xcframework
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios-sim/release/libdogtag_standard.a -headers "$hdr" \
  -output apps/ios/DogTagFFI.xcframework

# 5. Generate the Xcode project, build the debug app, install on a booted arm64 sim, run the flow.
( cd apps/ios && xcodegen )
SIM=$(xcrun simctl list devices available | awk -F'[()]' '/iPhone 16 \(/{print $2; exit}')
xcrun simctl boot "$SIM"; xcrun simctl bootstatus "$SIM" -b
( cd apps/ios && xcodebuild -project DogTag.xcodeproj -scheme DogTag -configuration Debug \
    -sdk iphonesimulator -destination "platform=iOS Simulator,id=$SIM" -derivedDataPath /tmp/dtbuild build )
xcrun simctl install "$SIM" /tmp/dtbuild/Build/Products/Debug-iphonesimulator/DogTag.app
maestro test apps/ios/maestro/zk_e2e.yaml   # Groth16 proving is slow; the flow waits up to 180s for PASS
```

### Sharp edges / gotchas (iOS)

- **xcframework is built `--features prover`** — without it the FFI surface has no `proveConsent`
  and the app won't link the prover symbols. The Swift binding is generated from a host dylib but MUST
  match the linked static lib's ABI; regenerate the `.swift` from the same crate build (step 3) so the
  embedded UniFFI checksums agree, otherwise the app traps at the first FFI call.
- **Simulator slice only** — the committed build path makes a `aarch64-apple-ios-sim` xcframework, so
  building for a *device* destination fails until you add an `aarch64-apple-ios` slice (+ signing). The
  e2e runs on the Simulator, which needs no Apple team.
- **Generated `DogTag.xcodeproj` is committed** — it is produced by `xcodegen` from
  `apps/ios/project.yml`; re-run `xcodegen` (don't hand-edit the project) after adding/removing source
  files, and commit the regenerated `project.pbxproj`. **Trap:** `xcodegen` enumerates the `DogTag/`
  folder, so regenerating in a checkout that has NOT vendored the referenced prover resources
  (gitignored) silently DROPS their Copy-Bundle-Resources entries from the committed `pbxproj`.
  The committed `pbxproj` references the CONSENT pair (`consent_final.zkey` + `consent.graph`; the
  retired verification pair is gone from the wiring), so vendor that pair (step 1) before
  any regen - it is also what the app proves with. A pure-UI change that
  adds no source file needs no regen at all: fold new views/types into an existing `.swift` and the
  `pbxproj` stays untouched.
- **Local pet photos are UI-only** — `PetPhotoStore` (LocalStore.swift) keeps per-`dogTagId` avatars as
  JPEGs under `Documents/pet-photos/`; deliberately separate from `Pet` (which `mergeCentralPets`
  overwrites) so a photo survives central sync. Never uploaded, never on-chain, never in a credential.
- **zkey + graph are gitignored under `apps/`** (`apps/.gitignore`) — a fresh checkout has neither;
  vendor the consent pair from `circuits/build/` (step 1) or the e2e fails to prove. Validate the
  graph/zkey pair on the host with `make test-consent-parity` (wraps `cargo test -p
  dogtag-standard-rs --features prover on_device_consent_proof_verifies_and_pub_matches`).
- **No biometric-gated flow is verifiable on a simulator (iOS 26 runtimes).**
  `Biometric.authenticate` (`Wallet.swift`) is written to fall through to success when
  `LAContext.canEvaluatePolicy(.deviceOwnerAuthentication)` is false — "e.g. headless sim", per its
  comment. That is **no longer true**: on iOS 26 sims `canEvaluatePolicy` returns **true** with no
  passcode enrolled, so `evaluatePolicy` shows an "Enter iPhone Passcode" sheet that cannot be
  satisfied headlessly. Everything behind the gate — create wallet, export account keys, replace
  wallet, every Danger-zone delete — therefore stops dead in the Simulator. Verified dead ends, do
  not re-walk them: a brand-new `simctl create` sim behaves the same (it is the runtime, not leftover
  device state); `notifyutil -s com.apple.BiometricKit_Sim.enrollmentChanged 1` does not persist
  (reads back `0`, even set+post in one process); Settings' Passcode pane renders **blank** on this
  runtime so a known passcode cannot be set from the UI; and driving Simulator.app's
  *Features → Face ID* menu needs an interactive Accessibility grant a headless agent cannot give.
  Consequence: gated flows are **device-only** verification (real Face ID), and Maestro flows must
  stop at the gate — which is what `maestro/wallet_reset.yaml` does.

### CI (iOS)

`.github/workflows/ios-mobile-e2e.yml` builds the xcframework + app and runs this Maestro flow, but is
**`workflow_dispatch`-only** and targets a **self-hosted Apple-Silicon (arm64) macOS runner**:
GitHub-hosted runners don't reliably provide the arm64 Simulator prover slice, and the proving
artifacts are gitignored. Wiring it to push/PR would make a perpetually-red check. The validated signal
is the local run above (this lab: iPhone 16 / iOS 18.6 simulator, real proof, `ZK-SELFTEST: PASS`).

## Building the mobile (iOS) holder app

This is the **signed build that installs the holder app on a physical iPhone** - the real-user device build.
It is distinct from the Simulator/e2e build in the "Mobile end-to-end testing (iOS, on-device ZK proof)" section above: that one assembles a **sim-only** xcframework and installs unsigned onto a booted Simulator; this one adds the **`aarch64-apple-ios` device slice + code-signing** and installs onto a plugged-in iPhone.
`docs/MOBILE_BUILD.md` §5 is the full cross-tier walkthrough; this section owns only the device delta, the canonical-checkout rule, and the zkey<->verifier gotcha that has actually shipped broken installs.

### 0. Build from the canonical checkout, not a stale clone

Build `origin/main` (or the exact release commit) in a checkout you have just `git fetch`ed - never a divergent local clone.
The proving key, witness graph and `DogTagFFI.xcframework` are all **gitignored** (they never appear in a commit), so a stale clone silently ships **old code AND an old zkey** with no diff to warn you: the app builds green and installs fine, then every ZK verification reverts on-chain (the gotcha below).
Multiple diverged clones of this repo on one machine is a real footgun - prep done in one worktree does not reach a phone built from another. Confirm before building:

```bash
git fetch origin && git rev-parse --short HEAD origin/main   # HEAD should equal, or descend from, origin/main
```

### 1. Build the DogTagFFI xcframework - device + simulator slices (`--features prover`)

Same recipe as the sim build in the e2e section above, plus the `aarch64-apple-ios` **device** slice, combined into one xcframework.
`--features prover` is mandatory: it compiles in the on-device Groth16 consent prover (`crates/dogtag-standard-rs/src/prover_ffi.rs`, gated `#[cfg(feature = "prover")]`); without it the `proveConsent` symbol is absent and the device build fails to link.

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
# device + simulator static libs (both arm64), plus a host build for the bindgen dylib
cargo build -p dogtag-standard-rs --features prover --release --target aarch64-apple-ios     --lib
cargo build -p dogtag-standard-rs --features prover --release --target aarch64-apple-ios-sim --lib
cargo build -p dogtag-standard-rs --features prover --release --lib
# regenerate the Swift bindings so the committed .swift stays ABI/checksum-consistent (see the e2e section)
gen=$(mktemp -d); cargo run --features uniffi/cli --release --bin uniffi-bindgen -- \
  generate --library target/release/libdogtag_standard.dylib --language swift --out-dir "$gen"
cp "$gen/dogtag_standard.swift" apps/ios/DogTag/dogtag_standard.swift
hdr=$(mktemp -d); cp "$gen/dogtag_standardFFI.h" "$hdr/"; cp "$gen/dogtag_standardFFI.modulemap" "$hdr/module.modulemap"
# assemble BOTH slices (device ios-arm64 + simulator ios-arm64-simulator) into the xcframework
rm -rf apps/ios/DogTagFFI.xcframework
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libdogtag_standard.a     -headers "$hdr" \
  -library target/aarch64-apple-ios-sim/release/libdogtag_standard.a -headers "$hdr" \
  -output apps/ios/DogTagFFI.xcframework
```

The sim-only recipe above omits the first `--target aarch64-apple-ios` build and the second `-library` line; a device install fails with a link/slice error until both are present.

### 2. Vendor the ZK ceremony assets into the bundle

Copy the CONSENT proving key + witness graph into `apps/ios/DogTag/` (both gitignored under `apps/`, absent on a fresh checkout; `docs/MOBILE_BUILD.md` §4) - these are the assets the app proves with:

```bash
cp circuits/build/consent_final.zkey apps/ios/DogTag/consent_final.zkey
cp circuits/build/consent.graph      apps/ios/DogTag/consent.graph
```

**The `consent.graph` IS produced by a plain checkout** - it is committed under `circuits/build/`, so `make vendor-mobile-artifacts` is all a fresh clone needs (see the graph note in the e2e "Sharp edges / gotchas"; a stub placeholder will NOT prove).
Validate the vendored pair on the host: `make test-consent-parity` (wraps `cargo test -p dogtag-standard-rs --features prover on_device_consent_proof_verifies_and_pub_matches`).

The committed `project.pbxproj` references exactly this CONSENT pair as bundle resources (the retired `verification_final.zkey`/`verification.graph` references are gone from the wiring), so the two copies above are also what makes a plain xcodebuild link: a checkout that has not vendored them fails loudly ("Build input file cannot be found") - that failure is the guard, not a project bug.

### 3. Regenerate the Xcode project + set the signing team

The project is generated from `apps/ios/project.yml` by `xcodegen`; the signing team is the `settings.base.DEVELOPMENT_TEAM` line there (with `CODE_SIGN_STYLE: Automatic`).
Set it to **your** Apple Developer team, then regenerate - editing the generated `DogTag.xcodeproj` does not stick:

```bash
# edit apps/ios/project.yml -> settings.base.DEVELOPMENT_TEAM: <YOUR_TEAM_ID>   (repo default: AYDBUX9433)
cd apps/ios && xcodegen
```

**Trap:** `xcodegen` enumerates `DogTag/`, so regenerating BEFORE step 2 silently drops the referenced prover-resource Copy-Bundle-Resources entries from the `pbxproj` - the CONSENT pair `consent_final.zkey` + `consent.graph` (see the xcodegen traps under "Sharp edges / gotchas (iOS)" and "Building / verifying UI changes"). Vendor the referenced pair first (step 2), regenerate second.

### 4. Build + install (signed) on the device

Plug in + unlock the iPhone and Trust the Mac. Simplest path: open the project in Xcode, select the **DogTag** scheme + your device, press **Run** (Xcode builds, signs, installs, launches in one step). Or from the CLI:

```bash
open apps/ios/DogTag.xcodeproj                                # then pick the DogTag scheme + device + Run
# --- OR the CLI path (Xcode Run is simpler for on-device debug; prefer it if signing gives trouble) ---
xcrun devicectl list devices                                 # copy the plugged-in iPhone's identifier/UDID
cd apps/ios && xcodebuild -project DogTag.xcodeproj -scheme DogTag \
  -destination 'platform=iOS,id=<DEVICE_UDID>' -derivedDataPath /tmp/dtdev -allowProvisioningUpdates build
xcrun devicectl device install app /tmp/dtdev/Build/Products/Debug-iphoneos/DogTag.app --device <DEVICE_UDID>
```

If the build fails with **code-signing / "no team" / "failed to register bundle identifier"**, the baked `DEVELOPMENT_TEAM` is not yours - fix it in `project.yml` and re-run `xcodegen` (step 3), never in the generated project (`docs/MOBILE_BUILD.md` §5/§9). If the phone shows **"Untrusted Developer"**, trust your team under **Settings -> General -> VPN & Device Management** on the phone, then relaunch.

### THE CRITICAL GOTCHA - the bundled zkey/graph/FFI MUST match the on-chain verifier

The bundled `consent_final.zkey` + `consent.graph` + the compiled-in FFI prover **must match the consent verifier currently deployed on-chain** - `VerificationRegistryConsent.zkVerifier()` for the target chain.
Unlike the **server** prover, which fails closed on a mismatched key (the consent descriptor's zkey pin; see "Deployment / production guards"), **the mobile bundle has no such guard** - it will happily ship any zkey and emit proofs the chain rejects.
A stale bundled key produces a proof the on-chain verifier refuses: `recordVerificationZK` reverts at `require(zkVerifier.verifyProof(...), "bad proof")`, surfacing to the operator as **`recordVerificationZK ... "bad proof"`** or a bare **`execution reverted, data: "0x"`**.
This is audit finding **H-1 (no zkey<->verifier version handshake)** made concrete: nothing on-chain advertises which zkey it expects, so the match is a **manual, mobile-side responsibility** (the M7 discovery anchor's artifact pins are the designed remedy once published).

Check it on every build:

```bash
# 1. hash the key you are bundling
shasum -a 256 apps/ios/DogTag/consent_final.zkey
# 2. read the deployed consent verifier (ROAX; addresses in contracts/deployments/roax.json)
cast call 0xaBFd6f6E31780EBcB7ABd28A2a9bCfc9C8e6A77B "zkVerifier()(address)" --rpc-url https://devrpc.roax.net
```

The bundled zkey's sha256 must be the ceremony output paired with whatever `zkVerifier()` returns.
Currently (see `roax.json` `Groth16VerifierConsent` + `_r8_fresh_redeploy`) that is `Groth16VerifierConsent` `0x1A9027986B859dc3879896B053deA78F636BE9b1`, paired with the frozen consent zkey sha256 `f83a111f…`.
Do not transcribe these values into new places - `roax.json` and its `_r8_fresh_redeploy` note own them.
(The retired-generation registry's v1/v2 verifier history lives in the historical "ZK trusted-setup ceremony" section; it is not a target for new bundles.)

**Rebuild + reinstall the app whenever the on-chain verifier is upgraded** - a trusted-setup/ceremony cutover done via `proposeZkVerifier(addr)` -> wait `ZK_TIMELOCK` (2 days) -> `executeZkVerifier()` (there is no single-call setter).
Re-vendor the new ceremony's zkey/graph (step 2), rebuild the xcframework (step 1), reinstall (step 4).
An already-installed app keeps proving against its **baked** key until you do, so a phone left on the old build silently starts reverting the moment the cutover lands on-chain.

## Contract sharp edges

- `VerificationRegistryConsent.recordVerificationZK(a, b, c, pub[7])` uses the frozen signal order
  `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`; `recordType` and `deadline` are
  proof-bound signals, not trailing calldata. `pub[2]` is range-checked below `2^160`, `pub[4]` must
  equal the tag's write-once `profileRoot`, and `ownerOf` is called only as a token-existence gate—its
  neutral-custodian return value must never be compared as owner identity. Every relay ABI must stay in
  sync with this four-argument signature.

## Governance authority (Phase-2 executed) - tooling signer

- **Governance authority is signer-1 `0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2`; the tooling ADMIN key
  is signer-1.** Governance Phase-2 executed on-chain 2026-07-05 (block 123835), moving registry
  `DEFAULT_ADMIN_ROLE` + `WHITELIST_ADMIN` and `DogTagIssuerFactory` `Ownable2Step` ownership off the old
  deployer EOA `0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96`. A 2026-07-16 audit found the old EOA still has the
  retired-generation `DogTagSBT`'s (still deployed) `ISSUER_ROLE` and known record-type whitelists, so it
  is **not role-free** and must never be treated as a neutral custodian. Governance writes (`whitelistFor` / factory `createIssuer` /
  `adminRevoke`) still require signer-1; separately retire the old EOA's legacy issuance capabilities.
- **Demo / relayer / demo-script tooling reads signer-1 from a captain-managed env var - the private-key
  VALUE is never committed.** The `scripts/*.sh` demo + e2e harnesses (`demo-up.sh`, `demo-bootstrap.sh`,
  `demo-prepare-phone.sh`, `e2e-smoke.sh`, `e2e-zk.sh`) source **`GOVERNANCE_PRIVATE_KEY` /
  `GOVERNANCE_ADDRESS`** (signer-1) from `contracts/.env` and fail closed if unset; `DEPLOYER_*` is kept
  only for `forge` contract deploys / ceremony scripts and must not be confused with a neutral key (the
  old deployer retains legacy issuance capabilities). The admin stack
  reads the same authority as `ADMIN_PRIVATE_KEY` / `ADMIN_ADDRESS` (`stacks/admin/.env`). See
  `docs/PREREQUISITES.md` §2.1. (The on-chain / backend-signer record lives in the "Governance / admin"
  section above and in `contracts/deployments/roax.json`.)

## Captain's conventions & vocabulary

(Folded in from the firstmate-private canonical record so any agent in this repo shares the captain's conventions and vocabulary.)

### Working environment (WezTerm tab + tmux flow)

- **One project, one tab.** Each project is developed in its **own dedicated WezTerm terminal tab**,
  backed by its **own dedicated tmux session** named for the project. A project's tab shows only that
  project's work - never another project's. Do **not** hardcode a tab number; the captain's tab ordering
  is environment-specific and may change - describe the convention, not "tab N".
- **Crewmates live inside their project's tab.** Every agent working on a repo runs as a tmux
  window/pane **within that project's session/tab**, alongside its sibling crewmates for the same
  project, so all of one project's parallel work is visible together in one tab.
- **Never share/group tmux sessions across projects.** Session grouping mirrors the same window list
  across tabs and scatters every project's work into every tab; keep each project's session independent
  (ungrouped) so tabs stay clean and project-scoped.
- A crewmate **may spawn as many additional tmux windows/panes as it needs** - builds, tests, logs,
  watchers, REPLs - within its project's session, so the work stays observable to the captain.
- Prefer giving long-running or noisy processes (servers, watchers, test loops, dev builds) **their own
  tmux window/pane** rather than blocking the main one. Keep the work visible.

### Common vocabulary the captain uses

- **Codex** - OpenAI's Codex coding agent / CLI; an alternative agent harness to Claude Code.
- **Claude** - Anthropic's Claude: the models and the Claude Code agent / CLI.
- **GPT** - OpenAI's GPT family of models.
- **axi** - the "agent-ergonomic" wrapper convention: a CLI suffixed `-axi` exposes an agent-friendly
  interface over an underlying tool. **Prefer the `-axi` wrapper over the raw tool.**
- **gh-axi** - agent-ergonomic GitHub CLI wrapper; use it for all GitHub operations instead of raw `gh`.
- **chrome-devtools-axi** - agent-ergonomic Chrome DevTools / browser-control CLI; use it for browser
  automation instead of raw browser tooling.
- **lavish-axi** - Lavish Editor CLI; turns HTML artifacts into collaborative, annotatable human-review
  surfaces.
- **gnhf** - the captain's code-cleanup framework / workflow: cleanup passes, typically run in isolated
  clones and staged as PRs for review. (Functional description - confirm exact definition with the
  captain if precision is needed.)
- **tmux** - terminal multiplexer used to run and observe agent work across windows and panes.

## iOS holder app (apps/ios)

### Record display (Home / Documents / Travel / detail / export picker)
- Every credential row must state WHAT the record is and WHICH pet it belongs to. Use the shared
  `CredentialLabel` view (DocumentsScreen.swift) + the `Credential` display helpers in Models.swift
  (`displayTypeLabel`, `vaccinationDetail`, `leafCount`, `exceedsZkLeafLimit`). Never render a bare
  `cred.title` / `recordType`.
- Pet name is NOT in `PetPhotoStore` (that stores photos only, keyed by dogTagId). Resolve it via
  `LocalStore.petDisplayName(forDogTagId:)`: synced `Pet.name` → the DOG_PROFILE credential's
  `credentialSubject.name` leaf → fall back to `DogTag #<id>` (never "Unnamed"/"Dog Profile").
- Vaccinations are the USDA rabies schema (`packages/ui` `RABIES_VACCINATION`, recordType
  `VACCINATION`). The specific vaccine + date are the `vaccineProductName` + `vaccinationDate` leaves,
  which sit at the `data` TOP LEVEL (the vet's `build_vc` wraps operator fields directly), not under
  `credentialSubject`. Extract by keyPath suffix.

### ZK export leaf limit
- **Retired.** The 24-leaf export cap belonged to the retired owner-revealing circuit (`DogTagVerification(24, 5)`), whose fixed-leaf-array prover, `pub const N = 24`, the `ZkCircuit.maxLeaves` display mirror, and the export picker's "too many fields" gating were all deleted with it.
- The live consent circuit (`DogTagConsent(6)`) proves reserved-leaf INCLUSION PATHS in a depth-6 tree (~64 leaves max), so record size no longer gates the ZK path; note there is deliberately no leaf-count guard in `build_profile_tree` (see "Known-uncovered surfaces").
- A record's leaf count still == `WrappedDoc.decodedFields().count`, which flattens `data` identically to the SDK's `flatten_data` (both skip empty collections and count only string leaves).

### Building / verifying UI changes
- Build: `xcodebuild build -project apps/ios/DogTag.xcodeproj -scheme DogTag -sdk iphonesimulator
  -destination 'id=<sim-udid>' CODE_SIGNING_ALLOWED=NO`. SourceKit single-file diagnostics report
  cross-file symbols (Credential, LocalStore, …) as "not found" — those are false positives; only the
  full `xcodebuild` result is authoritative.
- Do NOT re-run xcodegen (`project.yml`) casually: it silently drops the vendored prover resources
  (consent_final.zkey / consent.graph) from the pbxproj. Prefer editing existing `.swift`
  files over adding new ones so the pbxproj (which lists sources individually) needs no regen.
- To eyeball record lists without a backend: install to a booted sim, write `pets.json` +
  `credentials.json` into the app's `get_app_container … data`/Documents dir, relaunch, screenshot.

## D1 - vet-attested identity leaves + ProfileDisclosure (selective disclosure)

The owner's vet-collected identity (name/country/id) is committed into `R` as hidden,
selectively-disclosable **attribute** leaves in the sanctioned `owner.identity.*` namespace
(`fullName`/`country`/`docNumber`; `routes.rs` `KP_IDENTITY_*`).
Everything is off-circuit: identity leaves are ordinary `hash_leaf` leaves, the frozen consent
circuit is leaf-blind, and `make test-consent-parity` proves the VK did not move (its witness now
includes identity leaves on purpose).
The sharp edges:

- **The `owner.` prefix guard has ONE carve-out.** `build_profile_tree` rejects any caller
  attribute whose NFC keyPath starts `owner.` UNLESS it starts `owner.identity.` (trailing dots
  load-bearing: bare `owner.identity` = a blob leaf and `owner.identityX` = squatting both stay
  rejected). Do NOT "harden" this into a blanket `owner.` rejection - the tree is REBUILT from the
  same attribute list at consent-prove time, so a blanket guard breaks every proof for a tag with
  identity leaves. Android mirrors the predicate in `ProfileTreeBuilder.assertSingleOwnerTriple`.
- **One attribute list, three consumers.** Identity openings live in the SAME persisted
  `attributes` list (owner-secret store) that feeds (a) issuance `R`, (b) the verify-time
  `proveConsent` rebuild, and (c) the disclosure builder - order pet-then-identity on both the
  build and retry paths. A separate field or different order silently diverges `R` and fails every
  proof closed.
- **Salt ownership differs by namespace.** Pet-attribute salts are device-random; identity-leaf
  salts are VET-generated at `profile_issue_session_start` and travel to the device via the
  `/p/<token>` `identityLeaves` block. That is what powers the bind-time ATTESTATION-INTEGRITY
  GATE, a FULL-LEAF-LIST commitment check (`routes.rs::verify_leaf_commitment`): custodial-bind
  requires the device to OPEN every attribute leaf of its tree (`leaves`) and name the reserved
  owner-control triple's leaf hashes opaquely (`reservedLeafHashes`, exactly 3, preimages never
  sent); the vet recomputes every opened leaf, requires the `owner.identity.*` openings to
  EXACTLY equal its stored `{keyPath, salt, value}` set (no missing/extra/duplicate/altered
  entry - INJECTION of an unattested identity leaf is refused, not just replacement), rebuilds
  the Merkle root from [3 reserved hashes + attribute hashes], and refuses the bind (400, before
  ANY chain write, token consumed) unless it equals the posted `R`. A forged identity leaf must
  either be opened (refused by exact-set equality) or displace a reserved leaf hash - and a tree
  missing a reserved leaf can never produce a consent proof, while disclosures are only accepted
  alongside a consent proof for the same `R`. The bind deliberately reveals the device-random
  pet-attribute salts to the vet - zero-cost, the vet supplied every attribute value in the
  first place. A session whose operator collected no identity degrades to an EMPTY identity
  subset; posted identity openings against it are refused, not ignored. Tests:
  `custodial_bind_identity_gate.rs`.
- **`ProfileDisclosure` wire shape is Rust-owned.** `{dogTagId, R, disclosures:[{keyPath, saltHex,
  tag, value, proof}]}` (proof steps `"promote"` | `0x..` sibling hex), produced by
  `build_profile_disclosure_json` and consumed by `disclosure::verify_profile_disclosure` - mobile
  embeds the JSON verbatim and never hand-re-encodes it. It rides OPTIONALLY alongside the consent
  proof in the verify submission (`profileDisclosure` key), bound there to the proof's
  `R`/`dogTagId` (its only anti-replay context - a bare envelope is a replayable bearer
  credential). The verify handler records only the revealed keyPaths, never values. Tests:
  `submit_consent_disclosure.rs`.

## DogTag standard SDK (Rust + TS + Swift/Kotlin)

The credential crypto lives in three byte-for-byte-equivalent legs that MUST stay in lockstep:

- `crates/dogtag-standard-rs` — Rust core + the UniFFI mobile surface (`ffi.rs`).
- `packages/dogtag-standard-ts` — the TypeScript reference (**generates** the shared vectors).
- `apps/ios/DogTag` (Swift) + `apps/android` (Kotlin) — consume the Rust core through UniFFI.

### Shared test vectors are the cross-language contract

`packages/dogtag-standard-ts/testvectors.json` is the source of truth. The TS SDK generates it
(`pnpm --filter @dogtag/standard gen-vectors`); the Rust SDK asserts the exact same file
(`crates/dogtag-standard-rs/tests/sdk_parity.rs`), and the iOS app asserts it at runtime
(`apps/ios/DogTag/VerifyScreen.swift`). After regenerating you MUST copy it byte-identical to both
app bundles — they are plain copies, not symlinks:

```
cp packages/dogtag-standard-ts/testvectors.json apps/ios/DogTag/testvectors.json
cp packages/dogtag-standard-ts/testvectors.json apps/android/app/src/main/assets/testvectors.json
```

Readers ignore unknown keys, so adding a new vector section is backward-safe.

### Build / test

- Rust: `cargo test -p dogtag-standard-rs` (default), and `--features assemble` for the consent
  circuit-input assembly tests (`consent_assemble` is the only assembly module). `--features prover`
  additionally pulls the heavy on-device Groth16 consent prover (ark 0.5).
- TS: `pnpm --filter @dogtag/standard test` (vitest) and `... build` (tsc).
- Keep `cargo clippy -p dogtag-standard-rs --lib --bins --tests` warning-clean.

### Regenerating the Swift UniFFI binding (`apps/ios/DogTag/dogtag_standard.swift`)

This file is autogenerated but checked in. When you change `ffi.rs`, regenerate it — but build the
dylib **with `--features prover` first**, otherwise the `prover`/`assemble`-gated consent prover
surface (`proveConsent`, `ProofFfi`) is dropped and the binding regresses:

```
cargo build -p dogtag-standard-rs --lib --features prover
cargo run --features uniffi/cli --bin uniffi-bindgen -- \
  generate --library target/debug/libdogtag_standard.dylib --language swift --out-dir <dir>
```

The regenerated diff should be ONLY your changed functions (uniffi is pinned to 0.28.x). A large,
noisy diff means your local uniffi ≠ the version that produced the checked-in file — stop and
reconcile rather than committing the churn.

### Adding an iOS Swift source file

Add it **surgically** to `apps/ios/DogTag.xcodeproj/project.pbxproj` (four entries mirroring an
existing sibling: a `PBXBuildFile`, a `PBXFileReference`, a group child, and a Sources build-phase
entry, using fresh 24-char hex IDs). Do NOT blindly `xcodegen generate` — regenerating the project
silently strips the vendored prover resources (zkey / witness graph) from the pbxproj.

If you genuinely need a regen (e.g. adding a target), the safe procedure is: vendor the consent
pair per docs/MOBILE_BUILD.md §4 (`cp circuits/build/consent_final.zkey apps/ios/DogTag/` + the
`consent.graph` copy) - or `touch` both paths if you only need the wiring, not a proving build -
so xcodegen sees them (the committed pbxproj references the CONSENT pair; the retired
verification pair is gone from the wiring), `xcodegen generate`, then confirm with
`git diff --no-color apps/ios/DogTag.xcodeproj/project.pbxproj | grep '^-'` that **no** zkey/graph
line was removed. Both files are gitignored, so the placeholders can never be committed. Expect a
large but harmless diff: xcodegen re-randomises every object ID, so hand-written IDs churn while
target membership is unchanged — diff membership, not IDs. (Piping `git diff` without `--no-color`
into `grep '^-'` silently matches nothing because of the ANSI prefix; that false "clean" reading is
easy to trust by mistake.)

### iOS unit tests (`apps/ios/DogTagTests`)

There **is** now an XCTest target. It is deliberately **host-less and FFI-free**: it lists the
self-contained sources it covers directly (`sources: [DogTagTests, DogTag/QrPayload.swift,
DogTag/PublicSignalIndex.swift, DogTag/ZkeyAsset.swift, DogTag/AnchorResolver.swift,
DogTag/LocalDataSweep.swift, DogTag/Wallet.swift` + Wallet's closure
`Secp256k1`/`BigUInt`/`Keccak256`/`Wordlist`]` in `project.yml` - keep this list in step with
that file) rather than
using `@testable import DogTag`, because the app module links
`DogTagFFI.xcframework`, which is gitignored and absent until someone builds the Rust core. That
keeps the suite runnable on a plain checkout:

```
cd apps/ios && xcodebuild test -project DogTag.xcodeproj -scheme DogTagTests \
  -destination 'id=<simulator-udid>'      # `-destination 'name=iPhone 16'` is ambiguous; use the UDID
```

Adding a source here that transitively imports the FFI will break that property — extract the pure
logic instead.

**Host-less also means NO Keychain.** A bundle with no host application has no
keychain-access-group entitlement, so every `SecItemAdd`/`SecItemDelete` in it returns
`errSecMissingEntitlement (-34018)` no matter how correct the logic is. `Wallet.swift` compiles into
this target (its closure — `Secp256k1` → `BigUInt`/`Keccak256`, `Bip39` → `Wordlist` — is
Foundation/CryptoKit only), but only its Keychain-free surface is testable there: `SeedBackupGateTests`
covers the `SeedBackup` gate (`UserDefaults` + CryptoKit) and `Bip39` derivation against the published
BIP-39 vector. `Wallet.create/deleteKeys/replace` are verified in the running app instead. Do **not**
give this target a host app to get around it — that trades the plain-checkout property above for
coverage. Note the published all-zero-entropy BIP-39 seed vector
(`bda85446c684…`) is for passphrase **`"TREZOR"`**, not the empty passphrase `Wallet` actually uses
(`408b285c1238…`); pairing the wrong one looks like an implementation bug and is not.

`QrPayloadTests.swift` mirrors `QrPayloadTest.kt` case-for-case; keep them in step, as
their whole point is that the two platforms cannot silently diverge on what a QR means.
`ZkeyAssetTests.swift` likewise mirrors the Android `ZkeyAssetTest.kt` (and the Rust
`artifact.rs` tests), pinning the version-keyed resolver's contract (the consent set is the sole
entry and the default; the internal version key `dogtag-levelb/1` resolves to it; an unknown version
fails closed); its covered source `DogTag/ZkeyAsset.swift` is pure — only
`ensure`/`ensureGraph` touch `Bundle.main`, and the tests never call them, so it stays FFI-free.

### Getting real Swift signal without the xcframework

`DogTagFFI.xcframework` is gitignored / pipeline-built, but you can still exercise Swift end-to-end:
build the staticlib (`cargo build -p dogtag-standard-rs --lib --features prover` →
`target/debug/libdogtag_standard.a`) and `swiftc` a small harness that links the `.a` plus the
generated `dogtag_standardFFI.modulemap` (`-Xcc -fmodule-map-file=...`). Pass the `.a` positionally
to force static linking (a `-L/-l` pair prefers a stale dylib). Full-app typecheck without linking:
`swiftc -typecheck -sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)" -target arm64-apple-ios17.0-simulator <all app .swift> -I <gen> -Xcc -fmodule-map-file=<gen>/dogtag_standardFFI.modulemap`.

### Selective-Disclosure Protocol (DSDP) — Merkle inclusion proofs (plan §2.3)

The `Sibling | Promote` inclusion-proof engine lives in `merkle.{rs,ts}` and the Swift verifier in
`apps/ios/DogTag/InclusionProof.swift`. Sharp edges:

- `process_proof` / `processProof` is a fold **primitive**, NOT a membership check: it trusts an
  opaque leaf hash and an internal node folds to the root just as happily (the audit's C1/E2
  opaque-leaf hazard). The normative, safe entry point is `verify_inclusion` / `verifyInclusion`,
  which RECOMPUTES the leaf from `(keyPath, salt, tag, value)` under `DS_LEAF` (Poseidon5) before
  folding. The arity/domain split (leaf = Poseidon5/`DS_LEAF`, node = Poseidon3/`DS_NODE`) is what
  blocks presenting an internal node as a disclosed leaf.
- `Promote` steps are pass-throughs: they carry tree-shape/depth info, not authentication, so
  dropping one still folds a genuine member to the root. Do NOT add canonicality/shape checks to
  `verify_inclusion` — that diverges from the normative §2.3 fold.
- `checkIntegrity` requires an EMPTY `signature.proof` (single-doc credentials only; doc→batch-root
  inclusion never shipped and C1 forbids trusting the permissive fold in the trust path).
- The dogTagId canonical-keyPath binding (F1, plan §2.4) is NOT here — it lands in the reference
  verifier milestone (M3), not the inclusion-proof engine (M1).

---

## Level-B `DogTagConsent` circuit (M2) — owner-unlinkable consent

Source of truth: `/Users/zhenhaowu/firstmate/data/dogtag-zkverify-z2/level-b-spec.md`.
Circuit: `circuits/consent.circom` (template `DogTagConsent`, instantiated at `depth=6`).
Shared fold lib: `circuits/lib/merkle_inclusion.circom`.
Tests: `circuits/scripts/test-consent.mjs`. Dev setup: `circuits/scripts/setup-consent.sh`.

`DogTagConsent` proves in zero-knowledge that a **hidden** pet owner consented to a **disclosed**
relayer for a **disclosed** purpose, revealing nothing about the owner. It supersedes the Level-A
`verification.circom` (which exposed `subject` + `keyHash`). **That owner-revealing circuit source has
since been retired/removed** (its build products + ceremony transcript remain as historical provenance);
the shared `NodeHash`/`LessThanField` templates were *copied* into `lib/merkle_inclusion.circom`, not
refactored out of the then-frozen circuit.

### Public-signal vector (ORDER IS LOAD-BEARING for M4 calldata)

snarkjs emits public signals as the circuit's **output** signals in declaration order (all seven
public signals are declared as outputs to fix the order; verified via `build/consent.sym`):

| idx | signal       | meaning                                         | solidity type    |
|-----|--------------|-------------------------------------------------|------------------|
| 0   | `dogTagId`   | the tag being verified                          | uint256          |
| 1   | `purpose`    | purpose label, reduced mod field                | bytes32 / field  |
| 2   | `relayer`    | relayer address (range-checked `< 2^160`)       | address / uint160|
| 3   | `nullifier`  | `Poseidon6(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce)` | uint256 |
| 4   | `R`          | per-tag Merkle root the 3 owner leaves fold to  | uint256 / field  |
| 5   | `recordType` | record-type label (**prover-asserted**, see below) | bytes32 / field |
| 6   | `deadline`   | consent expiry (signed inside `M`)              | uint256          |

**No `subject`, no `keyHash`.** The owner never appears in the public signals.

### On-chain calldata shape (M4 `recordVerificationZK`) - SHIPPED as specced

This was M2's forward spec for M4; M4 has since built exactly it in
`contracts/src/VerificationRegistryConsent.sol` (see "Level-B `VerificationRegistryConsent` (M4)" below
for the as-built contract, incl. the one deviation noted in step 6). The snarkjs Solidity verifier exposes:

```solidity
function verifyProof(
    uint[2]    _pA,
    uint[2][2] _pB,
    uint[2]    _pC,
    uint[7]    _pubSignals   // == [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
) external view returns (bool);
```

`recordVerificationZK` therefore takes `(a, b, c, pubSignals[7])` and, per spec
§"On-chain `recordVerificationZK`":
1. `require(verifyProof(a,b,c,pubSignals))` against the **new VK** (from the M3 ceremony).
2. `require(pubSignals[4] /*R*/ == profileRoot(pubSignals[0] /*dogTagId*/))` — binds the proof to the
   real tag. **This is the only place `dogTagId ↔ R` is checked; the circuit does NOT bind it.**
3. `require(deadline >= block.timestamp)` (pubSignals[6]).
4. `require(!nullifierConsumed[pubSignals[3]])` then consume it.
5. `emit Verified(dogTagId, relayer, purpose, nullifier, deadline, block.timestamp)` — **owner-blind**.
6. **Delete** the old `ownerOf` / `keyOf` checks and the `subject`/`keyHash` handling. **As built, with one
   deviation:** the `keyOf` check and all `subject`/`keyHash` handling are gone, and no owner IDENTITY is
   ever compared - but `ownerOf(dogTagId)` is still CALLED, with its return value discarded, purely as a
   token-existence gate. Deleting that call would reopen the burn/GDPR-erasure hole (`burn` does not clear
   `profileRoot`), so do not "finish" step 6 by removing it; see the M4 section below.

### Reserved owner-leaf schema (M5 issuance MUST match this exactly)

The per-tag tree has three **private** owner-control leaves plus disclosable attribute leaves. Each
leaf = `Poseidon5(DS_LEAF=1, fieldOf(keyPath), fieldFromScalarBytes(salt16), typeTag, value)`, leaf
hashes sorted before folding (the M1 engine). The circuit **pins keyPath + typeTag** of the three
reserved leaves to these constants:

| leaf          | keyPath string      | `fieldOf(keyPath)` constant (pinned in circuit)                                   | typeTag     | value slot                    |
|---------------|---------------------|-----------------------------------------------------------------------------------|-------------|-------------------------------|
| owner-address | `owner.address`     | `20593649144631820416234157596070441856608371338897391424937040814759273231214`  | 5 (Bytes)   | app-supplied owner addr field |
| consent-key   | `owner.consentKey`  | `7822071287675030884271946396254564996644565056920260282559292033992393086992`   | 5 (Bytes)   | `Poseidon2(Ax, Ay)` (keyHash) |
| owner-secret  | `owner.secret`      | `11172449362271989869407103131203633198993612309996015027844083581837121079156`  | 5 (Bytes)   | random secret field (= nullifier secret) |

`test-consent.mjs` re-derives these constants via the SDK `fieldOfKeyPath()` and asserts they match
the circuit literals — a drift guard. `consentNonce` and the 16-byte salts stay private and
per-leaf-distinct; the circuit consumes them as opaque field inputs either way. Since the M5 app-side
builder landed, the split matters: the three **reserved** leaves' salts (and the owner-secret itself)
are **seed-derived**, not random, so restoring the phrase regenerates them - **attribute** leaves'
salts stay per-leaf-random and are NOT seed-derivable, so rebuilding `R` needs the credential too.
See "M5 app-side" below.

**Reserved-leaf value encoding (the sharpest M5 handoff edge):** unlike disclosable attribute leaves
(whose value slot is `fieldOfValue(typedScalar)` — the length-prefixed byte-fold), the three reserved
leaves write a **raw field directly** into the value slot: owner-address = the owner address as a
field, consent-key = `Poseidon2(Ax,Ay)`, owner-secret = the raw secret field (which is *also* the
nullifier's `ownerSecret`). M5 must build the committed leaf and the circuit input from that same raw
field, NOT run these three through `fieldOfValue`.

**Why pinning keyPath is load-bearing (soundness, not cosmetic):** if `keyPath` were a free prover
input, a prover could point the owner-secret inclusion proof at *any other in-tree leaf* (e.g. a
disclosable attribute), set `ownerSecret` to that leaf's value, and mint a **second valid nullifier
for one signed consent** — breaking D5 replay protection. Pinning forces the unique real leaf.
`test-consent.mjs` test (e) exercises exactly this substitution and asserts it fails.
**That "unique real leaf" holds only because exactly ONE reserved triple is ever built into a tree** -
it is an assumption about the tree, not a constraint the circuit enforces. See the NORMATIVE P-e
invariant under "Delegation - separate circuit" above, which binds every future issuance entry point.

### Consent message & nullifier (the exact preimages)

- EdDSA message: `M = Poseidon5(dogTagId, purpose, relayer, deadline, consentNonce)` — **no DS tag**,
  no `R`, no `subject`. Signed by the BabyJubJub consent key `(Ax, Ay)` whose `Poseidon2(Ax,Ay)` is
  the pinned consent-key leaf value. The signature is bound to the tag via `dogTagId ∈ M` + `consent-key ∈ R`.
- nullifier: `Poseidon6(DS_NULLIFIER=4, ownerSecret, dogTagId, purpose, relayer, consentNonce)`.
  Scope = per `(dogTagId, purpose, relayer)` + nonce (D5): same signed consent → same nullifier
  (rejected on replay); fresh nonce → new nullifier (a genuine repeat visit is allowed).

### `recordType` is prover-asserted, NOT consent-signed

`recordType` (pubSignals[5]) is **not** in `M` and **not** in the nullifier, so the owner's EdDSA
consent does not attest it. It is safe because only the owner's app can generate this proof (it
needs the private leaves + salts, not merely the signature), so the app — not the relayer — chooses
`recordType`. Groth16 still binds it to the specific proof (it cannot be swapped post-proof). **M4
must treat `recordType` as a prover-supplied label, not as an owner-attested field.**

### M3 trusted-setup ceremony — DONE (VK FROZEN, testnet-grade)

The **M3 ceremony is complete**. `circuits/scripts/ceremony-consent.sh` ran a **testnet-grade
single-contributor** phase-2 (captain-approved for ROAX testnet; the mainnet ≥3-independent-contributor
re-run stays deferred): public Hermez `powersOfTau28_hez_final_17.ptau` (reused, phase-1 NOT re-run;
sha256 `6b662a32…`, byte-identical to the v2 ptau) → one contribution (fresh entropy, destroyed) →
public **drand** beacon (chain `8990e7a9…`, round `6286835`). Full transcript + reproduce/audit steps:
`docs/CEREMONY_TRANSCRIPT.consent.md`. Pinned outputs (committed, force-added past the `build/` ignore):

- **VK:** `circuits/build/consent_verification_key.json` (sha256 `27879dd7c4eabb6acea4d1be1249ba3c4212f95a27237e7e1e1220557b4e2d7f`, `nPublic=7`).
- **proving zkey:** `circuits/build/consent_final.zkey` (sha256 `f83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868`) — `snarkjs zkey verify` → `ZKey Ok!`; M7's prover pins this hash.
- **verifier:** `circuits/Groth16Verifier.consent.sol` → `contracts/src/Groth16VerifierConsent.sol` (contract **`Groth16VerifierConsent`** — renamed so it does NOT collide with the live v2 `Groth16Verifier`). `verifyProof(a,b,c,pub[7])`.
- This REPLACES the M2 DEV throwaway (dev VK `3f79a5ff…`, dev zkey `12df8ea4…`, both gitignored, forgeable, never deployed).
- `node circuits/scripts/test-consent.mjs` → **33/33 green** against this production key (round-trip verify, R-parity {3,4,5,7,10,20} leaves, 6 negatives, D5 nullifier).
- **Deployed ROAX `Groth16VerifierConsent`:** `0x272be146C0aEd6401000E9Aa8241201F6f0fdF1a` (chainId 135, `--legacy`, deployer `0x119F8c…`, deploy tx `0xcd1cd5fa…`, block 190760). On-chain `cast code` == the compiled runtime (1933 bytes); `verifyProof`(valid consent proof)=`true`, (tampered `R`)=`false`. Recorded in `contracts/deployments/roax.json` (`Groth16VerifierConsent` + `_m3_consent_verifier`). This is a SEPARATE verifier — it did NOT replace the retired-generation `Groth16Verifier` `0xEEFCf…` (still on-chain as deployment history only). It is wired into the then-canonical M5 `VerificationRegistryConsent` `0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87`; the former M4 `0x53F988Ae…` instance is deprecated. (The M3/M5 instances are themselves deployment history since the 2026-07-23 r8 fresh redeploy - the live verifier/registry pair lives in `roax.json`; see `_r8_fresh_redeploy`. The fresh `Groth16VerifierConsent` runtime is byte-identical to this audited ceremony verifier, so the frozen VK is unchanged.)

**VK-freeze checkpoint (`M`-preimage) — reviewed, frozen.** `M = Poseidon5(dogTagId, purpose, relayer,
deadline, consentNonce)` shares arity + first slot with the leaf hash `Poseidon5(DS_LEAF=1, …)` when
`dogTagId == 1`. **No exploit exists** (EdDSA needs the private key; leaves are never signed); the
public-signal order/count was re-verified from the freshly compiled circuit (7 outputs, 0 public
inputs); the captain-approved spec fixes `M` in this exact form (no DS tag). Changing `M` would require
changing the spec, this circuit, and M7's app proof-gen together, and re-running the ceremony — out of
M3 scope. VK **frozen** against `consent.circom` as merged in #42.

### Build / test / reproduce

```bash
# M3 REAL testnet ceremony -> committed build/consent_final.zkey + VK + Groth16VerifierConsent.sol (see transcript)
bash circuits/scripts/ceremony-consent.sh
# fast: witness/proof round-trip + R-parity + negatives + keyPath-substitution + D5 nullifier (vs the committed prod key)
pnpm --filter @dogtag/circuits run test-consent
# ⚠ DEV/THROWAWAY setup — self-generated ptau, forgeable; OVERWRITES the committed M3 zkey/VK. Do NOT run to deploy.
pnpm --filter @dogtag/circuits run build-consent
```

Since M3, the **production** consent artifacts are **committed** (force-added past the `build/` ignore):
`build/consent.r1cs`, `build/consent_final.zkey`, `build/consent_verification_key.json`,
`build/consent_js/consent.wasm`, plus `circuits/Groth16Verifier.consent.sol` /
`contracts/src/Groth16VerifierConsent.sol`. The **intermediate/DEV** artifacts stay **gitignored** and
must never be deployed: `build/consent_000{0,1}.zkey`, the ptau (`circuits/ptau/*.ptau`), and
`Groth16Verifier.consent.dev.sol` (`*.dev.sol`). `test-consent` now runs against the committed prod key
(33/33 green) and is a standalone heavy gate, intentionally **not** in `make test`.

---

## Level-B `VerificationRegistryConsent` (M4) — the owner-blind on-chain verify path

Source of truth: `/Users/zhenhaowu/firstmate/data/dogtag-zkverify-z2/level-b-spec.md`.
Contract: `contracts/src/VerificationRegistryConsent.sol`. Deploy: **`contracts/script/DeployCustodialIssuance.s.sol`**
(M5) - `DeployConsentRegistry.s.sol` was the M4 script (now removed) and is **SUPERSEDED**: it defaulted `SBT` to the
Level-A `DogTagSBT`, whose mutable `setProfileRoot` is the hijack M5 closes, and the registry's `sbt` is immutable so
that mistake is unrepairable. Tests: `contracts/test/ConsentRegistry.t.sol` (16, real M3 proof; deliberately still
pairs the registry with the Level-A SBT, proving it is SBT-agnostic) + `contracts/test/CustodialIssuance.t.sol` (the
production pairing). Fixture: `circuits/scripts/gen-consent-fixture.mjs`.

**Former M4 ROAX instance (superseded):** `VerificationRegistryConsent`
**`0x53F988Ae0124b96069d90CBC78E6245FeB01E125`** (chainId 135,
`--legacy`, deploy tx `0xbdcbb27d…`, block 195443, admin = governance `0x8E27E117…`). It verifies against the
M3 `Groth16VerifierConsent` `0x272be146…`. Recorded in `contracts/deployments/roax.json`
(`VerificationRegistryConsent_M4_mutableRoot_legacy` + `_m4_consent_registry`). The deployed runtime is **byte-identical** to the
committed source once the 3 immutables are blanked (6317 bytes).

**⚠ DEPRECATED / DO NOT USE for Level-B.** Its `sbt` is immutable and bound to the mutable Level-A
`DogTagSBT`, so M5 redeployed this same registry code against the custodial `DogTagSBTConsent`; canonical
M5 registry `0xb9B313C1…` supersedes `0x53F988Ae…`. The registry CODE below is still current - only the
deployed instance is deprecated. Safe because this instance was never live (zero `Verified` events,
re-verified immediately before the M5 broadcast on 2026-07-16);
see "M5 as-built" below and `roax.json` `_m5_custodial_issuance.old_instance_safety_check`.

**Superseded first deploy.** `0x57A2998…` (block 194489, runtime 6179 bytes, now
`VerificationRegistryConsent_preErasureGate_legacy`) was deployed BEFORE the review-round hardening that
added the `ownerOf` token-existence gate, so its bytecode LACKS the erasure gate: a burned tag would still
verify there. It was **never live** and no consumer ever pointed at it. It was **redeployed** rather than
left stale, because a canonical address whose bytecode differs from its source is the same landmine class
as `data/dogtag-zkfail-z9`. **Do not use `0x57A2998…`.**

### Superseded M4 rollout snapshot (historical only)

M4 temporarily deployed the owner-hidden registry beside the older registry. That coexistence was a
migration step, not an architecture to preserve: the owner-hidden registry is the only live/forward
model. The cutover has since completed - the repo carries no code path to the older registry, whose
deployed instance remains on the disposable testnet solely as deployment history, superseded in
place by the 2026-07-23 r8 fresh redeploy.

### What it does (spec §"On-chain `recordVerificationZK`")

`recordVerificationZK(a, b, c, pub[7])` — **4 args**, because `recordType`/`deadline` are public SIGNALS
now (Level-A took them as unbound calldata). `pub = [dogTagId, purpose, relayer, nullifier, R, recordType,
deadline]`. In order: range-check all 7 signals; `relayer < 2^160` (audit L1); `deadline >= block.timestamp`;
Art. 9 guard; `relayer == msg.sender`; `VERIFY:` whitelist; **`R == profileRoot(dogTagId)`**; the
`ownerOf(dogTagId)` token-EXISTENCE gate (fails burn/GDPR-erasure closed; the return value is DISCARDED, it
is NOT an owner-identity check); `verifyProof` vs the consent VK; consume the nullifier; resolve
`rootIssuer[R]` + `isValid(R)`; emit owner-blind `Verified`.

- **`R == profileRoot(dogTagId)` is THE Level-B binding.** The circuit deliberately does not bind
  `dogTagId ↔ R`, so this is the ONLY place it happens. Without it a prover folds a tree they fully control
  and consents as any tag.
- **No owner-IDENTITY check, no `keyOf`, no `ConsentKeyRegistry` (D2), no Poseidon6.** There is no
  `ownerOf == subject` comparison. `ownerOf(dogTagId)` IS called, but purely as a **token-existence gate**
  whose return value is DISCARDED (never compared) - under D1 it is the neutral custodian, so it leaks
  nothing and the owner-blind property holds. It exists because `DogTagSBT.burn` (GDPR-erasure) does NOT
  clear `profileRoot[id]`, so `R == profileRoot` still passes for an erased tag; OZ `ownerOf` is
  `_requireOwned` and reverts on a burned token, which is what fails erasure closed. Level-A got that for
  free as a side effect of its `ownerOf == subject` check. The nullifier is a public signal bound in-circuit
  to the hidden `ownerSecret` + `consentNonce` (D5); Level-A derived it on-chain from `subject`, which
  Level-B does not have. Constructor is 5-arg `(ir, sbt, zk, ridx, admin)`, not 7.
- **`Verified(dogTagId, relayer, purpose, nullifier, deadline, ts)`** — `subject` is GONE. Same event NAME as
  Level-A but a different signature ⇒ **different topic0**; the indexer decodes by `Verified::SIGNATURE_HASH`.
  **M8 (landed) taught the oversight indexer to dual-decode BOTH shapes during the migration** (since
  collapsed - the indexer now decodes only the subject-less owner-hidden `Verified` shape; see the
  "Oversight indexer" note). The receipt-side parser in **vet-api** `chain.rs` that assumed the retired
  subject-bearing shape has likewise been retired: `chain.rs` now carries only the owner-blind
  `ConsentVerifiedEvent` decode.

### ⚠ Two traps worth knowing before you touch this

1. **`recordVerificationZK(...)` = selector `0xdd080593`, byte-identical to the RETIRED Level-A 4-arg
   selector** — same ABI shape, completely different `pub` semantics (Level-A `pub[3]`=subject,
   `pub[4]`=nullifier, `pub[6]`=R). A stale pre-PR#7 client aimed here DISPATCHES instead of bouncing, but
   fails closed twice (`R !profileRoot`, then a Level-A proof cannot verify against the consent VK). The
   reverse — a consent client aimed at the deployed retired-generation registry (6-arg `0x423a45b6` only) — gives the bare
   `execution reverted, data: "0x"` from `data/dogtag-zkfail-z9`. **Check the ADDRESS first when debugging.**
2. **The Art. 9 constant MUST be reduced mod r.** `recordType` is a public signal, so it is always `< r`,
   while raw `keccak256("SERVICE_ATTESTATION")` (`0xa757…ed43`) **EXCEEDS r** — copying Level-A's raw
   constant makes the guard **dead code that can never fire**. The contract pins
   `SERVICE_ATTESTATION_FIELD = keccak256("SERVICE_ATTESTATION") % r`
   (`10025591956217394737855806998434905929145386518960477508456501950324730293568`); `ConsentRegistry.t.sol`
   recomputes it natively in Solidity and fires it on a REAL proof, so the regression cannot come back
   silently. The same applies to any bytes32 label crossing into a signal (`purpose` is already reduced
   by the live label→field reducers: vet-api's `purpose_key` in `stacks/vet/api/src/verify.rs` and the
   fixture generator's `labelField` in `circuits/scripts/gen-consent-fixture.mjs`).

### M5 handoff (issuance) — two hard requirements

1. `profileRoot(dogTagId) = R`, the per-tag M1-engine tree root.
   > **SUPERSEDED by M5 (2026-07-15).** This originally read *"No SBT change is needed - `DogTagSBT`
   > already stores `profileRoot` as `bytes32`."* Storage-wise that was true, but it missed the
   > `setProfileRoot` hijack below: a MUTABLE root is unsafe once the `ownerOf` identity check is gone.
   > M5 therefore ships a fresh **`DogTagSBTConsent`** with a write-once root and a structural custodial
   > mint. See "M5 as-built" below.
2. **`issue(R)` the root into a `DogTagIssuer` clone too, not only `setProfileRoot`.** The registry keeps
   Level-A's revocation path (`rootIssuer[R]` → clone → `isValid(R)`), so a root that is only set as
   profileRoot and never issued reverts **`unknown root` on every verify**. This is what keeps `revoke(R)`
   working under Level-B.

D1 (custodial mint) needs nothing from the registry: it reads `profileRoot` for the binding and `ownerOf`
for EXISTENCE only (value discarded), never for owner identity.
`test_owner_identity_is_never_read_from_chain` mints to a custodian and proves verification still passes;
`test_burned_tag_cannot_verify` proves erasure fails closed (and asserts `profileRoot` SURVIVES the burn,
which is why the existence gate is load-bearing rather than redundant).

**✅ RESOLVED IN M5 (2026-07-15) - captain chose a fresh Level-B SBT.** The hijack described below is
structurally closed on `DogTagSBTConsent`: `profileRoot` is **write-once**, set at mint, with **no setter at
all** and no burn/re-mint escape (`mintCustodial` rejects an id whose root is already set).
See "M5 as-built" below for the reasoning and the redeploy cascade it forced. The description is kept
because it is exactly why the owner-hidden SBT looks the way it does - and because it still applies verbatim to
the retired-generation `DogTagSBT`, which remains deployed unchanged on the disposable testnet as deployment history only, superseded in place by the 2026-07-23 r8 fresh redeploy.

**⚠ ORIGINAL OPEN SPEC QUESTION - `setProfileRoot` hijack.** `R == profileRoot(dogTagId)` is the SOLE
tag↔owner binding, and
`DogTagSBT.setProfileRoot` is **mutable post-issuance** by `issuerOf[id]` or ANY `AUTHORITY_ROLE` holder.
With no `ownerOf` identity gate left, a compromised issuer/authority can build a tree whose three reserved
owner leaves it controls, `issue(R2)` it through its own whitelisted clone, call
`setProfileRoot(victimTag, R2)`, and record a `Verified` for a victim tag - a forgery Level-A prevented by
requiring BOTH `ownerOf == subject` AND `keyOf(subject) == keyHash`, both owner-controlled. M5 must resolve
it (e.g. append-only/immutable `profileRoot` post-issuance, or a `status == Active` gate). Deliberately NOT
fixed in M4: the SBT hardening belongs with the M5 issuance rework, and Level-B is not live (no consumer
points at this registry until M7).

### M5 as-built (custodial issuance) - contract/issuer side

**Captain decision (2026-07-15): fresh Level-B SBT. Deployed + verified 2026-07-16.**
`DogTagSBTConsent` `0x96Cba4580D79bc9b8e51Fc1B3a044A29592AfFFc` (tx `0xbf4b52f5…`, block
202601) + `VerificationRegistryConsent` `0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87` (tx
`0xc215d980…`, block 202602). Both exact deployed runtimes match the compiled source, including expected
immutable values; the registry points at the new SBT, M3 verifier `0x272be146…`, IssuerRegistry
`0x5d86e4CF…`, and root index `0xd3179AbB…`; both admins are governance `0x8E27E117…`. The SBT's immutable
neutral custodian is `0x637A514628d06Af711e3C9A2636fdBe5AE0E5A10`, with no code, prior role, or
whitelist history. Deployment grants no `ISSUER_ROLE` (member count zero). The prior-generation `DogTagSBT`
(`0x1FB89865…`) stayed FROZEN beside it - the same additive pattern as M2/M3/M4 - and has since been
retired from the repo (its deployed instance remains on-chain as deployment history only). **No consumer
was changed in M5 itself**; the M5 pair later became the sole consumer target, and the 2026-07-23 r8
fresh redeploy has since superseded it - every consumer now points at the fresh owner-hidden set
(roax.json `_r8_fresh_redeploy`).
Deploy script: `script/DeployCustodialIssuance.s.sol`.

**The issuance flow is TWO writes, both required:**
```solidity
vacc.issue(R);                    // register the root in a DogTagIssuer clone (rootIssuer[R])
sbt.mintCustodial(dogTagId, R);   // mint to the neutral custodian; sets profileRoot = R, sealed
```
Minting alone yields a tag that reverts `unknown root` on EVERY verify. The spec's step list names only
`profileRoot`, so this is the trap to avoid; `test_root_must_also_be_issued_into_a_clone` pins it.

**What `DogTagSBTConsent` changes vs Level-A, and why:**
- `mintCustodial(id, root)` takes **no `to`** - the custodian is immutable, so an issuer cannot mint to an
  owner's wallet even by mistake. The owner's wallet is not an argument, so it is in neither calldata nor state.
- **Write-once `profileRoot`**, no setter (closes the hijack - see the resolved note above). The seal holds
  across a **burn** too: `mintCustodial` rejects any id whose `profileRoot` is already set. That guard is not
  redundant with ERC-721's duplicate-mint check - OZ `_burn` leaves no tombstone and `_mint` only rejects a
  re-mint when the previous owner was non-zero, so `_safeMint` alone would let an ISSUER holder burn a tag
  and re-mint the same id under an attacker root, reaching the same forgery without a setter. Consequence,
  intended: a `dogTagId` is single-use FOREVER, so a burned/GDPR-erased id can never be re-minted (erasure is
  permanent) and recovery uses a fresh id per D3. `profileRoot` still deliberately SURVIVES the burn (never
  cleared) - the M4 registry's `ownerOf` existence gate depends on that, and the mapping doubles as the
  permanent "id already used" record. Pinned by `test_burned_dogTagId_can_never_be_reminted`.
- **No `recover`/`Recovered`** (D3): a rebind names the new owner on-chain. Recovery = fresh issuance. The
  `_inRecovery` soulbound bypass is gone with it, so the lock is absolute.
- `CUSTODIAN` is mandatory at deploy with no default: it must be neutral (not an owner, and not a vet
  signer - that would re-link tags to the practice's key). It never signs; it is a sink, not an actor.

**The owner-absence guard** - `CustodialIssuance.t.sol::test_owner_wallet_absent_from_issuance_state_and_calldata`
is the load-bearing test: it byte-scans the issuance calldata, sweeps every storage slot the issuance
writes, and checks the logs. It has a **positive control**
(`test_owner_absence_scanner_actually_detects_the_owner`) because an absence assertion passes vacuously if
the scanner is broken. Verified by mutation: reintroducing an owner-supplied recipient fails three guards.
If you touch issuance, keep the control.

#### Why a fresh SBT - the redeploy cascade (established 2026-07-15)

Recorded so it is not re-derived or "simplified" back into a broken shape.

1. **The suggested `status == Active` gate is already there and does NOT fix the hijack.**
   `DogTagSBT.setProfileRoot` (in the deployed Level-A `DogTagSBT`) already does
   `require(status[id] == Status.Active, "!active")`. A hijacker targets an *Active* victim tag, so this
   gate never fires against them. **Append-only/immutable `profileRoot` is the only structural fix of the
   two the open question offers.**
2. **A standalone `CustodialIssuer` contract cannot close the hijack.** Routing issuance through one makes
   `issuerOf[id]` the contract (killing the *issuerOf* vector), but `AUTHORITY_ROLE` holders call
   `setProfileRoot` on the SBT **directly** - the gate lives on the SBT, so no external contract can
   constrain it. Closing `AUTHORITY_ROLE` requires changing `DogTagSBT` itself.
3. **Sealing therefore cascades:** new `setProfileRoot` semantics ⟹ new SBT bytecode ⟹ new SBT deploy ⟹
   **new `VerificationRegistryConsent` deploy too**, because its `sbt` is `immutable`
   (`VerificationRegistryConsent.sol:90`, set in the constructor) and cannot be repointed.
4. **The M4 redeploy was near-free - verified on-chain, not assumed.** `0x53F988Ae…` carries exactly ONE log
   (the deployment `RoleGranted`) and **zero `Verified` events**
   (`cast logs --address 0x53F988Ae… --from-block 0`, topic0 `0xeb5f75f2…`), re-verified through block
   202680 after M5 finality. Nothing consumes it; M7 has not cut over.
5. **`setProfileRoot` has ZERO production callers.** Only `ConsentRegistry.t.sol:198` (the hijack test) and
   docs reference it - the vet-api mints the root directly via `mint(to,id,root)`. Sealing it breaks no
   live flow, and D3 (recovery = re-issue a fresh tag ⇒ new `R`) means a tag's `profileRoot` never
   legitimately changes after issuance. Both point at write-once.
6. **PR #39 was CLOSED UNMERGED** (`mergedAt: null`; head `fm/dogtag-issfix-i4`), so none of it reached
   `main` - not the dormant `mintNext`, and not the two "must-fix" items. M5 re-implemented the
   owner-independent ones (7B-1 dead `DOG_PROFILE` option; register-first naming) and **deliberately did NOT
   re-implement 7B-2's `ownerOf` existence pre-flight**: under D1 the tag is custodial, so an `ownerOf` read
   says nothing about the owner and must not gate issuance. There was no code to delete - "dropping the
   gate" means not building it. Do not confuse it with the dup-collision `owner_of` loop
   (`routes.rs:1723`, which M-2 extended to consult the Level-B `profileRoot` marker too) or the
   post-mint read-back (`routes.rs:1978-1982`), which are unrelated and stay.
   `mintNext` was NOT re-implemented either: `DogTagSBTConsent` takes an issuer-supplied `id` like Level-A.
   Contract-assigned ids remain open and are orthogonal to owner-unlinkability.
7. **The register-pet flow at the time still minted to the OWNER's wallet** (`mint_wallet = wallet`,
   asserting `ownerOf == wallet`). That was the linkability M5 removed at the contract layer. **M-2
   then added the custodial route BESIDE it** (`POST /profiles/issue/custodial-bind`, see "Custodial
   issuance bridge" below), accepting a device-supplied `R` rather than building the tree (see the custodial issuance
   bridge section below). **(Since retired:** the owner-revealing wallet-mint route was deleted in the
   backend cutover, PR #72, and the mobile custodial-bind QR issuance call site landed in PR #71 -
   custodial-bind is now the sole issuance path.)

### Build / test

```bash
# regenerate the real-proof fixture (needs the committed M3 zkey/wasm) -> contracts/test/consent-fixture.json
node circuits/scripts/gen-consent-fixture.mjs      # or: pnpm --filter @dogtag/circuits run gen-consent-fixture
forge test --match-contract ConsentRegistryTest    # 16 green, incl. a REAL Groth16 proof on-chain
```

Unlike `test-consent`, this suite runs in plain `forge test` (the fixture is committed, so it needs no
circuit toolchain) and is part of the normal contracts gate.

---

## M5 app-side - device-side tree building + the recoverable owner-secret

The other half of M5. The contract side (above) can seal a `profileRoot`, but nothing could *produce*
one owner-privately until this landed: `crates/dogtag-standard-rs/src/profile_tree.rs` is the
device-side per-tag tree builder, reached from the holder apps through the UniFFI core they already
link. User/dev doc: **`docs/MOBILE_OWNER_SECRET.md`** (owns the local-file contract).

**Android reached device-side parity in M-2b (`profile/ProfileTreeBuilder.kt` + `ProfileTreeStore.kt`).**
Both holder apps are thin wrappers over the SAME compiled `buildProfileTreeHex`, so `R` is identical by construction, not by two ports agreeing - never re-implement the tree math in Swift or Kotlin.
The single-owner-triple invariant - an `R` must hold exactly ONE `(owner.address, owner.consentKey, owner.secret)` triple, since `consent.circom` proves the reserved leaves by pinned keyPath and assumes each is unique - is enforced in `build_profile_tree`.
Kotlin mirrors it only to FAIL FAST with a named error, and compares NFC strings where the SDK compares field elements, so it is a convenience mirror and not an equivalent guard; the Rust core stays the authority.
`ProfileTreeParityTest` drift-guards the mirrored list against the Rust constants.

**Trap - the device-root fixture's `dogTagId` is the RAW integer field.**
`device_root_fixture_witness()` binds to `Fr::from(424242u64)`, *not* `dog_tag_id_field_hex("424242")`, which folds the ENCODED decimal and is a different value.
The committed `contracts/test/device-profile-root.json` `R` corresponds to the raw-field id, so anything comparing against that fixture must pass the bare integer field.
Reaching for the canonical helper because it "looks more correct" silently yields a root nothing ever built.
Production paths (`buildAndPersist` on both platforms) DO use the canonical encoding, which is why the two entry points are split deliberately (`build` vs `buildForIdField`).
Recovery must rebuild from the STORED canonical field rather than re-converting the decimal.

**The owner-secret is seed-derived, not random (captain's requirement, 2026-07-15).** The spec's
"a random secret" is satisfied by a KDF output - indistinguishable from random to anyone without the
seed - while additionally being *recoverable*, which a random secret is not:

| input | derivation |
|---|---|
| owner-secret | `BLAKE-512("DogTag/owner-secret/v1" ‖ dogTagId[32B BE] ‖ u64be(0) ‖ seed)` → `from_be_bytes_mod_order` over all **64** bytes |
| reserved-leaf salts | `BLAKE-512("DogTag/reserved-leaf-salt/v1" ‖ dogTagId[32B BE] ‖ u64be(len(UTF8(keyPath))) ‖ UTF8(keyPath) ‖ seed)[0..16]` |
| consent-key | `BLAKE-512("DogTag/consent-key/babyjubjub/v2" ‖ dogTagId[32B BE] ‖ u64be(0) ‖ seed)[0..32]` → `prv2pub` (`eddsa::derive_babyjub_consent_key_per_tag`) |

Reduce the **full 64-byte** digest, never a 32-byte prefix: a bare 32-byte hash mod r is measurably
biased. Binding to `dogTagId` is what keeps one wallet's two tags mutually unlinkable.

All three share ONE preimage builder, `kdf::kdf` (`domain ‖ dogTagId[32B BE] ‖ u64be(len(extra)) ‖
extra ‖ seed`). Keep it that way: the consent key spent its first life on a hand-rolled
`domain ‖ seed` preimage in `eddsa.rs` and that is precisely how it stayed seed-only while its two
siblings were already per-tag. A second preimage builder is the drift.

**The consent key is per-tag as of 2026-07-19 (captain's decision), domain bumped `v1` → `v2`.**
Free at the time: no Level-B tag had been minted, so no migration. It feeds the `owner.consentKey`
leaf and therefore `R`, which is write-once - so this was a now-or-never change. Two tags of one
wallet now get different `(Ax, Ay)`, closing the last cross-linking vector in the owner-control core
(previously the raw pubkey was shared wallet-wide, harmless only for as long as it never left the
device). Purely an off-circuit derivation change: `consent.circom` takes `Ax`/`Ay` as plain inputs,
so the R1CS, the frozen VK and the ceremony were all untouched.

**The old wallet-level sibling `derive_babyjub_consent_key_from_seed` (`v1` domain) is DELETED.**
That one served the retired owner-revealing path ONLY, where the consent key lived OUTSIDE the tree:
the retired circuit emitted `keyHash = Poseidon2(Ax,Ay)` as a public signal and the retired registry
checked it against `ConsentKeyRegistry.keyOf[subject]`, a `mapping(address => bytes32)` -
per-WALLET by contract design. The owner-hidden model retires that path entirely (the consent key
moved INTO the tree, so `keyOf` is retired), and the function went with it in the SDK cleanup slice;
`derive_babyjub_consent_key_per_tag` (`v2`) is the only consent-key derivation left.

**Salts had to be seed-derived too, and this is the non-obvious part.** A recoverable secret alone
does NOT rebuild the tree: fresh random salts change every leaf hash and therefore `R`. Reserved-leaf
salts are therefore KDF'd; **attribute** salts are not (their values come from the issuer and are not
seed-derivable either), so they are backed up in the device file instead. Do not "simplify" the salt
KDF back to an RNG - the recovery round-trip is what would break, and only in the field.

**Why `hash_reserved_leaf` exists and must not be folded into `hash_leaf`.** It writes a RAW field
into the value slot; `hash_leaf` always runs the value through `field_of_value`. This is the
"sharpest M5 handoff edge" from the reserved-leaf schema section above, now enforced by
`reserved_leaf_writes_a_raw_field_not_a_folded_value`.

**What pins device-built `R` to the chain - three gates, each catching a different regression:**

```bash
cargo test -p dogtag-standard-rs   # core + FFI round-trip + circuit-R parity + fixture drift
(cd contracts && forge test --match-contract CustodialIssuanceTest)   # 17 green, incl. device-root
cargo run -p dogtag-standard-rs --bin gen-device-profile-root   # regenerate the bridge fixture
```

1. `tests/profile_tree_parity.rs` rebuilds the **fixture's** witness and asserts the Rust root equals
   `consent-fixture.json`'s `R` - an `R` a REAL Groth16 proof was made against and the M4 registry
   accepted. So the builder is checked against the circuit, not against itself.
2. `contracts/test/device-profile-root.json` carries an `R` from the REAL builder over a fixed demo
   seed (generated by the bin above, drift-guarded from the Rust side by
   `committed_device_profile_root_matches_a_fresh_device_build`). `CustodialIssuance.t.sol` mints it
   and asserts `profileRoot == R`. **If you change the builder, regenerate that file** or the Rust
   gate fails first, with the command in the message.
3. The keyPath drift guard re-derives the three pinned `consent.circom` constants via
   `field_of_keypath` (mirrors `test-consent.mjs`).

**Read the guarantee precisely - it is TWO legs, not one transitive equality.** Two different roots
are involved: `R_fixture` (gate 1, circuit-verified) and `R_demo` (gate 2, built by
`build_profile_tree` over the demo seed). No proof is ever generated against `R_demo`, so it is NOT
itself circuit-verified. What actually holds is:

1. the builder's PRIMITIVES (reserved-leaf raw-field encoding, pinned keyPaths, the M1 fold)
   reproduce a root a real Groth16 proof was made against, and
2. `build_profile_tree`'s OWN output is what the contract stores as `profileRoot`.

So a device-built tree is provable *because its primitives are the circuit's*, not because that exact
root was proven. Proving a seed-derived root end-to-end needs the prover (M7); do not upgrade this
claim without generating a proof over `R_demo`.

**The owner-hidden server-side bridge is the issuance end-state, and the cutover is COMPLETE.** The
owner-revealing wallet-mint route has been deleted from the repo (backend PR #72) and the device
call site landed (mobile PR #71): custodial-bind is the sole issuance path.

### Level-B custodial issuance bridge (M-2) - `POST /profiles/issue/custodial-bind`

The server path that mints an owner-hidden tag - now the ONLY issuance path. Same operator-started QR
session shape the retired wallet-mint flow used; the device redeems the one-time bind token with
`{ token, root }` and the server anchors + seals `R`.

**It inverts who computes `R`.** The retired wallet-mint flow built `R` server-side (`wrap_vc` over
an owner-identity VC). The owner-hidden path cannot: `R` is folded on the DEVICE from the wallet seed
(`ProfileTreeStore.swift` / `build_profile_tree`), which the server has and must have no access to.
The handler therefore builds no VC - it treats `R` as opaque. **Do not "fix" this by wrapping a VC
server-side**; that produces an owner-revealing root that `consent.circom` cannot prove against.

**No wallet, no signature, by design.** `mintCustodial` has no recipient - the tag goes to the
immutable custodian - so the retired flow's EIP-191 wallet signature has nothing to attest, and
accepting one would hand the server exactly the owner link the model removes. The authorization is
the one-time, operator-minted, 180s bind token alone; whoever redeems it defines ownership via the
owner-secret inside `R`.

**Ordering is load-bearing: `issue(R)` FIRST, then `mintCustodial(id, R)`** (the contract says so at
`DogTagSBTConsent.sol:139-143`). The mint is the irreversible half - `profileRoot[id]` is write-once
and survives a burn - so a mint that lands before a failing `issue` retires the `dogTagId` forever.
Both writes then get read back before the session flips to `bound`, and **`owner_of` is deliberately
NOT compared to anything**: the owner is the neutral custodian, and comparing it reintroduces the
linkage. The anchor read-back uses `isValid(R)` on our own clone, which is strictly stronger than
`rootIssuer[R] != 0` (a successful `issue` implies `registerRoot`, which is globally write-once).

**Two env vars, both required, both fail-closed when unset** (checked BEFORE the token is
consumed, so a half-wired stack never burns an operator's QR):
- `SBT_CONSENT_ADDR` - the `DogTagSBTConsent`. (It began life beside the retired `SBT_ADDR` during
  the migration; the retired var is gone with the wallet-mint route.)
- `PROFILE_ISSUER_ADDR` - a real factory-deployed `DogTagIssuer` clone. **Not a document-store /
  SBT address**: `issue(R)` sent to the SBT reverts.

Issuance stamps `LEVEL_B_VERSION` (`dogtag-levelb/1`, the internal protocol version key) - the
**on-chain `ContractSet` axis** of the two-axis registry (R-5), never the artifact axis, since a
zkey rotation must not move what an already-minted tag claims. (The retired `LEVEL_A_VERSION`
constant and its producers are deleted; every issuer stamps the unified key.)

Coverage: `stacks/vet/api/tests/custodial_issuance_bridge.rs` (real device-built `R`, both on-chain
conditions, raw-handle and skip-issue fail-closed cases, unconfigured/malformed fail-closed cases).

### Level-B unified submission path (M-3) - now `POST /v1/verify/consent`

The other half of M-2: the network layer that carries an owner-hidden consent proof to the chain.
Before it, a device could prove consent but nothing could submit that proof. (M-3 landed it as
`/verify/consent/levelb`; the migration-era route split has since collapsed and the sole submit
route today is `POST /v1/verify/consent`, handler `crate::verify::consent_submit_levelb` - the
internal name mirrors the internal version key, not a product mode.)

**The `sol!` interface was built NEW, never an edit of the retired one** ([e9] R-2). During the
migration `chain.rs` carried both `IVerificationRegistry` (retired shape, frozen) and
`IVerificationRegistryConsent` side by side; the retired interface has since been deleted and
`IVerificationRegistryConsent` is the only one left. Editing the old one in place would have yielded
a hybrid matching neither deployed contract, since the owner-hidden shape deletes `subject` and
retires `ConsentKeyRegistry`. Three concrete differences vs the retired shape:

- **`recordVerificationZK` is 4-arg, a DIFFERENT SELECTOR** (`dd080593` vs the retired 6-arg
  `423a45b6`). `recordType`/`deadline` moved out of relayer calldata into `pub[5]`/`pub[6]`, so they
  are bound to the proof. The consent selector is pinned in `chain.rs`'s test module - the two calls
  shared the same `(a,b,c,pub)` prefix, so a mix-up was invisible at the type level and surfaced
  on-chain only as an empty `0x` revert.
- **`Verified` drops `subject`** (gains `deadline`), hence a different topic0. Decoders are
  address-gated AND shape-gated; `ConsentVerifiedEvent` carries no `subject` field at all, so there
  is no owner slot for a caller to fill.
- **No `ConsentKeyRegistry` leg at all** - no bind, no `keyOf`.

**Route every `pub[n]` through `dogtag_standard::public_signals::level_b`** (the module name mirrors
the internal version key). The retired order's NULLIFIER slot (4) is the consent order's ROOT:
keying the one-time check on `pub[4]` makes a SUCCESSFUL verification read as forever-unconsumed
(the E-1 bug).

**The relayer trust model is settled by the contract, not by config.** `relayer` is bound into both
the EdDSA consent message `M` and the nullifier, and the registry requires
`address(uint160(pub[2])) == msg.sender` - so the owner consents to ONE submitter and no other
address can carry that proof. The HTTP route's operator gate is therefore only a GAS-SPEND gate
(an open endpoint would let anyone burn the relayer's balance on reverting proofs); it confers no
authority over the submission itself.

**The preflight and the broadcast MUST resolve the same signer, and do so through
`custody::ACTIVE_SIGNER_INDEX` - never `Config::vet_signer_index`.** `pub[relayer]` is validated
against `custody.active_address()` (account 0) while `vet_signer_index` names the DOG_PROFILE
SBT-MINTING signer, an unrelated role that merely happens to be 0 today because `main.rs` hardcodes
the field. Broadcasting from the config field would preflight green against account 0 and then revert
on-chain at `"not relayer"` the moment that field is wired to an env var - a failure that appears only
in a specific deployment, never in tests that leave it 0. `active_address()` now reads the same
constant, so the two are one source by construction.
`the_broadcast_uses_the_same_signer_the_preflight_validated` pins it by setting `vet_signer_index` to
an index that is never unlocked and asserting the submit still succeeds (`MemChain` errors
`"no signer for index"` on a miss, and genesis registers ONLY account 0). Leave the M-2 issuance path
alone - it uses `vet_signer_index` legitimately, for minting.

**The broadcast is DETACHED behind a WIDE deadline margin - and the margin is what makes detaching
safe.** The route acks `"recording"` + a `sessionId` and broadcasts from a
`tokio::spawn`; the consumer polls `GET /verify/session/:id` for the terminal `recorded`/`error` +
txHash. Detaching is not stylistic: Axum CANCELS a handler's future when the client disconnects, so
awaiting the ~12-24s receipt inline lets any client timeout or proxy cutoff strand the audit row at
`"recording"` while the tx still mines and spends gas - and the retry then reverts `"replayed"`, so a
verification that SUCCEEDED reads in the trail as a failure, gutting the very trail the row exists
for.

What this path cannot copy from the retired flow is its `zk_record_deadline` trick: the retired
relayer INVENTED a generous 1h `deadline` to cover its deferred broadcast, whereas the consent
`deadline` is `pub[6]` - proof-bound and device-chosen - so the relayer cannot widen it. **The
preflight margin does that job instead.**
`MIN_DEADLINE_MARGIN_SECS` is therefore 120s, not 30s: wide enough to cover a deferred-plus-retried
broadcast window. Refusing a too-near deadline up front (with an instruction to re-prove with a
further one) is the substitute for widening it, and is why deferring no longer risks an `"expired"`
revert. Do NOT narrow the margin back toward the broadcast latency - that reintroduces the doomed-tx
case the synchronous design originally existed to avoid.

The detached task drives the row to a TERMINAL state on BOTH arms (`Ok` -> `recorded` + txHash,
`Err` -> `error` + the on-chain revert reason in `tx_hash`); it must never be possible to leave a row
at `"recording"`. It deliberately does NOT gate `recorded` on a `consumed` read-back: the task only
reaches that point after a receipt with `status == true`, so the nullifier IS consumed and a `false`
read could only ever be wrong. Consequently the ack no longer carries `txHash`/`consumed`/`verified`
- those moved to the polled session row, and the route tests must assert on the SETTLED row, never
the ack, or they pass vacuously (this is exactly how the signer-coupling guard would silently
retire).

The preflight mirrors the registry's requires (field range, `addr range` on the FULL element before
narrowing, deadline, art9, relayer, whitelist) but is **not** the security boundary - the on-chain
gates are, and they run again regardless. Its only job is to avoid paying gas for a tx that cannot
mine.

**The submit writes a `VerifySession` audit row into the operator trail**
(`GET /verify/history`, `GET /verify/session/:id`) - otherwise the cutover would have silently
dropped every owner-hidden verification out of the verifier's operational record. A cold submit
(self-authenticating proof, no operator-started session) MINTS the row here with an empty
`challenge` (replay protection is the proof-bound nullifier, not an operator nonce) and
`purpose`/`recordType` stored as the bytes32 WORDS they arrive as - the labels they reduce from are
one-way, so there is no honest way to recover them. (The migration-era `mode` field is gone from
`VerifySession`; there is one flow.)
The row is written as `recording` BEFORE the broadcast and updated to `recorded`/`error` after, so a
submission that spends gas is auditable even if the process dies mid-tx; a revert stashes its reason
in `tx_hash`. It stays **owner-blind by construction**: `VerifySession` has no
`subject` field and no public signal could fill one - never add one. The response
echoes the new `sessionId`.

**Two response surfaces, and neither carries a `consumed` field.** The ack returns
`status` (always `"recording"`), `protocolVersion`, `sessionId`, `registry`, and
`nullifier` - the last is safe to echo before the broadcast because it is `pub[3]`, already known,
and it is what a caller keys its OWN `consumed` read against. The terminal result is the polled
session row (`GET /verify/session/:id`): `status`, `txHash`, `nullifier`. There is no
server-side `consumed` read-back on either surface - see the terminal-state paragraph above for why
one would be pure downside.

The registry address env var is `VERIFICATION_REGISTRY_CONSENT_ADDR`, fail-closed when unset. (Its
retired sibling `VERIFICATION_REGISTRY_ADDR` is gone from vet-api with the retired path; the name
was kept distinct during the migration because the two registries encoded different selectors.)

**Test fixtures: `relayer` is bound INTO the proof, so a fixture can only ever be submitted by the
address it names.** The committed `contracts/test/consent-fixture.json` names `0x1111…1111`, which no
key we hold can broadcast - fine for Foundry (it `prank`s), useless for a Rust relayer E2E. Hence
`contracts/test/consent-fixture-anvil.json`, the same witness rebound to anvil account 0.
`gen-consent-fixture.mjs` takes `CONSENT_FIXTURE_RELAYER` / `CONSENT_FIXTURE_OUT` overrides, both
defaulting to the original values. Changing a PUBLIC INPUT does not move the VK - same ceremony key,
same verifier.

Note the fixtures are **semantically, not byte-wise, reproducible**: Groth16 proving draws fresh
randomness per run, so re-running the generator reproduces every public value (`pub`, `R`,
`nullifier`, `recordType`, `deadline`, ...) but emits different `a`/`b`/`c`. Do not treat a diff in
the proof elements as drift, and do not regenerate a committed fixture just to "check" it - the
regenerated file will always differ.

Coverage, split by what each harness can actually prove:
- `stacks/vet/api/tests/submit_consent_onchain.rs` - real ceremony-key proof through the new
  interface against the real registry + real verifier on anvil (all 12 gates), fail-closed on
  mismatched root and consumed nullifier, and a test that verification records without any
  `ConsentKeyRegistry` (the retired-encoder cross-drive test went with the deleted retired encoder).
  Skips without Foundry.
- `stacks/vet/api/tests/submit_consent_levelb_route.rs` - the handler preflight against `MemChain`.
  Bad-root cannot live here: `MemChain` only checks `consumed`, so a mismatched root would wrongly
  succeed.

### Superseded M-4 mobile mode gate (historical only)

The `mode == "levelb"` branch below records transitional wiring. The mode gate has since been
deleted (mobile PR #71): the apps run the single owner-hidden consent flow with no protocol mode,
and the wiring below survives as THAT flow's mechanics, not as a branch.

Both native `ScanScreen` implementations kept the owner-hidden flow behind an explicit stored-session
`mode == "levelb"` branch. The flow reads the seed from `Wallet.seedHex` and the decimal tag
handle, owner address, and salted attributes from the throwing `ProfileTreeStore.load` accessor;
an unreadable owner-secret store must fail closed. It passes those values directly to
`proveConsent` (the stored `ownerSecretHex` never crosses the caller seam), with the attributes'
stored `saltHex` encoded as the FFI JSON field `salt`. Artifacts are always resolved by
version (the internal key `dogtag-levelb/1`), and the device creates a fresh 32-byte consent nonce
plus a 10-minute deadline. The latter deliberately exceeds the server's 120-second preflight floor
so the detached, retried broadcast still has room to settle.

Submit the proof (server-side the migration-era `/v1/verify/consent/levelb` alias has collapsed
into the sole submit route `POST /v1/verify/consent`), then reuse the detached-broadcast
poll primitive: query the validated `verificationRegistry` for
`consumed(pubSignals[nullifier])` while polling the same export session for
a terminal error. The nullifier is index **3**; index 4 is the public profile root `R`, and
polling it produces the classic successful-submit hang. This flow has no EdDSA signing or consent
key bind step. (The retired `proveVerification` / bind path has been deleted outright.)

The first native release carrying this path is semver **1.4.0** (build/versionCode 4). Keep both app
versions, `ProtocolVersions.sol`, `dogtag-prover-rs`'s manifest mirror, and the registry runbook at
that exact floor. The registry entry has since been published and is active: the 2026-07-23 r8
fresh redeploy deployed the `ProtocolRegistry` and executed the `dogtag-levelb/1` publication with
`minAppVersion` at that floor. `DiscoveryError` still crosses UniFFI as a
string, so update-routing currently uses the generic validation failure and a typed error surface is
a follow-up.

### iOS wiring

`apps/ios/DogTag/ProfileTreeStore.swift` builds via FFI and persists `Documents/dogtag-owner-secrets.json`.
New contents are written into an already-protected, already-backup-excluded sibling before an atomic
replacement; the previous protected file is retained until the destination's exclusion flag is reasserted.
It is also flagged **`isExcludedFromBackup`**, which is what makes it genuinely DEVICE-LOCAL, at parity
with the seed/entropy Keychain items' `…ThisDeviceOnly` class: `.completeFileProtection` governs at-rest
encryption, NOT backup inclusion, and `Documents/` rides in iCloud/Finder backups by default. Consequence for
recovery, and the thing to state correctly: the file is NOT a
cross-device backup, so recovery needs the **seed AND the credential** - the phrase re-derives the
owner-control core (owner-secret, consent key, reserved-leaf salts), while the attribute values+salts are
not seed-derivable and come back from the wrapped credential itself (`wrap_document` packs each leaf as
`"<saltHex>:<tag>:<value>"`). The seed ALONE does not reproduce `R`.
`Wallet.seedHex()` was added because the seed had NO public accessor (`loadBlob` is private).

**The seed-backup gate is load-bearing, not a nag - do not "simplify" it into a warning.** Because the
store is device-local, the phrase is the ONLY thing that regenerates an owner-secret on a replacement
phone, and `profileRoot` is write-once so there is no on-chain remedy (D3: re-issue). So
`ProfileTreeStore.buildAndPersist` THROWS `seedBackupNotConfirmed` unless
`SeedBackup.isConfirmed(forSeedHex:)` matches the exact seed supplied to the builder (`Wallet.swift`).
The shared "I've saved it" action appears on both the genesis phrase card and export sheet
(`ProfileScreen.swift`). It records an ASSERTION, not proof - it closes the SILENT failure (a tag minted
against a phrase the owner never saw), not a determined tap-through. `UserDefaults` stores only a
domain-separated SHA-256 fingerprint of the confirmed seed, so a migrated preference cannot confirm a
new `…ThisDeviceOnly` wallet. **M7 owns the other half:** the credential (values + salts) must be
re-obtainable after device loss, or a phrase backup alone still will not save the tag.

`ProfileTreeStore.upsert` is deliberately fail-closed for the write-once root: an identical root is
an idempotent retry, while a different root for the same canonical `dogTagIdHex` is rejected before
the existing witness is changed. Explicit draft/sealed state, replacement, and issuance-handoff
tracking remain deferred to M7.

The `DogTagTests` target (see "iOS unit tests") does not reach this code: it is host-less and FFI-free,
while `ProfileTreeStore` builds through the FFI. So the Swift side here is still covered only by
`swiftc -typecheck` (the recipe in "Getting real Swift signal without the xcframework") and every
assertion worth making lives in the Rust tests instead.
Adding the FFI export forced regenerating BOTH `apps/ios/DogTag/dogtag_standard.swift`
and the Android `.kt` - a clean regen is **purely additive**; if you see removals, your local uniffi
≠ 0.28.x and you should stop rather than commit the churn. Canary it by regenerating BEFORE your
change and diffing against the committed file: it should be byte-identical.

### M6 app-side - recovery is re-issue (D3), never a rebind

M6 is deliberately SMALL: the deployed contract already forces the whole model, so it needs NO
contract change.
`mintCustodial` + write-once `profileRoot` + permanent burn-retirement mean a lost owner-secret has
no on-chain repair, and `CustodialIssuance.t.sol::test_no_recover_surface` already pins that
`recover()` does not exist (a keyed rebind would name the new owner on-chain, which is exactly what
the owner-hidden model removes).
M6 makes the re-issue path a first-class, tested device flow.

Recovery has two branches, decided by whether the owner-secret can be regenerated
(`docs/MOBILE_OWNER_SECRET.md`):
- **Repair** - the owner still has seed + credential, which rebuild the same `R`; the same tag keeps
  working (`crates/dogtag-standard-rs/tests/device_recovery_journey.rs`).
- **Re-issue (D3)** - the owner-secret is gone for good, so the remedy is a fresh custodial issuance
  under a NEW `dogTagId` with a new `R`.
  The forks are FORCED by the model, not chosen: a fresh id (a burned/abandoned id is retired forever,
  `test_burned_dogTagId_can_never_be_reminted`), a new `R` (write-once), and the old tag left
  abandoned (no on-chain remedy).

Device side: `ProfileTreeStore.reissue(...)` builds the fresh tag and marks the abandoned record,
keeping the old<->new link **device-local only** (the store is `isExcludedFromBackup` and never
transmitted).
The re-issued tag is mutually unlinkable from the abandoned one because the owner-secret is bound to
`dogTagId`, so even the SAME wallet's fresh tag derives an independent nullifier secret - pinned
end-to-end through the FFI by `tests/device_reissue_journey.rs`.
**Never** surface the old<->new link in an on-chain event, a `setStatus` reason, or an issuer record:
that would reintroduce the owner linkage the recent ZK audit validated as absent.
The seed-backup gate (`StoreError.seedBackupNotConfirmed`) applies to `reissue` exactly as to
`buildAndPersist` - the fresh tag's owner-secret is just as reliant on the phrase.

**Referencing credentials across issuers - accept the break (captain decision, 2026-07-16).**
A re-issued pet gets a NEW `dogTagId`, so any credential another issuer (vet/government) previously
anchored to the OLD id now points at the abandoned tag - the retired owner-revealing generation
preserved these across `recover()` via a stable `tokenId` + `issuerOf`, but owner-hidden re-issue
deliberately does NOT.
Prior attestations are **not** re-anchored or transferred to the new id: doing so would forge
attestation applicability (a vet/gov signature applying to an id it never signed), breaking the
cross-issuer trust model.
The owner re-obtains each referencing credential fresh from its issuer under the new id, via normal
issuance.
**M6 ships the device/app re-issue flow + these semantics + docs/tests only - there is NO
re-issue-AWARE issuer endpoint.**
Read that precisely, because **M-2 has since wired a custodial issuance path into vet-api**
(`POST /profiles/issue/custodial-bind` - see "Level-B custodial issuance bridge" above): the
statement "the custodial path is not wired until M7" is no longer true.
What M-2 provides is the MECHANICAL half a re-issue needs - an operator starts a fresh session (which
allocates a fresh `dogTagId`) and the device posts the new `R` - so a re-issue can be performed today
by simply issuing a new tag.
What does NOT exist is any issuer-side notion OF a re-issue: nothing marks the abandoned tag, links
old to new, or drives a re-issue-specific operator flow - and per the paragraph above that link must
never reach an issuer record anyway, so the old<->new association stays device-local.
The owner-hidden custodial route is the end-state, and the cutover is complete: the device call site
landed (mobile PR #71) and the owner-revealing wallet-mint route was deleted (backend PR #72) - it
must never be rebuilt as a default, opt-in, or fallback.

### Legacy-wallet rescue + the Danger zone (iOS Profile)

A wallet stored **before the BIP-39 entropy was persisted** has a live seed but no reconstructable
phrase (`Wallet.revealMnemonic()` → `nil`). That state used to be a **permanent dead end**, and the
shape of it is worth remembering because every piece looked correct on its own:
`ProfileTreeStore.buildAndPersist` gates issuance on `SeedBackup.isConfirmed`; the only thing that can
set that flag is the "I've saved it" button; and that button renders only inside
`if let m = mnemonic`. No mnemonic → no button → no confirmation → **no dog tag, ever**. There was no
reset path anywhere in the app, and because both Keychain items are
`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, they **survive app deletion** — so reinstalling did not
clear it either.

- **The gate is not the bug; do not loosen it.** Letting a user confirm they backed up a phrase the
  app cannot show them would be a lie that costs them a tag. The resolution is
  `Wallet.replace()` — destroy the Keychain seed + entropy and the stale confirmation, then run
  genesis — surfaced as "Replace wallet…" in the export sheet's no-phrase branch, deliberately placed
  **below** the private-key export, since that key is the only thing that survives. Normal wallets are
  untouched: a reconstructable phrase still requires the real "I've saved it".
- **The rescue has TWO entry points on purpose.** Besides the export sheet, the Danger zone leads with
  a callout whenever `Wallet.hasExportablePhrase()` is false — that is where someone who is stuck goes
  looking. It is also the VERIFIED route: the export-sheet hand-off cannot be exercised in the
  Simulator (it needs a wallet, which needs the biometric gate), whereas the callout uses the same
  row -> sheet path `wallet_reset.yaml` covers. Both routes hand off through Profile's single
  `.sheet(item:)` route binding by nil-ing it and presenting on the NEXT runloop turn. Swapping the
  item's identity in place was tried and reversed: it relies on SwiftUI honouring a dismiss and a
  present coalesced into one update, which it drops, leaving the button looking dead.
  `hasExportablePhrase()` is a presence-only Keychain query (no
  `kSecReturnData`), so a view can ask on every render without pulling the entropy into memory —
  `exists()` now does the same instead of loading the 64-byte seed to check for it.
- **`Wallet.deleteKeys()` drops the seed BEFORE the entropy.** `exists()` keys off the seed, so if the
  entropy delete fails the app is already in first-run state. The reverse order could drop the entropy
  while leaving a live seed — manufacturing exactly the phrase-less wallet this exists to escape.
- **`AppReset`** owns delete-wallet / dog-tags / records / reset-everything. `resetEverything()` is
  ordered data-first, wallet-last AND short-circuits - `guard outcome.isComplete` skips the wallet
  wipe entirely once any data sweep leaves something behind. Ordering alone would not achieve this:
  the data sweeps carry the unrecoverable material (the attribute salts), so the seed must SURVIVE a
  partial sweep, which is what keeps the failure actionable (export the keys, then retry). Its other
  half is mandatory - the caller MUST report that the wallet was deliberately KEPT
  (`DangerAction.partialNote`), or a partial reset reads as "some of it silently went missing" and
  the stated remedy looks impossible.
- **`LocalDataSweep` also removes `ProfileTreeStore.write`'s `.<name>.<token>.tmp`/`.bak` staging
  siblings.** They hold the same owner-secret material as the destination and a crash can leave one
  behind, so a sweep that took only the canonical file would leave the secret it promised to destroy
  readable on disk. Partial sweeps are reported as partial — never as success.
- **The warning contract is load-bearing, not boilerplate.** Nothing on-chain is deleted, but `R` is
  write-once and `proveConsent` re-derives the owner-secret from the seed on every proof, so a tag
  whose seed *or* salts are gone can never prove consent again — not by re-import, not from a backup
  (D3: re-issue under a new `dogTagId`). Every confirmation states what dies AND what survives, and
  each demands its **own** typed phrase ("DELETE DOG TAGS", not a shared word a thumb already knows).
- Coverage: `maestro/wallet_reset.yaml` (the typed gate, incl. near-miss and cross-action phrases),
  `SeedBackupGateTests` (the gate + `Bip39`), and `LocalDataSweepTests` (staging-sibling sweep,
  idempotence, partial-failure reporting). The gate's far side is device-only — see "No
  biometric-gated flow is verifiable on a simulator".

### Known-uncovered surfaces (deliberate, not oversights)

- **The iOS `ProfileTreeStore` has ZERO automated-test coverage.** The `DogTagTests` target (see "iOS unit tests")
  cannot reach it: that suite is deliberately FFI-free and `ProfileTreeStore` builds through the FFI,
  so it stays out of scope until the pure logic is extracted. Both sides are now called in
  production - the present flow calls `ProfileTreeStore.load()` and the custodial-bind QR issuance
  flow (`ScanScreen` -> `buildAndPersist` -> `POST /profiles/issue/custodial-bind`, landed with the
  mobile cutover) exercises the WRITE side - but the Codable-encode round-trip, the
  atomic/`.completeFileProtection` write and `verifyRecoverable` still have no automated coverage.
  The Android namesake is a SEPARATE coverage story - see the next bullet; do not read this one as
  covering both.
- **The Android `ProfileTreeStore`'s device-side half is uncovered too, though its pure logic is
  not.** M-2b deliberately kept the parts worth pinning `Context`-free so `gradle test` could reach
  them without an emulator: `OwnerSecretRecords` (codec + write-once upsert), `SeedBackup.fingerprint`
  and the whole `ProfileTreeBuilder` are covered, and `OwnerSecretRecoveryJourneyTest` joins the codec
  to the real Rust core over the full recover-on-a-new-phone path. What is NOT covered is everything
  that needs a real device: the Keystore envelope and its `StrongBox → unlockedDeviceRequired → plain`
  ladder, the `noBackupFilesDir` placement, and the `.bak`-parking write/`load()`-promote sequence.
  Those need an instrumented test, not a JVM one. The ladder in particular degrades downward by
  design and reports it only as a `Log.w` - discoverable in logcat, but invisible to the test suite -
  so a regression that lands every device on the plain rung would not turn a single suite red.
- **The `[u8; 20] -> Fr` owner-address packing is untested.** The parity test feeds an `Fr` straight
  to `hash_reserved_leaf`, bypassing `build_profile_tree`'s `field_from_scalar_bytes(&addr)`. It is
  the documented address-packing primitive and the device is the sole builder (no external encoder to
  disagree with), so the risk is low - but no test pins it.
- **`hash_reserved_leaf` / `build_profile_tree` are Rust-only.** `packages/dogtag-standard-ts` has no
  equivalent, so the usual "three legs in lockstep via testvectors.json" invariant does not yet cover
  the profile tree. Fine while the native holders use the Rust core; the **web holder**
  (`stacks/owner/web`) would need the TS leg before it can build an owner-hidden tag.
- **No leaf-count guard.** `consent.circom` is instantiated at `depth=6` (~64 leaves). A tree larger
  than that produces an `R` no proof can be made against, and `build_profile_tree` will not stop you.
  Realistic pet credentials are far under it.

> Note: the ONLY committed UniFFI bindings are the live `apps/` pair (the stale crates-local
> `crates/dogtag-standard-rs/bindings/{swift,kotlin}/` snapshot, which nothing consumed, was deleted
> in the final cleanup slice). iOS CI regenerates and copies the live Swift binding, but Android CI
> rebuilds only the native `.so` and consumes the committed `apps/android/.../dogtag_standard.kt`
> unchanged. Regenerate and commit BOTH live `apps/` bindings after every FFI change.

---

## Groomer role gate + the shop CRM (clients / appointments / all verifications)

### The groomer IS the vet binary — `BUSINESS_TYPE` is the only difference

`stacks/groomer/` has no `api/src`: the groomer backend runs `target/release/vet-api` with
`BUSINESS_TYPE=groomer` (see `scripts/demo-up.sh`, `stacks/groomer/docker-compose.yml`,
`scripts/e2e-zk.sh`). So "remove X from the groomer" is a ROLE question, never a delete.

`Config::business_type` drives `Config::issuance_enabled()`, and `routes::public_router` collects the
issuance routes into a sub-router it does not merge when the role is `groomer`. The gated set is the
FULL issuance surface — `/credentials/prepare`, `/credentials/confirm`, `GET /records`,
`/records/{id}/revoke`, `/records/{id}/share`, `/records/{id}` (GET **and** PATCH), `/r/{token}`,
`/profiles/issue/session/start`, `/profiles/issue/session/{id}`, `/profiles/issue/custodial-bind`,
`/p/{token}`. Anything issuance-only added later must be gated too, or the groomer silently regrows
an issuer surface. The role **fails open**: anything that is not the literal `groomer`
(case-insensitive) keeps the full issuing surface, so a typo'd or absent `BUSINESS_TYPE` can never
silently strip a live vet of issuance. `tests/role_gating.rs` pins both directions.

The web apps are already separate (`stacks/{vet,groomer}/web`), so portal-side removals do not touch
the vet at all — only the shared `packages/ui` and the shared API need role thinking.

#### Distinguishing "route not mounted" from "handler said 404"

Axum's route-miss returns 404 with an **empty body**; every handler here returns `{"error": ...}`.
A role-gating test must assert on both, or a legitimate 404 (unknown record id, expired token) reads
as a missing route. See `route_absent()` in `tests/role_gating.rs`.

### `/v1/appointments` is already taken by the Phase-7 central replica

`ApptReplica` + `/v1/appointments*` mirror CENTRAL-owned cross-business bookings (central is the sole
`rev` allocator). The shop's OWN booking book is a different entity (`store::Appointment`) on the
UNVERSIONED `/appointments` — matching the convention that `/v1/*` is for cross-service callers
(central HMAC, the owner's phone) and unversioned paths are operator-facing. The CRM routes
(`/clients`, `/appointments`, `/verifications`) are mounted for ALL roles: a booking book is not
role-specific, and mounting them everywhere keeps the vet's behavior unchanged.

### Adding a `Store` entity: four places, and `cargo test` only checks three

1. the struct + trait method in `store.rs`
2. `MemStore` impl
3. `MongoStore` impl — **behind `#[cfg(feature = "mongo")]`, so `cargo test` does NOT compile it.**
   Run `cargo check -p vet-api --features mongo` explicitly or the mongo build breaks silently.
4. new fields on an EXISTING persisted struct need `#[serde(default)]`, or live Mongo rows written
   before the field existed fail to deserialize.

`Config` is built by struct literal in `main.rs`, `app.rs`'s test module, `tests/common/mod.rs`,
`tests/gate_dual_signing_parity.rs`, `tests/submit_consent_levelb_e2e.rs` and
`tests/role_gating.rs` — grep `Config {`, do not assume the count.

### The verify session's terminal sites

Anything that must observe a verification's outcome has to hook every site where the session's status
settles. Since the owner-hidden collapse there are two, both inside the `tokio::spawn` in
`verify::consent_submit_levelb`: the record success (`recorded`) and the record failure (`error`).
`crm::attach_evidence` runs once before the spawn, at the `recording` write, because that is where
the opaque `pub[dogTagId]` and the validated disclosed keyPaths are known. The spawned task clones
`store`/`chain` out of `AppState`, so such a hook must take `&Arc<dyn Store>`, not `&AppState`.

`attach_evidence` upserts rather than requiring an existing row, so a COLD submit (no
operator-started session, hence no `crm::start_log`) still lands in "All verifications".

### Privacy invariants that outrank convenience

- `GET /x/{token}` is UNAUTHENTICATED — anyone who scans the QR reads it. Its response is exactly
  `{sessionId, relayer, purpose, recordType, challenge, unverifiedClaims}`. Never add client,
  appointment or pet context there; the linkage lives server-side only.
- The shop's verification history deliberately does NOT store the owner's `subject` wallet. It is
  derivable from the tx, but persisting it creates a client -> wallet linkage the protocol withholds
  from a verifier. (There is no `subject` on the owner-hidden path to store in the first place.)
- `VerificationLog` mirrors `VerifySession::disclosed_key_paths` — keyPaths only, never the disclosed
  VALUES, which are shown to the operator at the time of the check and never persisted. On an
  ordinary owner-hidden verification the list is EMPTY, and that emptiness is the guarantee, not a
  gap: never backfill it. `tests/crm.rs` pins these.
- There is no verification `mode` anywhere in this layer. The product is one owner-hidden flow (see
  "Product model"), so the history has no mode column, no mode filter and no mode badge — it reports
  what the owner chose to disclose instead.

### The dogTagId a test must assert under is the FIELD-HASHED one

The DOG_PROFILE SBT is minted under `field_of_value(Integer(handle))` (the circuit's `pub[0]`), not
the raw operator-facing handle — otherwise the owner's later consent proof fails the
`R == profileRoot(dogTagId)` binding. A test reading `ownerOf`/`profileRoot` must hash the handle the
same way; `tests/custodial_issuance_bridge.rs` does this and additionally pins the raw-handle case
as fail-closed.

### Live-validating without disturbing a running demo stack

Boot a second instance from your own worktree on a spare port (`PORT`, `CUSTODY_SEAL_PATH`) rather
than restarting the shared one — the shared instance's custody unlock is in-memory, so a restart
strands it until someone re-enters the passphrase.

Whitelisting a fresh relayer needs the WHITELIST_ADMIN key from `contracts/.env`, which is
**gitignored and therefore absent from every fresh worktree** — so `demo-bootstrap.sh` and
`e2e-zk.sh` cannot run there. The fallback is the central admin API's
`POST /v1/issuer-applications` + `/{id}/approve`, which whitelists `VERIFY:<purpose>` as the deployer.
Caveat: approve whitelists on the registry **that running admin-api instance was started with**,
which may not be `contracts/deployments/roax.json`'s current `IssuerRegistry` — check the tx `to`
address before concluding the whitelist failed.

### What the groomer portal no longer has

`Records.tsx`, `Issue.tsx` and `Traceability.tsx` were removed from `stacks/groomer/web` (with their
e2e specs) — a groomer verifies and does not issue, and the on-chain event console is not its business
history. The VET keeps all three, untouched. The groomer's `Traceability` nav slot became **"All
verifications"** (`pages/Verifications.tsx`): the shop's own searchable history, joined to appointment
and client. Statements elsewhere in this file about the two portals SHARING a Records page, or about a
groomer `Traceability.tsx`, describe the pre-CRM layout.

The verify surface is one component, `@dogtag/ui`'s `VerifyFlow`, used by BOTH the appointment-linked
flow (`AppointmentDetail.tsx`) and the ad-hoc one (`Verify.tsx`), so a verification means the same
thing however it was started. It offers NO mode/disclosure choice: the retired ZK-vs-Normal selector
was removed from `VerifyFlow` when the backend collapsed to the single owner-hidden submit route, and
`stacks/groomer/web/e2e/verify.spec.ts` asserts neither "Mode" nor "Normal" appears.

## Mobile: exercising either app's UI without a scan flow

Both apps persist to two plain JSON files (`pets.json`, `credentials.json`) in the app's private
directory, and read them at startup, so a UI state can be staged directly instead of driving a QR
import. On the iOS simulator:

```bash
DATA=$(xcrun simctl get_app_container <udid> io.liberalize.dogtag data)   # write $DATA/Documents/*.json
```

Records whose `wrappedDocJson` is hand-written will fail the offline integrity check and never reach
the on-chain branch. Generate genuinely valid ones with `wrapDocument` from
`packages/dogtag-standard-ts` (the TS mirror of the Rust canonicalization) - scalars must be typed,
e.g. `{tag: 2, value: "…"}`.

**A worktree's `DogTagFFI.xcframework` can be stale**, and it reads as a source break. It is
gitignored, so it does not update with a checkout. If the Swift build fails with
`cannot find 'uniffi_dogtag_standard_checksum_func_…' in scope` in `dogtag_standard.swift`, the
vendored xcframework predates the committed UniFFI bindings - rebuild it (see "Building the mobile
(iOS) holder app") rather than editing the bindings. Compare the two symbol sets first:

```bash
comm -13 \
  <(grep -o "uniffi_dogtag_standard_checksum_func_[a-z0-9_]*" \
      apps/ios/DogTagFFI.xcframework/ios-arm64-simulator/Headers/dogtag_standardFFI.h | sort -u) \
  <(grep -o "uniffi_dogtag_standard_checksum_func_[a-z0-9_]*" \
      apps/ios/DogTag/dogtag_standard.swift | sort -u)
```

Empty output means the header covers the bindings. The FFI is arm64-only, so build the simulator
slice with `ARCHS=arm64` (a generic-simulator build fails to link x86_64).

## Verification verdicts are point-in-time, and must not fail open

A stored `Credential.verdict` is whatever the chain said at import; it does not track later
revocations. `CredentialRefresher` (iOS `RecordImporter.swift`, Android `data/CredentialRefresher.kt`)
re-reads it, and `lastCheckedAt` is what makes an old answer visibly old.

When a chain read is inconclusive the verdict is **UNVERIFIED with the reason, never VALID** - not
being able to check is not the same as checking and finding the record good, and the whole point of
a revocation check is that distinction. `verdict`, `verdictReason` and `lastCheckedAt` are always
written together so they describe the same check. Two anchor shapes exist: an ordinary record is
anchored in a DogTagIssuer clone (`isValid(root)`), while a `DOG_PROFILE` is anchored in the
DogTagSBT (`profileRoot(dogTagId)`), so any re-check has to branch on `issuer.documentStore`.

The SBT branch checks the profile root ONLY. There is deliberately no owner comparison: the tag is
custodial under the owner-hidden model, so `ownerOf` is the neutral custodian and says nothing about
the holder - the retired mobile `RoaxRpc.ownerOf`/`Net.swift` reads went with the owner-revealing
layer. Never reintroduce an ownership check here to "strengthen" the refresh; it would fail every
legitimately-held tag.

**The no-fail-open rule cuts BOTH ways: an unestablished answer must not be stamped INVALID either.**
Asserting INVALID from a read that never resolved tells an owner their genuine credential is bad,
which is the exact mirror of the fail-open. Two SBT-branch traps, both fixed and both easy to
reintroduce: (1) `dogTagIdFieldHex` THROWS for a non-decimal handle, and the importer stores a
32-hex share token as `dogTagId` whenever the wrapped doc carries none - so never fall back to the
raw handle, which `padUint` silently mangles into a lookup of an unset slot; UNVERIFIED "this dog
tag id could not be resolved on-chain" is the answer. (2) An UNSET `profileRoot` slot returns an
all-zero 32-byte word that passes every well-formedness guard, so a never-anchored tag would compare
as a mismatch; all-zero is UNVERIFIED "no profile root is anchored on-chain for this dog tag", and
INVALID is reserved for a real non-zero root that genuinely differs. Both reason strings are
byte-identical across the two ports on purpose - string drift is how the two refreshers diverged
before. An EMPTY/short return is deliberately NOT distinguished: both `RoaxRpc.profileRoot`
implementations collapse it to nil, so it already lands on UNVERIFIED, just with the
could-not-reach-the-chain reason.
Known drift hazard, accepted: on Android the unset-slot predicate now lives in TWO places -
`CredentialRefresher`'s inline `drop(2).all { it == '0' }` and `RoaxRpc.classifyProfileRoot`
(which the scan-time poll uses) - because the helper folds a null RPC read and an unset slot into
one `Pending` while the refresher needs a distinct reason string for each; revise both together.

The delete confirmation resolves its own pet label, on BOTH ports: iOS inside
`confirmDeleteCredential` (via `LocalStore.petDisplayName` + `PetLabel.line`) and Android inside
`DeleteCredentialDialog` (via `List<Pet>.petLabel`). Never reintroduce a caller-supplied name
parameter - two callers passing different fallbacks (`""` vs `"DogTag #<id>"`) is exactly how the
same record came to be named two different ways depending on which screen raised the dialog.

## On-chain provenance in the audit surfaces (government Oversight, vet Traceability)

The oversight indexer's demo mode makes the activity feeds **entirely synthetic**, and nothing in the
payload says so. `INDEXER_DEMO_MODE` / `DEMO_MODE` / `VITE_DEMO_MODE` (`stacks/indexer/api/src/main.rs`)
swaps `AlloyLogSource` for a `MemLogSource` seeded by `demo_seed()`, which emits placeholder identifiers
- `txHash` `0x0100`…`0x0800`, `blockHash` `0x01`…`0x08`, blocks 1-8, roots `0x1111…`/`0x2222…`. It also
uses the REAL registry addresses, so the rows look plausible. Worse, `routes.rs` composes
`txUrl = EXPLORER_BASE/tx/<hash>` **unconditionally**, so every synthetic row arrives carrying a
live-looking `https://explorer.roax.net/tx/0x0800` that resolves to nothing.

The government `/health` `demo` flag does NOT cover this: it is the API's ephemeral-store flag, and the
topbar's `LIVE CHAIN` badge describes the ISSUANCE backend. A stack can therefore be truthfully live on
chain 135 while its Oversight table is 100% scripted.

The fix lives in `packages/ui/src/chain/` (`provenance.ts` + `ChainValue.tsx`), shared by BOTH portals so
they cannot fork into separate dialects:

- `chainProvenance(ev)` returns `synthetic` unless `txHash` is a well-formed 32-byte value. This is
  arithmetic, not a demo-mode guess - an EVM transaction hash is keccak256 output, so `0x0800` cannot
  address a transaction on any chain. It therefore also catches a single synthetic row inside an
  otherwise-real feed, which a header badge cannot.
  **Both badge labels are deliberately about SHAPE and symmetric** - `chain-addressable` /
  `not chain-addressable`, never `on-chain`. No chain read happens on this path, so a green badge
  claiming the transaction EXISTS would be asserting a fact from arithmetic, the same over-claiming the
  module exists to remove. The `data-testid`s stay `provenance-onchain` / `provenance-synthetic`.
- `txExplorerHref(ev)` returns `null` for a synthetic event **even when the API supplied a `txUrl`**;
  callers render the hash inert rather than linking. Prefer the API's `txUrl` over composing one - the
  deployment's `EXPLORER_BASE` need not be the ROAX default. When composing, it delegates to
  `explorerTxUrl`/`explorerAddressUrl` (`packages/ui/src/wallet/chain.ts`) so the explorer path has one
  home and this module cannot drift from what the rest of the portals link to.
- `eventDetailFields(ev, joined?)` labels every identifier. `recordType` arrives EITHER as a human label
  (`TRAVEL_CLEARANCE`) or as its keccak key depending on whether the indexer's directory reversed it, so
  the label follows the value's shape (`isHash32`). The optional `ChainDetailContext` folds in what the
  portal's OWN joined row knows, and exists because the chain payload is owner-blind and label-free:
  `purpose` is only ever `keccak256(label) % r`, and **`RootIssued`/`RootRevoked` carry no `recordType`
  either** (`chain.rs` sets it only for `issuerCreated`/`whitelisted`/`delisted`), so on those rows and
  on `verified` the readable values can come from nowhere else. The context can only make a value more
  readable or drop a duplicate - `eventDetailFields` resolves `ev.recordType ?? joined.recordType`, so a
  join can never override what the chain itself asserts. The row-expansion panel deliberately passes
  none of it (that panel is the raw chain payload).
- **`joinedDetailContext(local)` builds that context, and BOTH consoles must go through it.** It exists
  because government and vet each assembled the context themselves and so rendered the SAME on-chain
  event with different facts - an operator comparing the two consoles saw them disagree. It encodes one
  doctrinal rule: a `joinedBy: "dogTagId"` join is TAG-granular, proving only that the tag is one this
  operator credentialed and never WHICH credential was verified (the owner-hidden `Verified` binds no
  root and no record type), so such a join lends the event neither its `recordType` - that would assert
  the very thing the event does not establish - nor its `dogTagId`, which the row's join cell already
  shows in readable form. Every other join is by anchored root or tx hash, an exact match, so its record
  type describes that event and is shown. Do not re-open-code this at a call site.
- `emittingContractRole(type)` names what `contract` is: it is NOT always the issuer clone -
  `issuerCreated` **and `rootRegistered`** come from the factory (`chain.rs` decodes both only when the
  emitting address equals `ctx.factory`), `whitelisted`/`delisted` from the IssuerRegistry, `verified`
  from the verification registry. Only `rootIssued`/`rootRevoked` are emitted by the clone itself.
- `emittingCloneName(ev)` gates the clone's human name on `contract == clone`. The indexer resolves
  `cloneName` from `ev.clone`, which on a factory-emitted row is the clone the factory ACTED ON - so
  rendering it under the emitting address makes the factory read as that named clone. The clone and its
  name stay together in the expansion panel, where they are labelled as the clone.

Two table-layout traps, both of which produced visibly broken output before being caught by screenshot:
the `<TableHead>` order and the `<TableCell>` order are independent and silently render mismatched
columns if you reorder one; and a `ChainValue` with an inline label cannot shrink (the label is
`shrink-0`), so in a width-capped column its content overflows and collides with the next cell - pass
`stacked` in dense columns. Verify layout by measuring
`table.getBoundingClientRect().width - .overflow-x-auto.clientWidth` at a 1512px viewport, not by eye.

**Only an OPAQUE identifier may be middle-truncated, and `shortValue` is the one place that decides.**
`shortHex` is a middle-truncator, which is right for hex - head and tail identify it, the middle is
noise - and exactly WRONG for a human label, where the middle carries the meaning. `ChainValue` used to
apply it to every value, so at the tables' `head = 8` the government's flagship record type rendered as
`TRAVEL_C…ARANCE` (and `SERVICE_ATTESTATION` as `SERVICE_…TATION`, an `issuerCreated` name as
`DogTag G…hority`) - corruption, not elision, on the cell that says what kind of record an event
concerns. `shortValue` now gates on `isOpaqueIdentifier` (`0x`-hex, or an all-digit field element such
as the decimal `dogTagId`); human text is returned whole and left to CSS `truncate`, so it degrades by
clipping while staying complete in the DOM, the `title`, and the copy affordance. Do not reach for
`shortHex` directly in a component - a new call site is how the mangling comes back.

The row-expansion panel renders values in full via `ChainValue`/`TxRef`'s `full` prop, which swaps
`truncate` for `break-all` (`break-words` for human text, which has word boundaries to break on):
**dropping the `head` truncation alone is not enough**, because `truncate` would still clip the value in
CSS - the string would be in the DOM (so a `toContainText` assertion passes) while the reader still
cannot see it. `labelHidden` is the companion for a caller that prints the label itself; it keeps the
label for the copy button's accessible name rather than degrading it to `Copy full ` as an empty
`label` string did.

`CopyButton` falls back to a hidden-textarea `document.execCommand("copy")` when `navigator.clipboard`
is unavailable, and shows a FAILED state when both paths fail. Not legacy nostalgia: `navigator.clipboard`
is undefined in any non-secure context, and these portals are routinely served over plain `http://` on a
LAN - so without the fallback the button is dead in exactly the demo topology. It has to be visible
rather than silent because middle-truncation removes characters from the STRING, so for an opaque
identifier the elided characters are not in the DOM at all and copy is the operator's only route to the
full value (a human label is merely CSS-clipped, so it does survive in the DOM). Its failure message may
only name a fallback every
consumer HAS: the row expansion is government-only (vet/groomer Traceability has no expander), so the
message points at the value's own hover text, which both portals render.

The e2e fixtures must use well-formed 32-byte hashes or every row reads as synthetic; `oversight.spec.ts`
and `traceability.spec.ts` were updated accordingly and now assert both provenance verdicts explicitly.

### Running the portal Playwright specs by hand WRITES to a live backend

`vite preview` **honours `server.proxy`** in these portals, so serving the app on your own port does
NOT give you your own backend: `/api` on any port you serve from proxies to
`VITE_GOV_API_PROXY || http://localhost:44832`. "My own port" is not "my own backend" - a crew that
carefully picked a spare port still drove the captain's live government API on ROAX chain 135.

Only ONE government spec mocks the backend. `e2e/oversight.spec.ts` intercepts with `page.route` +
`fulfill` and makes no backend calls. `e2e/government.spec.ts`, `e2e/receipt.spec.ts` and
`e2e/records-crud.spec.ts` are deliberately unmocked live-portal drivers: between them they issue
credentials, edit, revoke and expire, each anchoring on-chain. So `npx playwright test` with **no file
filter** writes real records to whatever backend the proxy points at. That has happened once (five
records created, later revoked with captain authorisation).

Two safe forms:

    GOV_URL=http://localhost:<your-port> npx playwright test e2e/oversight.spec.ts
    VITE_GOV_API_PROXY=http://127.0.0.1:9 npx vite preview --port <your-port> --strictPort

Note that `government.spec.ts` reads `/api/health` through `page.request`, which BYPASSES `page.route`
entirely - so "this spec mocks" is never by itself proof that nothing escapes to a real backend.

All five `stacks/vet/web/e2e` specs mock, so the vet suite is safe unfiltered.

Neither `playwright.config.ts` is in `pnpm test` or CI (both need a served portal + browsers). The
practical consequence, seen for real: an assertion that could never pass sat in the tree and the
pipeline's test step did not catch it. Every e2e assertion on these portals is only as good as
someone remembering to run it by hand, so run the relevant spec before claiming it passes.

### Rendering a portal against YOUR OWN backend, with the proxy trap disarmed

The safe counterpart to the section above, and the cheaper way to verify a portal surface end-to-end
when the spec you would otherwise run is unmocked (or, as for `stacks/admin/web`, does not exist).

Every portal has an **absolute API-base override**, which is strictly better than pointing the proxy at
a dead port: an absolute base takes `/api` out of the picture entirely, so `server.proxy` is never
consulted and the captain's backend is unreachable by construction rather than merely failing closed.
Note #88 recommended the dead-port proxy trick for government even though `VITE_GOV_API_BASE` existed.

| Portal | Var | Declared in | Default |
|---|---|---|---|
| admin | `VITE_CENTRAL_API_BASE` | `src/lib/env.ts` | `/api` |
| government | `VITE_GOV_API_BASE` | `src/lib/api.ts` (**not** `env.ts`) | `/api` |
| vet | `VITE_VET_API_BASE` | `src/lib/env.ts` | `/api` |
| groomer | `VITE_GROOMER_API_BASE` | `src/lib/env.ts` | `/api` |

**Vet and groomer additionally take `VITE_CENTRAL_API_BASE`, and its default is already the ABSOLUTE
`http://localhost:39742`** - not `/api`. Overriding only the portal's own base therefore still leaves
central calls pointed at a shared backend on the default port, with no proxy involved to notice it.
Override both when serving either of those two.

    VITE_CENTRAL_API_BASE=http://127.0.0.1:<your-mock-port> npx vite build
    npx vite preview --port <your-port> --strictPort

It must be set at **build** time, not preview time: `import.meta.env` is inlined by the bundler, so
exporting it before `vite preview` alone does nothing.

Portal auth is a localStorage token, so a mock needs no login endpoint - seed it directly
(admin: `window.localStorage.setItem("admin.token", "anything")`) and the mock can accept any bearer.
Only admin's key was verified by this run; check the other portals' `AppContext.tsx` for theirs.

This is how the admin Activity provenance work was verified: a ~120-line `node:http` mock served
`GET /v1/admin/activity` with a deliberately MIXED feed - real 32-byte tx hashes alongside a scripted
`0x0800` row - which is the case that matters, because a wholly-synthetic fixture cannot distinguish a
per-row arithmetic gate from a feed-level demo-mode flag. Remember CORS headers on the mock, since the
app's origin is now a different port from the API's.

Kill the servers you started by the PID **on the port you chose**
(`lsof -nP -iTCP:<port> -sTCP:LISTEN -t`), never by matching a path fragment: many checkouts share a
`target/release/<name>` path and a fleet has killed a captain's live service that way.

### Reading credential state straight from chain 135

`isValid(bytes32)` = `0x6a938567`, `isRevoked(bytes32)` = `0x4294857f`, `issuedAt(bytes32)` =
`0x6240dded`, called on the per-recordType issuer clone (`TRAVEL_CLEARANCE`
`0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F`, `EU_HEALTH_CERT`
`0x421cacf2a526726635fe16ac2c26d3f95c7726de`) via `eth_call` against `https://devrpc.roax.net`. Useful
to verify a revoke rather than trusting the API's echo.

Confirmed by that read: **`expired` is a document-borne status with no on-chain effect**. A record the
store reports as `expired` is still `isValid=true` on chain, so expiry alone does not revoke.

## Pets are a collection of their own (stored inside their owner)

A pet is **addressed** in its own right (`/pets`, `/pets/{petId}`, `GET/PUT/DELETE`,
`POST|DELETE /pets/{id}/dogtag`, `GET /pets/{id}/credentials`) but is still **stored** embedded in its
owner's client document — `Client.pets: Vec<ClientPet>`, there is no pets collection on disk. Three
consequences that are easy to get wrong:

- **Every pet write is a client write, and must patch ONE pet.** `PUT /clients/{id}` is a whole-document
  replace, so a pet route implemented the same way silently deletes every pet the caller did not mention.
  `crm::mutate_pet` re-reads the owner, mutates only the addressed pet, writes the document back, then
  `rebuild_search_key()` + `resync_client_labels()` (the calendar and settled history denormalize
  `petName`). `tests/pets.rs` pins sibling survival, including a sibling's appointment link.
- **Paging cannot be folded over `list_clients`.** `total` must count PETS and a client page boundary
  falls between clients, so `Store::list_pets` is a real store query: MemStore flattens then sorts,
  Mongo uses `$unwind` + `$facet`. Order is `updatedAt desc, clientId asc, petId asc` — all three keys
  are load-bearing, because siblings share their owner's `updatedAt` exactly and without the `petId`
  tiebreak the pager both repeats and skips rows.
- **Pet search must not reuse `Client::search_key`.** That key concatenates every pet of the client, so
  a needle matching one pet matches all its siblings. `PetRow::search_key()` is per-pet (pet fields +
  the OWNER's name, so either half finds it); the Mongo side recomputes it in an `$addFields` stage and
  needs `$ifNull` on every part, since one absent `dogTagId` would otherwise null the whole `$concat`.

`GET /pets/{id}/credentials` is the read side of `POST /import/pull`, which wrote the accepted document
to the per-tag `client_cache` that nothing ever read back — an imported credential was stored and then
unreachable. It returns the document and **no verdict**: validity is an on-chain fact that changes after
the import, so the caller re-checks the chain.

`GET /verifications?petId=` was parsed off the query string and then never applied, so it returned the
UNFILTERED history — one pet's page would have shown its sibling's checks as its own. A client with two
pets is the case that catches it; `clientId` cannot.

## On-chain DogTag discovery (`packages/ui/src/chain/tagDiscovery.ts`)

### `dogTagId`: the operator-facing handle is NOT the chain key

The stored `ClientPet.dogTagId` (and the `credentialSubject.dogTagId` leaf) is the short decimal
**handle** from `Store::next_dog_tag_id`. The chain is keyed by `fieldOfValue(Integer(handle))` — a
BN254 field element — in `DogTagSBTConsent.profileRoot`, the circuit's `pub[0]` and every indexed event
topic. Verified against live chain 135: handle `"3"` →
`1195241908933892557940129631300775214454584041594363078565480038450625444405`, handle `"4"` →
`12814611650400102124986144372704047117871762901294624833396466912543715809135`, both real minted tags.
Filtering logs on the raw handle is a well-formed uint256 that matches nothing, so the portal would
render a confident "no on-chain activity" for a tag with plenty — use `resolveDogTagId`, which also
accepts a field element already in decimal or `0x` form.

### What a tag id can and cannot discover

Discoverable, because keyed by / indexed on `dogTagId`: `profileRoot`, `issuerOf`, `status`, the
`Issued`/`StatusChanged`/`Burned` logs, the Level-A profile credential resolved
`profileRoot -> factory.rootIssuer(root) -> clone.issuedAt/isValid/isRevoked`, and every
`VerificationRegistryConsent.Verified` event by ANY relayer.

**Not discoverable, by design: Level-B credential roots are not bound to a `dogTagId` on chain.**
`DogTagIssuer.RootIssued` carries only the root, and the consent circuit binds `dogTagId <-> R` for the
tag's profile root alone. That link is exactly what would make a tag a public lookup key for an
animal's history, so no scan can enumerate "this pet's vaccination records" and no surface may imply it
can. What makes the gap useful rather than merely honest: a `Verified` event from a relayer that is not
this shop is positive evidence such credentials exist, which is the cue for the import handoff.

Read the registry address carefully — discovery needs `VerificationRegistryConsent`
(`DEPLOYED_ADDRESSES.VerificationRegistryConsent`), whose `Verified` indexes `dogTagId`. The superseded
`VerificationRegistry` emits no such event, so pointing a scan at it returns zero rows and is
indistinguishable from a tag with no history.

### A partial scan must never read as a complete one

`DiscoveryCoverage` separates the REQUESTED window (`fromBlock`) from the extent actually reached
(`reachedBlock`, `null` when no chunk completed) and lists each failed chunk's range. Head-first
chunking means a cancelled run covers `[reachedBlock, toBlock]` and knows nothing below it; printing
`fromBlock` as read is the same defect as rendering an unchecked credential as valid. `discoverTag`
resolves on chunk failure or cancellation (the outcome is in the coverage) and rejects only when the
point-in-time reads fail, because then there is no result to qualify.

## `@dogtag/standard`'s barrel is browser-hostile — import the submodule

`packages/dogtag-standard-ts`'s index re-exports `consent.ts`, whose `circomlibjs` EdDSA/BabyJubjub
dependency touches the Node `Buffer` global at module init. In a browser that is an uncaught
`ReferenceError` that takes down whatever imported it — and a production bundler tree-shakes the unused
import away, so **it reproduces only under a dev server**. That is why the package now also exposes
`"./*"` subpath exports and why `packages/ui` imports `@dogtag/standard/leaf`, `/verify`, `/field`,
`/types` rather than the barrel. Every module except `consent.ts` is Buffer-free. Browser code that
needs EdDSA (the owner wallet) still uses the barrel and must accept that constraint.

Confirming this is a two-line check in the page, not a guess:

```js
await import("/@fs/<repo>/packages/dogtag-standard-ts/dist/index.js") // throws: Buffer is not defined
await import("/@fs/<repo>/packages/dogtag-standard-ts/dist/leaf.js")  // fine
```

## Driving a portal in a browser while other crews are live

`chrome-devtools-axi` attaches to a SHARED bridge by default, so a bare `chrome-devtools-axi open` can
navigate a tab another crew is driving (observed: commands landing on a portal at `:43917`, neither mine
nor the captain's). Set **both** `CHROME_DEVTOOLS_AXI_PORT` (a distinct bridge port) and
`CHROME_DEVTOOLS_AXI_USER_DATA_DIR` (a scratch profile) to get an isolated browser, and check
`location.href` before trusting a snapshot.

Also: `pnpm --filter <pkg> dev -- --port N` does NOT reach vite (the `--` is passed through literally,
and `strictPort` then fails on the config's own port). Run `./node_modules/.bin/vite --port N` from the
package directory instead.

### The issuer↔domain DNS binding — read `docs/ISSUER_DOMAIN_BINDING.md` before touching it

The normative convention, the three-link verification chain, the six states and the display rules all
live in `docs/ISSUER_DOMAIN_BINDING.md`. Four things there are counter-intuitive enough to restate:

**The DoH classification rule is duplicated in three languages on purpose.** Rust
(`dogtag-dns-rs::classify_txt`), Swift (`IssuerBindingResolver.classifyDoh`) and Kotlin (same name) each
run in a different process with no shared runtime. All three carry the same unit tests over the same DoH
bodies. If you change one, change all three — a phone and a portal disagreeing about whether a domain
published a record is worse than either answer.

**`DogTagIssuer` clones have no owner.** They are `Initializable` only; all write authority is
`IssuerRegistry.isWhitelistedFor`. `createIssuer` salts a clone with `keccak256(recordType, business)`
and never stores `business`, so the clone→business relationship is verifiable only via
`factory.predictIssuer(recordType, candidate) == clone` — one-way (verify, not enumerate), which is
exactly what an authorization check needs. This is why `IssuerDomainRegistry` is a new additive contract:
a field on the clone would need a new impl, `factory.implementation` is `immutable` so that means a new
factory, and `VerificationRegistryConsent.rootIndex` is `immutable` too — a new factory strands
`rootIssuer[R]` at `address(0)` and breaks owner-hidden consent for every credential issued after it.
That is a protocol-wide v2 hiding behind a one-field change.

**`business` frequently defaults to the operator's own signer.** `resolve_business`
(`stacks/admin/api/src/routes.rs`) falls back to the admin signer when the caller omits it, and
`scripts/demo-provision-government.sh` does exactly that. So for existing clones, "the spawning business"
tier authorizes the OPERATOR, not the organisation — which is why `domainAdmin[clone]` exists. Create new
clones with an explicit, organisation-controlled `business` address.

**The ROAX dev node serves archive queries.** Verified empirically (Geth v1.15.10): `eth_call name()` at
head−5000 returns full historical state and `eth_getStorageAt` at head−100000 answers without a
missing-trie-node error. Block-pinned reads are therefore reproducible, and the block anchor in a
verification response is real rather than decorative. DNS, by contrast, has NO history — a TXT record is
only ever observable now — so the DNS half of a binding is labelled `dnsObservation` / `dnsHistorical:
false` and must never be presented as proving the past.

### Android JVM unit tests and `org.json`

`android.jar` ships `org.json` as a stub whose every method throws `"Method … not mocked"`, so any pure
unit test over a JSON-parsing code path fails for a toolchain reason rather than a code one.
`app/build.gradle.kts` therefore puts the REAL `org.json:json` on the unit-test classpath. That is the
same library the device provides at runtime, not a fake — tests exercise actual parsing behaviour. If you
add a pure test that touches `JSONObject` and it explodes with "not mocked", the dependency is missing,
not your code.

### Regenerating `apps/ios/DogTag.xcodeproj` without dropping the prover artifacts

`project.yml` globs `sources: - path: DogTag`, so xcodegen picks up new Swift files automatically — but
the BUNDLE copies `DogTag/consent_final.zkey` and `DogTag/consent.graph` are **gitignored** and absent from a
fresh worktree (their sources under `circuits/build/` are committed), while the
committed `project.pbxproj` references both. Running `xcodegen generate` on a checkout that lacks them
silently strips those resource references, and the app then builds without its proving artifacts.

Vendor them (or `touch` placeholders) BEFORE generating, then verify the diff is purely additive:

```sh
# Real vendoring (what a proving build needs) - the copies STAY:
make vendor-mobile-artifacts
cd apps/ios && xcodegen generate
grep -o 'consent_final.zkey\|consent.graph' DogTag.xcodeproj/project.pbxproj | sort -u   # both present
git diff --stat DogTag.xcodeproj/project.pbxproj                                          # insertions only
```

If you only need the pbxproj wiring and not a proving build, empty placeholders are enough - and
those you do remove afterwards (do NOT run this `rm` after a real vendor, it deletes the artifacts):

```sh
cd apps/ios && touch DogTag/consent_final.zkey DogTag/consent.graph
xcodegen generate && grep -o 'consent_final.zkey\|consent.graph' DogTag.xcodeproj/project.pbxproj | sort -u
rm DogTag/consent_final.zkey DogTag/consent.graph
```

The host-less `DogTagTests` target lists sources INDIVIDUALLY and must stay Foundation-only (no FFI), so
adding a file there means checking its whole import closure.
