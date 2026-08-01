//! indexer-api server entrypoint. Binds the Axum query API on port 46001 (config-overridable) and
//! runs the ingest loop in the background.
//!
//! Production wiring: `AlloyLogSource` (ROAX RPC `eth_getLogs`) + `MongoStore` (built with
//! `--features mongo`). Demo/local (`INDEXER_DEMO_MODE=1`) uses `MemLogSource` seeded with a scripted
//! event history + `MemStore`, so the full scan→index→query flow runs with no node and no Mongo.

use std::collections::HashMap;
use std::sync::Arc;

use indexer_api::app::{
    keccak_key, parse_watch_generations, validate_watch_generations, watch_generation, AppState,
    Config,
};
use indexer_api::chain::{AlloyLogSource, LogSource, MemLogSource};
use indexer_api::directory::Directory;
use indexer_api::indexer::Indexer;
use indexer_api::mirror::{ContentMirror, MemContentMirror};
use indexer_api::scope::{ScopeConfig, ScopeRegistry};
use indexer_api::store::{MemStore, Store};

// ROAX deployment defaults (contracts/deployments/roax.json), lowercased. The atomic
// INDEXER_GENERATIONS setting supersedes these; they remain the deprecated one-generation fallback.
const DEFAULT_FACTORY: &str = "0xed20269e3ebf0119739aab5258741f3aeb49f140";
const DEFAULT_REGISTRY: &str = "0xaee540350292e49a9aedf19dd4c3bac6abee6c21";
// Unified owner-hidden `VerificationRegistryConsent` (roax.json canonical pairing), lowercased.
const DEFAULT_VREG_CONSENT: &str = "0xabfd6f6e31780ebcb7abd28a2a9bcfc9c8e6a77b";
// The government TRAVEL_CLEARANCE clone on the FRESH owner-hidden set (roax.json government_clones).
// The earlier 0x8e276bd4… / 0xe30a1739… pair is bound to the RETIRED IssuerRegistry and quarantined as
// government_clones_deadRegistry_legacy - seeding those would name clones no issuance flows through.
// EU_HEALTH_CERT has no clone on the fresh set yet, so there is nothing to seed for it.
const GOV_TRAVEL_CLONE: &str = "0xb5d6654d8b29096c8fcf71d24bbe6f6de86c5f9f";
const DEMO_DOGPROFILE_CLONE: &str = "0x0e56ae2e1ef684d3e90d7699b981c6b76df922bf";
const DEMO_VACCINATION_CLONE: &str = "0x1456f93f7376789c46408cc4616751eb853edd9a";
// On testnet the government signer is the protocol admin address (roax.json government_clones note).
const GOV_SIGNER: &str = "0x119f8c7f6d7ec10e7376983739c6f46cf9cc3e96";
// A stand-in demo groomer signer that issues on the DOG_PROFILE clone (demo scoped-view data).
const DEMO_GROOMER_SIGNER: &str = "0x00000000000000000000000000000000c710f000";

