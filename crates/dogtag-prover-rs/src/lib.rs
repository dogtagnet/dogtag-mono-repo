//! DogTag Groth16 proving service.
//!
//! Generates a Groth16 proof for the owner-hidden consent circuit
//! (`circuits/consent.circom`, `DogTagConsent(6)`) so the backend's server-prove
//! fallback (`/prove-consent`) can hand back on-chain-submittable calldata.
//!
//! The circuit's public-signal vector (snarkjs order — all seven are circuit
//! OUTPUTS declared in spec order) is:
//!
//! ```text
//! [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
//! ```
//!
//! # API
//!
//! - [`Prover::load_versioned`] — load the r1cs + wasm witness calculator + zkey ONCE, for a named
//!   protocol version.
//! - [`Prover::prove_consent_inputs`] — generate a [`Groth16Output`] per request.
//! - [`Prover::zkey_hash`] — SHA-256 of the zkey file (pinned at load, impl §11.8(f)).
//!
//! # Version-keyed artifacts
//!
//! Which files a prover loads, and the hashes it pins them to, come from an
//! [`artifact::ArtifactDescriptor`] resolved by protocol version ([`artifact::resolve`]) rather than
//! from hard-coded filenames. The consent set ([`artifact::LEVEL_B_V1_DESCRIPTOR`]) is the sole
//! entry and the default. See [`artifact`] for the model — including why the zkey hash and the VK
//! hash are distinct fields, and why nothing is fetched here.
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
//! `Groth16VerifierConsent.verifyProof(uint[2], uint[2][2], uint[2], uint[7])`.

use std::path::{Path, PathBuf};

use ark_bn254::{Bn254, Fr};
use ark_circom::{read_zkey, CircomBuilder, CircomConfig, CircomReduction};
use ark_crypto_primitives::snark::SNARK;
use ark_ff::PrimeField;
use ark_groth16::{Groth16, ProvingKey};
use num_bigint::BigInt;
use sha2::{Digest, Sha256};

pub mod artifact;
pub mod manifest;

pub use artifact::ArtifactDescriptor;

/// Number of public signals the consent circuit exposes:
/// `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`.
///
/// The width of [`Groth16Output::public_signals`]. Per-version it is
/// [`ArtifactDescriptor::num_public`], which [`Prover::load_versioned`] checks against this width.
pub const NUM_PUBLIC: usize = 7;

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
    #[error("{artifact} hash mismatch: expected {expected}, got {got}")]
    ArtifactHashMismatch {
        artifact: String,
        expected: String,
        got: String,
    },
    #[error("unexpected public-signal count: got {got}, expected {expected}")]
    PublicSignals { got: usize, expected: usize },
    /// A caller named a protocol version this build has no artifact set for. Fail-closed: never
    /// substitute another version's artifacts (`artifact::resolve`).
    #[error("unknown protocol version {version:?} (known: {})", known.join(", "))]
    UnknownVersion { version: String, known: Vec<String> },
}

/// Inclusion-path depth for the consent circuit — `component main = DogTagConsent(6)`.
pub const CONSENT_DEPTH: usize = 6;

/// Named inputs mirroring the **consent** circuit's private signals (`DogTagConsent(6)`).
///
/// Every value is a decimal string (a base-10 field element). The three inclusion paths are
/// front-packed: `*_siblings[0..*_path_len]` are the real siblings (leaf→root order), the tail is
/// zero-padded to [`CONSENT_DEPTH`], exactly as `consent.circom`'s `MerkleInclusion` consumes them.
/// This is the shape the SDK's `consent_assemble::consent_circuit_input_value` emits.
#[derive(Debug, Clone)]
pub struct ConsentProveInputs {
    pub dog_tag_id: String,
    pub purpose: String,
    pub relayer: String,
    pub record_type: String,
    pub deadline: String,
    pub consent_nonce: String,
    pub owner_address: String,
    pub owner_secret: String,
    pub ax: String,
    pub ay: String,
    pub r8x: String,
    pub r8y: String,
    pub s: String,
    pub owner_salt: String,
    pub key_salt: String,
    pub secret_salt: String,
    pub owner_siblings: [String; CONSENT_DEPTH],
    pub owner_path_len: String,
    pub key_siblings: [String; CONSENT_DEPTH],
    pub key_path_len: String,
    pub secret_siblings: [String; CONSENT_DEPTH],
    pub secret_path_len: String,
}

