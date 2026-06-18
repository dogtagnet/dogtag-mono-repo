//! Original BLAKE-512 (the SHA-3 finalist, NOT BLAKE2) — circomlibjs-compatible.
//!
//! circomlibjs EdDSA-BabyJubjub derives both the private scalar and the per-message nonce via
//! `createBlakeHash("blake512")` (the npm `blake-hash` package, which is the *original* BLAKE-512).
//! The pure-Rust `blake-hash` crate is unusable on aarch64 (its ppv-lite86 SIMD backend hits
//! `unimplemented!()`), so we ship a small, self-contained BLAKE-512 here. This is ONLY used by the
//! new EdDSA signing path; it does NOT touch the Poseidon/field/leaf/merkle/encode core.
//!
//! Reference: Aumasson, Henzen, Meier, Phan — "SHA-3 proposal BLAKE" (v1.4). 64-bit words,
//! 16 rounds, big-endian I/O. Verified byte-for-byte against npm `blake-hash` test vectors.

/// BLAKE-512 initialization vector (== SHA-512 IV).
const IV: [u64; 8] = [
    0x6a09e667f3bcc908,
    0xbb67ae8584caa73b,
    0x3c6ef372fe94f82b,
    0xa54ff53a5f1d36f1,
    0x510e527fade682d1,
    0x9b05688c2b3e6c1f,
    0x1f83d9abfb41bd6b,
    0x5be0cd19137e2179,
];

/// 16 constants c_0..c_15 (digits of pi), 64-bit.
const C: [u64; 16] = [
    0x243f6a8885a308d3,
    0x13198a2e03707344,
    0xa4093822299f31d0,
    0x082efa98ec4e6c89,
    0x452821e638d01377,
    0xbe5466cf34e90c6c,
    0xc0ac29b7c97c50dd,
    0x3f84d5b5b5470917,
    0x9216d5d98979fb1b,
    0xd1310ba698dfb5ac,
    0x2ffd72dbd01adfb7,
    0xb8e1afed6a267e96,
    0xba7c9045f12c7f99,
    0x24a19947b3916cf7,
    0x0801f2e2858efc16,
    0x636920d871574e69,
];

/// The 10 permutations sigma (each a permutation of 0..16). BLAKE-512 uses 16 rounds, so the
/// permutations cycle (sigma[round % 10]).
const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

#[inline(always)]
fn g(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64) {
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(32);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(25);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(11);
}

/// Compress one 128-byte block. `t` is the bit counter (count of message bits hashed *including*
/// this block). `h` is the 8-word chain value (updated in place).
fn compress(h: &mut [u64; 8], block: &[u8; 128], t: u128) {
    // message words m_0..m_15 (big-endian).
    let mut m = [0u64; 16];
    for i in 0..16 {
        let mut w = 0u64;
        for j in 0..8 {
            w = (w << 8) | block[i * 8 + j] as u64;
        }
        m[i] = w;
    }

    let t0 = (t & 0xffff_ffff_ffff_ffff) as u64;
    let t1 = (t >> 64) as u64;

    let mut v = [0u64; 16];
    v[..8].copy_from_slice(h);
    v[8] = C[0];
    v[9] = C[1];
    v[10] = C[2];
    v[11] = C[3];
    v[12] = C[4] ^ t0;
    v[13] = C[5] ^ t0;
    v[14] = C[6] ^ t1;
    v[15] = C[7] ^ t1;

    for r in 0..16 {
        let s = &SIGMA[r % 10];
        g(&mut v, 0, 4, 8, 12, m[s[0]] ^ C[s[1]], m[s[1]] ^ C[s[0]]);
        g(&mut v, 1, 5, 9, 13, m[s[2]] ^ C[s[3]], m[s[3]] ^ C[s[2]]);
        g(&mut v, 2, 6, 10, 14, m[s[4]] ^ C[s[5]], m[s[5]] ^ C[s[4]]);
        g(&mut v, 3, 7, 11, 15, m[s[6]] ^ C[s[7]], m[s[7]] ^ C[s[6]]);
        g(&mut v, 0, 5, 10, 15, m[s[8]] ^ C[s[9]], m[s[9]] ^ C[s[8]]);
        g(&mut v, 1, 6, 11, 12, m[s[10]] ^ C[s[11]], m[s[11]] ^ C[s[10]]);
        g(&mut v, 2, 7, 8, 13, m[s[12]] ^ C[s[13]], m[s[13]] ^ C[s[12]]);
        g(&mut v, 3, 4, 9, 14, m[s[14]] ^ C[s[15]], m[s[15]] ^ C[s[14]]);
    }

    for i in 0..8 {
        h[i] ^= v[i] ^ v[i + 8];
    }
}

