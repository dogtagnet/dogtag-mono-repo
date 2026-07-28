//! Signed-manifest fallback for the discovery anchor (M7 §5.1 1B / §5.2 TRUST tier).
//!
//! The on-chain [`ProtocolRegistry`](../../../contracts/src/ProtocolRegistry.sol) is the ROOT OF TRUTH
//! for "what is version V". This module is its **cache/fallback**: dogtag serves the SAME version
//! content as a **dogtag-key-signed HTTPS JSON** an app can verify OFFLINE (no RPC, no server
//! liveness) with a pinned dogtag public key.
//!
//! # It is a fallback, NOT a second authority
//!
//! On ANY conflict, **on-chain wins**. [`reconcile`] enforces this: it verifies the signature, then
//! compares the manifest's mirror fields to the on-chain record; the authoritative values it returns
//! are ALWAYS the on-chain ones, and every disagreement is reported as a [`FieldConflict`]. A stale or
//! lying manifest can therefore never override the chain — at worst it is discarded and the app uses
//! the on-chain record directly.
//!
//! # DRY with the on-chain pins
//!
//! A manifest is built with [`Manifest::from_descriptor`] straight from the version's
//! [`ArtifactDescriptor`] (crate [`artifact`]), so its `circuit_id`, `public_signal_layout`, and every
//! artifact pin are the SAME values the crate already file-verifies against the committed artifacts
//! (`*_descriptor_pins_match_the_real_artifacts`). The [`VersionDeployment`] only ADDS the trio/verifier
//! addresses (the on-chain-axis half, mirrored from `contracts/deployments/roax.json` and
//! `ProtocolVersions.sol`), while the [`ArtifactRelease`] adds the artifact-axis half —
//! `artifact_base_url` + `min_app_version`.
//!
//! # On-chain VK identity vs the manifest VK hash (§3.2)
//!
//! `verifier` is the ON-CHAIN VK identity (an address). The manifest ALSO carries
//! `verification_key_sha256` — the OFF-CHAIN VK identity (the `verification_key.json` file hash), which
//! is deliberately NOT an on-chain field. The two are distinct and never conflated.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::artifact::ArtifactDescriptor;

/// `0x`-hex keccak256 of a UTF-8 string — the encoding the on-chain `ProtocolRegistry` uses for the
/// bytes32 fields it derives from strings (`versionId`, `circuitId`).
fn keccak_hex(s: &str) -> String {
    format!("0x{}", hex::encode(Keccak256::digest(s.as_bytes())))
}

/// `0x`-hex keccak256 of a version string — the on-chain `ProtocolRegistry` map key for that version.
/// MUST match `keccak256(bytes(version))` in `ProtocolVersions.sol`.
pub fn version_id(version: &str) -> String {
    keccak_hex(version)
}

/// The signature scheme tag carried in the envelope (only ed25519 is defined).
pub const MANIFEST_ALG: &str = "ed25519";

/// Domain separator prefixed to the canonical bytes before signing/verifying, so a manifest signature
/// can never be replayed as a signature over some other dogtag message with the same key.
const SIGNING_DOMAIN: &[u8] = b"dogtag-protocol-manifest/1\n";

/// The compile-pinned dogtag manifest public key an app verifies against — the REAL trust anchor for
/// offline verification. `None` here (P3): the key is provisioned at go-live and pinned into the app
/// build; until then [`verify`]/[`reconcile`] take the trusted key as a parameter and the tests
/// generate an ephemeral one. Offline verification means nothing without a pinned key, so this constant
/// is the seam that key plugs into — it is intentionally explicit rather than silently absent.
pub const DOGTAG_MANIFEST_PUBKEY_HEX: Option<&str> = None;

/// The ON-CHAIN AXIS half of a version, mirrored from `contracts/deployments/roax.json` /
/// `ProtocolVersions.sol` (`ProtocolRegistry.ContractSet`). Carries ONLY things that live on-chain.
///
/// Combined with an [`ArtifactDescriptor`] (the pins/circuit half) and an [`ArtifactRelease`] (the
/// off-chain artifact axis) this is everything the manifest needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionDeployment {
    pub chain_id: u64,
    pub factory: &'static str,
    pub verification_registry: &'static str,
    pub sbt: &'static str,
    /// The on-chain VK identity (`Groth16Verifier*` address). NOT a hash.
    pub verifier: &'static str,
}

