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
///
/// SUPPLIED BY THE CALLER, never by this crate. It used to be a `const` of `&'static str`s, and that
/// was the wrong shape twice over. It is a MIRROR of chain state, so it is configuration by nature -
/// its own note called itself a redeploy landmine, and by the time that was acted on every one of its
/// four addresses had been superseded, so this build would have served a manifest disagreeing with
/// the chain on every member. And a library constant cannot be repointed by the operator who actually
/// ran the deploy. `reconcile` treating the chain as authoritative made the consequence a pile of
/// phantom conflicts rather than a wrong answer, but a fallback that can only ever conflict is not a
/// fallback. The caller reads these from its own configuration - for vet-api, the same env the deploy
/// writes via `scripts/gen-deployment-env.sh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionDeployment {
    pub chain_id: u64,
    pub factory: String,
    pub verification_registry: String,
    pub sbt: String,
    /// The on-chain VK identity (`Groth16Verifier*` address). NOT a hash.
    pub verifier: String,
    /// The provider-authority core in the verification registry's immutable `issuerRegistry` slot —
    /// generation 2's `ProviderRegistryV2.DiscoverySet.providerRegistry`.
    ///
    /// `None` for a version published to generation 1's `ProtocolRegistry`, whose `ContractSet` struct
    /// has no such member. That absence is load-bearing rather than cosmetic: [`reconcile`] treats a
    /// manifest-`Some` against an on-chain-`None` as a CONFLICT, so claiming an address a
    /// generation-1 record cannot supply would make every reconcile of that version report a phantom
    /// disagreement.
    pub provider_registry: Option<String>,
    /// The contract in the verification registry's immutable `rootIndex` slot — generation 2's
    /// `CloneProvenanceRouter`. `None` for a generation-1 version, for the same reason as
    /// [`Self::provider_registry`]: generation 1's record carries no separate root index (there, the
    /// root index IS `factory`, and duplicating it here would assert a member the chain does not have).
    pub root_index: Option<String>,
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

/// The artifact set currently BOUND to `dogtag-levelb/1` (mirrors `activeArtifactSetOf`).
pub const LEVEL_B_ARTIFACT_RELEASE: ArtifactRelease = ArtifactRelease {
    artifact_set: "dogtag-levelb-artifacts/1",
    artifact_base_url: "https://artifacts.dogtag.io/levelb1",
    min_app_version: "1.4.0", // M-4 PR4 app release floor (iOS + Android)
};

/// The generation-2 DISCOVERY key, published to `ProtocolRegistryV2` (`ProtocolVersionsV2.sol`).
///
/// Deliberately NOT in `artifact.rs` beside [`crate::artifact::LEVEL_B_V1`]: for generation 1 the
/// discovery key and the artifact key are the same string, and for generation 2 they are NOT — the
/// artifacts are byte-for-byte generation 1's, so they keep `dogtag-levelb-artifacts/1`. Filing this
/// under the artifact registry would assert an artifact set that does not exist (R-5, the two axes).
pub const LEVEL_B_V2_VERSION: &str = "dogtag-levelb/2";

/// A version key this build RECOGNIZES but holds no [`VersionDeployment`] for. Carries what is still
/// outstanding and who does it, and NO address field at all — a placeholder or a zero address here is
/// exactly the invented data this fleet forbids.
///
/// For `dogtag-levelb/2` the reason is NOT that the contracts are unbuilt or undeployed: registry-plan
/// S-14 deployed all six live on ROAX, and their addresses are in `contracts/deployments/roax.json`
/// under `_s14_cutover`. What is missing is the PUBLICATION of a discovery set to the deployed
/// `ProtocolRegistryV2`, which carries none — so there is no on-chain record for a manifest to be
/// reconciled against, and serving one would advertise a version the chain does not carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitingDeployment {
    pub version: &'static str,
    /// Who fills this in, so the reader knows what has to happen rather than only that something has
    /// not.
    pub recorded_by: &'static str,
    /// The steps that remain before this version can be served, in the order they happen.
    pub outstanding: &'static [&'static str],
}

