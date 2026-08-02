//! Application state + configuration for the central/admin backend.

use std::sync::Arc;

use crate::auth::JwtKeys;
use crate::business::BusinessClient;
use crate::chain::ChainClient;
use crate::crypto::KeyVault;
use crate::dns::DnsChecker;
use crate::indexer::OversightFeed;
use crate::store::Store;

/// The DOG_PROFILE record type used by issuer onboarding and governance.
pub const DOG_PROFILE: &str = "DOG_PROFILE";

/// Resolved central config (contract addresses + the admin's signer roles).
#[derive(Clone)]
pub struct Config {
    pub deployment_url: String,
    pub rpc_url: String,
    pub issuer_registry_addr: String,
    /// EIP-155 chain id used for unified protocol metadata and chain transactions.
    pub chain_id: u64,
    /// Unified owner-hidden verification registry used when importing an unstamped document.
    pub verification_registry_addr: String,
    /// `DogTagSBTConsent` governance target for ISSUER_ROLE administration; admin never issues tags.
    pub sbt_addr: String,
    /// DogTagIssuerFactory address — the `createIssuer`/`predictIssuer` target + the Ownable owner whose
    /// key gates deploys (plan PR-A). Empty (zero) until `FACTORY_ADDR` is configured.
    pub factory_addr: String,
    /// Generation-2 `ProviderRegistry` — the registrar surface's target and the `Ownable2Step` owner
    /// whose key gates every write on it (registry plan C-2). Empty (zero) until
    /// `PROVIDER_REGISTRY_ADDR` is configured, which every provider route reports LOUDLY rather than
    /// degrading: a registrar screen that silently read nothing would say "no providers exist" about a
    /// registry it never asked.
    pub provider_registry_addr: String,
    /// admin-session password HASH ("<salt_hex>$<hash_hex>", audit L4) — never the plaintext. Set from
    /// `ADMIN_PASSWORD_HASH` (prod) or computed once at startup from `ADMIN_PASSWORD` (demo). `admin_login`
    /// verifies the submitted password against this with `auth::verify_password`.
    pub admin_password_hash: String,
    /// account index of the admin signer (WHITELIST_ADMIN + ISSUER roles).
    pub admin_signer_index: u32,
    /// The operator's DECLARATION that this deployment signs privileged writes out-of-band, so a
    /// `disposition:"proposed"` grant/revoke is the intended outcome rather than a wrong-key failure.
    /// Set via `ADMIN_PROPOSE_ONLY` (or the equivalent `ALLOW_UNAUTHORIZED_ADMIN_SIGNER`). It only
    /// changes how an outcome is REPORTED - never whether an action is dispatched, and never who holds
    /// an authority, which is always read live from the chain.
    pub propose_only: bool,
}

/// The shared application state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub chain: Arc<dyn ChainClient>,
    pub dns: Arc<dyn DnsChecker>,
    pub business: Arc<dyn BusinessClient>,
    pub vault: Arc<dyn KeyVault>,
    /// UNSCOPED consumer of the PR-4 oversight indexer (the "see on-chain activity" data layer, PR-B).
    /// `DisabledFeed` when `INDEXER_API_BASE` is unset; the real `HttpOversightFeed` otherwise.
    pub feed: Arc<dyn OversightFeed>,
    pub jwt: JwtKeys,
    pub cfg: Arc<Config>,
    /// in-memory login rate limiter (lenient; demo-safe).
    pub ratelimit: Arc<crate::auth::RateLimiter>,
}
