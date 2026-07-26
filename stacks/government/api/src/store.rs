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
    /// == the `protocol` block's `issuerClone` (M7 §4.2).
    #[serde(rename = "issuerAddr")]
    pub issuer_addr: String,
    /// M7 provenance mirror (§4.2), populated from the `WrappedDoc.protocol` block - persisted BESIDE
    /// `R`, never inside it. IMMUTABLE once set. `issuer_signer` is the on-chain `clone.issuedBy[R]`
    /// (== this authority's issuing signer). `Option`/defaulted so older rows still load.
    #[serde(rename = "chainId", default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(
        rename = "protocolVersion",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub protocol_version: Option<String>,
    #[serde(
        rename = "verificationRegistry",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub verification_registry: Option<String>,
    #[serde(
        rename = "issuerSigner",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub issuer_signer: Option<String>,
    /// The public Crockford-base32 receipt handle (also a salted leaf committed in R, so it is
    /// IMMUTABLE). This is the `/r/:receiptId` lookup key — unique across the authority's records.
    /// `Option` for backward-compat with rows written before receipts existed.
    #[serde(rename = "receiptId", default, skip_serializing_if = "Option::is_none")]
    pub receipt_id: Option<String>,
    /// Cleartext projection of the credential's `credentialSubject` (the same content committed in R,
    /// so it is IMMUTABLE). Denormalized for list/search/detail rendering without re-parsing the
    /// wrapped doc. Never a new PII surface beyond what the authority already custodies in `wrappedDoc`.
    #[serde(rename = "subject", default, skip_serializing_if = "Value::is_null")]
    pub subject: Value,
    /// Denormalized `validity.validUntil` (ISO-8601 date) for derived-expiry queries + rendering. It
    /// mirrors a leaf committed in R, so it is IMMUTABLE.
    #[serde(
        rename = "validUntil",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub valid_until: Option<String>,
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

/// An owner-hidden verification session — the authority acting as a VERIFIER.
///
/// Mirror of `stacks/vet/api/src/store.rs::VerifySession` (same field names, same status vocabulary),
/// so the owner's phone drives the government exactly as it drives the vet/groomer: it resolves the
/// session from the export token at `GET /x/{token}`, proves on-device, and polls
/// `GET /verify/session/{id}`. Nothing here is credential PII — it is verifier-side operational proof
/// metadata (purpose, relayer, tx, nullifier).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifySession {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    /// The government signer that will submit the consent proof (`pub[relayer]` must name it).
    pub relayer: String,
    pub purpose: String,
    #[serde(rename = "recordType")]
    pub record_type: String,
    pub challenge: String,
    /// "pending" | "recording" | "recorded" | "error".
    pub status: String,
    #[serde(rename = "txHash", default)]
    pub tx_hash: Option<String>,
    /// The consumed consent nullifier (set once a proof is accepted for recording).
    #[serde(default)]
    pub nullifier: Option<String>,
    #[serde(rename = "createdAt", default)]
    pub created_at: u64,
    #[serde(rename = "updatedAt", default)]
    pub updated_at: u64,
    /// D1: the identity-leaf keyPaths the owner chose to REVEAL alongside this consent proof. KeyPaths
    /// only — the disclosed VALUES are shown to the verifying operator, never stored.
    #[serde(rename = "disclosedKeyPaths", default)]
    pub disclosed_key_paths: Vec<String>,
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
    /// Resolve a credential by its public `receiptId` handle (the `/r/:receiptId` lookup). `None`
    /// when no credential carries that receipt id.
    async fn get_credential_by_receipt_id(&self, receipt_id: &str) -> Option<IssuedCredential>;
    async fn list_credentials(&self) -> Vec<IssuedCredential>;
    async fn put_verification(&self, rec: VerificationRecord);
    async fn list_verifications(&self) -> Vec<VerificationRecord>;

    // ---- share tokens (short one-time QR token -> credential root) ----
    // Mirror of the vet-api share-token contract (`stacks/vet/api/src/store.rs`) so the owner's phone
    // imports from the government exactly as it imports from the vet. The value is the credential
    // ROOT here (the government's primary key) where the vet stores its record id.
    /// Store a short one-time share token mapping to `root`, expiring at unix-seconds `exp`.
    async fn put_share_token(&self, token: &str, root: &str, exp: u64);
    /// Atomically REMOVE the token (one-time consume) and return its `root` iff it exists and has not
    /// expired. A missing/expired token returns `None` (and is purged if expired).
    async fn take_share_token(&self, token: &str) -> Option<String>;

    // ---- export tokens (short-lived verify-session QR token -> verify session) ----
    /// Store a short-lived export token mapping to `session_id`, expiring at unix-seconds `exp`.
    async fn put_export_token(&self, token: &str, session_id: &str, exp: u64);
    /// NON-consuming lookup: the export token's `session_id` iff present and unexpired. Used by
    /// `GET /x/{token}` and the status poll — the session status provides the replay guard, so the
    /// token is deliberately never consumed here.
    async fn peek_export_token(&self, token: &str) -> Option<String>;

    // ---- owner-hidden verify sessions ----
    async fn put_session(&self, s: VerifySession);
    async fn get_session(&self, id: &str) -> Option<VerifySession>;
    async fn update_session(&self, s: VerifySession);
    /// Verify sessions as a durable audit trail, most-recently-created first.
    async fn list_sessions(&self) -> Vec<VerifySession>;
}