/// The OFF-CHAIN ARTIFACT AXIS half, mirroring `ProtocolRegistry.ArtifactSet` plus the binding that
/// points a contract set at it (R-5). Kept separate from [`VersionDeployment`] for exactly the reason
/// the on-chain axes are separate: rotating the proving artifacts must not touch anything on-chain.
///
/// The pins themselves are NOT repeated here — they come from the version's [`ArtifactDescriptor`],
/// which the crate already file-verifies against the committed artifacts. This struct carries only what
/// the descriptor does not: the artifact-set IDENTITY and the two governed strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRelease {
    /// The artifact-set name, e.g. `dogtag-levelb-artifacts/1`. Its keccak is the on-chain
    /// `artifactSetId` — a DIFFERENT keyspace from the contract-set `versionId`.
    pub artifact_set: &'static str,
    /// Where the artifact bytes live (any host; integrity comes from the pins, not the host).
    pub artifact_base_url: &'static str,
    /// Minimum app build allowed to use these artifacts (semver) — the deprecation lever (§5.3 step 5).
    /// It sits on the ARTIFACT axis: the app gate is a property of the proving artifacts an app must be
    /// new enough to load, not of the deployed contracts.
    pub min_app_version: &'static str,
}

/// `dogtag-levelb/1` on-chain set — the fresh owner-hidden set (r8 redeploy, roax.json).
///
/// NOTE (redeploy landmine): these are hard-coded non-env constants; a fresh redeploy MUST
/// repoint every one or discovery silently resolves to non-existent contracts.
pub const LEVEL_B_DEPLOYMENT: VersionDeployment = VersionDeployment {
    chain_id: 135,
    factory: "0xED20269E3eBF0119739aaB5258741F3aEb49F140",
    verification_registry: "0xaBFd6f6E31780EBcB7ABd28A2a9bCfc9C8e6A77B",
    sbt: "0xBEbc45A838643D27004827b797b30A464b2b02c0",
    verifier: "0x1A9027986B859dc3879896B053deA78F636BE9b1",
};

/// The artifact set currently BOUND to `dogtag-levelb/1` (mirrors `activeArtifactSetOf`).
pub const LEVEL_B_ARTIFACT_RELEASE: ArtifactRelease = ArtifactRelease {
    artifact_set: "dogtag-levelb-artifacts/1",
    artifact_base_url: "https://artifacts.dogtag.io/levelb1",
    min_app_version: "1.4.0", // M-4 PR4 app release floor (iOS + Android)
};

/// The on-chain contract-set record for a known version key, or `None` for an unrecognized one
/// (fail-closed — the serving path returns 404, never a guessed/empty manifest).
pub fn deployment_for(version: &str) -> Option<&'static VersionDeployment> {
    match version {
        crate::artifact::LEVEL_B_V1 => Some(&LEVEL_B_DEPLOYMENT),
        _ => None,
    }
}

/// The artifact set currently bound to a known version key — the off-chain mirror of
/// `ProtocolRegistry.activeArtifactSetOf`. Rotating artifacts changes ONLY this table.
pub fn artifact_release_for(version: &str) -> Option<&'static ArtifactRelease> {
    match version {
        crate::artifact::LEVEL_B_V1 => Some(&LEVEL_B_ARTIFACT_RELEASE),
        _ => None,
    }
}

/// Build the manifest content for a known version from its file-verified descriptor, its on-chain
/// contract set, and its bound artifact set. `None` if the version is unrecognized.
pub fn build(version: &str) -> Option<Manifest> {
    let desc = crate::artifact::resolve(Some(version)).ok()?;
    let deploy = deployment_for(version)?;
    let release = artifact_release_for(version)?;
    Some(Manifest::from_descriptor(desc, deploy, release))
}

/// The manifest CONTENT for one version (§5.2 TRUST tier). Mirrors BOTH on-chain axes — the
/// `ContractSet` and the `ArtifactSet` its binding points at — PLUS the off-chain VK identity + artifact
/// URLs. Serialized deterministically (struct field order) for signing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    /// keccak256(version) `0x`-hex — the on-chain `contractSetId`. Lets a verifier cross-check against
    /// the on-chain key.
    pub version_id: String,
    /// The artifact set currently bound to `version`, e.g. `dogtag-levelb-artifacts/1` (R-5). This is
    /// the OFF-CHAIN axis identity; a zkey rotation changes this (and the pins below) while `version` /
    /// `version_id` and every address field stay put.
    pub artifact_set: String,
    /// keccak256(artifact_set) `0x`-hex — the on-chain `artifactSetId`.
    pub artifact_set_id: String,
    pub chain_id: u64,
    pub factory: String,
    pub verification_registry: String,
    pub sbt: String,
    /// On-chain VK identity (address). NOT a hash.
    pub verifier: String,
    pub circuit_id: String,
    pub num_public: usize,
    pub public_signal_layout: Vec<String>,
    /// FETCH pin — SHA-256 of the zkey file (mandatory).
    pub zkey_sha256: String,
    /// FETCH pin — SHA-256 of the `.graph` (`None` == unpinned: not committed, §3.5).
    pub witness_mobile_sha256: Option<String>,
    /// FETCH pin — SHA-256 of the `.r1cs`.
    pub witness_server_r1cs_sha256: Option<String>,
    /// FETCH pin — SHA-256 of the `.wasm`.
    pub witness_server_wasm_sha256: Option<String>,
    /// OFF-CHAIN VK identity — SHA-256 of `verification_key.json` (NOT an on-chain field, §3.2).
    pub verification_key_sha256: Option<String>,
    pub artifact_base_url: String,
    pub min_app_version: String,
}

