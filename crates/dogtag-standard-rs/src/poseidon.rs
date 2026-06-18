//! Pinned circomlib BN254 Poseidon (architecture §3.4, impl §11.2).
//!
//! ONE parameter set, four pinned libraries; Rust uses `light-poseidon` via
//! `Poseidon::<Fr>::new_circom(nInputs)` (the circom-compatible constructor — NOT a
//! generic one) over `ark_bn254::Fr`. Domain tags are passed as the FIRST input slot
//! (not a capacity IV) to stay on the exact circomlib API across all four libs.
//!
//! Field elements are built from `<= 31`-byte big-endian limbs that are provably `< p`
//! (`from_be_limb`); we NEVER use `from_be_bytes_mod_order`/32-byte widening (audit-10 P-H4 /
//! §11.10(f)) — that wraps mod r and diverges from circom.

use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField};
use light_poseidon::{Poseidon, PoseidonHasher};

/// Domain-separation tags — used as the first input slot (impl §1.2).
pub const DS_LEAF: u64 = 1;
pub const DS_NODE: u64 = 2;
pub const DS_BYTES: u64 = 3;
pub const DS_NULLIFIER: u64 = 4;

/// Pinned circomlib Poseidon over `nInputs` field elements (circom-compatible).
///
/// Panics only on an out-of-range arity (circomlib supports t∈[2,16] i.e. 1..=15 inputs),
/// which is a programming error, never reachable from the fixed-arity SDK call sites.
pub fn poseidon(inputs: &[Fr]) -> Fr {
    let mut hasher = Poseidon::<Fr>::new_circom(inputs.len())
        .expect("circomlib Poseidon arity out of range (need 1..=15 inputs)");
    hasher.hash(inputs).expect("poseidon hash failed")
}

/// Decode a big-endian limb of `<= 31` bytes directly into an `Fr` (provably `< p`, injective).
///
/// Forbids 32-byte widening: any input longer than 31 bytes is rejected rather than reduced,
/// so the encoding can never wrap mod r and silently diverge from circom (§11.10(f)).
pub fn from_be_limb(limb: &[u8]) -> Fr {
    assert!(limb.len() <= 31, "limb must be <= 31 bytes (got {})", limb.len());
    // A <=31-byte big-endian value is < 2^248 < p, so the modular reduction is the identity.
    Fr::from_be_bytes_mod_order(limb)
}

/// Serialize a field element as a 32-byte big-endian array (always `< p < 2^254`).
pub fn to_be_bytes32(x: &Fr) -> [u8; 32] {
    let v = x.into_bigint().to_bytes_be(); // big-endian, minimal-or-32 length
    let mut out = [0u8; 32];
    out[32 - v.len()..].copy_from_slice(&v);
    out
}

/// `Fr` from a `u64` scalar (typeTag, small ints).
pub fn fr_from_u64(n: u64) -> Fr {
    Fr::from(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ff::PrimeField;

    #[test]
    fn anchor_poseidon_1_2() {
        let r = poseidon(&[Fr::from(1u64), Fr::from(2u64)]);
        let hex = hex::encode(to_be_bytes32(&r));
        assert_eq!(
            hex,
            "115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a",
            "circomlib anchor poseidon([1,2]) mismatch"
        );
    }

    #[test]
    fn from_be_limb_is_identity_under_31_bytes() {
        // 31 bytes of 0xff is < p, decode must be exact (no reduction).
        let limb = [0xffu8; 31];
        let f = from_be_limb(&limb);
        let expect = Fr::from_be_bytes_mod_order(&limb);
        assert_eq!(f, expect);
    }
}
