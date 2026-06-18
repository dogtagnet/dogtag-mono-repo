//! Persistence abstraction (impl §11.4). `MemStore` (in-memory, used by tests) and an optional
//! `MongoStore` (production, behind the `mongo` feature) implement the same `Store` trait.
//!
//! The store holds: issuance/verification records, verify sessions, one-time JWT jti set,
//! issuer settings (signing mode), and keystore metadata (addresses/labels only — NEVER the seed).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A prepared/issued credential record. `prepared_calldata` pins the exact `issue(bytes32)` calldata
/// so confirm can bind the broadcast tx to THIS draft (impl §11.6).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub record_id: String,
    pub record_type: String,
    pub dog_tag_id: String,
    /// The wrapped document (dogtag-standard WrappedDoc), serialized.
    pub wrapped_doc: serde_json::Value,
    /// The single Poseidon root R (`0x..` hex32) == doc.signature.merkleRoot.
    pub root: String,
    /// hex calldata for issue(root), pinned at prepare time.
    pub prepared_calldata: String,
    /// the issuer clone address (documentStore) this record anchors to.
    pub issuer_addr: String,
    pub status: RecordStatus,
    pub tx_hash: Option<String>,
    pub confirmed_tx_hash: Option<String>,
    pub signer_address: Option<String>,
    pub signing_mode: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordStatus {
    Prepared,
    Confirming,
    Issued,
    Revoked,
}

/// A verifier session (impl §3.9).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifySession {
    pub session_id: String,
    pub relayer: String,
    pub purpose: String,
    pub record_type: String,
    pub mode: String, // "normal" | "zk"
    pub challenge: String,
    pub status: String, // "pending" | "recorded"
    pub tx_hash: Option<String>,
}

/// Persisted per-issuer settings (impl §3.8).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuerSettings {
    pub signing_mode: String, // "wallet" | "backend"
}

impl Default for IssuerSettings {
    fn default() -> Self {
        IssuerSettings { signing_mode: "backend".to_string() }
    }
}

/// Keystore metadata — addresses + labels ONLY. The encrypted seed is held separately.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct KeystoreMeta {
    /// derived accounts: index -> (address, label)
    pub accounts: Vec<AccountMeta>,
    pub state: String, // "uninitialized" | "initialized"
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountMeta {
    pub index: u32,
    pub address: String,
    pub label: String,
}

/// The custody blob: the age-encrypted (scrypt passphrase) BIP-39 seed/mnemonic + meta.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CustodyBlob {
    /// age-encrypted ciphertext (armored).
    pub encrypted_seed: Vec<u8>,
    pub meta: KeystoreMeta,
}

/// The persistence trait. All methods async so MongoStore is a drop-in.
#[async_trait]
pub trait Store: Send + Sync {
    // ---- records ----
    async fn put_record(&self, r: Record);
    async fn get_record(&self, id: &str) -> Option<Record>;
    async fn update_record(&self, r: Record);
    /// true if any record currently has status == prepared.
    async fn has_prepared(&self) -> bool;
    /// idempotency lookup: record already confirmed at this txHash.
    async fn record_by_confirmed_tx(&self, tx_hash: &str) -> Option<Record>;

    // ---- verify sessions ----
    async fn put_session(&self, s: VerifySession);
    async fn get_session(&self, id: &str) -> Option<VerifySession>;
    async fn update_session(&self, s: VerifySession);

    // ---- jwt jti (one-time) ----
    /// Atomic consume: returns true if the jti was unused (now consumed), false if already used.
    async fn consume_jti(&self, jti: &str) -> bool;

    // ---- issuer settings ----
    async fn get_settings(&self) -> IssuerSettings;
    async fn put_settings(&self, s: IssuerSettings);

    // ---- custody ----
    async fn get_custody(&self) -> Option<CustodyBlob>;
    async fn put_custody(&self, blob: CustodyBlob);

    // ---- operator sessions (bearer tokens) ----
    async fn put_op_session(&self, token: String);
    async fn has_op_session(&self, token: &str) -> bool;

    // ---- imported client cache (import/pull) ----
    async fn upsert_client_cache(&self, dog_tag_id: String, doc: serde_json::Value);
    async fn get_client_cache(&self, dog_tag_id: &str) -> Option<serde_json::Value>;
}

// --------------------------------------------------------------------------------------------
// MemStore — Arc<RwLock<...>>; used by tests (no live Mongo required).
// --------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemInner {
    records: HashMap<String, Record>,
    sessions: HashMap<String, VerifySession>,
    jtis: std::collections::HashSet<String>,
    settings: Option<IssuerSettings>,
    custody: Option<CustodyBlob>,
    op_sessions: std::collections::HashSet<String>,
    client_cache: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Default)]
pub struct MemStore {
    inner: Arc<RwLock<MemInner>>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Store for MemStore {
    async fn put_record(&self, r: Record) {
        self.inner.write().unwrap().records.insert(r.record_id.clone(), r);
    }
    async fn get_record(&self, id: &str) -> Option<Record> {
        self.inner.read().unwrap().records.get(id).cloned()
    }
    async fn update_record(&self, r: Record) {
        self.inner.write().unwrap().records.insert(r.record_id.clone(), r);
    }
    async fn has_prepared(&self) -> bool {
        self.inner
            .read()
            .unwrap()
            .records
            .values()
            .any(|r| r.status == RecordStatus::Prepared)
    }
    async fn record_by_confirmed_tx(&self, tx_hash: &str) -> Option<Record> {
        self.inner
            .read()
            .unwrap()
            .records
            .values()
            .find(|r| r.confirmed_tx_hash.as_deref() == Some(tx_hash))
            .cloned()
    }

    async fn put_session(&self, s: VerifySession) {
        self.inner.write().unwrap().sessions.insert(s.session_id.clone(), s);
    }
    async fn get_session(&self, id: &str) -> Option<VerifySession> {
        self.inner.read().unwrap().sessions.get(id).cloned()
    }
    async fn update_session(&self, s: VerifySession) {
        self.inner.write().unwrap().sessions.insert(s.session_id.clone(), s);
    }

    async fn consume_jti(&self, jti: &str) -> bool {
        // atomic under the write lock: insert returns true iff newly inserted.
        self.inner.write().unwrap().jtis.insert(jti.to_string())
    }

    async fn get_settings(&self) -> IssuerSettings {
        self.inner.read().unwrap().settings.clone().unwrap_or_default()
    }
    async fn put_settings(&self, s: IssuerSettings) {
        self.inner.write().unwrap().settings = Some(s);
    }

    async fn get_custody(&self) -> Option<CustodyBlob> {
        self.inner.read().unwrap().custody.clone()
    }
    async fn put_custody(&self, blob: CustodyBlob) {
        self.inner.write().unwrap().custody = Some(blob);
    }

    async fn put_op_session(&self, token: String) {
        self.inner.write().unwrap().op_sessions.insert(token);
    }
    async fn has_op_session(&self, token: &str) -> bool {
        self.inner.read().unwrap().op_sessions.contains(token)
    }

    async fn upsert_client_cache(&self, dog_tag_id: String, doc: serde_json::Value) {
        self.inner.write().unwrap().client_cache.insert(dog_tag_id, doc);
    }
    async fn get_client_cache(&self, dog_tag_id: &str) -> Option<serde_json::Value> {
        self.inner.read().unwrap().client_cache.get(dog_tag_id).cloned()
    }
}
