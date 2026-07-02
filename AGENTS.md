# Project agent memory

This file is the project's committed home for project-intrinsic agent knowledge: build, test, release, architecture, and sharp-edge notes that should travel with the code.

## Build & test (what actually runs offline)

Toolchain: Rust (cargo workspace), Foundry (`forge`/`cast`), Node 22 + pnpm 10, circom 2.1.9 + snarkjs 0.7.6, Docker.

- `cargo check --workspace` / `cargo build` — Rust workspace: `dogtag-standard-rs`, `dogtag-prover-rs`, `vet-api`, `admin-api`, `government-api`.
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
- `circuits` — Groth16 `DogTagVerification(N=24, depth=5)`: Poseidon-Merkle membership + EdDSA consent sig + nullifier + keyHash. Committed artifacts (`verification_final.zkey`, `.r1cs`, `.wasm`, vkey) are a **single-operator testnet** trusted setup — NOT production-secure (run `circuits/scripts/ceremony.sh` with ≥3 independent contributors before mainnet).
- `contracts` — `DogTagSBT` (ERC-5192), `IssuerRegistry`, `DogTagIssuer` clones + factory, `VerificationRegistry` (real Groth16 verify, timelocked verifier swap), `ConsentKeyRegistry` (gasless meta-tx), `Groth16Verifier` (snarkjs-generated). Live on ROAX (chainId 135); addresses in `contracts/deployments/roax.json`.
- `stacks/vet` + `stacks/groomer` — same `vet-api` binary (`BUSINESS_TYPE` switch) + SPA + Mongo. `stacks/admin` — central registry/admin-api.
- `stacks/government` — **net-new, separately-deployable** role stack running its **own** `government-api` crate (NOT vet-api): a government credential authority that issues authority-endorsed `TRAVEL_CLEARANCE`/`EU_HEALTH_CERT` (anchors root via `DogTagIssuer.issue`) and does government-grade verify (integrity + `isValid` + `isWhitelistedFor`, all gasless reads). Own Mongo (`governmentdata`), ports 44831/44832, `make up-government`. `GOV_DEMO_MODE=1` → `MemChain`+`MemStore` (no node/gas/Mongo, used by `tests/flow_memchain.rs`); live mode → `AlloyChain` (+ `GOV_SIGNER_KEY` to anchor). It reuses the shared `dogtag-standard-rs` SDK for credential build/wrap but has its own trimmed `chain.rs`. Design: `docs/ROLE_APPS.md`.
- **Three-role showcase**: `scripts/demo-up.sh` boots all role stacks as separate services (admin/vet/groomer/government + portals). `scripts/e2e-roles.sh` (default = hermetic government ISSUE→VERIFY in `GOV_DEMO_MODE`, no deps; `--live` = vet ISSUES → government VERIFIES → government ISSUES across the running stacks over ROAX, needs `contracts/.env`). `government-api tests/cross_role.rs` codifies "vet ISSUES → government VERIFIES" deterministically over MemChain. See `docs/ROLE_APPS.md` §8.
- **Government per-record-type fields**: each credential type has its OWN field set — backend `credentialSubject` is built per type in `government/api/src/app.rs::build_gov_vc` (`TRAVEL_CLEARANCE` = origin/destination/purpose/clearanceRef; `EU_HEALTH_CERT` = species/microchip/rabies/examining-vet/health-status), and the web Issue form mirrors it via `RECORD_TYPE_FIELDS` in `government/web/src/App.tsx`. Keep the two in sync (a form field `key` must equal the `credentialSubject` leaf name). After a successful issue the portal shows the wrapped doc with a one-click **Copy** button to paste into Verify. The whitelist pillar is exercised because the Verify page pre-fills the signer from `/health`.
- **Government web e2e (Playwright)**: `stacks/government/web/e2e/government.spec.ts` (config `playwright.config.ts`) drives issue→copy→verify for both record types against a LIVE portal. It is NOT in `pnpm test`/CI (needs a running portal + browsers); run it against a served instance: `GOV_URL=<portal-url> pnpm --filter @dogtag/government-web test:e2e` (one-off `pnpm exec playwright install chromium`). A same-registry live serve reuses the deployed TRAVEL_CLEARANCE clone for BOTH `*_ISSUER_ADDR` and `GOV_SIGNER_KEY=$DEPLOYER_PRIVATE_KEY` (already whitelisted for both types).
- `stacks/owner/web` (`@dogtag/owner-web`, port **45931**) - the **pet-owner (holder) wallet**, the consumer front. Web mirror of the native `apps/android`+`apps/ios` holder: a self-custodial wallet that **receives** an issued wrapped doc (integrity-checked offline via `@dogtag/standard checkIntegrity`, held in localStorage), **displays** it (decoded leaves + `DogTagIssuer.isValid` read), and **presents** a ZK proof. It has **no backend** - it talks directly to two hosts given at runtime: the verifier's `…/x/<token>` session it scans and a **trusted prover-service** (`POST /prove-verification`, `VITE_OWNER_PROVER_URL`, default :41875). The "phone ZK" client crypto (build §1.10 consent + `signConsentEddsa` EdDSA-BabyJubjub + EIP-712 `BindConsentKey` sig via `viem`) runs **in the browser**; only the heavy Groth16 proof is delegated to the prover (the verifier never sees the witness). Present flow = `src/lib/present.ts`; wired into `scripts/demo-up.sh`.
  - **Sharp edge (browser Buffer)**: `@dogtag/standard`'s EdDSA path pulls in `circomlibjs`, which needs Node `Buffer`/`global` at runtime. The vite **build** tree-shakes past it but the **dev server crashes** ("Buffer is not defined") without a shim. `src/polyfills.ts` (imported first in `main.tsx`, `buffer` npm dep) provides them. Any new web app that signs consent client-side needs the same shim.
