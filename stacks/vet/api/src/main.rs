//! vet-api server entrypoint. Binds the Axum router on port 41874 (impl Phase 3).
//!
//! Production wiring: AlloyChain (ROAX RPC). Without `mongo`, the binary uses the in-memory MemStore.

use std::collections::HashMap;
use std::sync::Arc;

use vet_api::app::{AppState, Config};
use vet_api::auth::JwtKeys;
use vet_api::chain::AlloyChain;
use vet_api::custody::Custody;
use vet_api::prover::{ArkProver, ProverClient, StubProver};
use vet_api::store::MemStore;

const PORT: u16 = 41874;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());

    let rpc_url = env("ROAX_RPC", "https://devrpc.roax.net");
    let mut issuer_addrs = HashMap::new();
    if let Ok(a) = std::env::var("VACCINATION_ISSUER_ADDR") {
        issuer_addrs.insert("VACCINATION".to_string(), a);
    }

    let cfg = Config {
        deployment_url: env("DEPLOYMENT_URL", &format!("http://localhost:{PORT}")),
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
    };

    let chain = AlloyChain::new(rpc_url);

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
        custody: Custody::new(),
        jwt: JwtKeys::generate(),
        cfg: Arc::new(cfg),
    };

    let app = vet_api::router(state);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], PORT));
    tracing::info!("vet-api listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}
