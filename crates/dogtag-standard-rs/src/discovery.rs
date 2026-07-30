//! Discovery anchor-validation — the client TRUST gate (M7 §5.2 / §5.3, brick P4).
//!
//! A platform's non-consuming resolve GET returns a CONVENIENCE tier — [`ConvenienceClaims`]:
//! platform-OWNED `{protocolVersion, chainId, verificationRegistry, issuerClone, purpose}`. Those are
//! CLAIMS, never authority (arch §1.2 trust boundary: the platform backend is NOT trusted for the
//! `{version, registry}` fields). Before the app acts on any platform-supplied version/registry it
//! resolves the dogtag-owned TRUST tier — [`TrustedAnchor`] — from
//! `ProtocolRegistry.getContractSet(keccak256(version))` + `getActiveArtifactSet` (the on-chain root of
//! truth) or the P3 signed-manifest fallback ([`crate`] has no manifest types; they live in the
//! server-only prover crate `dogtag_prover::manifest`, which mirrors the same two on-chain axes).
//! [`validate`] then REQUIRES the
//! claims to MATCH the anchor and enforces `minAppVersion`, FAIL-CLOSED on every axis.
//!
//! # Why this is the load-bearing security step (§5.3 step 4-5)
//!
//! A lying platform's only lever is the convenience tier. If the app trusted it, the platform could
//! steer a verifier onto an ATTACKER registry/chain (redirect the proof) or silently swap the purpose
//! the owner is consenting to. Pinning every trust-critical claim to the dogtag anchor removes that
//! lever: the platform can lie, but the app aborts before proving. The residual trust is dogtag's
//! ProtocolRegistry alone (arch §8.6) — exactly where it already sits for the trio.
//!
//! # Why it lives in the standard crate, and is PURE
//!
//! The mobile app links THIS crate (the UniFFI surface) but NOT the ark-heavy `dogtag_prover` (server
//! only), so the validator that "the app path calls" must live here. It is deliberately PURE — string /
//! integer / dotted-numeric-semver comparison only, no ZK, no chain I/O, no signature check — so it is
//! unit-testable independent of the mobile UI and reusable by the server. RESOLVING the anchor (the
//! `eth_call`, or the ed25519 manifest verify + on-chain reconcile in P3 `dogtag_prover::manifest`) is
//! the CALLER's job; this validates the ALREADY-resolved anchor.
//!
//! # `purpose` is checked against the app's OUT-OF-BAND intent, not a chain field
//!
//! Neither on-chain axis — the `ContractSet` nor the `ArtifactSet` — carries a purpose (purpose is
//! per-verification, not per-version), so [`validate`] compares the claimed purpose to
//! [`ClientContext::expected_purpose`] —
//! the purpose the app/user independently intends for this scan. That value MUST come from a source
//! independent of the platform's claim; comparing a platform claim against the platform's own session
//! would be vacuous. This is a consent-integrity check complementary to the registry/chain anti-redirect
//! checks.

use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

/// `0x`-hex keccak256 of a version string — the on-chain `ProtocolRegistry` map key for that version,
/// and the cross-boundary tie between a claimed version STRING and the on-chain `versionId`. MUST equal
/// `dogtag_prover::manifest::version_id` and `keccak256(bytes(version))` in `ProtocolVersions.sol`.
pub fn version_id(version: &str) -> String {
    format!("0x{}", hex::encode(Keccak256::digest(version.as_bytes())))
}

/// The CONVENIENCE tier (§5.2): the platform-OWNED claims a resolve GET returns. NONE of these is
/// authority — every trust-critical field is validated against the [`TrustedAnchor`] before use
/// ([`validate`]). Serialized camelCase so it is the exact nested `unverifiedClaims` block the resolve GET emits
/// and the app deserializes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ConvenienceClaims {
    /// CLAIMED protocol version, e.g. `dogtag-levelb/1`. The app resolves the anchor BY this string.
    pub protocol_version: String,
    /// CLAIMED chain id. A lying platform would forge this to point at a fork/testnet.
    pub chain_id: u64,
    /// CLAIMED verification registry address — THE redirect target a lying platform would forge.
    pub verification_registry: String,
    /// CLAIMED issuer clone (documentStore) the platform will issue/verify against. Carried for the app;
    /// it is NOT a trust-tier axis — the on-chain re-derivation binds the clone from `rootIssuer[R]`
    /// (§4.3), so a forged `issuerClone` cannot make an invalid record verify.
    pub issuer_clone: String,
    /// CLAIMED purpose (the verify-gate namespace / consent purpose). Checked against the app's
    /// out-of-band expected purpose ([`ClientContext::expected_purpose`]), never the platform's session.
    pub purpose: String,
}

