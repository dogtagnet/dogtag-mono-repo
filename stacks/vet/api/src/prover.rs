//! Groth16 prover client (impl §3.10). The ZK verification path calls `ProverClient::prove(...)`.
//!
//! Two implementations:
//!   * [`StubProver`] — a deterministic placeholder (NOT a real proof) that echoes the public signals
//!     so the ZK control flow is exercised end-to-end without a live proving service. Used by the
//!     hermetic unit/memchain tests.
//!   * [`ArkProver`] — the REAL prover, wrapping `dogtag_prover::Prover` (ark-circom + ark-groth16),
//!     loaded once from a configured circuits `build` directory. It generates a genuine Groth16 proof
//!     ready for `recordVerificationZK`.
//!
//! The 7 public signals are [dogTagId, purpose, relayer, subject, nullifier, keyHash, R] (impl §11.9(d)).

use std::sync::Arc;

use async_trait::async_trait;

pub use dogtag_prover::{Groth16Output, ProveInputs, Prover as ArkInnerProver};

/// Inputs the verify backend hands the prover (impl §3.10).
///
/// `circuit_input_json` carries the FULL circuit-input object (all 19 named circom signals, as
/// produced by `circuits/scripts/gen-zk-fixture.mjs` / `tests/gen_input.mjs`). The real [`ArkProver`]
/// requires it to derive a genuine proof; the [`StubProver`] ignores it and echoes the lean fields.
#[derive(Clone, Debug, Default)]
pub struct ProveInput {
    pub dog_tag_id: String,
    pub purpose: String, // bytes32 hex
    pub relayer: String,
    pub subject: String,
    pub nonce: String,
    pub r: String, // credential root R (bytes32 hex)
    pub eddsa_sig: String,
    /// Full circuit input (the 19 named signals as decimal strings) for the real prover.
    pub circuit_input_json: Option<serde_json::Value>,
}

/// A Groth16 proof + the 7 public signals, ready for `recordVerificationZK`.
#[derive(Clone, Debug)]
pub struct ZkProof {
    pub a: [String; 2],
    pub b: [[String; 2]; 2],
    pub c: [String; 2],
    pub pub_signals: [String; 7],
}

impl From<Groth16Output> for ZkProof {
    fn from(o: Groth16Output) -> Self {
        ZkProof {
            a: o.a,
            b: o.b,
            c: o.c,
            pub_signals: o.public_signals,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("prover unavailable: {0}")]
    Unavailable(String),
    #[error("prove failed: {0}")]
    Prove(String),
}

#[async_trait]
pub trait ProverClient: Send + Sync {
    async fn prove(&self, input: ProveInput) -> Result<ZkProof, ProverError>;
}

/// Placeholder prover — NOT a real Groth16 proof. Returns zeroed (a,b,c) and echoes the public
/// signals so the registry call shape can be assembled. The real [`ArkProver`] replaces this.
pub struct StubProver;

#[async_trait]
impl ProverClient for StubProver {
    async fn prove(&self, input: ProveInput) -> Result<ZkProof, ProverError> {
        let zero = "0".to_string();
        Ok(ZkProof {
            a: [zero.clone(), zero.clone()],
            b: [[zero.clone(), zero.clone()], [zero.clone(), zero.clone()]],
            c: [zero.clone(), zero.clone()],
            pub_signals: [
                input.dog_tag_id,
                input.purpose,
                input.relayer,
                input.subject,
                zero.clone(), // nullifier (circuit output) — placeholder
                zero,         // keyHash (circuit output) — placeholder
                input.r,
            ],
        })
    }
}

/// The REAL Groth16 prover, wrapping `dogtag_prover::Prover`. Loaded once (r1cs + wasm + zkey) from a
/// circuits `build` directory and re-used for every request (each `prove` re-reads the wasm config,
/// which is the prover crate's documented behaviour).
#[derive(Clone)]
pub struct ArkProver {
    inner: Arc<ArkInnerProver>,
}

impl ArkProver {
    /// Load the circuit artifacts from `build_dir` (the `circuits/build` directory).
    pub fn load(build_dir: impl AsRef<std::path::Path>) -> Result<Self, ProverError> {
        let inner = ArkInnerProver::load(build_dir)
            .map_err(|e| ProverError::Unavailable(e.to_string()))?;
        Ok(ArkProver { inner: Arc::new(inner) })
    }

    /// Direct full-fidelity prove from the complete circuit inputs (used by tests / the real ZK leg).
    /// Runs the (CPU-heavy) Groth16 prover on a blocking thread so it never stalls the async runtime.
    pub async fn prove_inputs(&self, inputs: ProveInputs) -> Result<Groth16Output, ProverError> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || inner.prove(inputs))
            .await
            .map_err(|e| ProverError::Prove(format!("join: {e}")))?
            .map_err(|e| ProverError::Prove(e.to_string()))
    }

    /// SHA-256 hex of the loaded zkey (impl §11.8(f)).
    pub fn zkey_hash_hex(&self) -> String {
        self.inner.zkey_hash_hex()
    }
}

#[async_trait]
impl ProverClient for ArkProver {
    async fn prove(&self, input: ProveInput) -> Result<ZkProof, ProverError> {
        let json = input.circuit_input_json.ok_or_else(|| {
            ProverError::Unavailable("real prover requires circuit_input_json".to_string())
        })?;
        let inputs = ProveInputs::from_circuit_input_json(&json)
            .map_err(|e| ProverError::Prove(format!("bad circuit input: {e}")))?;
        let out = self.prove_inputs(inputs).await?;
        Ok(out.into())
    }
}
