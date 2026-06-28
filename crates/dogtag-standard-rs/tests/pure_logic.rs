//! Unit coverage for the crate's pure, dependency-light helpers (no external proving deps):
//! `util` big-int <-> decimal codecs, `encode` canonicalization, `types` tag mapping,
//! `field` packing/encoding, `merkle` tree/proof round-trips, and `leaf` input validation.
//!
//! These tests assert internal invariants (round-trips, commutativity, determinism, error paths)
//! and well-known public constants only - they do NOT hardcode guessed Poseidon outputs, so they
//! stay independent of the cross-language `testvectors.json` parity gate while still pinning the
//! observable contract of each helper.

use ark_bn254::Fr;

use dogtag_standard::encode::{as_string, canonical_decimal, canonical_integer, encode_value, nfc};
use dogtag_standard::field::{field_le, field_modulus_dec, to_hex32};
use dogtag_standard::leaf::hash_leaf;
use dogtag_standard::merkle::{build_merkle, hash_node, merkle_proof, process_proof};
use dogtag_standard::types::{DogTagError, TypeTag, TypedScalar};
use dogtag_standard::util::{be_bytes_to_dec, dec_to_le_bytes};

// ---------------------------------------------------------------------------
// util - schoolbook big-integer <-> decimal codecs
// ---------------------------------------------------------------------------

#[test]
fn be_bytes_to_dec_known_values() {
    assert_eq!(be_bytes_to_dec(&[]), "0");
    assert_eq!(be_bytes_to_dec(&[0x00]), "0");
    assert_eq!(be_bytes_to_dec(&[0x01]), "1");
    assert_eq!(be_bytes_to_dec(&[0x10]), "16");
    assert_eq!(be_bytes_to_dec(&[0xff]), "255");
    assert_eq!(be_bytes_to_dec(&[0x01, 0x00]), "256");
    assert_eq!(be_bytes_to_dec(&[0xff, 0xff]), "65535");
    // leading zero bytes do not change the value
    assert_eq!(be_bytes_to_dec(&[0x00, 0x00, 0x2a]), "42");
}

#[test]
fn dec_to_le_bytes_known_values() {
    assert_eq!(dec_to_le_bytes("0"), vec![0]);
    assert_eq!(dec_to_le_bytes("255"), vec![255]);
    assert_eq!(dec_to_le_bytes("256"), vec![0, 1]);
    assert_eq!(dec_to_le_bytes("65535"), vec![255, 255]);
    assert_eq!(dec_to_le_bytes("42"), vec![42]);
}

#[test]
fn dec_and_be_roundtrip_over_modulus() {
    // The two codecs are inverses (modulo leading-zero trimming) for an arbitrary large value:
    // start from the BN254 modulus decimal, go decimal -> little-endian -> big-endian -> decimal.
    let dec = field_modulus_dec();
    let le = dec_to_le_bytes(&dec);
    let be: Vec<u8> = le.iter().rev().copied().collect();
    assert_eq!(be_bytes_to_dec(&be), dec);
}

#[test]
fn field_modulus_dec_is_bn254_scalar_order() {
    // The well-known BN254 / alt_bn128 scalar field order r (EIP-197 / circom).
    assert_eq!(
        field_modulus_dec(),
        "21888242871839275222246405745257275088548364400416034343698204186575808495617"
    );
}

// ---------------------------------------------------------------------------
// encode - NFC + canonical integer/decimal
// ---------------------------------------------------------------------------

#[test]
fn nfc_composes_combining_sequence() {
    // "e" + COMBINING ACUTE ACCENT (U+0301) -> precomposed "é" (U+00E9).
    let composed = nfc("e\u{0301}");
    assert_eq!(composed, "\u{00e9}");
    assert_eq!(composed.chars().count(), 1);
    // Already-NFC ASCII is unchanged.
    assert_eq!(nfc("plain"), "plain");
}

#[test]
fn canonical_integer_accepts_and_normalizes() {
    assert_eq!(canonical_integer("0").unwrap(), "0");
    assert_eq!(canonical_integer("-0").unwrap(), "0");
    assert_eq!(canonical_integer("123").unwrap(), "123");
    assert_eq!(canonical_integer("-123").unwrap(), "-123");
}

