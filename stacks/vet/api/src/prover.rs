//! Lazily-loaded Groth16 prover for the owner-hidden consent circuit.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use dogtag_prover::{artifact, ConsentProveInputs, Groth16Output, Prover as ArkInnerProver};

#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("prover unavailable: {0}")]
    Unavailable(String),
    #[error("bad input: {0}")]
    BadInput(String),
    #[error("prove failed: {0}")]
    Prove(String),
}

/// A lazily-loaded consent prover. Loading is deferred until the first `/prove-consent` request so a
/// normal vet or groomer instance can run without proving artifacts. Both a successful load and a
/// load error are cached until restart.
pub struct ConsentProver {
    build_dir: Option<PathBuf>,
    expected_zkey: Option<String>,
    slot: OnceLock<Result<Arc<ArkInnerProver>, String>>,
}

impl ConsentProver {
    /// Configure (without loading) from an explicit build directory and optional zkey hash override.
    pub fn new(build_dir: Option<PathBuf>, expected_zkey: Option<String>) -> Self {
        Self {
            build_dir,
            expected_zkey,
            slot: OnceLock::new(),
        }
    }

    /// Configure from `CIRCUITS_BUILD_DIR` and optional `CONSENT_EXPECTED_ZKEY_SHA256`.
    pub fn from_env() -> Self {
        let build_dir = std::env::var("CIRCUITS_BUILD_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let expected_zkey = std::env::var("CONSENT_EXPECTED_ZKEY_SHA256")
            .ok()
            .filter(|s| !s.trim().is_empty());
        Self::new(build_dir, expected_zkey)
    }

    /// A prover that always fails closed as unavailable.
    pub fn disabled() -> Self {
        Self::new(None, None)
    }

    /// The sole protocol version served by this backend prover.
    pub fn version(&self) -> &'static str {
        artifact::LEVEL_B_V1
    }

    fn loaded(&self) -> Result<Arc<ArkInnerProver>, ProverError> {
        let cached = self.slot.get_or_init(|| {
            let dir = self.build_dir.as_ref().ok_or_else(|| {
                "consent prover not configured (CIRCUITS_BUILD_DIR unset)".to_string()
            })?;
            let inner = match self.expected_zkey.as_deref() {
                Some(hex) => ArkInnerProver::load_versioned_with_expected_zkey(
                    dir,
                    &artifact::LEVEL_B_V1_DESCRIPTOR,
                    hex,
                ),
                None => ArkInnerProver::load_versioned(dir, &artifact::LEVEL_B_V1_DESCRIPTOR),
            };
            inner.map(Arc::new).map_err(|e| e.to_string())
        });
        cached
            .as_ref()
            .map(Arc::clone)
            .map_err(|e| ProverError::Unavailable(e.clone()))
    }

    /// Prove a pre-assembled `DogTagConsent(6)` circuit input. The owner assembles inputs locally;
    /// this service performs only the heavy Groth16 operation.
    ///
    /// The request still contains `ownerSecret` and `ownerAddress`, so this is a trusted-prover
    /// fallback and does not hide the owner from the prover operator. It does hide the owner from the
    /// verifier, relayer, chain, and emitted event.
    pub async fn prove(
        &self,
        circuit_input: serde_json::Value,
    ) -> Result<Groth16Output, ProverError> {
        let inputs = ConsentProveInputs::from_circuit_input_json(&circuit_input)
            .map_err(|e| ProverError::BadInput(format!("bad consent circuit input: {e}")))?;
        let inner = self.loaded()?;
        tokio::task::spawn_blocking(move || inner.prove_consent_inputs(inputs))
            .await
            .map_err(|e| ProverError::Prove(format!("join: {e}")))?
            .map_err(|e| ProverError::Prove(e.to_string()))
    }
}
