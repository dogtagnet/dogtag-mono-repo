//! Phase 1 cross-language parity (impl §9) — the Rust SDK asserts the SAME
//! packages/dogtag-standard-ts/testvectors.json the TS SDK generated. Any divergence in
//! encodeValue / fieldOf / hashLeaf / buildMerkle fails here, guaranteeing TS == Rust.

use ark_bn254::Fr;
use ark_ff::PrimeField;
use dogtag_standard::field::{bytes_to_field, field_modulus_dec, to_hex32};
use dogtag_standard::leaf::hash_leaf;
use dogtag_standard::merkle::build_merkle;
use dogtag_standard::types::{TypeTag, TypedScalar};
use serde_json::Value;

const VECTORS: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/dogtag-standard-ts/testvectors.json");

fn load() -> Value {
    let raw = std::fs::read_to_string(VECTORS)
        .unwrap_or_else(|e| panic!("read {VECTORS}: {e} — run `pnpm --filter @dogtag/standard gen-vectors`"));
    serde_json::from_str(&raw).unwrap()
}

fn scalar_of(tag: u8, value: &Value) -> TypedScalar {
    match TypeTag::from_u8(tag).expect("bad tag") {
        TypeTag::Null => TypedScalar::Null,
        TypeTag::Bool => TypedScalar::Bool(value.as_str() == Some("true")),
        TypeTag::String => TypedScalar::Str(value.as_str().unwrap().to_string()),
        TypeTag::Integer => TypedScalar::Integer(value.as_str().unwrap().to_string()),
        TypeTag::Decimal => TypedScalar::Decimal(value.as_str().unwrap().to_string()),
        TypeTag::Bytes => TypedScalar::Bytes(hex::decode(value.as_str().unwrap()).unwrap()),
    }
}

fn fr_from_hex32(h: &str) -> Fr {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let bytes = hex::decode(s).unwrap();
    Fr::from_be_bytes_mod_order(&bytes)
}

#[test]
fn field_modulus_matches_ts() {
    let v = load();
    assert_eq!(
        field_modulus_dec(),
        v["field_p"].as_str().unwrap(),
        "BN254 scalar field mismatch (modulus confusion — §11.10(c))"
    );
}

#[test]
fn leaf_vectors_parity() {
    let v = load();
    let mut saw_string_five = None;
    let mut saw_integer_five = None;
    for leaf in v["leaves"].as_array().unwrap() {
        let name = leaf["name"].as_str().unwrap();
        let keypath = leaf["keyPath"].as_str().unwrap();
        let salt = hex::decode(leaf["saltHex"].as_str().unwrap()).unwrap();
        let tag = leaf["tag"].as_u64().unwrap() as u8;
        let scalar = scalar_of(tag, &leaf["value"]);
        let got = to_hex32(&hash_leaf(keypath, &salt, &scalar).unwrap());
        let want = leaf["expected_hex"].as_str().unwrap();
        assert_eq!(got, want, "leaf {name} mismatch (TS != Rust)");
        if name == "string_five" {
            saw_string_five = Some(got.clone());
        }
        if name == "integer_five" {
            saw_integer_five = Some(got.clone());
        }
    }
    assert_ne!(saw_string_five, saw_integer_five, "tag 2 \"5\" must differ from tag 3 5");
}

#[test]
fn bytes_to_field_parity() {
    let v = load();
    for b in v["bytesToField"].as_array().unwrap() {
        let name = b["name"].as_str().unwrap();
        let input = hex::decode(b["inputHex"].as_str().unwrap()).unwrap();
        let got = to_hex32(&bytes_to_field(&input));
        assert_eq!(got, b["expected_hex"].as_str().unwrap(), "bytesToField {name} mismatch");
    }
}

#[test]
fn merkle_vectors_parity() {
    let v = load();
    for m in v["merkle"].as_array().unwrap() {
        let name = m["name"].as_str().unwrap();
        let leaves: Vec<Fr> = m["leaf_hexes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| fr_from_hex32(h.as_str().unwrap()))
            .collect();
        let root = to_hex32(&build_merkle(&leaves).root);
        assert_eq!(root, m["root_hex"].as_str().unwrap(), "merkle {name} root mismatch");
        if let Some(rev) = m.get("reversed_root_hex").and_then(|x| x.as_str()) {
            let mut r = leaves.clone();
            r.reverse();
            assert_eq!(to_hex32(&build_merkle(&r).root), rev, "merkle {name} commutativity");
        }
    }
}