/// `dogtag-levelb/2` — recognized; its contracts are deployed, but nothing is published for them yet.
/// See `docs/CLIENT_REPOINT.md`.
pub const LEVEL_B_V2_AWAITING: AwaitingDeployment = AwaitingDeployment {
    version: LEVEL_B_V2_VERSION,
    recorded_by: "publication to the deployed ProtocolRegistryV2 \
                  (contracts/script/PublishProtocolVersionsV2.s.sol), then recording that published \
                  record here",
    outstanding: &[
        "publish a generation-2 discovery set to the deployed ProtocolRegistryV2 - S-14 deployed that \
         registry but published nothing to it, so it holds no discovery record",
        "record the resulting on-chain record here as a VersionDeployment, so a served manifest has \
         something to be reconciled against",
        "repoint clients at the generation-2 addresses (cutover C-9/C-10)",
    ],
};

/// Why a version has no [`VersionDeployment`]. The two absences are DIFFERENT and must not collapse.
///
/// `Unknown` is a typo, or a version this build does not serve.
/// `AwaitingDeployment` is a key this build knows and holds no [`VersionDeployment`] for.
///
/// Collapsing them is the could-not-check-rendered-as-a-neighbour defect this repo closes everywhere
/// else: an operator asking for `dogtag-levelb/2` mid-cutover and reading `unknown version` goes
/// hunting a misspelling, when the real answer is that its contracts are deployed (S-14) and no
/// discovery set has been published to `ProtocolRegistryV2` yet. Both still fail closed and serve
/// nothing — this changes the DIAGNOSIS, never the outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentStatus {
    /// This build serves the version. Its on-chain record is CONFIGURATION and comes from the caller
    /// (see [`VersionDeployment`]), which is why this arm carries no addresses of its own.
    Served,
    AwaitingDeployment(&'static AwaitingDeployment),
    Unknown,
}

