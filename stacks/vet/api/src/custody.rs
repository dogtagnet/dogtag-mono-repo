//! HD custody (impl §3.1 / §11.4 / §1.8): genesis (24-word BIP-39), age-encrypt the mnemonic under a
//! scrypt passphrase, persist the encrypted blob + keystore_meta (addresses/labels only). `unlock`
//! decrypts into a `secrecy::SecretBox` (zeroized on drop), caches Alloy signers; `accounts` derives the
//! next index m/44'/60'/0'/0/{i} via alloy-signer-local MnemonicBuilder.

use std::io::{Read, Write};
use std::sync::{Arc, RwLock};

use alloy::signers::local::coins_bip39::English;
use alloy::signers::local::{MnemonicBuilder, PrivateKeySigner};
use secrecy::SecretString;
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum CustodyError {
    #[error("custody not in the right state: {0}")]
    State(String),
    #[error("challenge words do not match")]
    BadChallenge,
    #[error("decrypt failed (wrong passphrase?)")]
    Decrypt,
    #[error("not unlocked")]
    Locked,
    #[error("{0}")]
    Other(String),
}

/// The in-memory genesis stash (the freshly generated mnemonic awaiting backup confirmation).
#[derive(Clone)]
pub struct GenesisStash {
    pub mnemonic: Zeroizing<String>,
    pub challenge_indices: Vec<usize>,
}

/// Generate a fresh 24-word (256-bit) BIP-39 mnemonic + 3 random challenge positions.
pub fn genesis_generate() -> Result<GenesisStash, CustodyError> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut entropy = [0u8; 32]; // 256 bits -> 24 words
    rng.fill(&mut entropy);
    let m = bip39::Mnemonic::from_entropy(&entropy)
        .map_err(|e| CustodyError::Other(format!("mnemonic gen: {e}")))?;
    let phrase = m.words().collect::<Vec<_>>().join(" ");

    // 3 distinct random positions in [0, 24).
    let mut idx = std::collections::BTreeSet::new();
    while idx.len() < 3 {
        idx.insert(rng.gen_range(0..24usize));
    }
    Ok(GenesisStash {
        mnemonic: Zeroizing::new(phrase),
        challenge_indices: idx.into_iter().collect(),
    })
}

/// Words of a phrase as a vec.
pub fn words_of(phrase: &str) -> Vec<String> {
    phrase.split_whitespace().map(|s| s.to_string()).collect()
}

/// Derive the Alloy local signer at account `index` (path m/44'/60'/0'/0/{index}).
pub fn derive_account(phrase: &str, index: u32) -> Result<PrivateKeySigner, CustodyError> {
    MnemonicBuilder::<English>::default()
        .phrase(phrase.to_string())
        .index(index)
        .map_err(|e| CustodyError::Other(format!("derive index: {e}")))?
        .build()
        .map_err(|e| CustodyError::Other(format!("derive build: {e}")))
}

/// age-encrypt the mnemonic with a scrypt passphrase. Returns ASCII-armored ciphertext.
pub fn encrypt_seed(phrase: &str, passphrase: &str) -> Result<Vec<u8>, CustodyError> {
    let encryptor = age::Encryptor::with_user_passphrase(SecretString::from(passphrase.to_string()));
    let mut out = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut out)
        .map_err(|e| CustodyError::Other(format!("age wrap: {e}")))?;
    writer
        .write_all(phrase.as_bytes())
        .map_err(|e| CustodyError::Other(format!("age write: {e}")))?;
    writer
        .finish()
        .map_err(|e| CustodyError::Other(format!("age finish: {e}")))?;
    Ok(out)
}

/// age-decrypt the mnemonic back into a zeroizing string.
pub fn decrypt_seed(ciphertext: &[u8], passphrase: &str) -> Result<Zeroizing<String>, CustodyError> {
    let decryptor = age::Decryptor::new(ciphertext).map_err(|_| CustodyError::Decrypt)?;
    let identity = age::scrypt::Identity::new(SecretString::from(passphrase.to_string()));
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| CustodyError::Decrypt)?;
    let mut plaintext = Zeroizing::new(String::new());
    reader
        .read_to_string(&mut plaintext)
        .map_err(|_| CustodyError::Decrypt)?;
    Ok(plaintext)
}

