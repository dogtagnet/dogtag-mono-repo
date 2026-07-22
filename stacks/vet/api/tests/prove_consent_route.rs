//! `POST /prove-consent` route test (M7 P0) — runs ONLY with `--features prover`.
//!
//! Covers the route glue: version routing, fail-closed-per-request when this instance has no consent
//! artifacts, and (env-gated) a full HTTP prove round-trip. The heavy prove→verify itself is the
//! ground truth in `consent_prove.rs`; here we assert the ROUTE selects the version-keyed consent
//! artifact and fails closed per request, without re-running the multi-minute Groth16 prove by
//! default.
#![cfg(feature = "prover")]

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use ark_bn254::Fr;
use serde_json::json;

use dogtag_standard::consent_assemble::{
    assemble_consent, consent_circuit_input_value, ConsentWitness,
};
use dogtag_standard::profile_tree::{AttributeLeaf, SALT_LEN};
use dogtag_standard::types::TypedScalar;
// LEVEL-B indices: these exercise the consent circuit.
use dogtag_standard::public_signals::level_b as P;

use vet_api::chain::MemChain;
use vet_api::prover::ConsentProver;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// A base AppState; the caller sets `consent_prover` as needed.
fn base_state() -> vet_api::app::AppState {
    common::state_with(
        Arc::new(MemChain::new()),
        "memchain".to_string(),
        "0x00000000000000000000000000000000000000aa".to_string(),
        "0x00000000000000000000000000000000000000cc".to_string(),
        "vet.example".to_string(),
        1,
    )
}

fn sample_circuit_input() -> serde_json::Value {
    let attrs = vec![AttributeLeaf {
        key_path: "credentialSubject.name".to_string(),
        salt: [7u8; SALT_LEN],
        value: TypedScalar::Str("Rex".to_string()),
    }];
    let mut owner = [0u8; 20];
    owner[16..].copy_from_slice(&0xdead_beefu32.to_be_bytes());
    let inp = assemble_consent(&ConsentWitness {
        seed: b"prove-consent route test seed - TEST MATERIAL ONLY",
        dog_tag_id_handle: "424242",
        owner_address: owner,
        attributes: &attrs,
        purpose: Fr::from(7u64),
        relayer: [0x11u8; 20],
        record_type: Fr::from(19u64),
        deadline: Fr::from(1_893_456_000u64),
        consent_nonce: Fr::from(99u64),
    })
    .expect("assemble consent");
    consent_circuit_input_value(&inp)
}

/// Fail-closed PER REQUEST: an instance with no consent artifacts (the default, `disabled()`) returns
/// 503 for `/prove-consent` — it does NOT crash the rest of the vet API.
#[tokio::test]
async fn prove_consent_unavailable_without_artifacts_is_503() {
    let state = base_state(); // consent_prover defaults to disabled()
    let router = vet_api::public_router(state);
    let body = json!({ "circuitInput": sample_circuit_input() });

    let (status, resp) = common::call(&router, "POST", "/prove-consent", None, Some(body)).await;
    assert_eq!(
        status,
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "no consent artifacts must fail closed per request (not boot): {resp}"
    );
}

/// A malformed device-assembled `circuitInput` is a CLIENT error → 400 (not a 5xx), and this holds
/// even on an instance with NO consent artifacts: the input is parsed BEFORE the prover load, so a
/// bad body is distinguished from this instance being unable to prove (503) or a genuine prove
/// failure (502).
#[tokio::test]
async fn prove_consent_malformed_input_is_400() {
    let state = base_state(); // consent_prover defaults to disabled() — no artifacts needed
    let router = vet_api::public_router(state);
    // `{}` cannot parse into `ConsentProveInputs` (missing every named signal).
    let body = json!({ "circuitInput": {} });

    let (status, resp) = common::call(&router, "POST", "/prove-consent", None, Some(body)).await;
    assert_eq!(
        status,
        axum::http::StatusCode::BAD_REQUEST,
        "a malformed circuitInput must be a client 400, not a 5xx: {resp}"
    );
    assert!(
        resp.to_string().contains("consent circuit input"),
        "the 400 body must name the parse cause: {resp}"
    );
}

/// Fail-closed on version: naming a version this route does not serve is refused up front (400),
/// before any prove — the consent route only serves the Level-B consent version.
#[tokio::test]
async fn prove_consent_refuses_a_non_consent_version() {
    let state = base_state();
    let router = vet_api::public_router(state);
    let body = json!({
        "circuitInput": sample_circuit_input(),
        "version": "unsupported/1",
    });

    let (status, resp) = common::call(&router, "POST", "/prove-consent", None, Some(body)).await;
    assert_eq!(
        status,
        axum::http::StatusCode::BAD_REQUEST,
        "a non-consent version must be refused: {resp}"
    );
    assert!(
        resp.to_string().contains("unsupported/1"),
        "the error must name the refused version: {resp}"
    );
    // The consent version explicitly is accepted (reaches the prover; disabled here ⇒ 503, not 400).
    let ok_version = json!({
        "circuitInput": sample_circuit_input(),
        "version": dogtag_prover::artifact::LEVEL_B_V1,
    });
    let (status2, _) = common::call(&router, "POST", "/prove-consent", None, Some(ok_version)).await;
    assert_eq!(
        status2,
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "the consent version must be accepted by the route (then 503 for want of artifacts)"
    );
}

/// Full HTTP prove round-trip through the route with a REAL consent prover. Gated behind
/// `DOGTAG_RUN_CONSENT_ROUTE_PROVE=1` (the prove is multi-minute; the ground truth in
/// `consent_prove.rs` already exercises the prove→verify), and self-skips if artifacts are absent.
#[tokio::test]
async fn prove_consent_full_http_round_trip() {
    if std::env::var("DOGTAG_RUN_CONSENT_ROUTE_PROVE").ok().as_deref() != Some("1") {
        eprintln!("SKIP: set DOGTAG_RUN_CONSENT_ROUTE_PROVE=1 to run the multi-minute HTTP prove");
        return;
    }
    let build_dir = repo_root().join("circuits").join("build");
    if !build_dir.join("consent_final.zkey").exists() {
        eprintln!("SKIP: consent artifacts absent");
        return;
    }
    let mut state = base_state();
    state.consent_prover = Arc::new(ConsentProver::new(Some(build_dir), None));
    let router = vet_api::public_router(state);
    let body = json!({ "circuitInput": sample_circuit_input() });

    let (status, resp) = common::call(&router, "POST", "/prove-consent", None, Some(body)).await;
    assert_eq!(status, axum::http::StatusCode::OK, "resp: {resp}");

    let pubs = resp["pub"].as_array().expect("pub array");
    assert_eq!(pubs.len(), 7, "consent pub must be the 7-vector");
    let a = resp["a"].as_array().expect("a array");
    let b = resp["b"].as_array().expect("b array");
    let c = resp["c"].as_array().expect("c array");
    assert_eq!(a.len(), 2);
    assert_eq!(b.len(), 2);
    assert_eq!(c.len(), 2);
    // The nullifier and R are non-trivial circuit outputs.
    assert_ne!(
        pubs[P::NULLIFIER].as_str().unwrap(),
        "0",
        "nullifier must be a real output"
    );
}
