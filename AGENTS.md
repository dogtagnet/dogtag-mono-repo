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
- **Mobile `eth_call` selectors must be DERIVED from the signature, never hard-coded.** `apps/*` hand-encode selectors in `RoaxRpc.kt` / `Net.swift` (no ABI lib). `isValid`'s was once the stale literal `0x6d04f0bc` (its comment *claimed* to be the keccak but wasn't) - that selector REVERTS on the deployed ROAX `DogTagIssuer` clone, so every mobile validity read silently fell through to `Unknown`/accept-with-caveat and a revoked credential never showed as revoked. The canonical selector is `keccak256("isValid(bytes32)")[:4] = 0x6a938567` (what viem, the alloy `sol!` ABI, vet-api `verify_credential`, and the web direct-RPC path in `packages/ui` all bind). It is now derived on-device via `Keccak256` (`RoaxRpc.functionSelector` / `Net.swift` `functionSelector`); `apps/android/app/src/test/.../RoaxRpcSelectorTest.kt` pins it. Verify any new mobile selector against the chain before shipping: `eth_call` a real clone (VACCINATION `0x5c703910111f942EE0f47E02214291b5274cDb53` on `https://devrpc.roax.net`) - the correct selector returns a 32-byte word, a wrong one returns `execution reverted`. Note mobile has only the single `isValid` bool (no `issuedAt`/`isRevoked` decomposition like web), so it renders revoked and never-anchored identically as "REVOKED / not anchored"; that is intentional, not a bug.

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
  - **Owner-web receipt renderer (`src/pages/Receipt.tsx`, `/receipt/:root`; index `/receipts`)** - govarch PR-6 holder-side receipt for `TRAVEL_CLEARANCE` and `EU_HEALTH_CERT`, derived entirely from the locally held `WrappedDoc` plus a live `DogTagIssuer.isValid(root)` read. It mirrors the government/mobile receipt anatomy: fixed-light printable sheet, Receipt ID, issuance/validity, Section A/B/C or Annex-IV rows, QR to `https://<issuer.domain>/r/<receiptId>`, root/provenance, and holder-redaction awareness (`privacy.obfuscated[]` count; redacted copies only render leaves still present). Status derivation: `isValid=false` → `REVOKED / not anchored`, else lapsed ISO `validity.validUntil`/`rabiesValidUntil` → `EXPIRED`, else `VALID`; wallet cards/detail reuse this so revoked/expired receipts are not mislabeled as merely "not anchored". No backend, no new PII, no ZK on this path.
  - **Selective disclosure / "Share a redacted copy" (`src/pages/Share.tsx` at `/share/:id`, logic in `src/lib/redact.ts`)** - the Merkle counterpart to the ZK Present flow, and the web mirror of the native apps' "Share redacted" (mobile FFI `obfuscateDocumentJson`). The holder toggles which leaves to reveal; withheld leaves run through `@dogtag/standard`'s `obfuscate` (leaf hash → `privacy.obfuscated[]`, cleartext dropped, **Merkle root R unchanged**), so the recipient still `checkIntegrity`-verifies the SAME authentic credential + can read `isValid` on-chain, seeing only revealed fields. Default = reveal-all (the holder explicitly withholds; no fragile PII classifier). `credentialSubject.dogTagId` is **locked-on** (`NON_OBFUSCATABLE_PATHS`, mirrors verify's `NON_OBFUSCATABLE` - withholding it would fail integrity), and `recordType` is **locked as public** (`PUBLIC_PATHS` - its value is also carried in the always-revealed `issuer` block, so a toggle to "withhold" it would be a lie). Output is copy-JSON + download (same paste-JSON idiom as Receive / the issuers' "Copy wrapped document"); NO ZK on this path, NO backend, no store mutation (the held full credential is untouched). Reached via a "Share a redacted copy →" button on `CredentialDetail`.
- **Owner web e2e (Playwright)**: `stacks/owner/web/e2e/owner.spec.ts` drives the whole holder loop (receive → hold/display → generate ZK proof → present → verified) + a tamper-rejection test + a **receipt test** (receive the CDC-modeled travel sample → `/receipts` → `/receipt/:root` renders Receipt ID, Section A/B/C, QR/public URL, derived status/provenance) + a **selective-disclosure test** (open Share → withhold a field → the redacted copy still `checkIntegrity`-verifies with the SAME `merkleRoot` + the withheld cleartext gone + `privacy.obfuscated` grown; re-importing that redacted copy makes the receipt omit the withheld value and show the obfuscated-count notice). Like the government e2e it is NOT in `pnpm test`/CI. It starts its OWN vite dev server and **mocks the prover + verifier + ROAX RPC** at the network layer (deterministic), but runs the REAL client-side crypto. `pnpm --filter @dogtag/owner-web test:e2e`; `OWNER_URL=<url>` runs it against a live wallet instead (no self-server).

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

