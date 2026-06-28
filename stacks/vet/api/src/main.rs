//! vet-api server entrypoint. Binds the Axum router on port 41874 (impl Phase 3).
//!
//! Production wiring: AlloyChain (ROAX RPC). Without `mongo`, the binary uses the in-memory MemStore.

use std::collections::HashMap;
use std::sync::Arc;

use vet_api::app::{AppState, Config};
use vet_api::auth::JwtKeys;
use vet_api::calendar::{CalendarProvider, CentralClient, GoogleCalendar, ReqwestCentralClient};
use vet_api::chain::AlloyChain;
use vet_api::custody::Custody;
use vet_api::prover::{ArkProver, ProverClient, StubProver};
use vet_api::store::{MemStore, Store};
use tower_http::cors::CorsLayer;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    // PORT from env so the same binary serves vet (41874) and groomer (43618).
    let port: u16 = env("PORT", "41874").parse().unwrap_or(41874);

    let rpc_url = env("ROAX_RPC", "https://devrpc.roax.net");
    // CHAIN_ID is env-driven so a different/production chain is a pure config swap (default 135 = ROAX).
    let chain_id: u64 = std::env::var("CHAIN_ID")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(135);
    let mut issuer_addrs = HashMap::new();
    if let Ok(a) = std::env::var("VACCINATION_ISSUER_ADDR") {
        issuer_addrs.insert("VACCINATION".to_string(), a);
    }

    let cfg = Config {
        deployment_url: env("DEPLOYMENT_URL", &format!("http://localhost:{port}")),
        rpc_url: rpc_url.clone(),
        issuer_registry_addr: env(
            "ISSUER_REGISTRY_ADDR",
            "0x0000000000000000000000000000000000000000",
        ),
        verification_registry_addr: env(
            "VERIFICATION_REGISTRY_ADDR",
            "0x0000000000000000000000000000000000000000",
        ),
        consent_key_registry_addr: env(
            "CONSENT_KEY_REGISTRY_ADDR",
            "0x0000000000000000000000000000000000000000",
        ),
        issuer_addrs,
        issuer_name: env("ISSUER_NAME", "DogTag Vet"),
        issuer_domain: env("ISSUER_DOMAIN", "vet.example"),
        // DogTagSBT mint target (the vet signer must hold ISSUER_ROLE). PROFILE_DOCUMENT_STORE
        // conventionally == SBT_ADDR (the SBT contract acts as the DOG_PROFILE document store).
        sbt_addr: env("SBT_ADDR", "0x0000000000000000000000000000000000000000"),
        profile_document_store: {
            let sbt = env("SBT_ADDR", "0x0000000000000000000000000000000000000000");
            env("PROFILE_DOCUMENT_STORE", &sbt)
        },
        vet_signer_index: 0,
        operator_password: env("OPERATOR_PASSWORD", "operator-dev-password"),
        admin_password: env("ADMIN_PASSWORD", "admin-dev-password"),
        confirmations: env("CONFIRMATIONS", "1").parse().unwrap_or(1),
        business_id: env("BUSINESS_ID", "biz-local"),
        central_hmac_secret: env("CENTRAL_HMAC_SECRET", "dev-central-hmac-secret"),
        // OPTIONAL disk seal: when set, the sealed custody (ciphertext + meta) is persisted on
        // genesis and re-loaded on startup so the signer survives a restart. Unset -> in-memory only.
        custody_seal_path: std::env::var("CUSTODY_SEAL_PATH").ok().filter(|s| !s.trim().is_empty()),
    };

    // Fail-closed (audit H2): refuse to boot in production with unset/dev-default secrets. The
    // local/demo path (DEMO_MODE / VITE_DEMO_MODE set) keeps the convenient defaults.
    let demo = vet_api::startup::is_demo_mode();
    if let Err(e) = vet_api::startup::validate_production_secrets(
        demo,
        &[
            vet_api::startup::SecretSpec {
                name: "OPERATOR_PASSWORD",
                value: cfg.operator_password.as_str(),
                dev_default: "operator-dev-password",
            },
            vet_api::startup::SecretSpec {
                name: "ADMIN_PASSWORD",
                value: cfg.admin_password.as_str(),
                dev_default: "admin-dev-password",
            },
            vet_api::startup::SecretSpec {
                name: "CENTRAL_HMAC_SECRET",
                value: cfg.central_hmac_secret.as_str(),
                dev_default: "dev-central-hmac-secret",
            },
        ],
    ) {
        eprintln!("FATAL: {e}");
        std::process::exit(1);
    }

    let chain = AlloyChain::new(rpc_url).with_chain_id(chain_id);

    // Google Calendar provider (real reqwest impl; UNtested against live Google without OAuth creds).
    let calendar: Arc<dyn CalendarProvider> = Arc::new(GoogleCalendar::new(
        env("GOOGLE_CLIENT_ID", ""),
        env("GOOGLE_CLIENT_SECRET", ""),
        env("GOOGLE_REDIRECT_URI", &format!("http://localhost:{port}/calendar/google/callback")),
        env("GOOGLE_CALENDAR_ID", "primary"),
    ));
    // central appointment-events callback (HMAC-signed).
    let central: Arc<dyn CentralClient> = Arc::new(ReqwestCentralClient::new(
        env("CENTRAL_BASE_URL", "http://localhost:39742"),
        cfg.central_hmac_secret.clone(),
    ));

    // Choose the prover: if CIRCUITS_BUILD_DIR points at a circuits `build` dir, load the REAL
    // ark-circom Groth16 prover; otherwise fall back to the StubProver (ZK control-flow only).
    let prover: Arc<dyn ProverClient> = match std::env::var("CIRCUITS_BUILD_DIR") {
        Ok(dir) if !dir.is_empty() => match ArkProver::load(&dir) {
            Ok(p) => {
                tracing::info!("loaded real Groth16 prover from {dir} (zkey {})", p.zkey_hash_hex());
                Arc::new(p)
            }
            Err(e) => {
                // Fail-closed (audit M1): a configured real prover that fails to load must NOT silently
                // degrade to the StubProver (which emits zeroed proofs the chain would reject). A
                // prover-service whose whole job is real proofs has no business booting without them.
                eprintln!(
                    "FATAL: CIRCUITS_BUILD_DIR is set but the real Groth16 prover failed to load: {e}"
                );
                std::process::exit(1);
            }
        },
        _ => Arc::new(StubProver),
    };

    // Store selection: persistent MongoStore when MONGO_URI is set (fail-closed), else ephemeral MemStore
    // (demo/local — unchanged). Demo behavior is byte-for-byte preserved when MONGO_URI is unset/empty.
    let store: Arc<dyn Store> = build_store().await;

    // Hydrate the sealed custody from disk (if CUSTODY_SEAL_PATH is set and the file exists) so the
    // store starts "initialized but locked" after a restart. We do NOT auto-unlock — there is no
    // passphrase on disk; the operator still unlocks (the passphrase decrypts the disk-loaded blob ->
    // the SAME signer). Skip if the store already has a custody blob (e.g. Mongo already persisted it).
    if let Some(path) = cfg.custody_seal_path.as_deref() {
        if store.get_custody().await.is_none() {
            match vet_api::custody::read_seal_file(path) {
                Ok(Some((encrypted_seed, meta))) => {
                    store
                        .put_custody(vet_api::store::CustodyBlob { encrypted_seed, meta })
                        .await;
                    tracing::info!("hydrated sealed custody from {path} (initialized but locked)");
                }
                Ok(None) => tracing::info!("no custody seal at {path}; starting uninitialized"),
                Err(e) => {
                    tracing::error!("CUSTODY_SEAL_PATH set but seal load failed: {e}; refusing to start");
                    std::process::exit(1);
                }
            }
        }
    }

    let state = AppState {
        store,
        chain: Arc::new(chain),
        prover,
        calendar,
        central,
        custody: Custody::new(),
        jwt: JwtKeys::generate(),
        cfg: Arc::new(cfg),
        ratelimit: Arc::new(vet_api::auth::RateLimiter::new()),
    };

    // CORS: explicit allowlist when CORS_ALLOW_ORIGINS is set (prod), else permissive (demo).
    let cors = build_cors();

    // Admin-router loopback isolation (ADMIN_LOOPBACK_ONLY): when truthy, the public 0.0.0.0:PORT
    // listener omits /admin/*, and the /admin/* custody routes are served on a separate
    // 127.0.0.1:ADMIN_PORT listener (default PORT+1). Default (unset): everything on one listener.
    let admin_loopback = matches!(env("ADMIN_LOOPBACK_ONLY", "").as_str(), "1" | "true");

    if admin_loopback {
        let admin_port: u16 = std::env::var("ADMIN_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(port + 1);

        let public_app = vet_api::public_router(state.clone()).layer(cors.clone());
        let admin_app = vet_api::admin_router(state).layer(cors);

        let public_addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        let admin_addr = std::net::SocketAddr::from(([127, 0, 0, 1], admin_port));
        tracing::info!("vet-api public listening on {public_addr}; /admin/* loopback-only on {admin_addr}");

        let public_listener = tokio::net::TcpListener::bind(public_addr).await.expect("bind public");
        let admin_listener = tokio::net::TcpListener::bind(admin_addr).await.expect("bind admin");

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
        let app = vet_api::router(state).layer(cors);
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        tracing::info!("vet-api listening on {addr}");
        let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve");
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
        match vet_api::mongo::MongoStore::connect(&uri, &db).await {
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
