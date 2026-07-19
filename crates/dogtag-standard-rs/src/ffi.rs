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
use crate::merkle::{build_merkle, hash_node, verify_inclusion, ProofStep};
use crate::types::{TypeTag, TypedScalar};
use crate::verify::{check_integrity, FragmentState};
use crate::wrap::{
    from_hex32, obfuscate, scalar_from_packed, wrap_document, IssuerMeta, WrappedDoc,
};

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
    let type_tag =
        TypeTag::from_u8(tag).ok_or_else(|| FfiError::Invalid(format!("unknown tag {tag}")))?;
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

/// hashNode: the commutative internal-node hash `Poseidon3(DS_NODE, min(a,b), max(a,b))` over two
/// 0x.. 32-byte field hexes -> the 0x.. 32-byte node hex. The primitive a foreign `Sibling | Promote`
/// inclusion verifier folds with (Swift/Kotlin) so the Poseidon parameter set stays pinned in Rust.
#[uniffi::export]
pub fn hash_node_hex(a_hex: String, b_hex: String) -> Result<String, FfiError> {
    let a = from_hex32(&a_hex)?;
    let b = from_hex32(&b_hex)?;
    Ok(to_hex32(&hash_node(a, b)))
}

/// verifyInclusionProof: the NORMATIVE DSDP §2.3 disclosed-leaf check over the FFI boundary.
///
/// RECOMPUTES the leaf hash from `(key_path, salt, tag, value)` under `DS_LEAF` (Poseidon5) — never
/// trusting a supplied leaf hash — then folds the root-ward `Sibling | Promote` proof and returns
/// whether it equals `root_hex`. `proof_steps` encodes each step as either the literal `"promote"`
/// (a lone-odd-node pass-through) or a 0x.. 32-byte sibling hex. This is the canonical reference the
/// foreign verifiers cross-check against (their own fold via [`hash_leaf_hex`] + [`hash_node_hex`]
/// must agree with this).
#[uniffi::export]
pub fn verify_inclusion_proof_hex(
    key_path: String,
    salt_hex: String,
    tag: u8,
    value: String,
    proof_steps: Vec<String>,
    root_hex: String,
) -> Result<bool, FfiError> {
    let salt = decode_salt(&salt_hex)?;
    let type_tag = TypeTag::from_u8(tag)
        .ok_or_else(|| FfiError::Invalid(format!("unknown tag {tag}")))?;
    let scalar: TypedScalar = scalar_from_packed(type_tag, &value)?;
    let mut steps: Vec<ProofStep> = Vec::with_capacity(proof_steps.len());
    for s in &proof_steps {
        if s.eq_ignore_ascii_case("promote") {
            steps.push(ProofStep::Promote);
        } else {
            steps.push(ProofStep::Sibling(from_hex32(s)?));
        }
    }
    let root = from_hex32(&root_hex)?;
    Ok(verify_inclusion(&key_path, &salt, &scalar, &steps, root)?)
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
    let typed: Value = serde_json::from_str(&typed_credential_json)
        .map_err(|e| err(format!("bad credential json: {e}")))?;
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
    let doc: WrappedDoc = serde_json::from_str(&wrapped_doc_json)
        .map_err(|e| err(format!("bad wrapped doc json: {e}")))?;
    let (state, _root) = check_integrity(&doc);
    Ok(match state {
        FragmentState::Valid => "VALID".to_string(),
        _ => "INVALID".to_string(),
    })
}

/// obfuscate — the merkle selective-disclosure primitive (mirror `wrap::obfuscate`). Moves each
/// named field's leaf hash into `privacy.obfuscated[]` and drops its cleartext, leaving the Merkle
/// root (== the on-chain root R) UNCHANGED. This is what lets the phone produce a PII-free
/// presentation copy LOCALLY: the holder hides Section-A person-PII leaves while the tree still
/// rebuilds to R, so every disclosed (Section B/C/validity/receiptId) leaf stays verifiable on-chain.
///
/// `key_paths` are dotted leaf key-paths as they appear in `data` (e.g.
/// `credentialSubject.importer.idNumber`). `credentialSubject.dogTagId` is the one leaf that must
/// never be obfuscated (`verify.rs` rejects it as the non-obfuscatable SBT binding); obfuscating a
/// missing key errors. Returns the redacted WrappedDoc JSON, which `verify_integrity` still passes.
#[uniffi::export]
pub fn obfuscate_document_json(
    wrapped_doc_json: String,
    key_paths: Vec<String>,
) -> Result<String, FfiError> {
    let doc: WrappedDoc = serde_json::from_str(&wrapped_doc_json)
        .map_err(|e| err(format!("bad wrapped doc json: {e}")))?;
    let out = obfuscate(&doc, &key_paths)?;
    serde_json::to_string(&out).map_err(|e| err(format!("serialize: {e}")))
}

