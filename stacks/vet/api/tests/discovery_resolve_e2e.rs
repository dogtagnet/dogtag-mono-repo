//! END-TO-END discovery: the platform's resolve GET emits the CONVENIENCE tier, and the app's TRUST
//! gate validates it against the dogtag anchor (M7 §5.2 / §5.3, brick P4).
//!
//! `discovery_validation.rs` covers the trust-tier MAPPING (manifest/reconcile -> `TrustedAnchor`) with
//! hand-built claims, and `dogtag_standard::discovery`'s unit tests cover each validator axis in
//! isolation. Neither exercises the actual seam: the JSON a REAL resolve GET puts on the wire being
//! deserialized and fed to the validator. THIS test walks the whole end-user path against the real Axum
//! router (MemChain + MemStore, no node):
//!
//!   operator starts a verify session -> device scans the QR -> `GET /x/<token>` -> parse
//!   `unverifiedClaims` -> resolve the dogtag TRUST anchor -> `validate_discovery` (the FFI entry the
//!   mobile app calls) -> PASS, then a LYING platform's tampered claims -> REFUSED on every axis.
//!
//! Both `/p/` (issuance) and `/x/` (verify) surfaces are covered.
//!
//! The anchor is built from `dogtag_prover::manifest::build` — an INDEPENDENT source of truth (the
//! file-verified artifact descriptors + the recorded on-chain deployment), never an echo of the claims —
//! so a PASS means the platform's claims genuinely agree with dogtag's own record.

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;
use vet_api::chain::MemChain;
use vet_api::verify::verify_key;

use dogtag_standard::discovery::{
    validate, validate_discovery, ClientContext, ConvenienceClaims, DiscoveryError, TrustedAnchor,
};
use vet_api::discovery::anchor_from_manifest;

const ISSUER_REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const VACCINATION_ISSUER: &str = "0x00000000000000000000000000000000000000bb";
/// The purpose the operator opens the session for — and, out-of-band, what the owner's app expects.
const PURPOSE: &str = "GROOMING_INTAKE";
const VERSION: &str = dogtag_standard::wrap::LEVEL_B_VERSION;

/// Pull the one-time export TOKEN out of the export QR URL (`.../x/<token>?a=<relayer>`).
fn token_from_qr(qr: &str) -> String {
    qr.rsplit('/').next().unwrap().split('?').next().unwrap().to_string()
}

/// The dogtag-owned TRUST tier for the version this deployment serves, resolved the way an app would
/// (P3 signed manifest / on-chain record), NOT from the platform's claims.
fn dogtag_anchor() -> TrustedAnchor {
    let m = dogtag_prover::manifest::build(VERSION).expect("unified version is known");
    anchor_from_manifest(&m, true, true)
}

/// Report one validation attempt exactly as the app experiences it (PASS + the artifact key it selects,
/// or REFUSED + the reason it aborts on), so the transcript reads as the product behaviour.
fn report(label: &str, claims: &ConvenienceClaims, anchor: &TrustedAnchor, app_version: &str, expected_purpose: &str) {
    match validate_discovery(
        claims.clone(),
        anchor.clone(),
        app_version.to_string(),
        expected_purpose.to_string(),
    ) {
        Ok(v) => println!(
            "  {label:<34} -> PASS     version={} circuitId={} registry={}",
            v.version, v.circuit_id, v.verification_registry
        ),
        Err(e) => println!("  {label:<34} -> REFUSED  {e}"),
    }
}

