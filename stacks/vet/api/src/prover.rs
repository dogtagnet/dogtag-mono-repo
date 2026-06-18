//! Groth16 prover client (impl §3.10). The ZK verification path calls `ProverClient::prove(...)`; the
//! real `dogtag-prover-rs` (ark-circom + ark-groth16) is wired in later. Until then a `StubProver`
//! returns a deterministic placeholder proof so the ZK control flow is exercised end-to-end without a
//! live proving service. The 7 public signals are [dogTagId, purpose, relayer, subject, nullifier,
//! keyHash, R] (impl §11.9(d)).

use async_trait::async_trait;

/// Inputs the verify backend hands the prover (impl §3.10).
#[derive(Clone, Debug)]
pub struct ProveInput {
    pub dog_tag_id: String,
    pub purpose: String, // bytes32 hex
    pub relayer: String,
    pub subject: String,
    pub nonce: String,
    pub r: String, // credential root R (bytes32 hex)
    pub eddsa_sig: String,
}

/// A Groth16 proof + the 7 public signals, ready for `recordVerificationZK`.
#[derive(Clone, Debug)]
pub struct ZkProof {
    pub a: [String; 2],
    pub b: [[String; 2]; 2],
    pub c: [String; 2],
    pub pub_signals: [String; 7],
}

#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("prover unavailable: {0}")]
    Unavailable(String),
}

#[async_trait]
pub trait ProverClient: Send + Sync {
    async fn prove(&self, input: ProveInput) -> Result<ZkProof, ProverError>;
}

/// Placeholder prover — NOT a real Groth16 proof. Returns zeroed (a,b,c) and echoes the public
/// signals so the registry call shape can be assembled. The real prover replaces this.
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
