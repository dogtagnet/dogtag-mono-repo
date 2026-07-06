# Project agent memory

This file is the project's committed home for project-intrinsic agent knowledge: build, test, release, architecture, and sharp-edge notes that should travel with the code.

## Build & test (what actually runs offline)

Toolchain: Rust (cargo workspace), Foundry (`forge`/`cast`), Node 22 + pnpm 10, circom 2.1.9 + snarkjs 0.7.6, Docker.

- `cargo check --workspace` / `cargo build` — Rust workspace: `dogtag-standard-rs`, `dogtag-prover-rs`, `vet-api`, `admin-api`, `government-api`, `indexer-api`.
- `cargo test -p indexer-api` — the oversight indexer (scope + store unit tests + `tests/query_api.rs` end-to-end over MemLogSource + MemStore). Hermetic, fast (no node/Mongo). See the "Oversight indexer (PR-4)" section.
- `cargo test -p dogtag-standard-rs` — trust-core crypto + cross-language parity vectors.
- `cargo test -p vet-api -p admin-api` — backends. (One vet-api suite, `gate_dual_signing_parity`, is slow — ~5 min — it runs the real prover/signing; this is expected, not a hang.)
- `cd contracts && forge test` — 55 tests incl. `ZkIntegration.t.sol` (real Groth16 proof verified on-chain), `Verification.t.sol`, and `GovernanceMigration.t.sol` (EOA→multisig hand-off). Use `forge test`, **not** bare `forge build`: a bare full build tries to compile the OZ submodule's `certora/harnesses/*` which import generated `../patched/*` files that aren't present, so it fails with "File not found" — a vendored-submodule artifact, NOT a project error. `forge test` only compiles the real dependency closure and is green.
- `cd circuits && node scripts/test-circuit.mjs` — generates REAL Groth16 proofs (leaf counts 1..24) + negative tests. Needs the TS SDK built first (`pnpm --filter @dogtag/standard build`) and `pnpm install`. Slow (large r1cs witness gen).
- `make parity` — the Poseidon anchor gate; `make test` — parity + TS + Rust + contracts.
- `cargo test -p vet-api --test verify_onchain` — on-chain integration (self-spawns anvil). The ZK-path
  test (`zk_path_records_verified_onchain`, real Groth16 proof, ~270s) needs forge/cast/anvil on PATH AND
  the JS toolchain built first: `pnpm install` in `circuits/` plus `pnpm install && pnpm run build` in
  `packages/dogtag-standard-ts/` (`crates/dogtag-prover-rs/tests/gen_input.mjs` imports its `dist/`). It
  does NOT skip gracefully when those are missing.

### Sharp edges learned
- **The parity gate is `circuits/scripts/gen-vectors.mjs`.** It is the source of truth: it computes the circom witness (reference-of-record) and cross-checks `poseidon-lite` (TS) and `circomlibjs`, then writes `circuits/poseidon-vectors.json` which Rust (`sdk_parity.rs`/`poseidon_parity.rs`) and Solidity (`PoseidonParity.t.sol`) assert. The "4-language" gate is the union of `make parity` + `test-rs` + `test-contracts`. (`circuits/scripts/check-ts.mjs` was referenced by `package.json` but never existed; it was removed — `gen-vectors.mjs` already covers TS↔circom.)
- `gen-vectors.mjs` rewrites `poseidon-vectors.json` deterministically, so running `make parity` leaves the tree clean (no spurious diff).
- `rust-analyzer` in this worktree can't find the proc-macro server and emits false `E0308`/`tokio::test` errors; trust `cargo`, not the IDE diagnostics.
- Pre-existing harmless warning: unused import `BigInteger` in `crates/dogtag-standard-rs/src/bin/field-hash.rs`.

## Architecture quick map
- `crates/dogtag-standard-rs` — trust core: canonicalization, field/type-tag encoding, circom-compatible Poseidon (`light-poseidon`), salted Merkle, verify, EdDSA-BabyJubjub signer, BLAKE-512 (circomlibjs parity), UniFFI → mobile.
- `crates/dogtag-prover-rs` — real ark-circom/ark-groth16 prover (self-verifies). Test oracle + backend prover-service.
- `circuits` — Groth16 `DogTagVerification(N=24, depth=5)`: Poseidon-Merkle membership + EdDSA consent sig + nullifier + keyHash. Committed artifacts (`verification_final.zkey`, `.r1cs`, `.wasm`, vkey) are a **testnet self-run** trusted setup produced by `circuits/scripts/ceremony.sh` (public Hermez phase-1 ptau + 3 phase-2 contributions + a public drand beacon), recorded in `docs/CEREMONY_TRANSCRIPT.md`. All 3 contributions were run on our own infra, so it does **NOT** yet have the 1-of-N-independent-honest guarantee — it is a real ceremony process producing a **testnet-grade** key, to be re-run with ≥3 genuinely independent external contributors before mainnet. The phase-1 ptau is the public Hermez/Perpetual-PoT file, fetched from a mirror and cryptographically re-verified by `ceremony.sh init` (`snarkjs powersoftau verify`), so its trust does not depend on the download URL.
- `contracts` — `DogTagSBT` (ERC-5192), `IssuerRegistry`, `DogTagIssuer` clones + factory, `VerificationRegistry` (real Groth16 verify, timelocked verifier swap), `ConsentKeyRegistry` (gasless meta-tx), `Groth16Verifier` (snarkjs-generated). Live on ROAX (chainId 135); addresses in `contracts/deployments/roax.json`.
- `stacks/vet` + `stacks/groomer` — same `vet-api` binary (`BUSINESS_TYPE` switch) + SPA + Mongo. `stacks/admin` — central registry/admin-api.
- `stacks/government` — **net-new, separately-deployable** role stack running its **own** `government-api` crate (NOT vet-api): a government credential authority that issues authority-endorsed `TRAVEL_CLEARANCE`/`EU_HEALTH_CERT` (anchors root via `DogTagIssuer.issue`) and does government-grade verify (integrity + `isValid` + `isWhitelistedFor`, all gasless reads). Own Mongo (`governmentdata`), ports 44831/44832, `make up-government`. `GOV_DEMO_MODE=1` → `MemChain`+`MemStore` (no node/gas/Mongo, used by `tests/flow_memchain.rs`); live mode → `AlloyChain` (+ `GOV_SIGNER_KEY` to anchor). It reuses the shared `dogtag-standard-rs` SDK for credential build/wrap but has its own trimmed `chain.rs`. Design: `docs/ROLE_APPS.md`.
- **Three-role showcase**: `scripts/demo-up.sh` boots all role stacks as separate services (admin/vet/groomer/government + portals). `scripts/e2e-roles.sh` (default = hermetic government ISSUE→VERIFY in `GOV_DEMO_MODE`, no deps; `--live` = vet ISSUES → government VERIFIES → government ISSUES across the running stacks over ROAX, needs `contracts/.env`). `government-api tests/cross_role.rs` codifies "vet ISSUES → government VERIFIES" deterministically over MemChain. See `docs/ROLE_APPS.md` §8.
- **Government per-record-type fields**: each credential type has its OWN field set — backend `credentialSubject` is built per type in `government/api/src/app.rs::build_gov_vc` (`TRAVEL_CLEARANCE` = the CDC-sectioned nested subject: Section A importer/consignee + B animal + C travel + validity + public `receiptId` — see the "Government travel receipt" section above; `EU_HEALTH_CERT` = species/microchip/rabies/examining-vet/health-status), and the web Issue form (`government/web/src/pages/Issue.tsx`, `RECORD_TYPE_SECTIONS`) mirrors those leaves as a **sectioned** A/B/C+validity form. Keep the two in sync (a form field `key` must equal the flat input key `build_gov_vc` reads via `get(...)`; for `EU_HEALTH_CERT` that key equals the leaf name, while `TRAVEL_CLEARANCE` maps flat keys onto nested leaves, e.g. `importerLastName` → `importer.lastName`, `animalName` → `animal.name`). **e2e-locked field keys:** `government.spec.ts` asserts TRAVEL_CLEARANCE has `field-animalName` and NOT `field-microchipNumber`, and EU_HEALTH_CERT the reverse — do NOT add a `microchipNumber` input to the TRAVEL form (the backend defaults it under `animal`), or the per-type field test breaks. After a successful issue the portal shows the wrapped doc with a one-click **Copy** button to paste into Verify + a link to the printable receipt. The whitelist pillar is exercised because the Verify page pre-fills the signer from `/health`.
- **Government web e2e (Playwright)**: `stacks/government/web/e2e/government.spec.ts` (config `playwright.config.ts`) drives issue→copy→verify for both record types against a LIVE portal. It is NOT in `pnpm test`/CI (needs a running portal + browsers); run it against a served instance: `GOV_URL=<portal-url> pnpm --filter @dogtag/government-web test:e2e` (one-off `pnpm exec playwright install chromium`). A same-registry live serve reuses the deployed TRAVEL_CLEARANCE clone for BOTH `*_ISSUER_ADDR` and `GOV_SIGNER_KEY=$DEPLOYER_PRIVATE_KEY` (already whitelisted for both types).
- `stacks/owner/web` (`@dogtag/owner-web`, port **45931**) - the **pet-owner (holder) wallet**, the consumer front. Web mirror of the native `apps/android`+`apps/ios` holder: a self-custodial wallet that **receives** an issued wrapped doc (integrity-checked offline via `@dogtag/standard checkIntegrity`, held in localStorage), **displays** it (decoded leaves + `DogTagIssuer.isValid` read), and **presents** a ZK proof. It has **no backend** - it talks directly to two hosts given at runtime: the verifier's `…/x/<token>` session it scans and a **trusted prover-service** (`POST /prove-verification`, `VITE_OWNER_PROVER_URL`, default :41875). The "phone ZK" client crypto (build §1.10 consent + `signConsentEddsa` EdDSA-BabyJubjub + EIP-712 `BindConsentKey` sig via `viem`) runs **in the browser**; only the heavy Groth16 proof is delegated to the prover (the verifier never sees the witness). Present flow = `src/lib/present.ts`; wired into `scripts/demo-up.sh`.
  - **Sharp edge (browser Buffer)**: `@dogtag/standard`'s EdDSA path pulls in `circomlibjs`, which needs Node `Buffer`/`global` at runtime. The vite **build** tree-shakes past it but the **dev server crashes** ("Buffer is not defined") without a shim. `src/polyfills.ts` (imported first in `main.tsx`, `buffer` npm dep) provides them. Any new web app that signs consent client-side needs the same shim.
  - **Selective disclosure / "Share a redacted copy" (`src/pages/Share.tsx` at `/share/:id`, logic in `src/lib/redact.ts`)** - the Merkle counterpart to the ZK Present flow, and the web mirror of the native apps' "Share redacted" (mobile FFI `obfuscateDocumentJson`). The holder toggles which leaves to reveal; withheld leaves run through `@dogtag/standard`'s `obfuscate` (leaf hash → `privacy.obfuscated[]`, cleartext dropped, **Merkle root R unchanged**), so the recipient still `checkIntegrity`-verifies the SAME authentic credential + can read `isValid` on-chain, seeing only revealed fields. Default = reveal-all (the holder explicitly withholds; no fragile PII classifier). `credentialSubject.dogTagId` is **locked-on** (`NON_OBFUSCATABLE_PATHS`, mirrors verify's `NON_OBFUSCATABLE` - withholding it would fail integrity), and `recordType` is **locked as public** (`PUBLIC_PATHS` - its value is also carried in the always-revealed `issuer` block, so a toggle to "withhold" it would be a lie). Output is copy-JSON + download (same paste-JSON idiom as Receive / the issuers' "Copy wrapped document"); NO ZK on this path, NO backend, no store mutation (the held full credential is untouched). Reached via a "Share a redacted copy →" button on `CredentialDetail`.
