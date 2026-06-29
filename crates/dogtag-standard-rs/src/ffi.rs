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

/// Field-hash a numeric dogTagId EXACTLY as its credential leaf value is hashed:
/// `field_of_value(Integer(dec))` -> 0x.. 32-byte hex. THE CANONICAL dogTagId: the §1.10 consent's
/// dogTagId, the EdDSA consent message M, the Poseidon nullifier, AND the on-chain DOG_PROFILE SBT id
/// must ALL be this value — it equals `leafValues[dogTagIdLeafIndex]`, which the verification circuit
/// compares to the dogTagId input DIRECTLY (constraint §(b)), not the raw decimal id.
#[uniffi::export]
pub fn dog_tag_id_field_hex(dog_tag_id_dec: String) -> Result<String, FfiError> {
    let scalar = scalar_from_packed(TypeTag::Integer, &dog_tag_id_dec)?;
    let f = crate::leaf::field_of_value(&scalar)?;
    Ok(to_hex32(&f))
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

/// The EIP-712 digest the owner's secp256k1 wallet signs to authorize a relayer-sponsored
/// consent-key bind (`ConsentKeyRegistry.bindConsentKeyFor`). Returns 0x.. 32-byte hex of
/// keccak256(0x1901 || domainSeparator("DogTag","1",chainId,consentKeyRegistry) ||
/// keccak256(abi.encode(BIND_TYPEHASH, keyHash, wallet, nonce))). NOT feature-gated — mobile
/// needs it regardless of the `prover` feature. `nonce` is `bindNonce[wallet]` (a uint256 < 2^64
/// in practice; passed as u64 and BE-padded to 32 bytes).
#[uniffi::export]
pub fn bind_consent_key_digest_hex(
    consent_key_registry_addr: String,
    key_hash_hex: String,
    wallet_addr: String,
    nonce: u64,
    chain_id: u64,
) -> Result<String, FfiError> {
    let registry = decode_word::<20>("consentKeyRegistryAddr", &consent_key_registry_addr)?;
    let key_hash = decode_word::<32>("keyHash", &key_hash_hex)?;
    let wallet = decode_word::<20>("walletAddr", &wallet_addr)?;
    let mut nonce_word = [0u8; 32];
    nonce_word[24..].copy_from_slice(&nonce.to_be_bytes());
    let digest =
        crate::consent::bind_consent_key_digest(registry, &key_hash, &wallet, &nonce_word, chain_id);
    Ok(format!("0x{}", hex::encode(digest)))
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

// --------------------------------------------------------------------------------------------
// EdDSA-BabyJubjub consent SIGNING (the mobile crypto) — circomlibjs-compatible. The wallet derives
// a per-pet consent key from its seed, binds keyHash=Poseidon(Ax,Ay) on-chain via ConsentKeyRegistry,
// then signs the §1.10 consent message for the ZK verification path.
// --------------------------------------------------------------------------------------------

/// A derived BabyJubjub consent keypair crossing the FFI boundary. `prvHex` is the 32-byte private
/// key (keep encrypted behind the platform keystore); Ax/Ay are 0x.. 32-byte BE public-point hex;
/// keyHashHex = Poseidon(Ax,Ay) is what the wallet binds in ConsentKeyRegistry.
#[derive(uniffi::Record)]
pub struct BabyjubConsentKeyFfi {
    pub prv_hex: String,
    pub ax_hex: String,
    pub ay_hex: String,
    pub key_hash_hex: String,
}

/// An EdDSA-BabyJubjub Poseidon consent signature: R8 point (0x.. 32-byte hex) + scalar S (decimal).
#[derive(uniffi::Record)]
pub struct EddsaSignatureFfi {
    pub r8x_hex: String,
    pub r8y_hex: String,
    pub r8x_dec: String,
    pub r8y_dec: String,
    pub s_dec: String,
}

/// Derive a deterministic BabyJubjub consent key from a hex seed (any length). The seed is wrapped
/// in a distinct domain from the secp256k1 wallet path (§6) before BLAKE-512, so the two keys are
/// independent. Returns the 32-byte private key + public point (Ax, Ay) + keyHash.
#[uniffi::export]
pub fn derive_babyjub_consent_key(seed_hex: String) -> Result<BabyjubConsentKeyFfi, FfiError> {
    let s = seed_hex.strip_prefix("0x").unwrap_or(&seed_hex);
    let seed = hex::decode(s).map_err(|e| err(format!("bad seed hex: {e}")))?;
    if seed.is_empty() {
        return Err(FfiError::Invalid("seed must be non-empty".to_string()));
    }
    let key = crate::eddsa::derive_babyjub_consent_key_from_seed(&seed);
    Ok(consent_key_to_ffi(&key))
}

/// Build a consent key directly from a 32-byte circomlibjs private key (the raw private buffer is
/// the key — no domain wrapping). For interop with vectors / externally-derived keys.
#[uniffi::export]
pub fn babyjub_consent_key_from_prv(prv_hex: String) -> Result<BabyjubConsentKeyFfi, FfiError> {
    let prv = decode_word::<32>("prv", &prv_hex)?;
    let key = crate::eddsa::consent_key_from_raw_prv(&prv);
    Ok(consent_key_to_ffi(&key))
}

fn consent_key_to_ffi(key: &crate::eddsa::BabyjubConsentKey) -> BabyjubConsentKeyFfi {
    BabyjubConsentKeyFfi {
        prv_hex: format!("0x{}", hex::encode(key.prv)),
        ax_hex: to_hex32(&key.ax),
        ay_hex: to_hex32(&key.ay),
        key_hash_hex: format!("0x{}", hex::encode(crate::consent::key_hash(key.ax, key.ay))),
    }
}

/// Sign the §1.10 consent message M = Poseidon6(dogTagId, purpose, relayer, subject, credentialRoot,
/// nonce) with a 32-byte private key, producing the EdDSA-BabyJubjub Poseidon signature the ZK
/// circuit's `EdDSAPoseidonVerifier` accepts. Consent fields are hex (same shape as the other
/// consent functions); `prvHex` is the 32-byte private key.
#[allow(clippy::too_many_arguments)]
#[uniffi::export]
pub fn sign_consent_eddsa(
    prv_hex: String,
    dog_tag_id_hex: String,
    record_type_hex: String,
    purpose_hex: String,
    credential_root_hex: String,
    challenge_hex: String,
    relayer_hex: String,
    subject_hex: String,
    nonce_hex: String,
    deadline_hex: String,
) -> Result<EddsaSignatureFfi, FfiError> {
    let prv = decode_word::<32>("prv", &prv_hex)?;
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
    let m = crate::consent::eddsa_consent_message(&c);
    let sig = crate::eddsa::sign_poseidon(&prv, &m);
    Ok(EddsaSignatureFfi {
        r8x_hex: to_hex32(&sig.r8x),
        r8y_hex: to_hex32(&sig.r8y),
        r8x_dec: crate::eddsa::fr_to_dec(&sig.r8x),
        r8y_dec: crate::eddsa::fr_to_dec(&sig.r8y),
        s_dec: sig.s.to_str_radix(10),
    })
}

/// Verify an EdDSA-BabyJubjub Poseidon consent signature against the public key (Ax,Ay) and the
/// consent fields. Mirrors circomlibjs `verifyPoseidon`. Returns true/false (no throw).
#[allow(clippy::too_many_arguments)]
#[uniffi::export]
pub fn verify_consent_eddsa(
    ax_hex: String,
    ay_hex: String,
    r8x_hex: String,
    r8y_hex: String,
    s_dec: String,
    dog_tag_id_hex: String,
    record_type_hex: String,
    purpose_hex: String,
    credential_root_hex: String,
    challenge_hex: String,
    relayer_hex: String,
    subject_hex: String,
    nonce_hex: String,
    deadline_hex: String,
) -> Result<bool, FfiError> {
    let ax = field_from_hex(&ax_hex)?;
    let ay = field_from_hex(&ay_hex)?;
    let r8x = field_from_hex(&r8x_hex)?;
    let r8y = field_from_hex(&r8y_hex)?;
    let s = num_bigint::BigUint::parse_bytes(s_dec.as_bytes(), 10)
        .ok_or_else(|| FfiError::Invalid("bad S decimal".to_string()))?;
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
    let m = crate::consent::eddsa_consent_message(&c);
    // An off-curve / small-subgroup point is malformed input -> FfiError::Invalid (no panic).
    crate::eddsa::verify_poseidon(&ax, &ay, &r8x, &r8y, &s, &m)
        .map_err(|e| FfiError::Invalid(e.to_string()))
}

/// NFC-normalize a string (exposed for cross-language canonicalization sanity checks).
#[uniffi::export]
pub fn nfc_normalize(input: String) -> String {
    nfc(&input)
}

// --------------------------------------------------------------------------------------------
// VERIFY-whitelist key (pre-proof relayer check) — NOT feature-gated. The mobile app calls this
// even WITHOUT the `prover` feature: before signing/proving, it must verify on-chain that the
// scanned relayer is a whitelisted groomer for this purpose. Byte-for-byte parity with the
// backend `stacks/vet/api/src/verify.rs` `verify_key`/`purpose_key`:
//   purpose = keccak256(label) mod BN254_r   (32-byte BE word)
//   key     = keccak256(abi.encode("VERIFY:", purpose))
// where abi.encode(string,bytes32) lays out [offset=0x40][purpose word][len=7]["VERIFY:" padded].
// --------------------------------------------------------------------------------------------

/// `purpose` field element for a purpose label: keccak256(label) reduced mod the BN254 scalar
/// field r, as a 32-byte big-endian word. Mirrors backend `purpose_key`.
fn purpose_key_word(label: &str) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    let mut h = Keccak256::new();
    h.update(label.as_bytes());
    let digest: [u8; 32] = h.finalize().into();
    // Reduce mod r in the BN254 scalar field, then re-encode canonical 32-byte BE.
    let reduced = Fr::from_be_bytes_mod_order(&digest);
    crate::poseidon::to_be_bytes32(&reduced)
}

/// The IssuerRegistry whitelist key the VerificationRegistry checks for the relayer on a given
/// purpose label: `keccak256(abi.encode("VERIFY:", purpose_key(label)))` as `0x..` hex.
///
/// Used by the mobile pre-proof check (`IssuerRegistry.isWhitelistedFor(key, relayer)`). Available
/// even without the `prover` feature. Byte-for-byte parity with backend `verify.rs::verify_key`.
#[uniffi::export]
pub fn verify_whitelist_key_hex(purpose_label: String) -> String {
    use sha3::{Digest, Keccak256};
    let purpose = purpose_key_word(&purpose_label);
    // abi.encode(string "VERIFY:", bytes32 purpose):
    let mut buf = Vec::with_capacity(128);
    // [0] offset to the string data = 0x40 (after the two head words).
    let mut off = [0u8; 32];
    off[31] = 0x40;
    buf.extend_from_slice(&off);
    // [1] the bytes32 purpose word.
    buf.extend_from_slice(&purpose);
    // [2] string length = 7 ("VERIFY:").
    let mut len = [0u8; 32];
    len[31] = 7;
    buf.extend_from_slice(&len);
    // [3] string bytes, right-padded to 32.
    let mut data = [0u8; 32];
    data[..7].copy_from_slice(b"VERIFY:");
    buf.extend_from_slice(&data);
    let mut h = Keccak256::new();
    h.update(&buf);
    let key: [u8; 32] = h.finalize().into();
    format!("0x{}", hex::encode(key))
}

#[cfg(test)]
mod verify_key_tests {
    use super::*;

    /// Independently recompute the backend `verify.rs::verify_key` (keccak + num-bigint mod r,
    /// the same path alloy's `U256 % r` takes) and assert byte-for-byte parity with our FFI fn.
    #[test]
    fn verify_whitelist_key_matches_backend() {
        use num_bigint::BigUint;
        use sha3::{Digest, Keccak256};
        let r = BigUint::parse_bytes(
            b"21888242871839275222246405745257275088548364400416034343698204186575808495617",
            10,
        )
        .unwrap();
        let reference = |label: &str| -> String {
            let mut h = Keccak256::new();
            h.update(label.as_bytes());
            let full = BigUint::from_bytes_be(&h.finalize());
            let reduced = full % &r;
            let mut purpose = [0u8; 32];
            let rb = reduced.to_bytes_be();
            purpose[32 - rb.len()..].copy_from_slice(&rb);
            let mut buf = Vec::new();
            let mut off = [0u8; 32];
            off[31] = 0x40;
            buf.extend_from_slice(&off);
            buf.extend_from_slice(&purpose);
            let mut len = [0u8; 32];
            len[31] = 7;
            buf.extend_from_slice(&len);
            let mut data = [0u8; 32];
            data[..7].copy_from_slice(b"VERIFY:");
            buf.extend_from_slice(&data);
            let mut h2 = Keccak256::new();
            h2.update(&buf);
            format!("0x{}", hex::encode(h2.finalize()))
        };
        for label in ["boarding_intake", "grooming_intake", "daycare_access", ""] {
            assert_eq!(
                verify_whitelist_key_hex(label.to_string()),
                reference(label),
                "verify_whitelist_key_hex parity for label {label:?}"
            );
        }
    }
}
