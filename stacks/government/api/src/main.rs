//! government-api server entrypoint. Binds the Axum router on port 44832 (config-overridable).
//!
//! Two independent axes, deliberately NOT collapsed into one `demo` switch:
//!
//!   - CHAIN  — `GOV_CHAIN_BACKEND`: `live` (default, real ROAX RPC via `AlloyChain`) or `mem`
//!     (explicit opt-in simulation via `MemChain`). The default is ALWAYS the real node, and `/health`
//!     reports which one is in use, so a simulated stack can never pass for live.
//!   - STORE  — `GOV_DEMO_MODE`/`DEMO_MODE` + `MONGO_URI`: ephemeral `MemStore` for demo/local,
//!     `MongoStore` for production (built with `--features mongo`).
//!
//! Collapsing them is what produced the "simulated chain reporting `chainId:135`, `canSign:true`" bug:
//! a demo stack wanted an ephemeral STORE and silently got a simulated CHAIN as well.

use std::sync::Arc;

use government_api::app::{AppState, Config};
use government_api::chain::{AlloyChain, ChainClient, MemChain};
use government_api::oversight::{DisabledFeed, HttpOversightFeed, OversightFeed};
use government_api::store::{MemStore, Store};

#[tokio::main]
async fn main() {
    // Default to info for this crate (mirrors admin-api). `from_default_env()` alone defaults to ERROR,
    // which silently swallowed the startup lines that say WHICH chain backend is in use - the operator
    // signal this service most needs to emit. RUST_LOG still overrides.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "government_api=info,tower_http=info".into()),
        )
        .init();

    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let truthy = |k: &str| matches!(env(k, "").as_str(), "1" | "true" | "TRUE");

    let port: u16 = env("PORT", "44832").parse().unwrap_or(44832);
    let rpc_url = env("ROAX_RPC", "https://devrpc.roax.net");
    let chain_id: u64 = env("CHAIN_ID", "135").parse().unwrap_or(135);
    // Demo mode: ephemeral MemStore + the well-known API-token fallback. It does NOT choose the chain
    // backend (that is GOV_CHAIN_BACKEND) — see the module doc. Production leaves GOV_DEMO_MODE unset.
    let demo = truthy("GOV_DEMO_MODE") || truthy("VITE_DEMO_MODE") || truthy("DEMO_MODE");

    // Bearer token gating the record MUTATION endpoints (PATCH + revoke). Demo mode falls back to
    // the well-known demo token; production without GOV_API_TOKEN fails closed on mutations (503).
    let api_token = match std::env::var("GOV_API_TOKEN") {
        Ok(t) if !t.trim().is_empty() => Some(t.trim().to_string()),
        _ if demo => Some("dogtag-gov-demo-token".to_string()),
        _ => None,
    };
    if api_token.is_none() {
        tracing::warn!(
            "no GOV_API_TOKEN configured - record mutation endpoints (PATCH /v1/records/:root, \
             POST /v1/records/:root/revoke) will refuse with 503 until it is set"
        );
    }

    let cfg = Config {
        deployment_url: env("DEPLOYMENT_URL", &format!("http://localhost:{port}")),
        rpc_url: rpc_url.clone(),
        chain_id,
        issuer_registry_addr: env(
            "ISSUER_REGISTRY_ADDR",
            "0x0000000000000000000000000000000000000000",
        ),
        // LINK 1 of the issuer↔domain chain: the factory that KYC-gated this clone into existence.
        factory_addr: env("FACTORY_ADDR", "0x0000000000000000000000000000000000000000"),
        // The ADDITIVE issuer↔domain claim registry. Deliberately NOT resolved from the credential
        // document: the point of reading the domain from the chain is that a relabelled document cannot
        // move it, so the registry address must come from deployment config, at the same trust level as
        // ISSUER_REGISTRY_ADDR and the RPC URL.
        issuer_domain_registry_addr: env(
            "ISSUER_DOMAIN_REGISTRY_ADDR",
            "0x0000000000000000000000000000000000000000",
        ),
        dns_doh_endpoint: env("DNS_DOH_ENDPOINT", "https://cloudflare-dns.com/dns-query"),
        // Unified owner-hidden verification registry: both the routing key stamped in the M7 `protocol`
        // block AND the submit target when this authority records a consent proof as a VERIFIER. The
        // sibling stacks name the SAME contract `VERIFICATION_REGISTRY_CONSENT_ADDR`, so accept that as
        // an alias — a compose file that sets only the sibling name must not silently fall back to the
        // baked default and then have every phone scan refuse on an anchor mismatch.
        verification_registry_addr: std::env::var("VERIFICATION_REGISTRY_ADDR")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                std::env::var("VERIFICATION_REGISTRY_CONSENT_ADDR")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
            })
            .unwrap_or_else(|| "0xaBFd6f6E31780EBcB7ABd28A2a9bCfc9C8e6A77B".to_string()),
        travel_clearance_issuer_addr: env(
            "TRAVEL_CLEARANCE_ISSUER_ADDR",
            "0x0000000000000000000000000000000000000000",
        ),
        eu_health_cert_issuer_addr: env(
            "EU_HEALTH_CERT_ISSUER_ADDR",
            "0x0000000000000000000000000000000000000000",
        ),
        issuer_name: env("ISSUER_NAME", "DogTag Government Authority"),
        issuer_domain: env("ISSUER_DOMAIN", "gov.example"),
        demo,
        api_token,
    };

    // Chain client selection — DELIBERATELY INDEPENDENT OF `demo`.
    //
    // `demo` used to select MemChain, which meant a demo stack silently ran its verify/records surfaces
    // on a simulated chain while `CHAIN_ID=135` was still reported. Simulation is now an EXPLICIT
    // opt-in via `GOV_CHAIN_BACKEND=mem`; the default is always the real node, so nobody gets a
    // simulated chain by accident. `demo` continues to govern the store and the API-token fallback.
    let backend_pref = env("GOV_CHAIN_BACKEND", "live").trim().to_lowercase();
    let use_mem = match backend_pref.as_str() {
        "live" | "alloy" | "rpc" => false,
        "mem" | "memory" | "simulated" | "sim" => true,
        other => {
            eprintln!(
                "FATAL: GOV_CHAIN_BACKEND={other:?} is not recognised. Use \"live\" (real RPC node, \
                 the default) or \"mem\" (in-process simulation; nothing is broadcast)."
            );
            std::process::exit(1);
        }
    };

    let chain: Arc<dyn ChainClient> = if use_mem {
        let mem = MemChain::new();
        // Pre-whitelist the demo signer for both record types so the verify path can demonstrate the
        // issuer-identity pillar end-to-end without an admin round-trip.
        if let Some(signer) = mem.signer_address() {
            for rt in [
                government_api::app::TRAVEL_CLEARANCE,
                government_api::app::EU_HEALTH_CERT,
            ] {
                mem.whitelist(
                    &cfg.issuer_registry_addr,
                    &government_api::app::record_type_key(rt),
                    &signer,
                );
            }
            // ...and for the VERIFY: purposes the portal offers, so the owner-hidden QR flow is
            // demoable too. `VERIFY:` is a namespace SEPARATE from the issuer record types above: on a
            // live deployment these are granted by the admin apply→approve flow, never by this stack.
            for purpose in government_api::app::VERIFY_PURPOSES {
                mem.whitelist(
                    &cfg.issuer_registry_addr,
                    &government_api::verify::verify_key(purpose),
                    &signer,
                );
            }
        }
        tracing::warn!(
            "GOV_CHAIN_BACKEND=mem — SIMULATED chain (in-process MemChain). No live node, no gas, \
             NOTHING IS BROADCAST and nothing survives this process. /health reports \
             backend=\"simulated\" with chainId=null so this is never mistaken for {chain_id}."
        );
        Arc::new(mem)
    } else {
        let mut alloy = AlloyChain::new(rpc_url).with_chain_id(chain_id);
        // Load the government signer (32-byte hex private key) when configured. Reads work without it;
        // on-chain issuance requires it. A malformed key fails closed (refuses to boot).
        if let Ok(key) = std::env::var("GOV_SIGNER_KEY") {
            if !key.trim().is_empty() {
                match alloy.with_signer_hex(&key) {
                    Ok(a) => {
                        alloy = a;
                        tracing::info!(
                            "loaded government signer {}",
                            alloy.signer_address().unwrap_or_default()
                        );
                    }
                    Err(e) => {
                        eprintln!("FATAL: GOV_SIGNER_KEY is set but invalid: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        if !alloy.can_sign() {
            tracing::warn!(
                "no GOV_SIGNER_KEY configured — LIVE chain reads work, but /issue CANNOT anchor \
                 on-chain and will only build+persist via dry_run. On-chain issuance needs a funded \
                 GOV_SIGNER_KEY whitelisted for the record type (scripts/demo-provision-government.sh)."
            );
        } else {
            tracing::info!(
                "LIVE chain backend: {} (chainId {chain_id}), signer {}",
                alloy.rpc_url,
                alloy.signer_address().unwrap_or_default()
            );
        }
        Arc::new(alloy)
    };

    let store: Arc<dyn Store> = build_store(demo).await;

    // Oversight-indexer consumer (govarch PR-5, the oversight console's data layer): the UNSCOPED
    // cross-issuer feed. Wired when INDEXER_API_BASE is set (with INDEXER_OVERSIGHT_TOKEN — the
    // indexer's `unscoped:true` bearer); otherwise DisabledFeed → the /v1/oversight/* surfaces 503.
    let feed: Arc<dyn OversightFeed> = build_feed();

    // The server-side DNS resolver, shared so its TTL cache is shared. There is no fixture behind it:
    // every binding state a client sees is a real resolution or a real failure.
    let dns = Arc::new(dogtag_dns_rs::BindingResolver::production(
        cfg.dns_doh_endpoint.clone(),
    ));
    let state = AppState {
        store,
        chain,
        cfg: Arc::new(cfg),
        dns,
        feed,
    };

    let cors = build_cors();
    let app = government_api::router(state).layer(cors);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("government-api listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app.into_make_service())
        .await
        .expect("serve");
}

/// Build the UNSCOPED oversight-indexer consumer (govarch PR-5). With `INDEXER_API_BASE` set: a real
/// `HttpOversightFeed` presenting the UNSCOPED bearer from `INDEXER_OVERSIGHT_TOKEN` (an
/// `INDEXER_SCOPES` entry with `unscoped:true`; `GOV_INDEXER_TOKEN` is accepted as an alias). Unset
/// base → a `DisabledFeed` so the `/v1/oversight/*` surfaces return 503 while the rest of the backend
/// runs. Demo wiring (base + token) is supplied by `scripts/demo-up.sh`, matching the admin stack.
fn build_feed() -> Arc<dyn OversightFeed> {
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty());
    match env("INDEXER_API_BASE") {
        Some(base) => {
            let token = env("INDEXER_OVERSIGHT_TOKEN")
                .or_else(|| env("GOV_INDEXER_TOKEN"))
                .unwrap_or_default();
            if token.is_empty() {
                tracing::warn!(
                    "INDEXER_API_BASE set but no INDEXER_OVERSIGHT_TOKEN; the indexer will 401 unless \
                     it is running in demo mode (fail-closed scope registry)"
                );
            }
            tracing::info!("oversight indexer consumer wired to {base} (unscoped)");
            Arc::new(HttpOversightFeed::new(base, token))
        }
        None => {
            tracing::warn!(
                "INDEXER_API_BASE unset; the oversight surfaces (/v1/oversight/*) return 503 until an \
                 indexer is configured"
            );
            Arc::new(DisabledFeed)
        }
    }
}

/// Build the backing store. With `MONGO_URI` set & non-empty (and NOT demo): persistent MongoStore
/// (fail-closed on connect error). Otherwise: ephemeral MemStore.
async fn build_store(demo: bool) -> Arc<dyn Store> {
    let uri = std::env::var("MONGO_URI").unwrap_or_default();
    if demo || uri.trim().is_empty() {
        return Arc::new(MemStore::new());
    }

    #[cfg(feature = "mongo")]
    {
        let db = std::env::var("MONGO_DB").unwrap_or_else(|_| "dogtag".to_string());
        match government_api::mongo::MongoStore::connect(&uri, &db).await {
            Ok(s) => {
                tracing::info!("connected to MongoStore (db={db})");
                Arc::new(s)
            }
            Err(e) => {
                tracing::error!(
                    "MONGO_URI set but MongoStore::connect failed: {e}; refusing to start"
                );
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