// --------------------------------------------------------------------------------------------
// Consent commitments (mirror consent.rs) — digest / nullifier / message / keyHash for parity.
// --------------------------------------------------------------------------------------------

/// keccak256 of the EIP-712 VerificationConsent type string (0x.. 32-byte hex).
#[uniffi::export]
pub fn verification_consent_typehash_hex() -> String {
    format!(
        "0x{}",
        hex::encode(crate::consent::verification_consent_typehash())
    )
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
    Ok(format!(
        "0x{}",
        hex::encode(crate::consent::consent_nullifier(&c))
    ))
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
    Ok(format!(
        "0x{}",
        hex::encode(crate::consent::key_hash(ax, ay))
    ))
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
    let digest = crate::consent::bind_consent_key_digest(
        registry,
        &key_hash,
        &wallet,
        &nonce_word,
        chain_id,
    );
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

/// Derive the **wallet-level** BabyJubjub consent key from a hex seed (any length) - one key per
/// wallet, NOT per tag. The seed is wrapped in a distinct domain from the secp256k1 wallet path
/// (§6) before BLAKE-512, so the two keys are independent. Returns the 32-byte private key +
/// public point (Ax, Ay) + keyHash.
///
/// # Callers: this is the Level-A key, not the profile-tree key
///
/// Use this ONLY for the Level-A path (`verification.circom` + the per-wallet
/// `ConsentKeyRegistry.keyOf` bind). The Level-B `owner.consentKey` leaf uses a **per-tag** key
/// derived from `(seed, dogTagId)`; you get it from
/// [`build_profile_tree_hex`]'s `consentPrvHex` (or implicitly via `prove_consent`, which derives
/// it internally from the `dogTagId` it already receives). See
/// [`crate::eddsa::derive_babyjub_consent_key_per_tag`].
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
        key_hash_hex: format!(
            "0x{}",
            hex::encode(crate::consent::key_hash(key.ax, key.ay))
        ),
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

// ---------------------------------------------------------------------------------------------
// Level-B M5 - device-side per-tag profile tree + recoverable owner-secret.
// ---------------------------------------------------------------------------------------------

/// A disclosable attribute leaf crossing the FFI boundary. `tag`+`value` are reconstructed into a
/// `TypedScalar` exactly like `hash_leaf_hex` does. `salt_hex` is 16 bytes.
#[derive(uniffi::Record)]
pub struct AttributeLeafFfi {
    pub key_path: String,
    pub salt_hex: String,
    pub tag: u8,
    pub value: String,
}

/// A device-built per-tag profile tree.
///
/// Everything here except `root_hex` is OWNER-PRIVATE and must never be transmitted: only
/// `root_hex` (`R`) is handed to the issuer, which seals it as `profileRoot(dogTagId)`.
#[derive(uniffi::Record)]
pub struct ProfileTreeFfi {
    /// `R` - the value the issuer seals as `profileRoot(dogTagId)`.
    pub root_hex: String,
    /// The nullifier secret. Regenerable from the wallet seed; keep it device-local and never send
    /// or back it up separately.
    pub owner_secret_hex: String,
    pub ax_hex: String,
    pub ay_hex: String,
    pub consent_prv_hex: String,
    pub key_hash_hex: String,
    pub owner_salt_hex: String,
    pub key_salt_hex: String,
    pub secret_salt_hex: String,
    pub owner_address_leaf_hex: String,
    pub consent_key_leaf_hex: String,
    pub owner_secret_leaf_hex: String,
}

fn attr_leaves_from_ffi(
    attributes: &[AttributeLeafFfi],
) -> Result<Vec<crate::profile_tree::AttributeLeaf>, FfiError> {
    attributes
        .iter()
        .map(|a| {
            let salt = decode_word::<{ crate::profile_tree::SALT_LEN }>("salt", &a.salt_hex)?;
            let type_tag = TypeTag::from_u8(a.tag)
                .ok_or_else(|| FfiError::Invalid(format!("unknown tag {}", a.tag)))?;
            Ok(crate::profile_tree::AttributeLeaf {
                key_path: a.key_path.clone(),
                salt,
                value: scalar_from_packed(type_tag, &a.value)?,
            })
        })
        .collect()
}

/// Derive the owner-secret from the wallet seed, bound to `dogTagId` - the RECOVERABLE nullifier
/// secret (`profile_tree::derive_owner_secret`).
///
/// Deterministic: restoring the seed regenerates the same secret. Rebuilding the same tree and `R`
/// additionally requires the original owner address plus the exact attribute values and salts.
/// `dog_tag_id_hex` is the canonical dogTagId field (what `dogTagIdFieldHex` returns).
///
/// The result is owner-private and MUST NOT be transmitted: a server that learned it could link
/// nullifiers back to the owner, defeating the owner-unlinkable model.
#[uniffi::export]
pub fn derive_owner_secret_hex(seed_hex: String, dog_tag_id_hex: String) -> Result<String, FfiError> {
    let s = seed_hex.strip_prefix("0x").unwrap_or(&seed_hex);
    let seed = hex::decode(s).map_err(|e| err(format!("bad seed hex: {e}")))?;
    let dog_tag_id = from_hex32(&dog_tag_id_hex).map_err(FfiError::from)?;
    let secret = crate::profile_tree::derive_owner_secret(&seed, dog_tag_id)?;
    Ok(to_hex32(&secret))
}

/// Build the per-tag Merkle tree ON THE DEVICE (`profile_tree::build_profile_tree`) and return `R`
/// plus the owner-private witness.
///
/// This is spec §"Issuance" step 2: *the owner's app builds the tree locally (owner-address,
/// consent-key, owner-secret, attributes as salted leaves), computes `R`*. Hand the issuer ONLY
/// `root_hex`.
///
/// `owner_address_hex` is the 20-byte wallet address; `dog_tag_id_hex` the canonical dogTagId field.
#[uniffi::export]
pub fn build_profile_tree_hex(
    seed_hex: String,
    dog_tag_id_hex: String,
    owner_address_hex: String,
    attributes: Vec<AttributeLeafFfi>,
) -> Result<ProfileTreeFfi, FfiError> {
    let s = seed_hex.strip_prefix("0x").unwrap_or(&seed_hex);
    let seed = hex::decode(s).map_err(|e| err(format!("bad seed hex: {e}")))?;
    let dog_tag_id = from_hex32(&dog_tag_id_hex).map_err(FfiError::from)?;
    let owner_address = decode_word::<20>("ownerAddress", &owner_address_hex)?;
    let attrs = attr_leaves_from_ffi(&attributes)?;

    let t = crate::profile_tree::build_profile_tree(&seed, dog_tag_id, &owner_address, &attrs)?;
    Ok(ProfileTreeFfi {
        root_hex: to_hex32(&t.root),
        owner_secret_hex: to_hex32(&t.owner_secret),
        ax_hex: to_hex32(&t.ax),
        ay_hex: to_hex32(&t.ay),
        consent_prv_hex: format!("0x{}", hex::encode(t.consent_prv)),
        key_hash_hex: format!("0x{}", hex::encode(crate::consent::key_hash(t.ax, t.ay))),
        owner_salt_hex: format!("0x{}", hex::encode(t.owner_salt)),
        key_salt_hex: format!("0x{}", hex::encode(t.key_salt)),
        secret_salt_hex: format!("0x{}", hex::encode(t.secret_salt)),
        owner_address_leaf_hex: to_hex32(&t.owner_address_leaf),
        consent_key_leaf_hex: to_hex32(&t.consent_key_leaf),
        owner_secret_leaf_hex: to_hex32(&t.owner_secret_leaf),
    })
}

#[cfg(test)]
mod profile_tree_ffi_tests {
    use super::*;

    const SEED_HEX: &str = "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const ADDR_HEX: &str = "0x00000000000000000000000000000000deadbeef";

    fn tag_id_hex() -> String {
        to_hex32(&ark_bn254::Fr::from(424242u64))
    }

    fn attrs() -> Vec<AttributeLeafFfi> {
        vec![AttributeLeafFfi {
            key_path: "credentialSubject.name".to_string(),
            salt_hex: "0x0102030405060708090a0b0c0d0e0f10".to_string(),
            tag: TypeTag::String as u8,
            value: "Rex".to_string(),
        }]
    }

    /// The FFI is the device's entry point, so the recoverability round-trip must hold ACROSS the
    /// boundary too - not just in the core.
    #[test]
    fn ffi_round_trips_the_root_and_secret_from_the_seed() {
        let a = build_profile_tree_hex(
            SEED_HEX.to_string(),
            tag_id_hex(),
            ADDR_HEX.to_string(),
            attrs(),
        )
        .unwrap();
        let b = build_profile_tree_hex(
            SEED_HEX.to_string(),
            tag_id_hex(),
            ADDR_HEX.to_string(),
            attrs(),
        )
        .unwrap();

        assert_eq!(a.root_hex, b.root_hex, "same seed must rebuild the same R");
        assert_eq!(a.owner_secret_hex, b.owner_secret_hex);
        // The standalone derivation agrees with the one the builder embedded.
        assert_eq!(
            derive_owner_secret_hex(SEED_HEX.to_string(), tag_id_hex()).unwrap(),
            a.owner_secret_hex
        );
    }

    #[test]
    fn ffi_rejects_malformed_inputs() {
        assert!(build_profile_tree_hex(
            "0xzz".to_string(),
            tag_id_hex(),
            ADDR_HEX.to_string(),
            attrs()
        )
        .is_err());
        // A 19-byte address must not silently pass.
        assert!(build_profile_tree_hex(
            SEED_HEX.to_string(),
            tag_id_hex(),
            "0x000000000000000000000000000000deadbeef".to_string(),
            attrs()
        )
        .is_err());
        assert!(derive_owner_secret_hex("".to_string(), tag_id_hex()).is_err());
    }
}

#[cfg(test)]
mod obfuscate_ffi_tests {
    use super::*;

    /// The mobile holder flow: wrap a credential, obfuscate a Section-A PII leaf LOCALLY, and assert
    /// (a) integrity still verifies (the tree rebuilds to the same root R), (b) the cleartext is gone,
    /// and (c) it lands in `privacy.obfuscated[]`. This is the PII-free presentation copy the phone
    /// produces without any server round-trip.
    #[test]
    fn obfuscate_hides_leaf_but_keeps_integrity() {
        let credential = r#"{
            "credentialSubject": {
                "dogTagId": {"tag": 3, "value": "7"},
                "receiptId": {"tag": 2, "value": "9RVBXK8AFQ2C"},
                "importer": {
                    "firstName": {"tag": 2, "value": "Dominic"},
                    "idNumber": {"tag": 2, "value": "887524355"}
                },
                "animal": {"name": {"tag": 2, "value": "Blaze"}}
            }
        }"#;
        let issuer = r#"{
            "name": "Example Competent Authority",
            "domain": "gov.example",
            "documentStore": "0x0000000000000000000000000000000000000001",
            "recordType": "TRAVEL_CLEARANCE"
        }"#;

        let wrapped = wrap_document_json(credential.to_string(), issuer.to_string()).unwrap();
        assert_eq!(verify_integrity(wrapped.clone()).unwrap(), "VALID");

        let redacted = obfuscate_document_json(
            wrapped,
            vec!["credentialSubject.importer.idNumber".to_string()],
        )
        .unwrap();

        // Integrity survives obfuscation — the on-chain root R is unchanged.
        assert_eq!(verify_integrity(redacted.clone()).unwrap(), "VALID");
        // The PII cleartext is dropped and a hash moved to privacy.obfuscated[].
        assert!(
            !redacted.contains("887524355"),
            "idNumber cleartext must be gone"
        );
        let v: Value = serde_json::from_str(&redacted).unwrap();
        assert_eq!(v["privacy"]["obfuscated"].as_array().unwrap().len(), 1);
        // A public/disclosed leaf is untouched.
        assert!(
            redacted.contains("Blaze"),
            "disclosed animal leaf stays visible"
        );
    }

    /// Obfuscating an absent key path is an error surfaced across the FFI (not a silent no-op).
    #[test]
    fn obfuscate_missing_field_errors() {
        let credential = r#"{"credentialSubject": {"dogTagId": {"tag": 3, "value": "7"}}}"#;
        let issuer = r#"{"name":"A","domain":"a.example","documentStore":"0x0000000000000000000000000000000000000001","recordType":"TRAVEL_CLEARANCE"}"#;
        let wrapped = wrap_document_json(credential.to_string(), issuer.to_string()).unwrap();
        let res = obfuscate_document_json(wrapped, vec!["credentialSubject.nope".to_string()]);
        assert!(res.is_err());
    }
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
