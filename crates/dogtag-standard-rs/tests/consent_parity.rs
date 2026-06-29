//! Consent cross-language parity (impl §9, §11.8/§11.9) — the Rust SDK asserts the SAME
//! packages/dogtag-standard-ts/consent-vectors.json the TS SDK generated. Any divergence in the
//! EIP-712 digest / nullifier / eddsa message / keyHash fails here, guaranteeing TS == Rust.

use ark_bn254::Fr;
use ark_ff::PrimeField;
use dogtag_standard::consent::{
    consent_nullifier, domain_separator, eddsa_consent_message, hash_typed_consent, key_hash,
    verification_consent_typehash, VerificationConsent, DOGTAG_CHAIN_ID,
    VERIFICATION_CONSENT_TYPE_STRING,
};
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

fn hex32(h: &str) -> String {
    let s = h.strip_prefix("0x").unwrap_or(h);
    format!("0x{s}")
}

fn bytes32_of(h: &str) -> [u8; 32] {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let v = hex::decode(s).unwrap();
    assert!(v.len() <= 32, "bytes32 too long: {h}");
    let mut out = [0u8; 32];
    out[32 - v.len()..].copy_from_slice(&v);
    out
}

fn addr_of(h: &str) -> [u8; 20] {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let v = hex::decode(s).unwrap();
    assert!(v.len() <= 20, "address too long: {h}");
    let mut out = [0u8; 20];
    out[20 - v.len()..].copy_from_slice(&v);
    out
}

/// Decimal uint256 string -> 32-byte big-endian word.
fn dec_to_word(s: &str) -> [u8; 32] {
    let le = dogtag_standard_dec_to_le(s);
    assert!(le.len() <= 32, "uint256 overflow: {s}");
    let mut out = [0u8; 32];
    for (i, b) in le.iter().enumerate() {
        out[31 - i] = *b;
    }
    out
}

// dec -> little-endian bytes (schoolbook /256), mirrors util::dec_to_le_bytes without exporting it.
fn dogtag_standard_dec_to_le(s: &str) -> Vec<u8> {
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

fn consent_of(c: &Value) -> VerificationConsent {
    VerificationConsent {
        dog_tag_id: dec_to_word(c["dogTagId"].as_str().unwrap()),
        record_type: bytes32_of(c["recordType"].as_str().unwrap()),
        purpose: bytes32_of(c["purpose"].as_str().unwrap()),
        credential_root: bytes32_of(c["credentialRoot"].as_str().unwrap()),
        challenge: bytes32_of(c["challenge"].as_str().unwrap()),
        relayer: addr_of(c["relayer"].as_str().unwrap()),
        subject: addr_of(c["subject"].as_str().unwrap()),
        nonce: dec_to_word(c["nonce"].as_str().unwrap()),
        deadline: dec_to_word(c["deadline"].as_str().unwrap()),
    }
}

fn fr_from_dec(s: &str) -> Fr {
    Fr::from_le_bytes_mod_order(&dogtag_standard_dec_to_le(s))
}

#[test]
fn typehash_and_type_string_match_ts() {
    let v = load();
    assert_eq!(
        VERIFICATION_CONSENT_TYPE_STRING,
        v["type_string"].as_str().unwrap(),
        "EIP-712 type string mismatch (field order!)"
    );
    let got = format!("0x{}", hex::encode(verification_consent_typehash()));
    assert_eq!(
        got,
        v["typehash"].as_str().unwrap(),
        "typehash mismatch (TS != Rust)"
    );
}

#[test]
fn domain_separator_matches_ts() {
    let v = load();
    let chain_id: u64 = v["chain_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(chain_id, DOGTAG_CHAIN_ID);
    let vc = addr_of(v["verifying_contract"].as_str().unwrap());
    let got = format!("0x{}", hex::encode(domain_separator(vc, chain_id)));
    assert_eq!(
        got,
        v["domain_separator"].as_str().unwrap(),
        "domainSeparator mismatch"
    );
}

#[test]
fn consent_vectors_parity() {
    let v = load();
    let chain_id: u64 = v["chain_id"].as_str().unwrap().parse().unwrap();
    let vc = addr_of(v["verifying_contract"].as_str().unwrap());

    for vec in v["vectors"].as_array().unwrap() {
        let name = vec["name"].as_str().unwrap();
        let consent = consent_of(&vec["consent"]);

        let digest = format!(
            "0x{}",
            hex::encode(hash_typed_consent(&consent, vc, chain_id))
        );
        assert_eq!(
            digest,
            hex32(vec["eip712_digest"].as_str().unwrap()),
            "EIP-712 digest {name}"
        );

        let null = format!("0x{}", hex::encode(consent_nullifier(&consent)));
        assert_eq!(
            null,
            hex32(vec["nullifier"].as_str().unwrap()),
            "nullifier {name}"
        );

        let msg = eddsa_consent_message(&consent);
        assert_eq!(
            msg,
            fr_from_dec(vec["eddsa_message_dec"].as_str().unwrap()),
            "eddsa message {name}"
        );
    }
}

#[test]
fn key_hash_parity() {
    let v = load();
    for kh in v["keyHash"].as_array().unwrap() {
        let name = kh["name"].as_str().unwrap();
        let ax = fr_from_dec(kh["Ax"].as_str().unwrap());
        let ay = fr_from_dec(kh["Ay"].as_str().unwrap());
        let got = format!("0x{}", hex::encode(key_hash(ax, ay)));
        assert_eq!(got, kh["expected"].as_str().unwrap(), "keyHash {name}");
    }
}

#[test]
fn nullifier_matches_poseidon_gate_anchor() {
    let v = load();
    let anchor = v["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"].as_str() == Some("anchor"))
        .expect("anchor vector");
    let consent = consent_of(&anchor["consent"]);
    let got = format!("0x{}", hex::encode(consent_nullifier(&consent)));
    assert_eq!(
        got, "0x055077ae7cbe2e123ad701247450fa222fabe3d3b399bfd40f416da970cfca11",
        "consent nullifier must equal poseidon-vectors.json nullifier_basic (sanity anchor)"
    );
    // sanity-cross-check: the file's recorded anchor nullifier agrees too
    assert_eq!(got, v["poseidon_gate_anchor"].as_str().unwrap());
}
