//! Phase 6 acceptance test (the MUST-PASS): drive the UniFFI foreign-callable surface over the
//! shared `packages/dogtag-standard-ts/testvectors.json` and assert that every leaf hex and every
//! merkle root produced through the FFI wrappers is byte-identical to the server/TS reference.
//!
//! This proves "mobile root == server root" at the Rust FFI level (the exact functions Kotlin/Swift
//! call through the generated bindings). The JVM-level repeat lives in apps/android.

use dogtag_standard::ffi::{
    build_merkle_root_hex, bytes_to_field_hex, hash_leaf_hex, verify_integrity, wrap_document_json,
};
use serde_json::Value;

fn vectors() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packages/dogtag-standard-ts/testvectors.json"
    );
    let raw = std::fs::read_to_string(path).expect("read testvectors.json");
    serde_json::from_str(&raw).expect("parse testvectors.json")
}

#[test]
fn ffi_leaf_vectors_parity() {
    let v = vectors();
    for leaf in v["leaves"].as_array().unwrap() {
        let key_path = leaf["keyPath"].as_str().unwrap().to_string();
        let salt_hex = leaf["saltHex"].as_str().unwrap().to_string();
        let tag = leaf["tag"].as_u64().unwrap() as u8;
        // The vectors store `value` as the canonical string (null -> JSON null -> "").
        let value = match &leaf["value"] {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let expected = leaf["expected_hex"].as_str().unwrap();
        let got = hash_leaf_hex(key_path, salt_hex, tag, value).expect("hash_leaf_hex");
        assert_eq!(
            got, expected,
            "FFI leaf hash mismatch for vector {}",
            leaf["name"]
        );
    }
}

#[test]
fn ffi_bytes_to_field_parity() {
    let v = vectors();
    for b in v["bytesToField"].as_array().unwrap() {
        let input_hex = b["inputHex"].as_str().unwrap().to_string();
        let expected = b["expected_hex"].as_str().unwrap();
        let got = bytes_to_field_hex(input_hex).expect("bytes_to_field_hex");
        assert_eq!(got, expected, "FFI bytesToField mismatch for {}", b["name"]);
    }
}

#[test]
fn ffi_merkle_root_parity() {
    let v = vectors();
    for m in v["merkle"].as_array().unwrap() {
        let leaf_hexes: Vec<String> = m["leaf_hexes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h.as_str().unwrap().to_string())
            .collect();
        let expected = m["root_hex"].as_str().unwrap();
        let got = build_merkle_root_hex(leaf_hexes).expect("build_merkle_root_hex");
        assert_eq!(
            got, expected,
            "FFI merkle root mismatch for {} (mobile root != server root)",
            m["name"]
        );
    }
}

/// End-to-end through the FFI: wrap a credential, then verify its integrity, both via the
/// foreign-callable surface only (no internal types).
#[test]
fn ffi_wrap_then_verify_integrity_valid() {
    let credential = r#"{
        "credentialSubject": {
            "dogTagId": {"tag": 3, "value": "42"},
            "name": {"tag": 2, "value": "Rex"}
        }
    }"#;
    let issuer = r#"{
        "name": "Acme Vet",
        "domain": "acme.example",
        "documentStore": "0x0000000000000000000000000000000000000001",
        "recordType": "VACCINATION"
    }"#;
    let wrapped = wrap_document_json(credential.to_string(), issuer.to_string()).expect("wrap");
    let verdict = verify_integrity(wrapped).expect("verify");
    assert_eq!(verdict, "VALID");
}

#[test]
fn ffi_verify_integrity_rejects_tamper() {
    let credential = r#"{
        "credentialSubject": {
            "dogTagId": {"tag": 3, "value": "42"},
            "name": {"tag": 2, "value": "Rex"}
        }
    }"#;
    let issuer = r#"{
        "name": "Acme Vet",
        "domain": "acme.example",
        "documentStore": "0x0000000000000000000000000000000000000001",
        "recordType": "VACCINATION"
    }"#;
    let wrapped = wrap_document_json(credential.to_string(), issuer.to_string()).expect("wrap");
    // Tamper: replace the dogTagId cleartext value (keep salt:tag) -> integrity must be INVALID.
    let mut doc: Value = serde_json::from_str(&wrapped).unwrap();
    let subj = doc["data"]["credentialSubject"].as_object_mut().unwrap();
    let packed = subj["name"].as_str().unwrap();
    let parts: Vec<&str> = packed.splitn(3, ':').collect();
    let tampered = format!("{}:{}:Max", parts[0], parts[1]);
    subj.insert("name".to_string(), Value::String(tampered));
    let verdict = verify_integrity(serde_json::to_string(&doc).unwrap()).expect("verify");
    assert_eq!(verdict, "INVALID");
}