#[test]
fn canonical_integer_rejects_malformed() {
    for bad in ["", "-", "007", "1.5", "abc", "+5", " 5"] {
        assert!(
            matches!(canonical_integer(bad), Err(DogTagError::InvalidInteger(_))),
            "expected InvalidInteger for {bad:?}"
        );
    }
}

#[test]
fn canonical_decimal_strips_trailing_zeros() {
    assert_eq!(canonical_decimal("1.2300").unwrap(), "1.23");
    assert_eq!(canonical_decimal("1.000").unwrap(), "1");
    assert_eq!(canonical_decimal("0.0").unwrap(), "0");
    assert_eq!(canonical_decimal("-0").unwrap(), "0");
    assert_eq!(canonical_decimal("-0.0").unwrap(), "0");
    assert_eq!(canonical_decimal("123").unwrap(), "123");
    assert_eq!(canonical_decimal("-1.5").unwrap(), "-1.5");
}

#[test]
fn canonical_decimal_rejects_malformed() {
    for bad in ["00.5", "1.", ".5", "1.2.3", "", "abc", "1,5"] {
        assert!(
            matches!(canonical_decimal(bad), Err(DogTagError::InvalidDecimal(_))),
            "expected InvalidDecimal for {bad:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// encode - encode_value / as_string across every TypedScalar variant
// ---------------------------------------------------------------------------

#[test]
fn encode_value_per_variant() {
    assert_eq!(encode_value(&TypedScalar::Null).unwrap(), Vec::<u8>::new());
    assert_eq!(encode_value(&TypedScalar::Bool(true)).unwrap(), vec![0x01]);
    assert_eq!(encode_value(&TypedScalar::Bool(false)).unwrap(), vec![0x00]);
    assert_eq!(
        encode_value(&TypedScalar::Str("e\u{0301}".into())).unwrap(),
        "\u{00e9}".as_bytes()
    );
    assert_eq!(
        encode_value(&TypedScalar::Integer("42".into())).unwrap(),
        b"42".to_vec()
    );
    assert_eq!(
        encode_value(&TypedScalar::Bytes(vec![0xde, 0xad])).unwrap(),
        vec![0xde, 0xad]
    );
    // canonicalization errors propagate out of encode_value.
    assert!(encode_value(&TypedScalar::Integer("007".into())).is_err());
}

#[test]
fn as_string_per_variant() {
    assert_eq!(as_string(&TypedScalar::Null).unwrap(), "");
    assert_eq!(as_string(&TypedScalar::Bool(true)).unwrap(), "true");
    assert_eq!(as_string(&TypedScalar::Bool(false)).unwrap(), "false");
    assert_eq!(
        as_string(&TypedScalar::Str("e\u{0301}".into())).unwrap(),
        "\u{00e9}"
    );
    assert_eq!(as_string(&TypedScalar::Integer("-0".into())).unwrap(), "0");
    assert_eq!(
        as_string(&TypedScalar::Decimal("1.2300".into())).unwrap(),
        "1.23"
    );
    assert_eq!(
        as_string(&TypedScalar::Bytes(vec![0xde, 0xad])).unwrap(),
        "dead"
    );
}

// ---------------------------------------------------------------------------
// types - TypeTag <-> u8 and TypedScalar::tag
// ---------------------------------------------------------------------------

#[test]
fn type_tag_from_u8_roundtrips() {
    let all = [
        TypeTag::Null,
        TypeTag::Bool,
        TypeTag::String,
        TypeTag::Integer,
        TypeTag::Decimal,
        TypeTag::Bytes,
    ];
    for t in all {
        assert_eq!(TypeTag::from_u8(t as u8), Some(t));
    }
    assert_eq!(TypeTag::from_u8(6), None);
    assert_eq!(TypeTag::from_u8(255), None);
}

#[test]
fn type_tag_repr_is_stable() {
    // The on-wire tag values are part of the canonical encoding - pin them.
    assert_eq!(TypeTag::Null as u8, 0);
    assert_eq!(TypeTag::Bool as u8, 1);
    assert_eq!(TypeTag::String as u8, 2);
    assert_eq!(TypeTag::Integer as u8, 3);
    assert_eq!(TypeTag::Decimal as u8, 4);
    assert_eq!(TypeTag::Bytes as u8, 5);
}

#[test]
fn typed_scalar_tag_matches_variant() {
    assert_eq!(TypedScalar::Null.tag(), TypeTag::Null);
    assert_eq!(TypedScalar::Bool(true).tag(), TypeTag::Bool);
    assert_eq!(TypedScalar::Str("x".into()).tag(), TypeTag::String);
    assert_eq!(TypedScalar::Integer("1".into()).tag(), TypeTag::Integer);
    assert_eq!(TypedScalar::Decimal("1.0".into()).tag(), TypeTag::Decimal);
    assert_eq!(TypedScalar::Bytes(vec![]).tag(), TypeTag::Bytes);
}

// ---------------------------------------------------------------------------
// field - to_hex32 / field_le
// ---------------------------------------------------------------------------

#[test]
fn to_hex32_is_padded_big_endian() {
    let zero = to_hex32(&Fr::from(0u64));
    assert_eq!(zero, format!("0x{}", "0".repeat(64)));
    let one = to_hex32(&Fr::from(1u64));
    assert_eq!(one.len(), 66);
    assert!(one.starts_with("0x"));
    assert!(one.ends_with("01"));
}

#[test]
fn field_le_orders_by_integer_value() {
    let a = Fr::from(1u64);
    let b = Fr::from(2u64);
    assert!(field_le(&a, &b));
    assert!(!field_le(&b, &a));
    assert!(field_le(&a, &a)); // reflexive (<=)
}

// ---------------------------------------------------------------------------
// merkle - hash_node commutativity + build/prove/process round-trips
// ---------------------------------------------------------------------------

#[test]
fn hash_node_is_commutative() {
    let a = Fr::from(7u64);
    let b = Fr::from(99u64);
    assert_eq!(hash_node(a, b), hash_node(b, a));
}

#[test]
fn single_leaf_tree_is_its_own_root() {
    let leaf = Fr::from(123u64);
    let tree = build_merkle(&[leaf]);
    assert_eq!(tree.root, leaf);
    let proof = merkle_proof(&tree.layers, leaf);
    assert!(proof.is_empty());
    assert_eq!(process_proof(&proof, leaf), tree.root);
}

#[test]
fn merkle_proofs_reconstruct_root_for_all_sizes() {
    // Use distinct synthetic leaf hashes; merkle ops are defined over arbitrary Fr.
    for n in 1u64..=8 {
        let leaves: Vec<Fr> = (0..n).map(|i| Fr::from(1000 + i)).collect();
        let tree = build_merkle(&leaves);
        for &leaf in &leaves {
            let proof = merkle_proof(&tree.layers, leaf);
            assert_eq!(
                process_proof(&proof, leaf),
                tree.root,
                "proof failed to reconstruct root for n={n}"
            );
        }
    }
}

#[test]
fn build_merkle_is_order_independent_and_deterministic() {
    let a = build_merkle(&[Fr::from(3u64), Fr::from(1u64), Fr::from(2u64)]);
    let b = build_merkle(&[Fr::from(1u64), Fr::from(2u64), Fr::from(3u64)]);
    // Leaves are sorted internally, so input order does not change the root.
    assert_eq!(a.root, b.root);
}

// ---------------------------------------------------------------------------
// leaf - input validation + determinism
// ---------------------------------------------------------------------------

#[test]
fn hash_leaf_requires_16_byte_salt() {
    let value = TypedScalar::Str("rex".into());
    assert!(matches!(
        hash_leaf("pet.name", &[0u8; 8], &value),
        Err(DogTagError::Other(_))
    ));
    assert!(hash_leaf("pet.name", &[0u8; 16], &value).is_ok());
}

#[test]
fn hash_leaf_is_deterministic() {
    let salt = [9u8; 16];
    let value = TypedScalar::Integer("5".into());
    let h1 = hash_leaf("pet.age", &salt, &value).unwrap();
    let h2 = hash_leaf("pet.age", &salt, &value).unwrap();
    assert_eq!(h1, h2);
}

#[test]
fn hash_leaf_propagates_canonicalization_error() {
    // A non-canonical integer ("007") must surface as an error, not a silent hash.
    let salt = [0u8; 16];
    assert!(hash_leaf("pet.age", &salt, &TypedScalar::Integer("007".into())).is_err());
}