/// A Groth16 proof formatted as the on-chain Solidity calldata expects.
///
/// - `a` / `c` are G1 points `[x, y]`.
/// - `b` is a G2 point with the snarkjs→Solidity coordinate swap already applied:
///   `b[0] = [bx_c1, bx_c0]`, `b[1] = [by_c1, by_c0]`.
/// - `pub` is the public-signal vector, in the consent circuit's frozen OUTPUT order:
///   `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`.
///
///   Read these through the named constants in `dogtag_standard::public_signals::level_b` rather
///   than by literal index. (This crate cannot import them: it pins the ark 0.6 stack while
///   `dogtag-standard-rs` pins 0.5, and the two coexist only because ark types never cross the
///   boundary — see the note in `Cargo.toml`. Keep the two definitions in step by hand.)
///
/// All values are base-10 decimal strings (`Groth16VerifierConsent.verifyProof` takes
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
    descriptor: &'static ArtifactDescriptor,
    wasm_path: PathBuf,
    r1cs_path: PathBuf,
    proving_key: ProvingKey<Bn254>,
    zkey_sha256: [u8; 32],
}

impl Prover {
    /// Load the artifacts a specific protocol version names, from `build_dir`.
    ///
    /// The descriptor supplies the filenames AND the hashes: every pinned file is verified before it
    /// is parsed, so a swapped artifact fails closed regardless of which version asked for it. Resolve
    /// a descriptor from a version key with [`artifact::resolve`].
    ///
    /// Nothing is fetched — `build_dir` must already hold the files (see [`artifact`]).
    pub fn load_versioned(
        build_dir: impl AsRef<Path>,
        descriptor: &'static ArtifactDescriptor,
    ) -> Result<Self, ProverError> {
        Self::load_inner(build_dir, descriptor, None)
    }

    /// [`Prover::load_versioned`] with the version's zkey pin overridden by an explicit expected
    /// SHA-256 — the deployment-config escape hatch for a deployment that ships a different proving
    /// key (e.g. a production ceremony output) than the bundled testnet one.
    pub fn load_versioned_with_expected_zkey(
        build_dir: impl AsRef<Path>,
        descriptor: &'static ArtifactDescriptor,
        expected_zkey_sha256_hex: &str,
    ) -> Result<Self, ProverError> {
        Self::load_inner(build_dir, descriptor, Some(expected_zkey_sha256_hex))
    }

