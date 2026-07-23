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
            "pet": {
                "name":"Rex","species":"Canis lupus familiaris","breedVbo":"VBO_0200800",
                "breedLabel":"Shiba Inu","sex":"male","neuterStatus":"neutered","dateOfBirth":"2021-05-01",
                "weightHistory":[{"unit":"kg","value":"22.7","measuredOn":"2026-07-01"}],
                "microchip":{"code":"985113001234567","standard":"ISO 11784","implantDate":"2021-06-01","bodyLocation":"left shoulder"}
            }
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "profile issue session start: {b}");
    let token = b["token"].as_str().expect("token").to_string();

    let (s, meta) = call(&app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "GET /p/<token> resolve: {meta}");

    println!("\n=== GET /p/<token>  (issuance resolve — what the owner's device receives) ===");
    println!("{}", serde_json::to_string_pretty(&meta).unwrap());

    // --- the MOBILE issuance contract (apps/ios Net.swift `resolveDogTagIssue`, apps/android
    // CentralApi `parseProfileIssueSession`). Both parsers FAIL CLOSED unless the resolve carries a
    // pet-metadata container AND an ownerIdentity container with a resolvable non-empty pet name —
    // these assertions mirror those guards, so this test breaking means mobile issuance breaking.
    assert_eq!(meta["sessionId"], b["sessionId"], "existing resolve fields unchanged");
    assert_eq!(meta["dogTagId"], b["dogTagId"], "existing resolve fields unchanged");
    assert_eq!(meta["status"], "pending");
    let pet = &meta["pet"];
    assert!(pet.is_object(), "mobile fails closed without a pet container: {meta}");
    assert_eq!(pet["name"], "Rex", "pet name resolvable at the `pet` level");
    let profile = &pet["profile"];
    assert!(profile.is_object(), "nested pet.profile container: {meta}");
    assert_eq!(profile["species"], "Canis lupus familiaris");
    assert_eq!(profile["breedVbo"], "VBO_0200800");
    assert_eq!(profile["breedLabel"], "Shiba Inu");
    assert_eq!(profile["sex"], "male");
    assert_eq!(profile["neuterStatus"], "neutered");
    assert_eq!(profile["dateOfBirth"], "2021-05-01");
    let weights = profile["weightHistory"].as_array().expect("weightHistory array");
    assert_eq!(weights.len(), 1);
    assert_eq!(weights[0]["unit"], "kg");
    assert_eq!(weights[0]["measuredOn"], "2026-07-01");
    assert!(
        weights[0]["value"].is_string(),
        "weight value stays a decimal STRING (precision/leading-zero discipline): {meta}"
    );
    assert_eq!(weights[0]["value"], "22.7");
    let chip = &pet["microchip"];
    assert!(chip.is_object(), "microchip container beside the profile: {meta}");
    assert_eq!(chip["code"], "985113001234567");
    assert_eq!(chip["standard"], "ISO 11784");
    assert_eq!(chip["implantDate"], "2021-06-01");
    assert_eq!(chip["bodyLocation"], "left shoulder");
    let owner = &meta["ownerIdentity"];
    assert!(owner.is_object(), "mobile fails closed without ownerIdentity: {meta}");
    assert_eq!(owner["countryOfIdentification"], "GB");
    assert_eq!(owner["identification"], "PASSPORT-123");
    assert_eq!(owner["name"], "Alice Owner");

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

/// A session started with NO identity data (and a name-only pet) still resolves with BOTH containers
/// present. The mobile parsers fail closed only when a container is ABSENT and tolerate empty fields,
/// so the degrade mode is empty strings under an always-present `ownerIdentity` — never a missing key
/// (which would brick issuance for exactly the sessions that carry the least data).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issuance_resolve_without_identity_data_still_emits_the_containers() {
    let mem = MemChain::new();
    let state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        ISSUER_REGISTRY.to_string(),
        VACCINATION_ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let (_admin, op, _relayer) = boot_custody(&app).await;

    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/session/start",
        Some(&op),
        Some(serde_json::json!({ "ownerIdentity": {}, "pet": {"name":"Rex"} })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "profile issue session start: {b}");
    let token = b["token"].as_str().expect("token").to_string();

    let (s, meta) = call(&app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "GET /p/<token> resolve: {meta}");

    // ownerIdentity: container present (the mobile fail-closed guard), every field an empty string.
    let owner = &meta["ownerIdentity"];
    assert!(owner.is_object(), "ownerIdentity container must survive empty identity: {meta}");
    assert_eq!(owner["countryOfIdentification"], "");
    assert_eq!(owner["identification"], "");
    assert_eq!(owner["name"], "");

    // pet: name resolvable; the profile + microchip containers still present with empty/absent leaves.
    let pet = &meta["pet"];
    assert!(pet.is_object(), "pet container must survive a name-only pet: {meta}");
    assert_eq!(pet["name"], "Rex");
    assert!(pet["profile"].is_object(), "profile container present: {meta}");
    assert_eq!(
        pet["profile"]["weightHistory"].as_array().map(Vec::len),
        Some(0),
        "no weights -> empty array, not a missing key: {meta}"
    );
    assert!(pet["microchip"].is_object(), "microchip container present: {meta}");
    assert_eq!(pet["microchip"]["code"], "");
}

/// A blank (or whitespace-only) pet name fails fast with 400 at session start. Both mobile parsers
/// refuse a session whose pet name resolves blank — Android trims via `isNotBlank`, so whitespace-only
/// counts too — meaning such a session would resolve fine yet always fail on-device, wasting the
/// operator's one-time QR far from the cause. The server refuses it at the operator instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issuance_session_start_refuses_a_blank_pet_name() {
    let mem = MemChain::new();
    let state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        ISSUER_REGISTRY.to_string(),
        VACCINATION_ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let (_admin, op, _relayer) = boot_custody(&app).await;

    for name in ["", " ", " \t\n "] {
        let (s, b) = call(
            &app,
            "POST",
            "/profiles/issue/session/start",
            Some(&op),
            Some(serde_json::json!({ "ownerIdentity": {}, "pet": {"name": name} })),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "pet name {name:?} must fail fast at start: {b}");
    }

    // A name that is non-blank after trimming still starts a session — the guard is trim-based only.
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/session/start",
        Some(&op),
        Some(serde_json::json!({ "ownerIdentity": {}, "pet": {"name": " Rex "} })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "non-blank name must still start a session: {b}");
}
