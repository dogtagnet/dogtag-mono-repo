//! Unit coverage for the pure packed-value codecs in the `wrap` module (impl §1.4, §11.2 F2b).
//!
//! `from_hex32`, `parse_packed`, `scalar_from_packed`, `leaf_from_packed`, and `flatten_data` are
//! pure, deterministic functions whose observable contract - the `salt:tag:value` packing grammar,
//! the per-`TypeTag` scalar reconstruction, the field-element hex guard, and the data-tree walk -
//! is load-bearing for verify/obfuscate but was previously exercised only indirectly through
//! `wrap_document`. These tests pin each branch and error path directly. Behavior-preserving:
//! they assert existing behavior only.

use dogtag_standard::types::{DogTagError, TypeTag, TypedScalar};
use dogtag_standard::wrap::{
    flatten_data, from_hex32, leaf_from_packed, parse_packed, scalar_from_packed,
};
use serde_json::json;

fn msg(e: DogTagError) -> String {
    e.to_string()
}

// ---- from_hex32 ----------------------------------------------------------------------------

#[test]
fn from_hex32_accepts_canonical_value_with_and_without_prefix() {
    let one = "0".repeat(62) + "01"; // 32 bytes BE == field element 1
    let with = from_hex32(&format!("0x{one}")).unwrap();
    let without = from_hex32(&one).unwrap();
    assert_eq!(with, without);
    // round-trips back through the canonical hex encoder
    assert_eq!(dogtag_standard::to_hex32(&with), format!("0x{one}"));
}

#[test]
fn from_hex32_rejects_non_hex() {
    assert!(msg(from_hex32("0xzz").unwrap_err()).contains("bad hex32"));
}

#[test]
fn from_hex32_rejects_wrong_length() {
    // 2 bytes, not 32
    assert!(msg(from_hex32("0xaabb").unwrap_err()).contains("hex32 must be 32 bytes (got 2)"));
}

#[test]
fn from_hex32_rejects_value_at_or_above_field_modulus() {
    // all-ones 32-byte word is far above the BN254 scalar field modulus, so the canonical
    // re-encode no longer matches the input and the guard fires.
    let too_big = "f".repeat(64);
    assert!(msg(from_hex32(&too_big).unwrap_err()).contains("hex exceeds field"));
}

// ---- parse_packed --------------------------------------------------------------------------

#[test]
fn parse_packed_keeps_colons_in_value_and_allows_empty_value() {
    let (salt, tag, rest) = parse_packed("aabb:2:a:b:c").unwrap();
    assert_eq!((salt.as_str(), tag), ("aabb", TypeTag::String));
    assert_eq!(rest, "a:b:c");

    let (_, _, empty) = parse_packed("aabb:2:").unwrap();
    assert_eq!(empty, "");
}

#[test]
fn parse_packed_requires_two_colons() {
    assert!(msg(parse_packed("aabb").unwrap_err()).contains("bad packed value"));
    assert!(msg(parse_packed("aabb:2").unwrap_err()).contains("bad packed value"));
}

#[test]
fn parse_packed_rejects_non_numeric_and_unknown_tag() {
    assert!(msg(parse_packed("aa:x:v").unwrap_err()).contains("bad packed value"));
    assert_eq!(msg(parse_packed("aa:9:v").unwrap_err()), "unknown tag 9");
}

// ---- scalar_from_packed --------------------------------------------------------------------

#[test]
fn scalar_from_packed_reconstructs_each_variant() {
    assert_eq!(
        scalar_from_packed(TypeTag::Null, "ignored").unwrap(),
        TypedScalar::Null
    );
    assert_eq!(
        scalar_from_packed(TypeTag::Bool, "true").unwrap(),
        TypedScalar::Bool(true)
    );
    // anything other than the exact literal "true" is false
    assert_eq!(
        scalar_from_packed(TypeTag::Bool, "false").unwrap(),
        TypedScalar::Bool(false)
    );
    assert_eq!(
        scalar_from_packed(TypeTag::Bool, "TRUE").unwrap(),
        TypedScalar::Bool(false)
    );
    assert_eq!(
        scalar_from_packed(TypeTag::String, "Rex").unwrap(),
        TypedScalar::Str("Rex".to_string())
    );
    assert_eq!(
        scalar_from_packed(TypeTag::Integer, "42").unwrap(),
        TypedScalar::Integer("42".to_string())
    );
    assert_eq!(
        scalar_from_packed(TypeTag::Decimal, "22.7").unwrap(),
        TypedScalar::Decimal("22.7".to_string())
    );
    assert_eq!(
        scalar_from_packed(TypeTag::Bytes, "deadbeef").unwrap(),
        TypedScalar::Bytes(vec![0xde, 0xad, 0xbe, 0xef])
    );
}

#[test]
fn scalar_from_packed_rejects_bad_bytes_hex() {
    assert!(msg(scalar_from_packed(TypeTag::Bytes, "zz").unwrap_err()).contains("bad bytes hex"));
}

// ---- leaf_from_packed ----------------------------------------------------------------------

#[test]
fn leaf_from_packed_round_trips_a_valid_entry() {
    // a 16-byte salt (32 hex chars), String tag, value "Rex"
    let salt_hex = "00112233445566778899aabbccddeeff";
    let leaf = leaf_from_packed("credentialSubject.name", &format!("{salt_hex}:2:Rex"));
    assert!(leaf.is_ok());
}

#[test]
fn leaf_from_packed_rejects_bad_salt_hex() {
    // parses as a packed value (salt "zz", String tag, "hi") but the salt is not valid hex
    assert!(msg(leaf_from_packed("k", "zz:2:hi").unwrap_err()).contains("bad salt hex"));
}

// ---- flatten_data --------------------------------------------------------------------------

#[test]
fn flatten_data_walks_objects_arrays_and_skips_non_strings() {
    let data = json!({
        "a": "s1:2:x",
        "arr": ["p:2:one", "q:2:two"],
        "b": { "c": "s2:3:5" },
        "skipNumber": 42,
        "skipNull": null
    });
    // serde_json's default Map is a BTreeMap, so sibling object keys come out alphabetically
    // (a < arr < b); array elements stay in index order; numbers/null produce no leaf.
    let pairs = flatten_data(&data);
    assert_eq!(
        pairs,
        vec![
            ("a".to_string(), "s1:2:x".to_string()),
            ("arr[0]".to_string(), "p:2:one".to_string()),
            ("arr[1]".to_string(), "q:2:two".to_string()),
            ("b.c".to_string(), "s2:3:5".to_string()),
        ]
    );
}

#[test]
fn flatten_data_handles_root_string_with_empty_path() {
    let pairs = flatten_data(&json!("root:2:hi"));
    assert_eq!(pairs, vec![("".to_string(), "root:2:hi".to_string())]);
}
