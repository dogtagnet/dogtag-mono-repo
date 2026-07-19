//! Anchor-validation integration: the P3 signed-manifest / on-chain trust tier FEEDS the P4 validator
//! (M7 §5.3).
//!
//! The pure validator lives in the standard crate (`dogtag_standard::discovery`) and is unit-tested
//! there for every mismatch axis. THIS test proves the other half of §5.3: that the trust tier the app
//! actually resolves — a dogtag-signed manifest (§5.2 1B) reconciled against the on-chain record (1C) —
//! maps cleanly into the validator's [`TrustedAnchor`] and drives it end-to-end. It is the
//! "signed-manifest fallback path" the acceptance calls for, exercised through the REAL P3 builder
//! (`dogtag_prover::manifest::build`) and the vet-api mapping seam (`vet_api::discovery`).

use dogtag_prover::manifest::{self, version_id, OnchainVersion, SignedManifest};
use dogtag_standard::discovery::{validate, ClientContext, ConvenienceClaims, DiscoveryError};
use ed25519_dalek::SigningKey;
use vet_api::discovery::{anchor_from_manifest, anchor_from_reconciliation};

const VERSION: &str = "dogtag-levelb/1";
const PURPOSE: &str = "GROOMING_INTAKE";

/// A deterministic dogtag signing key for the test (matches the P3 manifest tests' convention).
fn test_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// The convenience-tier claims a HONEST platform on this version would emit — every trust-critical field
/// mirrors the manifest, `issuer_clone`/`purpose` are the flow's own.
fn honest_claims(m: &manifest::Manifest) -> ConvenienceClaims {
    ConvenienceClaims {
        protocol_version: m.version.clone(),
        chain_id: m.chain_id,
        verification_registry: m.verification_registry.clone(),
        issuer_clone: "0x00000000000000000000000000000000000c10e0".to_string(),
        purpose: PURPOSE.to_string(),
    }
}

/// Build the on-chain record that AGREES with a manifest, so `reconcile` finds no conflict. Mirrors the
/// P3 manifest test's `onchain_from`: on-chain `circuitId` is `keccak256(circuit-string)`, which the
/// public `version_id` helper computes (it is keccak-hex of any string).
fn agreeing_onchain(m: &manifest::Manifest) -> OnchainVersion {
    OnchainVersion {
        version_id: m.version_id.clone(),
        factory: m.factory.clone(),
        verification_registry: m.verification_registry.clone(),
        sbt: m.sbt.clone(),
        verifier: m.verifier.clone(),
        circuit_id: version_id(&m.circuit_id),
        zkey_sha256: m.zkey_sha256.clone(),
        witness_mobile_sha256: m.witness_mobile_sha256.clone(),
        witness_server_r1cs_sha256: m.witness_server_r1cs_sha256.clone(),
        witness_server_wasm_sha256: m.witness_server_wasm_sha256.clone(),
        artifact_base_url: m.artifact_base_url.clone(),
        min_app_version: m.min_app_version.clone(),
        active: true,
    }
}

/// The signed-manifest fallback path: a P3-built manifest maps to a `TrustedAnchor` that VALIDATES an
/// honest platform's claims, and the result selects the version's artifact by `circuit_id`.
#[test]
fn manifest_trust_tier_validates_honest_claims() {
    let m = manifest::build(VERSION).expect("Level-B is a known version");
    let anchor = anchor_from_manifest(&m, true);
    let claims = honest_claims(&m);
    let ctx = ClientContext { app_version: &m.min_app_version, expected_purpose: PURPOSE };

    let v = validate(&claims, &anchor, &ctx).expect("honest claims validate against the manifest anchor");
    assert_eq!(v.version, VERSION);
    assert_eq!(v.circuit_id, m.circuit_id, "the validated artifact-selection key is the version's circuit");
    assert_eq!(v.verification_registry, m.verification_registry);
    assert_eq!(v.chain_id, m.chain_id);
}

/// Each trust-critical axis FAILS CLOSED against the manifest anchor — a lying platform cannot redirect
/// the registry/chain nor swap the purpose the owner consents to.
#[test]
fn manifest_trust_tier_fails_closed_on_each_mismatch() {
    let m = manifest::build(VERSION).unwrap();
    let anchor = anchor_from_manifest(&m, true);
    let ctx = ClientContext { app_version: &m.min_app_version, expected_purpose: PURPOSE };

    // Registry redirect.
    let mut c = honest_claims(&m);
    c.verification_registry = "0x000000000000000000000000000000000000dEaD".to_string();
    assert!(matches!(
        validate(&c, &anchor, &ctx),
        Err(DiscoveryError::RegistryMismatch { .. })
    ));

    // Wrong chain.
    let mut c = honest_claims(&m);
    c.chain_id = 1;
    assert!(matches!(
        validate(&c, &anchor, &ctx),
        Err(DiscoveryError::ChainIdMismatch { .. })
    ));

    // Purpose swap: the platform claims a different purpose than the app's out-of-band intent.
    let c = honest_claims(&m); // claims.purpose == GROOMING_INTAKE
    let ctx_other = ClientContext { app_version: &m.min_app_version, expected_purpose: "AIRLINE_CHECKIN" };
    assert!(matches!(
        validate(&c, &anchor, &ctx_other),
        Err(DiscoveryError::PurposeMismatch { .. })
    ));
}