/// The dogtag-owned TRUST tier (§5.2) for one version, RESOLVED by the caller from
/// `ProtocolRegistry.getContractSet` + `getActiveArtifactSet` (root of truth) or the P3 signed-manifest
/// fallback (both mirror the same two on-chain axes). These are the AUTHORITATIVE values a platform's
/// [`ConvenienceClaims`] are
/// checked against. Constructed from plain fields so the app can build it from an `eth_call` result or a
/// parsed/reconciled manifest, and the server can map it from `dogtag_prover::manifest` types — keeping
/// this crate free of any prover/manifest dependency.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TrustedAnchor {
    /// The version string this record certifies (e.g. `dogtag-levelb/1`).
    pub version: String,
    /// keccak256(version) `0x`-hex — the on-chain `contractSetId`; ties the claimed version STRING to
    /// the on-chain key so a caller cannot silently validate against the wrong record.
    pub version_id: String,
    /// The ARTIFACT-AXIS identity the caller resolved for this version (e.g.
    /// `dogtag-levelb-artifacts/1`) — the artifact set `activeArtifactSetOf[versionId]` points at.
    ///
    /// It is a SECOND, independent axis (R-5): rotating the proving artifacts changes this and
    /// `min_app_version` while `version`/`version_id` and `verification_registry` stay put. Carrying it
    /// on the anchor is what lets a caller say WHICH artifact set it is about to fetch, rather than
    /// inferring it from the version.
    pub artifact_set: String,
    /// keccak256(artifact_set) `0x`-hex — the on-chain `artifactSetId`. A DIFFERENT keyspace from
    /// `version_id`; [`validate`] checks their coherence independently.
    pub artifact_set_id: String,
    /// The chain the trio lives on. (Carried by the manifest / known by the app from its RPC endpoint;
    /// the on-chain `ContractSet` struct itself has no chain-id member — the registry IS on a chain.)
    pub chain_id: u64,
    /// The registry a proof is submitted to (trio leg). The anti-redirect anchor.
    pub verification_registry: String,
    /// The circuit this version proves (`<source>/<template>(<params>)`) — the artifact-selection key
    /// (§5.3 step 6; version↔circuit_id is 1:1 in `dogtag_prover::artifact`).
    pub circuit_id: String,
    /// Minimum app build (semver) allowed to use this version — the deprecation lever (§5.3 step 5).
    pub min_app_version: String,
    /// The ON-CHAIN axis's lifecycle bit — `ContractSet.active`, as read from
    /// `ProtocolRegistry.getContractSet`. `deprecateContractSet` flips it false.
    ///
    /// It is an INDEPENDENT kill switch, and [`validate`] requires BOTH it and
    /// [`Self::artifact_set_active`] to be true. Populate the two SEPARATELY from the two on-chain
    /// records — never AND them into one field and never wire only one: R-5 splits the axes precisely so
    /// each can retire the other's counterpart without touching it, so collapsing them here would
    /// silently discard whichever lever you dropped. A false bit FAILS CLOSED (anti-downgrade, §8.4).
    ///
    /// The signed manifest carries no lifecycle bit, so a manifest-only (offline) resolution assumes
    /// `true` for both; the authoritative values are the two on-chain `active` members when online — see
    /// `anchor_from_manifest` / `anchor_from_reconciliation` on the server side.
    pub contract_set_active: bool,
    /// The ARTIFACT axis's lifecycle bit — `ArtifactSet.active`, as read from
    /// `ProtocolRegistry.getActiveArtifactSet`. `deprecateArtifactSet` flips it false.
    ///
    /// The independent counterpart to [`Self::contract_set_active`]: BOTH must be true for [`validate`]
    /// to pass. This is the bit that lets dogtag retire a compromised proving-artifact set (a bad zkey)
    /// and stop every app WITHOUT moving a single trio address. See that field's note for why the two
    /// must be populated separately.
    pub artifact_set_active: bool,
    /// The provider-authority core (`ProviderRegistry`) this version's verification registry holds in its
    /// immutable `issuerRegistry` slot — and the root of the resolver layer, since a provider's directory
    /// resolver and a service's domain resolver are both selected THROUGH it.
    ///
    /// `None` is the honest shape of a GENERATION-1 record: `ProtocolRegistry.ContractSet` has no such
    /// member, so a caller resolving that record has nothing to report and must say so rather than invent
    /// an address. That is an ACCURATE OBSERVATION about the record's shape, and it is NOT a
    /// could-not-check: a read that FAILED must surface as a failed resolution and must never reach
    /// [`validate`] as a `None`. Generation 2's `ProtocolRegistryV2.DiscoverySet` always carries it (the
    /// registry refuses to publish a zero), so a caller reading that record must populate it.
    pub provider_registry: Option<String>,
    /// Whatever this version's verification registry holds in its immutable `rootIndex` slot — the
    /// contract that answers `rootIssuer(bytes32)` and `isClone(address)`. In generation 2 that is the
    /// `CloneProvenanceRouter`, which resolves a root across factory generations.
    ///
    /// This is NOT the factory. Generation 1 could conflate them because its registry's root index WAS
    /// its factory; generation 2 cannot, and a consumer that reads a factory address here resolves only
    /// the roots anchored in that generation while silently missing every earlier one — the exact failure
    /// the router exists to prevent. `None` carries the same generation-1 meaning as
    /// [`Self::provider_registry`].
    pub root_index: Option<String>,
}

/// Client-side context the validation needs beyond the anchor: this build's version and the app's
/// OUT-OF-BAND expected purpose. A borrow struct (never crosses the FFI boundary — the FFI wrapper
/// takes the two strings directly), so it carries a lifetime and is not a UniFFI record.
#[derive(Debug, Clone, Copy)]
pub struct ClientContext<'a> {
    /// This app build's version (dotted-numeric semver), compared against the anchor's `min_app_version`.
    pub app_version: &'a str,
    /// The purpose the app/user independently intends for this scan (§5.3 step 4). MUST be sourced
    /// independently of the platform's claim.
    pub expected_purpose: &'a str,
}

/// Which of the anchor's two independent axes (R-5) was found deprecated. Carried by
/// [`DiscoveryError::DeprecatedVersion`] and rendered into its message, so a caller that only ever sees
/// the flattened FFI error string can still tell which lever fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeprecatedAxis {
    /// The on-chain `ContractSet` — `deprecateContractSet`. The protocol version itself is retired.
    ContractSet,
    /// The bound `ArtifactSet` — `deprecateArtifactSet`. The proving artifacts were pulled; the trio is
    /// untouched, so a NEWER artifact set may already be published for the same version.
    ArtifactSet,
}

impl std::fmt::Display for DeprecatedAxis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ContractSet => f.write_str("on-chain contract set"),
            Self::ArtifactSet => f.write_str("proving-artifact set"),
        }
    }
}

