//! DogTag open pet-credential standard — Rust SDK.
//!
//! Byte-for-byte equivalent to `packages/dogtag-standard-ts`; both assert the shared
//! `testvectors.json` / `poseidon-vectors.json` in CI to guarantee cross-language determinism
//! (impl §9). The credential commitment is a single Poseidon root `R` over BN254
//! (architecture §3 / CHANGESPEC-v4).

pub mod encode;
pub mod field;
pub mod leaf;
pub mod merkle;
pub mod poseidon;
pub mod types;
pub mod util;
pub mod flatten;
pub mod wrap;
pub mod verify;
pub mod schema;

pub use field::{bytes_to_field, to_hex32};
pub use leaf::hash_leaf;
pub use merkle::{build_merkle, merkle_proof, process_proof};
pub use poseidon::{poseidon as poseidon_hash, DS_BYTES, DS_LEAF, DS_NODE, DS_NULLIFIER};
pub use types::{DogTagError, TypeTag, TypedScalar};

pub mod consent;

// EdDSA-BabyJubjub consent SIGNING (Phase 6 — mobile crypto). Additive: a self-contained
// circomlibjs-compatible BLAKE-512 + BabyJubjub Edwards curve + signer over the existing Poseidon.
// Does NOT modify poseidon/field/leaf/merkle/encode.
pub mod blake512;
pub mod eddsa;

// Phase 6 — mobile UniFFI binding surface (additive; does not touch the core algorithm modules).
pub mod ffi;

// Workstream A — circuit-input ASSEMBLY (prover-independent). Gated behind the lightweight
// `assemble` feature: it pulls NO circom-prover (ark-0.5) deps, only the SDK's own field/merkle, so
// the 64-bit backend (vet-api, on ark-0.6 dogtag-prover-rs) can reuse the SAME 19-input assembly to
// drive the server proving API. Only decimal strings cross the boundary — no ark-version clash. The
// full on-device `prover` feature implies `assemble` (the on-device prover reuses this assembly).
#[cfg(feature = "assemble")]
pub mod prover_assemble;

// Workstream A — on-device Groth16 prover (mopro/circom-prover + circom-witnesscalc graph witness).
// Gated behind the OFF-by-default `prover` feature so default workspace builds never pull the heavy
// ark-0.5 deps. It layers the circom-prover proving on top of `prover_assemble`'s assembly.
#[cfg(feature = "prover")]
pub mod prover_ffi;

uniffi::setup_scaffolding!();

