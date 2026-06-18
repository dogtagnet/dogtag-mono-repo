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
use vet_api::store::MemStore;
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
        issuer_addrs,
        issuer_name: env("ISSUER_NAME", "DogTag Vet"),
        issuer_domain: env("ISSUER_DOMAIN", "vet.example"),
        operator_password: env("OPERATOR_PASSWORD", "operator-dev-password"),
        admin_password: env("ADMIN_PASSWORD", "admin-dev-password"),
        confirmations: env("CONFIRMATIONS", "1").parse().unwrap_or(1),
        business_id: env("BUSINESS_ID", "biz-local"),
        central_hmac_secret: env("CENTRAL_HMAC_SECRET", "dev-central-hmac-secret"),
    };

    let chain = AlloyChain::new(rpc_url);

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
                tracing::warn!("CIRCUITS_BUILD_DIR set but prover load failed ({e}); using StubProver");
                Arc::new(StubProver)
            }
        },
        _ => Arc::new(StubProver),
    };

    let state = AppState {
        store: Arc::new(MemStore::new()),
        chain: Arc::new(chain),
        prover,
        calendar,
        central,
        custody: Custody::new(),
        jwt: JwtKeys::generate(),
        cfg: Arc::new(cfg),
    };

    let app = vet_api::router(state).layer(CorsLayer::permissive());
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("vet-api listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}
