//! UniFFI foreign-callable surface for the DogTag standard SDK (Phase 6 — mobile).
//!
//! This module is the ONLY binding surface; it exposes string/bytes/record wrappers over the
//! pure core (poseidon/field/leaf/merkle/encode/wrap/verify/consent) so Kotlin (Android) and
//! Swift (iOS) can run the offline §11.3 integrity verify and reproduce server-side roots
//! byte-for-byte. The core algorithm modules are NOT modified — this is additive only.
//!
//! Proc-macro (no-UDL) UniFFI 0.28. Errors are surfaced as a single `FfiError` enum so the
//! foreign bindings get idiomatic thrown exceptions.

use ark_bn254::Fr;
use ark_ff::PrimeField;
use serde_json::Value;

use crate::encode::nfc;
use crate::field::{bytes_to_field, to_hex32};
use crate::leaf::hash_leaf;
use crate::merkle::build_merkle;
use crate::types::{TypeTag, TypedScalar};
use crate::verify::{check_integrity, FragmentState};
use crate::wrap::{from_hex32, scalar_from_packed, wrap_document, IssuerMeta, WrappedDoc};

/// A single error type crossing the FFI boundary (UniFFI maps this to a thrown exception).
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("{0}")]
    Invalid(String),
}

impl From<crate::types::DogTagError> for FfiError {
    fn from(e: crate::types::DogTagError) -> Self {
        FfiError::Invalid(e.to_string())
    }
}

fn err<E: std::fmt::Display>(e: E) -> FfiError {
    FfiError::Invalid(e.to_string())
}