- **Owner web e2e (Playwright)**: `stacks/owner/web/e2e/owner.spec.ts` drives the whole holder loop (receive → hold/display → generate ZK proof → present → verified) + a tamper-rejection test + a **selective-disclosure test** (open Share → withhold a field → the redacted copy still `checkIntegrity`-verifies with the SAME `merkleRoot` + the withheld cleartext gone + `privacy.obfuscated` grown). Like the government e2e it is NOT in `pnpm test`/CI. It starts its OWN vite dev server and **mocks the prover + verifier + ROAX RPC** at the network layer (deterministic), but runs the REAL client-side crypto. `pnpm --filter @dogtag/owner-web test:e2e`; `OWNER_URL=<url>` runs it against a live wallet instead (no self-server).

### Per-role records DB + CRUD (management layer)
Each role platform persists the records it issues into its OWN store (separate Mongo per running instance; `MemStore` for demo/tests), bundling the credential data with its **immutable on-chain proof**: tx hash, block number, contract (DogTagIssuer clone) address, and a ready-to-click explorer link `https://explorer.roax.net/tx/<hash>`.
- **vet-api** (serves vet + groomer via `BUSINESS_TYPE`, one DB per instance): `store::Record` gained `block_number`/`explorer_url`/`created_at`/`updated_at`/`label`/`notes`/`revoked_*`/`invalidated_at`/`invalidation_reason` + `RecordStatus::Expired`; `Store::list_records` (Mem + Mongo, most-recent first). Routes: `GET /records` (operator-gated list, surfaces explorer links), `PATCH /records/:id` (off-chain metadata only), plus the existing soft-invalidating `POST /records/:id/revoke`. `block_number` is captured in `confirm_inner` from `TxView.block_number`; the revoke path reads the revoke tx's block via `get_tx_view`.
- **government-api** (own DB): `store::IssuedCredential` gained the same proof + metadata fields + a `CredentialStatus` enum; routes `PATCH /v1/records/:root` and `POST /v1/records/:root/revoke` (adds `ChainClient::revoke` + `revoke_calldata`; `SentTx` now carries `block_number`).
  These routes are gated by `Authorization: Bearer <GOV_API_TOKEN>` — as are issue and the operator record reads (`GET /v1/records`, `GET /v1/records/:root`, which leak Section A person PII if open); health, verify, the verifications audit log, and the public PII-free receipt endpoints stay open (see the "Government travel receipt" section above for the full gating rationale). Missing/wrong token → 401; in demo mode (`GOV_DEMO_MODE` et al) an unset `GOV_API_TOKEN` defaults to `dogtag-gov-demo-token` (the portal's `VITE_GOV_API_TOKEN` falls back to the same value); in non-demo mode with no token configured, the gated routes fail closed with 503.
- **Immutability**: `PATCH` accepts ONLY off-chain fields (`label`/`notes`, and `status` → `expired`); any on-chain-derived key in the body (tx hash, block, contract/issuer addr, root, wrapped doc, explorer url) is **rejected 400** ("… is on-chain-derived and immutable"). See the `IMMUTABLE_KEYS` list in each `routes.rs`.
- **Soft-invalidation, never hard delete**: revoke flips status to `revoked` on-chain (isValid → false) but keeps the row + its original issuance proof AND adds a revoke-tx proof; `expired` is an off-chain-only status transition (anchor untouched). Both stay listed + explorer-verifiable. There is NO delete endpoint by design. State machine: revoke accepts `issued` OR `expired` records (a compromised-but-expired credential can still be invalidated on-chain); expire accepts ONLY `issued` — anything else, incl. a revoked record, is rejected 409 (an off-chain `expired` must never mask an on-chain revocation; `revoked` is terminal).
- **Web**: the vet + groomer portals share `stacks/{vet,groomer}/web/src/pages/Records.tsx` (identical) which now reads `api.listRecords()` from the backend DB (NOT the old localStorage `recordsStore`) and offers edit/expire/revoke via the shared `@dogtag/ui` client (`listRecords`/`updateRecord` in `packages/ui/src/api/client.ts`). The government portal has its own `Records` page (`stacks/government/web/src/pages/Records.tsx`) using the `@dogtag/ui` `Table`/`Badge`.
- **Tests**: hermetic Rust integration tests (`stacks/{vet,government}/api/tests/records_crud.rs`, MemChain+MemStore) prove issue→persist-proof→list→patch(reject on-chain)→revoke(soft)→expire. Playwright: `government/web/e2e/records-crud.spec.ts` runs full-stack against a demo `GOV_DEMO_MODE` backend (real store + mem chain); `stacks/{vet,groomer}/web/e2e/records.spec.ts` drive the shared Records UI against a **mocked** backend (route regex `^https?://[^/]+/api/` — a `**/api/**` glob wrongly swallows `@dogtag/ui`'s `src/api/*.ts` module scripts and breaks the mount). None are in CI (need a served portal + browsers).

### Government travel receipt (CDC-modeled TRAVEL_CLEARANCE)
The government `TRAVEL_CLEARANCE` credential is a CDC-modeled travel receipt (research `dogtag-govreceipt-r7` §2.1 + arch `dogtag-govarch-r8`). Grounding rules that are easy to get wrong:
- **Nested CDC subject.** `build_gov_vc` (`stacks/government/api/src/app.rs`) builds a nested `credentialSubject`: `importer`/`consignee` (**Section A** — person PII, the private/obfuscatable block), `animal` (**Section B**), `travel` (**Section C**), a `validity` block, plus top-level `receiptId`. Nesting flattens to leaf key-paths automatically (`credentialSubject.importer.firstName`, …). B/C + validity + receiptId are PUBLIC (revealed leaves); A is obfuscated by the holder. `dogTagId` stays mandatory + non-obfuscatable. The envelope (attestationType/trustTier/legalEffect/legalBasisVersion/jurisdiction) is UNCHANGED. The web Issue form (`stacks/government/web/src/pages/Issue.tsx` `RECORD_TYPE_SECTIONS`) sends a flat subset whose keys map 1:1 onto these leaves (`importerLastName`, `animalName`, `travelType`, …), grouped into the CDC sections; the sectioned form + printable receipt view landed in **PR-2** (see "Government receipt UI" below).
- **Receipt ID = public salted leaf + off-chain lookup handle — NOT the nullifier.** 12-char Crockford-base32 from a CSPRNG (~60 bits), minted in `routes::issue` (`gen_receipt_id`, uniqueness-retried), committed into `R` as a leaf AND stored on `IssuedCredential.receipt_id` (Mongo unique+sparse index on `receiptId`; `Store::get_credential_by_receipt_id`). Equating it to the ZK nullifier was ruled unsound (nullifier is per-verification, consumed once, unlinkable). `IssuedCredential` also denormalizes a cleartext `subject` projection + `valid_until`; all three (`receiptId`/`subject`/`validUntil`) are in `IMMUTABLE_KEYS` (mirror content committed in R).
- **Issuance date is DERIVED from the chain, never a leaf** (arch DP-2): read `DogTagIssuer.issuedAt[R]` (the anchoring block timestamp). `validUntil` DOES stay a public salted leaf (policy-variable window).
- **Derived `effectiveStatus`** computed at read time everywhere a record renders: `revoked ? REVOKED : (status==expired || today > validUntil) ? EXPIRED : VALID` (a never-anchored draft → `DRAFT`). `routes.rs` has `derive_effective_status` (pure, for list/detail) and folds it against a LIVE `isValid(R)` read in `resolve_receipt_status` (public endpoints). Date math uses a self-contained civil-from-days helper (no chrono/time dep); ISO dates compare as strings.
- **Public, PII-free endpoints (no auth):** `GET /v1/receipts/:receiptId/status` (JSON: effectiveStatus, recordType, receiptId, validUntil, issuanceDate, root, issuerAddr, explorer links, checkedAt — via a LIVE `isValid(R)` read, not a DB echo) and `GET /r/:receiptId` (server-rendered HTML status page, status-only by default per arch DP-5 — NO Section A/B/C content).
- **Issue AND the operator record reads are now GATED** behind the `require_api_token` bearer: `/v1/travel-clearance/issue` (arch DP-6; was open) plus `GET /v1/records` and `GET /v1/records/:root`, which are gated because the CDC subject denormalizes Section A person PII (idNumber, dateOfBirth, email, phone, name) into the record — an unauthenticated read would leak it. Verify, health, the verifications audit log, and the PUBLIC PII-free receipt endpoints (`GET /v1/receipts/:receiptId/status`, `GET /r/:receiptId`) stay open; demo keeps the baked `dogtag-gov-demo-token`. Callers must send the bearer — the web app (`apiGet`/`apiPost(..., {auth:true})`), `scripts/e2e-roles.sh` (`$GTOK`), and the Rust integration tests were updated accordingly.
- **OPS-0 (on-chain prereq, already live on ROAX chainId 135).** The per-record-type `DogTagIssuer` clones are deployed via `DogTagIssuerFactory.createIssuer(name, keccak256(recordType), business)` with `business == the protocol admin 0x119F8c7F…` (single-authority topology, arch DP-3), and the government signer (that same admin address on testnet) is whitelisted on `IssuerRegistry.whitelistFor(keccak256(recordType), signer)` — `DogTagIssuer.issue` is `onlyWhitelisted`. Addresses (in `contracts/deployments/roax.json` → `government_clones` and `stacks/government/.env.example`): **TRAVEL_CLEARANCE `0x8e276BD4c57740766A7e173D05F4f02013681c6a`**, **EU_HEALTH_CERT `0xe30A17396c0fb75D3e8bFc862a49677B3dd568E2`**. These clones were deployed while the deployer EOA still held factory ownership + `WHITELIST_ADMIN` (pre-Phase-2, so this OPS step needed no multisig). Governance Phase-2 has since moved all three authorities to the governance signer `0x8E27E117…` (see "Governance / admin" below) — the deployed clones are unaffected (immutable once deployed); only NEW factory deploys / whitelist grants now flow through the governance holder.

### Government receipt UI + portal shell (PR-2)
The government web portal (`stacks/government/web`) was migrated from the hand-rolled dark SPA onto the shared **`@dogtag/ui` AppShell + Tailwind + tokens** (same stack as vet/groomer/admin) and gained the printable CDC-modeled receipt view. Structure + sharp edges:
- **Build wiring (was a lean SPA, now a `@dogtag/ui` consumer):** added `tailwind.config.ts` (scans `../../../packages/ui/src/**` so the shared components' token classes are emitted), `postcss.config.js`, deps (`@dogtag/ui`/`@dogtag/standard`/`lucide-react` + tailwind/postcss/autoprefixer), `index.css` = `@import "@dogtag/ui/tokens.css"` + `@tailwind` layers, and `vite.config.ts` `optimizeDeps.exclude` for the workspace-source packages. `main.tsx` wraps in `ThemeProvider`(default light)+`ToastProvider` (NO WalletProvider — government auths with a bearer token, not a wallet). App split into `app/Layout.tsx` (AppShell) + `pages/{Issue,Verify,Records,Receipt}.tsx` + `lib/api.ts`. The Dockerfile already `COPY packages` so the shared UI builds in-image.
- **Receipt view** `pages/Receipt.tsx` at route `/receipt/:root`: the authenticated, CDC-anatomy receipt. Fetches `GET /v1/records/:root` (auth, carries the Section A/B/C PII `subject`) for content + `GET /v1/receipts/:receiptId/status` (public) for the LIVE `effectiveStatus` + chain-derived `issuanceDate`. Renders letterhead + status chip + Receipt ID / issuance / validity block + legal preamble + Section A/B/C tables + a Verification block with a **QR** to the public status page. The receipt sheet uses a FIXED light palette (the `.receipt-sheet` CSS in `index.css`, NOT the theme tokens) so it always looks like the official paper in dark theme AND when printed. `@media print` strips the AppShell chrome (`aside`,`header`,`.no-print`) so browser "Print → Save as PDF" yields the clean document.
- **QR target = same-origin `/r/:receiptId`** (`publicReceiptUrl` in `lib/api.ts`), the PII-free server-rendered page PR-1 built on the BACKEND. Because the SPA history-fallback would otherwise swallow `/r/*`, both the vite dev proxy (`vite.config.ts`) AND nginx (`nginx.conf`) proxy `/r/` straight to the api service (NOT prefix-stripped) so the QR resolves on the portal's own origin. If you add more public backend-owned paths, proxy them the same way.
- **Derived-status rendering:** the Records table shows the backend's derived `effectiveStatus` (VALID/EXPIRED/REVOKED) as the colored `Badge`, plus an amber "expires ≤30d" chip; a separate `data-testid="record-status"` span keeps the RAW lifecycle status text (`issued`/`revoked`/`expired`) that `records-crud.spec.ts` asserts exact-match on. Don't merge the two — the e2e needs the raw word.
- **e2e test-id contract (do not rename):** the migration preserved every selector the two Playwright specs use — `record-type` MUST stay a native `<select>` (Playwright `selectOption` can't drive a radix Select); the dogTagId field MUST stay the FIRST `<input>` on `/issue`; the dogTag cell keeps class `mono` (`td.mono`); the Verify verdict keeps a literal `ok`/`bad` class token; and `issue-submit`/`wrapped-doc`/`copy-wrapped`/`verify-*`/`pillar-*`/`record-row`/`edit-*`/`expire`/`revoke`/`explorer-link`/`revoke-explorer-link`/`records-refresh` are all retained. `receipt.spec.ts` adds the new flow (issue → open receipt → sections + QR render → public `/r/:id` page shows the verdict). None of these are in CI (need a served portal + browsers).

### Mobile travel receipt + `obfuscate()` FFI (PR-3)
The pet-owner HOLDER apps (iOS `apps/ios/DogTag`, Android `apps/android`) render a held `TRAVEL_CLEARANCE` credential as the same CDC receipt the web portal shows, produced LOCALLY from the stored `wrappedDocJson`. Structure + sharp edges:
- **`obfuscate()` is now in the mobile FFI.** `crates/dogtag-standard-rs/src/ffi.rs` exposes `obfuscate_document_json(wrapped_doc_json, key_paths) -> String` (UniFFI → Swift `obfuscateDocumentJson(wrappedDocJson:keyPaths:)`, Kotlin `obfuscateDocumentJson(wrappedDocJson, keyPaths)`). It wraps `wrap::obfuscate` (already existed, just wasn't surfaced): moves each named leaf's hash into `privacy.obfuscated[]` and drops the cleartext, leaving the Merkle root == on-chain root R UNCHANGED. So the phone builds a PII-free presentation copy with ZERO new ceremony — it's the merkle selective-disclosure proof, NOT a ZK proof. `credentialSubject.dogTagId` must never be obfuscated (`verify.rs` rejects it). Key paths are the FULL dotted path incl. the `credentialSubject.` prefix.
- **Regenerating the bindings is MANDATORY after any FFI change.** The committed `apps/ios/DogTag/dogtag_standard.swift` and `apps/android/app/src/main/java/uniffi/dogtag_standard/dogtag_standard.kt` carry UniFFI contract CHECKSUMS; if they don't match the freshly-built `.so`/`.a` the app traps at the first FFI call. Android CI rebuilds only the `.so` (cargo-ndk) and bundles the committed `.kt` as-is — it does NOT regenerate it — so you MUST regenerate + commit both. Build the host dylib WITH `--features prover` (else `proveVerification` drops out of the surface and the ABI shifts), then `cargo run --features prover,uniffi/cli --release --bin uniffi-bindgen -- generate --library target/release/libdogtag_standard.dylib --language {swift,kotlin} --out-dir <tmp>` and copy both outputs over the committed files (the generator output matches the committed style; the diff is additive).
- **`TravelReceiptView.swift` / `TravelReceiptScreen.kt`** mirror `stacks/government/web/src/pages/Receipt.tsx` 1:1 (Section A/B/C labels, sex+neutered combine, humanize, empty-row omission). Reached from `CredentialDetailScreen` via a "Show travel receipt" button gated on `group == .travel`. They decode `credentialSubject` leaves into a dotted-path→value map from `WrappedDoc.decodedFields()` (strip the `credentialSubject.` prefix), render the effectiveStatus banner (live `RoaxRpc.isValid` → REVOKED wins, then lapsed `validity.validUntil` → EXPIRED, else VALID; chain-unreachable falls back to the stored integrity verdict), and a Verification block with a QR.
- **The QR is PII-free and points at `https://<issuer.domain>/r/<receiptId>`** — the public status page PR-1 built. This is a NEW, deliberate exception to the "QR generation removed" rule in `QR.swift` (that removal was for the one-time verification-JWT presentation QR; a status-page URL leaks nothing). iOS draws it with CoreImage `CIFilter.qrCodeGenerator` (no dep); Android with `com.google.zxing:core` (added to `app/build.gradle.kts` — ML Kit only SCANS, it can't ENCODE). `receiptId` and `issuer.domain` come from the credential itself (the `receiptId` leaf + `issuer.domain`); if the gov web app is hosted somewhere other than its `did:web` domain the URL would need another source, but the receipt also prints the id as text.
- **Selective disclosure is holder-controlled.** Section-A person-PII leaves default to WITHHELD; per-field reveal toggles flip them; `dogTagId` + Section B/C default visible. "Share redacted" runs `obfuscateDocumentJson` over the withheld leaves and hands the redacted `wrappedDoc` to the OS share sheet (iOS `UIActivityViewController`, Android `ACTION_SEND`). Withheld rows render as "— withheld by holder —". NO ZK on this path; the on-device Groth16 prover stays reserved for the separate anonymous verification-record flow.
- **Issuance date** comes from the `validity.issuedOn` leaf (the phone can't read on-chain `issuedAt[R]`); falls back to the imported record's `issuedOn`.

### Oversight indexer (PR-4)
The **net-new, separately-deployable** `stacks/indexer/api` crate (`indexer-api`, port **46001**, own Mongo `indexerdata`, `stacks/indexer/docker-compose.yml`, `make`-free — run via compose) is the on-chain oversight feed the arch calls for (`dogtag-govarch-r8` Part 4; the admin portal `dogtag-adminportal-a3` is its later UNSCOPED consumer).
It scans the ROAX (chainId 135) contract event logs into a **non-PII** queryable index and serves a **scope-enforced** query API. It is a backend service only — **no web UI in this PR** (the admin/government/vet portals are the later consumers). Design + sharp edges:
- **What it watches (all non-PII, arch §4.3):** `DogTagIssuerFactory` `IssuerCreated(clone,recordType,name)` + `RootRegistered(root,clone)`; `IssuerRegistry` `Whitelisted`/`Delisted(recordType,signer)`; every `DogTagIssuer` clone `RootIssued`/`RootRevoked(root,by,ts)`; `VerificationRegistry` `Verified(dogTagId,relayer,subject,purpose,nullifier,ts)`. Each log is flattened into a uniform `IndexedEvent` (`src/events.rs`) keyed by `id = txHash:logIndex` (the idempotency key — re-scans upsert, never duplicate). Roots are salted commitments, `dogTagId` is the non-personal SBT id, addresses are public signers — **no PII in the index** (doctrine).
- **Scan / decode (`src/chain.rs`, `LogSource` trait).** `AlloyLogSource` = real `eth_getLogs` filtered by event *signature* (topic0) with **no address filter**, so it catches every clone's `RootIssued`/`RootRevoked` regardless of when the clone was deployed. Each decoded log is then **anti-spoof-gated by emitting address**: factory events must come from the factory, registry events from the registry, `Verified` from the VerificationRegistry, and a `RootIssued`/`RootRevoked` only from a **known clone** (seeded from `roax.json` government + demo clones via `SEED_CLONES`, extended at runtime by `IssuerCreated`). Logs are processed in `(block,logIndex)` order so an `IssuerCreated` folds its clone into the known set before that clone's first issuance in the same range. `MemLogSource` is a scriptable in-memory source (with `chain::emit::*` alloy-encoding helpers) so the whole scan→index→query flow is testable with no node — the SAME `decode_log` runs both paths.
- **Finality-aware ingest loop / resume (`src/indexer.rs`) — captain-directed model.** ROAX is an EVM/PoS chain **with block finality** (verified live: `devrpc.roax.net` exposes the `finalized` AND `safe` block tags — `finalized` sits ~80 blocks behind `latest`). A finalized block can never reorg, so every indexed event carries a `Finality` lifecycle (`src/events.rs`): **finalized** (block ≤ the finalized watermark — immutable, never rewound/re-scanned) vs **pending** (block > watermark — still reorg-able, the ONLY range reorg logic touches). This matters for a *government oversight* feed: it must never present a pre-finality, reorg-able issuance as authoritative. Each tick: read `head` + the `finalized` tag (`LogSource::finalized_block()`; **fallback** to a `head - CONFIRMATIONS` watermark, logged as `confirmations-fallback`, if a node ever lacks the tag); scan `[last_finalized+1 .. head]` into a buffer (stamping each event finalized/pending from the watermark), then **atomically swap** the pending range — `delete_pending()` + upsert the re-derived set — only after the whole fallible scan succeeds, so a transient RPC error on any chunk leaves the prior pending rows intact instead of blanking the feed. A pending event orphaned by a reorg simply disappears (absent from the re-derived set) and finalized rows are untouched (no rewind needed). Promotion pending→finalized happens naturally as the watermark advances and the range is re-derived. The resume cursor persists the **finalized watermark** (`last_finalized` + its hash); a defensive hash-divergence guard at the watermark only ever fires under the confirmations fallback (a deeper-than-N reorg), rewinding via `delete_from_block`. `rebuild_known_clones()` on startup re-derives the clone set from previously-indexed `IssuerCreated` rows.
- **Finality on the query surface.** Every event JSON carries `finality`; `?finality=finalized|pending` filters; `/v1/stats` reports `finalized`/`pending` counts; `/v1/status` reports `finalizedBlock` + `finalitySource` (`finalized-tag` vs `confirmations-fallback`) + `lastFinalizedIndexed` + `lag`. The feed returns ALL events clearly annotated (not hidden), so an oversight consumer can default its authoritative view to `finality=finalized` while still seeing in-flight activity. Scope enforcement is unchanged.
- **Scoping is server-side (`src/scope.rs`) — the load-bearing doctrine.** A bearer token resolves (via `INDEXER_SCOPES` JSON) to a `Scope`: `Unscoped` (government oversight — every event) or `Signers{signers,clones}` (a business sees ONLY events whose acting signer ∈ its signers OR whose clone ∈ its clones). `Store::query_events(&q, &scope)` enforces admission; client filters (`type`/`signer`/`issuer`/`recordType`/`root`/`dogTagId`/`since`/`until`) only ever **narrow within** the token's ceiling — a scoped token can never reach another issuer via a query param (there is an integration test for exactly this). Empty registry + not demo ⇒ every query 401s (fail-closed, mirrors the government stack).
- **Query API (`src/routes.rs`, all bearer-gated except `/health`):** `GET /v1/events` (the feed — filters + newest-first + pagination), `GET /v1/stats` (in-scope counters: issued/revoked/active/verifications/clones/signers), `GET /v1/issuers` (deployed clones + per-clone issued/revoked counts), `GET /v1/status` (head/lastIndexedBlock/lag/scope). Every event is joined to the **signer→business directory** (`src/directory.rs`) to add `actorName`/`cloneName` where possible, plus a `txUrl` explorer link. `?recordType=` accepts a human label (keccak'd server-side) or a raw `0x` key.
- **Directory join (`src/directory.rs`) — doctrine-safe naming.** Two layers: operator-authoritative static seeds (`INDEXER_DIRECTORY` JSON `{addr:name}`), and optional admin-API enrichment (`ADMIN_API_BASE`/`ADMIN_API_TOKEN`) that periodically reads the admin `/v1/businesses` (public) + `/v1/issuer-applications` (admin-token) and joins signer addresses → business names on the shared `domain`. Reads **business identity only — never any role's PII Mongo**; any admin-API failure degrades to static-only.
- **Store (`src/store.rs`, `Store` trait).** `MemStore` (demo/tests) + `MongoStore` (feature `mongo`, `src/mongo.rs`; `events` keyed by `id` unique, `cursor` single doc). The Mongo query pushes the high-selectivity equality/range predicates then re-applies scope + `EventQuery::matches` + pagination in Rust (identical semantics to MemStore). The index is fully rebuildable from the chain, so a lost volume just triggers a re-scan from `START_BLOCK`.
- **Demo mode** (`INDEXER_DEMO_MODE=1`): scripted `MemLogSource` history (deploy → whitelist → issue×2 → verify → revoke on the gov clone, plus a demo-groomer issuance on the DOG_PROFILE clone) + `MemStore`, and two well-known tokens — `dogtag-indexer-oversight-demo-token` (unscoped) and `dogtag-indexer-vet-demo-token` (scoped to the DOG_PROFILE clone + demo-groomer signer). The demo sets the finalized watermark to block 6, so the gov flow shows as **finalized** and the newer demo-groomer DOG_PROFILE events show as **pending** — the feed demonstrates the finality lifecycle with no node/Mongo. `MemLogSource::set_finalized(h)` scripts the `finalized` tag; tests use it (+ `reorg_from`) to drive finality/promotion/reorg cases.
- **Tests:** unit (`scope`/`store` modules, incl. `delete_pending` keeps finalized + finality filter) + `tests/query_api.rs` drives the real ingest loop + HTTP router end-to-end (unscoped vs scoped counts, scope-cannot-be-widened, filters/stats/issuers, 401 auth, idempotent re-scan; **finalized events survive a pending-range reorg while orphaned pending events are dropped**; **promotion pending→finalized at the watermark**). All hermetic (MemLogSource + MemStore), in `cargo test -p indexer-api`.

### Governance / admin (audit H-3)
- Governed contracts split admin two ways: `IssuerRegistry` (3-day), `VerificationRegistry` (2-day), and `DogTagSBT` (3-day) use OZ `AccessControlDefaultAdminRules` (two-step `begin`/`acceptDefaultAdminTransfer` + timelock); `DogTagIssuerFactory` uses `Ownable2Step`. `DogTagIssuer` clones have no own admin — they read `IssuerRegistry.hasRole(0x00)`. `ConsentKeyRegistry`/`Groth16Verifier`/`Poseidon6` have no admin.
- `DogTagSBT` inherits BOTH `AccessControlEnumerable` and `AccessControlDefaultAdminRules`, so it must explicitly override `grantRole`/`revokeRole`/`renounceRole`/`_setRoleAdmin` (`override(AccessControl, IAccessControl, AccessControlDefaultAdminRules)`) plus `_grantRole`/`_revokeRole`/`supportsInterface` — `super` resolves to the ACDAR rules first, then chains the enumerable bookkeeping. Do NOT `_grantRole(DEFAULT_ADMIN_ROLE,...)` in the constructor; the `AccessControlDefaultAdminRules(delay, admin)` base already does, and a second grant reverts (`AccessControlEnforcedDefaultAdminRules`).
- **Governance handover is DONE on ROAX (Phase-2 executed).** The governance signer **signer-1 `0x8E27E117…`** now holds the registry `DEFAULT_ADMIN_ROLE` + `WHITELIST_ADMIN` AND is the `DogTagIssuerFactory` `Ownable2Step` owner; the old deployer EOA `0x119F8c7F…` (`roax.json:admin`, kept as the historical deploy record) was **stripped of all roles**. Consequence for tooling: the demo/relayer/admin `ADMIN_PRIVATE_KEY` (the control-plane / GovernanceAction signer) **must now be signer-1 `0x8E27E117…`** — with the old EOA any privileged write (`createIssuer`/`whitelistFor`/`adminRevoke`) correctly downgrades to a `Disposition::Proposed` payload instead of broadcasting. The key value itself is captain-managed env, never committed. The EOA→governance migration is shipped as reviewable code (`contracts/script/GovernanceMigration.sol` library + `MigrateGovernance.s.sol` two-phase Begin/Accept scripts + `GovernanceMigration.t.sol`) and lives on mainline (merged via PR #8) — see `docs/GOVERNANCE_MIGRATION.md`. The **live** `DogTagSBT` (`0x1FB8…`) predates the two-step upgrade and is still plain `AccessControlEnumerable`; it can't be retrofitted without a state-orphaning redeploy, so the migration hands it over with an atomic `grantRole`→`revokeRole` (the script's `supportsTwoStep` auto-picks the branch). Never re-run the migration on live testnet without explicit captain approval.
- Removed dead governance surface: `IssuerRegistry.PROFILE_ISSUER_ROLE` and `DogTagSBT.UPDATER_ROLE` were declared but never enforced (SBT mint = `ISSUER_ROLE`; `setProfileRoot` = originator-or-`AUTHORITY_ROLE`). Don't re-add them.

### Admin control-plane foundation (PR-A: `GovernanceAction` + factory bindings)
The admin portal is the protocol control plane — it **extends** the existing `stacks/admin/web` (shared `@dogtag/ui` `AppShell`) + its `stacks/admin/api` AlloyChain signer; it is NOT a greenfield build (scout `dogtag-adminportal-a3`). "See on-chain activity" = the UNSCOPED consumer of the PR-4 indexer above (that UI is PR-B/PR-D). This PR-A landed only the backend **foundation**: the governance-action abstraction + factory bindings (no new web pages).
- **Three distinct on-chain authorities (`chain.rs`, plan Part 2).** Every privileged write is gated by ONE of: the **factory `Ownable2Step` owner** (`createIssuer`), the registry **`WHITELIST_ADMIN` role** (`whitelistFor`/`delistFor`), or the registry **`DEFAULT_ADMIN_ROLE`** (`adminRevoke`/role-admin/verifier+consent-key swaps, behind the 2–3 day ACDAR timelock). Governance Phase-2 has **executed**: all three now rest with the governance signer `0x8E27E117…` (the deployer EOA `0x119F8c7F…` was stripped). **Do NOT hardcode any EOA as the authority** — the dispatcher reads the holder live, so the control plane keeps working (executing when the hosted key IS the holder, else proposing) across the handover.
- **`GovernanceAction` (`src/governance.rs`) — the key-holder-agnostic abstraction.** A privileged write is a value `{target, calldata, authority, summary}` where `authority` is `Owner{owner_target}` or `Role{role_target, role, default_admin}`. `governance::dispatch(chain, signer_index, &action)` asks the chain WHO holds the authority (factory `owner()` / registry `hasRole()` / `defaultAdmin()`): if the hosted signer holds it → `send_action` (sign-and-broadcast, the existing legacy-gas path) returning `Disposition::Executed{txHash,holder}`; else → `Disposition::Proposed{holder,target,calldata,…}` for the governance signer / Safe to execute out-of-band. This survives the Phase-2 split BY CONSTRUCTION: an action silently flips executed→proposed the moment its role leaves the hosted key — no code path assumes which key holds which role.
- **Factory bindings added to `chain.rs` (`ChainClient` trait, both `AlloyChain` + `MemChain`):** `predict_issuer` (deterministic clone preview, `salt = keccak256(recordType, business)` — exact BEFORE deploy), `create_issuer_calldata`, `is_clone`, `root_issuer`, plus the authority reads `ownable_owner`/`ownable_pending_owner`, `has_role`, `default_admin`, `pending_default_admin` (the Phase-2 handover surfaces here), and `signer_address(index)` (Alloy derives it from the key) so the dispatcher can test hosted-key holdership. `MemChain` gains seed setters (`set_factory_owner`, `set_role`, `set_default_admin`, `set_pending_default_admin`) + a deterministic (non-CREATE2) clone preview for hermetic tests.
- **Endpoints (`admin_router`, admin-session gated):** `POST /v1/admin/factory/predict` (address preview), `POST /v1/admin/factory/issuers` (deploy via `GovernanceAction`; `business` defaults to the hosted signer = single-authority topology, matching the deployed government clones; returns predicted address + `Disposition`), `GET /v1/admin/governance/authority` (the live authority map: factory owner + pending, WHITELIST_ADMIN/DEFAULT_ADMIN holders, `heldByHosted` per authority, pending DEFAULT_ADMIN transfer + ETA — best-effort, unreachable target → `null`). `recordType` accepts a human label (keccak'd server-side via `record_type_key`) or a raw `0x`+64-hex key.
- **Config:** `FACTORY_ADDR` (new; roax.json `DogTagIssuerFactory` `0xd317…511D`) + `ADMIN_SIGNER_INDEX` (now HONORED — `main.rs` previously hardcoded index 0 and ignored the env). Doctrine holds: the control plane reads the chain + the admin business directory only — never another role's PII Mongo.

### Admin indexer consumption + signer→business directory (PR-B: `admin-indexer-consume`)
The "see on-chain activity" **data layer** — the admin/central backend becomes the UNSCOPED consumer of the PR-4 oversight indexer, plus the authoritative signer→business directory that NAMES on-chain signers. Backend only; the Activity/Dashboard UI is PR-D. Doctrine: read the CHAIN (via the indexer) + the admin business directory; NEVER another role's PII Mongo; no PII in these aggregates.
- **`src/indexer.rs` — the `OversightFeed` client (unscoped consumer).** A trait (`events`/`stats`/`issuers`/`status`) with a real `HttpOversightFeed` (reqwest → the indexer on `:46001`, presenting the `unscoped:true` bearer so it sees EVERY issuer's events — no client filter can widen the token's server-side ceiling), a `DisabledFeed` (unset `INDEXER_API_BASE` → every call `NotConfigured` → 503, fail-closed, rest of the backend unaffected), and a `MemFeed` (canned payloads for hermetic tests). Mirrors the `business.rs` outbound-HTTP + `MemChain` mock patterns. Injected as `AppState.feed: Arc<dyn OversightFeed>`. The admin does NO `eth_getLogs` itself — the indexer is the sole event source.
- **`src/directory.rs` — `SignerDirectory` (the naming join, plan §3.5).** Built live from the store (`all_businesses()` + `all_applications()`) into a `HashMap<signer_addr, DirectoryEntry{business, businessId, entity, recordTypes, verifyPurposes, domain, status}>`. Join key = `IssuerApplication.addresses[]`; business name resolved by `issuerEntityId → Business.business_id`, else `domain → Business.domain`, else the bare application domain. Approved applications win over pending on a signer collision. This is the AUTHORITATIVE source; the indexer's own `directory.rs` is a best-effort copy it pulls from this same admin API. It carries zero client PII (business + signer identity only) and needs NO indexer (store-derived, always live). Promotes the old O(n) `verify_relay.rs` relayer→business scan to a proper indexed lookup.
- **Endpoints (`admin_router`, admin-session gated):** `GET /v1/admin/activity` (unscoped cross-issuer feed; pass-through narrowing filters `type`/`signer`/`issuer`/`recordType`/`root`/`dogTagId`/`finality`/`since`/`until`/`limit`/`offset`; each event re-enriched with the admin directory's authoritative `actorName`/`cloneName`, overriding the indexer's copy), `GET /v1/admin/activity/stats` (cross-issuer counts: active vs revoked credentials, verifications, whitelisted/delisted, distinct clones/signers, finalized/pending — the aggregates PR-D renders), `GET /v1/admin/activity/issuers` (per-clone issued/revoked/active, name-enriched), `GET /v1/admin/directory` (full signer→business listing), `GET /v1/admin/directory/signer/:addr` (one signer → `{business, entity, recordTypes, …}`, 404 if unknown). An unconfigured indexer 503s the `activity*` surfaces but the `directory*` surfaces keep working.
- **Config:** `INDEXER_API_BASE` (+ alias `ADMIN_INDEXER_BASE`) + `INDEXER_OVERSIGHT_TOKEN` (+ alias `ADMIN_INDEXER_TOKEN`) — the indexer root + its `unscoped:true` bearer. Tests: `tests/indexer_consume.rs` (7 end-to-end over the real router with a seeded `MemFeed` + store directory) + `directory.rs`/`indexer.rs` unit tests. `tests/common/mod.rs` gained `hermetic_state_with_feed(feed)` (the base `hermetic_state()` wires an empty `MemFeed`).

### Issuers / Factory deploy UI (PR-C: `admin-factory-ui`)
The web surface for the captain's "deploy contracts from our factory". A new **Issuers / Factory** nav item on the shared `@dogtag/ui` `AppShell` (`stacks/admin/web/src/pages/Issuers.tsx`, nav in `app/Layout.tsx`, route in `App.tsx`) — the first web page consuming the PR-A backend. No new backend; it is pure UI over the PR-A/PR-B endpoints.
- **Live deterministic address preview.** The Deploy dialog debounces (recordType, business) and calls `POST /v1/admin/factory/predict` → shows the exact CREATE2 clone address BEFORE committing (salt = `keccak256(recordType, business)`). `business` is optional; blank = the single-authority topology (backend defaults it to the hosted signer). A stale-response guard (`seq` ref) drops superseded keystrokes so the preview never flickers to an old address.
- **Deploy routes through the GovernanceAction layer — the web NEVER assumes the old EOA.** Submit calls `POST /v1/admin/factory/issuers`; the response `result.disposition` is either `executed` (hosted key IS the factory owner → real ROAX tx, shown with an `explorer.roax.net/tx/…` link) or `proposed` (ownership sits with the governance signer post Phase-2 → the `{target, calldata, holder}` payload is rendered for out-of-band execution, nothing broadcast). An **authority banner** at the top reads `GET /v1/admin/governance/authority` and tells the operator up-front which path a deploy will take ("Hosted key deploys directly" vs "Deploys route to governance as proposals"). This is why the tooling `ADMIN_PRIVATE_KEY` must be signer-1 `0x8E27E117…` post-handover — otherwise every deploy comes back `proposed` rather than executed.
- **Clone list is best-effort.** The table reads `GET /v1/admin/activity/issuers` (needs the oversight indexer); a 503/unwired indexer degrades to an inline "activity unavailable" note WITHOUT breaking deploys or the preview (those need only the chain). Client types + `predictIssuer`/`createIssuer`/`governanceAuthority`/`listIssuers` methods live in `packages/ui/src/api/{types,central}.ts`. Web has no unit suite — `tsc --noEmit` + `vite build` are the gates.

### Admin whitelist management console (PR-E: `admin-whitelist-mgmt`)
Promotes the read-only `stacks/admin/web` Whitelist viewer to a **direct grant/revoke management console** — the whitelisting machinery `approve_application` runs, exposed as a standalone control-plane action decoupled from the issuer-application queue (key rotation, ad-hoc grants, incident response). Web + backend; builds on PR-A's `GovernanceAction`.
- **Two new admin-gated endpoints (`routes.rs`, admin-session):** `POST /v1/admin/whitelist/grant` and `POST /v1/admin/whitelist/revoke`. Body `{ signer, recordType?, verifyPurposes? }` (at least one of `recordType`/`verifyPurposes` required — else 400; malformed signer → 400). **Grant** builds a `whitelistFor` `GovernanceAction` per capability (the `recordType` key via `to_record_type_key` + each `verify_key(purpose)`) and, for a `DOG_PROFILE` recordType, ALSO a `grantRole(ISSUER)` action on the SBT (idempotent: `has_issuer_role` pre-check → `{status:"alreadyHeld"}` when already held). **Revoke** builds `delistFor` per capability; it does NOT revoke `ISSUER_ROLE` or on-chain roots (that is a DEFAULT_ADMIN `adminRevoke`, a PR-F Governance action) — mirrors `delist_application` (delistFor only).
- **Everything routes through `governance::dispatch` (never the direct `whitelist_for`/`delist_for` path).** The whitelist capabilities are gated by `Authority::Role{registry, whitelist_admin_role(), default_admin:false}`; the DOG_PROFILE ISSUER grant by `Authority::Role{sbt, default_admin_role(), default_admin:true}` (the SBT is `AccessControlDefaultAdminRules`, so `defaultAdmin()` resolves the holder). Response: `{ signer, recordType, actions: [Disposition…], issuerRole? }` — each `Disposition` is `executed{txHash,holder}` (hosted key holds the role) or `proposed{holder,target,calldata,authority}` (role moved to governance). So a grant/revoke flips executed→proposed by construction the moment WHITELIST_ADMIN leaves the hosted key (Phase-2), exactly like the factory deploy.
- **Web (`pages/Whitelist.tsx`, now "Whitelist management"):** the derived + live-`isWhitelistedFor` state view is kept; each (recordType,address) row gains **Grant**/**Revoke** buttons behind a confirm dialog, plus a header **"Grant capability"** dialog for an arbitrary (signer, recordType, verifyPurposes) pair. Each dispatched capability renders inline as a `Disposition`: executed → `explorerTxUrl` link, proposed → holder + authority + truncated calldata. After an action the affected row re-reads on-chain. Client: `central.whitelistGrant`/`whitelistRevoke` (`packages/ui/src/api/central.ts`) + `GovernanceDisposition`/`WhitelistActionReq`/`WhitelistGrantResp`/`WhitelistRevokeResp` types. The nav label `Layout.tsx` changed `Whitelist viewer` → `Whitelist`.
- **Tests:** `tests/control_plane.rs` (7 new, MemChain): grant proposes when hosted lacks WHITELIST_ADMIN / executes all capabilities (recordType + 2 verify purposes) when it holds it; DOG_PROFILE grant also executes the ISSUER-role grant; revoke executes; requires-admin (401); missing-capability + bad-signer (400). Set the hosted role via `chain.set_role(REGISTRY, &whitelist_admin_role(), HOSTED)` / `set_role(SBT, &default_admin_role(), HOSTED)`.

### Vet/groomer verification audit history (verify2-s4)
The shared vet/groomer verifier flow now keeps a durable operator-visible audit history for owner-consent verification sessions, using the existing `VerifySession` rows instead of a parallel table. `VerifySession` carries `created_at`/`updated_at`; the status lifecycle is `pending` -> `recording` -> `recorded` or `error`. `GET /verify/history` is operator-gated and returns most-recent-first rows with purpose, recordType, mode, relayer, status, txHash, explorerUrl, nullifier, and timestamps. It intentionally stores verifier operational proof metadata only, not credential PII. `packages/ui` exposes `verificationHistory()` plus `VerificationHistoryPanel`; both `stacks/vet/web/src/pages/Verify.tsx` and `stacks/groomer/web/src/pages/Verify.tsx` render it under the QR export flow. Hermetic coverage lives in `stacks/vet/api/tests/flow_memchain.rs::verify_session_status_polls_pending_to_recorded` and checks auth gating plus pending -> recorded history rows.

### dogTagId encoding (easy to get wrong)
The operator-facing **handle** is a small integer. The **on-chain** dogTagId minted into `DogTagSBT` and emitted as the circuit's `pub[0]` is the Poseidon **field-hash** of that handle: `routes::onchain_dog_tag_id(handle)` = `to_hex32(field_of_value(Integer(handle)))` (mirrors the `dog_tag_id_field_hex` FFI / `field-hash` bin). The SBT is keyed by the field element, NOT the raw handle — `ownerOf`/`profileRoot` lookups (and tests) must field-hash first.

## Deployment / production guards (fail-closed)
- Demo vs prod is gated by `DEMO_MODE` / `VITE_DEMO_MODE` (set = demo/local, unset = production).
- Both backends call `startup::validate_production_secrets(...)` at boot: in production they **refuse to start** if `OPERATOR_PASSWORD`/`ADMIN_PASSWORD`/`CENTRAL_HMAC_SECRET` (vet) or `ADMIN_PASSWORD`/`ADMIN_PRIVATE_KEY` (admin) are unset or equal to the known dev defaults. Set `DEMO_MODE=1` to keep the convenient demo defaults.
- vet-api: if `CIRCUITS_BUILD_DIR` is set but the real `ArkProver` fails to load, the process **exits** (it must not silently degrade to `StubProver`, which emits zeroed proofs the chain rejects). Unset `CIRCUITS_BUILD_DIR` still uses `StubProver` (demo / on-device-proof production model).
- The prover **enforces a pinned zkey sha256** (`dogtag-prover-rs::EXPECTED_ZKEY_SHA256_HEX`, the testnet ceremony hash): `Prover::load` rejects any zkey whose hash differs, so a swapped/corrupt key fails closed instead of proving against the wrong key (audit M4). A deployment shipping a **different** zkey (a production ceremony output) sets the `EXPECTED_ZKEY_SHA256` env var on vet-api (→ `load_with_expected_zkey`) — a config swap, not a code change. Leave it unset to enforce the bundled testnet hash.
- **Shared JWT signing key** (`SHARE_JWT_SIGNING_KEY`, 32-byte hex; vet + admin): the Ed25519 share/record JWT key. MUST be identical across restarts and horizontally-scaled instances or tokens break (audit L4). `load_jwt_keys` requires it (fail-closed) in production (same `DEMO_MODE` signal as the secret guard above), and uses an ephemeral key + warning in demo. `JwtKeys::generate()` alone is per-process/ephemeral — never the production path.
- **Admin password hashing** (`ADMIN_PASSWORD_HASH`, `"<salt_hex>$<hash_hex>"` from `auth::hash_password`; admin): the stored hash `admin_login` verifies against with `auth::verify_password` (audit L4 — replaces the old cosmetic plaintext compare). Optional; unset → the H2-required `ADMIN_PASSWORD` plaintext is hashed once at startup.

## ZK trusted-setup ceremony

- Two scripts, do not confuse them: `circuits/scripts/setup.sh` is the **DEV/TEST** single-contributor setup (self-generated ptau, throwaway beacon) and must never secure production; `circuits/scripts/ceremony.sh` is the **production** multi-party ceremony (public Hermez phase-1 ptau + ≥3 independent contributors + public beacon). Subcommands: `init` → `contribute IN OUT "name"` (×N) → `beacon LAST 0x<hex> "note"` → `finalize`.
- Security model is **1-of-N honest, NOT majority/multisig**: the setup is sound if *any one* contributor destroys their toxic waste (entropy); broken only if *all* collude. So maximize diverse, independent contributors — adding more can only help. Do not describe it as a threshold/quorum scheme.
- The testnet key currently on-chain is a **single-operator self-run** (`docs/CEREMONY_TRANSCRIPT.md`, audit Finding H3) → forgeable; production requires re-running `ceremony.sh` per `docs/CEREMONY_RUNBOOK.md`. The ceremony gates only the ZK path (`recordVerificationZK`); the ECDSA path and three-pillar trust model are unaffected.
- Circuit `DogTagVerification(24,5)` = 94,459 constraints → needs **2^17** powers of tau (`PTAU_POW=17`).
- Final artifacts: `circuits/build/verification_final.zkey` (proving key the Rust prover loads + pins SHA-256, impl §11.8(f)), `circuits/Groth16Verifier.sol` (vkey compiled in → deployed), `circuits/build/verification_key.json` (for `snarkjs groth16 verify`). `finalize` exports all three; verify with `snarkjs zkey verify r1cs ptau zkey` → `ZKey Ok!`.
- On-chain verifier swap has **no single-call setter**: `VerificationRegistry.proposeZkVerifier(addr)` → wait `ZK_TIMELOCK = 2 days` → `executeZkVerifier()`; confirm with `zkVerifier()`. Live registry `0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1` (`contracts/deployments/roax.json`).
- The **v2 ceremony verifier `0xEEFCfAF026931b7325472A88fd14Ee780Da13559` is the LIVE on-chain verifier** since the 2026-07-02 `executeZkVerifier()` cutover (tx `0xe2e3270f…40e70`, block 103419); the v1 verifier `0x138b4330…1761` is retired and rejects v2-key proofs (and vice versa). The live verifier address is baked in several places that must move together on any future swap: `contracts/deployments/roax.json`, `README.md` (Live ROAX addresses table), `stacks/owner/web/src/lib/config.ts`, `packages/ui/src/wallet/contracts.ts`, `scripts/e2e-zk.sh` (`ZKV=`), the live-chain parity tests (`crates/dogtag-standard-rs/tests/prove_parity.rs`, `stacks/vet/api/tests/prove_verification.rs`), and the docs that quote the live address (`docs/DEPLOY.md`, `docs/DEPLOYMENT.md`, `docs/DEMO.md`, `docs/CEREMONY_RUNBOOK.md`).

## Mobile end-to-end testing (Android, on-device ZK proof)

The Android app's on-device Groth16 proving flow has a real device/emulator e2e driven by
[Maestro](https://maestro.mobile.dev): `apps/android/maestro/zk_e2e.yaml`. It exercises the SAME
native code path the privacy-preserving groomer export uses — UniFFI → Rust SDK + circom-prover
(graph witness calculator) + the bundled proving key — with no camera, biometric, or network.

### How the e2e works (and why it's shaped this way)

The production export→prove path is entangled with the camera QR scan, a biometric prompt, live
ROAX-chain RPC calls (groomer whitelist, bind nonce, `consumed(nullifier)` polling) and a groomer
host — none reliably automatable on an emulator. So instead of faking all of that, the e2e drives a
**debug-only ZK self-test** on the Profile screen (`ui/screens/ZkSelfTest.kt`, gated by
`BuildConfig.DEBUG` — never in release). It runs, on-device:

1. `signConsentEddsa` — EdDSA-BabyJubjub consent signature (the circuit re-verifies it inside the proof).
2. `proveVerification` — the REAL on-device Groth16 proof (graph witnesscalc + bundled zkey).
3. public-signal check — the proof's 7 `pubSignals` must equal the server-recomputed vector, plus the
   32-bit-ARM regression guard (nullifier `pub[4]` and keyHash `pub[5]` non-zero).
4. `keyHashHex` + `bindConsentKeyDigestHex` — the consent-key bind digest.

It renders the stable text `ZK-SELFTEST: PASS` / `ZK-SELFTEST: FAIL` that the Maestro flow asserts on.
The Maestro flow also asserts the Verify tab's `mobile root == server root: PASS` (the import/issuance
trust core through the native `.so`).

The fixed input vector is `apps/android/app/src/main/assets/zk_selftest.json` (committed, small). It is
generated by, and byte-for-byte mirrors, `crates/dogtag-standard-rs/tests/prove_parity.rs`
(`fixed_prove_inputs`), so the device proof MUST reproduce the same public signals the server SDK
computes. Regenerate it after any change to that test/circuit:

```bash
cargo test -p dogtag-standard-rs --features prover dump_selftest_fixture -- --nocapture
```

### Running the e2e locally

A 64-bit (**arm64**) runtime is required — the prover ships only as `arm64-v8a` / `armeabi-v7a`
native libs, so an x86_64 emulator cannot load them. On this machine the SDK is at
`~/Library/Android/sdk` and the `roax_test` AVD is already `arm64-v8a` / android-34.

```bash
export ANDROID_HOME=~/Library/Android/sdk
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/27.0.12077973

# 1. Vendor the gitignored proving artifacts into the app bundle (see docs/MOBILE_BUILD.md §4).
cp circuits/build/verification_final.zkey apps/android/app/src/main/assets/
cp circuits/build/verification.graph      apps/android/app/src/main/assets/   # see graph note below

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

- **Witness graph is not in the repo and not built by the published crate.**
  `circuits/build/verification.graph` (`wtns.graph.001` format, consumed by `circom_witnesscalc::
  calc_witness`) is gitignored AND the published `circom-witnesscalc` 0.2.1 crate ships no
  `build-circuit` binary (only `calc-witness`/`cvm-compile`). It is built from
  `circuits/verification.circom` by iden3's `build-circuit` tool. Validate any graph against the
  zkey with `cargo test -p dogtag-standard-rs --features prover on_device_proof_verifies_and_pub_matches`.
- **arm64 emulator only** — see above. `Build.SUPPORTED_64_BIT_ABIS` being empty (32-bit-only) routes
  to the remote prover-service instead, which is a different (network) path the self-test does not cover.
- **Gradle wrapper jar gitignored** — a global `*.jar` ignore drops `gradle-wrapper.jar`. Use system
  Gradle 9.5.1, or `gradle wrapper` to regenerate it.
- **`buildConfig = true`** is enabled in `app/build.gradle.kts` so `BuildConfig.DEBUG` gates the
  self-test card.
- **`verifyConsentEddsa` SIGSEGVs via JNA on arm64** — calling that specific UniFFI export from Kotlin
  crashed natively on the emulator. It is redundant here (the circuit verifies the EdDSA signature as
  a proof constraint), so the self-test omits it; if you need on-device EdDSA verify, investigate the
  JNA binding for that function before relying on it.

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

### The iOS ZK self-test

`apps/ios/DogTag/ZkSelfTestScreen.swift` (`ZkSelfTestCard`) is the Swift port of Android
`ui/screens/ZkSelfTest.kt`, wrapped in `#if DEBUG` so it never ships in a release build. It runs, on
the device's own arm64 code: `signConsentEddsa` → `proveVerification` (the REAL on-device Groth16
proof) → public-signal check (7/7 == the server-recomputed vector, plus the nullifier/keyHash non-zero
guard) → `keyHashHex` + `bindConsentKeyDigestHex`. It reads the SAME fixed vector both apps share,
`apps/ios/DogTag/zk_selftest.json`, which is byte-for-byte identical to the Android fixture and emitted
by the SAME test (`crates/dogtag-standard-rs/tests/prove_parity.rs::dump_selftest_fixture`, which now
writes both apps' copies):

```bash
cargo test -p dogtag-standard-rs --features prover dump_selftest_fixture -- --nocapture
```

### Building the on-device prover xcframework + running the e2e locally

`DogTagFFI.xcframework` is gitignored and is NOT produced by a plain Xcode build — build it from the
Rust crate (`--features prover`) for the iOS Simulator, regenerate the Swift bindings (keeping the
committed `apps/ios/DogTag/dogtag_standard.swift` ABI-consistent), then assemble it. On an
Apple-Silicon Mac:

```bash
# 1. Vendor the gitignored proving artifacts into the app bundle (docs/MOBILE_BUILD.md §4).
cp circuits/build/verification_final.zkey apps/ios/DogTag/verification_final.zkey
cp circuits/build/verification.graph      apps/ios/DogTag/verification.graph

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

- **xcframework is built `--features prover`** — without it the FFI surface has no `proveVerification`
  and the app won't link the prover symbols. The Swift binding is generated from a host dylib but MUST
  match the linked static lib's ABI; regenerate the `.swift` from the same crate build (step 3) so the
  embedded UniFFI checksums agree, otherwise the app traps at the first FFI call.
- **Simulator slice only** — the committed build path makes a `aarch64-apple-ios-sim` xcframework, so
  building for a *device* destination fails until you add an `aarch64-apple-ios` slice (+ signing). The
  e2e runs on the Simulator, which needs no Apple team.
- **Generated `DogTag.xcodeproj` is committed** — it is produced by `xcodegen` from
  `apps/ios/project.yml`; re-run `xcodegen` (don't hand-edit the project) after adding/removing source
  files, and commit the regenerated `project.pbxproj`.
- **zkey + graph are gitignored** (`apps/.gitignore`) — a fresh checkout has neither; vendor them from
  `circuits/build/` (step 1) or the e2e fails to prove. Validate the graph/zkey pair on the host with
  `cargo test -p dogtag-standard-rs --features prover on_device_proof_verifies_and_pub_matches`.

### CI (iOS)

`.github/workflows/ios-mobile-e2e.yml` builds the xcframework + app and runs this Maestro flow, but is
**`workflow_dispatch`-only** and targets a **self-hosted Apple-Silicon (arm64) macOS runner**:
GitHub-hosted runners don't reliably provide the arm64 Simulator prover slice, and the proving
artifacts are gitignored. Wiring it to push/PR would make a perpetually-red check. The validated signal
is the local run above (this lab: iPhone 16 / iOS 18.6 simulator, real proof, `ZK-SELFTEST: PASS`).
## Contract sharp edges

- `VerificationRegistry.recordVerificationZK(a, b, c, pub[7], bytes32 recordType, uint256 deadline)` —
  the trailing `recordType`/`deadline` are defense-in-depth guards supplied by the relayer (NOT bound to
  the proof; audit L2). Address-typed public signals `pub[2]` (relayer) and `pub[3]` (subject) are
  range-checked `< 2^160` so `uint160(..)` truncation can't alias a victim address (audit L1). The Rust
  relay ABI (`stacks/vet/api/src/chain.rs`) must stay in sync with this signature.

## Governance authority (Phase-2 executed) - tooling signer

- **Governance authority is signer-1 `0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2`; the tooling ADMIN key
  is signer-1.** Governance Phase-2 executed on-chain 2026-07-05 (block 123835), stripping the old deployer
  EOA `0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96` of ALL roles (registry `DEFAULT_ADMIN_ROLE` +
  `WHITELIST_ADMIN`, the `DogTagIssuerFactory` `Ownable2Step` ownership, and `DogTagSBT` `ISSUER_ROLE`) and
  moving them to governance signer-1. Any tooling that signs a privileged write (`whitelistFor` / SBT
  `mint` / factory `createIssuer` / `adminRevoke`) as the old EOA now reverts (or, in the admin control
  plane, downgrades to a `Disposition::Proposed`), so **wire the admin authority to signer-1, never `0x119F…`.**
- **Demo / relayer / demo-script tooling reads signer-1 from a captain-managed env var - the private-key
  VALUE is never committed.** The `scripts/*.sh` demo + e2e harnesses (`demo-up.sh`, `demo-bootstrap.sh`,
  `demo-prepare-phone.sh`, `e2e-smoke.sh`, `e2e-zk.sh`) source **`GOVERNANCE_PRIVATE_KEY` /
  `GOVERNANCE_ADDRESS`** (signer-1) from `contracts/.env` and fail closed if unset; `DEPLOYER_*` is kept
  only for `forge` contract deploys / ceremony scripts (it holds no roles post-Phase-2). The admin stack
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
