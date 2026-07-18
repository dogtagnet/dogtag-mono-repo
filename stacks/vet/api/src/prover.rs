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

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

pub use dogtag_prover::{
    artifact, ArtifactDescriptor, ConsentProveInputs, Groth16Output, ProveInputs,
    Prover as ArkInnerProver,
};

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
    /// The caller asked for a protocol version this prover cannot serve. Fail-closed: another
    /// version's artifacts are never substituted (M7 §7.4).
    #[error("unsupported protocol version {requested:?} (this prover serves {loaded:?})")]
    UnsupportedVersion { requested: String, loaded: String },
}

#[async_trait]
pub trait ProverClient: Send + Sync {
    async fn prove(&self, input: ProveInput) -> Result<ZkProof, ProverError>;

    /// The protocol version this prover proves for — what a request naming no version resolves to.
    ///
    /// Defaults to the current (Level-A) version: the [`StubProver`] stands in for the real prover on
    /// the current version, so the default is right for it.
    fn version(&self) -> &'static str {
        artifact::LEVEL_A_V1
    }

    /// Check that this prover can serve `requested`.
    ///
    /// `None` (the caller named no version) always resolves to [`ProverClient::version`] — the
    /// back-compat contract for every pre-M7 caller. A named version must be the one this prover
    /// loaded; anything else fails closed rather than being proven with the wrong artifacts.
    fn accepts_version(&self, requested: Option<&str>) -> Result<(), ProverError> {
        match requested {
            None => Ok(()),
            Some(v) if v == self.version() => Ok(()),
            Some(v) => Err(ProverError::UnsupportedVersion {
                requested: v.to_string(),
                loaded: self.version().to_string(),
            }),
        }
    }
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
    /// Load the CURRENT (Level-A) version's circuit artifacts from `build_dir` (the `circuits/build`
    /// directory), enforcing that version's pinned zkey hash.
    pub fn load(build_dir: impl AsRef<std::path::Path>) -> Result<Self, ProverError> {
        Self::load_versioned(build_dir, artifact::current(), None)
    }

    /// Like [`ArkProver::load`] but pins an explicit expected zkey SHA-256 (lowercase hex), so a
    /// deployment shipping a different proving key (e.g. a production ceremony output) is not blocked
    /// by the crate's hardcoded testnet hash.
    pub fn load_with_expected_zkey(
        build_dir: impl AsRef<std::path::Path>,
        expected_zkey_sha256_hex: &str,
    ) -> Result<Self, ProverError> {
        Self::load_versioned(build_dir, artifact::current(), Some(expected_zkey_sha256_hex))
    }

    /// Load the artifacts a named protocol version pins, optionally overriding its zkey hash.
    ///
    /// Resolve `descriptor` from a version key with [`artifact::resolve`] (`None` ⇒ the current
    /// version). Nothing is fetched: `build_dir` must already hold the files.
    pub fn load_versioned(
        build_dir: impl AsRef<std::path::Path>,
        descriptor: &'static ArtifactDescriptor,
        expected_zkey_sha256_hex: Option<&str>,
    ) -> Result<Self, ProverError> {
        let inner = match expected_zkey_sha256_hex {
            Some(hex) => ArkInnerProver::load_versioned_with_expected_zkey(build_dir, descriptor, hex),
            None => ArkInnerProver::load_versioned(build_dir, descriptor),
        }
        .map_err(|e| ProverError::Unavailable(e.to_string()))?;
        Ok(ArkProver {
            inner: Arc::new(inner),
        })
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

    /// The protocol version whose artifacts this prover loaded.
    pub fn version(&self) -> &'static str {
        self.inner.version()
    }
}

#[async_trait]
impl ProverClient for ArkProver {
    fn version(&self) -> &'static str {
        self.inner.version()
    }

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

/// A lazily-loaded Level-B **consent** prover (M7 P0).
///
/// Unlike the Level-A [`ArkProver`], which is loaded at BOOT (fail-closed boot), the consent prover
/// is loaded on the FIRST `/prove-consent` request, and a load failure fails THAT REQUEST — not boot.
/// This is the coexistence contract (M7 §3.5): a prover-service serving Level-A must NOT refuse to
/// start just because the consent artifacts are absent. The loaded prover is cached (`version ->
/// Arc<Prover>`), so subsequent requests reuse it; a cached load ERROR is also retained (a broken
/// artifact set fails closed on every request until the service is restarted with a fixed one).
///
/// It is kept OUT of the shared [`ProverClient`] trait on purpose: the consent circuit's inputs
/// ([`ConsentProveInputs`]) and its public-signal ORDER differ from Level-A's, so conflating them
/// behind one trait would invite feeding a consent request through the Level-A signal names.
pub struct ConsentProver {
    /// The circuits `build` dir the consent artifacts live in (`CIRCUITS_BUILD_DIR`). `None` ⇒ this
    /// instance cannot prove consent, and every `/prove-consent` request fails closed (unavailable).
    build_dir: Option<PathBuf>,
    /// Optional zkey-hash override (a production ceremony output), mirroring `EXPECTED_ZKEY_SHA256`.
    expected_zkey: Option<String>,
    /// The lazily-loaded, cached prover (or the cached load error). Loaded on first request.
    slot: OnceLock<Result<Arc<ArkInnerProver>, String>>,
}

impl ConsentProver {
    /// Configure (do NOT load) a consent prover from an explicit build dir + optional zkey override.
    pub fn new(build_dir: Option<PathBuf>, expected_zkey: Option<String>) -> Self {
        Self {
            build_dir,
            expected_zkey,
            slot: OnceLock::new(),
        }
    }