/// Every way [`validate`] can FAIL CLOSED. Each variant means "abort — never prove, never submit". Kept
/// as a plain `thiserror` enum (not a UniFFI error): the FFI wrapper flattens it into the crate's single
/// `FfiError`, and the server matches on it directly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    /// The caller resolved an anchor whose version does not match the claimed version (a coherence bug
    /// or a swapped record). Ties by both the string and its keccak `versionId`.
    #[error("claimed version {claimed:?} does not match the resolved anchor version {anchor:?}")]
    VersionMismatch { claimed: String, anchor: String },
    /// The anchor's artifact-axis halves disagree: `artifact_set_id != keccak256(artifact_set)`. The
    /// artifact axis is dogtag-owned (a platform never claims it), so this is a CALLER-INTEGRITY guard —
    /// the same guard check (1) applies to the contract axis, applied independently to the second axis
    /// (R-5). A caller that stitched an anchor from a mismatched pair is refused rather than allowed to
    /// fetch artifacts under the wrong identity.
    #[error("anchor artifactSet {artifact_set:?} does not hash to its artifactSetId {artifact_set_id:?}")]
    ArtifactSetIncoherent {
        artifact_set: String,
        artifact_set_id: String,
    },
    /// The claimed chain id differs from the anchor — the platform cannot point the app at another chain.
    #[error("claimed chainId {claimed} does not match anchor chainId {anchor}")]
    ChainIdMismatch { claimed: u64, anchor: u64 },
    /// The claimed verification registry differs from the anchor — the anti-redirect trip.
    #[error("claimed verificationRegistry {claimed:?} does not match anchor {anchor:?}")]
    RegistryMismatch { claimed: String, anchor: String },
    /// The claimed purpose differs from the app's out-of-band expected purpose.
    #[error("claimed purpose {claimed:?} does not match the app's expected purpose {expected:?}")]
    PurposeMismatch { claimed: String, expected: String },
    /// One of the anchor's two axes is deprecated (`active=false`) — refused (anti-downgrade, §8.4).
    /// `axis` names WHICH lever fired, because the two are independent and the remedy differs: a
    /// deprecated contract set means the whole protocol version is retired, while a deprecated artifact
    /// set means only the proving artifacts were pulled and the trio is untouched.
    #[error("the {axis} of version {version:?} is deprecated (inactive) at the anchor")]
    DeprecatedVersion {
        version: String,
        axis: DeprecatedAxis,
    },
    /// The app build is older than the version's `minAppVersion` — refuse and route to update/owner-web.
    #[error("app build {build:?} is older than minAppVersion {min:?} for this version")]
    AppTooOld { build: String, min: String },
    /// A version string was not a clean dotted-numeric semver. FAIL CLOSED — a malformed version must
    /// never read as "new enough".
    #[error("{which} {value:?} is not a valid dotted-numeric semver")]
    BadSemver { which: &'static str, value: String },
    /// An anchor address the caller DID report is not a usable address (empty, not `0x` + 40 hex, or the
    /// zero address). A CALLER-INTEGRITY guard, and the only check available for these two members: unlike
    /// `verification_registry` — which is compared against the platform's claim, so a garbled value fails
    /// that comparison — nothing claims a provider registry or a root index, so shape is all there is.
    ///
    /// Absence is NOT an error (a generation-1 record has no such member, see [`TrustedAnchor`]); a
    /// present-but-unusable value is, because the caller would go on to `eth_call` it and read an empty
    /// answer that is neither a definite yes nor a definite no.
    #[error("anchor {which} {value:?} is not a usable address")]
    MalformedAnchorAddress { which: &'static str, value: String },
}

/// The result of a PASSING validation (§5.3): the version + its artifact-selection key + the
/// authoritative trust-critical fields the caller may now act on. The caller selects the proving
/// artifact by `version`/`circuit_id` (§5.3 step 6): `dogtag_prover::artifact::resolve(Some(&version))`
/// yields the descriptor, whose `circuit_id` MUST equal this.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ValidatedVersion {
    pub version: String,
    pub circuit_id: String,
    pub chain_id: u64,
    pub verification_registry: String,
    /// The validated ARTIFACT-AXIS identity (R-5) — which proving-artifact set the caller may now
    /// fetch. Returned separately from `version` precisely because it moves separately: an artifact
    /// rotation changes this while `version`/`circuit_id`/`verification_registry` are unchanged.
    pub artifact_set: String,
    /// The validated provider-authority core, or `None` when the resolved record does not carry one.
    /// Returned so a caller acts on the value this function checked rather than re-reading the anchor —
    /// the same reason `verification_registry` is returned.
    pub provider_registry: Option<String>,
    /// The validated root index (generation 2's `CloneProvenanceRouter`), or `None` when the resolved
    /// record does not carry one. A caller reading `rootIssuer`/`isClone` MUST use this rather than a
    /// bundled factory address; see [`TrustedAnchor::root_index`].
    pub root_index: Option<String>,
}

