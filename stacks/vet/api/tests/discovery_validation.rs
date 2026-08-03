//! Anchor-validation integration: the P3 signed-manifest / on-chain trust tier FEEDS the P4 validator
//! (M7 §5.3).
//!
//! The pure validator lives in the standard crate (`dogtag_standard::discovery`) and is unit-tested
//! there for every mismatch axis. THIS test proves the other half of §5.3: that the trust tier the app
//! actually resolves — a dogtag-signed manifest (§5.2 1B) reconciled against the on-chain record (1C) —
//! maps cleanly into the validator's [`TrustedAnchor`] and drives it end-to-end. It is the
//! "signed-manifest fallback path" the acceptance calls for, exercised through the REAL P3 builder
//! (`dogtag_prover::manifest::build`) and the vet-api mapping seam (`vet_api::discovery`).

use dogtag_prover::manifest::{self, version_id, OnchainArtifactSet, OnchainContractSet, SignedManifest};
use dogtag_standard::discovery::{
    validate, ClientContext, ConvenienceClaims, DeprecatedAxis, DiscoveryError,
};
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

/// Build the on-chain records that AGREE with a manifest, so `reconcile` finds no conflict. Mirrors the
/// P3 manifest tests' `contracts_from`/`artifacts_from`: on-chain `circuitId` is
/// `keccak256(circuit-string)`, which the public `version_id` helper computes (it is keccak-hex of any
/// string).
///
/// There are TWO of these because the registry keys the on-chain contract set and the off-chain proving
/// artifacts on two independent axes (R-5), and `reconcile` takes them separately.
fn agreeing_contracts(m: &manifest::Manifest) -> OnchainContractSet {
    OnchainContractSet {
        version_id: m.version_id.clone(),
        factory: m.factory.clone(),
        verification_registry: m.verification_registry.clone(),
        sbt: m.sbt.clone(),
        verifier: m.verifier.clone(),
        // Mirrored from the manifest so an agreeing record stays agreeing on both members. For
        // `dogtag-levelb/1` both are `None`: generation 1's on-chain `ContractSet` has no such member,
        // and back-filling one here would build a record no generation-1 registry could ever return.
        provider_registry: m.provider_registry.clone(),
        root_index: m.root_index.clone(),
        circuit_id: version_id(&m.circuit_id),
        active: true,
    }
}

/// A SYNTHETIC on-chain record. `dogtag_prover` ships none - a deployment mirrors chain state and is
/// the caller's configuration - and every case below is about the reconcile/validation RULES, which
/// do not depend on which addresses the members hold. Distinct per member so a field-order mistake
/// cannot pass on two slots sharing a value.
fn test_deployment() -> manifest::VersionDeployment {
    manifest::VersionDeployment {
        chain_id: 135,
        factory: "0x1C9Ac2eB3f1A2D4B5C6d7E8f90A1B2C3D4e5F607".to_string(),
        verification_registry: "0x2B4d6f8a0c1e3a5b7d9f0e2C4a6b8d0F1E3A5c70".to_string(),
        sbt: "0x3c5e7A9b0D2F4a6c8E0b1d3F5A7c9e0B2D4F6a80".to_string(),
        verifier: "0x4d6F8B0C2E4A6b8d0F2c4e6a8b0D2F4c6e8A0b90".to_string(),
        provider_registry: None,
        root_index: None,
    }
}

