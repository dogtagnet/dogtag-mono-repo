//! Shared application state + configuration for the oversight indexer.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use alloy::primitives::Address;
use serde::Deserialize;

use crate::chain::{LogSource, WatchContext, WatchGeneration};
use crate::directory::Directory;
use crate::mirror::ContentMirror;
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

    /// `MIRROR_INGEST_TOKEN`: the bearer `PUT /v1/content/:address` requires, and the ONLY thing
    /// that bearer authorizes.
    ///
    /// **PUBLIC BY CONSTRUCTION, and that is a statement about where it lives rather than advice.**
    /// The provider portals ship it to the browser as `VITE_CONTENT_MIRROR_TOKEN`, which vite
    /// INLINES into the bundle at build time, so every visitor holds it. "Keep this secret" would be
    /// advice nobody could follow; the honest question is what it GRANTS, and the answer must be the
    /// narrowest thing that still works.
    ///
    /// It grants exactly one capability: publish bytes that hash to their own content address. That
    /// is nearly inert - such a caller cannot overwrite, cannot displace, cannot shadow anybody
    /// else's content, cannot READ anything at all, and is bounded by [`MAX_MIRROR_OBJECTS`] and
    /// [`MAX_MIRROR_TOTAL_BYTES`].
    ///
    /// **It is NOT an oversight bearer and confers no read of any kind.** Never set it to an
    /// `INDEXER_SCOPES` token: those resolve through the scope registry, which gates `/v1/status`,
    /// `/v1/events`, `/v1/stats` and `/v1/issuers`, so a scope token in a browser bundle would make
    /// that operator's event feed world-readable - and an unscoped one would publish every issuer's.
    ///
    /// `None` REFUSES every write. Never fall back to the scope registry, and never leave writes
    /// open when unset: an open write surface on a public read endpoint is a free content host.
    ///
    /// [`MAX_MIRROR_OBJECTS`]: crate::mirror::MAX_MIRROR_OBJECTS
    /// [`MAX_MIRROR_TOTAL_BYTES`]: crate::mirror::MAX_MIRROR_TOTAL_BYTES
    pub mirror_ingest_token: Option<String>,
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

/// The watch configuration as READ, before any decision is taken about it. `main` fills this in from
/// the environment and nothing else; every rule lives in [`resolve_watch_generations`].
///
/// The split exists so the rules are testable at all. They used to live inside the env reader, which
/// meant exercising them required writing the PROCESS environment - shared by every test thread, so
/// the cases would have raced each other exactly as `vm.setEnv` does under forge's default threads.
/// A rule that can only be tested by a racing test is a rule that ends up untested.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WatchConfigInput {
    /// `INDEXER_GENERATIONS`, verbatim. `None` when the variable is absent.
    pub generations_json: Option<String>,
    pub factory: Option<String>,
    pub issuer_registry: Option<String>,
    pub verification_registry: Option<String>,
    pub seed_clones: Option<String>,
}

impl WatchConfigInput {
    /// The legacy singleton keys that are set, in the order they are reported to the operator.
    fn legacy_present(&self) -> Vec<&'static str> {
        [
            ("FACTORY_ADDR", &self.factory),
            ("ISSUER_REGISTRY_ADDR", &self.issuer_registry),
            (
                "VERIFICATION_REGISTRY_CONSENT_ADDR",
                &self.verification_registry,
            ),
            ("SEED_CLONES", &self.seed_clones),
        ]
        .into_iter()
        .filter(|(_, v)| v.is_some())
        .map(|(k, _)| k)
        .collect()
    }
}

/// What a resolution produced, so the caller can log the deprecation warning without re-deriving
/// which branch ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWatch {
    pub generations: Vec<WatchGeneration>,
    /// True when the deprecated singleton variables supplied the triple.
    pub used_legacy_singleton: bool,
}