/// The unlocked custody: the seed phrase held in a SecretBox-like Zeroizing string + cached signers.
#[derive(Clone, Default)]
pub struct Custody {
    inner: Arc<RwLock<CustodyInner>>,
}

#[derive(Default)]
struct CustodyInner {
    /// Pending genesis stash (between start and confirm).
    stash: Option<GenesisStash>,
    /// The unlocked mnemonic (zeroized on drop). None == locked.
    unlocked: Option<Zeroizing<String>>,
    /// cached signers by index.
    signers: std::collections::HashMap<u32, PrivateKeySigner>,
}

impl Custody {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stash_genesis(&self, stash: GenesisStash) {
        self.inner.write().unwrap().stash = Some(stash);
    }

    pub fn take_stash(&self) -> Option<GenesisStash> {
        self.inner.read().unwrap().stash.clone()
    }

    pub fn clear_stash(&self) {
        self.inner.write().unwrap().stash = None;
    }

    /// Load the decrypted phrase, derive+cache account 0, mark unlocked.
    pub fn unlock_with(&self, phrase: Zeroizing<String>) -> Result<(), CustodyError> {
        let signer0 = derive_account(&phrase, 0)?;
        let mut g = self.inner.write().unwrap();
        g.signers.insert(0, signer0);
        g.unlocked = Some(phrase);
        Ok(())
    }

    pub fn is_unlocked(&self) -> bool {
        self.inner.read().unwrap().unlocked.is_some()
    }

    /// Derive (and cache) the signer at `index`, requiring the custody to be unlocked.
    pub fn signer(&self, index: u32) -> Result<PrivateKeySigner, CustodyError> {
        {
            let g = self.inner.read().unwrap();
            if let Some(s) = g.signers.get(&index) {
                return Ok(s.clone());
            }
        }
        let mut g = self.inner.write().unwrap();
        let phrase = g.unlocked.as_ref().ok_or(CustodyError::Locked)?.clone();
        let s = derive_account(phrase.expose_secret(), index)?;
        g.signers.insert(index, s.clone());
        Ok(s)
    }

    /// Export the raw secp256k1 private key for the signer at `index` (to register into the chain
    /// client at unlock). Stays in-process; never persisted or logged.
    pub fn private_key(&self, index: u32) -> Result<[u8; 32], CustodyError> {
        let s = self.signer(index)?;
        Ok(s.to_bytes().into())
    }

    /// The address of the active signer (account 0 by default).
    pub fn active_address(&self) -> Result<String, CustodyError> {
        let s = self.signer(0)?;
        Ok(format!("{:#x}", s.address()))
    }

    /// All currently-cached account addresses (index, address).
    pub fn cached_accounts(&self) -> Vec<(u32, String)> {
        let g = self.inner.read().unwrap();
        let mut v: Vec<(u32, String)> = g
            .signers
            .iter()
            .map(|(i, s)| (*i, format!("{:#x}", s.address())))
            .collect();
        v.sort_by_key(|(i, _)| *i);
        v
    }
}

// secrecy SecretString exposes via expose_secret; Zeroizing<String> derefs to &str.
trait PhraseExpose {
    fn expose_secret(&self) -> &str;
}
impl PhraseExpose for Zeroizing<String> {
    fn expose_secret(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genesis_encrypt_decrypt_roundtrip() {
        let stash = genesis_generate().unwrap();
        assert_eq!(words_of(&stash.mnemonic).len(), 24);
        assert_eq!(stash.challenge_indices.len(), 3);
        let ct = encrypt_seed(&stash.mnemonic, "hunter2-passphrase").unwrap();
        let pt = decrypt_seed(&ct, "hunter2-passphrase").unwrap();
        assert_eq!(*pt, *stash.mnemonic);
        // wrong passphrase fails.
        assert!(decrypt_seed(&ct, "wrong").is_err());
    }

    #[test]
    fn derive_is_deterministic() {
        let phrase =
            "test test test test test test test test test test test junk";
        let a = derive_account(phrase, 0).unwrap();
        let b = derive_account(phrase, 0).unwrap();
        assert_eq!(a.address(), b.address());
        // anvil account 0 for this mnemonic is well-known.
        assert_eq!(
            format!("{:#x}", a.address()),
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
        );
    }
}
