//! DogTag central/admin backend entrypoint. Binds Axum on port 39742 (impl §4).

use std::sync::Arc;

use admin_api::app::{AppState, Config};
use admin_api::auth::JwtKeys;
use admin_api::business::ReqwestBusinessClient;
use admin_api::chain::{AlloyChain, ChainClient};
use admin_api::crypto::MemVault;
use admin_api::dns::{DnsChecker, DohDnsChecker, MockDnsChecker};
use admin_api::store::{MemStore, Store};
use tower_http::cors::CorsLayer;

const PORT: u16 = 39742;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "admin_api=info,tower_http=info".into()),
        )
        .init();

    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let rpc_url = env("ROAX_RPC", "http://127.0.0.1:8545");
    // CHAIN_ID is env-driven so a different/production chain is a pure config swap (default 135 = ROAX).
    let chain_id: u64 = std::env::var("CHAIN_ID")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(135);

    // Kept as a local (not a Config field) so the H2 boot-guard can check the plaintext while Config
    // stores only the hash (audit L4).
    let admin_password = env("ADMIN_PASSWORD", "admin-pw");
    let cfg = Config {
        deployment_url: env("DEPLOYMENT_URL", &format!("http://localhost:{PORT}")),
        rpc_url: rpc_url.clone(),
        issuer_registry_addr: env(
            "ISSUER_REGISTRY_ADDR",
            "0x0000000000000000000000000000000000000000",
        ),
        sbt_addr: env("SBT_ADDR", "0x0000000000000000000000000000000000000000"),
        issuer_name: env("ISSUER_NAME", "DogTag Central"),
        issuer_domain: env("ISSUER_DOMAIN", "dogtag.example"),
        profile_document_store: env(
            "PROFILE_DOCUMENT_STORE",
            "0x0000000000000000000000000000000000000000",
        ),
        // Store a real password HASH, never the plaintext (audit L4) — admin_login verifies against
        // this with auth::verify_password. Optional `ADMIN_PASSWORD_HASH` ("<salt_hex>$<hash_hex>")
        // overrides; otherwise the ADMIN_PASSWORD plaintext (still required non-default in prod by the
        // H2 boot-guard below) is hashed once here at startup.
        admin_password_hash: std::env::var("ADMIN_PASSWORD_HASH")
            .ok()
            .filter(|h| !h.trim().is_empty())
            .map(|h| h.trim().to_string())
            .unwrap_or_else(|| admin_api::auth::hash_password(&admin_password)),
        admin_signer_index: 0,
    };

    // Fail-closed (audit H2): refuse to boot in production with an unset/dev-default ADMIN_PASSWORD or
    // an unset ADMIN_PRIVATE_KEY (they guard whitelisting, erasure, and every on-chain admin write).
    // The local/demo path (DEMO_MODE / VITE_DEMO_MODE set) keeps the convenient defaults.
    let admin_private_key = std::env::var("ADMIN_PRIVATE_KEY").unwrap_or_default();
    let demo = admin_api::startup::is_demo_mode();
    if let Err(e) = admin_api::startup::validate_production_secrets(
        demo,
        &[
            admin_api::startup::SecretSpec {
                name: "ADMIN_PASSWORD",
                value: admin_password.as_str(),
                dev_default: "admin-pw",
            },
            admin_api::startup::SecretSpec {
                name: "ADMIN_PRIVATE_KEY",
                value: admin_private_key.as_str(),
                dev_default: "",
            },
        ],
    ) {
        eprintln!("FATAL: {e}");
        std::process::exit(1);
    }

    // Wire the admin/WHITELIST_ADMIN+ISSUER signer at the configured index from ADMIN_PRIVATE_KEY so
    // whitelistFor/delistFor/mint can broadcast on-chain. Without this the chain client has no signer
    // and every admin write fails with "no signer for index". (The custody stacks unlock their own
    // signers; the central stack's signer is a static deployer key supplied at boot.)
    let chain = AlloyChain::new(rpc_url).with_chain_id(chain_id);
    if !admin_private_key.trim().is_empty() {
        let pk_hex = admin_private_key
            .trim()
            .strip_prefix("0x")
            .unwrap_or(admin_private_key.trim());
        match hex::decode(pk_hex) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut pk = [0u8; 32];
                pk.copy_from_slice(&bytes);
                let addr = std::env::var("ADMIN_ADDRESS").unwrap_or_default();
                chain
                    .register_signer(cfg.admin_signer_index, pk, addr)
                    .await;
                tracing::info!(
                    "admin signer registered at index {}",
                    cfg.admin_signer_index
                );
            }
            _ => tracing::warn!(
                "ADMIN_PRIVATE_KEY set but not a 32-byte hex key; admin writes will fail"
            ),
        }
    } else {
        tracing::warn!(
            "ADMIN_PRIVATE_KEY unset; on-chain admin writes (whitelistFor/mint) will fail"
        );
    }

    // DNS legitimacy check: real DoH in prod; set DNS_CHECK=skip for the local demo where the
    // business domain (e.g. vet.local) has no published TXT record.
    let dns: Arc<dyn DnsChecker> = if env("DNS_CHECK", "doh") == "skip" {
        tracing::warn!("DNS_CHECK=skip: DNS TXT legitimacy verification is BYPASSED (demo only)");
        Arc::new(MockDnsChecker::ok())
    } else {
        Arc::new(DohDnsChecker::default())
    };

    // Store selection: persistent MongoStore when MONGO_URI is set (fail-closed), else ephemeral
    // MemStore (demo/local — unchanged). Demo behavior is preserved when MONGO_URI is unset/empty.
    let store: Arc<dyn Store> = build_store().await;

    let state = AppState {
        store,
        chain: Arc::new(chain),
        dns,
        business: Arc::new(ReqwestBusinessClient::new()),
        vault: Arc::new(MemVault::new()),
        // Shared JWT signing key from SHARE_JWT_SIGNING_KEY (audit L4) so share tokens survive restart
        // and work across instances; fail closed when missing in production (same DEMO_MODE signal as
        // the H2 secret guard above).
        jwt: load_jwt_keys(!demo),
        cfg: Arc::new(cfg),
        ratelimit: Arc::new(admin_api::auth::RateLimiter::new()),
    };

    // CORS: explicit allowlist when CORS_ALLOW_ORIGINS is set (prod), else permissive (demo).
    let cors = build_cors();

    // Admin-router loopback isolation (ADMIN_LOOPBACK_ONLY): when truthy, the public 0.0.0.0:PORT
    // listener omits the admin-console routes, which are served on a separate 127.0.0.1:ADMIN_PORT
    // listener (default PORT+1). Default (unset): everything on one listener exactly as today.
    let admin_loopback = matches!(env("ADMIN_LOOPBACK_ONLY", "").as_str(), "1" | "true");

    if admin_loopback {
        let admin_port: u16 = std::env::var("ADMIN_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(PORT + 1);

        let public_app = admin_api::public_router(state.clone()).layer(cors.clone());
        let admin_app = admin_api::admin_router(state).layer(cors);

        let public_addr = std::net::SocketAddr::from(([0, 0, 0, 0], PORT));
        let admin_addr = std::net::SocketAddr::from(([127, 0, 0, 1], admin_port));
        tracing::info!(%public_addr, %admin_addr, "admin-api public + loopback-only admin console listening");

        let public_listener = tokio::net::TcpListener::bind(public_addr)
            .await
            .expect("bind public");
        let admin_listener = tokio::net::TcpListener::bind(admin_addr)
            .await
            .expect("bind admin");

        let public_srv = axum::serve(
            public_listener,
            public_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        );
        let admin_srv = axum::serve(
            admin_listener,
            admin_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        );
        let (a, b) = tokio::join!(public_srv, admin_srv);
        a.expect("serve public");
        b.expect("serve admin");
    } else {
        let app = admin_api::router(state).layer(cors);
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], PORT));
        tracing::info!(%addr, "admin-api listening");
        let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve");
    }
}

