//! Pure/behavioral unit coverage for `dns.rs::expected_txt` and the `crypto.rs` crypto-shred vault.
//!
//! These exercise the off-network, deterministic surface only: the canonical DNS TXT challenge string
//! and the `MemVault` seal/open/shred contract (incl. the JSON convenience wrappers and per-record DEK
//! isolation). Behavior-preserving - asserts the existing contract, changes no crypto.

use admin_api::crypto::{open_json, seal_json, CryptoError, KeyVault, MemVault, Sealed};
use admin_api::dns::expected_txt;

// --------------------------------------------------------------------------------------------
// expected_txt - the canonical `dogtag-verify=<documentStore>` challenge (lower-cased).
// --------------------------------------------------------------------------------------------

#[test]
fn expected_txt_is_prefixed_and_lowercased() {
    assert_eq!(
        expected_txt("0xABCdef123"),
        "dogtag-verify=0xabcdef123",
        "document store is lower-cased so checks are case-insensitive"
    );
}

#[test]
fn expected_txt_empty_store_still_has_prefix() {
    assert_eq!(expected_txt(""), "dogtag-verify=");
}

// --------------------------------------------------------------------------------------------
// MemVault - seal/open round-trip, crypto-shred, per-record DEK isolation, tamper detection.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn seal_open_roundtrip_recovers_plaintext() {
    let vault = MemVault::new();
    let sealed = vault.seal(b"owner pii").await.unwrap();
    assert_eq!(vault.open(&sealed).await.unwrap(), b"owner pii");
}

#[tokio::test]
async fn each_seal_allocates_a_distinct_dek() {
    let vault = MemVault::new();
    let a = vault.seal(b"same plaintext").await.unwrap();
    let b = vault.seal(b"same plaintext").await.unwrap();
    assert_ne!(a.dek_id, b.dek_id, "per-record DEK: ids never collide");
    // identical plaintext under distinct DEK + random nonce -> distinct ciphertext.
    assert_ne!(a.ct, b.ct, "no deterministic ciphertext across records");
}

#[tokio::test]
async fn shredding_one_dek_leaves_other_records_decryptable() {
    let vault = MemVault::new();
    let a = vault.seal(b"record A").await.unwrap();
    let b = vault.seal(b"record B").await.unwrap();
    vault.shred(&a.dek_id).await;
    assert!(matches!(vault.open(&a).await, Err(CryptoError::KeyGone)));
    assert_eq!(
        vault.open(&b).await.unwrap(),
        b"record B",
        "isolation: B unaffected"
    );
}

#[tokio::test]
async fn shred_is_idempotent() {
    let vault = MemVault::new();
    let s = vault.seal(b"x").await.unwrap();
    vault.shred(&s.dek_id).await;
    // a second shred of an already-gone DEK is a no-op, not a panic.
    vault.shred(&s.dek_id).await;
    assert!(!vault.has_dek(&s.dek_id).await);
}

#[tokio::test]
async fn open_unknown_dek_is_key_gone() {
    let vault = MemVault::new();
    let bogus = Sealed {
        dek_id: "never-allocated".to_string(),
        nonce: hex::encode([0u8; 12]),
        ct: hex::encode([1u8, 2, 3]),
    };
    assert!(matches!(
        vault.open(&bogus).await,
        Err(CryptoError::KeyGone)
    ));
}

#[tokio::test]
async fn tampered_ciphertext_fails_gcm_auth() {
    let vault = MemVault::new();
    let sealed = vault.seal(b"authentic").await.unwrap();
    // flip the last ciphertext byte; the GCM tag must reject it.
    let mut ct = hex::decode(&sealed.ct).unwrap();
    *ct.last_mut().unwrap() ^= 0x01;
    let tampered = Sealed {
        ct: hex::encode(ct),
        ..sealed
    };
    assert!(matches!(
        vault.open(&tampered).await,
        Err(CryptoError::Decrypt)
    ));
}

#[tokio::test]
async fn malformed_hex_fails_decrypt_not_panic() {
    let vault = MemVault::new();
    let sealed = vault.seal(b"x").await.unwrap();
    let bad_nonce = Sealed {
        nonce: "zzzz".to_string(),
        ..sealed
    };
    assert!(matches!(
        vault.open(&bad_nonce).await,
        Err(CryptoError::Decrypt)
    ));
}

// --------------------------------------------------------------------------------------------
// seal_json / open_json - the serde convenience wrappers preserve typed values.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn seal_json_open_json_roundtrips_typed_value() {
    let vault = MemVault::new();
    let value = vec!["jane".to_string(), "doe".to_string()];
    let sealed = seal_json(&vault, &value).await.unwrap();
    let back: Vec<String> = open_json(&vault, &sealed).await.unwrap();
    assert_eq!(back, value);
}

#[tokio::test]
async fn open_json_after_shred_is_key_gone() {
    let vault = MemVault::new();
    let sealed = seal_json(&vault, &"secret".to_string()).await.unwrap();
    vault.shred(&sealed.dek_id).await;
    let r: Result<String, CryptoError> = open_json(&vault, &sealed).await;
    assert!(matches!(r, Err(CryptoError::KeyGone)));
}
