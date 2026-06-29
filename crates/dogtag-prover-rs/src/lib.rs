//! DogTag Groth16 proving service.
//!
//! Generates a Groth16 proof for the DogTag verification circuit
//! (`circuits/verification.circom`, `DogTagVerification(24, 5)`) so the vet
//! backend's ZK path can submit `recordVerificationZK` on-chain.
//!
//! The circuit's public-signal vector (snarkjs order — all seven are circuit
//! OUTPUTS declared in spec order) is:
//!
//! ```text
//! [dogTagId, purpose, relayer, subject, nullifier, keyHash, R]
//! ```
//!
//! # API
//!
//! - [`Prover::load`] — load the r1cs + wasm witness calculator + zkey ONCE.
//! - [`Prover::prove`] — generate a [`Groth16Output`] per request.
//! - [`Prover::zkey_hash`] — SHA-256 of the zkey file (pinned at load, impl §11.8(f)).
//!
//! # ark version isolation
//!
//! This crate uses the **ark 0.6** stack (pulled in transitively by `ark-circom`
//! 0.6). The workspace's `dogtag-standard-rs` pins **ark 0.5**. The two majors
//! coexist in the lockfile; to keep them from clashing this crate deliberately
//! does NOT depend on `dogtag-standard-rs` and exposes only strings at its public
//! boundary — no ark types cross out.
//!
//! [`Groth16Output`] is formatted exactly as `snarkjs zkey export
//! soliditycalldata` would emit it, i.e. with the snarkjs→Solidity **b-coordinate
//! swap** applied (`b[0] = [bx_c1, bx_c0]`, `b[1] = [by_c1, by_c0]`), so it drops
//! straight into the on-chain
//! `Groth16Verifier.verifyProof(uint[2], uint[2][2], uint[2], uint[7])`.

use std::path::{Path, PathBuf};

use ark_bn254::{Bn254, Fr};
use ark_circom::{read_zkey, CircomBuilder, CircomConfig, CircomReduction};
use ark_crypto_primitives::snark::SNARK;
use ark_ff::PrimeField;
use ark_groth16::{Groth16, ProvingKey};
use num_bigint::BigInt;
use sha2::{Digest, Sha256};

/// `N` — maximum number of leaves the circuit supports (`DogTagVerification(24, 5)`).
pub const N: usize = 24;

/// Number of public signals the circuit exposes:
/// `[dogTagId, purpose, relayer, subject, nullifier, keyHash, R]`.
pub const NUM_PUBLIC: usize = 7;

/// Expected SHA-256 (lowercase hex) of the pinned `verification_final.zkey` — the testnet self-run
/// ceremony output recorded in `contracts/deployments/roax.json` (`_zk_ceremony`) and
/// `docs/CEREMONY_TRANSCRIPT.md`. [`Prover::load`] refuses any zkey whose hash differs, so a swapped
/// or corrupt proving key fails closed instead of silently producing proofs against the wrong key
/// (audit M4). A deployment pinning a DIFFERENT zkey (e.g. a production ceremony output) loads it via
/// [`Prover::load_with_expected_zkey`].
pub const EXPECTED_ZKEY_SHA256_HEX: &str =
    "45d0b6fb78591548f5763e86f614d1c04cf48a80d35445d1740c0ba561fdc03e";

/// Errors that can arise while loading artifacts or proving.
#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("io error reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to load circuit artifacts: {0}")]
    Load(String),
    #[error("invalid input field {field}: {reason}")]
    Input { field: String, reason: String },
    #[error("witness/proof generation failed: {0}")]
    Prove(String),
    #[error("self-verification of generated proof failed")]
    Verify,
    #[error("zkey hash mismatch: expected {expected}, got {got}")]
    ZkeyHashMismatch { expected: String, got: String },
    #[error("unexpected public-signal count: got {got}, expected {expected}")]
    PublicSignals { got: usize, expected: usize },
}

/// Named inputs mirroring the circuit's private signals.
///
/// Every value is a decimal string (a base-10 field element / integer). Fixed-width
/// arrays carry exactly `N = 24` entries (the circuit width); only the first
/// `numLeaves` are semantically meaningful, the rest are the padding/identity slots.
#[derive(Debug, Clone)]
pub struct ProveInputs {
    pub dog_tag_id: String,
    pub purpose: String,
    pub relayer: String,
    pub subject: String,
    pub num_leaves: String,
    pub leaf_key_path_hashes: [String; N],
    pub leaf_type_tags: [String; N],
    pub leaf_salts: [String; N],
    pub leaf_values: [String; N],
    pub dog_tag_id_leaf_index: String,
    pub sorted_leaf_hashes: [String; N],
    pub perm: [String; N],
    pub dog_tag_key_path_field: String,
    pub consent_nonce: String,
    pub ax: String,
    pub ay: String,
    pub r8x: String,
    pub r8y: String,
    pub s: String,
}