/// The `/x/` (verify) resolve GET carries the convenience tier, an honest platform's claims validate
/// against the dogtag anchor, and every tampered axis a lying platform could forge is REFUSED.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_resolve_convenience_tier_is_validated_against_the_dogtag_anchor() {
    // An honestly configured deployment uses the registry named by the unified manifest,
    // so the claims it emits should agree with dogtag's own anchor.
    let anchor = dogtag_anchor();
    let mem = MemChain::new();
    let mut state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        ISSUER_REGISTRY.to_string(),
        VACCINATION_ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let mut cfg = (*state.cfg).clone();
    cfg.verification_registry_consent_addr = anchor.verification_registry.clone();
    state.cfg = Arc::new(cfg);
    let app = vet_api::router(state);
    let (_admin, op, relayer) = boot_custody(&app).await;
    mem.whitelist(ISSUER_REGISTRY, &verify_key(PURPOSE), &relayer);

    // --- operator opens a verify session -> a one-time export token in the QR the owner scans ---
    let (s, b) = call(
        &app,
        "POST",
        "/verify/session/start",
        Some(&op),
        Some(serde_json::json!({"purpose": PURPOSE, "recordType": "VACCINATION"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "verify session start: {b}");
    let token = token_from_qr(b["qrUrl"].as_str().expect("qrUrl"));

    // --- the owner's device resolves the QR (non-consuming) -> the real wire response ---
    let (s, meta) = call(&app, "GET", &format!("/x/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "GET /x/<token> resolve: {meta}");

    println!("\n=== GET /x/<token>  (verify resolve — what the owner's device receives) ===");
    println!("{}", serde_json::to_string_pretty(&meta).unwrap());

    assert_eq!(meta["purpose"], PURPOSE, "existing resolve fields unchanged: {meta}");

    // The convenience tier is present, clearly labelled UNVERIFIED, and parses into the exact struct
    // the validator consumes (the camelCase wire contract).
    let claims_json = &meta["unverifiedClaims"];
    assert!(claims_json.is_object(), "resolve must carry unverifiedClaims: {meta}");
    let claims: ConvenienceClaims =
        serde_json::from_value(claims_json.clone()).expect("unverifiedClaims parses as ConvenienceClaims");
    assert_eq!(claims.protocol_version, VERSION);
    assert_eq!(
        claims.purpose, PURPOSE,
        "the /x/ convenience purpose is the SESSION's verify purpose, not a fabricated namespace"
    );
    assert_eq!(
        claims.issuer_clone.to_lowercase(),
        VACCINATION_ISSUER,
        "the /x/ issuerClone is the clone for the session's record type"
    );
    assert_eq!(claims.chain_id, 135);

    println!("\n=== dogtag TRUST anchor (resolved independently: P3 manifest / ProtocolRegistry) ===");
    println!(
        "  version={} chainId={} registry={} circuitId={} minAppVersion={} contractSetActive={} artifactSetActive={}",
        anchor.version,
        anchor.chain_id,
        anchor.verification_registry,
        anchor.circuit_id,
        anchor.min_app_version,
        anchor.contract_set_active,
        anchor.artifact_set_active
    );

    println!("\n=== app TRUST gate: validate(claims, anchor, ctx) ===");
    let ctx = ClientContext { app_version: "1.4.0", expected_purpose: PURPOSE };

    // (1) HONEST platform -> the app proceeds, selecting the artifact by the validated circuitId.
    report("honest platform", &claims, &anchor, "1.4.0", PURPOSE);
    let v = validate(&claims, &anchor, &ctx).expect("honest wire claims must validate");
    assert_eq!(v.version, VERSION);
    assert_eq!(v.circuit_id, anchor.circuit_id);
    assert_eq!(v.verification_registry, anchor.verification_registry);

    // (2) LYING platform, registry redirect: the wire claims are tampered to steer the proof at an
    //     attacker registry. THE core attack — refused before the app ever proves.
    let mut redirect = claims.clone();
    redirect.verification_registry = "0x000000000000000000000000000000000000dEaD".to_string();
    report("lying platform: registry swap", &redirect, &anchor, "1.4.0", PURPOSE);
    assert!(matches!(
        validate(&redirect, &anchor, &ctx),
        Err(DiscoveryError::RegistryMismatch { .. })
    ));

    // (3) LYING platform, wrong chain (a fork/testnet).
    let mut forked = claims.clone();
    forked.chain_id = 1;
    report("lying platform: chain swap", &forked, &anchor, "1.4.0", PURPOSE);
    assert!(matches!(
        validate(&forked, &anchor, &ctx),
        Err(DiscoveryError::ChainIdMismatch { .. })
    ));

    // (4) LYING platform, purpose swap: the owner intends GROOMING_INTAKE out-of-band; the platform
    //     claims something else. Consent integrity — refused.
    let mut swapped = claims.clone();
    swapped.purpose = "AIRLINE_CHECKIN".to_string();
    report("lying platform: purpose swap", &swapped, &anchor, "1.4.0", PURPOSE);
    assert!(matches!(
        validate(&swapped, &anchor, &ctx),
        Err(DiscoveryError::PurposeMismatch { .. })
    ));

    // (5) DEPRECATED version (what `ProtocolRegistry.deprecateContractSet` produces on-chain): every claim
    //     is honest, yet the app still refuses — the anti-downgrade lever (§8.4).
    let mut deprecated = anchor.clone();
    deprecated.contract_set_active = false;
    report("contract set deprecated on-chain", &claims, &deprecated, "1.4.0", PURPOSE);
    assert!(matches!(
        validate(&claims, &deprecated, &ctx),
        Err(DiscoveryError::DeprecatedVersion { .. })
    ));

    // (5b) The INDEPENDENT artifact-axis lever (R-5): the trio is perfectly live and every claim is
    //      honest, but the bound proving-artifact set was retired — so the app still refuses. This is the
    //      kill switch that would vanish if the two on-chain `active` bits were collapsed into one.
    let mut artifacts_pulled = anchor.clone();
    artifacts_pulled.artifact_set_active = false;
    report("artifact set deprecated on-chain", &claims, &artifacts_pulled, "1.4.0", PURPOSE);
    assert!(matches!(
        validate(&claims, &artifacts_pulled, &ctx),
        Err(DiscoveryError::DeprecatedVersion { .. })
    ));

    // (6) The app build predates the version's floor -> refuse and route to update.
    let mut floored = anchor.clone();
    floored.min_app_version = "1.9.0".to_string();
    report("app build 1.10.0 vs floor 1.9.0", &claims, &floored, "1.10.0", PURPOSE);
    report("app build 1.3.0 vs floor 1.9.0", &claims, &floored, "1.3.0", PURPOSE);
    let too_old = ClientContext { app_version: "1.3.0", expected_purpose: PURPOSE };
    assert!(matches!(
        validate(&claims, &floored, &too_old),
        Err(DiscoveryError::AppTooOld { .. })
    ));
    // Numeric, not lexical: 1.10.0 clears a 1.9.0 floor.
    let newer = ClientContext { app_version: "1.10.0", expected_purpose: PURPOSE };
    assert!(validate(&claims, &floored, &newer).is_ok(), "1.10.0 must clear a 1.9.0 floor");
    println!();
}

/// The `/p/` (issuance) resolve GET carries the same convenience tier, whose purpose is the record-type
/// namespace, and it validates against the same anchor. Printed alongside `/x/` so the transcript shows
/// BOTH resolve surfaces the change extends.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issuance_resolve_convenience_tier_is_validated_against_the_dogtag_anchor() {
    let anchor = dogtag_anchor();
    let mem = MemChain::new();
    let mut state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        ISSUER_REGISTRY.to_string(),
        VACCINATION_ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let mut cfg = (*state.cfg).clone();
    cfg.verification_registry_consent_addr = anchor.verification_registry.clone();
    state.cfg = Arc::new(cfg);
    let app = vet_api::router(state);
    let (_admin, op, _relayer) = boot_custody(&app).await;

    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/session/start",
        Some(&op),
        Some(serde_json::json!({
            "ownerIdentity": {"countryOfIdentification":"GB","identification":"PASSPORT-123","name":"Alice Owner"},
            "pet": {"name":"Rex","species":"Canis lupus familiaris","sex":"male","dateOfBirth":"2021-05-01"}
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "profile issue session start: {b}");
    let token = b["token"].as_str().expect("token").to_string();

    let (s, meta) = call(&app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "GET /p/<token> resolve: {meta}");

    println!("\n=== GET /p/<token>  (issuance resolve — what the owner's device receives) ===");
    println!("{}", serde_json::to_string_pretty(&meta).unwrap());

    let claims: ConvenienceClaims = serde_json::from_value(meta["unverifiedClaims"].clone())
        .expect("unverifiedClaims parses as ConvenienceClaims");
    // Issuance has no verify purpose, so the namespace is the record type the app independently knows.
    assert_eq!(claims.purpose, "DOG_PROFILE");
    assert_eq!(claims.protocol_version, VERSION);

    println!("\n=== app TRUST gate: validate(claims, anchor, ctx) ===");
    report("honest platform (issuance)", &claims, &anchor, "1.4.0", "DOG_PROFILE");
    let ctx = ClientContext { app_version: "1.4.0", expected_purpose: "DOG_PROFILE" };
    assert!(validate(&claims, &anchor, &ctx).is_ok(), "honest issuance claims must validate");

    // A lying platform on the issuance path is refused the same way.
    let mut redirect = claims.clone();
    redirect.verification_registry = "0x000000000000000000000000000000000000dEaD".to_string();
    report("lying platform: registry swap", &redirect, &anchor, "1.4.0", "DOG_PROFILE");
    assert!(matches!(
        validate(&redirect, &anchor, &ctx),
        Err(DiscoveryError::RegistryMismatch { .. })
    ));
    println!();
}
