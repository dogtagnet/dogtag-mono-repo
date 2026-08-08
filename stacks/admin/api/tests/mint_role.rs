//! The dog-tag MINT-ROLE card's backend (`/v1/admin/sbt/mint-role`).
//!
//! Why this surface exists, measured rather than reasoned (2026-08-07 live walk):
//! `DogTagSBTConsent.mintCustodial` is `onlyRole(ISSUER_ROLE)`, `Deploy.s.sol` grants that role to
//! NOBODY, and the first vet dog-tag issuance therefore died as a silent gas-estimation revert —
//! `AccessControlUnauthorizedAccount(vetSigner, keccak256("ISSUER"))` — after `issue(R)` had
//! already landed, stranding the root. The captain's ruling: the grant is a BUTTON on the admin
//! portal, in the operator's vocabulary, never a cast command in anyone's path.
//!
//! Shape rules pinned here, inherited from the registrar surfaces:
//! - every write routes through `governance::dispatch` (executed when the hosted key IS the SBT's
//!   DEFAULT_ADMIN, else an unsigned proposal for the holder — the tri-state `outcome`);
//! - "who holds it" is tri-state: a FAILED enumeration is `unavailable` with its reason, never an
//!   empty list ("nobody holds the mint role" is an accusation only a successful read may make);
//! - a definite already-in-that-state read refuses the no-op (OZ grant/revoke succeed silently, so
//!   a landed no-op would report a transaction that changed nothing).
mod common;

use axum::http::StatusCode;
use common::*;

const HOSTED: &str = "0x00000000000000000000000000000000000000ad";
const VET_SIGNER: &str = "0x00000000000000000000000000000000000000e7";

async fn app_with_admin() -> (axum::Router, String, admin_api::chain::MemChain) {
    let (state, chain, _v, _b) = hermetic_state();
    let app = admin_api::routes::router(state);
    let tok = admin_token(&app).await;
    (app, tok, chain)
}

/// The read: a fresh SBT reports ZERO holders — the exact misprovisioning the live walk hit —
/// as a definite, resolved answer, beside the role key and the DEFAULT_ADMIN who can fix it.
#[tokio::test]
async fn the_card_reports_an_empty_holder_set_as_a_definite_answer() {
    let (app, tok, chain) = app_with_admin().await;
    chain.set_default_admin(SBT, HOSTED);

    let (s, b) = call(&app, "GET", "/v1/admin/sbt/mint-role", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["holders"]["state"], "resolved");
    assert_eq!(b["holders"]["accounts"], serde_json::json!([]));
    assert_eq!(
        b["roleKey"],
        admin_api::chain::issuer_role_key(),
        "the role key is derived, shown so an operator can verify against the contract"
    );
    assert_eq!(b["defaultAdmin"], HOSTED, "who can grant is named: {b}");
}

/// The grant EXECUTES when the hosted key is the SBT's DEFAULT_ADMIN, and the holder list then
/// carries the signer — read back through the same code path production reads.
#[tokio::test]
async fn granting_the_mint_role_executes_and_the_holder_list_reads_it_back() {
    let (app, tok, chain) = app_with_admin().await;
    chain.set_default_admin(SBT, HOSTED);
    chain.set_role(SBT, &admin_api::chain::default_admin_role(), HOSTED);

    let (s, b) = call(
        &app,
        "POST",
        "/v1/admin/sbt/mint-role",
        Some(&tok),
        Some(serde_json::json!({ "signer": VET_SIGNER, "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "executed", "{b}");
    assert!(b["actions"][0]["txHash"].as_str().is_some(), "{b}");

    let (s, b) = call(&app, "GET", "/v1/admin/sbt/mint-role", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        b["holders"]["accounts"],
        serde_json::json!([VET_SIGNER]),
        "{b}"
    );

    // The withdrawal direction is the same lever.
    let (s, b) = call(
        &app,
        "POST",
        "/v1/admin/sbt/mint-role",
        Some(&tok),
        Some(serde_json::json!({ "signer": VET_SIGNER, "allowed": false })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "executed");
    let (_s, b) = call(&app, "GET", "/v1/admin/sbt/mint-role", Some(&tok), None).await;
    assert_eq!(b["holders"]["accounts"], serde_json::json!([]), "{b}");
}

/// Without the authority, NOTHING is broadcast: the action comes back as an unsigned proposal
/// naming the real holder, and the undeclared case carries the loud wrong-key warning.
#[tokio::test]
async fn a_hosted_key_without_the_sbt_admin_proposes_rather_than_broadcasting() {
    let (app, tok, chain) = app_with_admin().await;
    // The SBT's DEFAULT_ADMIN is someone else (post-handover governance, or a hand-deployed SBT).
    chain.set_default_admin(SBT, "0x00000000000000000000000000000000000000d0");

    let (s, b) = call(
        &app,
        "POST",
        "/v1/admin/sbt/mint-role",
        Some(&tok),
        Some(serde_json::json!({ "signer": VET_SIGNER, "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "proposed_unauthorized", "{b}");
    assert!(
        b["warning"]
            .as_str()
            .unwrap_or_default()
            .contains("NOTHING WAS BROADCAST"),
        "{b}"
    );
    assert_eq!(
        b["actions"][0]["holder"], "0x00000000000000000000000000000000000000d0",
        "the proposal names who CAN execute it: {b}"
    );

    // And the holder list is untouched.
    let (_s, b) = call(&app, "GET", "/v1/admin/sbt/mint-role", Some(&tok), None).await;
    assert_eq!(b["holders"]["accounts"], serde_json::json!([]), "{b}");
}

/// A no-op is refused on a DEFINITE read: OZ's grantRole succeeds silently on an already-held
/// role, which would report "executed" while changing nothing.
#[tokio::test]
async fn an_already_held_role_refuses_the_redundant_grant() {
    let (app, tok, chain) = app_with_admin().await;
    chain.set_role(SBT, &admin_api::chain::default_admin_role(), HOSTED);
    let (s, _b) = call(
        &app,
        "POST",
        "/v1/admin/sbt/mint-role",
        Some(&tok),
        Some(serde_json::json!({ "signer": VET_SIGNER, "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, b) = call(
        &app,
        "POST",
        "/v1/admin/sbt/mint-role",
        Some(&tok),
        Some(serde_json::json!({ "signer": VET_SIGNER, "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap_or_default()
            .contains("already holds"),
        "{b}"
    );
}

/// The gate is the admin session — a well-formed body without it never reaches dispatch.
#[tokio::test]
async fn the_card_requires_an_admin_session() {
    let (app, _tok, _chain) = app_with_admin().await;
    let (s, _b) = call(&app, "GET", "/v1/admin/sbt/mint-role", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // The auth gate must be what refuses, so the body is WELL-FORMED (a `{}` body would 422 at the
    // extractor before `require_admin` ever ran, passing this test for the wrong reason).
    let (s, _b) = call(
        &app,
        "POST",
        "/v1/admin/sbt/mint-role",
        None,
        Some(serde_json::json!({ "signer": VET_SIGNER, "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

/// A malformed signer is refused before any read or dispatch.
#[tokio::test]
async fn a_malformed_signer_is_refused() {
    let (app, tok, _chain) = app_with_admin().await;
    for bad in ["", "0x123", "not-an-address"] {
        let (s, b) = call(
            &app,
            "POST",
            "/v1/admin/sbt/mint-role",
            Some(&tok),
            Some(serde_json::json!({ "signer": bad, "allowed": true })),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "signer {bad:?}: {b}");
    }
}