/// BLAKE-512 of `data` with salt = 0 (the circomlibjs / npm `blake-hash` default). Returns 64 bytes.
pub fn blake512(data: &[u8]) -> [u8; 64] {
    let mut h = IV;

    let bitlen = (data.len() as u128) * 8;
    let mut counter: u128 = 0; // bits hashed so far

    // Process all full 128-byte blocks except a trailing partial/final region handled below.
    let mut offset = 0usize;
    while data.len() - offset >= 128 {
        // If this is the LAST full block AND there is no remainder, it is still a full data block;
        // padding goes in a new block. So only feed it as a normal block here.
        let mut block = [0u8; 128];
        block.copy_from_slice(&data[offset..offset + 128]);
        counter += 1024;
        compress(&mut h, &block, counter);
        offset += 128;
    }

    // Remaining bytes (0..=127) plus padding. BLAKE padding: 0x80, then zeros, then 0x01 before
    // the 16-byte big-endian bit length; the final block carries the total bit length in `t`.
    let rem = &data[offset..];
    let remlen = rem.len();

    // One or two final blocks depending on whether the remainder + 1 (0x80) + 16 (len) fits in 128.
    // The byte just before the length gets its low bit set (|= 0x01) for BLAKE-512.
    if remlen <= 111 {
        let mut block = [0u8; 128];
        block[..remlen].copy_from_slice(rem);
        block[remlen] = 0x80;
        block[111] |= 0x01;
        // 16-byte big-endian bit length in bytes 112..128.
        block[112..128].copy_from_slice(&bitlen.to_be_bytes());
        // Counter for the final block = total bit length (BLAKE sets t to the number of message
        // bits up to and including this block; for a padding-only-tail block where the remainder
        // is the last data, that is `bitlen`).
        let t = if remlen == 0 { 0u128 } else { bitlen };
        compress(&mut h, &block, t);
    } else {
        // First final block: remainder + 0x80 + zeros; t = full bit length (data bits in this block).
        let mut b1 = [0u8; 128];
        b1[..remlen].copy_from_slice(rem);
        b1[remlen] = 0x80;
        // counter for this block: total bits (all message bits are within or before this block).
        compress(&mut h, &b1, bitlen);
        // Second final block: zeros, 0x01 before length, then 16-byte length; t = 0 (no new bits).
        let mut b2 = [0u8; 128];
        b2[111] |= 0x01;
        b2[112..128].copy_from_slice(&bitlen.to_be_bytes());
        compress(&mut h, &b2, 0);
    }

    let mut out = [0u8; 64];
    for i in 0..8 {
        out[i * 8..i * 8 + 8].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hx(d: &[u8]) -> String {
        hex::encode(blake512(d))
    }

    #[test]
    fn blake512_empty_matches_npm_blake_hash() {
        assert_eq!(
            hx(b""),
            "a8cfbbd73726062df0c6864dda65defe58ef0cc52a5625090fa17601e1eecd1b\
628e94f396ae402a00acc9eab77b4d4c2e852aaaa25a636d80af3fc7913ef5b8"
        );
    }

    #[test]
    fn blake512_abc_matches_npm_blake_hash() {
        assert_eq!(
            hx(b"abc"),
            "14266c7c704a3b58fb421ee69fd005fcc6eeff742136be67435df995b7c986e7\
cbde4dbde135e7689c354d2bc5b8d260536c554b4f84c118e61efc576fed7cd3"
        );
    }

    #[test]
    fn blake512_32x07_matches_npm_blake_hash() {
        assert_eq!(
            hx(&[7u8; 32]),
            "b6314cf29d6e1813a61b1da73b1dea328cb4009624774cf0a6fdfa424e1c7bab\
529601d99c8258dc3110405f21ce26f8fc5d1df4a7be0f05df7204db62b4f101"
        );
    }
}
