//! Application state + config + the server-side VC build (impl §3.3/§11.6: build is ALWAYS server-side).

use std::sync::Arc;

use dogtag_standard::wrap::{wrap_document, IssuerMeta, WrappedDoc};
use serde_json::Value;

use crate::auth::JwtKeys;
use crate::chain::{record_type_key, ChainClient};
use crate::custody::Custody;
use crate::prover::ProverClient;
use crate::store::Store;

/// Resolved issuer/contract addresses + deployment config.
#[derive(Clone)]
pub struct Config {
    pub deployment_url: String,
    pub rpc_url: String,
    pub issuer_registry_addr: String,
    pub verification_registry_addr: String,
    /// recordType (string) -> issuer clone address (documentStore).
    pub issuer_addrs: std::collections::HashMap<String, String>,
    pub issuer_name: String,
    pub issuer_domain: String,
    /// operator portal password (in prod: hashed/secret-managed).
    pub operator_password: String,
    /// admin-session password for /admin/* custody routes.
    pub admin_password: String,
    /// confirmations to wait at confirm time (low for tests).
    pub confirmations: u64,
}

impl Config {
    pub fn issuer_addr_for(&self, record_type: &str) -> Option<String> {
        self.issuer_addrs.get(record_type).cloned()
    }
}

/// The shared application state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub chain: Arc<dyn ChainClient>,
    pub prover: Arc<dyn ProverClient>,
    pub custody: Custody,
    pub jwt: JwtKeys,
    pub cfg: Arc<Config>,
}

/// Build the issuer metadata for a record type (documentStore = the issuer clone address).
pub fn issuer_meta(cfg: &Config, record_type: &str, issuer_addr: &str) -> IssuerMeta {
    IssuerMeta {
        name: cfg.issuer_name.clone(),
        domain: cfg.issuer_domain.clone(),
        document_store: issuer_addr.to_string(),
        record_type: record_type.to_string(),
    }
}

/// Build a typed credential (the typed-scalar `data` the SDK wraps) from operator `fields`.
///
/// The operator supplies `fields` already in the SDK's typed-scalar input shape
/// (`{tag:<u8>, value:"..."}` leaves, nested under `credentialSubject`/top-level). We inject the
/// mandatory, non-obfuscatable `credentialSubject.dogTagId` (tag 3 = INTEGER, or tag 2 if non-numeric).
pub fn build_vc(record_type: &str, fields: &Value, dog_tag_id: &str) -> Value {
    let mut cred = fields.clone();
    if !cred.is_object() {
        cred = serde_json::json!({});
    }
    let obj = cred.as_object_mut().unwrap();
    let subject = obj
        .entry("credentialSubject")
        .or_insert_with(|| serde_json::json!({}));
    if let Some(s) = subject.as_object_mut() {
        // dogTagId is INTEGER if it parses as a decimal integer, else STRING.
        let is_int = dog_tag_id.bytes().all(|b| b.is_ascii_digit()) && !dog_tag_id.is_empty();
        let tag = if is_int { 3 } else { 2 };
        s.insert(
            "dogTagId".to_string(),
            serde_json::json!({ "tag": tag, "value": dog_tag_id }),
        );
    }
    // attach recordType passthrough for downstream context (not wrapped into leaves unless present).
    let _ = record_type;
    cred
}

/// Wrap a VC into a `WrappedDoc` using a cryptographically-random salt provider.
pub fn wrap(record_type: &str, issuer_meta: IssuerMeta, vc: &Value) -> Result<WrappedDoc, String> {
    let _ = record_type;
    let mut salt = || {
        let mut s = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut s);
        s
    };
    wrap_document(vc, issuer_meta, &mut salt).map_err(|e| format!("wrap: {e}"))
}

/// Convenience: the bytes32 whitelist/issuer key for a record type.
pub fn rt_key(record_type: &str) -> String {
    record_type_key(record_type)
}
