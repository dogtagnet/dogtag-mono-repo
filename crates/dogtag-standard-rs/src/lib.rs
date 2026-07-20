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
pub use merkle::{build_merkle, merkle_proof, process_proof, verify_inclusion, ProofStep};
pub use poseidon::{poseidon as poseidon_hash, DS_BYTES, DS_LEAF, DS_NODE, DS_NULLIFIER};
pub use types::{DogTagError, TypeTag, TypedScalar};

pub mod consent;

// EdDSA-BabyJubjub consent SIGNING (Phase 6 — mobile crypto). Additive: a self-contained
// circomlibjs-compatible BLAKE-512 + BabyJubjub Edwards curve + signer over the existing Poseidon.
// Does NOT modify poseidon/field/leaf/merkle/encode.
pub mod blake512;
// The ONE per-tag KDF preimage builder shared by every seed-derived owner-control secret (owner
// secret, reserved-leaf salts, per-tag consent key). Internal: a second builder is the drift that
// once let the consent key stay seed-only while its siblings were already per-tag.
mod kdf;
pub mod eddsa;

// Level-B M5 - the DEVICE-side per-tag profile tree (3 reserved owner-control leaves + attribute
// leaves -> R) and the recoverable, seed-derived owner-secret. Additive: builds on
// field/leaf/merkle/eddsa without modifying them.
pub mod profile_tree;

// Phase 6 — mobile UniFFI binding surface (additive; does not touch the core algorithm modules).
pub mod ffi;

// M7 P4 — discovery anchor-validation: the pure client TRUST gate that checks a platform's convenience
// tier against the dogtag-owned ProtocolRegistry / signed-manifest anchor before proving (§5.3). Lives
// here (not the server-only prover crate) because the mobile app links this crate; additive, no ark.
pub mod discovery;

// Workstream A — circuit-input ASSEMBLY (prover-independent). Gated behind the lightweight
// `assemble` feature: it pulls NO circom-prover (ark-0.5) deps, only the SDK's own field/merkle, so
// the 64-bit backend (vet-api, on ark-0.6 dogtag-prover-rs) can reuse the SAME 19-input assembly to
// drive the server proving API. Only decimal strings cross the boundary — no ark-version clash. The
// full on-device `prover` feature implies `assemble` (the on-device prover reuses this assembly).
#[cfg(feature = "assemble")]
pub mod prover_assemble;

// Workstream A — Level-B CONSENT circuit-input assembly (M7 P0). Same lightweight `assemble` gating
// and ark-disjoint discipline as `prover_assemble`, but for the frozen `consent.circom` seven-signal
// public layout; built from the circuit, NOT the stale Level-A `consent.rs` (ZK cross-check §2).
#[cfg(feature = "assemble")]
pub mod consent_assemble;

// Workstream A — on-device Groth16 prover (mopro/circom-prover + circom-witnesscalc graph witness).
// Gated behind the OFF-by-default `prover` feature so default workspace builds never pull the heavy
// ark-0.5 deps. It layers the circom-prover proving on top of `prover_assemble`'s assembly.
#[cfg(feature = "prover")]
pub mod prover_ffi;

uniffi::setup_scaffolding!();

