//! PHASE-8 GATE — Erasure-unlinkability (BUILD_PROMPT §8 acceptance; impl §4.5/§11.6/§13.5).
//!
//! Right-to-erasure = crypto-shred. After `erase()`, the per-record DEK is DESTROYED and the
//! previously-decryptable salts/PII become PERMANENTLY UNRECOVERABLE — every ciphertext copy (DB,
//! oplog, WAL, backups, importer caches) decrypts to nothing. The on-chain salted commitment stays
//! but, with its salt unrecoverable, is now UNLINKABLE to the person (a documented mitigation, NOT a
//! regulator-blessed safe harbour — see docs/DPIA.md).
//!
//! This gate is assertion-focused: it proves DECRYPT FAILS after erase for BOTH credential salts and
//! `verification_records`. (The HTTP delete-request -> fulfill path is covered in central.rs `(d)`;
//! this isolates the unlinkability property at the crypto layer.)
//!
//! Hermetic: MemVault/MemStore, no chain/network.

mod common;

use common::*;

use admin_api::crypto::{seal_json, CryptoError, KeyVault, Sealed};
use admin_api::erasure::erase;
use admin_api::store::{Credential, VerificationRecord};

const OWNER: &str = "owner-erase-gate";

#[tokio::test]
async fn after_erase_dek_destroyed_and_salts_pii_unrecoverable() {
    let (state, _chain, vault, _biz) = hermetic_state();

    // ---- seed: an owner with a credential (carrying SALTS) + a verification_record, both sealed.
    let cred_salts = serde_json::json!({
        "data": { "microchip.code": "aabbccddeeff00112233445566778899:2:985141006580319" },
        "salts": ["aabbccddeeff00112233445566778899"]
    });
    let cred_sealed: Sealed = seal_json(&vault, &cred_salts).await.unwrap();
    let cred_dek = cred_sealed.dek_id.clone();
    state
        .store
        .put_credential(Credential {
            credential_id: "cred-1".into(),
            owner_id: OWNER.into(),
            dog_tag_id: "7".into(),
            root: "0x1111111111111111111111111111111111111111111111111111111111111111".into(),
            sealed_doc: cred_sealed.clone(),
        })
        .await;

    let vr_body = serde_json::json!({ "consent": { "subject": "0x...secret", "dogTagId": "7" }, "purpose": "VET_INTAKE" });
    let vr_sealed: Sealed = seal_json(&vault, &vr_body).await.unwrap();
    let vr_dek = vr_sealed.dek_id.clone();
    state
        .store
        .put_verification_record(VerificationRecord {
            record_id: "vr-1".into(),
            owner_id: OWNER.into(),
            dog_tag_id: "7".into(),
            purpose: "VET_INTAKE".into(),
            relayer: "0x00000000000000000000000000000000000000aa".into(),
            mode: "zk".into(),
            status: "recorded".into(),
            sealed: vr_sealed.clone(),
        })
        .await;

    // ---- PRE-erasure: everything is recoverable (the salt + PII decrypt cleanly).
    assert!(vault.has_dek(&cred_dek).await, "credential DEK present pre-erase");
    assert!(vault.has_dek(&vr_dek).await, "verification_record DEK present pre-erase");
    let pre_cred = vault.open(&cred_sealed).await.expect("credential salts decryptable pre-erase");
    assert!(
        String::from_utf8_lossy(&pre_cred).contains("985141006580319"),
        "PRE-erase: the salt+microchip are recoverable"
    );
    assert!(vault.open(&vr_sealed).await.is_ok(), "PRE-erase: verification_record decryptable");

    // ---- ERASE (scope: all -> credentials + verification_records).
    let (creds, vers, _receipts) = erase(&state, OWNER, "all").await;
    assert_eq!(creds, 1, "one credential shredded");
    assert_eq!(vers, 1, "one verification_record shredded");

    // ---- POST-erasure GATE: the DEKs are gone AND decrypt now FAILS (KeyGone) — UNRECOVERABLE.
    assert!(!vault.has_dek(&cred_dek).await, "credential DEK DESTROYED");
    assert!(!vault.has_dek(&vr_dek).await, "verification_record DEK DESTROYED");

    assert!(
        matches!(vault.open(&cred_sealed).await, Err(CryptoError::KeyGone)),
        "POST-erase: credential salts/PII MUST be unrecoverable (decrypt fails)"
    );
    assert!(
        matches!(vault.open(&vr_sealed).await, Err(CryptoError::KeyGone)),
        "POST-erase: verification_record consent/PII MUST be unrecoverable (decrypt fails)"
    );

    // ---- the rows themselves are deleted too (defence in depth).
    assert!(
        state.store.credentials_of_owner(OWNER).await.is_empty(),
        "credential rows deleted"
    );
    assert!(
        state.store.verification_records_of_owner(OWNER).await.is_empty(),
        "verification_record rows deleted"
    );
}
