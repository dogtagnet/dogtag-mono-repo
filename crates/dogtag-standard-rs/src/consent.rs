//! DogTag consent module — on-chain proof-of-verification consent artifact (impl §11.8/§11.9, §1.10).
//!
//! Byte-for-byte equivalent to `packages/dogtag-standard-ts/src/consent.ts` for the three commitments
//! the registry / prover sides need parity on:
//!   (1) EIP-712 typed-data digest (keccak256 — `_hashTypedDataV4`; the wallet ECDSA-signs it).
//!   (2) Poseidon nullifier = Poseidon(DS_NULLIFIER, dogTagId, purpose, relayer, subject, nonce).
//!   (3) EdDSA-BabyJubjub consent message M = Poseidon(dogTagId, purpose, relayer, subject, R, nonce).
//! plus keyHash = Poseidon(Ax, Ay).
//!
//! NOTE: EdDSA-BabyJubjub SIGNING is intentionally NOT implemented in Rust — it is a Phase-6 /
//! mobile (UniFFI) concern. Rust only computes the digest / nullifier / message / keyHash so the
//! registry and prover sides have full cross-language parity with the TS SDK (impl §9).

use ark_bn254::Fr;
use ark_ff::PrimeField;
use sha3::{Digest, Keccak256};

use crate::poseidon::{poseidon, to_be_bytes32, DS_NULLIFIER};

/// The default EIP-712 chainId (impl §11.8(a)).
pub const DOGTAG_CHAIN_ID: u64 = 135;

/// EIP-712 type string — field order MUST match the struct (impl §11.8(a)).
pub const VERIFICATION_CONSENT_TYPE_STRING: &str =
    "VerificationConsent(uint256 dogTagId,bytes32 recordType,bytes32 purpose,bytes32 credentialRoot,\
bytes32 challenge,address relayer,address subject,uint256 nonce,uint256 deadline)";

/// The EIP-712 EIP712Domain type string.
const EIP712_DOMAIN_TYPE_STRING: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";

/// The FINAL on-chain VerificationConsent (impl §11.9(a)) — NINE fields in this exact order.
///
/// uint256 fields (dogTagId/nonce/deadline) and bytes32 fields (recordType/purpose/credentialRoot/
/// challenge) are stored as 32-byte big-endian arrays; addresses as 20-byte arrays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationConsent {
    pub dog_tag_id: [u8; 32],
    pub record_type: [u8; 32],
    pub purpose: [u8; 32],
    pub credential_root: [u8; 32],
    pub challenge: [u8; 32],
    pub relayer: [u8; 20],
    pub subject: [u8; 20],
    pub nonce: [u8; 32],
    pub deadline: [u8; 32],
}

fn keccak(bytes: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(bytes);
    h.finalize().into()
}

/// keccak256 of the EIP-712 type string (impl §11.8(a)).
pub fn verification_consent_typehash() -> [u8; 32] {
    keccak(VERIFICATION_CONSENT_TYPE_STRING.as_bytes())
}

/// Left-pad a 20-byte address into a 32-byte abi.encode word.
fn address_word(addr: &[u8; 20]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[12..].copy_from_slice(addr);
    out
}

/// EIP-712 domainSeparator for the DogTag domain (impl §11.8(a)).
pub fn domain_separator(verifying_contract: [u8; 20], chain_id: u64) -> [u8; 32] {
    let domain_typehash = keccak(EIP712_DOMAIN_TYPE_STRING.as_bytes());
    let name_hash = keccak(b"DogTag");
    let version_hash = keccak(b"1");

    let mut chain_word = [0u8; 32];
    chain_word[24..].copy_from_slice(&chain_id.to_be_bytes());

    let mut buf = Vec::with_capacity(32 * 5);
    buf.extend_from_slice(&domain_typehash);
    buf.extend_from_slice(&name_hash);
    buf.extend_from_slice(&version_hash);
    buf.extend_from_slice(&chain_word);
    buf.extend_from_slice(&address_word(&verifying_contract));
    keccak(&buf)
}