/// Decode a salt hex string (16 bytes / 32 hex chars) into bytes.
fn decode_salt(salt_hex: &str) -> Result<Vec<u8>, FfiError> {
    let s = salt_hex.strip_prefix("0x").unwrap_or(salt_hex);
    let bytes = hex::decode(s).map_err(|e| err(format!("bad salt hex: {e}")))?;
    if bytes.len() != 16 {
        return Err(FfiError::Invalid(format!(
            "salt must be 16 bytes (got {})",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// hashLeaf over a single field: Poseidon(DS_LEAF, fieldOf(keyPath), fieldOf(salt),
/// fieldOf(typeTag), fieldOf(value)). `tag`+`value` are reconstructed into a TypedScalar exactly
/// like `wrap::scalar_from_packed` does (the same path verify uses). Returns 0x.. 32-byte hex.
#[uniffi::export]
pub fn hash_leaf_hex(
    key_path: String,
    salt_hex: String,
    tag: u8,
    value: String,
) -> Result<String, FfiError> {
    let salt = decode_salt(&salt_hex)?;
    let type_tag = TypeTag::from_u8(tag)
        .ok_or_else(|| FfiError::Invalid(format!("unknown tag {tag}")))?;
    let scalar: TypedScalar = scalar_from_packed(type_tag, &value)?;
    let f = hash_leaf(&key_path, &salt, &scalar)?;
    Ok(to_hex32(&f))
}

/// buildMerkle over a set of 0x.. 32-byte leaf hashes -> the 0x.. 32-byte root hex.
/// Sorts ascending and folds bottom-up (promote lone odd) — mirrors the SDK / TS.
#[uniffi::export]
pub fn build_merkle_root_hex(leaf_hexes: Vec<String>) -> Result<String, FfiError> {
    if leaf_hexes.is_empty() {
        return Err(FfiError::Invalid("empty leaf set".to_string()));
    }
    let mut leaves: Vec<Fr> = Vec::with_capacity(leaf_hexes.len());
    for h in &leaf_hexes {
        leaves.push(from_hex32(h)?);
    }
    Ok(to_hex32(&build_merkle(&leaves).root))
}

/// bytesToField: the length-prefixed, 31-byte-chunked, domain-separated Poseidon fold of raw
/// bytes (hex in) -> the 0x.. 32-byte field hex. Used for keyPath/value parity vectors.
#[uniffi::export]
pub fn bytes_to_field_hex(input_hex: String) -> Result<String, FfiError> {
    let s = input_hex.strip_prefix("0x").unwrap_or(&input_hex);
    let bytes = hex::decode(s).map_err(|e| err(format!("bad input hex: {e}")))?;
    Ok(to_hex32(&bytes_to_field(&bytes)))
}

/// wrapDocument — typed credential JSON + issuer JSON -> WrappedDoc JSON. Salts come from the OS
/// RNG (each leaf gets 16 fresh bytes). Mirrors `wrap::wrap_document`.
#[uniffi::export]
pub fn wrap_document_json(
    typed_credential_json: String,
    issuer_json: String,
) -> Result<String, FfiError> {
    let typed: Value =
        serde_json::from_str(&typed_credential_json).map_err(|e| err(format!("bad credential json: {e}")))?;
    let issuer: IssuerMeta =
        serde_json::from_str(&issuer_json).map_err(|e| err(format!("bad issuer json: {e}")))?;

    let mut salt_provider = || {
        let mut s = [0u8; 16];
        getrandom::getrandom(&mut s).expect("OS RNG failure");
        s
    };
    let doc = wrap_document(&typed, issuer, &mut salt_provider)?;
    serde_json::to_string(&doc).map_err(|e| err(format!("serialize: {e}")))
}

/// The pure §11.3 integrity pillar over a WrappedDoc JSON: rebuild the whole tree and compare to
/// targetHash/merkleRoot. This is what mobile runs OFFLINE. Returns "VALID" / "INVALID".
#[uniffi::export]
pub fn verify_integrity(wrapped_doc_json: String) -> Result<String, FfiError> {
    let doc: WrappedDoc =
        serde_json::from_str(&wrapped_doc_json).map_err(|e| err(format!("bad wrapped doc json: {e}")))?;
    let (state, _root) = check_integrity(&doc);
    Ok(match state {
        FragmentState::Valid => "VALID".to_string(),
        _ => "INVALID".to_string(),
    })
}

// --------------------------------------------------------------------------------------------
// Consent commitments (mirror consent.rs) — digest / nullifier / message / keyHash for parity.
// --------------------------------------------------------------------------------------------

/// keccak256 of the EIP-712 VerificationConsent type string (0x.. 32-byte hex).
#[uniffi::export]
pub fn verification_consent_typehash_hex() -> String {
    format!("0x{}", hex::encode(crate::consent::verification_consent_typehash()))
}

/// Decode a 0x.. hex string into exactly N bytes (big-endian word / address).
fn decode_word<const N: usize>(label: &str, h: &str) -> Result<[u8; N], FfiError> {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let bytes = hex::decode(s).map_err(|e| err(format!("bad {label} hex: {e}")))?;
    if bytes.len() != N {
        return Err(FfiError::Invalid(format!(
            "{label} must be {N} bytes (got {})",
            bytes.len()
        )));
    }
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// Build a VerificationConsent from hex inputs. uint256/bytes32 fields are 32-byte BE hex,
/// addresses are 20-byte hex.
#[allow(clippy::too_many_arguments)]
fn consent_from_hex(
    dog_tag_id_hex: &str,
    record_type_hex: &str,
    purpose_hex: &str,
    credential_root_hex: &str,
    challenge_hex: &str,
    relayer_hex: &str,
    subject_hex: &str,
    nonce_hex: &str,
    deadline_hex: &str,
) -> Result<crate::consent::VerificationConsent, FfiError> {
    Ok(crate::consent::VerificationConsent {
        dog_tag_id: decode_word::<32>("dogTagId", dog_tag_id_hex)?,
        record_type: decode_word::<32>("recordType", record_type_hex)?,
        purpose: decode_word::<32>("purpose", purpose_hex)?,
        credential_root: decode_word::<32>("credentialRoot", credential_root_hex)?,
        challenge: decode_word::<32>("challenge", challenge_hex)?,
        relayer: decode_word::<20>("relayer", relayer_hex)?,
        subject: decode_word::<20>("subject", subject_hex)?,
        nonce: decode_word::<32>("nonce", nonce_hex)?,
        deadline: decode_word::<32>("deadline", deadline_hex)?,
    })
}

/// The consent nullifier (impl §11.9(b)): Poseidon(DS_NULLIFIER, dogTagId, purpose, relayer,
/// subject, nonce) -> 0x.. 32-byte hex.
#[allow(clippy::too_many_arguments)]
#[uniffi::export]
pub fn consent_nullifier_hex(
    dog_tag_id_hex: String,
    record_type_hex: String,
    purpose_hex: String,
    credential_root_hex: String,
    challenge_hex: String,
    relayer_hex: String,
    subject_hex: String,
    nonce_hex: String,
    deadline_hex: String,
) -> Result<String, FfiError> {
    let c = consent_from_hex(
        &dog_tag_id_hex,
        &record_type_hex,
        &purpose_hex,
        &credential_root_hex,
        &challenge_hex,
        &relayer_hex,
        &subject_hex,
        &nonce_hex,
        &deadline_hex,
    )?;
    Ok(format!("0x{}", hex::encode(crate::consent::consent_nullifier(&c))))
}

/// The EdDSA-BabyJubjub consent message M (impl §11.9(d)): Poseidon(dogTagId, purpose, relayer,
/// subject, credentialRoot, nonce) -> 0x.. 32-byte hex.
#[allow(clippy::too_many_arguments)]
#[uniffi::export]
pub fn eddsa_consent_message_hex(
    dog_tag_id_hex: String,
    record_type_hex: String,
    purpose_hex: String,
    credential_root_hex: String,
    challenge_hex: String,
    relayer_hex: String,
    subject_hex: String,
    nonce_hex: String,
    deadline_hex: String,
) -> Result<String, FfiError> {
    let c = consent_from_hex(
        &dog_tag_id_hex,
        &record_type_hex,
        &purpose_hex,
        &credential_root_hex,
        &challenge_hex,
        &relayer_hex,
        &subject_hex,
        &nonce_hex,
        &deadline_hex,
    )?;
    Ok(to_hex32(&crate::consent::eddsa_consent_message(&c)))
}

/// keyHash = Poseidon(Ax, Ay) -> 0x.. 32-byte hex. Ax/Ay are 0x.. 32-byte BE field hex.
#[uniffi::export]
pub fn key_hash_hex(ax_hex: String, ay_hex: String) -> Result<String, FfiError> {
    let ax = field_from_hex(&ax_hex)?;
    let ay = field_from_hex(&ay_hex)?;
    Ok(format!("0x{}", hex::encode(crate::consent::key_hash(ax, ay))))
}

/// Parse a 0x.. 32-byte hex into a field element (reduced mod r if needed, like the TS leg).
fn field_from_hex(h: &str) -> Result<Fr, FfiError> {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let bytes = hex::decode(s).map_err(|e| err(format!("bad field hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(FfiError::Invalid(format!(
            "field hex must be 32 bytes (got {})",
            bytes.len()
        )));
    }
    Ok(Fr::from_be_bytes_mod_order(&bytes))
}

/// NFC-normalize a string (exposed for cross-language canonicalization sanity checks).
#[uniffi::export]
pub fn nfc_normalize(input: String) -> String {
    nfc(&input)
}
