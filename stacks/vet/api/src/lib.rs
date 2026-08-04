//! DogTag vet business backend (Axum + Tokio + Alloy + dogtag-standard-rs) — a self-hosted issuer.
//!
//! Module map:
//!   custody.rs     — HD genesis / age-encrypt / unlock / derive (impl §3.1, §11.4)
//!   chain.rs       — `ChainClient` trait + Alloy impl + `MemChain` stub (issue/isValid/RootIssued)
//!   signing        — EIP-1559 sign+send helper (re-exported from chain)
//!   store.rs       — `Store` trait + `MemStore` (+ optional `MongoStore`)
//!   auth.rs        — operator/admin sessions + EdDSA share/verify JWTs
//!   prover.rs      — owner-hidden consent prover service
//!   app.rs         — AppState/config + server-side VC build/wrap
//!   verify.rs      — third-party verify + canonical owner-hidden consent submission
//!   crm.rs         — the shop's clients/appointments + the verification history they link to
//!   ics.rs         — pure RFC 5545 (iCalendar) serialization
//!   calendar_ics.rs— the published `.ics` subscription feed + `.ics` import routes (the SHOP's
//!                    whole schedule)
//!   appointment_share.rs — the CLIENT half: one booking handed to the person it belongs to, as a
//!                    scannable page + `.ics` + add-to-Google link
//!   routes.rs      — Axum router + all handlers

pub mod app;
pub mod appointment_share;
pub mod auth;
pub mod calendar;
pub mod calendar_ics;
pub mod chain;
pub mod crm;
pub mod ics;
pub mod custody;
pub mod discovery;
pub mod issuance_allowed;
pub mod microchip;
pub mod oversight;
pub mod protocol;
pub mod prover;
pub mod routes;
pub mod startup;
pub mod store;
pub mod sync;
pub mod trace;
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
