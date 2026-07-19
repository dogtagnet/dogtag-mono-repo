//! The one per-tag KDF preimage builder for every seed-derived owner-control secret.
//!
//! Every piece of the owner-control core - the owner secret, the reserved-leaf salts, and the
//! BabyJubjub consent key - is derived from `(seed, dogTagId)` through THIS function, so the
//! binding is uniform and there is a single place to audit. Keeping one builder is the point: a
//! second, hand-rolled `domain || dogTagId || seed` elsewhere is exactly the drift that let the
//! consent key stay seed-only while its siblings were already per-tag.
//!
//! Domains are versioned (`.../vN`) and frozen: bumping a version re-derives everything downstream
//! of it, and `profileRoot` is write-once on-chain, so a bump after mint needs a migration.

use ark_bn254::Fr;

use crate::blake512::blake512;
use crate::poseidon::to_be_bytes32;

/// KDF preimage: `domain || dogTagId (32B BE) || len(extra) (8B BE) || extra || seed`.
///
/// The variable-length `seed` goes last and `extra` carries an explicit length prefix, so no two
/// distinct `(extra, seed)` pairs can share a preimage.
pub(crate) fn kdf(domain: &[u8], dog_tag_id: &Fr, extra: &[u8], seed: &[u8]) -> [u8; 64] {
    let mut buf = Vec::with_capacity(domain.len() + 32 + 8 + extra.len() + seed.len());
    buf.extend_from_slice(domain);
    buf.extend_from_slice(&to_be_bytes32(dog_tag_id));
    buf.extend_from_slice(&(extra.len() as u64).to_be_bytes());
    buf.extend_from_slice(extra);
    buf.extend_from_slice(seed);
    blake512(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &[u8] = b"kdf test wallet seed - TEST MATERIAL ONLY, never hold value";

    /// The length prefix on `extra` is what makes the preimage unambiguous: without it,
    /// `(extra="ab", seed="c")` and `(extra="a", seed="bc")` would collide.
    #[test]
    fn the_extra_length_prefix_prevents_a_split_collision() {
        let id = Fr::from(7u64);
        let a = kdf(b"D", &id, b"ab", b"c");
        let b = kdf(b"D", &id, b"a", b"bc");
        assert_ne!(a, b, "extra/seed split must not be ambiguous");
    }

    #[test]
    fn the_dog_tag_id_changes_the_preimage() {
        let a = kdf(b"D", &Fr::from(1u64), b"", SEED);
        let b = kdf(b"D", &Fr::from(2u64), b"", SEED);
        assert_ne!(a, b, "dogTagId must be bound into every derivation");
    }

    #[test]
    fn the_domain_separates() {
        let id = Fr::from(7u64);
        assert_ne!(kdf(b"D1", &id, b"", SEED), kdf(b"D2", &id, b"", SEED));
    }
}