/// Validate a platform's [`ConvenienceClaims`] against the dogtag-owned [`TrustedAnchor`] and enforce
/// `minAppVersion` (§5.3 steps 4-6). FAIL-CLOSED on every axis: any mismatch returns `Err` and the
/// caller MUST abort. On success the caller may select the artifact by the returned `circuit_id`/`version`
/// and proceed to prove/submit.
///
/// The checks, in order (order affects only which error surfaces first — every one is fail-closed):
/// 1. version coherence (the anchor is the record for the claimed version),
///    1b. artifact-axis coherence (the anchor's `artifact_set` hashes to its `artifact_set_id`),
/// 2. neither axis deprecated — BOTH `contract_set_active` and `artifact_set_active` must hold,
/// 3. chainId == anchor,
/// 4. verificationRegistry == anchor (case-insensitive — checksum vs lowercase is not a real mismatch),
/// 5. purpose == the app's out-of-band expected purpose (case-SENSITIVE — a purpose is a semantic
///    namespace, not hex),
/// 6. build >= minAppVersion (numeric semver),
/// 7. any generation-2 member the record carries (`provider_registry`, `root_index`) is a usable
///    address. Absence is not an error — see [`TrustedAnchor::provider_registry`].
pub fn validate(
    claims: &ConvenienceClaims,
    anchor: &TrustedAnchor,
    ctx: &ClientContext<'_>,
) -> Result<ValidatedVersion, DiscoveryError> {
    // (1) Version coherence: the anchor MUST be the record for the version the platform claimed — else
    // the caller resolved the wrong anchor (or a record was swapped). Tie by BOTH the string and its
    // on-chain keccak key, so neither a mismatched string nor a mismatched `versionId` slips through.
    //
    // This is a CALLER-INTEGRITY guard, NOT the downgrade defense. The caller resolves the anchor BY
    // `claims.protocol_version`, so a platform that merely CLAIMS an older version resolves that
    // version's own legitimate record and coheres with it. The app is version-AGNOSTIC by design (lock
    // C: nothing bundled - it discovers the version), so there is deliberately no `expected_version` to
    // pin against; adding one would contradict the architecture. The version-DOWNGRADE defense is
    // therefore OPERATIONAL, enforced by two levers this function only executes:
    //   - dogtag MUST `deprecateContractSet` a superseded version in the `ProtocolRegistry` - once
    //     `dogtag-levelb/1` is the standard, `dogtag-levela/1` MUST be marked `active=false`, which
    //     check (2) below then refuses;
    //   - `minAppVersion` (check 6), which floors out builds that predate a required change.
    // The `versionId` halves are `0x`-hex, so they compare case-insensitively (an app that formats an
    // `eth_call` bytes32 as uppercase hex must not hard-fail); `protocol_version` is a semantic version
    // STRING, not hex, so it stays an exact compare.
    if claims.protocol_version != anchor.version
        || !version_id(&claims.protocol_version).eq_ignore_ascii_case(&anchor.version_id)
    {
        return Err(DiscoveryError::VersionMismatch {
            claimed: claims.protocol_version.clone(),
            anchor: anchor.version.clone(),
        });
    }

    // (1b) The SAME coherence guard, applied independently to the ARTIFACT axis (R-5). The platform
    // never claims an artifact set — that axis is dogtag-owned and reached through the on-chain binding —
    // so there is nothing to compare it against except itself: the anchor's `artifact_set` string must
    // hash to its `artifact_set_id`. A caller that stitched the two halves from different resolutions
    // (e.g. read the id from chain but the name from a stale manifest) is refused here rather than
    // allowed to fetch a zkey under an identity nobody attested.
    if !version_id(&anchor.artifact_set).eq_ignore_ascii_case(&anchor.artifact_set_id) {
        return Err(DiscoveryError::ArtifactSetIncoherent {
            artifact_set: anchor.artifact_set.clone(),
            artifact_set_id: anchor.artifact_set_id.clone(),
        });
    }

    // (2) Anti-downgrade: a deprecated version is refused before anything else version-specific (§8.4).
    // The two axes are checked SEPARATELY rather than pre-combined by the caller, so each stays a
    // standalone kill switch (R-5) and the error names which one fired. Deprecating EITHER is sufficient
    // to refuse: dogtag can retire a compromised proving-artifact set without touching the trio (or vice
    // versa) and the app still stops. Enforcing the pair HERE, rather than trusting a caller to AND them,
    // is what stops a native implementer who wires only one bit from silently losing the other lever.
    if !anchor.contract_set_active {
        return Err(DiscoveryError::DeprecatedVersion {
            version: anchor.version.clone(),
            axis: DeprecatedAxis::ContractSet,
        });
    }
    if !anchor.artifact_set_active {
        return Err(DiscoveryError::DeprecatedVersion {
            version: anchor.version.clone(),
            axis: DeprecatedAxis::ArtifactSet,
        });
    }

    // (3) chainId: a lying platform cannot point the app at a fork/testnet.
    if claims.chain_id != anchor.chain_id {
        return Err(DiscoveryError::ChainIdMismatch {
            claimed: claims.chain_id,
            anchor: anchor.chain_id,
        });
    }

    // (4) verificationRegistry: THE anti-redirect check. Addresses compared case-insensitively so a
    // checksummed claim vs a lowercase anchor (or vice versa) is not a false mismatch.
    if !claims
        .verification_registry
        .eq_ignore_ascii_case(&anchor.verification_registry)
    {
        return Err(DiscoveryError::RegistryMismatch {
            claimed: claims.verification_registry.clone(),
            anchor: anchor.verification_registry.clone(),
        });
    }

    // (5) purpose: the platform cannot swap the purpose the owner consents to. Exact, case-sensitive
    // compare against the app's OUT-OF-BAND expectation (never the platform's own session data).
    if claims.purpose != ctx.expected_purpose {
        return Err(DiscoveryError::PurposeMismatch {
            claimed: claims.purpose.clone(),
            expected: ctx.expected_purpose.to_string(),
        });
    }

    // (6) minAppVersion (§5.3 step 5): refuse a build older than the anchor's floor. Numeric semver, and
    // fail-closed on any malformed input.
    if compare_semver(
        ctx.app_version,
        "app_version",
        &anchor.min_app_version,
        "min_app_version",
    )? == std::cmp::Ordering::Less
    {
        return Err(DiscoveryError::AppTooOld {
            build: ctx.app_version.to_string(),
            min: anchor.min_app_version.clone(),
        });
    }

    // (7) The generation-2 members, when the resolved record carries them. There is nothing to compare
    // them against — no platform claims a provider registry or a root index, and the artifact axis has no
    // opinion on either — so the only available check is that a value the caller DID report is usable.
    // Absence passes untouched: a generation-1 record has no such member, and refusing that would refuse
    // every currently-published version.
    require_usable_address("providerRegistry", anchor.provider_registry.as_deref())?;
    require_usable_address("rootIndex", anchor.root_index.as_deref())?;

    // PASS — the caller selects the artifact by circuitId (§5.3 step 6) and proceeds.
    Ok(ValidatedVersion {
        version: anchor.version.clone(),
        circuit_id: anchor.circuit_id.clone(),
        chain_id: anchor.chain_id,
        verification_registry: anchor.verification_registry.clone(),
        artifact_set: anchor.artifact_set.clone(),
        provider_registry: anchor.provider_registry.clone(),
        root_index: anchor.root_index.clone(),
    })
}

/// A reported anchor address must be a usable one: `0x` + 40 hex digits, and not the zero address.
///
/// `None` is fine — it means the resolved record has no such member (see [`TrustedAnchor`]). What is
/// refused is a value the caller reported and cannot be acted on, because the caller's next move is to
/// `eth_call` it: the zero address answers empty returndata, which is neither a definite yes nor a
/// definite no, and a truncated or non-hex string is a different address entirely or none at all.
///
/// Case is not normalised — a checksummed and a lowercase address are both accepted, matching the
/// case-insensitive treatment `verification_registry` already gets.
fn require_usable_address(which: &'static str, value: Option<&str>) -> Result<(), DiscoveryError> {
    let Some(addr) = value else { return Ok(()) };
    let bad = || DiscoveryError::MalformedAnchorAddress {
        which,
        value: addr.to_string(),
    };
    let hex = addr
        .strip_prefix("0x")
        .or_else(|| addr.strip_prefix("0X"))
        .ok_or_else(bad)?;
    if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    if hex.bytes().all(|b| b == b'0') {
        return Err(bad());
    }
    Ok(())
}

/// Compare two dotted-numeric semver strings COMPONENT-WISE (numeric, not lexical — so `"1.10.0"` is
/// correctly GREATER than `"1.9.0"`, which a naive string compare gets wrong). Fail-closed on any
/// malformed component so a bad version can never be treated as "new enough".
fn compare_semver(
    a: &str,
    a_which: &'static str,
    b: &str,
    b_which: &'static str,
) -> Result<std::cmp::Ordering, DiscoveryError> {
    let pa = parse_semver(a, a_which)?;
    let pb = parse_semver(b, b_which)?;
    // Fixed-width [major, minor, patch] arrays compare element-by-element — exactly numeric semver order.
    Ok(pa.cmp(&pb))
}