/// The app-deprecation lever (§5.3 step 5): a build older than the version's `minAppVersion` is refused
/// even against a valid manifest anchor.
#[test]
fn manifest_trust_tier_enforces_min_app_version() {
    let m = manifest::build(VERSION).unwrap();
    // Level-B's floor is 1.4.0; an older build must be refused.
    assert_eq!(m.min_app_version, "1.4.0");
    let anchor = anchor_from_manifest(&m, true);
    let claims = honest_claims(&m);
    let ctx = ClientContext { app_version: "1.3.0", expected_purpose: PURPOSE };
    assert!(matches!(
        validate(&claims, &anchor, &ctx),
        Err(DiscoveryError::AppTooOld { .. })
    ));
}

/// A DEPRECATED version (on-chain `active=false`) fails closed through the mapping even when the manifest
/// itself carries no lifecycle bit — the anti-downgrade defense (§8.4).
#[test]
fn deprecated_onchain_version_fails_closed() {
    let m = manifest::build(VERSION).unwrap();
    let anchor = anchor_from_manifest(&m, false); // the online caller passes the on-chain active=false
    let claims = honest_claims(&m);
    let ctx = ClientContext { app_version: &m.min_app_version, expected_purpose: PURPOSE };
    assert!(matches!(
        validate(&claims, &anchor, &ctx),
        Err(DiscoveryError::DeprecatedVersion { .. })
    ));
}

/// On-chain precedence (1C over 1B): a manifest that AGREES with the on-chain record reconciles clean and
/// its anchor validates; a manifest that DISAGREES is rejected by `anchor_from_reconciliation` and never
/// reaches the validator, so a stale/compromised manifest cannot slip a different registry past the gate.
#[test]
fn reconciled_anchor_enforces_onchain_precedence() {
    let key = test_key();
    let m = manifest::build(VERSION).unwrap();
    let signed: SignedManifest = manifest::sign(&m, &key);

    // Agreeing on-chain record -> reconcile clean -> anchor validates honest claims.
    let onchain = agreeing_onchain(&m);
    let recon = manifest::reconcile(&signed, &key.verifying_key(), &onchain).unwrap();
    let anchor = anchor_from_reconciliation(&recon).expect("agreeing manifest yields an anchor");
    let claims = honest_claims(&m);
    let ctx = ClientContext { app_version: &m.min_app_version, expected_purpose: PURPOSE };
    assert!(validate(&claims, &anchor, &ctx).is_ok());

    // Disagreeing on-chain record (the chain's registry differs) -> reconcile reports a conflict ->
    // the mapping REFUSES to build an anchor (on-chain wins), so the validator is never even reached.
    let mut liar = agreeing_onchain(&m);
    liar.verification_registry = "0x1111111111111111111111111111111111111111".to_string();
    let recon_bad = manifest::reconcile(&signed, &key.verifying_key(), &liar).unwrap();
    match anchor_from_reconciliation(&recon_bad) {
        Err(conflicts) => {
            assert!(
                conflicts.iter().any(|c| c.field == "verification_registry"),
                "the registry conflict must be surfaced, got {conflicts:?}"
            );
        }
        Ok(_) => panic!("a manifest disagreeing with on-chain must NOT yield a trusted anchor"),
    }
}

/// The anti-downgrade defense is WIRED, not merely documented: an on-chain record whose `active=false`
/// (what `ProtocolRegistry.deprecateVersion` produces) flows through `reconcile` ->
/// `anchor_from_reconciliation` -> a `TrustedAnchor` with `active=false`, and the validator then fails
/// closed with `DeprecatedVersion` — even though the signed manifest itself carries no lifecycle bit and
/// agrees on every field it DOES carry. This is what makes "deprecate `dogtag-levela/1` at the M7 cutover"
/// an enforceable lever rather than a caller-supplied guess.
#[test]
fn onchain_deprecation_flows_through_reconciliation_and_fails_closed() {
    let key = test_key();
    let m = manifest::build(VERSION).unwrap();
    let signed: SignedManifest = manifest::sign(&m, &key);

    let mut deprecated = agreeing_onchain(&m);
    deprecated.active = false;

    let recon = manifest::reconcile(&signed, &key.verifying_key(), &deprecated).unwrap();
    assert!(
        recon.manifest_agrees(),
        "deprecation is a lifecycle bit the manifest does not attest, so it must NOT read as a conflict"
    );

    let anchor = anchor_from_reconciliation(&recon).expect("an agreeing manifest still yields an anchor");
    assert!(!anchor.active, "the anchor must inherit the on-chain lifecycle bit, not assume active");

    let claims = honest_claims(&m);
    let ctx = ClientContext { app_version: &m.min_app_version, expected_purpose: PURPOSE };
    assert!(
        matches!(validate(&claims, &anchor, &ctx), Err(DiscoveryError::DeprecatedVersion { .. })),
        "a chain-deprecated version must be refused even when every other axis is honest"
    );
}