impl Manifest {
    /// Build the manifest content from the version's artifact descriptor (the pins/circuit half), its
    /// on-chain contract set (`deploy`), and its bound artifact set (`release`). Every pin/circuit field
    /// comes from `desc`, so it stays in lock-step with the crate's file-verified descriptors.
    ///
    /// Note the two axes enter through two separate parameters — the same split the on-chain registry
    /// makes (R-5), so a caller physically cannot rotate one by editing the other.
    pub fn from_descriptor(
        desc: &ArtifactDescriptor,
        deploy: &VersionDeployment,
        release: &ArtifactRelease,
    ) -> Self {
        Manifest {
            version: desc.version.to_string(),
            version_id: version_id(desc.version),
            artifact_set: release.artifact_set.to_string(),
            artifact_set_id: version_id(release.artifact_set),
            chain_id: deploy.chain_id,
            factory: deploy.factory.to_string(),
            verification_registry: deploy.verification_registry.to_string(),
            sbt: deploy.sbt.to_string(),
            verifier: deploy.verifier.to_string(),
            circuit_id: desc.circuit_id.to_string(),
            num_public: desc.num_public,
            public_signal_layout: desc.public_signal_layout.iter().map(|s| s.to_string()).collect(),
            zkey_sha256: desc.zkey.sha256.to_string(),
            witness_mobile_sha256: desc.witness_graph.sha256.map(str::to_string),
            witness_server_r1cs_sha256: desc.r1cs.sha256.map(str::to_string),
            witness_server_wasm_sha256: desc.wasm.sha256.map(str::to_string),
            verification_key_sha256: desc.vk.verification_key_json.sha256.map(str::to_string),
            artifact_base_url: release.artifact_base_url.to_string(),
            min_app_version: release.min_app_version.to_string(),
        }
    }

    /// The exact bytes signed/verified: a domain tag followed by the deterministic JSON encoding of
    /// this content. Struct field order is stable, so two builds of the same content sign identically.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let json = serde_json::to_vec(self).expect("Manifest serializes");
        let mut buf = Vec::with_capacity(SIGNING_DOMAIN.len() + json.len());
        buf.extend_from_slice(SIGNING_DOMAIN);
        buf.extend_from_slice(&json);
        buf
    }
}

/// The wire envelope: content + detached ed25519 signature + the signing pubkey (for transport only —
/// [`verify`] checks against the caller's PINNED key, never blindly against this field).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedManifest {
    pub content: Manifest,
    pub alg: String,
    /// Hex (64 bytes) detached ed25519 signature over [`Manifest::canonical_bytes`].
    pub signature: String,
    /// Hex (32 bytes) ed25519 public key that produced `signature` (advisory; verify pins its own key).
    pub public_key: String,
}

/// A single field on which a signed manifest disagrees with the on-chain record. On-chain wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldConflict {
    pub field: &'static str,
    pub onchain: String,
    pub manifest: String,
}

/// The authoritative ON-CHAIN AXIS fields a manifest is reconciled against (read from
/// `ProtocolRegistry.getContractSet`). On any disagreement THESE win.
///
/// It mirrors EVERY member of the on-chain `ContractSet` struct that the signed manifest also carries —
/// the trio + verifier + `circuitId` — PLUS the `active` lifecycle bit, which the manifest does NOT
/// carry and which is therefore a pass-through rather than a cross-checked field (see
/// [`OnchainContractSet::active`]). The only on-chain member omitted is `publishedAt`.
///
/// `circuit_id` holds the on-chain `bytes32` value, i.e. `keccak256(circuit-string)` as `0x`-hex;
/// [`reconcile`] hashes the manifest's plain circuit string before comparing (§3.2 — this is distinct
/// from the verifier VK identity and is NOT one of the fetch pins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnchainContractSet {
    /// The on-chain `contractSetId` — `keccak256(version)` as `0x`-hex.
    pub version_id: String,
    pub factory: String,
    pub verification_registry: String,
    pub sbt: String,
    pub verifier: String,
    /// `keccak256(circuit-string)` as `0x`-hex — the authoritative on-chain `circuitId` bytes32.
    pub circuit_id: String,
    /// The on-chain `ContractSet.active` lifecycle bit — `deprecateContractSet` flips it false and the
    /// record is never deleted. It is attested ONLY on-chain (the signed manifest deliberately carries
    /// no lifecycle state), so [`reconcile`] never compares it; it simply rides through into
    /// [`Reconciliation::contract_set`], which is what lets a consumer enforce the anti-downgrade check
    /// against real chain state rather than a caller-supplied guess.
    pub active: bool,
}