/// A deprecated singleton watch variable, read as CONFIGURATION rather than as mere presence.
///
/// A blank value carries no configuration, so a deployment template that renders `FACTORY_ADDR=`
/// (Helm, systemd, a compose `environment:` interpolation) has not set it. Treating presence alone as
/// configuration made such a template refuse to boot, blaming legacy variables the operator never
/// set - and, on the fallback path, fail address parsing with a confusing "must be a 20-byte address".
/// The fail-closed property is untouched: a blank value cannot silently disagree with
/// `INDEXER_GENERATIONS`, because it states nothing to disagree with.
fn legacy_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Load the atomic generation-set configuration. The old four-variable shape remains a
/// singleton-only compatibility fallback, but the two forms may never coexist: accepting both would
/// recreate the split-brain where an operator edits one address source while the scanner reads the
/// other.
fn load_watch_generations() -> Result<Vec<indexer_api::chain::WatchGeneration>, String> {
    const LEGACY_KEYS: [&str; 4] = [
        "FACTORY_ADDR",
        "ISSUER_REGISTRY_ADDR",
        "VERIFICATION_REGISTRY_CONSENT_ADDR",
        "SEED_CLONES",
    ];

    match std::env::var("INDEXER_GENERATIONS") {
        Ok(raw) => {
            let conflicts: Vec<&str> = LEGACY_KEYS
                .into_iter()
                .filter(|key| legacy_env(key).is_some())
                .collect();
            if !conflicts.is_empty() {
                return Err(format!(
                    "INDEXER_GENERATIONS cannot be combined with legacy {}",
                    conflicts.join(", ")
                ));
            }
            parse_watch_generations(&raw)
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("INDEXER_GENERATIONS is not valid UTF-8".into())
        }
        Err(std::env::VarError::NotPresent) => {
            let legacy_configured = LEGACY_KEYS.into_iter().any(|key| legacy_env(key).is_some());
            if legacy_configured {
                tracing::warn!(
                    "legacy singleton indexer watch variables are deprecated; migrate the exact \
                     triple + seed clones to INDEXER_GENERATIONS"
                );
            }
            let factory = legacy_env("FACTORY_ADDR").unwrap_or_else(|| DEFAULT_FACTORY.to_string());
            let issuer_registry =
                legacy_env("ISSUER_REGISTRY_ADDR").unwrap_or_else(|| DEFAULT_REGISTRY.to_string());
            let verification_registry = legacy_env("VERIFICATION_REGISTRY_CONSENT_ADDR")
                .unwrap_or_else(|| DEFAULT_VREG_CONSENT.to_string());
            let seed_clones = legacy_env("SEED_CLONES")
                .unwrap_or_else(|| {
                    [GOV_TRAVEL_CLONE, DEMO_DOGPROFILE_CLONE, DEMO_VACCINATION_CLONE].join(",")
                })
                .split(',')
                .map(str::trim)
                .filter(|clone| !clone.is_empty())
                .map(str::to_string)
                .collect();
            validate_watch_generations(vec![watch_generation(
                &factory,
                &issuer_registry,
                &verification_registry,
                seed_clones,
            )?])
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let truthy = |k: &str| matches!(env(k, "").as_str(), "1" | "true" | "TRUE");
    let demo = truthy("INDEXER_DEMO_MODE") || truthy("DEMO_MODE") || truthy("VITE_DEMO_MODE");
    let port: u16 = env("PORT", "46001").parse().unwrap_or(46001);
    let rpc_url = env("ROAX_RPC", "https://devrpc.roax.net");
    let chain_id: u64 = env("CHAIN_ID", "135").parse().unwrap_or(135);

    let generations = load_watch_generations().unwrap_or_else(|error| {
        tracing::error!("invalid indexer watch configuration: {error}; refusing to start");
        std::process::exit(1);
    });

    let cfg = Config {
        rpc_url: rpc_url.clone(),
        generations,
        start_block: env("START_BLOCK", "0").parse().unwrap_or(0),
        // Demo has no reorgs and a scripted head — index everything immediately.
        confirmations: env("CONFIRMATIONS", if demo { "0" } else { "5" })
            .parse()
            .unwrap_or(if demo { 0 } else { 5 }),
        chunk_size: env("CHUNK_SIZE", "5000").parse().unwrap_or(5000),
        poll_interval_secs: env("POLL_INTERVAL_SECS", if demo { "2" } else { "12" })
            .parse()
            .unwrap_or(12),
        default_page_limit: env("DEFAULT_PAGE_LIMIT", "100").parse().unwrap_or(100),
        max_page_limit: env("MAX_PAGE_LIMIT", "1000").parse().unwrap_or(1000),
        explorer_base: env("EXPLORER_BASE", "https://explorer.roax.net"),
    };

    // --- scope registry (token -> {unscoped | signers/clones}) ---------------------------------
    let scopes = build_scopes(demo);
    if scopes.is_empty() {
        tracing::warn!(
            "no INDEXER_SCOPES configured and not in demo mode — every query will 401 until at least \
             one token→scope mapping is provided (fail-closed)"
        );
    } else {
        tracing::info!("loaded {} scope token(s)", scopes.len());
    }

    // --- signer→business directory -------------------------------------------------------------
    let directory = Arc::new(build_directory(demo));

    // --- store ---------------------------------------------------------------------------------
    let store: Arc<dyn Store> = build_store(demo).await;

    // --- log source ----------------------------------------------------------------------------
    let source: Arc<dyn LogSource> = if demo {
        let mem = MemLogSource::new();
        demo_seed(&mem, &cfg);
        tracing::info!("INDEXER_DEMO_MODE: using scripted in-memory MemLogSource (no live node)");
        Arc::new(mem)
    } else {
        Arc::new(AlloyLogSource::new(rpc_url).with_chain_id(chain_id))
    };

    // --- content mirror ------------------------------------------------------------------------
    // In memory for now, so the mirror is ephemeral and a restart empties it. That is honest rather
    // than convenient: content addressing means a lost object is re-publishable byte-for-byte by
    // whoever holds the original, and a missing address answers 404 (a fact) rather than a wrong
    // answer. A durable store slots in behind `ContentMirror` without touching either route.
    let mirror: Arc<dyn ContentMirror> = Arc::new(MemContentMirror::new());

    let state = AppState {
        store,
        source,
        scopes: Arc::new(scopes),
        directory: directory.clone(),
        mirror,
        cfg: Arc::new(cfg),
    };

    // --- background: directory refresh loop ----------------------------------------------------
    if directory.has_admin() {
        let dir = directory.clone();
        tokio::spawn(async move {
            loop {
                dir.refresh().await;
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        });
    }

    // --- background: ingest loop ---------------------------------------------------------------
    let indexer = Arc::new(Indexer::new(state.clone()));
    // Fail closed, like `build_store`: an index we cannot read yields an EMPTY clone-trust set, which
    // silently drops every `RootIssued`/`RootRevoked` from pre-restart clones while `/v1/status`
    // reports a complete watch set and zero lag. Refuse to start rather than serve that.
    if let Err(e) = indexer.rebuild_known_clones().await {
        tracing::error!(
            "could not rebuild the clone-trust set from the event index: {e}; refusing to start"
        );
        std::process::exit(1);
    }
    {
        let ix = indexer.clone();
        tokio::spawn(async move { ix.run().await });
    }

    // --- serve ---------------------------------------------------------------------------------
    let cors = build_cors();
    // The library router owns response compression so every embedding gets the same directory-page
    // transport behavior; the binary adds only its deployment-specific CORS policy.
    let app = indexer_api::router(state).layer(cors);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("indexer-api listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app.into_make_service())
        .await
        .expect("serve");
}

/// Parse `INDEXER_SCOPES` (JSON array of `ScopeConfig`). In demo, fall back to a well-known unscoped
/// oversight token + a scoped example bound to the demo DOG_PROFILE clone.
fn build_scopes(demo: bool) -> ScopeRegistry {
    if let Ok(raw) = std::env::var("INDEXER_SCOPES") {
        if !raw.trim().is_empty() {
            match serde_json::from_str::<Vec<ScopeConfig>>(&raw) {
                Ok(cfgs) => return ScopeRegistry::from_configs(cfgs),
                Err(e) => tracing::error!("INDEXER_SCOPES is not valid JSON: {e}; ignoring"),
            }
        }
    }
    if demo {
        return ScopeRegistry::from_configs(vec![
            ScopeConfig {
                token: "dogtag-indexer-oversight-demo-token".into(),
                label: "government-oversight".into(),
                unscoped: true,
                signers: vec![],
                clones: vec![],
            },
            ScopeConfig {
                token: "dogtag-indexer-vet-demo-token".into(),
                label: "demo-groomer (DOG_PROFILE clone)".into(),
                unscoped: false,
                signers: vec![DEMO_GROOMER_SIGNER.into()],
                clones: vec![DEMO_DOGPROFILE_CLONE.into()],
            },
        ]);
    }
    ScopeRegistry::default()
}

/// Build the directory from `INDEXER_DIRECTORY` (JSON `{addr: name}`) + admin-API enrichment env. In
/// demo, seed names for the government authority + its clones so the feed reads with human names.
fn build_directory(demo: bool) -> Directory {
    let mut static_map: HashMap<String, String> = HashMap::new();
    if let Ok(raw) = std::env::var("INDEXER_DIRECTORY") {
        if !raw.trim().is_empty() {
            match serde_json::from_str::<HashMap<String, String>>(&raw) {
                Ok(m) => static_map.extend(m),
                Err(e) => tracing::error!("INDEXER_DIRECTORY is not valid JSON: {e}; ignoring"),
            }
        }
    }
    if demo {
        static_map
            .entry(GOV_SIGNER.into())
            .or_insert_with(|| "DogTag Government Authority".into());
        static_map
            .entry(GOV_TRAVEL_CLONE.into())
            .or_insert_with(|| "Government TRAVEL_CLEARANCE issuer".into());
        static_map
            .entry(DEMO_DOGPROFILE_CLONE.into())
            .or_insert_with(|| "Demo DOG_PROFILE issuer".into());
        static_map
            .entry(DEMO_GROOMER_SIGNER.into())
            .or_insert_with(|| "The Seaport Paw (demo groomer)".into());
    }
    let admin_base = std::env::var("ADMIN_API_BASE").ok();
    let admin_token = std::env::var("ADMIN_API_TOKEN").ok();
    Directory::new(static_map, admin_base, admin_token)
}

/// Build the backing store. `MONGO_URI` set & non-empty (and NOT demo) ⇒ MongoStore (fail-closed on
/// connect error). Otherwise ⇒ ephemeral MemStore.
async fn build_store(demo: bool) -> Arc<dyn Store> {
    let uri = std::env::var("MONGO_URI").unwrap_or_default();
    if demo || uri.trim().is_empty() {
        return Arc::new(MemStore::new());
    }
    #[cfg(feature = "mongo")]
    {
        let db = std::env::var("MONGO_DB").unwrap_or_else(|_| "dogtag".to_string());
        match indexer_api::mongo::MongoStore::connect(&uri, &db).await {
            Ok(s) => {
                tracing::info!("connected to MongoStore (db={db})");
                Arc::new(s)
            }
            Err(e) => {
                tracing::error!("MONGO_URI set but MongoStore::connect failed: {e}; refusing to start");
                std::process::exit(1);
            }
        }
    }
    #[cfg(not(feature = "mongo"))]
    {
        tracing::error!(
            "MONGO_URI is set but this binary was built WITHOUT the `mongo` feature; \
             rebuild with --features mongo or unset MONGO_URI. Refusing to start."
        );
        std::process::exit(1);
    }
}

/// Script a small event history into the demo MemLogSource so the scan→index→query flow shows real,
/// decoded, non-PII oversight data end-to-end (deploy → whitelist → issue×2 → verify → revoke).
fn demo_seed(mem: &MemLogSource, cfg: &Config) {
    use indexer_api::chain::emit;
    let generation = cfg
        .generations
        .first()
        .expect("watch generation validation guarantees a non-empty set");
    let rt_travel = keccak_key("TRAVEL_CLEARANCE");
    let purpose = keccak_key("boarding_intake");
    let root1 = "0x1111111111111111111111111111111111111111111111111111111111111111";
    let root2 = "0x2222222222222222222222222222222222222222222222222222222222222222";
    let null1 = "0x3333333333333333333333333333333333333333333333333333333333333333";
    let base_ts = 1_782_864_000u64; // 2026-07-01T00:00:00Z, matches the sibling MemChain clock base

    mem.push_empty_block("0x00", base_ts); // genesis
    mem.push_events(
        "0x01",
        base_ts + 12,
        vec![emit::issuer_created(
            &generation.factory,
            GOV_TRAVEL_CLONE,
            &rt_travel,
            "Government TRAVEL_CLEARANCE",
        )],
    );
    mem.push_events(
        "0x02",
        base_ts + 24,
        vec![emit::whitelisted(
            &generation.issuer_registry,
            &rt_travel,
            GOV_SIGNER,
        )],
    );
    mem.push_events(
        "0x03",
        base_ts + 36,
        vec![emit::root_issued(GOV_TRAVEL_CLONE, root1, GOV_SIGNER, base_ts + 36)],
    );
    mem.push_events(
        "0x04",
        base_ts + 48,
        vec![emit::root_issued(GOV_TRAVEL_CLONE, root2, GOV_SIGNER, base_ts + 48)],
    );
    mem.push_events(
        "0x05",
        base_ts + 60,
        vec![emit::verified(
            &generation.verification_registry,
            42,
            GOV_SIGNER,
            &purpose,
            null1,
            base_ts + 3600,
            base_ts + 60,
        )],
    );
    mem.push_events(
        "0x06",
        base_ts + 72,
        vec![emit::root_revoked(GOV_TRAVEL_CLONE, root1, GOV_SIGNER, base_ts + 72)],
    );
    // A separate demo groomer issuing on the DOG_PROFILE clone — populates the SCOPED view so the
    // vet/groomer demo token returns its own (and only its own) on-chain activity.
    let rt_dogprofile = keccak_key("DOG_PROFILE");
    let root3 = "0x4444444444444444444444444444444444444444444444444444444444444444";
    mem.push_events(
        "0x07",
        base_ts + 84,
        vec![emit::issuer_created(
            &generation.factory,
            DEMO_DOGPROFILE_CLONE,
            &rt_dogprofile,
            "Demo DOG_PROFILE",
        )],
    );
    mem.push_events(
        "0x08",
        base_ts + 96,
        vec![emit::root_issued(
            DEMO_DOGPROFILE_CLONE,
            root3,
            DEMO_GROOMER_SIGNER,
            base_ts + 96,
        )],
    );
    // Finalize through block 6 (the government flow): blocks 7-8 (the demo-groomer DOG_PROFILE
    // issuance) stay PENDING, so the demo shows the finalized/pending lifecycle in the feed.
    mem.set_finalized(6);
}

/// CORS: explicit allowlist from `CORS_ALLOW_ORIGINS` (comma-separated) when set, else permissive.
fn build_cors() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{Any, CorsLayer};
    match std::env::var("CORS_ALLOW_ORIGINS") {
        Ok(s) if !s.trim().is_empty() => {
            let origins: Vec<axum::http::HeaderValue> = s
                .split(',')
                .map(|o| o.trim())
                .filter(|o| !o.is_empty())
                .filter_map(|o| o.parse().ok())
                .collect();
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods(Any)
                .allow_headers(Any)
        }
        _ => CorsLayer::permissive(),
    }
}
