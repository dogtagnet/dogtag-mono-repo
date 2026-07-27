//! Consent-key cross-language parity (impl §9) — the Rust SDK asserts the SAME
//! packages/dogtag-standard-ts/consent-vectors.json the TS SDK generated. Any divergence in
//! `keyHash = Poseidon(Ax, Ay)` — the value the `owner.consentKey` leaf commits into the profile
//! tree `R` — fails here, guaranteeing TS == Rust.

use ark_bn254::Fr;
use ark_ff::PrimeField;
use dogtag_standard::eddsa::key_hash;
use serde_json::Value;

const VECTORS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/dogtag-standard-ts/consent-vectors.json"
);

fn load() -> Value {
    let raw = std::fs::read_to_string(VECTORS).unwrap_or_else(|e| {
        panic!("read {VECTORS}: {e} — run `pnpm --filter @dogtag/standard gen-consent-vectors`")
    });
    serde_json::from_str(&raw).unwrap()
}

// dec -> little-endian bytes (schoolbook /256), mirrors util::dec_to_le_bytes without exporting it.
fn dec_to_le(s: &str) -> Vec<u8> {
    let mut digits: Vec<u8> = s.bytes().map(|b| b - b'0').collect();
    let mut out = Vec::new();
    while !(digits.len() == 1 && digits[0] == 0) {
        let mut rem = 0u32;
        let mut next = Vec::with_capacity(digits.len());
        for &d in &digits {
            let cur = rem * 10 + d as u32;
            next.push((cur / 256) as u8);
            rem = cur % 256;
        }
        let mut i = 0;
        while i + 1 < next.len() && next[i] == 0 {
            i += 1;
        }
        digits = next[i..].to_vec();
        out.push(rem as u8);
    }
    if out.is_empty() {
        out.push(0);
    }
    out
}

fn fr_from_dec(s: &str) -> Fr {
    Fr::from_le_bytes_mod_order(&dec_to_le(s))
}

#[test]
fn key_hash_parity() {
    let v = load();
    let vecs = v["keyHash"].as_array().unwrap();
    assert!(!vecs.is_empty(), "consent-vectors.json must carry keyHash vectors");
    for kh in vecs {
        let name = kh["name"].as_str().unwrap();
        let ax = fr_from_dec(kh["Ax"].as_str().unwrap());
        let ay = fr_from_dec(kh["Ay"].as_str().unwrap());
        let got = format!("0x{}", hex::encode(key_hash(ax, ay)));
        assert_eq!(got, kh["expected"].as_str().unwrap(), "keyHash {name}");
    }
}