fn agreeing_artifacts(m: &manifest::Manifest) -> OnchainArtifactSet {
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

/// The signed-manifest fallback path: a P3-built manifest maps to a `TrustedAnchor` that VALIDATES an
/// honest platform's claims, and the result selects the version's artifact by `circuit_id`.
#[test]
fn manifest_trust_tier_validates_honest_claims() {
    let m = manifest::build(VERSION, &test_deployment()).expect("Level-B is a known version");
    let anchor = anchor_from_manifest(&m, true, true);
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
    let m = manifest::build(VERSION, &test_deployment()).unwrap();
    let anchor = anchor_from_manifest(&m, true, true);
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
    let m = manifest::build(VERSION, &test_deployment()).unwrap();
    // Level-B's floor is 1.4.0; an older build must be refused.
    assert_eq!(m.min_app_version, "1.4.0");
    let anchor = anchor_from_manifest(&m, true, true);
    let claims = honest_claims(&m);
    let ctx = ClientContext { app_version: "1.3.0", expected_purpose: PURPOSE };
    assert!(matches!(
        validate(&claims, &anchor, &ctx),
        Err(DiscoveryError::AppTooOld { .. })
    ));
}

/// A DEPRECATED version (the on-chain contract set's `active=false`) fails closed through the mapping
/// even when the manifest itself carries no lifecycle bit — the anti-downgrade defense (§8.4).
#[test]
fn deprecated_onchain_version_fails_closed() {
    let m = manifest::build(VERSION, &test_deployment()).unwrap();
    // The online caller passes the on-chain bits; here the contract set is retired, the artifacts are not.
    let anchor = anchor_from_manifest(&m, false, true);
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
    let m = manifest::build(VERSION, &test_deployment()).unwrap();
    let signed: SignedManifest = manifest::sign(&m, &key);

    // Agreeing on-chain record -> reconcile clean -> anchor validates honest claims.
    let recon = manifest::reconcile(
        &signed,
        &key.verifying_key(),
        &agreeing_contracts(&m),
        &agreeing_artifacts(&m),
    )
    .unwrap();
    let anchor = anchor_from_reconciliation(&recon).expect("agreeing manifest yields an anchor");
    let claims = honest_claims(&m);
    let ctx = ClientContext { app_version: &m.min_app_version, expected_purpose: PURPOSE };
    assert!(validate(&claims, &anchor, &ctx).is_ok());

    // Disagreeing on-chain record (the chain's registry differs) -> reconcile reports a conflict ->
    // the mapping REFUSES to build an anchor (on-chain wins), so the validator is never even reached.
    let mut liar = agreeing_contracts(&m);
    liar.verification_registry = "0x1111111111111111111111111111111111111111".to_string();
    let recon_bad =
        manifest::reconcile(&signed, &key.verifying_key(), &liar, &agreeing_artifacts(&m)).unwrap();
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
/// (what `ProtocolRegistry.deprecateContractSet` produces) flows through `reconcile` ->
/// `anchor_from_reconciliation` -> a `TrustedAnchor` with `active=false`, and the validator then fails
/// closed with `DeprecatedVersion` — even though the signed manifest itself carries no lifecycle bit and
/// agrees on every field it DOES carry. This is what makes "deprecate `dogtag-levela/1` at the M7 cutover"
/// an enforceable lever rather than a caller-supplied guess.
#[test]
fn onchain_deprecation_flows_through_reconciliation_and_fails_closed() {
    let key = test_key();
    let m = manifest::build(VERSION, &test_deployment()).unwrap();
    let signed: SignedManifest = manifest::sign(&m, &key);

    let mut deprecated = agreeing_contracts(&m);
    deprecated.active = false;

    let recon =
        manifest::reconcile(&signed, &key.verifying_key(), &deprecated, &agreeing_artifacts(&m)).unwrap();
    assert!(
        recon.manifest_agrees(),
        "deprecation is a lifecycle bit the manifest does not attest, so it must NOT read as a conflict"
    );

    let anchor = anchor_from_reconciliation(&recon).expect("an agreeing manifest still yields an anchor");
    assert!(
        !anchor.contract_set_active,
        "the anchor must inherit the on-chain lifecycle bit, not assume active"
    );
    assert!(anchor.artifact_set_active, "only the contract axis was deprecated on-chain");

    let claims = honest_claims(&m);
    let ctx = ClientContext { app_version: &m.min_app_version, expected_purpose: PURPOSE };
    assert!(
        matches!(validate(&claims, &anchor, &ctx), Err(DiscoveryError::DeprecatedVersion { .. })),
        "a chain-deprecated version must be refused even when every other axis is honest"
    );
}

/// R-5 at the SERVER SEAM: rotating the bound proving-artifact set changes only the artifact axis of the
/// resolved anchor. The manifest still agrees on every contract-axis field, the anchor still validates
/// the same honest claims, and the trio/verifier/circuit the caller acts on are byte-identical.
///
/// This is the end-to-end statement of "a proving-artifact rotation no longer forces an on-chain
/// redeploy": nothing the app checks about the CHAIN moves when the zkey does.
#[test]
fn an_artifact_rotation_leaves_the_onchain_axis_of_the_anchor_untouched() {
    let m = manifest::build(VERSION, &test_deployment()).unwrap();
    let before = anchor_from_manifest(&m, true, true);

    // The chain has rotated the binding: a new artifact set with new pins, a new host and a new floor.
    // The manifest served alongside it describes the SAME contract set.
    let mut rotated = m.clone();
    rotated.artifact_set = "dogtag-levelb-artifacts/2".to_string();
    rotated.artifact_set_id = version_id("dogtag-levelb-artifacts/2");
    rotated.zkey_sha256 = "0xfeedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedface".to_string();
    rotated.artifact_base_url = "https://artifacts.dogtag.io/levelb2".to_string();
    rotated.min_app_version = "1.5.0".to_string();
    let after = anchor_from_manifest(&rotated, true, true);

    // The artifact axis moved...
    assert_ne!(after.artifact_set, before.artifact_set);
    assert_ne!(after.artifact_set_id, before.artifact_set_id);
    assert_ne!(after.min_app_version, before.min_app_version);

    // ...and every on-chain-axis field the anchor carries did not.
    assert_eq!(after.version, before.version);
    assert_eq!(after.version_id, before.version_id);
    assert_eq!(after.chain_id, before.chain_id);
    assert_eq!(after.verification_registry, before.verification_registry);
    assert_eq!(after.circuit_id, before.circuit_id);

    // And the rotated anchor still validates the same honest platform claims (with a new-enough build).
    let claims = honest_claims(&m);
    let ctx = ClientContext { app_version: "1.5.0", expected_purpose: PURPOSE };
    let v = validate(&claims, &after, &ctx).expect("the rotated artifact set still validates");
    assert_eq!(v.artifact_set, "dogtag-levelb-artifacts/2");
    assert_eq!(v.verification_registry, m.verification_registry, "no on-chain address moved");
}

/// The mirror: deprecating EITHER axis on-chain is enough to fail closed. The seam carries the two
/// `active` bits SEPARATELY and the validator requires both, so each axis stays an independent safety
/// lever — dogtag can retire a compromised artifact set without touching the trio (or the reverse), and
/// the app still refuses. The refusal also NAMES the axis, so an operator reading the abort knows
/// whether the trio moved or only the proving artifacts were pulled.
#[test]
fn deprecating_either_axis_fails_closed() {
    let key = test_key();
    let m = manifest::build(VERSION, &test_deployment()).unwrap();
    let signed: SignedManifest = manifest::sign(&m, &key);
    let claims = honest_claims(&m);
    let ctx = ClientContext { app_version: &m.min_app_version, expected_purpose: PURPOSE };

    // Only the ARTIFACT set is deprecated; the contract set is perfectly live.
    let mut dead_artifacts = agreeing_artifacts(&m);
    dead_artifacts.active = false;
    let recon =
        manifest::reconcile(&signed, &key.verifying_key(), &agreeing_contracts(&m), &dead_artifacts).unwrap();
    assert!(recon.manifest_agrees(), "a lifecycle bit is not a field conflict");
    let anchor = anchor_from_reconciliation(&recon).unwrap();
    assert!(!anchor.artifact_set_active, "the retired artifact axis must reach the anchor");
    assert!(anchor.contract_set_active, "...without disturbing the live contract axis");
    assert!(matches!(
        validate(&claims, &anchor, &ctx),
        Err(DiscoveryError::DeprecatedVersion { axis: DeprecatedAxis::ArtifactSet, .. })
    ));

    // The symmetric case: only the CONTRACT set is deprecated; the artifacts are perfectly live.
    let mut dead_contracts = agreeing_contracts(&m);
    dead_contracts.active = false;
    let recon_c =
        manifest::reconcile(&signed, &key.verifying_key(), &dead_contracts, &agreeing_artifacts(&m)).unwrap();
    let anchor_c = anchor_from_reconciliation(&recon_c).unwrap();
    assert!(!anchor_c.contract_set_active, "the retired contract axis must reach the anchor");
    assert!(anchor_c.artifact_set_active, "...without disturbing the live artifact axis");
    assert!(matches!(
        validate(&claims, &anchor_c, &ctx),
        Err(DiscoveryError::DeprecatedVersion { axis: DeprecatedAxis::ContractSet, .. })
    ));

    // Both live -> validates, confirming neither check is vacuously failing.
    let recon_ok = manifest::reconcile(
        &signed,
        &key.verifying_key(),
        &agreeing_contracts(&m),
        &agreeing_artifacts(&m),
    )
    .unwrap();
    let anchor_ok = anchor_from_reconciliation(&recon_ok).unwrap();
    assert!(anchor_ok.contract_set_active && anchor_ok.artifact_set_active);
    assert!(validate(&claims, &anchor_ok, &ctx).is_ok());
}

/// The generation-2 members travel the WHOLE seam: manifest -> reconcile -> `TrustedAnchor` -> `validate`
/// -> `ValidatedVersion`. A caller therefore reads `rootIssuer`/`isClone` from a root index the chain
/// attested, rather than from an address baked into its own bundle — which is the specific failure mode
/// that makes an un-updated client render a genuine generation-2 credential as unverified.
#[test]
fn the_generation_two_members_reach_the_validated_version() {
    const PROVIDER_REGISTRY: &str = "0x9309aB1c2D3e4F5061728394a5B6c7D8e9F00112";
    const ROOT_INDEX: &str = "0x120127E4a5B6c7D8E9f001122334455667788990";

    let key = test_key();
    // A generation-2 version's manifest: same frozen artifacts, plus the two new addresses.
    let mut m = manifest::build(VERSION, &test_deployment()).unwrap();
    m.provider_registry = Some(PROVIDER_REGISTRY.to_string());
    m.root_index = Some(ROOT_INDEX.to_string());
    let signed: SignedManifest = manifest::sign(&m, &key);

    let recon = manifest::reconcile(
        &signed,
        &key.verifying_key(),
        &agreeing_contracts(&m),
        &agreeing_artifacts(&m),
    )
    .unwrap();
    assert!(recon.manifest_agrees(), "conflicts: {:?}", recon.conflicts);

    let anchor = anchor_from_reconciliation(&recon).unwrap();
    assert_eq!(anchor.provider_registry.as_deref(), Some(PROVIDER_REGISTRY));
    assert_eq!(anchor.root_index.as_deref(), Some(ROOT_INDEX));
    assert_ne!(
        anchor.root_index.as_deref(),
        Some(m.factory.as_str()),
        "the root index is the provenance router, NOT the factory"
    );

    let ctx = ClientContext { app_version: &m.min_app_version, expected_purpose: PURPOSE };
    let v = validate(&honest_claims(&m), &anchor, &ctx).expect("a generation-2 anchor validates");
    assert_eq!(v.provider_registry.as_deref(), Some(PROVIDER_REGISTRY));
    assert_eq!(v.root_index.as_deref(), Some(ROOT_INDEX));
}

/// On-chain precedence extends to the two new members: a manifest naming a different root index than the
/// chain does is a CONFLICT, so `anchor_from_reconciliation` refuses to build an anchor at all rather than
/// letting a stale manifest steer a caller onto a provenance index the chain never named — which would
/// resolve a different set of historical roots and silently change which credentials verify.
#[test]
fn a_stale_manifest_cannot_steer_the_root_index() {
    let key = test_key();
    let mut m = manifest::build(VERSION, &test_deployment()).unwrap();
    m.provider_registry = Some("0x9309aB1c2D3e4F5061728394a5B6c7D8e9F00112".to_string());
    m.root_index = Some("0x120127E4a5B6c7D8E9f001122334455667788990".to_string());
    let signed: SignedManifest = manifest::sign(&m, &key);

    let mut chain = agreeing_contracts(&m);
    chain.root_index = Some("0x00000000000000000000000000000000dead0000".to_string());

    let recon =
        manifest::reconcile(&signed, &key.verifying_key(), &chain, &agreeing_artifacts(&m)).unwrap();
    assert!(!recon.manifest_agrees());
    let conflicts = anchor_from_reconciliation(&recon).expect_err("a disagreeing manifest builds no anchor");
    assert!(
        conflicts.iter().any(|c| c.field == "root_index"),
        "the disagreement must be named: {conflicts:?}"
    );
    // The reconciled record itself still holds the CHAIN's value, so a caller that inspects it sees the
    // authoritative address rather than the manifest's claim.
    assert_eq!(
        recon.contract_set.root_index.as_deref(),
        Some("0x00000000000000000000000000000000dead0000")
    );
}
