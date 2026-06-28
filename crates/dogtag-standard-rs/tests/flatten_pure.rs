//! Unit coverage for the `flatten` module's pinned keyPath grammar (impl §11.2 F2a).
//!
//! `flatten`/`tokenize_key_path`/`unflatten` are pure, deterministic functions that define the
//! load-bearing keyPath shape that gets hashed, so their observable contract - path formation,
//! the `{tag,value}` leaf decoder, the array-index grammar, and the structural error paths -
//! is pinned here without touching the cross-language parity vectors.

use dogtag_standard::flatten::{flatten, tokenize_key_path, unflatten, Token};
use dogtag_standard::types::TypedScalar;
use serde_json::json;

/// Collapse `flatten()` output into comparable (keyPath, scalar) pairs.
fn pairs(v: &serde_json::Value) -> Vec<(String, TypedScalar)> {
    flatten(v)
        .unwrap()
        .into_iter()
        .map(|e| (e.key_path, e.scalar))
        .collect()
}

// ---------------------------------------------------------------------------
// flatten - keyPath formation + the {tag,value} leaf decoder
// ---------------------------------------------------------------------------

#[test]
fn flatten_nested_object_builds_dotted_keypaths() {
    // tags: 2=String, 3=Integer. serde_json's default Map is a BTreeMap, so sibling object keys
    // emit in sorted order ("microchip" < "name") - that deterministic order is the hashed contract.
    let cred = json!({
        "credentialSubject": {
            "name": {"tag": 2, "value": "Rex"},
            "microchip": {"code": {"tag": 2, "value": "985141006580311"}}
        }
    });
    assert_eq!(
        pairs(&cred),
        vec![
            (
                "credentialSubject.microchip.code".to_string(),
                TypedScalar::Str("985141006580311".to_string())
            ),
            (
                "credentialSubject.name".to_string(),
                TypedScalar::Str("Rex".to_string())
            ),
        ]
    );
}

#[test]
fn flatten_arrays_use_bracket_indices() {
    let cred = json!({
        "weights": [{"tag": 4, "value": "22.7"}, {"tag": 4, "value": "23.1"}]
    });
    assert_eq!(
        pairs(&cred),
        vec![
            (
                "weights[0]".to_string(),
                TypedScalar::Decimal("22.7".to_string())
            ),
            (
                "weights[1]".to_string(),
                TypedScalar::Decimal("23.1".to_string())
            ),
        ]
    );
}

#[test]
fn flatten_typed_scalar_at_root_has_empty_keypath() {
    let cred = json!({"tag": 1, "value": true});
    assert_eq!(
        pairs(&cred),
        vec![("".to_string(), TypedScalar::Bool(true))]
    );
}

#[test]
fn flatten_empty_object_and_array_become_null_leaves() {
    assert_eq!(
        pairs(&json!({"a": {}})),
        vec![("a".to_string(), TypedScalar::Null)]
    );
    assert_eq!(
        pairs(&json!({"a": []})),
        vec![("a".to_string(), TypedScalar::Null)]
    );
}

#[test]
fn flatten_decodes_every_tag_variant() {
    // tag 0=Null, 1=Bool, 2=String, 3=Integer, 4=Decimal, 5=Bytes(hex).
    // Sibling keys emit in BTreeMap-sorted order: b, d, i, n, s, x.
    let cred = json!({
        "n": {"tag": 0, "value": null},
        "b": {"tag": 1, "value": false},
        "s": {"tag": 2, "value": "hi"},
        "i": {"tag": 3, "value": "42"},
        "d": {"tag": 4, "value": "3.14"},
        "x": {"tag": 5, "value": "deadbeef"}
    });
    assert_eq!(
        pairs(&cred),
        vec![
            ("b".to_string(), TypedScalar::Bool(false)),
            ("d".to_string(), TypedScalar::Decimal("3.14".to_string())),
            ("i".to_string(), TypedScalar::Integer("42".to_string())),
            ("n".to_string(), TypedScalar::Null),
            ("s".to_string(), TypedScalar::Str("hi".to_string())),
            (
                "x".to_string(),
                TypedScalar::Bytes(vec![0xde, 0xad, 0xbe, 0xef])
            ),
        ]
    );
}

