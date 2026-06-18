//! DogTag central/admin backend entrypoint. Binds Axum on port 39742 (impl §4).

use std::sync::Arc;

use admin_api::app::{AppState, Config};
use admin_api::auth::JwtKeys;
use admin_api::business::ReqwestBusinessClient;
use admin_api::chain::AlloyChain;
use admin_api::crypto::MemVault;
use admin_api::dns::DohDnsChecker;
use admin_api::store::MemStore;
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

    let cfg = Config {
        deployment_url: env("DEPLOYMENT_URL", &format!("http://localhost:{PORT}")),
        rpc_url: rpc_url.clone(),
        issuer_registry_addr: env("ISSUER_REGISTRY_ADDR", "0x0000000000000000000000000000000000000000"),
        sbt_addr: env("SBT_ADDR", "0x0000000000000000000000000000000000000000"),
        issuer_name: env("ISSUER_NAME", "DogTag Central"),
        issuer_domain: env("ISSUER_DOMAIN", "dogtag.example"),
        profile_document_store: env("PROFILE_DOCUMENT_STORE", "0x0000000000000000000000000000000000000000"),
        admin_password: env("ADMIN_PASSWORD", "admin-pw"),
        admin_signer_index: 0,
    };

    let state = AppState {
        store: Arc::new(MemStore::new()),
        chain: Arc::new(AlloyChain::new(rpc_url)),
        dns: Arc::new(DohDnsChecker::default()),
        business: Arc::new(ReqwestBusinessClient::new()),
        vault: Arc::new(MemVault::new()),
        jwt: JwtKeys::generate(),
        cfg: Arc::new(cfg),
    };

    let app = admin_api::router(state).layer(CorsLayer::permissive());
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], PORT));
    tracing::info!(%addr, "admin-api listening");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}
