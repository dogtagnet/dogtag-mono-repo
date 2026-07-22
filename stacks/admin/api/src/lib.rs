//! DogTag central/admin backend (Axum + Tokio + Alloy + dogtag-standard-rs) — the ONE central stack:
//! it powers the mobile apps + admin functions (impl §4).
//!
//! Module map:
//!   store.rs        — `Store` trait + `MemStore` (+ optional `MongoStore`); central DB collections
//!   chain.rs        — `ChainClient` trait + `AlloyChain` + `MemChain` (governance/whitelisting)
//!   dns.rs          — `DnsChecker` trait + DoH impl + mock (pre-whitelist DNS TXT verification)
//!   auth.rs         — owner/admin sessions + EdDSA share JWT + HMAC + password hashing
//!   crypto.rs       — `KeyVault` (per-record DEK) + crypto-shredding
//!   business.rs     — `BusinessClient` (PUT-to-business) + mock
//!   app.rs          — AppState/Config
//!   verify.rs       — credential-import structural verification (SDK canonicalization)
//!   erasure.rs      — `erase` + `fulfill_due_deletions` (crypto-shred)
//!   governance.rs   — `GovernanceAction` (sign-if-held / propose-if-not) authority abstraction (PR-A)
//!   indexer.rs      — `OversightFeed` client: the UNSCOPED consumer of the PR-4 oversight indexer (PR-B)
//!   directory.rs    — signer→business directory: names on-chain signers from the business registry (PR-B)
//!   routes.rs       — Axum router + all handlers (§4.1–§4.5)

pub mod app;
pub mod auth;
pub mod business;
pub mod chain;
pub mod crypto;
pub mod directory;
pub mod dns;
pub mod erasure;
pub mod governance;
pub mod indexer;
pub mod routes;
pub mod startup;
pub mod store;
pub mod verify;

#[cfg(feature = "mongo")]
pub mod mongo;

pub use app::{AppState, Config};
pub use routes::{admin_router, public_router, router};
