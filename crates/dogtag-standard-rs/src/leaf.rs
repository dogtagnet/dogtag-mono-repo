//! Poseidon leaf hashing (impl §1.2, §11.2) — mirror of dogtag-standard-ts/src/leaf.ts.
use ark_bn254::Fr;

use crate::encode::{encode_value, nfc};
use crate::field::{bytes_to_field, field_from_scalar_bytes, field_from_uint};
use crate::poseidon::{poseidon, DS_LEAF};
use crate::types::{DogTagError, TypedScalar};

pub fn field_of_keypath(keypath: &str) -> Fr {
    bytes_to_field(nfc(keypath).as_bytes())
}

pub fn field_of_value(s: &TypedScalar) -> Result<Fr, DogTagError> {
    Ok(bytes_to_field(&encode_value(s)?))
}

/// hashLeaf — Poseidon(DS_LEAF, fieldOf(keyPath), fieldOf(salt), fieldOf(typeTag), fieldOf(value)).
pub fn hash_leaf(keypath: &str, salt: &[u8], s: &TypedScalar) -> Result<Fr, DogTagError> {
    if salt.len() != 16 {
        return Err(DogTagError::Other(format!("salt must be 16 bytes (got {})", salt.len())));
    }
    Ok(poseidon(&[
        Fr::from(DS_LEAF),
        field_of_keypath(keypath),
        field_from_scalar_bytes(salt),
        field_from_uint(s.tag() as u64),
        field_of_value(s)?,
    ]))
}