/// Parse a `major.minor.patch` (or shorter, missing components == 0) dotted-numeric version into a
/// `[u64; 3]`. Accepts an optional single leading `v` and a trailing semver prerelease/build suffix
/// (`1.4.0-rc1`, `1.4.0+build.5`) - real mobile builds carry those, and they must not lock an app out of
/// verifying, so only the numeric `major.minor.patch` CORE is compared (prerelease ORDERING is
/// deliberately not modelled: a `-rc` build counts as its release core, which errs toward letting a
/// genuinely new-enough build through rather than gating an unrelated axis on release-candidate
/// semantics). Rejects (fail-closed [`DiscoveryError::BadSemver`]) an empty core, an empty component, a
/// non-numeric component, or more than three components — anything ambiguous is refused rather than
/// guessed.
fn parse_semver(v: &str, which: &'static str) -> Result<[u64; 3], DiscoveryError> {
    let bad = || DiscoveryError::BadSemver {
        which,
        value: v.to_string(),
    };
    let core = v.trim();
    let core = core
        .strip_prefix('v')
        .or_else(|| core.strip_prefix('V'))
        .unwrap_or(core);
    // Drop everything from the first `-` (prerelease) or `+` (build metadata); the numeric core alone is
    // compared. An input that is ONLY a suffix (`-rc1`) leaves an empty core and fails closed below.
    let core = match core.find(['-', '+']) {
        Some(i) => &core[..i],
        None => core,
    };
    if core.is_empty() {
        return Err(bad());
    }
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() > 3 {
        return Err(bad());
    }
    let mut out = [0u64; 3];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() {
            return Err(bad());
        }
        out[i] = p.parse::<u64>().map_err(|_| bad())?;
    }
    Ok(out)
}

