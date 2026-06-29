//! DogTag central/admin backend (Axum + Tokio + Alloy + dogtag-standard-rs) — the ONE central stack:
//! it powers the mobile apps + admin functions (impl §4).
//!
//! Module map:
//!   store.rs        — `Store` trait + `MemStore` (+ optional `MongoStore`); central DB collections
//!   chain.rs        — `ChainClient` trait + `AlloyChain` + `MemChain` (whitelistFor/delistFor/mint)
//!   dns.rs          — `DnsChecker` trait + DoH impl + mock (pre-whitelist DNS TXT verification)
//!   auth.rs         — owner/admin sessions + EdDSA share JWT + HMAC + password hashing
//!   crypto.rs       — `KeyVault` (per-record DEK) + crypto-shredding
//!   business.rs     — `BusinessClient` (PUT-to-business + relay-to-verifier) + mock
//!   app.rs          — AppState/Config + DOG_PROFILE VC build/wrap
//!   verify.rs       — credential-import structural verification (SDK canonicalization)
//!   verify_relay.rs — proof-of-verification consent relay
//!   erasure.rs      — `erase` + `fulfill_due_deletions` (crypto-shred)
//!   routes.rs       — Axum router + all handlers (§4.1–§4.5)

pub mod app;
pub mod auth;
pub mod business;
pub mod chain;
pub mod crypto;
pub mod dns;
pub mod erasure;
pub mod routes;
pub mod startup;
pub mod store;
pub mod verify;
pub mod verify_relay;

#[cfg(feature = "mongo")]
pub mod mongo;

pub use app::{AppState, Config};
pub use routes::{admin_router, public_router, router};