/// Wall-clock seconds — the expiry basis for both token kinds.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// --------------------------------------------------------------------------------------------
// MemStore
// --------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemInner {
    credentials: HashMap<String, IssuedCredential>,
    /// insertion order for stable listing.
    cred_order: Vec<String>,
    /// receiptId -> root, for the `/r/:receiptId` lookup (mirrors the Mongo unique index).
    receipt_index: HashMap<String, String>,
    verifications: Vec<VerificationRecord>,
    /// short one-time share tokens: token -> (credential root, exp unix-seconds).
    share_tokens: HashMap<String, (String, u64)>,
    /// short-lived verify-session export tokens: token -> (session_id, exp unix-seconds).
    export_tokens: HashMap<String, (String, u64)>,
    /// owner-hidden verify sessions by session id.
    sessions: HashMap<String, VerifySession>,
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
        if let Some(rid) = &cred.receipt_id {
            g.receipt_index.insert(rid.clone(), cred.root.clone());
        }
        g.credentials.insert(cred.root.clone(), cred);
    }
    async fn get_credential(&self, root: &str) -> Option<IssuedCredential> {
        self.inner.lock().unwrap().credentials.get(root).cloned()
    }
    async fn get_credential_by_receipt_id(&self, receipt_id: &str) -> Option<IssuedCredential> {
        let g = self.inner.lock().unwrap();
        let root = g.receipt_index.get(receipt_id)?;
        g.credentials.get(root).cloned()
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

    async fn put_share_token(&self, token: &str, root: &str, exp: u64) {
        self.inner
            .lock()
            .unwrap()
            .share_tokens
            .insert(token.to_string(), (root.to_string(), exp));
    }
    async fn take_share_token(&self, token: &str) -> Option<String> {
        // atomic remove under the lock == one-time consume.
        let mut g = self.inner.lock().unwrap();
        let (root, exp) = g.share_tokens.remove(token)?;
        // expired tokens are consumed-on-read and treated as missing.
        if now_secs() > exp {
            None
        } else {
            Some(root)
        }
    }

    async fn put_export_token(&self, token: &str, session_id: &str, exp: u64) {
        self.inner
            .lock()
            .unwrap()
            .export_tokens
            .insert(token.to_string(), (session_id.to_string(), exp));
    }
    async fn peek_export_token(&self, token: &str) -> Option<String> {
        let g = self.inner.lock().unwrap();
        let (session_id, exp) = g.export_tokens.get(token)?;
        if now_secs() > *exp {
            None
        } else {
            Some(session_id.clone())
        }
    }

    async fn put_session(&self, s: VerifySession) {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .insert(s.session_id.clone(), s);
    }
    async fn get_session(&self, id: &str) -> Option<VerifySession> {
        self.inner.lock().unwrap().sessions.get(id).cloned()
    }
    async fn update_session(&self, s: VerifySession) {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .insert(s.session_id.clone(), s);
    }
    async fn list_sessions(&self) -> Vec<VerifySession> {
        let mut v: Vec<VerifySession> = self
            .inner
            .lock()
            .unwrap()
            .sessions
            .values()
            .cloned()
            .collect();
        v.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
                .then_with(|| b.session_id.cmp(&a.session_id))
        });
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn future() -> u64 {
        now_secs() + 300
    }
    fn past() -> u64 {
        now_secs().saturating_sub(1)
    }

    #[tokio::test]
    async fn share_token_take_consumes_once_then_gone() {
        let s = MemStore::new();
        s.put_share_token("tok", "0xroot", future()).await;
        assert_eq!(s.take_share_token("tok").await, Some("0xroot".to_string()));
        // one-time: a second take finds nothing.
        assert_eq!(s.take_share_token("tok").await, None);
    }

    #[tokio::test]
    async fn share_token_missing_and_expired_are_none() {
        let s = MemStore::new();
        assert_eq!(s.take_share_token("absent").await, None);
        s.put_share_token("old", "0xroot", past()).await;
        assert_eq!(s.take_share_token("old").await, None);
    }

    #[tokio::test]
    async fn export_token_peek_does_not_consume_and_honours_expiry() {
        let s = MemStore::new();
        s.put_export_token("tok", "sess-1", future()).await;
        // NON-consuming: repeated peeks keep resolving (the phone polls with it).
        assert_eq!(s.peek_export_token("tok").await, Some("sess-1".to_string()));
        assert_eq!(s.peek_export_token("tok").await, Some("sess-1".to_string()));
        s.put_export_token("old", "sess-2", past()).await;
        assert_eq!(s.peek_export_token("old").await, None);
    }

    #[tokio::test]
    async fn sessions_list_newest_first() {
        let s = MemStore::new();
        let mk = |id: &str, created: u64| VerifySession {
            session_id: id.to_string(),
            relayer: "0xrelayer".into(),
            purpose: "travel_check".into(),
            record_type: "VACCINATION".into(),
            challenge: "0x00".into(),
            status: "pending".into(),
            tx_hash: None,
            nullifier: None,
            created_at: created,
            updated_at: created,
            disclosed_key_paths: Vec::new(),
        };
        s.put_session(mk("old", 100)).await;
        s.put_session(mk("new", 200)).await;
        let list = s.list_sessions().await;
        assert_eq!(list[0].session_id, "new");
        assert_eq!(list[1].session_id, "old");
    }
}
