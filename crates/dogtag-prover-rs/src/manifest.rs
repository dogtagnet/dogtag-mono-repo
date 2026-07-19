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
//! addresses + `artifact_base_url` + `min_app_version` (the on-chain-address half, mirrored from
//! `contracts/deployments/roax.json` and `ProtocolVersions.sol`).
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

/// The on-chain-address half of a version, mirrored from `contracts/deployments/roax.json` /
/// `ProtocolVersions.sol`. Combined with an [`ArtifactDescriptor`] (the pins/circuit half) this is
/// everything the manifest needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionDeployment {
    pub chain_id: u64,
    pub factory: &'static str,
    pub verification_registry: &'static str,
    pub sbt: &'static str,
    /// The on-chain VK identity (`Groth16Verifier*` address). NOT a hash.
    pub verifier: &'static str,
    /// Where the artifact bytes live (any host; integrity comes from the pins, not the host).
    pub artifact_base_url: &'static str,
    /// Minimum app build allowed to use this version (semver) — the deprecation lever (§5.3 step 5).
    pub min_app_version: &'static str,
}

/// `dogtag-levela/1` deployment — the still-live Level-A verification trio (roax.json).
pub const LEVEL_A_DEPLOYMENT: VersionDeployment = VersionDeployment {
    chain_id: 135,
    factory: "0xd3179AbBfb0274D0a5F7017d76015A93C159511D",
    verification_registry: "0x4E2f0996e1CB4E24F1053346f3da2186906835E8",
    sbt: "0x1FB8986573Ac36d532cF7d5a5352202B094D4233",
    verifier: "0xEEFCfAF026931b7325472A88fd14Ee780Da13559",
    artifact_base_url: "https://artifacts.dogtag.io/levela1",
    min_app_version: "1.0.0",
};

/// `dogtag-levelb/1` deployment — the canonical M5 Level-B consent trio (roax.json).
pub const LEVEL_B_DEPLOYMENT: VersionDeployment = VersionDeployment {
    chain_id: 135,
    factory: "0xd3179AbBfb0274D0a5F7017d76015A93C159511D",
    verification_registry: "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87",
    sbt: "0x96Cba4580D79bc9b8e51Fc1B3a044A29592AfFFc",
    verifier: "0x272be146C0aEd6401000E9Aa8241201F6f0fdF1a",
    artifact_base_url: "https://artifacts.dogtag.io/levelb1",
    min_app_version: "1.4.0",
};

/// The deployment record for a known version key, or `None` for an unrecognized one (fail-closed —
/// the serving path returns 404, never a guessed/empty manifest).
pub fn deployment_for(version: &str) -> Option<&'static VersionDeployment> {
    match version {
        crate::artifact::LEVEL_A_V1 => Some(&LEVEL_A_DEPLOYMENT),
        crate::artifact::LEVEL_B_V1 => Some(&LEVEL_B_DEPLOYMENT),
        _ => None,
    }
}

/// Build the manifest content for a known version from its file-verified descriptor + deployment.
/// `None` if the version is unrecognized.
pub fn build(version: &str) -> Option<Manifest> {
    let desc = crate::artifact::resolve(Some(version)).ok()?;
    let deploy = deployment_for(version)?;
    Some(Manifest::from_descriptor(desc, deploy))
}

