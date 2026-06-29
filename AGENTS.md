# Project agent memory

This file is the project's committed home for project-intrinsic agent knowledge: build, test, release, architecture, and sharp-edge notes that should travel with the code.

## Build & test (what actually runs offline)

Toolchain: Rust (cargo workspace), Foundry (`forge`/`cast`), Node 22 + pnpm 10, circom 2.1.9 + snarkjs 0.7.6, Docker.

- `cargo check --workspace` / `cargo build` — Rust workspace: `dogtag-standard-rs`, `dogtag-prover-rs`, `vet-api`, `admin-api`.
- `cargo test -p dogtag-standard-rs` — trust-core crypto + cross-language parity vectors.
- `cargo test -p vet-api -p admin-api` — backends. (One vet-api suite, `gate_dual_signing_parity`, is slow — ~5 min — it runs the real prover/signing; this is expected, not a hang.)
- `cd contracts && forge build && forge test` — 44 tests incl. `ZkIntegration.t.sol` (real Groth16 proof verified on-chain) and `Verification.t.sol`.
- `cd circuits && node scripts/test-circuit.mjs` — generates REAL Groth16 proofs (leaf counts 1..24) + negative tests. Needs the TS SDK built first (`pnpm --filter @dogtag/standard build`) and `pnpm install`. Slow (large r1cs witness gen).
- `make parity` — the Poseidon anchor gate; `make test` — parity + TS + Rust + contracts.

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

### dogTagId encoding (easy to get wrong)
The operator-facing **handle** is a small integer. The **on-chain** dogTagId minted into `DogTagSBT` and emitted as the circuit's `pub[0]` is the Poseidon **field-hash** of that handle: `routes::onchain_dog_tag_id(handle)` = `to_hex32(field_of_value(Integer(handle)))` (mirrors the `dog_tag_id_field_hex` FFI / `field-hash` bin). The SBT is keyed by the field element, NOT the raw handle — `ownerOf`/`profileRoot` lookups (and tests) must field-hash first.

## Deployment / production guards (fail-closed)
- Demo vs prod is gated by `DEMO_MODE` / `VITE_DEMO_MODE` (set = demo/local, unset = production).
- Both backends call `startup::validate_production_secrets(...)` at boot: in production they **refuse to start** if `OPERATOR_PASSWORD`/`ADMIN_PASSWORD`/`CENTRAL_HMAC_SECRET` (vet) or `ADMIN_PASSWORD`/`ADMIN_PRIVATE_KEY` (admin) are unset or equal to the known dev defaults. Set `DEMO_MODE=1` to keep the convenient demo defaults.
- vet-api: if `CIRCUITS_BUILD_DIR` is set but the real `ArkProver` fails to load, the process **exits** (it must not silently degrade to `StubProver`, which emits zeroed proofs the chain rejects). Unset `CIRCUITS_BUILD_DIR` still uses `StubProver` (demo / on-device-proof production model).
- The prover **enforces a pinned zkey sha256** (`dogtag-prover-rs::EXPECTED_ZKEY_SHA256_HEX`, the testnet ceremony hash): `Prover::load` rejects any zkey whose hash differs, so a swapped/corrupt key fails closed instead of proving against the wrong key (audit M4). A deployment shipping a **different** zkey (a production ceremony output) sets the `EXPECTED_ZKEY_SHA256` env var on vet-api (→ `load_with_expected_zkey`) — a config swap, not a code change. Leave it unset to enforce the bundled testnet hash.

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
