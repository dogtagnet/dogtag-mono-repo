//! DogTag central/admin backend entrypoint. Binds Axum on port 39742 (impl §4).

use std::sync::Arc;

use admin_api::app::{AppState, Config};
use admin_api::auth::JwtKeys;
use admin_api::business::ReqwestBusinessClient;
use admin_api::chain::{AlloyChain, ChainClient};
use admin_api::crypto::MemVault;
use admin_api::dns::{DnsChecker, DohDnsChecker, MockDnsChecker};
use admin_api::indexer::{DisabledFeed, HttpOversightFeed, OversightFeed};
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
        // Unified owner-hidden registry used for unstamped imported-document metadata.
        chain_id,
        verification_registry_addr: env(
            "VERIFICATION_REGISTRY_ADDR",
            "0xaBFd6f6E31780EBcB7ABd28A2a9bCfc9C8e6A77B",
        ),
        sbt_addr: env(
            "SBT_ADDR",
            "0xBEbc45A838643D27004827b797b30A464b2b02c0",
        ),
        factory_addr: env("FACTORY_ADDR", "0x0000000000000000000000000000000000000000"),
        // Store a real password HASH, never the plaintext (audit L4) — admin_login verifies against
        // this with auth::verify_password. Optional `ADMIN_PASSWORD_HASH` ("<salt_hex>$<hash_hex>")
        // overrides; otherwise the ADMIN_PASSWORD plaintext (still required non-default in prod by the
        // H2 boot-guard below) is hashed once here at startup.
        admin_password_hash: std::env::var("ADMIN_PASSWORD_HASH")
            .ok()
            .filter(|h| !h.trim().is_empty())
            .map(|h| h.trim().to_string())
            .unwrap_or_else(|| admin_api::auth::hash_password(&admin_password)),
        // Honor ADMIN_SIGNER_INDEX (was hardcoded 0). The signer at this index is unlocked below from
        // ADMIN_PRIVATE_KEY and is the key the GovernanceAction dispatcher checks role-holdership for.
        admin_signer_index: std::env::var("ADMIN_SIGNER_INDEX")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0),
        // Declared propose-for-external-signing. `ALLOW_UNAUTHORIZED_ADMIN_SIGNER` is the same
        // declaration scripts/demo-up.sh already documents as the escape hatch for this deployment
        // shape, so it is accepted as an alias rather than becoming a second, drifting concept.
        propose_only: ["ADMIN_PROPOSE_ONLY", "ALLOW_UNAUTHORIZED_ADMIN_SIGNER"]
            .iter()
            .any(|k| {
                matches!(
                    std::env::var(k).unwrap_or_default().trim().to_ascii_lowercase().as_str(),
                    "1" | "true"
                )
            }),
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
    // whitelist, issuer-role, and governance actions can broadcast. Without this the chain client has no signer
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
            "ADMIN_PRIVATE_KEY unset; on-chain admin/governance writes will fail"
        );
    }

    // Control-plane authority preflight: resolve, ONCE at boot, whether the hosted signer actually
    // holds the authorities every privileged write needs. Without this a stack booted with a retired
    // key starts clean and returns disposition:"proposed" for every grant — indistinguishable from the
    // legitimate propose-for-external-signing flow, with nothing reaching the chain.
    //
    // It is a DIAGNOSTIC and must never gate liveness: it runs before `axum::serve` binds, and the
    // alloy provider has no timeout of its own, so an endpoint that accepts TCP but never answers
    // would otherwise stall the boot and /health would never come up. On elapse the authorities are
    // simply UNRESOLVED, which is exactly the existing `AuthorityVerdict::Unknown` state - a warning,
    // never fatal, so ADMIN_REQUIRE_AUTHORITY does not fire on an unreadable chain either.
    const AUTHORITY_PREFLIGHT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    if tokio::time::timeout(AUTHORITY_PREFLIGHT_TIMEOUT, authority_preflight(&chain, &cfg))
        .await
        .is_err()
    {
        tracing::warn!(
            "control-plane authority preflight did not complete within {:?} (RPC accepted the \
             connection but did not answer): authorities UNRESOLVED, continuing boot. Privileged \
             writes may silently degrade to unsigned proposals - check GET /v1/admin/governance/authority.",
            AUTHORITY_PREFLIGHT_TIMEOUT
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

    // Oversight-indexer consumer (PR-B): the UNSCOPED cross-issuer feed. Wired when INDEXER_API_BASE is
    // set (with INDEXER_OVERSIGHT_TOKEN — the indexer's `unscoped:true` bearer); otherwise a
    // DisabledFeed makes the activity/directory-count surfaces fail closed with 503 while the rest of
    // the admin backend runs unchanged. The signer→business directory is store-derived and always live.
    let feed: Arc<dyn OversightFeed> = build_feed();

    let state = AppState {
        store,
        chain: Arc::new(chain),
        dns,
        business: Arc::new(ReqwestBusinessClient::new()),
        vault: Arc::new(MemVault::new()),
        feed,
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

/// Read the three control-plane authorities for the hosted signer and report what it does NOT hold.
///
/// A missing authority is not automatically an error — after the governance Phase-2 handover the
/// registry `DEFAULT_ADMIN_ROLE` legitimately lives on the governance signer, and those actions are
/// MEANT to come back as proposals for out-of-band signing. Holding *none* of them is different: it
/// means every privileged write degrades to unsigned calldata while the stack looks healthy. That case
/// logs at ERROR, and `ADMIN_REQUIRE_AUTHORITY=1` turns it into a refusal to boot.
async fn authority_preflight(chain: &AlloyChain, cfg: &Config) {
    use admin_api::chain::{default_admin_role, whitelist_admin_role};
    use admin_api::startup::{authority_preflight_message, authority_verdict, AuthorityCheck};

    const ZERO: &str = "0x0000000000000000000000000000000000000000";
    let hosted = chain.signer_address(cfg.admin_signer_index).await;
    let Some(hosted_addr) = hosted.clone() else {
        tracing::warn!(
            "control-plane authority preflight skipped: no hosted signer registered (ADMIN_PRIVATE_KEY \
             unset) - every privileged write will fail or degrade to an unsigned proposal"
        );
        return;
    };

    // An unconfigured target is reported as unknown rather than as a missing authority: the zero
    // address is a configuration gap (see FACTORY_ADDR), not evidence about the signer.
    let is_zero = |a: &str| a.eq_ignore_ascii_case(ZERO);
    let factory_held = if is_zero(&cfg.factory_addr) {
        None
    } else {
        chain
            .ownable_owner(&cfg.factory_addr)
            .await
            .ok()
            .map(|owner| owner.eq_ignore_ascii_case(&hosted_addr))
    };
    let registry = &cfg.issuer_registry_addr;
    let (wl_held, da_held) = if is_zero(registry) {
        (None, None)
    } else {
        (
            chain
                .has_role(registry, &whitelist_admin_role(), &hosted_addr)
                .await
                .ok(),
            chain
                .has_role(registry, &default_admin_role(), &hosted_addr)
                .await
                .ok(),
        )
    };

    let checks = [
        AuthorityCheck {
            name: "WHITELIST_ADMIN",
            target: registry,
            capability: "whitelistFor / delistFor - issuer + verifier grants",
            held: wl_held,
        },
        AuthorityCheck {
            name: "FACTORY_OWNER",
            target: &cfg.factory_addr,
            capability: "createIssuer - deploy an issuer clone",
            held: factory_held,
        },
        AuthorityCheck {
            name: "DEFAULT_ADMIN",
            target: registry,
            capability: "adminRevoke / role-admin / verifier swaps",
            held: da_held,
        },
    ];

    let verdict = authority_verdict(&checks);
    let msg = authority_preflight_message(hosted.as_deref(), &verdict);
    if verdict.is_unauthorized() {
        tracing::error!("{msg}");
        if matches!(
            std::env::var("ADMIN_REQUIRE_AUTHORITY").unwrap_or_default().as_str(),
            "1" | "true"
        ) {
            eprintln!("FATAL: ADMIN_REQUIRE_AUTHORITY=1 and the hosted signer holds no control-plane authority.");
            std::process::exit(1);
        }
    } else if matches!(verdict, admin_api::startup::AuthorityVerdict::AllHeld) {
        tracing::info!("{msg}");
    } else {
        tracing::warn!("{msg}");
    }
}

/// Resolve the JWT signing key (audit L4). `SHARE_JWT_SIGNING_KEY` (32-byte hex) is the shared,
/// restart- and instance-stable key. Malformed -> fail closed. Missing in a persistent deployment
/// (`prod`, i.e. DEMO_MODE/VITE_DEMO_MODE unset) -> fail closed. Demo/local -> ephemeral key + warning.
fn load_jwt_keys(prod: bool) -> JwtKeys {
    match std::env::var("SHARE_JWT_SIGNING_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Some(seed) => match JwtKeys::from_seed_hex(&seed) {
            Ok(k) => {
                tracing::info!("loaded shared JWT signing key from SHARE_JWT_SIGNING_KEY");
                k
            }
            Err(e) => {
                tracing::error!(
                    "SHARE_JWT_SIGNING_KEY is set but invalid ({e}); refusing to start"
                );
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

/// Build the oversight-indexer consumer (PR-B). With `INDEXER_API_BASE` set: a real
/// `HttpOversightFeed` presenting the UNSCOPED bearer token from `INDEXER_OVERSIGHT_TOKEN` (an
/// `INDEXER_SCOPES` entry with `unscoped:true`). Otherwise a `DisabledFeed` — the activity/count
/// surfaces fail closed with 503 and the rest of the admin backend runs unchanged. `ADMIN_INDEXER_BASE`
/// / `ADMIN_INDEXER_TOKEN` are accepted as aliases so the var names read clearly from the admin side.
fn build_feed() -> Arc<dyn OversightFeed> {
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty());
    let base = env("INDEXER_API_BASE").or_else(|| env("ADMIN_INDEXER_BASE"));
    match base {
        Some(base) => {
            let token = env("INDEXER_OVERSIGHT_TOKEN")
                .or_else(|| env("ADMIN_INDEXER_TOKEN"))
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
                "INDEXER_API_BASE unset; the on-chain activity + cross-issuer count surfaces \
                 (/v1/admin/activity*) return 503 until an indexer is configured"
            );
            Arc::new(DisabledFeed)
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