/// Decide the watch set from what was configured. PURE - no environment, no I/O.
///
/// THERE ARE NO ADDRESS DEFAULTS, and an unconfigured live instance is an ERROR.
///
/// This used to fall back to the then-live triple, baked in as constants. That made "the operator
/// configured nothing" indistinguishable from "the operator configured exactly this" - and the
/// moment those contracts were superseded, that indistinguishable state became a scanner watching
/// addresses that decide nothing, while `/v1/status.watchedGenerations` reported them as a
/// deliberate choice. A stale default is worse than no default in a service whose whole job is to
/// report what it watched, because it cannot report a gap it was built to paper over.
///
/// `demo_generation` is the caller's SYNTHETIC stand-in, used only when nothing at all is
/// configured and the instance is scripted. It is a parameter rather than a constant here so the
/// library never carries an address of its own.
pub fn resolve_watch_generations(
    input: &WatchConfigInput,
    demo_generation: Option<WatchGeneration>,
) -> Result<ResolvedWatch, String> {
    if let Some(raw) = &input.generations_json {
        // The two forms may never coexist: accepting both recreates the split-brain where an
        // operator edits one address source while the scanner reads the other.
        let conflicts = input.legacy_present();
        if !conflicts.is_empty() {
            return Err(format!(
                "INDEXER_GENERATIONS cannot be combined with legacy {}",
                conflicts.join(", ")
            ));
        }
        return Ok(ResolvedWatch {
            generations: parse_watch_generations(raw)?,
            used_legacy_singleton: false,
        });
    }

    if input.legacy_present().is_empty() {
        // Nothing configured at all. A scripted instance supplies its own synthetic generation; a
        // live one has nothing to watch and says so rather than inventing something to watch.
        let demo_generation = demo_generation.ok_or_else(|| {
            "no watch configuration: set INDEXER_GENERATIONS to the exact \
             factory/issuerRegistry/verificationRegistry triple this deployment watches (addresses \
             come from contracts/deployments/roax.json). There is no default - a baked-in triple \
             would make an unconfigured scanner indistinguishable from a deliberately configured one"
                .to_string()
        })?;
        return Ok(ResolvedWatch {
            generations: validate_watch_generations(vec![demo_generation])?,
            used_legacy_singleton: false,
        });
    }

    // Every address of the legacy triple must be supplied. Defaulting the ones the operator omitted
    // would silently mix their configuration with ours, which is the same defect one variable down.
    let required = |key: &'static str, value: &Option<String>| -> Result<String, String> {
        value.clone().ok_or_else(|| {
            format!(
                "legacy singleton watch configuration is incomplete: {key} is unset. Supply all of \
                 FACTORY_ADDR, ISSUER_REGISTRY_ADDR and VERIFICATION_REGISTRY_CONSENT_ADDR, or \
                 migrate to INDEXER_GENERATIONS"
            )
        })
    };
    let factory = required("FACTORY_ADDR", &input.factory)?;
    let issuer_registry = required("ISSUER_REGISTRY_ADDR", &input.issuer_registry)?;
    let verification_registry = required(
        "VERIFICATION_REGISTRY_CONSENT_ADDR",
        &input.verification_registry,
    )?;
    // Seed clones legitimately default to NONE: an empty seed set is a complete statement (discover
    // every clone from `IssuerCreated`), unlike an omitted emitter address.
    let seed_clones = input
        .seed_clones
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|clone| !clone.is_empty())
        .map(str::to_string)
        .collect();
    Ok(ResolvedWatch {
        generations: validate_watch_generations(vec![watch_generation(
            &factory,
            &issuer_registry,
            &verification_registry,
            seed_clones,
        )?])?,
        used_legacy_singleton: true,
    })
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
    /// The S-17 content-addressed mirror: the bytes a `ProfileAnchor` digest names.
    ///
    /// A separate trait from `Store` on purpose. The event index and the content mirror have
    /// different lifecycles (one is rebuildable from the chain by rescanning, the other is not
    /// derivable from anything on chain — the chain holds only the digest), so folding blob methods
    /// into `Store` would force every index implementation to grow a storage concern it does not own.
    pub mirror: Arc<dyn ContentMirror>,
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

    /// The shipped template DECLARES the variable and ships it BLANK.
    ///
    /// It used to ship a full triple, and this test asserted that triple parsed. That was the wrong
    /// property to hold: a template carrying addresses opts every deployment that copies it into
    /// watching a set nobody chose for it, and those addresses go stale silently. Both halves below
    /// are load-bearing - declared, so an operator can see what to set; blank, so copying the file
    /// configures nothing by accident.
    #[test]
    fn the_shipped_env_template_declares_the_variable_and_ships_it_blank() {
        let example = include_str!("../../.env.example");
        let raw = example
            .lines()
            .find_map(|line| line.strip_prefix("INDEXER_GENERATIONS="))
            .expect("indexer env template must DECLARE INDEXER_GENERATIONS");
        assert_eq!(
            raw.trim(),
            "",
            "the template must ship no addresses: {raw:?}"
        );
        // And a blank value really does carry no configuration, rather than parsing as something.
        assert!(parse_watch_generations(raw).is_err());
    }

    /// No line of the shipped template may carry a 20-byte address at all - not as a value, and not
    /// inside the `INDEXER_GENERATIONS` JSON. `make check-addresses` guards this from outside for the
    /// whole tree; this is the same property asserted where this file's own reader can see it.
    #[test]
    fn the_shipped_env_template_carries_no_address_anywhere() {
        let example = include_str!("../../.env.example");
        for (n, line) in example.lines().enumerate() {
            let hex_runs = line.split("0x").skip(1).filter(|rest| {
                rest.chars().take(40).filter(|c| c.is_ascii_hexdigit()).count() == 40
            });
            assert_eq!(
                hex_runs.count(),
                0,
                "line {} of stacks/indexer/.env.example carries an address: {line}",
                n + 1
            );
        }
    }

    // ---- resolve_watch_generations: there is no address default -----------------------------
    //
    // Every case below drives the PURE decision. None of them writes the process environment, which
    // is what lets them run concurrently with each other and with every other test in this crate.

    fn demo_stand_in() -> WatchGeneration {
        watch_generation(
            "0x00000000000000000000000000000000000000fa",
            "0x00000000000000000000000000000000000000fb",
            "0x00000000000000000000000000000000000000fc",
            vec![],
        )
        .expect("synthetic demo generation")
    }

    #[test]
    fn an_unconfigured_live_instance_refuses_to_start_rather_than_watching_a_baked_in_triple() {
        let err = resolve_watch_generations(&WatchConfigInput::default(), None)
            .expect_err("nothing configured, and nothing may be invented");
        assert!(
            err.contains("INDEXER_GENERATIONS"),
            "the refusal must name the variable to set, got: {err}"
        );
        assert!(
            err.contains("no default"),
            "and must say plainly that there is no default, got: {err}"
        );
    }

    #[test]
    fn an_unconfigured_demo_instance_watches_its_own_synthetic_generation() {
        let resolved = resolve_watch_generations(&WatchConfigInput::default(), Some(demo_stand_in()))
            .expect("a scripted instance supplies its own generation");
        assert_eq!(resolved.generations.len(), 1);
        assert_eq!(
            resolved.generations[0].factory, "0x00000000000000000000000000000000000000fa",
            "the demo watches exactly the stand-in the caller passed, never a deployed address"
        );
        assert!(!resolved.used_legacy_singleton);
    }

    #[test]
    fn a_partial_legacy_singleton_configuration_names_the_missing_variable() {
        let input = WatchConfigInput {
            factory: Some(FACTORY_ONE.into()),
            ..Default::default()
        };
        // Passing the demo stand-in too: a partially-configured instance must NOT silently fall
        // through to it, because half the triple really was chosen by the operator.
        let err = resolve_watch_generations(&input, Some(demo_stand_in()))
            .expect_err("half a triple is not a triple");
        assert!(
            err.contains("ISSUER_REGISTRY_ADDR"),
            "name the first variable that is missing, got: {err}"
        );
    }

    #[test]
    fn a_complete_legacy_singleton_configuration_still_works_and_is_flagged_as_deprecated() {
        let input = WatchConfigInput {
            factory: Some(FACTORY_ONE.into()),
            issuer_registry: Some(REGISTRY_ONE.into()),
            verification_registry: Some(VREG_ONE.into()),
            seed_clones: Some("0x00000000000000000000000000000000000000D1".into()),
            ..Default::default()
        };
        let resolved =
            resolve_watch_generations(&input, None).expect("a complete legacy triple is accepted");
        assert_eq!(resolved.generations.len(), 1);
        assert_eq!(resolved.generations[0].seed_clones.len(), 1);
        assert!(
            resolved.used_legacy_singleton,
            "the caller needs this to emit the deprecation warning"
        );
    }

    #[test]
    fn an_empty_seed_clone_set_is_a_complete_statement_unlike_an_omitted_emitter() {
        let input = WatchConfigInput {
            factory: Some(FACTORY_ONE.into()),
            issuer_registry: Some(REGISTRY_ONE.into()),
            verification_registry: Some(VREG_ONE.into()),
            ..Default::default()
        };
        let resolved = resolve_watch_generations(&input, None)
            .expect("no seed clones means discover them all from IssuerCreated");
        assert!(resolved.generations[0].seed_clones.is_empty());
    }

    #[test]
    fn the_two_configuration_forms_may_never_coexist() {
        let input = WatchConfigInput {
            generations_json: Some(format!(
                r#"[{{"factory":"{FACTORY_ONE}","issuerRegistry":"{REGISTRY_ONE}","verificationRegistry":"{VREG_ONE}"}}]"#
            )),
            factory: Some(FACTORY_TWO.into()),
            ..Default::default()
        };
        let err = resolve_watch_generations(&input, None)
            .expect_err("a stale legacy value must not silently disagree with the atomic form");
        assert!(err.contains("FACTORY_ADDR"), "got: {err}");
    }
}
