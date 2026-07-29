//! Shared application state + configuration for the oversight indexer.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use alloy::primitives::Address;
use serde::Deserialize;

use crate::chain::{LogSource, WatchContext, WatchGeneration};
use crate::directory::Directory;
use crate::scope::ScopeRegistry;
use crate::store::Store;

/// Runtime configuration (env-driven; see `main.rs`).
#[derive(Clone, Debug)]
pub struct Config {
    pub rpc_url: String,

    // Watched contract generations (validated, unique, lowercase `0x…`).
    pub generations: Vec<WatchGeneration>,

    // Scan tuning.
    /// First block to scan on a fresh index (at/before the oldest watched generation; 0 = genesis).
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
    /// The role-specific watch context for a scan, seeded with each generation's deployment-known
    /// clones plus any configured-generation clones the caller has discovered so far.
    pub fn watch_context(&self, discovered: &HashMap<String, String>) -> WatchContext {
        let mut factories = HashMap::new();
        let mut issuer_registries = HashMap::new();
        let mut verification_registries = HashMap::new();
        let mut known_clones = HashMap::new();
        let configured: HashSet<&str> =
            self.generations.iter().map(|g| g.generation.as_str()).collect();

        for generation in &self.generations {
            factories.insert(generation.factory.clone(), generation.generation.clone());
            issuer_registries.insert(
                generation.issuer_registry.clone(),
                generation.generation.clone(),
            );
            verification_registries.insert(
                generation.verification_registry.clone(),
                generation.generation.clone(),
            );
            for clone in &generation.seed_clones {
                known_clones.insert(clone.clone(), generation.generation.clone());
            }
        }
        for (clone, generation) in discovered {
            if configured.contains(generation.as_str()) {
                // A validated seed owns the address if a stale persisted discovery contradicts it.
                known_clones
                    .entry(clone.clone())
                    .or_insert_with(|| generation.clone());
            }
        }
        WatchContext {
            factories,
            issuer_registries,
            verification_registries,
            known_clones,
        }
    }

    /// Resolve a factory address to the immutable generation id that currently admits it.
    pub fn generation_for_factory(&self, factory: &str) -> Option<&str> {
        self.generations
            .iter()
            .find(|generation| generation.factory.eq_ignore_ascii_case(factory))
            .map(|generation| generation.generation.as_str())
    }

    /// `https://explorer.roax.net/tx/<hash>` for a tx hash.
    pub fn tx_url(&self, tx_hash: &str) -> String {
        format!("{}/tx/{}", self.explorer_base.trim_end_matches('/'), tx_hash)
    }
}

/// JSON shape accepted by `INDEXER_GENERATIONS`. The immutable `generation` id is deliberately not
/// operator-authored: it is derived from the normalized factory address, so a label rename cannot
/// split persisted history.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawWatchGeneration {
    factory: String,
    issuer_registry: String,
    verification_registry: String,
    #[serde(default)]
    seed_clones: Vec<String>,
}

fn normalized_address(value: &str, field: &str) -> Result<String, String> {
    let address: Address = value
        .trim()
        .parse()
        .map_err(|_| format!("{field} must be a 20-byte 0x-prefixed contract address, got {value:?}"))?;
    if address == Address::ZERO {
        return Err(format!("{field} must not be the zero address"));
    }
    Ok(format!("{address:#x}"))
}

