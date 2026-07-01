//! DogTag **government** credential authority backend (Axum + Alloy + dogtag-standard-rs).
//!
//! A net-new, separately-deployable role stack (its own docker-compose, own MongoDB, own on-chain
//! wiring) that mirrors the vet/groomer business-backend model for a **government-grade credential
//! issuer/verifier**: it issues authority-endorsed `TRAVEL_CLEARANCE` / `EU_HEALTH_CERT` credentials
//! (anchoring the salted Poseidon root on a ROAX `DogTagIssuer` clone) and records government-grade
//! verifications (integrity + on-chain status + issuer identity). See `docs/ROLE_APPS.md`.
//!
//! Module map:
//!   chain.rs  — `ChainClient` trait + `AlloyChain` (real ROAX) + `MemChain` (demo/tests)
//!   store.rs  — `Store` trait + `MemStore` (+ optional `MongoStore` behind `mongo`)
//!   app.rs    — AppState/config + government VC build/wrap (shared open standard)
//!   routes.rs — Axum router + handlers (issue / verify / records)

pub mod app;
pub mod chain;
pub mod routes;
pub mod store;

#[cfg(feature = "mongo")]
pub mod mongo;

pub use app::{AppState, Config};
pub use routes::router;