/// keccak256(abi.encode(typehash, ...9 fields...)) — the EIP-712 struct hash (impl §11.9(a)).
pub fn struct_hash(consent: &VerificationConsent) -> [u8; 32] {
    let mut buf = Vec::with_capacity(32 * 10);
    buf.extend_from_slice(&verification_consent_typehash());
    buf.extend_from_slice(&consent.dog_tag_id);
    buf.extend_from_slice(&consent.record_type);
    buf.extend_from_slice(&consent.purpose);
    buf.extend_from_slice(&consent.credential_root);
    buf.extend_from_slice(&consent.challenge);
    buf.extend_from_slice(&address_word(&consent.relayer));
    buf.extend_from_slice(&address_word(&consent.subject));
    buf.extend_from_slice(&consent.nonce);
    buf.extend_from_slice(&consent.deadline);
    keccak(&buf)
}

/// The EIP-712 digest (`_hashTypedDataV4`, impl §11.8): keccak256(0x1901 || domainSep || structHash).
pub fn hash_typed_consent(consent: &VerificationConsent, verifying_contract: [u8; 20], chain_id: u64) -> [u8; 32] {
    let ds = domain_separator(verifying_contract, chain_id);
    let sh = struct_hash(consent);
    let mut buf = Vec::with_capacity(2 + 64);
    buf.extend_from_slice(&[0x19, 0x01]);
    buf.extend_from_slice(&ds);
    buf.extend_from_slice(&sh);
    keccak(&buf)
}

// ----------------------------------------------------------------------------------------------
// Poseidon nullifier / eddsa message / keyHash — over BN254 Fr (parity with poseidon-lite TS leg).
// ----------------------------------------------------------------------------------------------

/// A 32-byte big-endian word reduced into [0, r) (purpose / roots / uint256 fields).
///
/// `from_be_bytes_mod_order` reduces mod r — correct here because these inputs are semantically
/// field elements (purpose is reduced per §11.9(b); dogTagId/nonce are < r in practice but we
/// reduce defensively to match the TS `% FIELD_P`).
fn field_mod_r(word: &[u8; 32]) -> Fr {
    Fr::from_be_bytes_mod_order(word)
}

/// uint160 field element of an address (2^160 < r, so the value is exact — no wraparound).
fn address_field(addr: &[u8; 20]) -> Fr {
    Fr::from_be_bytes_mod_order(addr)
}

/// The consent nullifier (impl §11.9(b)):
/// Poseidon(DS_NULLIFIER=4, dogTagId, purpose mod r, uint160(relayer), uint160(subject), nonce).
pub fn consent_nullifier(consent: &VerificationConsent) -> [u8; 32] {
    let m = poseidon(&[
        Fr::from(DS_NULLIFIER),
        field_mod_r(&consent.dog_tag_id),
        field_mod_r(&consent.purpose),
        address_field(&consent.relayer),
        address_field(&consent.subject),
        field_mod_r(&consent.nonce),
    ]);
    to_be_bytes32(&m)
}

/// The EdDSA consent message M (impl §11.9(d) / §1.10):
/// Poseidon(dogTagId, purpose, relayer, subject, credentialRoot(=R), nonce) — 6 inputs, NO DS tag.
pub fn eddsa_consent_message(consent: &VerificationConsent) -> Fr {
    poseidon(&[
        field_mod_r(&consent.dog_tag_id),
        field_mod_r(&consent.purpose),
        address_field(&consent.relayer),
        address_field(&consent.subject),
        field_mod_r(&consent.credential_root),
        field_mod_r(&consent.nonce),
    ])
}

/// keyHash = Poseidon(Ax, Ay) -> canonical 32-byte big-endian (impl §1.10).
pub fn key_hash(ax: Fr, ay: Fr) -> [u8; 32] {
    to_be_bytes32(&poseidon(&[ax, ay]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor() -> VerificationConsent {
        let mut purpose = [0u8; 32];
        purpose[31] = 7;
        VerificationConsent {
            dog_tag_id: {
                let mut a = [0u8; 32];
                a[31] = 42;
                a
            },
            record_type: [0u8; 32],
            purpose,
            credential_root: [0u8; 32],
            challenge: [0u8; 32],
            relayer: [0x11u8; 20],
            subject: [0x22u8; 20],
            nonce: {
                let mut a = [0u8; 32];
                a[31] = 99;
                a
            },
            deadline: [0u8; 32],
        }
    }

    #[test]
    fn nullifier_matches_poseidon_gate_anchor() {
        let got = format!("0x{}", hex::encode(consent_nullifier(&anchor())));
        assert_eq!(
            got, "0x055077ae7cbe2e123ad701247450fa222fabe3d3b399bfd40f416da970cfca11",
            "consent nullifier must equal poseidon-vectors.json nullifier_basic"
        );
    }
}