// ---------------------------------------------------------------------------
// flatten - structural error paths
// ---------------------------------------------------------------------------

#[test]
fn flatten_rejects_reserved_chars_in_object_key() {
    for bad in ["a.b", "a[b", "a]b"] {
        let cred = json!({ bad: {"tag": 1, "value": true} });
        assert!(flatten(&cred).is_err(), "key {bad:?} must be rejected");
    }
}

#[test]
fn flatten_rejects_unknown_tag() {
    let cred = json!({"a": {"tag": 9, "value": "x"}});
    assert!(flatten(&cred).is_err());
}

#[test]
fn flatten_rejects_bad_hex_for_bytes_tag() {
    let cred = json!({"a": {"tag": 5, "value": "zz"}});
    assert!(flatten(&cred).is_err());
}

#[test]
fn flatten_rejects_value_type_mismatch_per_tag() {
    // bool tag with a non-bool value, string tag with a number value, integer tag with a number.
    assert!(flatten(&json!({"a": {"tag": 1, "value": "yes"}})).is_err());
    assert!(flatten(&json!({"a": {"tag": 2, "value": 5}})).is_err());
    assert!(flatten(&json!({"a": {"tag": 3, "value": 5}})).is_err());
}

#[test]
fn flatten_rejects_bare_untyped_leaf() {
    // a raw scalar that is not wrapped as {tag,value}
    assert!(flatten(&json!({"a": "raw"})).is_err());
    assert!(flatten(&json!({"a": 5})).is_err());
}

#[test]
fn flatten_object_missing_value_is_not_a_typed_scalar() {
    // {tag} without "value" is treated as a plain object, so its (untyped) contents error out.
    assert!(flatten(&json!({"a": {"tag": 2}})).is_err());
}

// ---------------------------------------------------------------------------
// tokenize_key_path - the pinned segment grammar
// ---------------------------------------------------------------------------

#[test]
fn tokenize_mixed_path() {
    let toks = tokenize_key_path("a.b[0].c[12]").unwrap();
    assert_eq!(
        toks,
        vec![
            Token::Key("a".to_string()),
            Token::Key("b".to_string()),
            Token::Index(0),
            Token::Key("c".to_string()),
            Token::Index(12),
        ]
    );
}

#[test]
fn tokenize_rejects_unterminated_index() {
    assert!(tokenize_key_path("a[0").is_err());
}

#[test]
fn tokenize_rejects_leading_zero_index() {
    // is_array_index: "0" ok, but "01"/"00" rejected (no leading zeros)
    assert!(tokenize_key_path("a[0]").is_ok());
    assert!(tokenize_key_path("a[01]").is_err());
    assert!(tokenize_key_path("a[00]").is_err());
}

#[test]
fn tokenize_rejects_non_numeric_index() {
    assert!(tokenize_key_path("a[x]").is_err());
    assert!(tokenize_key_path("a[]").is_err());
}

// ---------------------------------------------------------------------------
// unflatten - rebuilds nested structure + array holes
// ---------------------------------------------------------------------------

#[test]
fn unflatten_rebuilds_nested_objects_and_arrays() {
    let entries = vec![
        ("a.b".to_string(), "p1".to_string()),
        ("a.c[0]".to_string(), "p2".to_string()),
        ("a.c[1]".to_string(), "p3".to_string()),
    ];
    let tree = unflatten(&entries).unwrap();
    assert_eq!(tree, json!({"a": {"b": "p1", "c": ["p2", "p3"]}}));
}

#[test]
fn unflatten_fills_array_holes_with_null() {
    // index 2 present, 0 and 1 absent -> nulls (mirrors JS sparse-array holes)
    let entries = vec![("a[2]".to_string(), "p".to_string())];
    let tree = unflatten(&entries).unwrap();
    assert_eq!(tree, json!({"a": [null, null, "p"]}));
}

#[test]
fn unflatten_propagates_bad_keypath_error() {
    let entries = vec![("a[01]".to_string(), "p".to_string())];
    assert!(unflatten(&entries).is_err());
}