/// Thin UniFFI surface so the mobile app's resolve→validate step can call the same pure validator the
/// server uses (§5.3). Takes the two `ClientContext` strings directly (the borrow struct cannot cross
/// the boundary) and flattens [`DiscoveryError`] into the crate's single [`crate::ffi::FfiError`].
#[uniffi::export]
pub fn validate_discovery(
    claims: ConvenienceClaims,
    anchor: TrustedAnchor,
    app_version: String,
    expected_purpose: String,
) -> Result<ValidatedVersion, crate::ffi::FfiError> {
    let ctx = ClientContext {
        app_version: &app_version,
        expected_purpose: &expected_purpose,
    };
    validate(&claims, &anchor, &ctx).map_err(|e| crate::ffi::FfiError::Invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A matched (claims, anchor, ctx) triple built around the frozen Level-B version, so every axis is
    // realistic. Each test perturbs exactly ONE field to isolate the axis under test.
    const VERSION: &str = "dogtag-levelb/1";
    const REGISTRY: &str = "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87";
    const CIRCUIT: &str = "consent.circom/DogTagConsent(6)";
    const PURPOSE: &str = "GROOMING_INTAKE";
    const ARTIFACTS: &str = "dogtag-levelb-artifacts/1";
    // Generation 2's two additional members, for the anchors that carry them.
    const VERSION_V2: &str = "dogtag-levelb/2";
    const PROVIDER_REGISTRY: &str = "0x9309aB1c2D3e4F5061728394a5B6c7D8e9F00112";
    const ROOT_INDEX: &str = "0x120127E4a5B6c7D8E9f001122334455667788990";

    /// A GENERATION-1 anchor: the on-chain record it was resolved from has no provider-authority or
    /// root-index member, so both are `None`. This stays the default fixture because it is what every
    /// currently-published version looks like.
    fn anchor() -> TrustedAnchor {
        TrustedAnchor {
            version: VERSION.to_string(),
            version_id: version_id(VERSION),
            artifact_set: ARTIFACTS.to_string(),
            artifact_set_id: version_id(ARTIFACTS),
            chain_id: 135,
            verification_registry: REGISTRY.to_string(),
            circuit_id: CIRCUIT.to_string(),
            min_app_version: "1.4.0".to_string(),
            contract_set_active: true,
            artifact_set_active: true,
            provider_registry: None,
            root_index: None,
        }
    }

    /// A GENERATION-2 anchor: resolved from a `ProtocolRegistryV2.DiscoverySet`, which always carries
    /// both members because the registry refuses to publish a zero for either.
    fn anchor_v2() -> TrustedAnchor {
        TrustedAnchor {
            version: VERSION_V2.to_string(),
            version_id: version_id(VERSION_V2),
            provider_registry: Some(PROVIDER_REGISTRY.to_string()),
            root_index: Some(ROOT_INDEX.to_string()),
            ..anchor()
        }
    }

    fn claims_v2() -> ConvenienceClaims {
        ConvenienceClaims {
            protocol_version: VERSION_V2.to_string(),
            ..claims()
        }
    }

    fn claims() -> ConvenienceClaims {
        ConvenienceClaims {
            protocol_version: VERSION.to_string(),
            chain_id: 135,
            verification_registry: REGISTRY.to_string(),
            issuer_clone: "0x00000000000000000000000000000000000c10e0".to_string(),
            purpose: PURPOSE.to_string(),
        }
    }

    fn ctx<'a>() -> ClientContext<'a> {
        ClientContext {
            app_version: "1.4.0",
            expected_purpose: PURPOSE,
        }
    }

    /// The happy path: matching claims + a new-enough build validate, and the result carries the version
    /// and its artifact-selection `circuit_id`.
    #[test]
    fn matching_claims_validate() {
        let v = validate(&claims(), &anchor(), &ctx()).expect("matching claims validate");
        assert_eq!(v.version, VERSION);
        assert_eq!(v.circuit_id, CIRCUIT);
        assert_eq!(v.chain_id, 135);
        assert_eq!(v.verification_registry, REGISTRY);
        assert_eq!(
            v.artifact_set, ARTIFACTS,
            "the validated artifact axis is surfaced to the caller"
        );
        // A generation-1 record carries neither generation-2 member, and the validator reports that
        // absence rather than inventing an address.
        assert_eq!(v.provider_registry, None);
        assert_eq!(v.root_index, None);
    }

    /// A GENERATION-2 anchor validates and surfaces both new members to the caller, so a caller reads
    /// `rootIssuer`/`isClone` from the validated root index rather than from a bundled factory address.
    #[test]
    fn a_generation_two_anchor_surfaces_its_authority_core_and_root_index() {
        let v = validate(&claims_v2(), &anchor_v2(), &ctx()).expect("generation 2 validates");
        assert_eq!(v.version, VERSION_V2);
        assert_eq!(v.provider_registry.as_deref(), Some(PROVIDER_REGISTRY));
        assert_eq!(v.root_index.as_deref(), Some(ROOT_INDEX));
        // The frozen artifact axis is unchanged across the generation boundary — the circuit, the
        // ceremony and every pin are generation 1's, so the identity is too. A bumped artifact key here
        // would refuse every already-shipped build with a stitched-anchor error instead of the
        // actionable `AppTooOld`.
        assert_eq!(v.artifact_set, ARTIFACTS);
        assert_eq!(v.circuit_id, CIRCUIT);
    }

    /// A reported member that cannot be acted on FAILS CLOSED. Nothing claims either address, so shape is
    /// the only check available — and the caller's next move is to `eth_call` the value, where the zero
    /// address answers empty returndata: neither a definite yes nor a definite no.
    #[test]
    fn an_unusable_reported_address_fails_closed() {
        for bad in [
            "",
            "0x",
            "0x0000000000000000000000000000000000000000", // the zero address
            "0x9309aB1c2D3e4F5061728394a5B6c7D8e9F0011",  // 39 hex digits
            "0x9309aB1c2D3e4F5061728394a5B6c7D8e9F001122", // 41
            "9309aB1c2D3e4F5061728394a5B6c7D8e9F00112",   // no 0x
            "0x9309aB1c2D3e4F5061728394a5B6c7D8e9F0011z", // not hex
        ] {
            let mut a = anchor_v2();
            a.provider_registry = Some(bad.to_string());
            assert!(
                matches!(
                    validate(&claims_v2(), &a, &ctx()),
                    Err(DiscoveryError::MalformedAnchorAddress {
                        which: "providerRegistry",
                        ..
                    })
                ),
                "providerRegistry {bad:?} must fail closed"
            );

            let mut b = anchor_v2();
            b.root_index = Some(bad.to_string());
            assert!(
                matches!(
                    validate(&claims_v2(), &b, &ctx()),
                    Err(DiscoveryError::MalformedAnchorAddress {
                        which: "rootIndex",
                        ..
                    })
                ),
                "rootIndex {bad:?} must fail closed"
            );
        }

        // ABSENCE is not an error: it is what a generation-1 record honestly reports, and refusing it
        // would refuse every currently-published version.
        assert!(validate(&claims(), &anchor(), &ctx()).is_ok());

        // ...and a checksummed or lowercase address both pass, matching how the registry address is
        // compared. A case-sensitive shape check would reject half of all real callers.
        let mut lower = anchor_v2();
        lower.provider_registry = Some(PROVIDER_REGISTRY.to_lowercase());
        lower.root_index = Some(ROOT_INDEX.to_uppercase().replace("0X", "0x"));
        assert!(validate(&claims_v2(), &lower, &ctx()).is_ok());
    }

    /// The two members are independent: one being unusable must not be masked by the other being fine,
    /// and the error names which one so an implementer knows what they wired wrong.
    #[test]
    fn the_malformed_address_error_names_the_member() {
        let mut a = anchor_v2();
        a.root_index = Some("0x0".to_string());
        let msg = validate(&claims_v2(), &a, &ctx()).unwrap_err().to_string();
        assert!(msg.contains("rootIndex"), "must name the member: {msg}");
        assert!(
            !msg.contains("providerRegistry"),
            "must not blame the healthy member: {msg}"
        );
    }

    /// Refusals that are about the anchor's OWN shape must not preempt the checks that are about the
    /// platform's claims: a lying platform is reported as lying, not as a caller-integrity problem.
    #[test]
    fn a_platform_lie_still_outranks_the_shape_check() {
        let mut a = anchor_v2();
        a.root_index = Some("0x0".to_string()); // also malformed
        let mut c = claims_v2();
        c.verification_registry = "0x000000000000000000000000000000000000dEaD".to_string();
        assert!(
            matches!(
                validate(&c, &a, &ctx()),
                Err(DiscoveryError::RegistryMismatch { .. })
            ),
            "the anti-redirect trip is the diagnosis the operator needs first"
        );
    }

    /// R-5 in the validator: an ARTIFACT rotation (new artifact set, raised app floor) validates on its
    /// own, and every ON-CHAIN field the caller acts on is byte-identical to before the rotation. This is
    /// what "the app is not forced to re-check the trio when a zkey rotates" means in code.
    #[test]
    fn an_artifact_rotation_does_not_disturb_the_onchain_axis() {
        let before = validate(&claims(), &anchor(), &ctx()).expect("baseline validates");

        let mut rotated = anchor();
        rotated.artifact_set = "dogtag-levelb-artifacts/2".to_string();
        rotated.artifact_set_id = version_id("dogtag-levelb-artifacts/2");
        rotated.min_app_version = "1.5.0".to_string();

        let newer = ClientContext {
            app_version: "1.5.0",
            expected_purpose: PURPOSE,
        };
        let after =
            validate(&claims(), &rotated, &newer).expect("the rotated artifact set validates");

        assert_eq!(
            after.artifact_set, "dogtag-levelb-artifacts/2",
            "the artifact axis moved"
        );
        // ...and nothing on the on-chain axis did.
        assert_eq!(after.version, before.version);
        assert_eq!(after.circuit_id, before.circuit_id);
        assert_eq!(after.chain_id, before.chain_id);
        assert_eq!(after.verification_registry, before.verification_registry);
    }

    /// The artifact axis is checked for coherence independently of the contract axis: a stitched anchor
    /// whose `artifact_set` and `artifact_set_id` came from different resolutions FAILS CLOSED, even
    /// though every contract-axis field is perfectly valid.
    #[test]
    fn incoherent_artifact_axis_fails_closed() {
        let mut a = anchor();
        a.artifact_set_id = version_id("dogtag-levelb-artifacts/2"); // id says v2, name still says v1
        match validate(&claims(), &a, &ctx()) {
            Err(DiscoveryError::ArtifactSetIncoherent { artifact_set, .. }) => {
                assert_eq!(artifact_set, ARTIFACTS);
            }
            other => panic!("expected ArtifactSetIncoherent, got {other:?}"),
        }
    }

    /// The two axis ids live in different keyspaces — an anchor can never accidentally use the version
    /// id as its artifact id.
    #[test]
    fn the_two_axis_ids_are_distinct() {
        let a = anchor();
        assert_ne!(a.version_id, a.artifact_set_id);
        assert_ne!(a.version, a.artifact_set);
    }

    /// The registry address may differ only in CASE (checksum vs lowercase) and still validate — a
    /// checksum/lowercase difference is not a redirect.
    #[test]
    fn registry_case_insensitive_still_validates() {
        let mut c = claims();
        c.verification_registry = REGISTRY.to_lowercase();
        assert!(validate(&c, &anchor(), &ctx()).is_ok());
    }

    /// A DIFFERENT claimed registry is the core attack — it FAILS CLOSED (the app never proves against an
    /// attacker registry).
    #[test]
    fn registry_mismatch_fails_closed() {
        let mut c = claims();
        c.verification_registry = "0x000000000000000000000000000000000000dEaD".to_string();
        match validate(&c, &anchor(), &ctx()) {
            Err(DiscoveryError::RegistryMismatch { claimed, anchor }) => {
                assert_eq!(claimed, c.verification_registry);
                assert_eq!(anchor, REGISTRY);
            }
            other => panic!("expected RegistryMismatch, got {other:?}"),
        }
    }

    /// A claimed chainId pointing at another chain FAILS CLOSED.
    #[test]
    fn chain_id_mismatch_fails_closed() {
        let mut c = claims();
        c.chain_id = 1;
        match validate(&c, &anchor(), &ctx()) {
            Err(DiscoveryError::ChainIdMismatch {
                claimed: 1,
                anchor: 135,
            }) => {}
            other => panic!("expected ChainIdMismatch, got {other:?}"),
        }
    }

    /// A claimed purpose that differs from the app's out-of-band expectation FAILS CLOSED — the platform
    /// cannot swap the purpose the owner consents to.
    #[test]
    fn purpose_mismatch_fails_closed() {
        let c = claims(); // claims.purpose == GROOMING_INTAKE
        let ctx = ClientContext {
            app_version: "1.4.0",
            expected_purpose: "AIRLINE_CHECKIN",
        };
        match validate(&c, &anchor(), &ctx) {
            Err(DiscoveryError::PurposeMismatch { claimed, expected }) => {
                assert_eq!(claimed, PURPOSE);
                assert_eq!(expected, "AIRLINE_CHECKIN");
            }
            other => panic!("expected PurposeMismatch, got {other:?}"),
        }
    }

    /// A claimed version that does not match the resolved anchor (wrong record) FAILS CLOSED.
    #[test]
    fn version_mismatch_fails_closed() {
        let mut c = claims();
        c.protocol_version = "dogtag-levela/1".to_string();
        match validate(&c, &anchor(), &ctx()) {
            Err(DiscoveryError::VersionMismatch { claimed, anchor }) => {
                assert_eq!(claimed, "dogtag-levela/1");
                assert_eq!(anchor, VERSION);
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
    }

    /// The anchor's `versionId` may differ only in HEX CASE and still validate — an app that formats an
    /// `eth_call` bytes32 as uppercase hex must not hard-fail every validation (parity with the
    /// case-insensitive registry compare). The version STRING itself stays an exact compare.
    #[test]
    fn version_id_case_insensitive_still_validates() {
        let mut a = anchor();
        a.version_id = version_id(VERSION).to_uppercase();
        assert!(
            validate(&claims(), &a, &ctx()).is_ok(),
            "an uppercase-hex versionId is a formatting artifact, not a mismatch"
        );
    }

    /// A DEPRECATED (inactive) anchor FAILS CLOSED even when every claim matches — the anti-downgrade
    /// lever (§8.4). Both axes are exercised, because R-5 makes each an INDEPENDENT kill switch: the
    /// validator must refuse when EITHER is retired, and must name which one so a native implementer
    /// debugging the abort knows whether the trio or only the artifacts were pulled.
    #[test]
    fn deprecating_either_axis_fails_closed() {
        // The on-chain axis alone — the whole protocol version is retired.
        let mut only_contracts_dead = anchor();
        only_contracts_dead.contract_set_active = false;
        match validate(&claims(), &only_contracts_dead, &ctx()) {
            Err(DiscoveryError::DeprecatedVersion { version, axis }) => {
                assert_eq!(version, VERSION);
                assert_eq!(axis, DeprecatedAxis::ContractSet);
            }
            other => panic!("expected DeprecatedVersion(ContractSet), got {other:?}"),
        }

        // The artifact axis alone — the trio is perfectly live, but a compromised zkey still stops the
        // app. This is the lever a single collapsed `active` bit would have silently discarded.
        let mut only_artifacts_dead = anchor();
        only_artifacts_dead.artifact_set_active = false;
        match validate(&claims(), &only_artifacts_dead, &ctx()) {
            Err(DiscoveryError::DeprecatedVersion { version, axis }) => {
                assert_eq!(version, VERSION);
                assert_eq!(axis, DeprecatedAxis::ArtifactSet);
            }
            other => panic!("expected DeprecatedVersion(ArtifactSet), got {other:?}"),
        }

        // Both live still validates, so neither check is vacuously failing.
        assert!(validate(&claims(), &anchor(), &ctx()).is_ok());
    }

    /// The axis reaches a caller that only ever sees the FLATTENED error string: the FFI wrapper renders
    /// `DiscoveryError` via `to_string()`, so a discriminator absent from the `Display` output would be
    /// invisible to exactly the native implementer it exists to help.
    #[test]
    fn the_deprecated_axis_is_named_in_the_error_message() {
        let mut a = anchor();
        a.artifact_set_active = false;
        let msg = validate(&claims(), &a, &ctx()).unwrap_err().to_string();
        assert!(
            msg.contains("proving-artifact set"),
            "artifact axis must be named: {msg}"
        );

        let mut b = anchor();
        b.contract_set_active = false;
        let msg_b = validate(&claims(), &b, &ctx()).unwrap_err().to_string();
        assert!(
            msg_b.contains("on-chain contract set"),
            "contract axis must be named: {msg_b}"
        );
        assert_ne!(msg, msg_b, "the two axes must not render identically");
    }

    /// A build older than `minAppVersion` FAILS CLOSED (refuse + route to update/owner-web, §5.3 step 5).
    #[test]
    fn app_older_than_min_fails_closed() {
        let ctx = ClientContext {
            app_version: "1.3.9",
            expected_purpose: PURPOSE,
        };
        match validate(&claims(), &anchor(), &ctx) {
            Err(DiscoveryError::AppTooOld { build, min }) => {
                assert_eq!(build, "1.3.9");
                assert_eq!(min, "1.4.0");
            }
            other => panic!("expected AppTooOld, got {other:?}"),
        }
    }

    /// minAppVersion is compared NUMERICALLY, not lexically: `1.10.0` is newer than `1.9.0` (a string
    /// compare would wrongly reject it), and `1.4.0` exactly meets a `1.4.0` floor.
    #[test]
    fn min_app_version_is_numeric_not_lexical() {
        // Build 1.10.0 against floor 1.9.0: numerically newer -> validates (string compare would fail).
        let mut a = anchor();
        a.min_app_version = "1.9.0".to_string();
        let ctx = ClientContext {
            app_version: "1.10.0",
            expected_purpose: PURPOSE,
        };
        assert!(
            validate(&claims(), &a, &ctx).is_ok(),
            "1.10.0 must satisfy a 1.9.0 floor"
        );

        // The reverse is refused: build 1.9.0 against a 1.10.0 floor is too old.
        let mut a2 = anchor();
        a2.min_app_version = "1.10.0".to_string();
        let ctx2 = ClientContext {
            app_version: "1.9.0",
            expected_purpose: PURPOSE,
        };
        assert!(matches!(
            validate(&claims(), &a2, &ctx2),
            Err(DiscoveryError::AppTooOld { .. })
        ));

        // A shorter build string is zero-extended: 1.4 == 1.4.0 meets the floor.
        let ctx3 = ClientContext {
            app_version: "1.4",
            expected_purpose: PURPOSE,
        };
        assert!(
            validate(&claims(), &anchor(), &ctx3).is_ok(),
            "1.4 == 1.4.0 meets the floor"
        );
    }

    /// A real mobile build carries a prerelease/build suffix (`1.4.0-rc1`, `1.4.0+build.5`). Only the
    /// numeric core is compared, so such a build is NOT locked out of verifying, while a core that is
    /// not dotted-numeric still FAILS CLOSED.
    #[test]
    fn prerelease_and_build_metadata_compare_by_numeric_core() {
        for build in ["1.4.0-rc1", "1.4.0+build.5", "v1.4.0-rc.1+exp.sha.5114f85"] {
            let ctx = ClientContext {
                app_version: build,
                expected_purpose: PURPOSE,
            };
            assert!(
                validate(&claims(), &anchor(), &ctx).is_ok(),
                "{build} must satisfy a 1.4.0 floor"
            );
        }

        // The suffix does not rescue a build that is numerically too old.
        let ctx_old = ClientContext {
            app_version: "1.3.9-rc1",
            expected_purpose: PURPOSE,
        };
        assert!(matches!(
            validate(&claims(), &anchor(), &ctx_old),
            Err(DiscoveryError::AppTooOld { .. })
        ));

        // A non-numeric core is still refused, suffix or not.
        let ctx_bad = ClientContext {
            app_version: "1.x.0-rc1",
            expected_purpose: PURPOSE,
        };
        assert!(matches!(
            validate(&claims(), &anchor(), &ctx_bad),
            Err(DiscoveryError::BadSemver {
                which: "app_version",
                ..
            })
        ));

        // A suffix with NO numeric core leaves nothing to compare - fail closed.
        let ctx_empty = ClientContext {
            app_version: "-rc1",
            expected_purpose: PURPOSE,
        };
        assert!(matches!(
            validate(&claims(), &anchor(), &ctx_empty),
            Err(DiscoveryError::BadSemver {
                which: "app_version",
                ..
            })
        ));
    }

    /// A malformed version on EITHER side FAILS CLOSED — a bad semver is never treated as new enough.
    #[test]
    fn malformed_semver_fails_closed() {
        // Malformed BUILD version.
        let ctx_bad = ClientContext {
            app_version: "1.x.0",
            expected_purpose: PURPOSE,
        };
        assert!(matches!(
            validate(&claims(), &anchor(), &ctx_bad),
            Err(DiscoveryError::BadSemver {
                which: "app_version",
                ..
            })
        ));

        // Empty build version.
        let ctx_empty = ClientContext {
            app_version: "",
            expected_purpose: PURPOSE,
        };
        assert!(matches!(
            validate(&claims(), &anchor(), &ctx_empty),
            Err(DiscoveryError::BadSemver {
                which: "app_version",
                ..
            })
        ));

        // Malformed ANCHOR floor.
        let mut a = anchor();
        a.min_app_version = "1..0".to_string();
        assert!(matches!(
            validate(&claims(), &a, &ctx()),
            Err(DiscoveryError::BadSemver {
                which: "min_app_version",
                ..
            })
        ));

        // Too many components.
        let mut a4 = anchor();
        a4.min_app_version = "1.4.0.1".to_string();
        assert!(matches!(
            validate(&claims(), &a4, &ctx()),
            Err(DiscoveryError::BadSemver {
                which: "min_app_version",
                ..
            })
        ));
    }

    /// `version_id` is `keccak256(version)` — the EXACT on-chain `ProtocolRegistry` key, matching
    /// `dogtag_prover::manifest::version_id` (same `cast keccak`-confirmed hexes). This is the
    /// cross-boundary tie the version-coherence check leans on, so it is pinned here too.
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
    }

    /// The convenience tier serializes to the EXACT camelCase wire shape the resolve GET emits (§5.2),
    /// and round-trips — so the app deserializes the same struct the validator consumes.
    #[test]
    fn convenience_claims_serialize_camelcase() {
        let json = serde_json::to_value(claims()).unwrap();
        let obj = json.as_object().unwrap();
        assert_eq!(obj["protocolVersion"], "dogtag-levelb/1");
        assert_eq!(obj["chainId"], 135);
        assert_eq!(obj["verificationRegistry"], REGISTRY);
        assert_eq!(obj["purpose"], PURPOSE);
        assert!(obj.contains_key("issuerClone"));
        // Exactly the five claim fields, nothing extra leaks onto the wire.
        assert_eq!(obj.len(), 5);
        // Round-trip: the app parses back the same claims the validator takes.
        let back: ConvenienceClaims = serde_json::from_value(json).unwrap();
        assert_eq!(back, claims());
    }
}
