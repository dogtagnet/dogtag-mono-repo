//! Crypto-shredding (impl §11.6 / §13.5): every off-chain record (owner PII, credential salts/data,
//! verification_records, consent receipts) is stored ENCRYPTED under a per-record DEK (AES-256-GCM).
//! Erasure = DESTROY THE DEK: every ciphertext copy (DB, oplog, WAL, backups, importer caches) becomes
//! permanently undecryptable. We delete the record too, but key-destruction is the load-bearing act,
//! because "scrub the salt from every replica" is only tractable as key destruction.
//!
//! A `KeyVault` holds DEKs by id. `seal` allocates a fresh DEK + encrypts; `open` decrypts; `shred`
//! destroys the DEK so `open` thereafter fails. `MemVault` is the in-memory implementation.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("DEK destroyed or unknown (crypto-shredded)")]
    KeyGone,
    #[error("decrypt failed")]
    Decrypt,
    #[error("encrypt failed")]
    Encrypt,
}

/// An opaque sealed blob: the DEK id + nonce + ciphertext. The plaintext is recoverable ONLY while
/// the vault still holds `dek_id`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sealed {
    pub dek_id: String,
    /// 12-byte AES-GCM nonce, hex.
    pub nonce: String,
    /// ciphertext (+ GCM tag), hex.
    pub ct: String,
}

/// A per-record key vault. DEKs live here; destroying one crypto-shreds every copy of its ciphertext.
#[async_trait]
pub trait KeyVault: Send + Sync {
    /// Allocate a fresh DEK, encrypt `plaintext` under it, return the sealed blob. The returned
    /// `dek_id` is owned by the record and shredded on erasure.
    async fn seal(&self, plaintext: &[u8]) -> Result<Sealed, CryptoError>;
    /// Decrypt a sealed blob — fails with `KeyGone` once the DEK is shredded.
    async fn open(&self, sealed: &Sealed) -> Result<Vec<u8>, CryptoError>;
    /// Destroy the DEK behind `dek_id` (irreversible). Idempotent.
    async fn shred(&self, dek_id: &str);
    /// True iff the DEK still exists (test/inspection helper).
    async fn has_dek(&self, dek_id: &str) -> bool;
}

#[derive(Default)]
struct MemVaultInner {
    /// dek_id -> 32-byte AES-256 key. Removing the entry == crypto-shred.
    deks: HashMap<String, [u8; 32]>,
}

#[derive(Clone, Default)]
pub struct MemVault {
    inner: Arc<RwLock<MemVaultInner>>,
}

impl MemVault {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl KeyVault for MemVault {
    async fn seal(&self, plaintext: &[u8]) -> Result<Sealed, CryptoError> {
        let mut key_bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut key_bytes);
        let mut nonce_bytes = [0u8; 12];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce_bytes);

        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
            .map_err(|_| CryptoError::Encrypt)?;

        let dek_id = uuid::Uuid::new_v4().to_string();
        self.inner.write().unwrap().deks.insert(dek_id.clone(), key_bytes);
        Ok(Sealed {
            dek_id,
            nonce: hex::encode(nonce_bytes),
            ct: hex::encode(ct),
        })
    }

    async fn open(&self, sealed: &Sealed) -> Result<Vec<u8>, CryptoError> {
        let key_bytes = self
            .inner
            .read()
            .unwrap()
            .deks
            .get(&sealed.dek_id)
            .copied()
            .ok_or(CryptoError::KeyGone)?;
        let nonce = hex::decode(&sealed.nonce).map_err(|_| CryptoError::Decrypt)?;
        let ct = hex::decode(&sealed.ct).map_err(|_| CryptoError::Decrypt)?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
        cipher
            .decrypt(Nonce::from_slice(&nonce), ct.as_ref())
            .map_err(|_| CryptoError::Decrypt)
    }

    async fn shred(&self, dek_id: &str) {
        // overwrite then remove (zeroize the key material before dropping the slot).
        let mut g = self.inner.write().unwrap();
        if let Some(k) = g.deks.get_mut(dek_id) {
            *k = [0u8; 32];
        }
        g.deks.remove(dek_id);
    }

    async fn has_dek(&self, dek_id: &str) -> bool {
        self.inner.read().unwrap().deks.contains_key(dek_id)
    }
}

/// Convenience: JSON-seal a serializable value.
pub async fn seal_json<T: Serialize>(vault: &dyn KeyVault, value: &T) -> Result<Sealed, CryptoError> {
    let bytes = serde_json::to_vec(value).map_err(|_| CryptoError::Encrypt)?;
    vault.seal(&bytes).await
}

/// Convenience: open a sealed blob into a deserializable value.
pub async fn open_json<T: for<'de> Deserialize<'de>>(
    vault: &dyn KeyVault,
    sealed: &Sealed,
) -> Result<T, CryptoError> {
    let bytes = vault.open(sealed).await?;
    serde_json::from_slice(&bytes).map_err(|_| CryptoError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn seal_open_then_shred() {
        let vault = MemVault::new();
        let sealed = vault.seal(b"owner pii: Jane Doe").await.unwrap();
        assert_eq!(vault.open(&sealed).await.unwrap(), b"owner pii: Jane Doe");
        assert!(vault.has_dek(&sealed.dek_id).await);
        vault.shred(&sealed.dek_id).await;
        assert!(!vault.has_dek(&sealed.dek_id).await);
        // the ciphertext copy is now permanently undecryptable.
        assert!(matches!(vault.open(&sealed).await, Err(CryptoError::KeyGone)));
    }
}