- **Owner web e2e (Playwright)**: `stacks/owner/web/e2e/owner.spec.ts` drives the whole holder loop (receive → hold/display → generate ZK proof → present → verified) + a tamper-rejection test. Like the government e2e it is NOT in `pnpm test`/CI. It starts its OWN vite dev server and **mocks the prover + verifier + ROAX RPC** at the network layer (deterministic), but runs the REAL client-side crypto. `pnpm --filter @dogtag/owner-web test:e2e`; `OWNER_URL=<url>` runs it against a live wallet instead (no self-server).

### Per-role records DB + CRUD (management layer)
Each role platform persists the records it issues into its OWN store (separate Mongo per running instance; `MemStore` for demo/tests), bundling the credential data with its **immutable on-chain proof**: tx hash, block number, contract (DogTagIssuer clone) address, and a ready-to-click explorer link `https://explorer.roax.net/tx/<hash>`.
- **vet-api** (serves vet + groomer via `BUSINESS_TYPE`, one DB per instance): `store::Record` gained `block_number`/`explorer_url`/`created_at`/`updated_at`/`label`/`notes`/`revoked_*`/`invalidated_at`/`invalidation_reason` + `RecordStatus::Expired`; `Store::list_records` (Mem + Mongo, most-recent first). Routes: `GET /records` (operator-gated list, surfaces explorer links), `PATCH /records/:id` (off-chain metadata only), plus the existing soft-invalidating `POST /records/:id/revoke`. `block_number` is captured in `confirm_inner` from `TxView.block_number`; the revoke path reads the revoke tx's block via `get_tx_view`.
- **government-api** (own DB): `store::IssuedCredential` gained the same proof + metadata fields + a `CredentialStatus` enum; routes `PATCH /v1/records/:root` and `POST /v1/records/:root/revoke` (adds `ChainClient::revoke` + `revoke_calldata`; `SentTx` now carries `block_number`).
  The two MUTATION routes are gated by `Authorization: Bearer <GOV_API_TOKEN>` (reads/list/health/verify/issue stay open): missing/wrong token → 401; in demo mode (`GOV_DEMO_MODE` et al) an unset `GOV_API_TOKEN` defaults to `dogtag-gov-demo-token` (the portal's `VITE_GOV_API_TOKEN` falls back to the same value); in non-demo mode with no token configured, mutations fail closed with 503.
- **Immutability**: `PATCH` accepts ONLY off-chain fields (`label`/`notes`, and `status` → `expired`); any on-chain-derived key in the body (tx hash, block, contract/issuer addr, root, wrapped doc, explorer url) is **rejected 400** ("… is on-chain-derived and immutable"). See the `IMMUTABLE_KEYS` list in each `routes.rs`.
- **Soft-invalidation, never hard delete**: revoke flips status to `revoked` on-chain (isValid → false) but keeps the row + its original issuance proof AND adds a revoke-tx proof; `expired` is an off-chain-only status transition (anchor untouched). Both stay listed + explorer-verifiable. There is NO delete endpoint by design. State machine: revoke accepts `issued` OR `expired` records (a compromised-but-expired credential can still be invalidated on-chain); `revoked` is terminal - expiring a revoked record is rejected 409 (an off-chain `expired` must never mask an on-chain revocation).
- **Web**: the vet + groomer portals share `stacks/{vet,groomer}/web/src/pages/Records.tsx` (identical) which now reads `api.listRecords()` from the backend DB (NOT the old localStorage `recordsStore`) and offers edit/expire/revoke via the shared `@dogtag/ui` client (`listRecords`/`updateRecord` in `packages/ui/src/api/client.ts`). The government portal has a `RecordsPage` in `App.tsx`.
- **Tests**: hermetic Rust integration tests (`stacks/{vet,government}/api/tests/records_crud.rs`, MemChain+MemStore) prove issue→persist-proof→list→patch(reject on-chain)→revoke(soft)→expire. Playwright: `government/web/e2e/records-crud.spec.ts` runs full-stack against a demo `GOV_DEMO_MODE` backend (real store + mem chain); `stacks/{vet,groomer}/web/e2e/records.spec.ts` drive the shared Records UI against a **mocked** backend (route regex `^https?://[^/]+/api/` — a `**/api/**` glob wrongly swallows `@dogtag/ui`'s `src/api/*.ts` module scripts and breaks the mount). None are in CI (need a served portal + browsers).

### Governance / admin (audit H-3)
- Governed contracts split admin two ways: `IssuerRegistry` (3-day), `VerificationRegistry` (2-day), and `DogTagSBT` (3-day) use OZ `AccessControlDefaultAdminRules` (two-step `begin`/`acceptDefaultAdminTransfer` + timelock); `DogTagIssuerFactory` uses `Ownable2Step`. `DogTagIssuer` clones have no own admin — they read `IssuerRegistry.hasRole(0x00)`. `ConsentKeyRegistry`/`Groth16Verifier`/`Poseidon6` have no admin.
- `DogTagSBT` inherits BOTH `AccessControlEnumerable` and `AccessControlDefaultAdminRules`, so it must explicitly override `grantRole`/`revokeRole`/`renounceRole`/`_setRoleAdmin` (`override(AccessControl, IAccessControl, AccessControlDefaultAdminRules)`) plus `_grantRole`/`_revokeRole`/`supportsInterface` — `super` resolves to the ACDAR rules first, then chains the enumerable bookkeeping. Do NOT `_grantRole(DEFAULT_ADMIN_ROLE,...)` in the constructor; the `AccessControlDefaultAdminRules(delay, admin)` base already does, and a second grant reverts (`AccessControlEnforcedDefaultAdminRules`).
- **The live ROAX admin is still the single deployer EOA** (`roax.json:admin` `0x119F8c7F…`), NOT a multisig. The EOA→multisig migration is shipped as code only (`contracts/script/GovernanceMigration.sol` library + `MigrateGovernance.s.sol` two-phase scripts + `GovernanceMigration.t.sol`), gated on a captain ceremony — see `docs/GOVERNANCE_MIGRATION.md`. The **live** `DogTagSBT` (`0x1FB8…`) predates the two-step upgrade and is still plain `AccessControlEnumerable`; it can't be retrofitted without a state-orphaning redeploy, so the migration hands it over with an atomic `grantRole`→`revokeRole` (the script's `supportsTwoStep` auto-picks the branch). Never execute the migration on live testnet without explicit captain approval.
- Removed dead governance surface: `IssuerRegistry.PROFILE_ISSUER_ROLE` and `DogTagSBT.UPDATER_ROLE` were declared but never enforced (SBT mint = `ISSUER_ROLE`; `setProfileRoot` = originator-or-`AUTHORITY_ROLE`). Don't re-add them.

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

## Captain's conventions & vocabulary

(Folded in from the firstmate-private canonical record so any agent in this repo shares the captain's conventions and vocabulary.)

### Working environment

- Each project is developed in a **dedicated WezTerm terminal tab**, supervised via **tmux**.
- A crewmate working on a repo runs in its own tmux window and **may spawn as many additional tmux
  windows as it needs** - builds, tests, logs, watchers, REPLs - so the work stays observable to the
  captain.
- Prefer giving long-running or noisy processes (servers, watchers, test loops, dev builds) **their own
  tmux window** rather than blocking the main one. Keep the work visible.

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
