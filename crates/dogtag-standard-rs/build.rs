//! Build script for `dogtag-standard-rs`.
//!
//! Historically (the `rust-witness` / wasm2c era) this transpiled the vendored
//! `circuit/verification.wasm` into native C so the witness calculator could be linked in. That
//! path MISCOMPILED the circuit's i64 BN254 field arithmetic on 32-bit ARM (armeabi-v7a), zeroing
//! the last-computed output wires (nullifier/keyHash).
//!
//! The prover now uses circom-prover's `circom-witnesscalc` GRAPH witness calculator — a pure-Rust
//! interpreter that loads a precompiled `verification.graph` asset at runtime (see
//! `circuits/build/verification.graph`, bundled into the app like the zkey). There is therefore no
//! build-time codegen to do, on any feature, so this script is intentionally a no-op.

fn main() {}
