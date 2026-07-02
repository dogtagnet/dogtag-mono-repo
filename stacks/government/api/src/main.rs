//! government-api server entrypoint. Binds the Axum router on port 44832 (config-overridable).
//!
//! Production wiring: `AlloyChain` (ROAX RPC) + `MongoStore` (built with `--features mongo`). Demo/
//! local (`GOV_DEMO_MODE=1`) uses `MemChain` + `MemStore` so the full issue/verify flow runs with no
//! node, no gas, and no Mongo.

use std::sync::Arc;

use government_api::app::{AppState, Config};
use government_api::chain::{AlloyChain, ChainClient, MemChain};
use government_api::store::{MemStore, Store};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let truthy = |k: &str| matches!(env(k, "").as_str(), "1" | "true" | "TRUE");

    let port: u16 = env("PORT", "44832").parse().unwrap_or(44832);
    let rpc_url = env("ROAX_RPC", "https://devrpc.roax.net");
    let chain_id: u64 = env("CHAIN_ID", "135").parse().unwrap_or(135);
    // Demo mode: MemChain + MemStore, relaxed. Also implied when neither a signer key nor MONGO_URI
    // is configured AND GOV_DEMO_MODE is set. Production leaves GOV_DEMO_MODE unset.
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

    // Chain client selection.
    let chain: Arc<dyn ChainClient> = if demo {
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
        }
        tracing::info!("GOV_DEMO_MODE: using in-memory MemChain (no live node, no gas)");
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
                "no GOV_SIGNER_KEY configured — verify (read-only) works, but /issue cannot anchor \
                 on-chain (build+persist only via dry_run)"
            );
        }
        Arc::new(alloy)
    };

    let store: Arc<dyn Store> = build_store(demo).await;

    let state = AppState {
        store,
        chain,
        cfg: Arc::new(cfg),
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