/// Classify a version key. Fail-closed in every arm: only `Served` can yield a manifest, and even
/// then only if the caller supplies the deployment.
pub fn deployment_status(version: &str) -> DeploymentStatus {
    match version {
        crate::artifact::LEVEL_B_V1 => DeploymentStatus::Served,
        LEVEL_B_V2_VERSION => DeploymentStatus::AwaitingDeployment(&LEVEL_B_V2_AWAITING),
        _ => DeploymentStatus::Unknown,
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

/// Build the manifest content for a known version from its file-verified descriptor, the CALLER's
/// on-chain contract set, and its bound artifact set. `None` if the version is unrecognized.
///
/// `deployment` is a parameter rather than a table lookup because it mirrors chain state, which this
/// crate cannot know and must not guess - see [`VersionDeployment`]. The descriptor and the artifact
/// release stay internal: those ARE properties of this build (it file-verifies the pins against the
/// committed artifacts), so they are not the caller's to supply.
pub fn build(version: &str, deployment: &VersionDeployment) -> Option<Manifest> {
    let desc = crate::artifact::resolve(Some(version)).ok()?;
    if !matches!(deployment_status(version), DeploymentStatus::Served) {
        return None;
    }
    let release = artifact_release_for(version)?;
    Some(Manifest::from_descriptor(desc, deployment, release))
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
    /// The provider-authority core, when the version's on-chain record carries one (generation 2).
    ///
    /// `skip_serializing_if` is what keeps this widening ADDITIVE: [`Manifest::canonical_bytes`] is
    /// `serde_json` over this struct, so a generation-1 manifest — where both new members are `None` —
    /// serializes to exactly the bytes it did before these fields existed, and any signature already
    /// produced over it still verifies. A generation-2 manifest carries them and signs over them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_registry: Option<String>,
    /// The root index (generation 2's `CloneProvenanceRouter`), when the version's record carries one.
    /// Same additive treatment as [`Self::provider_registry`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_index: Option<String>,
    pub circuit_id: String,
    pub num_public: usize,
    pub public_signal_layout: Vec<String>,
    /// FETCH pin — SHA-256 of the zkey file (mandatory).
    pub zkey_sha256: String,
    /// FETCH pin — SHA-256 of the `.graph` (`None` == unpinned, §3.5; pinned on ROAX since 2026-07-28).
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
            provider_registry: deploy.provider_registry.clone(),
            root_index: deploy.root_index.clone(),
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
    /// The on-chain `providerRegistry` member — `Some` when read from a generation-2
    /// `ProtocolRegistryV2.DiscoverySet`, `None` when read from a generation-1 `ContractSet`, which has
    /// no such member. Compared by [`reconcile`] like any other mirrored address, so a manifest that
    /// claims one where the chain has none (or vice versa) is a recorded CONFLICT rather than a silent
    /// acceptance.
    pub provider_registry: Option<String>,
    /// The on-chain `rootIndex` member — generation 2's `CloneProvenanceRouter`. `None` on generation 1,
    /// where the root index is the `factory` member and no separate slot exists.
    pub root_index: Option<String>,
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
    // --- the ON-CHAIN axis ---
    cmp(&mut conflicts, "version_id", &contract_set.version_id, &m.version_id);
    cmp(&mut conflicts, "factory", &contract_set.factory, &m.factory);
    cmp(&mut conflicts, "verification_registry", &contract_set.verification_registry, &m.verification_registry);
    cmp(&mut conflicts, "sbt", &contract_set.sbt, &m.sbt);
    cmp(&mut conflicts, "verifier", &contract_set.verifier, &m.verifier);
    // The generation-2 members. Optional on BOTH sides, and a present-vs-absent disagreement in either
    // direction is a conflict rather than a shrug — which is what makes the two generations' manifests
    // non-interchangeable: a generation-1 manifest reconciled against a generation-2 record reports the
    // two missing addresses instead of validating against a record whose authority core and root index it
    // never attested. Absence renders as `<absent>`, not as the pins' `<unpinned>`: a missing address is
    // a member the record does not have, not a hash somebody chose not to pin.
    cmp_opt_addr(&mut conflicts, "provider_registry", &contract_set.provider_registry, &m.provider_registry);
    cmp_opt_addr(&mut conflicts, "root_index", &contract_set.root_index, &m.root_index);
    // On-chain `circuitId` is a bytes32 (`keccak256(circuit-string)`) while the manifest carries the
    // plain circuit string, so hash the manifest's before the (case-insensitive hex) compare.
    cmp(&mut conflicts, "circuit_id", &contract_set.circuit_id, &keccak_hex(&m.circuit_id));

    // --- the ARTIFACT axis ---
    // `artifact_set_id` is the axis's own identity: comparing it is what stops a manifest describing
    // artifact set N from being accepted while the chain has rotated the binding to N+1.
    cmp(&mut conflicts, "artifact_set_id", &artifact_set.artifact_set_id, &m.artifact_set_id);
    cmp(&mut conflicts, "zkey_sha256", &artifact_set.zkey_sha256, &m.zkey_sha256);
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

/// Compare a mirrored field CASE-INSENSITIVELY (checksum vs lowercase hex must not read as a conflict;
/// the pins are lowercase on both sides, so it is harmless for them too).
///
/// A free function taking `&mut conflicts`, uniform with [`cmp_opt`]/[`cmp_str`], rather than a closure
/// capturing the vector: a closure holds the mutable borrow for its whole lifetime, so the comparisons had
/// to be ordered around the borrow checker rather than around the two axes they describe.
fn cmp(conflicts: &mut Vec<FieldConflict>, field: &'static str, oc: &str, mf: &str) {
    if !oc.eq_ignore_ascii_case(mf) {
        conflicts.push(FieldConflict {
            field,
            onchain: oc.to_string(),
            manifest: mf.to_string(),
        });
    }
}

/// Exact (case-sensitive) string comparison - for the on-chain string members (`artifactBaseUrl`,
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

/// The presence/case-folding rule shared by every optional member: case-insensitive when both sides are
/// present, agreement when both are absent, and a conflict whenever presence itself differs.
///
/// `absent` is the word rendered into a conflict for the missing side, and it is a PARAMETER rather than
/// a constant because the two kinds of optional member mean different things by absence - see
/// [`cmp_opt`] and [`cmp_opt_addr`], which are the only callers and exist solely to fix that word. The
/// rule lives here once so a future change to the presence semantics cannot land on one kind and not the
/// other.
fn cmp_opt_with(
    conflicts: &mut Vec<FieldConflict>,
    field: &'static str,
    oc: &Option<String>,
    mf: &Option<String>,
    absent: &str,
) {
    let same = match (oc, mf) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    };
    if !same {
        conflicts.push(FieldConflict {
            field,
            onchain: oc.clone().unwrap_or_else(|| absent.into()),
            manifest: mf.clone().unwrap_or_else(|| absent.into()),
        });
    }
}

/// An optional PIN. Absence renders as `<unpinned>`: the artifact exists and somebody chose not to pin
/// its hash.
fn cmp_opt(
    conflicts: &mut Vec<FieldConflict>,
    field: &'static str,
    oc: &Option<String>,
    mf: &Option<String>,
) {
    cmp_opt_with(conflicts, field, oc, mf, "<unpinned>");
}

/// An optional ADDRESS member. Same comparison rule as [`cmp_opt`] and a DIFFERENT word for absence,
/// which is the whole reason the two wrappers exist: `<unpinned>` would describe a hash nobody chose to
/// pin, while these two members are simply not part of a generation-1 record. Do not collapse the two
/// placeholders - the word is what tells a reader whether a record lacks the member or merely lacks a
/// hash for it.
fn cmp_opt_addr(
    conflicts: &mut Vec<FieldConflict>,
    field: &'static str,
    oc: &Option<String>,
    mf: &Option<String>,
) {
    cmp_opt_with(conflicts, field, oc, mf, "<absent>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact;

    fn test_key() -> SigningKey {
        // Deterministic key for tests (Date/rand are avoided so the round-trip is reproducible).
        SigningKey::from_bytes(&[7u8; 32])
    }

    /// A SYNTHETIC on-chain record. The crate ships none - a deployment mirrors chain state and is
    /// the caller's to supply - and these tests are about the manifest's shape, its signing and its
    /// reconcile rules, none of which depend on which addresses the members hold. Distinct per member
    /// so a field-order mistake cannot pass on two slots sharing a value.
    fn test_deployment() -> VersionDeployment {
        VersionDeployment {
            chain_id: 135,
            factory: "0x1C9Ac2eB3f1A2D4B5C6d7E8f90A1B2C3D4e5F607".to_string(),
            verification_registry: "0x2B4d6f8a0c1e3a5b7d9f0e2C4a6b8d0F1E3A5c70".to_string(),
            sbt: "0x3c5e7A9b0D2F4a6c8E0b1d3F5A7c9e0B2D4F6a80".to_string(),
            verifier: "0x4d6F8B0C2E4A6b8d0F2c4e6a8b0D2F4c6e8A0b90".to_string(),
            provider_registry: None,
            root_index: None,
        }
    }

    fn levelb_manifest() -> Manifest {
        Manifest::from_descriptor(
            artifact::resolve(Some(artifact::LEVEL_B_V1)).unwrap(),
            &test_deployment(),
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
            provider_registry: m.provider_registry.clone(),
            root_index: m.root_index.clone(),
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
    /// S-13: a recognized-but-undeployed version is NOT reported as an unknown one.
    ///
    /// The two absences have unrelated remedies — a typo is the caller's to fix, while a recognized
    /// key with no on-chain record is fixed by publishing that record — so collapsing them sends an
    /// operator hunting a misspelling that does not exist. Mutation: make `deployment_status`'s
    /// `LEVEL_B_V2_VERSION` arm return `Unknown` and this reddens.
    #[test]
    fn a_recognized_but_undeployed_version_is_distinguished_from_an_unknown_one() {
        assert!(matches!(
            deployment_status(crate::artifact::LEVEL_B_V1),
            DeploymentStatus::Served
        ));
        assert!(matches!(
            deployment_status(LEVEL_B_V2_VERSION),
            DeploymentStatus::AwaitingDeployment(_)
        ));
        assert_eq!(deployment_status("dogtag-levelb/9"), DeploymentStatus::Unknown);
        assert_eq!(deployment_status(""), DeploymentStatus::Unknown);
    }

    /// ...and it still FAILS CLOSED. The point of the state is diagnosis, never permission: only a
    /// `Served` version may yield a manifest, and supplying a perfectly good deployment does not
    /// change that - which is the case worth pinning now that the deployment is a parameter, because
    /// a caller CAN hand `build` a valid record for a version this build does not serve.
    #[test]
    fn an_undeployed_version_still_yields_no_manifest_even_with_a_deployment() {
        assert!(build(LEVEL_B_V2_VERSION, &test_deployment()).is_none());
        assert!(artifact_release_for(LEVEL_B_V2_VERSION).is_none());
        assert!(build("dogtag-levelb/9", &test_deployment()).is_none());
    }

    /// The awaiting record carries no address field at all, so it cannot become a placeholder that
    /// looks real. It must, however, name what is missing and who fills it — an unset state that says
    /// only "unset" is the thing this replaces.
    ///
    /// Since S-14 the remedy is PUBLICATION, not deployment: naming a deployed contract as outstanding
    /// would send an operator to redeploy something that already exists on chain, which is the same
    /// wrong-remedy defect the `Unknown`/`AwaitingDeployment` split exists to prevent.
    #[test]
    fn the_awaiting_record_names_what_is_outstanding_without_inventing_an_address() {
        let a = &LEVEL_B_V2_AWAITING;
        assert_eq!(a.version, "dogtag-levelb/2");
        assert!(!a.recorded_by.is_empty());
        assert!(!a.outstanding.is_empty());
        // The remedy names publication, and names the registry it must be published to.
        assert!(a.recorded_by.contains("ProtocolRegistryV2"));
        assert!(a
            .outstanding
            .iter()
            .any(|s| s.contains("publish") && s.contains("ProtocolRegistryV2")));
        // The two deliberately-reused addresses are NOT outstanding — moving either is unrecoverable.
        assert!(!a.outstanding.iter().any(|s| s.contains("SBT")));
        assert!(!a.outstanding.iter().any(|s| s.contains("Verifier")));
        // Still no address anywhere, in either field.
        assert!(!a.recorded_by.contains("0x"));
        assert!(!a.outstanding.iter().any(|s| s.contains("0x")));
    }

    /// Generation 2's DISCOVERY key is not an ARTIFACT key. The artifacts are byte-for-byte generation
    /// 1's, so `dogtag-levelb/2` must never resolve one: a second artifact identity for identical bytes
    /// would be a falsehood, and it would also swap the app-gate diagnostic away from `AppTooOld`.
    #[test]
    fn the_generation_2_discovery_key_is_not_an_artifact_key() {
        assert!(crate::artifact::resolve(Some(LEVEL_B_V2_VERSION)).is_err());
        assert_ne!(LEVEL_B_V2_VERSION, crate::artifact::LEVEL_B_V1);
    }

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

    /// The generation-2 members are ADDITIVE: a generation-1 manifest, where both are `None`, serializes
    /// to exactly the bytes it did before the members existed, so a signature already produced over such a
    /// manifest still verifies. `skip_serializing_if` is the whole mechanism, and it is what this asserts —
    /// dropping it would silently invalidate every previously-signed manifest, and only at go-live, when a
    /// pinned key finally makes signatures load-bearing.
    #[test]
    fn the_generation_two_members_do_not_disturb_a_generation_one_signature() {
        let m = levelb_manifest();
        assert_eq!(m.provider_registry, None, "generation 1's record has no authority-core member");
        assert_eq!(m.root_index, None, "...and no separate root-index member");

        let json: serde_json::Value =
            serde_json::from_slice(&serde_json::to_vec(&m).unwrap()).unwrap();
        let obj = json.as_object().unwrap();
        assert!(!obj.contains_key("provider_registry"), "an absent member emits no key");
        assert!(!obj.contains_key("root_index"), "an absent member emits no key");

        // ...and the signature over those bytes verifies, which is the property the key absence buys.
        let key = test_key();
        let signed = sign(&m, &key);
        verify(&signed, &key.verifying_key()).expect("a generation-1 manifest still verifies");
    }

    /// A generation-2 manifest carries both members, signs over them, and round-trips — so the additive
    /// treatment above does not amount to the fields being dropped.
    #[test]
    fn a_generation_two_manifest_carries_and_signs_both_members() {
        let mut m = levelb_manifest();
        m.provider_registry = Some("0x9309aB1c2D3e4F5061728394a5B6c7D8e9F00112".to_string());
        m.root_index = Some("0x120127E4a5B6c7D8E9f001122334455667788990".to_string());

        let json: serde_json::Value =
            serde_json::from_slice(&serde_json::to_vec(&m).unwrap()).unwrap();
        assert_eq!(json["provider_registry"], *m.provider_registry.as_ref().unwrap());
        assert_eq!(json["root_index"], *m.root_index.as_ref().unwrap());

        let key = test_key();
        let signed = sign(&m, &key);
        verify(&signed, &key.verifying_key()).unwrap();
        let back: Manifest = serde_json::from_value(json).unwrap();
        assert_eq!(back, m);

        // Tampering with either member breaks the signature, i.e. they are genuinely signed over.
        let mut tampered = signed.clone();
        tampered.content.root_index = Some("0x0000000000000000000000000000000000000bad".to_string());
        assert_eq!(
            verify(&tampered, &key.verifying_key()),
            Err(ManifestError::BadSignature),
            "the root index is inside the signed bytes"
        );
    }

    /// The two generations' manifests are NOT interchangeable. Reconciling a generation-1 manifest against
    /// a generation-2 record reports both missing addresses rather than accepting a record whose authority
    /// core and root index the manifest never attested — and the reverse direction is a conflict too.
    ///
    /// This is the same present-vs-absent rule the graph pin already follows, and it is why the members are
    /// `None` on generation 1 rather than being back-filled with the factory address: a back-fill would
    /// make every generation-1 reconcile report a phantom disagreement.
    #[test]
    fn a_generation_mismatch_is_a_conflict_on_both_members_in_both_directions() {
        let key = test_key();
        let signed = sign(&levelb_manifest(), &key); // manifest: both None

        let mut gen_two_chain = contracts_from(&signed.content);
        gen_two_chain.provider_registry = Some("0x9309aB1c2D3e4F5061728394a5B6c7D8e9F00112".to_string());
        gen_two_chain.root_index = Some("0x120127E4a5B6c7D8E9f001122334455667788990".to_string());

        let r = reconcile(&signed, &key.verifying_key(), &gen_two_chain, &artifacts_from(&signed.content))
            .unwrap();
        assert!(!r.manifest_agrees());
        let fields: Vec<&str> = r.conflicts.iter().map(|c| c.field).collect();
        assert!(fields.contains(&"provider_registry"), "got {fields:?}");
        assert!(fields.contains(&"root_index"), "got {fields:?}");
        // Absence is reported as absent, not as an unpinned hash — the word matters to whoever reads it.
        let root = r.conflicts.iter().find(|c| c.field == "root_index").unwrap();
        assert_eq!(root.manifest, "<absent>");
        assert_eq!(root.onchain, "0x120127E4a5B6c7D8E9f001122334455667788990");
        // The trio is untouched, so only the two new members disagree.
        assert!(!fields.contains(&"factory") && !fields.contains(&"verifier"), "got {fields:?}");

        // The reverse: a generation-2 manifest against a generation-1 record.
        let mut gen_two_manifest = levelb_manifest();
        gen_two_manifest.provider_registry = Some("0x9309aB1c2D3e4F5061728394a5B6c7D8e9F00112".to_string());
        gen_two_manifest.root_index = Some("0x120127E4a5B6c7D8E9f001122334455667788990".to_string());
        let signed_two = sign(&gen_two_manifest, &key);
        let mut gen_one_chain = contracts_from(&signed_two.content);
        gen_one_chain.provider_registry = None;
        gen_one_chain.root_index = None;
        let r2 =
            reconcile(&signed_two, &key.verifying_key(), &gen_one_chain, &artifacts_from(&signed_two.content))
                .unwrap();
        let fields2: Vec<&str> = r2.conflicts.iter().map(|c| c.field).collect();
        assert!(fields2.contains(&"provider_registry") && fields2.contains(&"root_index"), "got {fields2:?}");
    }

    /// A differing address on either member is a conflict, and on-chain wins — the same precedence the
    /// trio already gets. Without this, a stale manifest could steer a caller onto a root index the chain
    /// does not name, which resolves a different set of historical roots.
    #[test]
    fn a_differing_generation_two_member_resolves_to_onchain() {
        let key = test_key();
        let mut m = levelb_manifest();
        m.provider_registry = Some("0x9309aB1c2D3e4F5061728394a5B6c7D8e9F00112".to_string());
        m.root_index = Some("0x120127E4a5B6c7D8E9f001122334455667788990".to_string());
        let signed = sign(&m, &key);

        let mut chain = contracts_from(&signed.content);
        chain.root_index = Some("0x00000000000000000000000000000000dead0000".to_string());

        let r = reconcile(&signed, &key.verifying_key(), &chain, &artifacts_from(&signed.content)).unwrap();
        assert!(!r.manifest_agrees());
        assert_eq!(
            r.contract_set.root_index.as_deref(),
            Some("0x00000000000000000000000000000000dead0000"),
            "the reconciled record is the CHAIN's, never the manifest's"
        );

        // Case alone is not a disagreement, matching how the trio addresses are compared.
        let mut cased = contracts_from(&signed.content);
        cased.root_index = Some("0x120127e4a5b6c7d8e9f001122334455667788990".to_string());
        let r2 = reconcile(&signed, &key.verifying_key(), &cased, &artifacts_from(&signed.content)).unwrap();
        assert!(r2.manifest_agrees(), "checksum vs lowercase is not a conflict: {:?}", r2.conflicts);
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
