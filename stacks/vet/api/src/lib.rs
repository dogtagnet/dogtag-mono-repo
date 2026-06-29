//! DogTag vet business backend (Axum + Tokio + Alloy + dogtag-standard-rs) — a self-hosted issuer.
//!
//! Module map:
//!   custody.rs     — HD genesis / age-encrypt / unlock / derive (impl §3.1, §11.4)
//!   chain.rs       — `ChainClient` trait + Alloy impl + `MemChain` stub (issue/isValid/RootIssued)
//!   signing        — EIP-1559 sign+send helper (re-exported from chain)
//!   store.rs       — `Store` trait + `MemStore` (+ optional `MongoStore`)
//!   auth.rs        — operator/admin sessions + EdDSA share/verify JWTs
//!   prover.rs      — `ProverClient` trait + `StubProver` (ZK, wired later)
//!   app.rs         — AppState/config + server-side VC build/wrap
//!   verify.rs      — third-party verify + /verify/consent/submit
//!   routes.rs      — Axum router + all handlers

pub mod app;
pub mod auth;
pub mod calendar;
pub mod chain;
pub mod custody;
pub mod prover;
pub mod routes;
pub mod startup;
pub mod store;
pub mod sync;
pub mod verify;

#[cfg(feature = "mongo")]
pub mod mongo;

pub use app::{AppState, Config};
pub use routes::{admin_router, public_router, router};

/// `signing` is the EIP-1559/legacy sign-and-send surface; it lives in `chain` (AlloyChain).
pub mod signing {
    pub use crate::chain::{
        issue_calldata, revoke_calldata, AlloyChain, ChainClient, SentTx, TxView,
    };
}