/// The authoritative OFF-CHAIN ARTIFACT AXIS fields (read from
/// `ProtocolRegistry.getActiveArtifactSet(contractSetId)` — i.e. the artifact set the binding currently
/// points at). Mirrors every `ArtifactSet` member the manifest also carries, plus its own independent
/// `active` bit.
///
/// This is a SEPARATE struct from [`OnchainContractSet`] for the same reason the on-chain mappings are
/// separate (R-5): a zkey rotation produces a new value here and leaves the contract set alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnchainArtifactSet {
    /// The on-chain `artifactSetId` — `keccak256(artifact_set)` as `0x`-hex.
    pub artifact_set_id: String,
    pub zkey_sha256: String,
    pub witness_mobile_sha256: Option<String>,
    pub witness_server_r1cs_sha256: Option<String>,
    pub witness_server_wasm_sha256: Option<String>,
    pub artifact_base_url: String,
    pub min_app_version: String,
    /// The on-chain `ArtifactSet.active` bit — independent of the contract set's. Like its counterpart
    /// it is never compared, only carried through.
    pub active: bool,
}

/// The result of [`reconcile`]: the AUTHORITATIVE (always on-chain) fields of BOTH axes, the manifest's
/// signed extras, and any conflicts found. `conflicts.is_empty()` ⇒ the manifest agrees and its extras
/// (layout, VK hash) are safe to use; otherwise on-chain governs and the extras are suspect.
#[derive(Debug, Clone)]
pub struct Reconciliation {
    /// The authoritative on-chain axis.
    pub contract_set: OnchainContractSet,
    /// The authoritative artifact axis (whatever the binding currently points at).
    pub artifact_set: OnchainArtifactSet,
    pub manifest: Manifest,
    pub conflicts: Vec<FieldConflict>,
}

impl Reconciliation {
    /// True iff the signed manifest agrees with on-chain on every shared field.
    pub fn manifest_agrees(&self) -> bool {
        self.conflicts.is_empty()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("manifest alg '{0}' is not the expected '{MANIFEST_ALG}'")]
    UnsupportedAlg(String),
    #[error("signature hex is not a 64-byte ed25519 signature")]
    BadSignatureEncoding,
    #[error("public-key hex is not a 32-byte ed25519 key")]
    BadPublicKeyEncoding,
    #[error("manifest is signed by {got}, not the pinned dogtag key {expected}")]
    WrongSigner { expected: String, got: String },
    #[error("signature does not verify against the pinned dogtag key (tampered or wrong key)")]
    BadSignature,
}

/// Sign a manifest with the dogtag signing key. Server-side (`vet-api` holds the key).
pub fn sign(content: &Manifest, key: &SigningKey) -> SignedManifest {
    let sig: Signature = key.sign(&content.canonical_bytes());
    SignedManifest {
        content: content.clone(),
        alg: MANIFEST_ALG.to_string(),
        signature: hex::encode(sig.to_bytes()),
        public_key: hex::encode(key.verifying_key().to_bytes()),
    }
}

/// THE offline verify helper: verify a signed manifest against the PINNED dogtag public key.
///
/// Fail-closed: rejects a wrong alg, malformed encodings, a signature by a key other than `pinned`, or
/// a signature that does not verify (tampered content). Verifying against the *pinned* key — not the
/// envelope's advertised `public_key` — is what makes the transported key untrusted: an attacker who
/// re-signs tampered content with their OWN key still fails, because their key is not the pinned one.
pub fn verify(signed: &SignedManifest, pinned: &VerifyingKey) -> Result<(), ManifestError> {
    if signed.alg != MANIFEST_ALG {
        return Err(ManifestError::UnsupportedAlg(signed.alg.clone()));
    }
    let sig_bytes: [u8; 64] = hex::decode(&signed.signature)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(ManifestError::BadSignatureEncoding)?;
    let signature = Signature::from_bytes(&sig_bytes);

    // The advertised key must be the pinned one — reject a manifest signed by any other key up front
    // (a clearer failure than a generic bad-signature, and it closes the "valid sig, wrong signer" gap).
    let advertised: [u8; 32] = hex::decode(&signed.public_key)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(ManifestError::BadPublicKeyEncoding)?;
    if advertised != pinned.to_bytes() {
        return Err(ManifestError::WrongSigner {
            expected: hex::encode(pinned.to_bytes()),
            got: hex::encode(advertised),
        });
    }

    pinned
        .verify(&signed.content.canonical_bytes(), &signature)
        .map_err(|_| ManifestError::BadSignature)
}

