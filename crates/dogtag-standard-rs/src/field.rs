//! Byte->field packing (impl §1.2, §11.2) — byte-identical to packages/dogtag-standard-ts/src/field.ts.
use ark_bn254::Fr;
use ark_ff::PrimeField;

use crate::poseidon::{from_be_limb, poseidon, to_be_bytes32, DS_BYTES};

/// 8-byte big-endian length prefix.
fn u64be(n: u64) -> [u8; 8] {
    n.to_be_bytes()
}

/// Inject a byte string into one field via the length-prefixed, 31-byte-chunked,
/// domain-separated Poseidon fold (impl §1.2). Used for keyPath and value.
pub fn bytes_to_field(x: &[u8]) -> Fr {
    let mut b = Vec::with_capacity(8 + x.len());
    b.extend_from_slice(&u64be(x.len() as u64));
    b.extend_from_slice(x);

    let mut acc = Fr::from(DS_BYTES);
    let mut off = 0usize;
    while off < b.len() {
        let mut limb = [0u8; 31]; // last limb right-zero-padded to 31
        let end = (off + 31).min(b.len());
        limb[..end - off].copy_from_slice(&b[off..end]);
        acc = poseidon(&[acc, from_be_limb(&limb)]);
        off += 31;
    }
    acc
}

/// Pack bytes that fit one field directly (<= 31 bytes), big-endian: salt(16B), addresses(uint160).
pub fn field_from_scalar_bytes(x: &[u8]) -> Fr {
    assert!(x.len() <= 31, "scalar bytes must be <= 31 (got {})", x.len());
    from_be_limb(x)
}

/// A small unsigned integer (typeTag, indices) as a field element.
pub fn field_from_uint(n: u64) -> Fr {
    Fr::from(n)
}

/// Canonical 32-byte big-endian hex (0x-prefixed) of a field element.
pub fn to_hex32(x: &Fr) -> String {
    format!("0x{}", hex::encode(to_be_bytes32(x)))
}

/// Integer order in [0, p): compare canonical 32-byte big-endian encodings.
pub fn field_le(a: &Fr, b: &Fr) -> bool {
    to_be_bytes32(a) <= to_be_bytes32(b)
}

/// BN254 scalar field modulus as a decimal string (for the modulus-confusion guard).
pub fn field_modulus_dec() -> String {
    use ark_ff::BigInteger;
    let m = Fr::MODULUS;
    // big-endian bytes -> decimal
    let be = m.to_bytes_be();
    crate::util::be_bytes_to_dec(&be)
}