/// Resolve the JWT signing key (audit L4). `SHARE_JWT_SIGNING_KEY` (32-byte hex) is the shared,
/// restart- and instance-stable key. Malformed -> fail closed. Missing in a persistent deployment
/// (`prod`, i.e. DEMO_MODE/VITE_DEMO_MODE unset) -> fail closed. Demo/local -> ephemeral key + warning.
fn load_jwt_keys(prod: bool) -> JwtKeys {
    match std::env::var("SHARE_JWT_SIGNING_KEY").ok().filter(|s| !s.trim().is_empty()) {
        Some(seed) => match JwtKeys::from_seed_hex(&seed) {
            Ok(k) => {
                tracing::info!("loaded shared JWT signing key from SHARE_JWT_SIGNING_KEY");
                k
            }
            Err(e) => {
                tracing::error!("SHARE_JWT_SIGNING_KEY is set but invalid ({e}); refusing to start");
                std::process::exit(1);
            }
        },
        None if prod => {
            tracing::error!(
                "SHARE_JWT_SIGNING_KEY is required in production (DEMO_MODE unset) so share tokens \
                 survive restart and work across horizontally-scaled instances; refusing to start"
            );
            std::process::exit(1);
        }
        None => {
            tracing::warn!(
                "SHARE_JWT_SIGNING_KEY unset; using an EPHEMERAL JWT key (demo/local only — tokens \
                 will NOT survive restart or work across horizontally-scaled instances)"
            );
            JwtKeys::generate()
        }
    }
}

/// Build the backing store. With `MONGO_URI` set & non-empty: persistent MongoStore (fail-closed on
/// connect error). Otherwise: ephemeral MemStore (demo/local — unchanged).
async fn build_store() -> Arc<dyn Store> {
    let uri = std::env::var("MONGO_URI").unwrap_or_default();
    if uri.trim().is_empty() {
        return Arc::new(MemStore::new());
    }

    #[cfg(feature = "mongo")]
    {
        let db = std::env::var("MONGO_DB").unwrap_or_else(|_| "dogtag".to_string());
        match admin_api::mongo::MongoStore::connect(&uri, &db).await {
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

/// CORS layer: explicit allowlist from `CORS_ALLOW_ORIGINS` (comma-separated) when set, else permissive.
fn build_cors() -> CorsLayer {
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
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any)
        }
        _ => CorsLayer::permissive(),
    }
}