/// Verify a manifest AND reconcile it against the on-chain record, enforcing on-chain precedence.
///
/// Verification runs first (a bad signature is rejected before any field is trusted). Then EVERY
/// on-chain field the manifest mirrors is compared, ON BOTH AXES — the contract set's trio + verifier +
/// `circuit_id`, and the artifact set's `artifact_set_id`, every fetch-pin, `artifact_base_url` and
/// `min_app_version` (the on-chain lifecycle state is out of scope: `publishedAt` is not mirrored at
/// all, and each axis's `active` is carried through UNCOMPARED because the manifest does not attest it).
/// The returned [`Reconciliation::contract_set`] / [`Reconciliation::artifact_set`] are ALWAYS the
/// on-chain values, and each disagreement is recorded.
///
/// The two axes are passed SEPARATELY because they are read separately on-chain
/// (`getContractSet` + `getActiveArtifactSet`), and because a caller must be able to reconcile a
/// manifest against a freshly-rotated artifact set without having re-read the contract set (R-5).
///
/// This is how "on-chain wins on conflict" is enforced in code: the caller reads authoritative fields
/// from the two on-chain structs and only trusts the manifest's signed extras when
/// [`Reconciliation::manifest_agrees`] holds — so a stale/compromised manifest can never slip a
/// differing `min_app_version` (the deprecation lever) or `circuit_id` past the check.
pub fn reconcile(
    signed: &SignedManifest,
    pinned: &VerifyingKey,
    contract_set: &OnchainContractSet,
    artifact_set: &OnchainArtifactSet,
) -> Result<Reconciliation, ManifestError> {
    verify(signed, pinned)?;

    let m = &signed.content;
    let mut conflicts = Vec::new();
    let mut cmp = |field: &'static str, oc: &str, mf: &str| {
        // Address/hex comparisons are case-insensitive (checksum vs lowercase must not read as a
        // conflict); the pins are lowercase hex on both sides but this is harmless for them too.
        if !oc.eq_ignore_ascii_case(mf) {
            conflicts.push(FieldConflict {
                field,
                onchain: oc.to_string(),
                manifest: mf.to_string(),
            });
        }
    };
    // --- the ON-CHAIN axis ---
    cmp("version_id", &contract_set.version_id, &m.version_id);
    cmp("factory", &contract_set.factory, &m.factory);
    cmp("verification_registry", &contract_set.verification_registry, &m.verification_registry);
    cmp("sbt", &contract_set.sbt, &m.sbt);
    cmp("verifier", &contract_set.verifier, &m.verifier);
    // On-chain `circuitId` is a bytes32 (`keccak256(circuit-string)`) while the manifest carries the
    // plain circuit string, so hash the manifest's before the (case-insensitive hex) compare.
    cmp("circuit_id", &contract_set.circuit_id, &keccak_hex(&m.circuit_id));

    // --- the ARTIFACT axis ---
    // `artifact_set_id` is the axis's own identity: comparing it is what stops a manifest describing
    // artifact set N from being accepted while the chain has rotated the binding to N+1.
    cmp("artifact_set_id", &artifact_set.artifact_set_id, &m.artifact_set_id);
    cmp("zkey_sha256", &artifact_set.zkey_sha256, &m.zkey_sha256);
    cmp_opt(&mut conflicts, "witness_mobile_sha256", &artifact_set.witness_mobile_sha256, &m.witness_mobile_sha256);
    cmp_opt(&mut conflicts, "witness_server_r1cs_sha256", &artifact_set.witness_server_r1cs_sha256, &m.witness_server_r1cs_sha256);
    cmp_opt(&mut conflicts, "witness_server_wasm_sha256", &artifact_set.witness_server_wasm_sha256, &m.witness_server_wasm_sha256);
    // The two string extras are compared EXACTLY (not case-folded): a URL path and a semver are
    // case-sensitive, so `eq_ignore_ascii_case` could mask a real disagreement.
    cmp_str(&mut conflicts, "artifact_base_url", &artifact_set.artifact_base_url, &m.artifact_base_url);
    cmp_str(&mut conflicts, "min_app_version", &artifact_set.min_app_version, &m.min_app_version);

    Ok(Reconciliation {
        contract_set: contract_set.clone(),
        artifact_set: artifact_set.clone(),
        manifest: signed.content.clone(),
        conflicts,
    })
}

