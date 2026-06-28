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
