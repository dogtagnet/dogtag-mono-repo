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

// --------------------------------------------------------------------------------------------
// On-disk seal persistence (so custody survives a backend restart). We persist ONLY the
// age-encrypted seed ciphertext (already armored ASCII) + the non-secret keystore meta
// (addresses/labels/state). NEVER the plaintext mnemonic, NEVER the passphrase. On restart the
// store is hydrated to "initialized but locked"; the operator still supplies the passphrase to
// decrypt the disk-loaded blob -> the SAME signer.
// --------------------------------------------------------------------------------------------

/// The serializable on-disk seal: base64 of the age-armored ciphertext + the keystore meta. This
/// captures exactly what the store needs to later `unlock` (decrypt `sealed_b64`) and report
/// accounts (`meta`).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SealFile {
    /// base64(standard) of the age-armored seed ciphertext (the `encrypted_seed` blob).
    pub sealed_b64: String,
    /// non-secret keystore meta: derived account addresses/labels + state.
    pub meta: crate::store::KeystoreMeta,
}

/// Serialize `(ciphertext, meta)` to the on-disk seal JSON bytes.
pub fn seal_to_json(encrypted_seed: &[u8], meta: &crate::store::KeystoreMeta) -> Result<Vec<u8>, CustodyError> {
    use base64::Engine as _;
    let sf = SealFile {
        sealed_b64: base64::engine::general_purpose::STANDARD.encode(encrypted_seed),
        meta: meta.clone(),
    };
    serde_json::to_vec_pretty(&sf).map_err(|e| CustodyError::Other(format!("seal serialize: {e}")))
}

/// Parse the on-disk seal JSON back into `(ciphertext, meta)`.
pub fn seal_from_json(bytes: &[u8]) -> Result<(Vec<u8>, crate::store::KeystoreMeta), CustodyError> {
    use base64::Engine as _;
    let sf: SealFile =
        serde_json::from_slice(bytes).map_err(|e| CustodyError::Other(format!("seal parse: {e}")))?;
    let ct = base64::engine::general_purpose::STANDARD
        .decode(sf.sealed_b64.as_bytes())
        .map_err(|e| CustodyError::Other(format!("seal b64: {e}")))?;
    Ok((ct, sf.meta))
}

/// Atomically persist the sealed custody to `path` (temp file + rename, 0600 perms). Writes ONLY
/// the ciphertext + non-secret meta. Best surface for `genesis_confirm` to call after sealing.
pub fn write_seal_file(
    path: &str,
    encrypted_seed: &[u8],
    meta: &crate::store::KeystoreMeta,
) -> Result<(), CustodyError> {
    let bytes = seal_to_json(encrypted_seed, meta)?;
    let p = std::path::Path::new(path);
    if let Some(dir) = p.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)
                .map_err(|e| CustodyError::Other(format!("seal mkdir: {e}")))?;
        }
    }
    // unique temp sibling so concurrent writers don't clobber each other.
    let tmp = format!("{path}.tmp.{}", std::process::id());
    {
        let mut f = open_owner_only(&tmp)?;
        f.write_all(&bytes)
            .map_err(|e| CustodyError::Other(format!("seal write: {e}")))?;
        f.flush()
            .map_err(|e| CustodyError::Other(format!("seal flush: {e}")))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        CustodyError::Other(format!("seal rename: {e}"))
    })?;
    Ok(())
}

/// Load + parse the seal at `path` if it exists. `Ok(None)` when the file is absent (fresh boot).
pub fn read_seal_file(path: &str) -> Result<Option<(Vec<u8>, crate::store::KeystoreMeta)>, CustodyError> {
    match std::fs::read(path) {
        Ok(bytes) => seal_from_json(&bytes).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CustodyError::Other(format!("seal read: {e}"))),
    }
}

/// Create/truncate `path` with 0600 (owner-only) perms on unix; default perms elsewhere.
fn open_owner_only(path: &str) -> Result<std::fs::File, CustodyError> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
        .map_err(|e| CustodyError::Other(format!("seal open: {e}")))
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
