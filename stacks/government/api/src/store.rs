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

/// Lifecycle state of an issued credential. `delete` is NEVER a row removal — an anchored credential
/// transitions to `Revoked` (on-chain revoke path) or `Expired` (off-chain validity lapse) and stays
/// in the DB with its on-chain proof intact, still verifiable on the block explorer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialStatus {
    /// Built + persisted but not yet anchored on-chain (dry-run / no signer).
    Draft,
    /// Anchored on-chain and currently valid.
    Issued,
    /// Invalidated on-chain via `DogTagIssuer.revoke` — `isValid` is now false, history retained.
    Revoked,
    /// Marked expired off-chain (validity window lapsed) — the on-chain anchor is untouched.
    Expired,
}

/// An issued government credential (TRAVEL_CLEARANCE / EU_HEALTH_CERT / authority-endorsement).
///
/// The record bundles the credential data with its **immutable on-chain proof** — the anchoring tx
/// hash, the block it mined into, the DogTagIssuer clone (contract) address, and a ready-to-click
/// block-explorer link — so the authority can always trace a credential back to the chain and
/// re-verify it. On-chain-derived fields are never mutated by an update; only `label`/`notes`/`status`
/// (off-chain metadata) are editable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuedCredential {
    /// The anchored Poseidon root `R` (== signature.merkleRoot) — the primary key. IMMUTABLE.
    pub root: String,
    #[serde(rename = "recordType")]
    pub record_type: String,
    #[serde(rename = "dogTagId")]
    pub dog_tag_id: String,
    /// The DogTagIssuer clone (contract) address the root was anchored to. IMMUTABLE.
    #[serde(rename = "issuerAddr")]
    pub issuer_addr: String,
    /// The full wrapped credential document (the holder receives a copy; the authority is custodian).
    /// Carries the anchored credential/document hash, so it is IMMUTABLE.
    #[serde(rename = "wrappedDoc")]
    pub wrapped_doc: Value,
    /// Anchoring tx hash, when the root was issued on-chain (absent when built but not yet anchored).
    /// IMMUTABLE.
    #[serde(rename = "txHash", skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    /// The block number the anchoring tx mined into. IMMUTABLE.
    #[serde(rename = "blockNumber", skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,
    /// Ready-to-click block-explorer link for the anchoring tx (`https://explorer.roax.net/tx/<hash>`).
    /// IMMUTABLE.
    #[serde(rename = "explorerUrl", skip_serializing_if = "Option::is_none")]
    pub explorer_url: Option<String>,
    #[serde(rename = "anchored")]
    pub anchored: bool,
    /// Lifecycle state (see `CredentialStatus`). Mutable only through the invalidate / expire paths.
    #[serde(default = "default_status")]
    pub status: CredentialStatus,
    /// Operator-editable off-chain label (e.g. a case reference). Never anchored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Operator-editable off-chain notes. Never anchored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Set when revoked on-chain: the revoke tx hash. IMMUTABLE once set.
    #[serde(rename = "revokedTxHash", skip_serializing_if = "Option::is_none")]
    pub revoked_tx_hash: Option<String>,
    /// Set when revoked on-chain: the revoke tx block number. IMMUTABLE once set.
    #[serde(rename = "revokedBlockNumber", skip_serializing_if = "Option::is_none")]
    pub revoked_block_number: Option<u64>,
    /// Set when revoked on-chain: the revoke tx explorer link. IMMUTABLE once set.
    #[serde(rename = "revokeExplorerUrl", skip_serializing_if = "Option::is_none")]
    pub revoke_explorer_url: Option<String>,
    /// Unix seconds the credential was invalidated (revoked or expired), if it has been.
    #[serde(rename = "invalidatedAt", skip_serializing_if = "Option::is_none")]
    pub invalidated_at: Option<u64>,
    /// Optional human reason for the invalidation.
    #[serde(rename = "invalidationReason", skip_serializing_if = "Option::is_none")]
    pub invalidation_reason: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
    #[serde(rename = "updatedAt", default)]
    pub updated_at: u64,
}

fn default_status() -> CredentialStatus {
    CredentialStatus::Issued
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
