//! Discovery anchor-validation — the client TRUST gate (M7 §5.2 / §5.3, brick P4).
//!
//! A platform's non-consuming resolve GET returns a CONVENIENCE tier — [`ConvenienceClaims`]:
//! platform-OWNED `{protocolVersion, chainId, verificationRegistry, issuerClone, purpose}`. Those are
//! CLAIMS, never authority (arch §1.2 trust boundary: the platform backend is NOT trusted for the
//! `{version, registry}` fields). Before the app acts on any platform-supplied version/registry it
//! resolves the dogtag-owned TRUST tier — [`TrustedAnchor`] — from
//! `ProtocolRegistry.getVersion(keccak256(version))` (the on-chain root of truth) or the P3
//! signed-manifest fallback ([`crate`] has no manifest types; they live in the server-only prover crate
//! `dogtag_prover::manifest`, which mirrors the same on-chain `Version`). [`validate`] then REQUIRES the
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
//! The on-chain `Version` deliberately carries NO purpose (purpose is per-verification, not
//! per-version), so [`validate`] compares the claimed purpose to [`ClientContext::expected_purpose`] —
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
/// `ProtocolRegistry.getVersion` (root of truth) or the P3 signed-manifest fallback (both mirror the
/// same on-chain `Version`). These are the AUTHORITATIVE values a platform's [`ConvenienceClaims`] are
/// checked against. Constructed from plain fields so the app can build it from an `eth_call` result or a
/// parsed/reconciled manifest, and the server can map it from `dogtag_prover::manifest` types — keeping
/// this crate free of any prover/manifest dependency.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TrustedAnchor {
    /// The version string this record certifies (e.g. `dogtag-levelb/1`).
    pub version: String,
    /// keccak256(version) `0x`-hex — the on-chain map key; ties the claimed version STRING to the
    /// on-chain `versionId` so a caller cannot silently validate against the wrong record.
    pub version_id: String,
    /// The chain the trio lives on. (Carried by the manifest / known by the app from its RPC endpoint;
    /// the on-chain `Version` struct itself has no chain-id member — the registry IS on a chain.)
    pub chain_id: u64,
    /// The registry a proof is submitted to (trio leg). The anti-redirect anchor.
    pub verification_registry: String,
    /// The circuit this version proves (`<source>/<template>(<params>)`) — the artifact-selection key
    /// (§5.3 step 6; version↔circuit_id is 1:1 in `dogtag_prover::artifact`).
    pub circuit_id: String,
    /// Minimum app build (semver) allowed to use this version — the deprecation lever (§5.3 step 5).
    pub min_app_version: String,
    /// Whether the version is still active at the anchor. A deprecated (`active=false`) version FAILS
    /// CLOSED (the anti-downgrade defense, §8.4). The signed manifest does not carry this lifecycle bit,
    /// so a manifest-only (offline) resolution assumes `true` and the authoritative value is the on-chain
    /// `Version.active` when online — see `anchor_from_manifest` on the server side.
    pub active: bool,
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

/// Every way [`validate`] can FAIL CLOSED. Each variant means "abort — never prove, never submit". Kept
/// as a plain `thiserror` enum (not a UniFFI error): the FFI wrapper flattens it into the crate's single
/// `FfiError`, and the server matches on it directly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    /// The caller resolved an anchor whose version does not match the claimed version (a coherence bug
    /// or a swapped record). Ties by both the string and its keccak `versionId`.
    #[error("claimed version {claimed:?} does not match the resolved anchor version {anchor:?}")]
    VersionMismatch { claimed: String, anchor: String },
    /// The claimed chain id differs from the anchor — the platform cannot point the app at another chain.
    #[error("claimed chainId {claimed} does not match anchor chainId {anchor}")]
    ChainIdMismatch { claimed: u64, anchor: u64 },
    /// The claimed verification registry differs from the anchor — the anti-redirect trip.
    #[error("claimed verificationRegistry {claimed:?} does not match anchor {anchor:?}")]
    RegistryMismatch { claimed: String, anchor: String },
    /// The claimed purpose differs from the app's out-of-band expected purpose.
    #[error("claimed purpose {claimed:?} does not match the app's expected purpose {expected:?}")]
    PurposeMismatch { claimed: String, expected: String },
    /// The anchor version is deprecated (`active=false`) — refused (anti-downgrade, §8.4).
    #[error("version {version:?} is deprecated (inactive) at the anchor")]
    DeprecatedVersion { version: String },
    /// The app build is older than the version's `minAppVersion` — refuse and route to update/owner-web.
    #[error("app build {build:?} is older than minAppVersion {min:?} for this version")]
    AppTooOld { build: String, min: String },
    /// A version string was not a clean dotted-numeric semver. FAIL CLOSED — a malformed version must
    /// never read as "new enough".
    #[error("{which} {value:?} is not a valid dotted-numeric semver")]
    BadSemver { which: &'static str, value: String },
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
}