/// A Groth16 proof formatted as the on-chain Solidity calldata expects.
///
/// - `a` / `c` are G1 points `[x, y]`.
/// - `b` is a G2 point with the snarkjs→Solidity coordinate swap already applied:
///   `b[0] = [bx_c1, bx_c0]`, `b[1] = [by_c1, by_c0]`.
/// - `pub` is the public-signal vector in order
///   `[dogTagId, purpose, relayer, subject, nullifier, keyHash, R]`.
///
/// All values are base-10 decimal strings (`Groth16Verifier.verifyProof` takes
/// `uint256`s; decimal is accepted).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Groth16Output {
    pub a: [String; 2],
    pub b: [[String; 2]; 2],
    pub c: [String; 2],
    #[serde(rename = "pub")]
    pub public_signals: [String; NUM_PUBLIC],
}

/// A loaded prover: r1cs + wasm witness calculator + zkey, all parsed once.
///
/// `prove` is `&self` and re-buildable per request, but the underlying
/// `CircomConfig` (in particular the wasmer witness calculator + store) is not
/// `Sync`/cheaply shareable, so we re-read the config per `prove` call from the
/// cached paths. The expensive zkey/proving-key parse happens once at load.
pub struct Prover {
    build_dir: PathBuf,
    wasm_path: PathBuf,
    r1cs_path: PathBuf,
    proving_key: ProvingKey<Bn254>,
    zkey_sha256: [u8; 32],
}

impl Prover {
    /// Load the circuit artifacts from `build_dir` (the `circuits/build` directory), enforcing the
    /// pinned [`EXPECTED_ZKEY_SHA256_HEX`] — a zkey whose hash differs is rejected (audit M4).
    ///
    /// Expects:
    /// - `verification.r1cs`
    /// - `verification_js/verification.wasm`
    /// - `verification_final.zkey`
    pub fn load(build_dir: impl AsRef<Path>) -> Result<Self, ProverError> {
        Self::load_with_expected_zkey(build_dir, EXPECTED_ZKEY_SHA256_HEX)
    }

