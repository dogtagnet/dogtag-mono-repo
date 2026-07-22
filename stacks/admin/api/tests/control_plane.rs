//! Integration coverage for the PR-A control-plane surface: the factory deterministic-address preview,
//! the `GovernanceAction`-routed deploy (execute vs propose), and the on-chain authority map — all over
//! the real Axum admin router against MemChain.

mod common;

use axum::http::StatusCode;
use common::{admin_token, hermetic_state, FACTORY};

use admin_api::chain::{default_admin_role, whitelist_admin_role};

const HOSTED: &str = "0x00000000000000000000000000000000000000ad"; // hermetic_state's index-0 signer
const RT: &str = "VACCINATION";

/// predict returns a deterministic clone address that is stable across calls for the same inputs.
#[tokio::test]
async fn factory_predict_is_deterministic() {
    let (state, _chain, _v, _b) = hermetic_state();
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    let body = serde_json::json!({ "recordType": RT, "business": HOSTED });
    let (s1, b1) = common::call(
        &app,
        "POST",
        "/v1/admin/factory/predict",
        Some(&tok),
        Some(body.clone()),
    )
    .await;
    assert_eq!(s1, StatusCode::OK, "{b1}");
    let addr1 = b1["predicted"].as_str().unwrap().to_string();
    assert!(addr1.starts_with("0x") && addr1.len() == 42);
    // recordType label is hashed to its bytes32 key.
    assert_eq!(b1["recordTypeKey"], admin_api::chain::record_type_key(RT));

    let (_s2, b2) = common::call(
        &app,
        "POST",
        "/v1/admin/factory/predict",
        Some(&tok),
        Some(body),
    )
    .await;
    assert_eq!(
        b2["predicted"].as_str().unwrap(),
        addr1,
        "predict must be deterministic"
    );
}