/// The manifest CONTENT for one version (§5.2 TRUST tier). Mirrors the on-chain `Version` PLUS the
/// off-chain VK identity + artifact URLs. Serialized deterministically (struct field order) for signing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    /// keccak256(version) `0x`-hex — lets a verifier cross-check against the on-chain key.
    pub version_id: String,
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
    /// Build the manifest content from the version's artifact descriptor (the pins/circuit half) and
    /// its deployment (the address half). Every pin/circuit field comes from `desc`, so it stays in
    /// lock-step with the crate's file-verified descriptors.
    pub fn from_descriptor(desc: &ArtifactDescriptor, deploy: &VersionDeployment) -> Self {
        Manifest {
            version: desc.version.to_string(),
            version_id: version_id(desc.version),
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
            artifact_base_url: deploy.artifact_base_url.to_string(),
            min_app_version: deploy.min_app_version.to_string(),
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

/// The authoritative on-chain fields a manifest is reconciled against (read from
/// `ProtocolRegistry.getVersion`). On any disagreement THESE win.
///
/// It mirrors EVERY member of the on-chain `Version` struct that the signed manifest also carries, so
/// [`reconcile`] can cross-check all of them — the trio + verifier, the artifact fetch-pins, `circuitId`,
/// `artifactBaseUrl` and `minAppVersion` — PLUS the `active` lifecycle bit, which the manifest does NOT
/// carry and which is therefore a pass-through rather than a cross-checked field (see [`OnchainVersion::active`]).
/// The only on-chain member omitted is `publishedAt`.
///
/// `circuit_id` holds the on-chain `bytes32` value, i.e. `keccak256(circuit-string)` as `0x`-hex;
/// [`reconcile`] hashes the manifest's plain circuit string before comparing (§3.2 — this is distinct
/// from the verifier VK identity and is NOT one of the fetch pins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnchainVersion {
    pub version_id: String,
    pub factory: String,
    pub verification_registry: String,
    pub sbt: String,
    pub verifier: String,
    /// `keccak256(circuit-string)` as `0x`-hex — the authoritative on-chain `circuitId` bytes32.
    pub circuit_id: String,
    pub zkey_sha256: String,
    pub witness_mobile_sha256: Option<String>,
    pub witness_server_r1cs_sha256: Option<String>,
    pub witness_server_wasm_sha256: Option<String>,
    pub artifact_base_url: String,
    pub min_app_version: String,
    /// The on-chain `Version.active` lifecycle bit — `deprecateVersion` flips it false and the record is
    /// never deleted. It is attested ONLY on-chain (the signed manifest deliberately carries no lifecycle
    /// state), so [`reconcile`] never compares it; it simply rides through into
    /// [`Reconciliation::authoritative`], which is what lets a consumer enforce the anti-downgrade check
    /// against real chain state rather than a caller-supplied guess.
    pub active: bool,
}

/// The result of [`reconcile`]: the AUTHORITATIVE (always on-chain) shared fields, the manifest's
/// signed extras, and any conflicts found. `conflicts.is_empty()` ⇒ the manifest agrees and its extras
/// (URLs, layout, VK hash) are safe to use; otherwise on-chain governs and the extras are suspect.
#[derive(Debug, Clone)]
pub struct Reconciliation {
    pub authoritative: OnchainVersion,
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
/// on-chain field the manifest mirrors is compared — the trio + verifier, every artifact fetch-pin,
/// `circuit_id`, `artifact_base_url` and `min_app_version` (the on-chain lifecycle state is out of scope:
/// `publishedAt` is not mirrored at all, and `active` is carried through `authoritative` UNCOMPARED
/// because the manifest does not attest it). The returned
/// [`Reconciliation::authoritative`] is ALWAYS the on-chain value, and each disagreement is recorded.
/// This is how "on-chain wins on conflict" is enforced in code: the caller reads authoritative fields
/// from `authoritative` (on-chain) and only trusts the manifest's signed extras when
/// [`Reconciliation::manifest_agrees`] holds — so a stale/compromised manifest can never slip a
/// differing `min_app_version` (the deprecation lever) or `circuit_id` past the check.
pub fn reconcile(
    signed: &SignedManifest,
    pinned: &VerifyingKey,
    onchain: &OnchainVersion,
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
    cmp("version_id", &onchain.version_id, &m.version_id);
    cmp("factory", &onchain.factory, &m.factory);
    cmp("verification_registry", &onchain.verification_registry, &m.verification_registry);
    cmp("sbt", &onchain.sbt, &m.sbt);
    cmp("verifier", &onchain.verifier, &m.verifier);
    // On-chain `circuitId` is a bytes32 (`keccak256(circuit-string)`) while the manifest carries the
    // plain circuit string, so hash the manifest's before the (case-insensitive hex) compare.
    cmp("circuit_id", &onchain.circuit_id, &keccak_hex(&m.circuit_id));
    cmp("zkey_sha256", &onchain.zkey_sha256, &m.zkey_sha256);
    cmp_opt(&mut conflicts, "witness_mobile_sha256", &onchain.witness_mobile_sha256, &m.witness_mobile_sha256);
    cmp_opt(&mut conflicts, "witness_server_r1cs_sha256", &onchain.witness_server_r1cs_sha256, &m.witness_server_r1cs_sha256);
    cmp_opt(&mut conflicts, "witness_server_wasm_sha256", &onchain.witness_server_wasm_sha256, &m.witness_server_wasm_sha256);
    // The two string extras are compared EXACTLY (not case-folded): a URL path and a semver are
    // case-sensitive, so `eq_ignore_ascii_case` could mask a real disagreement.
    cmp_str(&mut conflicts, "artifact_base_url", &onchain.artifact_base_url, &m.artifact_base_url);
    cmp_str(&mut conflicts, "min_app_version", &onchain.min_app_version, &m.min_app_version);

    Ok(Reconciliation {
        authoritative: onchain.clone(),
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
        )
    }

    fn onchain_from(m: &Manifest) -> OnchainVersion {
        OnchainVersion {
            version_id: m.version_id.clone(),
            factory: m.factory.clone(),
            verification_registry: m.verification_registry.clone(),
            sbt: m.sbt.clone(),
            verifier: m.verifier.clone(),
            // On-chain the circuitId is `keccak256(circuit-string)`, mirrored here from the manifest's
            // plain circuit string so an agreeing manifest reconciles clean.
            circuit_id: keccak_hex(&m.circuit_id),
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
        let mut onchain = onchain_from(&signed.content);
        let true_verifier = "0x1111111111111111111111111111111111111111".to_string();
        onchain.verifier = true_verifier.clone();

        let r = reconcile(&signed, &key.verifying_key(), &onchain).unwrap();
        assert!(!r.manifest_agrees(), "a differing verifier is a conflict");
        assert_eq!(r.conflicts.len(), 1);
        assert_eq!(r.conflicts[0].field, "verifier");
        // The authoritative value the caller must use is the CHAIN's, not the manifest's.
        assert_eq!(r.authoritative.verifier, true_verifier);
        assert_ne!(r.authoritative.verifier, signed.content.verifier);
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

        // Start from an agreeing on-chain record, then diverge exactly the three newly-covered fields.
        let mut onchain = onchain_from(&signed.content);
        let true_circuit = keccak_hex("verification.circom/DogTagVerification(24,5)");
        onchain.circuit_id = true_circuit.clone();
        onchain.min_app_version = "9.9.9".to_string();
        onchain.artifact_base_url = "https://true.dogtag.io/levelb1".to_string();

        let r = reconcile(&signed, &key.verifying_key(), &onchain).unwrap();
        assert!(!r.manifest_agrees(), "differing circuit_id/min_app_version/artifact_base_url conflict");
        let fields: Vec<&str> = r.conflicts.iter().map(|c| c.field).collect();
        assert!(fields.contains(&"circuit_id"), "circuit_id conflict reported, got {fields:?}");
        assert!(fields.contains(&"min_app_version"), "min_app_version conflict reported, got {fields:?}");
        assert!(fields.contains(&"artifact_base_url"), "artifact_base_url conflict reported, got {fields:?}");
        // On-chain wins: the authoritative values are the chain's, not the (validly signed) manifest's.
        assert_eq!(r.authoritative.circuit_id, true_circuit);
        assert_ne!(r.authoritative.circuit_id, keccak_hex(&signed.content.circuit_id));
        assert_eq!(r.authoritative.min_app_version, "9.9.9");
        assert_ne!(r.authoritative.min_app_version, signed.content.min_app_version);
    }

    /// A verified manifest that AGREES with on-chain reconciles cleanly (no conflicts).
    #[test]
    fn agreeing_manifest_reconciles_clean() {
        let key = test_key();
        let signed = sign(&levelb_manifest(), &key);
        let onchain = onchain_from(&signed.content);

        let r = reconcile(&signed, &key.verifying_key(), &onchain).unwrap();
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
        // The graph is unpinned on both sides.
        assert_eq!(m.witness_mobile_sha256, None);
        // The OFF-CHAIN VK identity is present and is NOT the zkey pin (§3.2).
        assert_eq!(m.verification_key_sha256.as_deref(), d.vk.verification_key_json.sha256);
        assert_ne!(m.verification_key_sha256, Some(m.zkey_sha256.clone()));
    }
}
