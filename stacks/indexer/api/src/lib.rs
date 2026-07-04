//! DogTag oversight indexer — library surface.
//!
//! Scans the ROAX (chainId 135) factory / registry / `DogTagIssuer`-clone / VerificationRegistry
//! event logs into a queryable, **non-PII** index and serves a scope-enforced query API: the UNSCOPED
//! government oversight feed + per-signer/clone SCOPED views for vet/groomer. See `AGENTS.md`
//! (§ oversight indexer) and `dogtag-govarch-r8` Part 4 for the architecture.

pub mod app;
pub mod chain;
pub mod directory;
pub mod events;
pub mod indexer;
pub mod routes;
pub mod scope;
pub mod store;

#[cfg(feature = "mongo")]
pub mod mongo;

pub use routes::router;