/// Exact (case-sensitive) string comparison — for the on-chain string members (`artifactBaseUrl`,
/// `minAppVersion`) where case is significant, unlike the case-folded hex/address fields.
fn cmp_str(conflicts: &mut Vec<FieldConflict>, field: &'static str, oc: &str, mf: &str) {
    if oc != mf {
        conflicts.push(FieldConflict {
            field,
            onchain: oc.to_string(),
            manifest: mf.to_string(),
        });
    }
}

fn cmp_opt(
    conflicts: &mut Vec<FieldConflict>,
    field: &'static str,
    oc: &Option<String>,
    mf: &Option<String>,
) {
    let same = match (oc, mf) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    };
    if !same {
        conflicts.push(FieldConflict {
            field,
            onchain: oc.clone().unwrap_or_else(|| "<unpinned>".into()),
            manifest: mf.clone().unwrap_or_else(|| "<unpinned>".into()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact;

    fn test_key() -> SigningKey {
        // Deterministic key for tests (Date/rand are avoided so the round-trip is reproducible).
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn levelb_manifest() -> Manifest {
        Manifest::from_descriptor(
            artifact::resolve(Some(artifact::LEVEL_B_V1)).unwrap(),
            &LEVEL_B_DEPLOYMENT,
            &LEVEL_B_ARTIFACT_RELEASE,
        )
    }

    /// The on-chain AXIS-1 record an agreeing manifest reconciles clean against.
    fn contracts_from(m: &Manifest) -> OnchainContractSet {
        OnchainContractSet {
            version_id: m.version_id.clone(),
            factory: m.factory.clone(),
            verification_registry: m.verification_registry.clone(),
            sbt: m.sbt.clone(),
            verifier: m.verifier.clone(),
            // On-chain the circuitId is `keccak256(circuit-string)`, mirrored here from the manifest's
            // plain circuit string so an agreeing manifest reconciles clean.
            circuit_id: keccak_hex(&m.circuit_id),
            active: true,
        }
    }

    /// The on-chain AXIS-2 record (what the binding points at) for an agreeing manifest.
    fn artifacts_from(m: &Manifest) -> OnchainArtifactSet {
        OnchainArtifactSet {
            artifact_set_id: m.artifact_set_id.clone(),
            zkey_sha256: m.zkey_sha256.clone(),
            witness_mobile_sha256: m.witness_mobile_sha256.clone(),
            witness_server_r1cs_sha256: m.witness_server_r1cs_sha256.clone(),
            witness_server_wasm_sha256: m.witness_server_wasm_sha256.clone(),
            artifact_base_url: m.artifact_base_url.clone(),
            min_app_version: m.min_app_version.clone(),
            active: true,
        }
    }

    /// Leg 1: sign → serialize → deserialize → offline-verify PASSES.
    #[test]
    fn round_trip_sign_serve_verify() {
        let key = test_key();
        let signed = sign(&levelb_manifest(), &key);

        // "serve": cross the wire as JSON, then parse back (the app never sees the in-memory struct).
        let wire = serde_json::to_string(&signed).unwrap();
        let received: SignedManifest = serde_json::from_str(&wire).unwrap();

        verify(&received, &key.verifying_key()).expect("a well-formed manifest verifies offline");
    }

    /// Leg 2: a TAMPERED manifest fails (the signature is over the original content).
    #[test]
    fn tampered_manifest_fails() {
        let key = test_key();
        let mut signed = sign(&levelb_manifest(), &key);
        // Flip a single field after signing.
        signed.content.verifier = "0x000000000000000000000000000000000000dead".to_string();

        assert_eq!(verify(&signed, &key.verifying_key()), Err(ManifestError::BadSignature));
    }

    /// Leg 2b: a manifest re-signed with the WRONG key fails against the pinned key.
    #[test]
    fn wrong_signer_fails() {
        let attacker = SigningKey::from_bytes(&[9u8; 32]);
        let mut signed = sign(&levelb_manifest(), &attacker); // attacker re-signs their own content
        signed.content.min_app_version = "9.9.9".to_string();
        let resigned = sign(&signed.content, &attacker);

        let pinned = test_key().verifying_key();
        match verify(&resigned, &pinned) {
            Err(ManifestError::WrongSigner { .. }) => {}
            other => panic!("expected WrongSigner, got {other:?}"),
        }
    }

    /// Leg 3: on manifest-vs-on-chain CONFLICT, on-chain wins — the conflict is reported and the
    /// authoritative value is the on-chain one, never the manifest's.
    #[test]
    fn conflict_resolves_to_onchain() {
        let key = test_key();
        let signed = sign(&levelb_manifest(), &key);

        // The chain says the verifier is a DIFFERENT address than the (validly signed) manifest claims.
        let mut contracts = contracts_from(&signed.content);
        let true_verifier = "0x1111111111111111111111111111111111111111".to_string();
        contracts.verifier = true_verifier.clone();

        let r = reconcile(&signed, &key.verifying_key(), &contracts, &artifacts_from(&signed.content)).unwrap();
        assert!(!r.manifest_agrees(), "a differing verifier is a conflict");
        assert_eq!(r.conflicts.len(), 1);
        assert_eq!(r.conflicts[0].field, "verifier");
        // The authoritative value the caller must use is the CHAIN's, not the manifest's.
        assert_eq!(r.contract_set.verifier, true_verifier);
        assert_ne!(r.contract_set.verifier, signed.content.verifier);
    }

    /// The precedence invariant covers `circuit_id` + the string extras too: a validly-signed manifest
    /// whose `circuit_id`/`min_app_version`/`artifact_base_url` disagree with on-chain is reported as a
    /// conflict and the authoritative values stay the on-chain ones (not the manifest's). Without these
    /// three in the comparison a stale/compromised manifest could slip a differing `min_app_version` (the
    /// deprecation lever) past `manifest_agrees()`.
    #[test]
    fn differing_circuit_id_and_string_extras_are_conflicts() {
        let key = test_key();
        let signed = sign(&levelb_manifest(), &key);

        // Start from agreeing on-chain records, then diverge exactly the three newly-covered fields —
        // one on the contract axis, two on the artifact axis.
        let mut contracts = contracts_from(&signed.content);
        let mut artifacts = artifacts_from(&signed.content);
        let true_circuit = keccak_hex("verification.circom/DogTagVerification(24,5)");
        contracts.circuit_id = true_circuit.clone();
        artifacts.min_app_version = "9.9.9".to_string();
        artifacts.artifact_base_url = "https://true.dogtag.io/levelb1".to_string();

        let r = reconcile(&signed, &key.verifying_key(), &contracts, &artifacts).unwrap();
        assert!(!r.manifest_agrees(), "differing circuit_id/min_app_version/artifact_base_url conflict");
        let fields: Vec<&str> = r.conflicts.iter().map(|c| c.field).collect();
        assert!(fields.contains(&"circuit_id"), "circuit_id conflict reported, got {fields:?}");
        assert!(fields.contains(&"min_app_version"), "min_app_version conflict reported, got {fields:?}");
        assert!(fields.contains(&"artifact_base_url"), "artifact_base_url conflict reported, got {fields:?}");
        // On-chain wins: the authoritative values are the chain's, not the (validly signed) manifest's.
        assert_eq!(r.contract_set.circuit_id, true_circuit);
        assert_ne!(r.contract_set.circuit_id, keccak_hex(&signed.content.circuit_id));
        assert_eq!(r.artifact_set.min_app_version, "9.9.9");
        assert_ne!(r.artifact_set.min_app_version, signed.content.min_app_version);
    }

    /// R-5, in the manifest layer: reconciling against a chain whose ARTIFACT binding has rotated
    /// reports only artifact-axis conflicts — every contract-axis field still agrees. A manifest for the
    /// superseded artifact set is rejected (`manifest_agrees() == false`) WITHOUT the contract set ever
    /// being implicated, which is what proves the two axes are independently reconciled.
    #[test]
    fn an_artifact_rotation_conflicts_only_on_the_artifact_axis() {
        let key = test_key();
        let signed = sign(&levelb_manifest(), &key); // describes …-artifacts/1
        let contracts = contracts_from(&signed.content);

        // The chain has rotated the binding to a new artifact set; the contract set is untouched.
        let mut rotated = artifacts_from(&signed.content);
        rotated.artifact_set_id = version_id("dogtag-levelb-artifacts/2");
        rotated.zkey_sha256 = "0xfeed".to_string();
        rotated.min_app_version = "2.0.0".to_string();

        let r = reconcile(&signed, &key.verifying_key(), &contracts, &rotated).unwrap();
        assert!(!r.manifest_agrees(), "a stale artifact manifest must not pass");
        let fields: Vec<&str> = r.conflicts.iter().map(|c| c.field).collect();
        assert!(fields.contains(&"artifact_set_id"), "got {fields:?}");
        assert!(fields.contains(&"zkey_sha256"), "got {fields:?}");
        assert!(fields.contains(&"min_app_version"), "got {fields:?}");
        // Not one contract-axis field is implicated by an artifact rotation.
        for onchain_only in ["version_id", "factory", "verification_registry", "sbt", "verifier", "circuit_id"] {
            assert!(!fields.contains(&onchain_only), "{onchain_only} must not conflict, got {fields:?}");
        }
    }

    /// The mirror image: a TRIO rotation on-chain conflicts only on the contract axis. Together with the
    /// test above this is the Rust-side statement of the R-5 invariant.
    #[test]
    fn a_trio_rotation_conflicts_only_on_the_contract_axis() {
        let key = test_key();
        let signed = sign(&levelb_manifest(), &key);
        let artifacts = artifacts_from(&signed.content);

        let mut rotated = contracts_from(&signed.content);
        rotated.factory = "0x00000000000000000000000000000000000f00d0".to_string();
        rotated.verification_registry = "0x00000000000000000000000000000000dead0000".to_string();
        rotated.sbt = "0x0000000000000000000000000000000000beef00".to_string();

        let r = reconcile(&signed, &key.verifying_key(), &rotated, &artifacts).unwrap();
        let fields: Vec<&str> = r.conflicts.iter().map(|c| c.field).collect();
        assert!(fields.contains(&"factory") && fields.contains(&"verification_registry") && fields.contains(&"sbt"));
        for artifact_only in [
            "artifact_set_id",
            "zkey_sha256",
            "witness_server_r1cs_sha256",
            "witness_server_wasm_sha256",
            "artifact_base_url",
            "min_app_version",
        ] {
            assert!(!fields.contains(&artifact_only), "{artifact_only} must not conflict, got {fields:?}");
        }
    }

    /// The artifact axis has its own identity and its own keyspace: `artifact_set_id` is the keccak of
    /// the artifact-set NAME, never of the version, so the two ids can never be confused.
    #[test]
    fn the_two_axis_ids_are_distinct() {
        let m = levelb_manifest();
        assert_eq!(m.artifact_set, "dogtag-levelb-artifacts/1");
        assert_eq!(m.artifact_set_id, version_id("dogtag-levelb-artifacts/1"));
        assert_ne!(m.artifact_set_id, m.version_id, "the axes must not share an id");
    }

    /// A verified manifest that AGREES with on-chain reconciles cleanly (no conflicts).
    #[test]
    fn agreeing_manifest_reconciles_clean() {
        let key = test_key();
        let signed = sign(&levelb_manifest(), &key);

        let r = reconcile(
            &signed,
            &key.verifying_key(),
            &contracts_from(&signed.content),
            &artifacts_from(&signed.content),
        )
        .unwrap();
        assert!(r.manifest_agrees());
        assert!(r.conflicts.is_empty());
    }

    /// The manifest's `version_id` is `keccak256(version)` — the EXACT on-chain `ProtocolRegistry` key
    /// (`keccak256(bytes("dogtag-level{a,b}/1"))` in `ProtocolVersions.sol`, `cast keccak`-confirmed).
    /// This is the cross-boundary tie that lets an app match a manifest to an on-chain version.
    #[test]
    fn version_id_matches_onchain_keccak() {
        assert_eq!(
            version_id("dogtag-levela/1"),
            "0x6e9e08e062f74ab771e3140071eee36d4e0556158e0d99c36f294e36be18bf70"
        );
        assert_eq!(
            version_id("dogtag-levelb/1"),
            "0x36a8d69d16a9f540fa11be5f0311ebd5efd8e971b66cd704a6e197ee15b01b3d"
        );
        assert_eq!(levelb_manifest().version_id, version_id("dogtag-levelb/1"));
    }

    /// The manifest is built straight from the file-verified descriptor: its pins ARE the descriptor's.
    #[test]
    fn manifest_pins_come_from_the_descriptor() {
        let d = artifact::resolve(Some(artifact::LEVEL_B_V1)).unwrap();
        let m = levelb_manifest();
        assert_eq!(m.zkey_sha256, d.zkey.sha256);
        assert_eq!(m.witness_server_r1cs_sha256.as_deref(), d.r1cs.sha256);
        assert_eq!(m.witness_server_wasm_sha256.as_deref(), d.wasm.sha256);
        assert_eq!(m.circuit_id, d.circuit_id);
        // The graph is PINNED on both sides (ROAX 2026-07-28). Asserted as the exact attested hash
        // rather than merely `is_some()`: this is the only check keeping the descriptor and the
        // on-chain `witnessMobileSha256` in lockstep, and `reconcile` reports `(Some, None)` as a
        // conflict, so a test that tolerated either state would retire that guarantee.
        assert_eq!(
            m.witness_mobile_sha256.as_deref(),
            Some(artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256)
        );
        // The OFF-CHAIN VK identity is present and is NOT the zkey pin (§3.2).
        assert_eq!(m.verification_key_sha256.as_deref(), d.vk.verification_key_json.sha256);
        assert_ne!(m.verification_key_sha256, Some(m.zkey_sha256.clone()));
    }
}