/// A malformed caller-provided `business` is rejected with 400 rather than silently coerced to the
/// zero address (which would yield a wrong deterministic clone / deploy salt).
#[tokio::test]
async fn factory_malformed_business_is_rejected() {
    let (state, _chain, _v, _b) = hermetic_state();
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    for path in ["/v1/admin/factory/predict", "/v1/admin/factory/issuers"] {
        let body = serde_json::json!({ "name": "Vax Authority", "recordType": RT, "business": "0xnothex" });
        let (s, b) = common::call(&app, "POST", path, Some(&tok), Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{path}: {b}");
    }
}

/// The deploy requires an admin session.
#[tokio::test]
async fn factory_deploy_requires_admin() {
    let (state, _c, _v, _b) = hermetic_state();
    let app = admin_api::router(state);
    let (s, _b) = common::call(
        &app,
        "POST",
        "/v1/admin/factory/issuers",
        None,
        Some(serde_json::json!({ "name": "Vax Authority", "recordType": RT })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

/// When the hosted key is NOT the factory owner (default in hermetic_state), the deploy is PROPOSED:
/// nothing is broadcast; the calldata payload + predicted address are returned.
#[tokio::test]
async fn factory_deploy_proposes_when_not_owner() {
    let (state, _chain, _v, _b) = hermetic_state();
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    let (s, b) = common::call(
        &app,
        "POST",
        "/v1/admin/factory/issuers",
        Some(&tok),
        Some(serde_json::json!({ "name": "Vax Authority", "recordType": RT, "business": HOSTED })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["result"]["disposition"], "proposed");
    assert!(b["result"]["calldata"].as_str().unwrap().starts_with("0x"));
    assert!(b["predicted"].as_str().unwrap().starts_with("0x"));
}

/// When the hosted key IS the factory owner, the deploy is EXECUTED (signed + broadcast).
#[tokio::test]
async fn factory_deploy_executes_when_owner() {
    let (state, chain, _v, _b) = hermetic_state();
    chain.set_factory_owner(FACTORY, HOSTED);
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    let (s, b) = common::call(
        &app,
        "POST",
        "/v1/admin/factory/issuers",
        Some(&tok),
        Some(serde_json::json!({ "name": "Vax Authority", "recordType": RT, "business": HOSTED })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["result"]["disposition"], "executed");
    assert!(b["result"]["txHash"].as_str().unwrap().starts_with("0x"));
}

/// The authority map reflects role holdership and the pending DEFAULT_ADMIN transfer (Phase-2 shape).
#[tokio::test]
async fn governance_authority_map_reflects_holders_and_pending_transfer() {
    let (state, chain, _v, _b) = hermetic_state();
    let registry = state.cfg.issuer_registry_addr.clone();
    // hosted key holds WHITELIST_ADMIN + is the factory owner; DEFAULT_ADMIN is mid-handover to gov.
    chain.set_factory_owner(FACTORY, HOSTED);
    chain.set_role(&registry, &whitelist_admin_role(), HOSTED);
    // on-chain, defaultAdmin() and hasRole(DEFAULT_ADMIN_ROLE, holder) are consistent — seed both.
    chain.set_default_admin(&registry, HOSTED);
    chain.set_role(&registry, &default_admin_role(), HOSTED);
    let gov = "0x8e27e11700000000000000000000000000000000";
    chain.set_pending_default_admin(&registry, gov, 1_782_988_652);

    let app = admin_api::router(state);
    let tok = admin_token(&app).await;
    let (s, b) = common::call(
        &app,
        "GET",
        "/v1/admin/governance/authority",
        Some(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    assert_eq!(b["factoryOwner"]["heldByHosted"], true);
    assert_eq!(b["whitelistAdmin"]["heldByHosted"], true);
    // hosted still holds DEFAULT_ADMIN today, but the pending transfer to governance is surfaced.
    assert_eq!(b["defaultAdmin"]["heldByHosted"], true);
    assert_eq!(
        b["defaultAdmin"]["pendingTransfer"]["newAdmin"]
            .as_str()
            .unwrap(),
        gov
    );
    assert_eq!(
        b["defaultAdmin"]["pendingTransfer"]["acceptSchedule"],
        1_782_988_652u64
    );
}

// ============================================================================================
// PR-E — direct whitelist grant / revoke (management console), routed through GovernanceAction.
// ============================================================================================

use common::{REGISTRY, SBT};

/// A distinct, well-formed signer address to grant/revoke against (not the hosted key).
const SIGNER: &str = "0x00000000000000000000000000000000000000c3";

/// Grant requires an admin session.
#[tokio::test]
async fn whitelist_grant_requires_admin() {
    let (state, _c, _v, _b) = hermetic_state();
    let app = admin_api::router(state);
    let (s, _b) = common::call(
        &app,
        "POST",
        "/v1/admin/whitelist/grant",
        None,
        Some(serde_json::json!({ "signer": SIGNER, "recordType": RT })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

/// A grant with neither recordType nor verifyPurposes is a 400 (nothing to whitelist).
#[tokio::test]
async fn whitelist_grant_rejects_missing_capability() {
    let (state, _c, _v, _b) = hermetic_state();
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;
    let (s, b) = common::call(
        &app,
        "POST",
        "/v1/admin/whitelist/grant",
        Some(&tok),
        Some(serde_json::json!({ "signer": SIGNER })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
}

/// A malformed signer address is rejected rather than silently coerced to the zero address.
#[tokio::test]
async fn whitelist_grant_rejects_bad_signer() {
    let (state, _c, _v, _b) = hermetic_state();
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;
    let (s, b) = common::call(
        &app,
        "POST",
        "/v1/admin/whitelist/grant",
        Some(&tok),
        Some(serde_json::json!({ "signer": "0xnothex", "recordType": RT })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
}

/// When the hosted key does NOT hold WHITELIST_ADMIN (default hermetic state), the grant is PROPOSED:
/// nothing is broadcast; the calldata payload is returned for the role holder to execute.
#[tokio::test]
async fn whitelist_grant_proposes_when_hosted_lacks_role() {
    let (state, _chain, _v, _b) = hermetic_state();
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;
    let (s, b) = common::call(
        &app,
        "POST",
        "/v1/admin/whitelist/grant",
        Some(&tok),
        Some(serde_json::json!({ "signer": SIGNER, "recordType": RT })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["actions"].as_array().unwrap().len(), 1);
    assert_eq!(b["actions"][0]["disposition"], "proposed");
    assert!(b["actions"][0]["calldata"].as_str().unwrap().starts_with("0x"));
    // not a DOG_PROFILE grant -> no ISSUER role step.
    assert!(b["issuerRole"].is_null());
    assert_eq!(b["signer"].as_str().unwrap(), SIGNER);
}

/// When the hosted key holds WHITELIST_ADMIN, the grant (recordType + each verify purpose) is EXECUTED:
/// one broadcast tx per capability.
#[tokio::test]
async fn whitelist_grant_executes_all_capabilities_when_holds_role() {
    let (state, chain, _v, _b) = hermetic_state();
    chain.set_role(REGISTRY, &whitelist_admin_role(), HOSTED);
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;
    let (s, b) = common::call(
        &app,
        "POST",
        "/v1/admin/whitelist/grant",
        Some(&tok),
        Some(serde_json::json!({
            "signer": SIGNER,
            "recordType": RT,
            "verifyPurposes": ["grooming_intake", "boarding_intake"],
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let actions = b["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 3, "recordType + 2 verify purposes");
    for a in actions {
        assert_eq!(a["disposition"], "executed");
        assert!(a["txHash"].as_str().unwrap().starts_with("0x"));
    }
}

/// A DOG_PROFILE grant also grants DogTagSBTConsent.ISSUER_ROLE (`mintCustodial`) when the hosted key holds the
/// SBT DEFAULT_ADMIN authority — the same onboarding step approve runs, as a standalone action.
#[tokio::test]
async fn whitelist_grant_dog_profile_grants_issuer_role() {
    let (state, chain, _v, _b) = hermetic_state();
    chain.set_role(REGISTRY, &whitelist_admin_role(), HOSTED);
    chain.set_role(SBT, &default_admin_role(), HOSTED);
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;
    let (s, b) = common::call(
        &app,
        "POST",
        "/v1/admin/whitelist/grant",
        Some(&tok),
        Some(serde_json::json!({ "signer": SIGNER, "recordType": "DOG_PROFILE" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["actions"][0]["disposition"], "executed");
    // the ISSUER-role grant is a governance action too (executed here, holder holds SBT DEFAULT_ADMIN).
    assert_eq!(b["issuerRole"]["disposition"], "executed");
    assert!(b["issuerRole"]["txHash"].as_str().unwrap().starts_with("0x"));
}

/// Revoke delists the record type (delistFor) and is EXECUTED when the hosted key holds WHITELIST_ADMIN.
#[tokio::test]
async fn whitelist_revoke_executes_when_holds_role() {
    let (state, chain, _v, _b) = hermetic_state();
    chain.set_role(REGISTRY, &whitelist_admin_role(), HOSTED);
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;
    let (s, b) = common::call(
        &app,
        "POST",
        "/v1/admin/whitelist/revoke",
        Some(&tok),
        Some(serde_json::json!({ "signer": SIGNER, "recordType": RT })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["actions"].as_array().unwrap().len(), 1);
    assert_eq!(b["actions"][0]["disposition"], "executed");
    assert!(b["actions"][0]["txHash"].as_str().unwrap().starts_with("0x"));
    // revoke never carries an issuerRole field.
    assert!(b.get("issuerRole").is_none());
}
