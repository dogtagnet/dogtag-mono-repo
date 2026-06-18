//! DogTag central/admin backend entrypoint. Binds Axum on port 39742 (impl §4).

use std::sync::Arc;

use admin_api::app::{AppState, Config};
use admin_api::auth::JwtKeys;
use admin_api::business::ReqwestBusinessClient;
use admin_api::chain::{AlloyChain, ChainClient};
use admin_api::crypto::MemVault;
use admin_api::dns::{DnsChecker, DohDnsChecker, MockDnsChecker};
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

    // Wire the admin/WHITELIST_ADMIN+ISSUER signer at the configured index from ADMIN_PRIVATE_KEY so
    // whitelistFor/delistFor/mint can broadcast on-chain. Without this the chain client has no signer
    // and every admin write fails with "no signer for index". (The custody stacks unlock their own
    // signers; the central stack's signer is a static deployer key supplied at boot.)
    let chain = AlloyChain::new(rpc_url);
    if let Ok(pk_hex) = std::env::var("ADMIN_PRIVATE_KEY") {
        let pk_hex = pk_hex.trim().strip_prefix("0x").unwrap_or(pk_hex.trim());
        match hex::decode(pk_hex) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut pk = [0u8; 32];
                pk.copy_from_slice(&bytes);
                let addr = std::env::var("ADMIN_ADDRESS").unwrap_or_default();
                chain.register_signer(cfg.admin_signer_index, pk, addr).await;
                tracing::info!("admin signer registered at index {}", cfg.admin_signer_index);
            }
            _ => tracing::warn!("ADMIN_PRIVATE_KEY set but not a 32-byte hex key; admin writes will fail"),
        }
    } else {
        tracing::warn!("ADMIN_PRIVATE_KEY unset; on-chain admin writes (whitelistFor/mint) will fail");
    }

    // DNS legitimacy check: real DoH in prod; set DNS_CHECK=skip for the local demo where the
    // business domain (e.g. vet.local) has no published TXT record.
    let dns: Arc<dyn DnsChecker> = if env("DNS_CHECK", "doh") == "skip" {
        tracing::warn!("DNS_CHECK=skip: DNS TXT legitimacy verification is BYPASSED (demo only)");
        Arc::new(MockDnsChecker::ok())
    } else {
        Arc::new(DohDnsChecker::default())
    };

    let state = AppState {
        store: Arc::new(MemStore::new()),
        chain: Arc::new(chain),
        dns,
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