    /// Configure from the environment: `CIRCUITS_BUILD_DIR` (the same dir the Level-A boot prover
    /// uses) + optional `CONSENT_EXPECTED_ZKEY_SHA256`. Nothing is loaded here (lazy, per M7 §3.5).
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

    /// A consent prover that can never prove (no build dir) — every request fails closed. The default
    /// for local/demo instances and tests that do not exercise the real consent prove.
    pub fn disabled() -> Self {
        Self::new(None, None)
    }

    /// The protocol version this route serves.
    pub fn version(&self) -> &'static str {
        artifact::LEVEL_B_V1
    }

    /// Lazily load (once) and return the inner consent prover, or a per-request `Unavailable` error.
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

    /// Prove a PRE-ASSEMBLED consent circuit input (the shape `consent_assemble` emits). Fail-closed
    /// per request: a missing/hash-mismatched artifact set, or a malformed input, errors THIS request.
    ///
    /// The device assembles the consent inputs locally (cheap field math that runs fine on 32-bit ARM)
    /// and POSTs the assembled `circuit_input`; only the heavy Groth16 prove runs here. The wallet
    /// SEED therefore never reaches this service — owner-unlinkability is preserved even for the
    /// server-prove fallback (unlike Level-A, the consent witness cannot be assembled server-side
    /// without the seed, so it is assembled on-device).
    pub async fn prove(
        &self,
        circuit_input: serde_json::Value,
    ) -> Result<ZkProof, ProverError> {
        let inner = self.loaded()?;
        let inputs = ConsentProveInputs::from_circuit_input_json(&circuit_input)
            .map_err(|e| ProverError::Prove(format!("bad consent circuit input: {e}")))?;
        let out = tokio::task::spawn_blocking(move || inner.prove_consent_inputs(inputs))
            .await
            .map_err(|e| ProverError::Prove(format!("join: {e}")))?
            .map_err(|e| ProverError::Prove(e.to_string()))?;
        Ok(out.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stub echoes the four lean caller-supplied signals into pub_signals[0..4]
    /// and R into pub_signals[6], zeroes the two circuit-output signals (nullifier,
    /// keyHash) at [4]/[5], and returns an all-zero (a,b,c) proof.
    #[tokio::test]
    async fn stub_prove_echoes_lean_signals_and_zeros_proof() {
        let input = ProveInput {
            dog_tag_id: "111".to_string(),
            purpose: "0xpurpose".to_string(),
            relayer: "0xrelayer".to_string(),
            subject: "0xsubject".to_string(),
            nonce: "ignored-nonce".to_string(),
            r: "0xroot".to_string(),
            eddsa_sig: "ignored-sig".to_string(),
            circuit_input_json: None,
        };
        let p = StubProver.prove(input).await.unwrap();

        // (a,b,c) are all zero — it is NOT a real proof.
        assert_eq!(p.a, ["0".to_string(), "0".to_string()]);
        assert_eq!(
            p.b,
            [
                ["0".to_string(), "0".to_string()],
                ["0".to_string(), "0".to_string()]
            ]
        );
        assert_eq!(p.c, ["0".to_string(), "0".to_string()]);

        // Public signals: [dogTagId, purpose, relayer, subject, nullifier=0, keyHash=0, R].
        assert_eq!(
            p.pub_signals,
            [
                "111".to_string(),
                "0xpurpose".to_string(),
                "0xrelayer".to_string(),
                "0xsubject".to_string(),
                "0".to_string(),
                "0".to_string(),
                "0xroot".to_string(),
            ]
        );
    }

    /// The stub ignores `circuit_input_json`, `nonce`, and `eddsa_sig`; only the lean
    /// fields drive the output, so supplying them does not change the result.
    #[tokio::test]
    async fn stub_prove_ignores_circuit_input_and_secret_fields() {
        let base = ProveInput {
            dog_tag_id: "7".to_string(),
            purpose: "p".to_string(),
            relayer: "rel".to_string(),
            subject: "sub".to_string(),
            r: "r".to_string(),
            ..Default::default()
        };
        let mut with_extras = base.clone();
        with_extras.nonce = "abc".to_string();
        with_extras.eddsa_sig = "def".to_string();
        with_extras.circuit_input_json = Some(serde_json::json!({"anything": 1}));

        let a = StubProver.prove(base).await.unwrap();
        let b = StubProver.prove(with_extras).await.unwrap();
        assert_eq!(a.pub_signals, b.pub_signals);
        assert_eq!(a.a, b.a);
    }

    /// `From<Groth16Output>` is a verbatim field copy into `ZkProof`.
    #[test]
    fn zkproof_from_groth16output_preserves_all_fields() {
        let out = Groth16Output {
            a: ["1".to_string(), "2".to_string()],
            b: [
                ["3".to_string(), "4".to_string()],
                ["5".to_string(), "6".to_string()],
            ],
            c: ["7".to_string(), "8".to_string()],
            public_signals: [
                "s0".to_string(),
                "s1".to_string(),
                "s2".to_string(),
                "s3".to_string(),
                "s4".to_string(),
                "s5".to_string(),
                "s6".to_string(),
            ],
        };
        let z: ZkProof = out.clone().into();
        assert_eq!(z.a, out.a);
        assert_eq!(z.b, out.b);
        assert_eq!(z.c, out.c);
        assert_eq!(z.pub_signals, out.public_signals);
    }
}