/// Construct and normalize one generation. Call [`validate_watch_generations`] on the whole set
/// before using it so cross-generation ambiguity is rejected.
pub fn watch_generation(
    factory: &str,
    issuer_registry: &str,
    verification_registry: &str,
    seed_clones: Vec<String>,
) -> Result<WatchGeneration, String> {
    let factory = normalized_address(factory, "factory")?;
    let issuer_registry = normalized_address(issuer_registry, "issuerRegistry")?;
    let verification_registry =
        normalized_address(verification_registry, "verificationRegistry")?;
    let seed_clones = seed_clones
        .into_iter()
        .enumerate()
        .map(|(i, clone)| normalized_address(&clone, &format!("seedClones[{i}]")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WatchGeneration {
        generation: factory.clone(),
        factory,
        issuer_registry,
        verification_registry,
        seed_clones,
    })
}

/// Validate that a generation set is non-empty and has no ambiguous emitter address. A shared
/// factory, registry, verification registry, or seed clone could not be stamped with one generation,
/// so startup fails closed instead of choosing whichever entry happened to come first.
pub fn validate_watch_generations(
    generations: Vec<WatchGeneration>,
) -> Result<Vec<WatchGeneration>, String> {
    if generations.is_empty() {
        return Err("INDEXER_GENERATIONS must contain at least one generation".into());
    }

    let mut seen: HashMap<&str, (&str, &str)> = HashMap::new();
    for generation in &generations {
        for (role, address) in [
            ("factory", generation.factory.as_str()),
            ("issuerRegistry", generation.issuer_registry.as_str()),
            (
                "verificationRegistry",
                generation.verification_registry.as_str(),
            ),
        ]
        .into_iter()
        .chain(
            generation
                .seed_clones
                .iter()
                .map(|clone| ("seedClone", clone.as_str())),
        ) {
            if let Some((prior_generation, prior_role)) =
                seen.insert(address, (&generation.generation, role))
            {
                return Err(format!(
                    "ambiguous watched address {address}: {prior_role} in generation \
                     {prior_generation} and {role} in generation {}",
                    generation.generation
                ));
            }
        }
    }
    Ok(generations)
}

/// Parse the atomic generation-set configuration. Parallel address lists are deliberately avoided:
/// one JSON object owns the exact triple and its seed clones, so list ordering cannot cross-wire two
/// deployments.
pub fn parse_watch_generations(raw: &str) -> Result<Vec<WatchGeneration>, String> {
    if raw.trim().is_empty() {
        return Err("INDEXER_GENERATIONS is set but empty".into());
    }
    let raw_generations: Vec<RawWatchGeneration> = serde_json::from_str(raw)
        .map_err(|e| format!("INDEXER_GENERATIONS is not valid JSON: {e}"))?;
    let generations = raw_generations
        .into_iter()
        .map(|generation| {
            watch_generation(
                &generation.factory,
                &generation.issuer_registry,
                &generation.verification_registry,
                generation.seed_clones,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_watch_generations(generations)
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

#[cfg(test)]
mod tests {
    use super::*;

    const FACTORY_ONE: &str = "0x00000000000000000000000000000000000000A1";
    const REGISTRY_ONE: &str = "0x00000000000000000000000000000000000000B1";
    const VREG_ONE: &str = "0x00000000000000000000000000000000000000C1";
    const FACTORY_TWO: &str = "0x00000000000000000000000000000000000000A2";
    const REGISTRY_TWO: &str = "0x00000000000000000000000000000000000000B2";
    const VREG_TWO: &str = "0x00000000000000000000000000000000000000C2";

    #[test]
    fn generation_json_is_atomic_normalized_and_factory_identified() {
        let raw = format!(
            r#"[
                {{
                    "factory":"{FACTORY_ONE}",
                    "issuerRegistry":"{REGISTRY_ONE}",
                    "verificationRegistry":"{VREG_ONE}",
                    "seedClones":["0x00000000000000000000000000000000000000D1"]
                }},
                {{
                    "factory":"{FACTORY_TWO}",
                    "issuerRegistry":"{REGISTRY_TWO}",
                    "verificationRegistry":"{VREG_TWO}"
                }}
            ]"#
        );
        let generations = parse_watch_generations(&raw).expect("valid generation set");
        assert_eq!(generations.len(), 2);
        assert_eq!(
            generations[0].generation, generations[0].factory,
            "the immutable generation id is the normalized factory address"
        );
        assert_eq!(
            generations[0].factory,
            FACTORY_ONE.to_ascii_lowercase()
        );
        assert_eq!(generations[1].seed_clones, Vec::<String>::new());
    }

    #[test]
    fn generation_json_rejects_empty_and_ambiguous_sets() {
        assert!(parse_watch_generations("[]")
            .unwrap_err()
            .contains("at least one generation"));

        let duplicate_registry = format!(
            r#"[
                {{"factory":"{FACTORY_ONE}","issuerRegistry":"{REGISTRY_ONE}","verificationRegistry":"{VREG_ONE}"}},
                {{"factory":"{FACTORY_TWO}","issuerRegistry":"{REGISTRY_ONE}","verificationRegistry":"{VREG_TWO}"}}
            ]"#
        );
        let error = parse_watch_generations(&duplicate_registry).unwrap_err();
        assert!(error.contains("ambiguous watched address"), "{error}");
    }

    #[test]
    fn generation_json_rejects_malformed_or_zero_addresses() {
        let malformed = r#"[{
            "factory":"not-an-address",
            "issuerRegistry":"0x00000000000000000000000000000000000000b1",
            "verificationRegistry":"0x00000000000000000000000000000000000000c1"
        }]"#;
        assert!(parse_watch_generations(malformed)
            .unwrap_err()
            .contains("factory must be"));

        let zero = format!(
            r#"[{{"factory":"{FACTORY_ONE}","issuerRegistry":"0x0000000000000000000000000000000000000000","verificationRegistry":"{VREG_ONE}"}}]"#
        );
        assert!(parse_watch_generations(&zero)
            .unwrap_err()
            .contains("must not be the zero address"));
    }

    #[test]
    fn shipped_env_generation_set_parses() {
        let example = include_str!("../../.env.example");
        let raw = example
            .lines()
            .find_map(|line| line.strip_prefix("INDEXER_GENERATIONS="))
            .expect("indexer env template must carry INDEXER_GENERATIONS");
        let generations =
            parse_watch_generations(raw).expect("shipped INDEXER_GENERATIONS must be bootable");
        assert_eq!(generations.len(), 1);
        assert_eq!(generations[0].seed_clones.len(), 3);
    }
}