/// Validate a platform's [`ConvenienceClaims`] against the dogtag-owned [`TrustedAnchor`] and enforce
/// `minAppVersion` (§5.3 steps 4-6). FAIL-CLOSED on every axis: any mismatch returns `Err` and the
/// caller MUST abort. On success the caller may select the artifact by the returned `circuit_id`/`version`
/// and proceed to prove/submit.
///
/// The checks, in order (order affects only which error surfaces first — every one is fail-closed):
/// 1. version coherence (the anchor is the record for the claimed version),
/// 2. not deprecated (`active`),
/// 3. chainId == anchor,
/// 4. verificationRegistry == anchor (case-insensitive — checksum vs lowercase is not a real mismatch),
/// 5. purpose == the app's out-of-band expected purpose (case-SENSITIVE — a purpose is a semantic
///    namespace, not hex),
/// 6. build >= minAppVersion (numeric semver).
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
    //   - dogtag MUST `deprecateVersion` a superseded version in the `ProtocolRegistry` - once
    //     `dogtag-levelb/1` is the standard, `dogtag-levela/1` MUST be marked `active=false`, which
    //     check (2) below then refuses;
    //   - `minAppVersion` (check 6), which floors out builds that predate a required change.
    if claims.protocol_version != anchor.version
        || version_id(&claims.protocol_version) != anchor.version_id
    {
        return Err(DiscoveryError::VersionMismatch {
            claimed: claims.protocol_version.clone(),
            anchor: anchor.version.clone(),
        });
    }

    // (2) Anti-downgrade: a deprecated version is refused before anything else version-specific (§8.4).
    if !anchor.active {
        return Err(DiscoveryError::DeprecatedVersion {
            version: anchor.version.clone(),
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
    if compare_semver(ctx.app_version, "app_version", &anchor.min_app_version, "min_app_version")?
        == std::cmp::Ordering::Less
    {
        return Err(DiscoveryError::AppTooOld {
            build: ctx.app_version.to_string(),
            min: anchor.min_app_version.clone(),
        });
    }

    // PASS — the caller selects the artifact by circuitId (§5.3 step 6) and proceeds.
    Ok(ValidatedVersion {
        version: anchor.version.clone(),
        circuit_id: anchor.circuit_id.clone(),
        chain_id: anchor.chain_id,
        verification_registry: anchor.verification_registry.clone(),
    })
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
    let core = core.strip_prefix('v').or_else(|| core.strip_prefix('V')).unwrap_or(core);
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

    fn anchor() -> TrustedAnchor {
        TrustedAnchor {
            version: VERSION.to_string(),
            version_id: version_id(VERSION),
            chain_id: 135,
            verification_registry: REGISTRY.to_string(),
            circuit_id: CIRCUIT.to_string(),
            min_app_version: "1.4.0".to_string(),
            active: true,
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
        ClientContext { app_version: "1.4.0", expected_purpose: PURPOSE }
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
            Err(DiscoveryError::ChainIdMismatch { claimed: 1, anchor: 135 }) => {}
            other => panic!("expected ChainIdMismatch, got {other:?}"),
        }
    }

    /// A claimed purpose that differs from the app's out-of-band expectation FAILS CLOSED — the platform
    /// cannot swap the purpose the owner consents to.
    #[test]
    fn purpose_mismatch_fails_closed() {
        let c = claims(); // claims.purpose == GROOMING_INTAKE
        let ctx = ClientContext { app_version: "1.4.0", expected_purpose: "AIRLINE_CHECKIN" };
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

    /// A DEPRECATED (inactive) anchor version FAILS CLOSED even when every claim matches — the
    /// anti-downgrade lever (§8.4).
    #[test]
    fn deprecated_version_fails_closed() {
        let mut a = anchor();
        a.active = false;
        match validate(&claims(), &a, &ctx()) {
            Err(DiscoveryError::DeprecatedVersion { version }) => assert_eq!(version, VERSION),
            other => panic!("expected DeprecatedVersion, got {other:?}"),
        }
    }

    /// A build older than `minAppVersion` FAILS CLOSED (refuse + route to update/owner-web, §5.3 step 5).
    #[test]
    fn app_older_than_min_fails_closed() {
        let ctx = ClientContext { app_version: "1.3.9", expected_purpose: PURPOSE };
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
        let ctx = ClientContext { app_version: "1.10.0", expected_purpose: PURPOSE };
        assert!(validate(&claims(), &a, &ctx).is_ok(), "1.10.0 must satisfy a 1.9.0 floor");

        // The reverse is refused: build 1.9.0 against a 1.10.0 floor is too old.
        let mut a2 = anchor();
        a2.min_app_version = "1.10.0".to_string();
        let ctx2 = ClientContext { app_version: "1.9.0", expected_purpose: PURPOSE };
        assert!(matches!(
            validate(&claims(), &a2, &ctx2),
            Err(DiscoveryError::AppTooOld { .. })
        ));

        // A shorter build string is zero-extended: 1.4 == 1.4.0 meets the floor.
        let ctx3 = ClientContext { app_version: "1.4", expected_purpose: PURPOSE };
        assert!(validate(&claims(), &anchor(), &ctx3).is_ok(), "1.4 == 1.4.0 meets the floor");
    }

    /// A real mobile build carries a prerelease/build suffix (`1.4.0-rc1`, `1.4.0+build.5`). Only the
    /// numeric core is compared, so such a build is NOT locked out of verifying, while a core that is
    /// not dotted-numeric still FAILS CLOSED.
    #[test]
    fn prerelease_and_build_metadata_compare_by_numeric_core() {
        for build in ["1.4.0-rc1", "1.4.0+build.5", "v1.4.0-rc.1+exp.sha.5114f85"] {
            let ctx = ClientContext { app_version: build, expected_purpose: PURPOSE };
            assert!(
                validate(&claims(), &anchor(), &ctx).is_ok(),
                "{build} must satisfy a 1.4.0 floor"
            );
        }

        // The suffix does not rescue a build that is numerically too old.
        let ctx_old = ClientContext { app_version: "1.3.9-rc1", expected_purpose: PURPOSE };
        assert!(matches!(
            validate(&claims(), &anchor(), &ctx_old),
            Err(DiscoveryError::AppTooOld { .. })
        ));

        // A non-numeric core is still refused, suffix or not.
        let ctx_bad = ClientContext { app_version: "1.x.0-rc1", expected_purpose: PURPOSE };
        assert!(matches!(
            validate(&claims(), &anchor(), &ctx_bad),
            Err(DiscoveryError::BadSemver { which: "app_version", .. })
        ));

        // A suffix with NO numeric core leaves nothing to compare - fail closed.
        let ctx_empty = ClientContext { app_version: "-rc1", expected_purpose: PURPOSE };
        assert!(matches!(
            validate(&claims(), &anchor(), &ctx_empty),
            Err(DiscoveryError::BadSemver { which: "app_version", .. })
        ));
    }

    /// A malformed version on EITHER side FAILS CLOSED — a bad semver is never treated as new enough.
    #[test]
    fn malformed_semver_fails_closed() {
        // Malformed BUILD version.
        let ctx_bad = ClientContext { app_version: "1.x.0", expected_purpose: PURPOSE };
        assert!(matches!(
            validate(&claims(), &anchor(), &ctx_bad),
            Err(DiscoveryError::BadSemver { which: "app_version", .. })
        ));

        // Empty build version.
        let ctx_empty = ClientContext { app_version: "", expected_purpose: PURPOSE };
        assert!(matches!(
            validate(&claims(), &anchor(), &ctx_empty),
            Err(DiscoveryError::BadSemver { which: "app_version", .. })
        ));

        // Malformed ANCHOR floor.
        let mut a = anchor();
        a.min_app_version = "1..0".to_string();
        assert!(matches!(
            validate(&claims(), &a, &ctx()),
            Err(DiscoveryError::BadSemver { which: "min_app_version", .. })
        ));

        // Too many components.
        let mut a4 = anchor();
        a4.min_app_version = "1.4.0.1".to_string();
        assert!(matches!(
            validate(&claims(), &a4, &ctx()),
            Err(DiscoveryError::BadSemver { which: "min_app_version", .. })
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
