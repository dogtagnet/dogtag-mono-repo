//! Gate A (Rust leg) — assert `light-poseidon` (`new_circom`) matches the shared
//! `circuits/poseidon-vectors.json` bit-for-bit at every arity (t=3,4,6,7). circomlib is
//! the reference-of-record; if Rust disagrees, this test fails the CI/lockfile gate (§11.10(b)).

use ark_bn254::Fr;
use ark_ff::PrimeField;
use dogtag_standard::poseidon::{poseidon, to_be_bytes32};
use serde_json::Value;

fn fr_from_dec(s: &str) -> Fr {
    // vectors are decimal field elements already < r; decimal parse is exact.
    Fr::from_str_dec(s)
}

trait FromDec {
    fn from_str_dec(s: &str) -> Self;
}
impl FromDec for Fr {
    fn from_str_dec(s: &str) -> Self {
        use std::str::FromStr;
        let bi = num_from_dec(s);
        let _ = Fr::from_str; // keep import-free
        Fr::from_le_bytes_mod_order(&bi)
    }
}

// minimal decimal -> little-endian bytes (no extra deps): repeated /256 via big-decimal string math
fn num_from_dec(s: &str) -> Vec<u8> {
    // parse decimal string into base-256 little-endian using u128 chunking is fragile for >38 digits,
    // so do schoolbook division of the decimal string by 256.
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
        // strip leading zeros
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

#[test]
fn poseidon_vectors_parity() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../circuits/poseidon-vectors.json"
    );
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {path}: {e} — run `make parity` (circuits) first"));
    let v: Value = serde_json::from_str(&raw).unwrap();

    // field_r must match ark_bn254::Fr modulus (modulus confusion = silent divergence, §11.10(c)).
    let field_r = v["field_r"].as_str().unwrap();
    let r_bytes = num_from_dec(field_r); // little-endian
    let fr_mod = Fr::MODULUS;
    let mut mod_le = ark_ff::BigInteger::to_bytes_le(&fr_mod);
    while mod_le.last() == Some(&0) && mod_le.len() > r_bytes.len() {
        mod_le.pop();
    }
    let mut rb = r_bytes.clone();
    while rb.len() < mod_le.len() {
        rb.push(0);
    }
    assert_eq!(
        rb, mod_le,
        "BN254 scalar field r mismatch (modulus confusion)"
    );

    let anchor_dec = v["anchor"]["dec"].as_str().unwrap();
    let mut anchor_checked = false;

    for vec in v["vectors"].as_array().unwrap() {
        let name = vec["name"].as_str().unwrap();
        let inputs: Vec<Fr> = vec["in"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| fr_from_dec(x.as_str().unwrap()))
            .collect();
        let got = poseidon(&inputs);
        let got_hex = format!("0x{}", hex::encode(to_be_bytes32(&got)));
        let want_hex = vec["out_hex"].as_str().unwrap();
        assert_eq!(got_hex, want_hex, "Rust poseidon mismatch @ {name}");

        if name == "anchor_1_2" {
            anchor_checked = true;
            assert_eq!(fr_from_dec(anchor_dec), got, "anchor dec mismatch");
            assert_eq!(
                got_hex,
                "0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a"
            );
        }
    }
    assert!(
        anchor_checked,
        "anchor vector missing from poseidon-vectors.json"
    );
    eprintln!("Gate A (Rust leg) GREEN — light-poseidon new_circom == circom across all arities.");
}