    /// The one real load path every public loader composes from.
    ///
    /// `expected_zkey_override`: `None` uses the descriptor's own pin; `Some(hex)` replaces it (the
    /// deployment override). Either way the check is mandatory — there is no unpinned zkey load.
    fn load_inner(
        build_dir: impl AsRef<Path>,
        descriptor: &'static ArtifactDescriptor,
        expected_zkey_override: Option<&str>,
    ) -> Result<Self, ProverError> {
        // The output's public-signal array is a fixed NUM_PUBLIC width, so a version whose circuit
        // exposes a different count cannot be formatted by this build. Refuse at load rather than
        // silently truncating/padding `pub` at prove time.
        if descriptor.num_public != NUM_PUBLIC {
            return Err(ProverError::Load(format!(
                "version {} exposes {} public signals, but this build formats {NUM_PUBLIC}",
                descriptor.version, descriptor.num_public
            )));
        }

        let build_dir = build_dir.as_ref().to_path_buf();
        let r1cs_path = build_dir.join(descriptor.r1cs.rel_path);
        let wasm_path = build_dir.join(descriptor.wasm.rel_path);
        let zkey_path = build_dir.join(descriptor.zkey.rel_path);

        for p in [&r1cs_path, &wasm_path, &zkey_path] {
            if !p.exists() {
                return Err(ProverError::Load(format!("missing artifact: {}", p.display())));
            }
        }

        // Pin the zkey hash at load (impl §11.8(f)) and ENFORCE it: reject a swapped/corrupt key.
        let expected_zkey_sha256_hex = expected_zkey_override.unwrap_or(descriptor.zkey.sha256);
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

        // The witness artifacts carry the same discipline as the zkey: verify before parse, so the
        // constraint system / witness calculator a proof is built from is the one the version pinned.
        // An unpinned file (`sha256: None`) is skipped — see `artifact::ArtifactFile`.
        verify_pinned_file(&r1cs_path, &descriptor.r1cs)?;
        verify_pinned_file(&wasm_path, &descriptor.wasm)?;

        // Parse the proving key once.
        let mut zkey_reader = std::io::Cursor::new(zkey_bytes);
        let (proving_key, _matrices) =
            read_zkey(&mut zkey_reader).map_err(|e| ProverError::Load(format!("read_zkey: {e}")))?;

        // Validate the r1cs + wasm load up-front so `load` fails fast on bad artifacts.
        CircomConfig::<Fr>::new(&wasm_path, &r1cs_path)
            .map_err(|e| ProverError::Load(format!("CircomConfig::new: {e}")))?;

        Ok(Self {
            build_dir,
            descriptor,
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

    /// The artifact set this prover loaded — its version key, circuit id, pins and signal layout.
    pub fn descriptor(&self) -> &'static ArtifactDescriptor {
        self.descriptor
    }

    /// The protocol version key this prover proves for (e.g. [`artifact::LEVEL_B_V1`]).
    pub fn version(&self) -> &'static str {
        self.descriptor.version
    }

    /// SHA-256 of the `verification_final.zkey` file, pinned at load (impl §11.8(f)).
    pub fn zkey_hash(&self) -> [u8; 32] {
        self.zkey_sha256
    }

    /// Lowercase hex of [`Prover::zkey_hash`].
    pub fn zkey_hash_hex(&self) -> String {
        hex::encode(self.zkey_sha256)
    }

    /// Generate a Groth16 proof for the **consent** circuit's named inputs (M7 P0).
    ///
    /// The consent circuit (`DogTagConsent(6)`) folds three depth-6 inclusion PATHS, taken as
    /// [`ConsentProveInputs`] and pushed by signal name ([`push_consent_inputs`]). The prover MUST
    /// have been loaded with the consent descriptor ([`artifact::LEVEL_B_V1_DESCRIPTOR`]); the
    /// pinned zkey/r1cs/wasm are what make the self-verify a verification against the frozen
    /// consent VK.
    pub fn prove_consent_inputs(
        &self,
        inputs: ConsentProveInputs,
    ) -> Result<Groth16Output, ProverError> {
        self.prove_with(|builder| push_consent_inputs(builder, &inputs))
    }

    /// The prove core: build the witness from `push`, prove, self-verify, format. `push`
    /// supplies the circuit's named inputs.
    fn prove_with<F>(&self, push: F) -> Result<Groth16Output, ProverError>
    where
        F: FnOnce(&mut CircomBuilder<Fr>) -> Result<(), ProverError>,
    {
        // A fresh CircomConfig per call: the wasmer witness calculator + store are
        // not cheaply shareable across threads/calls. The r1cs parse is the only
        // re-done cost here; the proving key is reused.
        let cfg = CircomConfig::<Fr>::new(&self.wasm_path, &self.r1cs_path)
            .map_err(|e| ProverError::Prove(format!("CircomConfig::new: {e}")))?;
        let mut builder = CircomBuilder::new(cfg);

        push(&mut builder)?;

        let circom = builder
            .build()
            .map_err(|e| ProverError::Prove(format!("witness build: {e}")))?;

        let public_inputs = circom
            .get_public_inputs()
            .ok_or_else(|| ProverError::Prove("circuit produced no public inputs".into()))?;

        if public_inputs.len() != self.descriptor.num_public {
            return Err(ProverError::PublicSignals {
                got: public_inputs.len(),
                expected: self.descriptor.num_public,
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

/// Verify a version-pinned artifact file against its hash, BEFORE anything parses it.
///
/// An [`artifact::ArtifactFile`] with `sha256: None` publishes no pin, so there is nothing to check
/// and the file is accepted (the zkey is not routed through here — its pin is mandatory and checked
/// inline in [`Prover::load_inner`]).
fn verify_pinned_file(path: &Path, file: &artifact::ArtifactFile) -> Result<(), ProverError> {
    let Some(expected) = file.sha256 else {
        return Ok(());
    };
    let bytes = std::fs::read(path).map_err(|e| ProverError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let got_hex = hex::encode::<[u8; 32]>(hasher.finalize().into());
    if !got_hex.eq_ignore_ascii_case(expected) {
        return Err(ProverError::ArtifactHashMismatch {
            artifact: file.rel_path.to_string(),
            expected: expected.to_ascii_lowercase(),
            got: got_hex,
        });
    }
    Ok(())
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

/// Push the **consent** circuit's named signals into the builder (M7 P0).
///
/// The consent circuit's signal set is `dogTagId, purpose,
/// relayer, recordType, deadline, consentNonce, ownerAddress, ownerSecret, Ax, Ay, R8x, R8y, S,
/// ownerSalt, keySalt, secretSalt` plus three depth-[`CONSENT_DEPTH`] inclusion PATHS (front-packed
/// `*Siblings` + `*PathLen`). Order does not matter to `CircomBuilder` (it maps by signal name).
fn push_consent_inputs(
    builder: &mut CircomBuilder<Fr>,
    inputs: &ConsentProveInputs,
) -> Result<(), ProverError> {
    let push_path =
        |b: &mut CircomBuilder<Fr>, name: &str, siblings: &[String; CONSENT_DEPTH]| -> Result<(), ProverError> {
            for v in siblings.iter() {
                b.push_input(name, parse_bigint(name, v)?);
            }
            Ok(())
        };

    push_scalar(builder, "dogTagId", &inputs.dog_tag_id)?;
    push_scalar(builder, "purpose", &inputs.purpose)?;
    push_scalar(builder, "relayer", &inputs.relayer)?;
    push_scalar(builder, "recordType", &inputs.record_type)?;
    push_scalar(builder, "deadline", &inputs.deadline)?;
    push_scalar(builder, "consentNonce", &inputs.consent_nonce)?;
    push_scalar(builder, "ownerAddress", &inputs.owner_address)?;
    push_scalar(builder, "ownerSecret", &inputs.owner_secret)?;
    push_scalar(builder, "Ax", &inputs.ax)?;
    push_scalar(builder, "Ay", &inputs.ay)?;
    push_scalar(builder, "R8x", &inputs.r8x)?;
    push_scalar(builder, "R8y", &inputs.r8y)?;
    push_scalar(builder, "S", &inputs.s)?;
    push_scalar(builder, "ownerSalt", &inputs.owner_salt)?;
    push_scalar(builder, "keySalt", &inputs.key_salt)?;
    push_scalar(builder, "secretSalt", &inputs.secret_salt)?;
    push_path(builder, "ownerSiblings", &inputs.owner_siblings)?;
    push_scalar(builder, "ownerPathLen", &inputs.owner_path_len)?;
    push_path(builder, "keySiblings", &inputs.key_siblings)?;
    push_scalar(builder, "keyPathLen", &inputs.key_path_len)?;
    push_path(builder, "secretSiblings", &inputs.secret_siblings)?;
    push_scalar(builder, "secretPathLen", &inputs.secret_path_len)?;
    Ok(())
}

impl ConsentProveInputs {
    /// Build [`ConsentProveInputs`] from the consent circuit-input JSON the SDK's
    /// `consent_assemble::consent_circuit_input_value` emits (all string-valued; the three sibling
    /// signals are arrays of length [`CONSENT_DEPTH`]). This is the seam the server `/prove-consent`
    /// route feeds and the ground-truth prove→verify test drives.
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
        fn path(v: &serde_json::Value, k: &str) -> Result<[String; CONSENT_DEPTH], ProverError> {
            let raw = v
                .get(k)
                .and_then(|x| x.as_array())
                .ok_or_else(|| ProverError::Input {
                    field: k.to_string(),
                    reason: "missing or not an array".into(),
                })?;
            if raw.len() != CONSENT_DEPTH {
                return Err(ProverError::Input {
                    field: k.to_string(),
                    reason: format!("expected {CONSENT_DEPTH} entries, got {}", raw.len()),
                });
            }
            let mut out: [String; CONSENT_DEPTH] = Default::default();
            for (i, item) in raw.iter().enumerate() {
                out[i] = item.as_str().map(|x| x.to_string()).ok_or_else(|| ProverError::Input {
                    field: format!("{k}[{i}]"),
                    reason: "not a string".into(),
                })?;
            }
            Ok(out)
        }

        Ok(ConsentProveInputs {
            dog_tag_id: s(v, "dogTagId")?,
            purpose: s(v, "purpose")?,
            relayer: s(v, "relayer")?,
            record_type: s(v, "recordType")?,
            deadline: s(v, "deadline")?,
            consent_nonce: s(v, "consentNonce")?,
            owner_address: s(v, "ownerAddress")?,
            owner_secret: s(v, "ownerSecret")?,
            ax: s(v, "Ax")?,
            ay: s(v, "Ay")?,
            r8x: s(v, "R8x")?,
            r8y: s(v, "R8y")?,
            s: s(v, "S")?,
            owner_salt: s(v, "ownerSalt")?,
            key_salt: s(v, "keySalt")?,
            secret_salt: s(v, "secretSalt")?,
            owner_siblings: path(v, "ownerSiblings")?,
            owner_path_len: s(v, "ownerPathLen")?,
            key_siblings: path(v, "keySiblings")?,
            key_path_len: s(v, "keyPathLen")?,
            secret_siblings: path(v, "secretSiblings")?,
            secret_path_len: s(v, "secretPathLen")?,
        })
    }
}