### Self-custody export UX (iOS, `apps/ios/DogTag`)
Holder-side backup/migration rights: copy the phrase at creation, re-export it later, and export held credentials. Single account only (no HD multi-account/derivation switcher - deliberately scoped out).
- **The embedded wallet now persists the 32-byte BIP-39 *entropy*** (`Wallet.swift`, Keychain account `dogtag_wallet_entropy`, SAME protection as the seed: `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, no `kSecAttrSynchronizable`, so never iCloud-synced). This is a **deliberate change to the prior "mnemonic never persisted" property**: BIP-39 seed to mnemonic is one-way, so without the entropy the 24 words are unrecoverable after genesis, and "export your recovery phrase later" is impossible. `Wallet.revealMnemonic()` re-derives the exact phrase via `Bip39.entropyToMnemonic`; it returns `nil` for wallets created before this change (seed-only), whose phrase is genuinely gone. The entropy is no more sensitive than the already-stored seed (both fully control the wallet). Callers gate `revealMnemonic()` behind a fresh `Biometric.authenticate` and never log/transmit it.
- **`ProfileScreen.swift`** owns the wallet/Profile screen (a *separate* crew owns the rest of the app UI). At creation the recovery-phrase card has a **Copy phrase** action (auto-expiring pasteboard via `SecureClipboard.copySecret`, 90s TTL) + an "I've saved it" acknowledgment that hides it. A biometric-gated **Export account keys** button opens `ExportAccountSheet` (hard security warning + numbered 24-word grid + copy). Every displayed value is **tap-to-copy with a "Copied" flash** via the reusable `CopyRow` (wallet address, Consent Ax, keyHash, dog-tag ids; copies the FULL value, not the truncated preview). **Sharp edge:** the export sheet MUST be presented with `.sheet(item: $exportPayload)` (an `Identifiable` payload carrying the revealed secrets), NOT `.sheet(isPresented:)` reading sibling `@State` set in the same handler - SwiftUI evaluates that sibling state as still-nil on the first present, so the phrase/key silently render as "unavailable" even though `revealMnemonic()`/`revealPrivateKeyHex()` returned real values. Dismissing the `.sheet(item:)` nils the binding, which also releases the secrets from memory.
- **Document export** (`DocumentsScreen.swift` + `CredentialDetailScreen.swift`) uses the app's existing `WrappedDoc` JSON as the portable form, **no new format**. `ExportedDocument: Transferable` + native SwiftUI `ShareLink`/`SharePreview`; single credential = its `wrappedDocJson` verbatim, list export = a JSON array of the shown docs (respects the pet filter). No `UIActivityViewController` bridge added; `TravelReceiptView`'s existing redacted-share `ShareSheet` is left untouched. **Sharp edge:** use `FileRepresentation(exportedContentType: .json)` + `SentTransferredFile` (both iOS 16), which carry the filename via the written temp-file URL. Do NOT use `DataRepresentation(...).suggestedFileName {...}` - `suggestedFileName` is iOS 17+ and the deployment target is 16.0, so it fails the build.
- **Raw secp256k1 private-key export is included** (captain-approved) alongside the phrase in the same biometric-gated `ExportAccountSheet`: `Wallet.revealPrivateKeyHex()` returns the 0x-hex 32-byte key, shown with its own hard warning and tap-to-copy (auto-expiring clipboard), never logged/transmitted. It matters BECAUSE the private key is NOT a subset of the phrase for migration: this wallet derives the secp key as the **raw BIP-32 master key** (`Bip39.seedToSecp256k1Priv` = HMAC-SHA512("Bitcoin seed", seed)[:32]), **not** `m/44'/60'/0'/0/0`, so the mnemonic imported into a standard EVM wallet yields a DIFFERENT address; only the raw private key reproduces the on-chain `userWallet` elsewhere. Available even for legacy seed-only wallets (needs only the seed, not the entropy).

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

### Role-traceability portals (govarch PR-5: `dogtag-trace-w6`)
The per-role **consumers** of the PR-4 oversight indexer, one tier up from the admin PR-B consumer: each role sees the on-chain credential activity relevant to IT, joined to its own off-chain DB records. Three views, one doctrine — **government is UNSCOPED (every issuer), vet/groomer are SCOPED (own signer/clone only)**. No new PII; the feed is the non-PII chain layer (the join projection deliberately EXCLUDES the government TRAVEL_CLEARANCE `subject`/importer PII block).
- **`OversightFeed` client, ported per stack.** `stacks/vet/api/src/oversight.rs` (serves vet + groomer — same binary) and `stacks/government/api/src/oversight.rs` each carry the same trait (`events`/`stats`/`issuers`/`status`) + `HttpOversightFeed` (reqwest → indexer `:46001`) + `DisabledFeed` (unset `INDEXER_API_BASE` → `NotConfigured` → 503) + `MemFeed` (hermetic tests), mirroring the admin `src/indexer.rs`. Injected as `AppState.feed: Arc<dyn OversightFeed>`. Government had no `reqwest` dep — added inline (`0.12`, `rustls-tls`).
- **Two-layer server-side scoping (the load-bearing property).** (1) The INDEXER scopes by bearer token (a vet/groomer presents a SCOPED `INDEXER_SCOPES` token → `Scope::Signers`; the government presents the `unscoped:true` token). (2) The role backend RE-CHECKS every returned event against a **local scope gate** (`crate::trace::LocalScope::admits` — `actor ∈ own signers OR clone ∈ own clones`, the same rule as the indexer's `scope.rs`), built from the operator's own config issuer-clones + custody signer accounts + the signer/clone/relayer addresses on its own records+sessions (zero address never widens scope). So even a mis-scoped indexer token can never leak another operator's event into a vet's view. The government passes `scope = None` (admits everything, unscoped). This defense-in-depth makes "a vet cannot fetch another vet's activity" testable at the role layer without a live indexer.
- **The DB-record join (`crate::trace`).** Each on-chain event is matched to the operator's own record: vet/groomer by anchored `root` (issuances/revocations) or verification `nullifier` / tx (verifications) → `Record`/`VerifySession`; government by `root` / tx → `IssuedCredential`/`VerificationRecord`. The matched record's non-PII summary is attached as the event's `local` field (`null` when on-chain activity has no local record — a drift signal that is still shown in-scope).
- **Endpoints.** Vet/groomer (operator-session gated, `public_router`): `GET /trace/activity` (scoped + gated + joined; envelope adds `inScope`/`matched`/`droppedOutOfScope`/`localScope`), `GET /trace/stats` (indexer scoped counters + own record/session counts). Government (`GOV_API_TOKEN` gated): `GET /v1/oversight/activity` (unscoped + joined; `matched` = how many cross-issuer events are the authority's own), `GET /v1/oversight/stats`, `GET /v1/oversight/issuers`. Unconfigured indexer → 503 `{indexer:"not-configured"}` (rest of backend unaffected).
- **Web.** New nav+route+page per app (Waypoints icon): vet/groomer `pages/Traceability.tsx` ("Traceability") share the `@dogtag/ui` client — added `traceActivity`/`traceStats` + `Trace*` types to `packages/ui/src/api/{client,types}.ts`; government `pages/Oversight.tsx` ("Oversight") uses its local `lib/api.ts` (`apiGetResult` surfaces the 503 for a first-class "indexer not connected" state) + `VITE_GOV_API_TOKEN`. Each renders the joined feed with the local record highlighted, finality badges, and explorer links.
- **Config.** `INDEXER_API_BASE` for all three; vet/groomer add `INDEXER_SCOPED_TOKEN` (alias `INDEXER_TOKEN`), government adds `INDEXER_OVERSIGHT_TOKEN` (alias `GOV_INDEXER_TOKEN`). `scripts/demo-up.sh` starts the indexer (`INDEXER_DEMO_MODE=1`, `:46001`) and wires all three with the two well-known demo tokens. DEMO CAVEAT: the indexer's scoped demo token is bound to a FIXED stand-in signer/clone, so a freshly-genesis'd vet/groomer sees "0 in scope" (its local gate correctly rejects the demo-groomer's events) until its real signer is added to `INDEXER_SCOPES`; the government unscoped view always shows the full scripted cross-issuer feed.
- **Tests.** `stacks/vet/api/tests/trace.rs` (scoping: foreign vet excluded; join; auth; 503) + `crate::trace`/`crate::oversight` unit tests; `stacks/government/api/tests/oversight.rs` (unscoped feed sees all issuers, own highlighted, non-PII; auth; 503). Web e2e mirror the mocked-`/api/`-regex style: `stacks/{vet,groomer}/web/e2e/traceability.spec.ts`, `stacks/government/web/e2e/oversight.spec.ts` (not in CI — need a served portal). All 4 test constructors in vet `tests/common/mod.rs` + the 3 gov test `AppState` literals gained a `feed` field (default `DisabledFeed`; trace tests override `state.feed` with a seeded `MemFeed`).

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

### Verifier direct credential status (issuer-c3)
The vet/groomer verifier product now has a direct, operator-facing **pasted credential check** in addition to the existing owner-consent proof-export flow. It is intentionally NON-admin-nav work.
- **Backend (`stacks/vet/api`)**: `POST /verify/credential` (plus `/v1/verify/credential` alias) is operator-session gated and non-persistent. Body `{ wrappedDoc, issuerAddr?, signerAddr? }`; it recomputes wrapped-doc integrity with `dogtag_standard::verify::check_integrity`, defaults the issuer clone from `wrappedDoc.issuer.documentStore`, reads `DogTagIssuer.issuedAt(root)`, `isValid(root)`, and `isRevoked(root)`, and optionally checks `IssuerRegistry.isWhitelistedFor(keccak256(recordType), signerAddr)`. Response includes `{verdict,status,recordType,root,recomputedRoot,issuerAddr,issuedAt,fragments}` where `status` is `valid|revoked|not_issued|integrity_failed|invalid`. The handler stores no pasted credential data, so no new PII store is introduced.
- **Chain surface**: `ChainClient` gained `is_revoked`; `AlloyChain` binds `DogTagIssuer.isRevoked(bytes32)` and `MemChain` reads the existing in-memory revoked map.
- **Web (`stacks/vet/web`, `stacks/groomer/web`)**: the shared `@dogtag/ui` `CredentialVerifyPanel` is mounted on each Verify page above `VerifyFlow`. It accepts wrappedDoc JSON, optional issuer signer, and renders pass/fail plus integrity/on-chain/issued/revoked/whitelist pillars with issuer/root details. **As of `webverify-n3` the panel no longer calls `POST /verify/credential`** - see the next section.
- **Tests/builds**: `stacks/vet/api/tests/flow_memchain.rs::full_issuance_share_revoke_flow` now proves issue -> direct verify valid -> revoke -> direct verify revoked over `MemChain`.

### Web credential verify is permissionless direct-to-RPC (webverify-n3)
Credential verification is permissionless + on-chain, so the web `CredentialVerifyPanel` reads the chain itself instead of the operator-gated `POST /verify/credential`. The server endpoint is retained (it may serve other callers) but the web panel no longer depends on it.
- **Where**: `packages/ui/src/wallet/verifyCredential.ts` `verifyCredentialOnchain(...)` is a byte-for-byte TS port of the Rust `verify_credential` handler's classification. It runs `@dogtag/standard` `checkIntegrity` (pure offline recompute) then reads `DogTagIssuer.issuedAt/isValid/isRevoked` (and optional `IssuerRegistry.isWhitelistedFor`) via viem `eth_call` over the public ROAX RPC (`roax` chain def, chainId 135, `https://devrpc.roax.net`). All chain reads use the **claimed** root (`signature.merkleRoot`); the recomputed root only populates the `recomputedRoot` display field. Returns the identical `VerifyCredentialResp` shape, so the result renderer is unchanged.
- **Reader injection**: reads go through an `IssuerChainReader` interface (default `roaxIssuerChainReader`); tests inject a fake. Hermetic coverage: `packages/ui/test/verifyCredential.test.ts` (needs the `vitest` devDep added to `packages/ui`; picked up by root `pnpm -r --filter "./packages/**" test`).
- **Fail-closed**: an RPC read error rejects (panel shows a toast); it is never silently treated as valid. This is deliberately stricter than mobile, which accepts-with-caveat when the chain is unreachable.
- **Selector gotcha (verified on live chain)**: the deployed `DogTagIssuer.isValid(bytes32)` selector is `0x6a938567` (keccak of the canonical sig; what viem/the Rust ABI bind). The web path uses this selector (matching the server), so it is the faithful on-chain check. Mobile once hard-coded a stale `0x6d04f0bc` in `apps/*/.../RoaxRpc` / `Net.swift`, which **reverts** on the deployment -> its on-chain isValid resolved Unknown and fell back to accept-with-caveat; as of `dogtag-mobilefix-s7` mobile DERIVES the selector on-device (`Keccak256`) instead, so both paths now bind `0x6a938567` - see the mobile-selector note at the top of this file.

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

- This section is the **Level-A `verification.circom`** ceremony. The **Level-B `consent.circom`** circuit has its OWN M3 ceremony — see "M3 trusted-setup ceremony" under "Level-B `DogTagConsent` circuit (M2)" and `docs/CEREMONY_TRANSCRIPT.consent.md`. Three ceremony scripts now exist, do not confuse them: `scripts/setup.sh` (DEV verification), `scripts/setup-consent.sh` (DEV consent), and the real ones — `scripts/ceremony.sh` (verification, multi-party) + `scripts/ceremony-consent.sh` (consent, testnet single-contributor).
- Two scripts, do not confuse them: `circuits/scripts/setup.sh` is the **DEV/TEST** single-contributor setup (self-generated ptau, throwaway beacon) and must never secure production; `circuits/scripts/ceremony.sh` is the **production** multi-party ceremony (public Hermez phase-1 ptau + ≥3 independent contributors + public beacon). Subcommands: `init` → `contribute IN OUT "name"` (×N) → `beacon LAST 0x<hex> "note"` → `finalize`.
- Security model is **1-of-N honest, NOT majority/multisig**: the setup is sound if *any one* contributor destroys their toxic waste (entropy); broken only if *all* collude. So maximize diverse, independent contributors — adding more can only help. Do not describe it as a threshold/quorum scheme.
- The testnet key currently on-chain is a **single-operator self-run** (`docs/CEREMONY_TRANSCRIPT.md`, audit Finding H3) → forgeable; production requires re-running `ceremony.sh` per `docs/CEREMONY_RUNBOOK.md`. The ceremony gates only the ZK path (`recordVerificationZK`); the ECDSA path and three-pillar trust model are unaffected.
- Circuit `DogTagVerification(24,5)` = 94,459 constraints → needs **2^17** powers of tau (`PTAU_POW=17`).
- Final artifacts: `circuits/build/verification_final.zkey` (proving key the Rust prover loads + pins SHA-256, impl §11.8(f)), `circuits/Groth16Verifier.sol` (vkey compiled in → deployed), `circuits/build/verification_key.json` (for `snarkjs groth16 verify`). `finalize` exports all three; verify with `snarkjs zkey verify r1cs ptau zkey` → `ZKey Ok!`.
- On-chain verifier swap has **no single-call setter**: `VerificationRegistry.proposeZkVerifier(addr)` → wait `ZK_TIMELOCK = 2 days` → `executeZkVerifier()`; confirm with `zkVerifier()`. Live registry `0x4E2f0996e1CB4E24F1053346f3da2186906835E8` (`contracts/deployments/roax.json`; the prior `0x8bA836eCe9…` is `VerificationRegistry_4arg_legacy`).
- **The live `VerificationRegistry` address is baked into MANY committed consumers that must move together on any redeploy** (the 4-arg→6-arg fix split-brained precisely because only 2 of them were updated). The full set: `contracts/deployments/roax.json` (canonical; keep the old address as `VerificationRegistry_4arg_legacy`), the two compile-time mobile bundles `apps/ios/DogTag/roax.json` + `apps/android/app/src/main/assets/roax.json` (rebuild+reinstall both), the web/shared config `packages/ui/src/wallet/contracts.ts` + `stacks/owner/web/src/lib/config.ts`, the oversight indexer's `DEFAULT_VREG` in `stacks/indexer/api/src/main.rs` (its anti-spoof gate silently drops `Verified` logs from any other address) + `stacks/indexer/.env.example`, the demo/e2e scripts `scripts/{e2e-zk,demo-up,e2e-smoke}.sh` (`VR=`), the `stacks/{vet,groomer,admin,indexer}/**/.env.example` files, and the live-address tables in `README.md` + `AGENTS.md` + `docs/{DEPLOY,DEPLOYMENT,DEMO,GROOMER_ZK_DEMO,REMOTE_DEPLOYMENT,CEREMONY_RUNBOOK}.md`. `RedeployVerificationRegistry.s.sol` prints this checklist post-deploy. Do NOT rewrite the historical records (`roax.json` `_4arg_legacy`/`_zk_verifier_swap`/`_verification_registry_redeploy` fields, `docs/CEREMONY_TRANSCRIPT.md`) — those intentionally pin the old address.
- The **v2 ceremony verifier `0xEEFCfAF026931b7325472A88fd14Ee780Da13559` is the LIVE on-chain verifier** since the 2026-07-02 `executeZkVerifier()` cutover (tx `0xe2e3270f…40e70`, block 103419); the v1 verifier `0x138b4330…1761` is retired and rejects v2-key proofs (and vice versa). The live verifier address is baked in several places that must move together on any future swap: `contracts/deployments/roax.json`, `README.md` (Live ROAX addresses table), `stacks/owner/web/src/lib/config.ts`, `packages/ui/src/wallet/contracts.ts`, `scripts/e2e-zk.sh` (`ZKV=`), the live-chain parity tests (`crates/dogtag-standard-rs/tests/prove_parity.rs`, `stacks/vet/api/tests/prove_verification.rs`), and the docs that quote the live address (`docs/DEPLOY.md`, `docs/DEPLOYMENT.md`, `docs/DEMO.md`, `docs/CEREMONY_RUNBOOK.md`). The **mobile apps also carry the coupling** - each bundles the verifier's paired zkey/graph plus `roax.json` addresses and must be rebuilt + reinstalled on any swap (see "Building the mobile (iOS) holder app").

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
  files, and commit the regenerated `project.pbxproj`. **Trap:** `xcodegen` enumerates the `DogTag/`
  folder, so regenerating in a checkout that has NOT vendored `verification_final.zkey` +
  `verification.graph` (both gitignored) silently DROPS those two Copy-Bundle-Resources entries from the
  committed `pbxproj` — vendor them first (step 1) or the prover bundle breaks. A pure-UI change that
  adds no source file needs no regen at all: fold new views/types into an existing `.swift` and the
  `pbxproj` stays untouched.
- **Local pet photos are UI-only** — `PetPhotoStore` (LocalStore.swift) keeps per-`dogTagId` avatars as
  JPEGs under `Documents/pet-photos/`; deliberately separate from `Pet` (which `mergeCentralPets`
  overwrites) so a photo survives central sync. Never uploaded, never on-chain, never in a credential.
- **zkey + graph are gitignored** (`apps/.gitignore`) — a fresh checkout has neither; vendor them from
  `circuits/build/` (step 1) or the e2e fails to prove. Validate the graph/zkey pair on the host with
  `cargo test -p dogtag-standard-rs --features prover on_device_proof_verifies_and_pub_matches`.

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
`--features prover` is mandatory: it compiles in the on-device Groth16 prover (`crates/dogtag-standard-rs/src/prover_ffi.rs`, gated `#[cfg(feature = "prover")]`); without it the `proveVerification` symbol is absent and the device build fails to link.

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

Copy the proving key + witness graph into `apps/ios/DogTag/` (both gitignored, absent on a fresh checkout; `docs/MOBILE_BUILD.md` §4):

```bash
cp circuits/build/verification_final.zkey apps/ios/DogTag/verification_final.zkey
cp circuits/build/verification.graph      apps/ios/DogTag/verification.graph
```

**The `verification.graph` is not produced by a plain checkout.**
`circuits/build/verification.graph` is itself gitignored and is built from `circuits/verification.circom` by iden3's `build-circuit` tool (see the graph note in the e2e "Sharp edges / gotchas"); if it is missing, build it before this copy - a 26-byte `stub-graph-for-build-only` placeholder will NOT prove.
Validate the vendored pair on the host: `cargo test -p dogtag-standard-rs --features prover on_device_proof_verifies_and_pub_matches`.

### 3. Regenerate the Xcode project + set the signing team

The project is generated from `apps/ios/project.yml` by `xcodegen`; the signing team is the `settings.base.DEVELOPMENT_TEAM` line there (with `CODE_SIGN_STYLE: Automatic`).
Set it to **your** Apple Developer team, then regenerate - editing the generated `DogTag.xcodeproj` does not stick:

```bash
# edit apps/ios/project.yml -> settings.base.DEVELOPMENT_TEAM: <YOUR_TEAM_ID>   (repo default: AYDBUX9433)
cd apps/ios && xcodegen
```

**Trap:** `xcodegen` enumerates `DogTag/`, so regenerating BEFORE step 2 silently drops the `verification_final.zkey`/`verification.graph` Copy-Bundle-Resources entries from the `pbxproj` (see the xcodegen traps under "Sharp edges / gotchas (iOS)" and "Building / verifying UI changes"). Vendor first, regenerate second.

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

The bundled `verification_final.zkey` + `verification.graph` + the compiled-in FFI prover **must match the ZK verifier currently deployed on-chain** - `VerificationRegistry.zkVerifier()` for the target chain.
Unlike the **server** prover, which fails closed on a mismatched key (it pins `EXPECTED_ZKEY_SHA256_HEX`, `crates/dogtag-prover-rs/src/lib.rs`; see "Deployment / production guards"), **the mobile bundle has no such guard** - it will happily ship any zkey and emit proofs the chain rejects.
A stale bundled key produces a proof the on-chain verifier refuses: `VerificationRegistry.recordVerificationZK` reverts at `require(zkVerifier.verifyProof(...), "bad proof")` (`contracts/src/VerificationRegistry.sol`), surfacing to the operator as **`recordVerificationZK ... "bad proof"`** or a bare **`execution reverted, data: "0x"`**.
This is audit finding **H-1 (no zkey<->verifier version handshake)** made concrete: nothing on-chain advertises which zkey it expects, so the match is a **manual, mobile-side responsibility**.

Check it on every build:

```bash
# 1. hash the key you are bundling
shasum -a 256 apps/ios/DogTag/verification_final.zkey
# 2. read the LIVE on-chain verifier (ROAX; addresses in contracts/deployments/roax.json)
cast call 0x4E2f0996e1CB4E24F1053346f3da2186906835E8 "zkVerifier()(address)" --rpc-url https://devrpc.roax.net
```

The bundled zkey's sha256 must be the ceremony output paired with whatever `zkVerifier()` returns.
Currently (see the "ZK trusted-setup ceremony" section and `roax.json` `_zk_ceremony`/`_zk_verifier_swap`) the live verifier is the **v2** `0xEEFCfAF026931b7325472A88fd14Ee780Da13559`, paired with zkey sha256 `9e3636b9…`; the retired **v1** verifier `0x138b433071Ad806E841B5AD53623290a9bf21761` pairs with sha256 `45d0b6fb…`, and a v1-key proof reverts "bad proof" against the v2 verifier (and vice versa). Do not transcribe these values into new places - `roax.json` and the ceremony section own them.

**Rebuild + reinstall the app whenever the on-chain verifier is upgraded** - a trusted-setup/ceremony cutover done via `proposeZkVerifier(addr)` -> wait `ZK_TIMELOCK` (2 days) -> `executeZkVerifier()` (there is no single-call setter).
Re-vendor the new ceremony's zkey/graph (step 2), rebuild the xcframework (step 1), reinstall (step 4).
An already-installed app keeps proving against its **baked** key until you do, so a phone left on the old build silently starts reverting the moment the cutover lands on-chain.

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
- The on-device ZK circuit (`circuits/verification.circom` `DogTagVerification(24, 5)`,
  `crates/dogtag-prover-rs` `pub const N = 24`) proves at most 24 Merkle leaves; more aborts with
  `too many leaves: <n> > N=24`. `ZkCircuit.maxLeaves` (Models.swift) is the display-layer mirror —
  keep it in sync if the circuit width changes.
- A record's leaf count == `WrappedDoc.decodedFields().count`, which flattens `data` identically to the
  prover's `flatten_data` (both skip empty collections and count only string leaves), so the app's count
  always matches the prover's on the same doc. DOG_PROFILE credentials are ~34 leaves (they wrap the full
  VC envelope) → they EXCEED the limit and map to the `.health` group, so they appear as candidates for a
  VACCINATION export request; the export picker disables them in ZK mode with a "too many fields" note.
  VACCINATION records are ~14 leaves and prove fine.

### Building / verifying UI changes
- Build: `xcodebuild build -project apps/ios/DogTag.xcodeproj -scheme DogTag -sdk iphonesimulator
  -destination 'id=<sim-udid>' CODE_SIGNING_ALLOWED=NO`. SourceKit single-file diagnostics report
  cross-file symbols (Credential, LocalStore, …) as "not found" — those are false positives; only the
  full `xcodebuild` result is authoritative.
- Do NOT re-run xcodegen (`project.yml`) casually: it silently drops the vendored prover resources
  (verification_final.zkey / verification.graph) from the pbxproj. Prefer editing existing `.swift`
  files over adding new ones so the pbxproj (which lists sources individually) needs no regen.
- To eyeball record lists without a backend: install to a booted sim, write `pets.json` +
  `credentials.json` into the app's `get_app_container … data`/Documents dir, relaunch, screenshot.

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

- Rust: `cargo test -p dogtag-standard-rs` (default), and `--features assemble` for the circuit-input
  assembly tests. `--features prover` additionally pulls the heavy on-device Groth16 prover (ark 0.5).
- TS: `pnpm --filter @dogtag/standard test` (vitest) and `... build` (tsc).
- Keep `cargo clippy -p dogtag-standard-rs --lib --bins --tests` warning-clean.

### Regenerating the Swift UniFFI binding (`apps/ios/DogTag/dogtag_standard.swift`)

This file is autogenerated but checked in. When you change `ffi.rs`, regenerate it — but build the
dylib **with `--features prover` first**, otherwise the `prover`/`assemble`-gated symbols
(`proveVerification`, `ProofFfi`, `EddsaSigInput`) are dropped and the binding regresses:

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
`verification.circom` (which exposed `subject` + `keyHash`). **`verification.circom` is frozen — do
not edit it** (it has a pinned dev zkey and its own test); the shared `NodeHash`/`LessThanField`
templates were *copied* into `lib/merkle_inclusion.circom`, not refactored out of it.

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

### Intended on-chain calldata shape (M4 `recordVerificationZK`)

The snarkjs Solidity verifier exposes:

```solidity
function verifyProof(
    uint[2]    _pA,
    uint[2][2] _pB,
    uint[2]    _pC,
    uint[7]    _pubSignals   // == [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
) external view returns (bool);
```

`recordVerificationZK` should therefore take `(a, b, c, pubSignals[7])` (or the same fields unpacked)
and, per spec §"On-chain `recordVerificationZK`":
1. `require(verifyProof(a,b,c,pubSignals))` against the **new VK** (from the M3 ceremony).
2. `require(pubSignals[4] /*R*/ == profileRoot(pubSignals[0] /*dogTagId*/))` — binds the proof to the
   real tag. **This is the only place `dogTagId ↔ R` is checked; the circuit does NOT bind it.**
3. `require(deadline >= block.timestamp)` (pubSignals[6]).
4. `require(!nullifierConsumed[pubSignals[3]])` then consume it.
5. `emit Verified(dogTagId, relayer, purpose, nullifier, deadline, block.timestamp)` — **owner-blind**.
6. **Delete** the old `ownerOf` / `keyOf` checks and the `subject`/`keyHash` handling.

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
per-leaf-random.

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
- **Deployed ROAX `Groth16VerifierConsent`:** `0x272be146C0aEd6401000E9Aa8241201F6f0fdF1a` (chainId 135, `--legacy`, deployer `0x119F8c…`, deploy tx `0xcd1cd5fa…`, block 190760). On-chain `cast code` == the compiled runtime (1933 bytes); `verifyProof`(valid consent proof)=`true`, (tampered `R`)=`false`. Recorded in `contracts/deployments/roax.json` (`Groth16VerifierConsent` + `_m3_consent_verifier`). This is a SEPARATE verifier — it does NOT replace the live Level-A `Groth16Verifier` `0xEEFCf…`; wiring it into `VerificationRegistry` is **M4** (NOT done here).

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
(33/33 green) and is a standalone heavy gate (like `test-circuit`), intentionally **not** in `make test`.

---

## Level-B `VerificationRegistryConsent` (M4) — the owner-blind on-chain verify path

Source of truth: `/Users/zhenhaowu/firstmate/data/dogtag-zkverify-z2/level-b-spec.md`.
Contract: `contracts/src/VerificationRegistryConsent.sol`. Deploy: `contracts/script/DeployConsentRegistry.s.sol`.
Tests: `contracts/test/ConsentRegistry.t.sol` (15, real M3 proof). Fixture: `circuits/scripts/gen-consent-fixture.mjs`.

**Deployed ROAX:** `VerificationRegistryConsent` **`0x57A2998668B0F6332f7342016F5Df2Bb05cB900F`** (chainId 135,
`--legacy`, deploy tx `0x4fb52230…`, block 194489, admin = governance `0x8E27E117…`). It verifies against the
M3 `Groth16VerifierConsent` `0x272be146…`. Recorded in `contracts/deployments/roax.json`
(`VerificationRegistryConsent` + `_m4_consent_registry`).

### ADDITIVE, not a swap — the Level-A registry is still THE live one

`VerificationRegistry` `0x4E2f0996…` remains live and every committed consumer still points at it. M4 does
**not** repoint anything: today's apps still produce **Level-A** proofs (`subject`/`keyHash`) and Level-B
custodial tags do not exist until **M5**, so an early cutover would break the live verify flow. The app
cutover is **M7**; the exhaustive consumer list lives in `roax.json` `_m4_consent_registry.m7_cutover`.
This mirrors how M2 froze `verification.circom` and M3 added `Groth16VerifierConsent` alongside the live
`Groth16Verifier` — **the Level-A registry is FROZEN, not edited.**

### What it does (spec §"On-chain `recordVerificationZK`")

`recordVerificationZK(a, b, c, pub[7])` — **4 args**, because `recordType`/`deadline` are public SIGNALS
now (Level-A took them as unbound calldata). `pub = [dogTagId, purpose, relayer, nullifier, R, recordType,
deadline]`. In order: range-check all 7 signals; `relayer < 2^160` (audit L1); `deadline >= block.timestamp`;
Art. 9 guard; `relayer == msg.sender`; `VERIFY:` whitelist; **`R == profileRoot(dogTagId)`**; `verifyProof`
vs the consent VK; consume the nullifier; resolve `rootIssuer[R]` + `isValid(R)`; emit owner-blind `Verified`.

- **`R == profileRoot(dogTagId)` is THE Level-B binding.** The circuit deliberately does not bind
  `dogTagId ↔ R`, so this is the ONLY place it happens. Without it a prover folds a tree they fully control
  and consents as any tag.
- **No `ownerOf`, no `keyOf`, no `ConsentKeyRegistry` (D2), no Poseidon6.** The nullifier is a public signal
  bound in-circuit to the hidden `ownerSecret` + `consentNonce` (D5); Level-A derived it on-chain from
  `subject`, which Level-B does not have. Constructor is 5-arg `(ir, sbt, zk, ridx, admin)`, not 7.
- **`Verified(dogTagId, relayer, purpose, nullifier, deadline, ts)`** — `subject` is GONE. Same event NAME as
  Level-A but a different signature ⇒ **different topic0**; the indexer decodes by `Verified::SIGNATURE_HASH`
  and will silently skip Level-B events until **M8** teaches it the new shape.

### ⚠ Two traps worth knowing before you touch this

1. **`recordVerificationZK(...)` = selector `0xdd080593`, byte-identical to the RETIRED Level-A 4-arg
   selector** — same ABI shape, completely different `pub` semantics (Level-A `pub[3]`=subject,
   `pub[4]`=nullifier, `pub[6]`=R). A stale pre-PR#7 client aimed here DISPATCHES instead of bouncing, but
   fails closed twice (`R !profileRoot`, then a Level-A proof cannot verify against the consent VK). The
   reverse — a Level-B client aimed at the live Level-A registry (6-arg `0x423a45b6` only) — gives the bare
   `execution reverted, data: "0x"` from `data/dogtag-zkfail-z9`. **Check the ADDRESS first when debugging.**
2. **The Art. 9 constant MUST be reduced mod r.** `recordType` is a public signal, so it is always `< r`,
   while raw `keccak256("SERVICE_ATTESTATION")` (`0xa757…ed43`) **EXCEEDS r** — copying Level-A's raw
   constant makes the guard **dead code that can never fire**. The contract pins
   `SERVICE_ATTESTATION_FIELD = keccak256("SERVICE_ATTESTATION") % r`
   (`10025591956217394737855806998434905929145386518960477508456501950324730293568`); `ConsentRegistry.t.sol`
   recomputes it natively in Solidity and fires it on a REAL proof, so the regression cannot come back
   silently. The same applies to any bytes32 label crossing into a signal (`purpose` is already reduced —
   `packages/dogtag-standard-ts/src/consent.ts`).

### M5 handoff (issuance) — two hard requirements

1. `profileRoot(dogTagId) = R`, the per-tag M1-engine tree root. **No SBT change is needed** — `DogTagSBT`
   already stores `profileRoot` as `bytes32`.
2. **`issue(R)` the root into a `DogTagIssuer` clone too, not only `setProfileRoot`.** The registry keeps
   Level-A's revocation path (`rootIssuer[R]` → clone → `isValid(R)`), so a root that is only set as
   profileRoot and never issued reverts **`unknown root` on every verify**. This is what keeps `revoke(R)`
   working under Level-B.

D1 (custodial mint) needs nothing from the registry: it reads only `profileRoot`, never `ownerOf`.
`test_owner_is_never_read_from_chain` mints to a custodian and proves verification still passes.

### Build / test

```bash
# regenerate the real-proof fixture (needs the committed M3 zkey/wasm) -> contracts/test/consent-fixture.json
node circuits/scripts/gen-consent-fixture.mjs      # or: pnpm --filter @dogtag/circuits run gen-consent-fixture
forge test --match-contract ConsentRegistryTest    # 15 green, incl. a REAL Groth16 proof on-chain
```

Unlike `test-consent`, this suite runs in plain `forge test` (the fixture is committed, so it needs no
circuit toolchain) and is part of the normal contracts gate.
