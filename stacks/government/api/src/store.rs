//! `Store` — the government authority's centralized (off-chain) database surface.
//!
//! On-chain we anchor only the salted Poseidon root `R`. The centralized DB holds the operational
//! record the authority is legally the custodian of: the full issued credential (wrapped doc +
//! applicant/consignment metadata) and an audit log of every verification the authority performed.
//! This is exactly the "business backend keeps its own Mongo" model from architecture §1.2.
//!
//! `MemStore` (default) is ephemeral (demo/local + tests). `MongoStore` (feature `mongo`) persists.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An issued government credential (TRAVEL_CLEARANCE / EU_HEALTH_CERT / authority-endorsement).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuedCredential {
    /// The anchored Poseidon root `R` (== signature.merkleRoot) — the primary key.
    pub root: String,
    #[serde(rename = "recordType")]
    pub record_type: String,
    #[serde(rename = "dogTagId")]
    pub dog_tag_id: String,
    /// The DogTagIssuer clone the root was anchored to.
    #[serde(rename = "issuerAddr")]
    pub issuer_addr: String,
    /// The full wrapped credential document (the holder receives a copy; the authority is custodian).
    #[serde(rename = "wrappedDoc")]
    pub wrapped_doc: Value,
    /// Anchoring tx hash, when the root was issued on-chain (absent when built but not yet anchored).
    #[serde(rename = "txHash", skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(rename = "anchored")]
    pub anchored: bool,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
}

/// A recorded verification the authority performed against the ROAX contracts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationRecord {
    pub id: String,
    #[serde(rename = "recordType")]
    pub record_type: String,
    pub root: String,
    #[serde(rename = "issuerAddr")]
    pub issuer_addr: String,
    /// integrity | onchain | issuer_identity fragment states, folded to a single verdict.
    #[serde(rename = "integrityValid")]
    pub integrity_valid: bool,
    #[serde(rename = "onchainValid")]
    pub onchain_valid: bool,
    #[serde(rename = "issuerWhitelisted", skip_serializing_if = "Option::is_none")]
    pub issuer_whitelisted: Option<bool>,
    pub verdict: bool,
    #[serde(rename = "checkedAt")]
    pub checked_at: u64,
}

#[async_trait]
pub trait Store: Send + Sync {
    async fn put_credential(&self, cred: IssuedCredential);
    async fn get_credential(&self, root: &str) -> Option<IssuedCredential>;
    async fn list_credentials(&self) -> Vec<IssuedCredential>;
    async fn put_verification(&self, rec: VerificationRecord);
    async fn list_verifications(&self) -> Vec<VerificationRecord>;
}

// --------------------------------------------------------------------------------------------
// MemStore
// --------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemInner {
    credentials: HashMap<String, IssuedCredential>,
    /// insertion order for stable listing.
    cred_order: Vec<String>,
    verifications: Vec<VerificationRecord>,
}

#[derive(Clone, Default)]
pub struct MemStore {
    inner: Arc<Mutex<MemInner>>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Store for MemStore {
    async fn put_credential(&self, cred: IssuedCredential) {
        let mut g = self.inner.lock().unwrap();
        if !g.credentials.contains_key(&cred.root) {
            g.cred_order.push(cred.root.clone());
        }
        g.credentials.insert(cred.root.clone(), cred);
    }
    async fn get_credential(&self, root: &str) -> Option<IssuedCredential> {
        self.inner.lock().unwrap().credentials.get(root).cloned()
    }
    async fn list_credentials(&self) -> Vec<IssuedCredential> {
        let g = self.inner.lock().unwrap();
        g.cred_order
            .iter()
            .filter_map(|r| g.credentials.get(r).cloned())
            .collect()
    }
    async fn put_verification(&self, rec: VerificationRecord) {
        self.inner.lock().unwrap().verifications.push(rec);
    }
    async fn list_verifications(&self) -> Vec<VerificationRecord> {
        self.inner.lock().unwrap().verifications.clone()
    }
}
