//! DogTag open pet-credential standard — Rust SDK.
//!
//! Byte-for-byte equivalent to `packages/dogtag-standard-ts`; both assert the shared
//! `testvectors.json` / `poseidon-vectors.json` in CI to guarantee cross-language determinism
//! (impl §9). The credential commitment is a single Poseidon root `R` over BN254
//! (architecture §3 / CHANGESPEC-v4).

pub mod poseidon;

pub use poseidon::{poseidon as poseidon_hash, DS_BYTES, DS_LEAF, DS_NODE, DS_NULLIFIER};