    /// Like [`Prover::load`] but pins an explicit expected zkey SHA-256 (lowercase hex). Use this when
    /// a deployment ships a different proving key (e.g. a production ceremony output) than the bundled
    /// testnet one. The hash is checked BEFORE the (expensive) proving-key parse, so a wrong artifact
    /// fails fast and closed.
    pub fn load_with_expected_zkey(
        build_dir: impl AsRef<Path>,
        expected_zkey_sha256_hex: &str,
    ) -> Result<Self, ProverError> {
        let build_dir = build_dir.as_ref().to_path_buf();
        let r1cs_path = build_dir.join("verification.r1cs");
        let wasm_path = build_dir.join("verification_js").join("verification.wasm");
        let zkey_path = build_dir.join("verification_final.zkey");

        for p in [&r1cs_path, &wasm_path, &zkey_path] {
            if !p.exists() {
                return Err(ProverError::Load(format!("missing artifact: {}", p.display())));
            }
        }

        // Pin the zkey hash at load (impl §11.8(f)) and ENFORCE it: reject a swapped/corrupt key.
        let zkey_bytes = std::fs::read(&zkey_path).map_err(|e| ProverError::Io {
            path: zkey_path.display().to_string(),
            source: e,
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&zkey_bytes);
        let zkey_sha256: [u8; 32] = hasher.finalize().into();
        let got_hex = hex::encode(zkey_sha256);
        if !got_hex.eq_ignore_ascii_case(expected_zkey_sha256_hex) {
            return Err(ProverError::ZkeyHashMismatch {
                expected: expected_zkey_sha256_hex.to_ascii_lowercase(),
                got: got_hex,
            });
        }

        // Parse the proving key once.
        let mut zkey_reader = std::io::Cursor::new(zkey_bytes);
        let (proving_key, _matrices) =
            read_zkey(&mut zkey_reader).map_err(|e| ProverError::Load(format!("read_zkey: {e}")))?;

        // Validate the r1cs + wasm load up-front so `load` fails fast on bad artifacts.
        CircomConfig::<Fr>::new(&wasm_path, &r1cs_path)
            .map_err(|e| ProverError::Load(format!("CircomConfig::new: {e}")))?;

        Ok(Self {
            build_dir,
            wasm_path,
            r1cs_path,
            proving_key,
            zkey_sha256,
        })
    }

    /// The build directory the prover was loaded from.
    pub fn build_dir(&self) -> &Path {
        &self.build_dir
    }

    /// SHA-256 of the `verification_final.zkey` file, pinned at load (impl §11.8(f)).
    pub fn zkey_hash(&self) -> [u8; 32] {
        self.zkey_sha256
    }

    /// Lowercase hex of [`Prover::zkey_hash`].
    pub fn zkey_hash_hex(&self) -> String {
        hex::encode(self.zkey_sha256)
    }

    /// Generate a Groth16 proof for the given inputs.
    pub fn prove(&self, inputs: ProveInputs) -> Result<Groth16Output, ProverError> {
        // A fresh CircomConfig per call: the wasmer witness calculator + store are
        // not cheaply shareable across threads/calls. The r1cs parse is the only
        // re-done cost here; the proving key is reused.
        let cfg = CircomConfig::<Fr>::new(&self.wasm_path, &self.r1cs_path)
            .map_err(|e| ProverError::Prove(format!("CircomConfig::new: {e}")))?;
        let mut builder = CircomBuilder::new(cfg);

        push_named_inputs(&mut builder, &inputs)?;

        let circom = builder
            .build()
            .map_err(|e| ProverError::Prove(format!("witness build: {e}")))?;

        let public_inputs = circom
            .get_public_inputs()
            .ok_or_else(|| ProverError::Prove("circuit produced no public inputs".into()))?;

        if public_inputs.len() != NUM_PUBLIC {
            return Err(ProverError::PublicSignals {
                got: public_inputs.len(),
                expected: NUM_PUBLIC,
            });
        }

        let mut rng = rand::thread_rng();
        let proof = Groth16::<Bn254, CircomReduction>::prove(&self.proving_key, circom, &mut rng)
            .map_err(|e| ProverError::Prove(format!("groth16 prove: {e}")))?;

        // Self-verify in-process before handing back (cheap, catches a broken artifact set).
        let pvk = Groth16::<Bn254>::process_vk(&self.proving_key.vk)
            .map_err(|e| ProverError::Prove(format!("process_vk: {e}")))?;
        let ok = Groth16::<Bn254>::verify_with_processed_vk(&pvk, &public_inputs, &proof)
            .map_err(|e| ProverError::Prove(format!("verify: {e}")))?;
        if !ok {
            return Err(ProverError::Verify);
        }

        Ok(format_output(&proof, &public_inputs))
    }
}

/// Convert a BN254 base/scalar field element to a base-10 decimal string.
fn fe_to_dec<F: PrimeField>(f: &F) -> String {
    f.into_bigint().to_string()
}

/// Format an arkworks Groth16 proof + public inputs into the Solidity-calldata
/// [`Groth16Output`], applying the snarkjs→Solidity b-coordinate swap.
///
/// Mirrors `ark_circom::ethereum`'s `G2::as_tuple` (which swaps c1/c0) — the same
/// swap `snarkjs zkey export soliditycalldata` performs.
fn format_output(proof: &ark_groth16::Proof<Bn254>, public_inputs: &[Fr]) -> Groth16Output {
    let a = [fe_to_dec(&proof.a.x), fe_to_dec(&proof.a.y)];
    let c = [fe_to_dec(&proof.c.x), fe_to_dec(&proof.c.y)];

    // G2: arkworks stores x = c0 + c1*u, y = c0 + c1*u.
    // Solidity (snarkjs) expects each Fq2 limb as [c1, c0] (the swap).
    let b = [
        [fe_to_dec(&proof.b.x.c1), fe_to_dec(&proof.b.x.c0)],
        [fe_to_dec(&proof.b.y.c1), fe_to_dec(&proof.b.y.c0)],
    ];

    let mut public_signals: [String; NUM_PUBLIC] = Default::default();
    for (slot, fr) in public_signals.iter_mut().zip(public_inputs.iter()) {
        *slot = fe_to_dec(fr);
    }

    Groth16Output {
        a,
        b,
        c,
        public_signals,
    }
}

fn parse_bigint(field: &str, value: &str) -> Result<BigInt, ProverError> {
    value.parse::<BigInt>().map_err(|e| ProverError::Input {
        field: field.to_string(),
        reason: format!("not a decimal integer: {e}"),
    })
}

fn push_scalar(
    builder: &mut CircomBuilder<Fr>,
    name: &str,
    value: &str,
) -> Result<(), ProverError> {
    builder.push_input(name, parse_bigint(name, value)?);
    Ok(())
}

fn push_array(
    builder: &mut CircomBuilder<Fr>,
    name: &str,
    values: &[String; N],
) -> Result<(), ProverError> {
    for v in values.iter() {
        builder.push_input(name, parse_bigint(name, v)?);
    }
    Ok(())
}

/// Push the circuit's named signals into the builder, in the circom signal names.
fn push_named_inputs(
    builder: &mut CircomBuilder<Fr>,
    inputs: &ProveInputs,
) -> Result<(), ProverError> {
    push_scalar(builder, "dogTagId", &inputs.dog_tag_id)?;
    push_scalar(builder, "purpose", &inputs.purpose)?;
    push_scalar(builder, "relayer", &inputs.relayer)?;
    push_scalar(builder, "subject", &inputs.subject)?;
    push_scalar(builder, "numLeaves", &inputs.num_leaves)?;
    push_array(builder, "leafKeyPathHashes", &inputs.leaf_key_path_hashes)?;
    push_array(builder, "leafTypeTags", &inputs.leaf_type_tags)?;
    push_array(builder, "leafSalts", &inputs.leaf_salts)?;
    push_array(builder, "leafValues", &inputs.leaf_values)?;
    push_scalar(builder, "dogTagIdLeafIndex", &inputs.dog_tag_id_leaf_index)?;
    push_array(builder, "sortedLeafHashes", &inputs.sorted_leaf_hashes)?;
    push_array(builder, "perm", &inputs.perm)?;
    push_scalar(builder, "dogTagKeyPathField", &inputs.dog_tag_key_path_field)?;
    push_scalar(builder, "consentNonce", &inputs.consent_nonce)?;
    push_scalar(builder, "Ax", &inputs.ax)?;
    push_scalar(builder, "Ay", &inputs.ay)?;
    push_scalar(builder, "R8x", &inputs.r8x)?;
    push_scalar(builder, "R8y", &inputs.r8y)?;
    push_scalar(builder, "S", &inputs.s)?;
    Ok(())
}

impl ProveInputs {
    /// Build [`ProveInputs`] from the JSON object produced by the circuits'
    /// `gen-zk-fixture.mjs` input builder (all string-valued; arrays of length `N`).
    ///
    /// Useful for tests / cross-checking against the snarkjs pipeline.
    pub fn from_circuit_input_json(v: &serde_json::Value) -> Result<Self, ProverError> {
        fn s(v: &serde_json::Value, k: &str) -> Result<String, ProverError> {
            v.get(k)
                .and_then(|x| x.as_str())
                .map(|x| x.to_string())
                .ok_or_else(|| ProverError::Input {
                    field: k.to_string(),
                    reason: "missing or not a string".into(),
                })
        }
        fn arr(v: &serde_json::Value, k: &str) -> Result<[String; N], ProverError> {
            let raw = v
                .get(k)
                .and_then(|x| x.as_array())
                .ok_or_else(|| ProverError::Input {
                    field: k.to_string(),
                    reason: "missing or not an array".into(),
                })?;
            if raw.len() != N {
                return Err(ProverError::Input {
                    field: k.to_string(),
                    reason: format!("expected {N} entries, got {}", raw.len()),
                });
            }
            let mut out: [String; N] = Default::default();
            for (i, item) in raw.iter().enumerate() {
                out[i] = item.as_str().map(|x| x.to_string()).ok_or_else(|| {
                    ProverError::Input {
                        field: format!("{k}[{i}]"),
                        reason: "not a string".into(),
                    }
                })?;
            }
            Ok(out)
        }

        Ok(ProveInputs {
            dog_tag_id: s(v, "dogTagId")?,
            purpose: s(v, "purpose")?,
            relayer: s(v, "relayer")?,
            subject: s(v, "subject")?,
            num_leaves: s(v, "numLeaves")?,
            leaf_key_path_hashes: arr(v, "leafKeyPathHashes")?,
            leaf_type_tags: arr(v, "leafTypeTags")?,
            leaf_salts: arr(v, "leafSalts")?,
            leaf_values: arr(v, "leafValues")?,
            dog_tag_id_leaf_index: s(v, "dogTagIdLeafIndex")?,
            sorted_leaf_hashes: arr(v, "sortedLeafHashes")?,
            perm: arr(v, "perm")?,
            dog_tag_key_path_field: s(v, "dogTagKeyPathField")?,
            consent_nonce: s(v, "consentNonce")?,
            ax: s(v, "Ax")?,
            ay: s(v, "Ay")?,
            r8x: s(v, "R8x")?,
            r8y: s(v, "R8y")?,
            s: s(v, "S")?,
        })
    }
}
