//! Shared application state + configuration for the oversight indexer.

use std::collections::HashSet;
use std::sync::Arc;

use crate::chain::{LogSource, WatchContext};
use crate::directory::Directory;
use crate::scope::ScopeRegistry;
use crate::store::Store;

/// Runtime configuration (env-driven; see `main.rs`).
#[derive(Clone, Debug)]
pub struct Config {
    pub rpc_url: String,
    pub chain_id: u64,

    // Watched contracts (lowercase `0x…`).
    pub factory_addr: String,
    pub registry_addr: String,
    pub verification_registry_addr: String,
    /// `DogTagIssuer` clones known from the deployment record (government + demo clones), so
    /// issuances anchored *before* the indexer's first run are still attributed to a known clone.
    /// Discovered `IssuerCreated` clones extend this set at runtime.
    pub seed_clones: Vec<String>,

    // Scan tuning.
    /// First block to scan on a fresh index (the factory/registry deploy block; 0 = genesis).
    pub start_block: u64,
    /// Blocks below `head - confirmations` are treated as final; the rest are re-scanned each tick
    /// (a shallow-reorg buffer).
    pub confirmations: u64,
    /// Max block span per `eth_getLogs` request (RPC-friendly chunking).
    pub chunk_size: u64,
    /// Seconds between ingest ticks.
    pub poll_interval_secs: u64,

    // Query API.
    pub default_page_limit: usize,
    pub max_page_limit: usize,
    /// Block-explorer base (e.g. `https://explorer.roax.net`) for `txUrl` links in responses.
    pub explorer_base: String,
}

impl Config {
    /// The fixed watch context for a scan, seeded with the deployment-known clones plus any the
    /// caller has discovered so far.
    pub fn watch_context(&self, discovered: &HashSet<String>) -> WatchContext {
        let mut known: HashSet<String> = self.seed_clones.iter().cloned().collect();
        known.extend(discovered.iter().cloned());
        WatchContext {
            factory: self.factory_addr.clone(),
            registry: self.registry_addr.clone(),
            verification_registry: self.verification_registry_addr.clone(),
            known_clones: known,
        }
    }

    /// `https://explorer.roax.net/tx/<hash>` for a tx hash.
    pub fn tx_url(&self, tx_hash: &str) -> String {
        format!("{}/tx/{}", self.explorer_base.trim_end_matches('/'), tx_hash)
    }
}

/// keccak256(label) as lowercase `0x…` — the on-chain record-type / purpose key. Used to translate a
/// human `?recordType=TRAVEL_CLEARANCE` filter into the indexed `keccak256` key. Mirrors the
/// government stack's `record_type_key` (there via `sha3`; here via alloy's re-exported keccak).
pub fn keccak_key(label: &str) -> String {
    let h = alloy::primitives::keccak256(label.as_bytes());
    format!("0x{}", hex::encode(h.as_slice()))
}

/// The Axum shared state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub source: Arc<dyn LogSource>,
    pub scopes: Arc<ScopeRegistry>,
    pub directory: Arc<Directory>,
    pub cfg: Arc<Config>,
}
