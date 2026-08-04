//! `GET /issuer/issuance-allowed` — LAYER 2 of the two-layer issuance requirement, over the real
//! router.
//!
//! What these pin, and why each one exists rather than being obvious:
//!
//! * The roster reports CURRENT storage, so a withdrawn address is visibly withdrawn rather than
//!   merely absent - the distinction the guide-walk crew misread on a neighbouring screen.
//! * A read that FAILED answers `unavailable` with a reason, never an empty list. That is the whole
//!   honesty requirement of this surface: a provider deciding who may sign medical records in their
//!   name must never be shown "nobody is admitted" when the truth is "we could not ask".
//! * `activeSignerAllowed` answers the question the gap was actually about - "I am approved and my
//!   contract is attached, so why does issuing still fail" - per contract, including the
//!   `PROFILE_ISSUER_ADDR` clone the dog-tag bind anchors through.
//! * There is NO write route, in either direction, and that is asserted rather than assumed.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{call, mint_operator, state_with, PROFILE_ISSUER_ADDR};
use serde_json::Value;
use vet_api::chain::MemChain;

const VACC_CLONE: &str = "0x00000000000000000000000000000000000000c1";
const CLONE_OWNER: &str = "0x00000000000000000000000000000000000000a1";
/// A staff key the provider admitted and later withdrew.
const WITHDRAWN: &str = "0x00000000000000000000000000000000000000b2";
const REGISTRY: &str = "0x0000000000000000000000000000000000000011";

fn app_with(chain: Arc<MemChain>) -> (axum::Router, Arc<MemChain>) {
    let state = state_with(
        chain.clone(),
        "http://localhost:0".into(),
        REGISTRY.into(),
        VACC_CLONE.into(),
        "vet.example".into(),
        1,
    );
    (vet_api::routes::router(state), chain)
}

/// Unlock custody through the real genesis path so `active_address()` is a real derived signer
/// rather than a value the test asserted into place.
async fn boot(app: &axum::Router) -> (String, String) {
    let (_admin, operator, backend) = common::boot_custody(app).await;
    (operator, backend.to_lowercase())
}

fn contract<'a>(body: &'a Value, record_type: &str) -> &'a Value {
    body["contracts"]
        .as_array()
        .expect("contracts array")
        .iter()
        .find(|c| c["recordType"] == record_type)
        .unwrap_or_else(|| panic!("no {record_type} contract in {body}"))
}

// -------------------------------------------------------------------------------------------------
// The roster itself
// -------------------------------------------------------------------------------------------------

/// The headline read: who owns the contract, who is on its list, and whether OUR signer is.
#[tokio::test]
async fn the_roster_reports_the_owner_and_the_current_list() {
    let chain = Arc::new(MemChain::new());
    let (app, chain) = app_with(chain);
    let (operator, backend) = boot(&app).await;

    chain.set_clone_owner(VACC_CLONE, CLONE_OWNER);
    // The creation seed: a real clone admits its deployer at `initialize`.
    chain.set_issuance_allowed(VACC_CLONE, CLONE_OWNER, true);
    chain.set_issuance_allowed(VACC_CLONE, &backend, true);

    let (s, b) = call(
        &app,
        "GET",
        "/issuer/issuance-allowed",
        Some(&operator),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["activeSigner"], backend);

    let vacc = contract(&b, "VACCINATION");
    assert_eq!(vacc["read"]["state"], "resolved");
    assert_eq!(vacc["read"]["owner"], CLONE_OWNER);
    assert_eq!(
        vacc["read"]["activeSignerAllowed"], true,
        "an admitted signer was not reported as able to issue"
    );
    let entries = vacc["read"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "roster: {entries:?}");
    assert!(entries
        .iter()
        .any(|e| e["address"] == backend && e["allowed"] == true));
}

/// WITHDRAWN and NEVER-ADMITTED are different facts and must not render the same. This is the
/// distinction the task names as previously misread - it is carried as DATA (`everNamed`) so it
/// survives being read as plain text, not only as styling.
#[tokio::test]
async fn a_withdrawn_signer_is_reported_apart_from_one_that_was_never_admitted() {
    let chain = Arc::new(MemChain::new());
    let (app, chain) = app_with(chain);
    let (operator, backend) = boot(&app).await;

    chain.set_clone_owner(VACC_CLONE, CLONE_OWNER);
    chain.set_issuance_allowed(VACC_CLONE, WITHDRAWN, true);
    chain.set_issuance_allowed(VACC_CLONE, WITHDRAWN, false);

    let (s, b) = call(
        &app,
        "GET",
        "/issuer/issuance-allowed",
        Some(&operator),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let entries = contract(&b, "VACCINATION")["read"]["entries"]
        .as_array()
        .unwrap()
        .clone();

    let withdrawn = entries
        .iter()
        .find(|e| e["address"] == WITHDRAWN)
        .expect("the withdrawn address vanished from the roster");
    assert_eq!(withdrawn["allowed"], false);
    assert_eq!(
        withdrawn["everNamed"], true,
        "a withdrawn entry must record that the list once held it"
    );

    let ours = entries
        .iter()
        .find(|e| e["address"] == backend)
        .expect("our own signer had no row to point at");
    assert_eq!(ours["allowed"], false);
    assert_eq!(
        ours["everNamed"], false,
        "our never-admitted signer was reported as withdrawn"
    );
}

/// The diagnostic the whole gap was about. Our signer must have a row BEFORE anyone admits it -
/// otherwise the address the provider needs to act on is invisible in exactly the state they are
/// trying to fix.
#[tokio::test]
async fn our_own_signer_is_listed_and_reported_unable_to_issue_before_it_is_admitted() {
    let chain = Arc::new(MemChain::new());
    let (app, chain) = app_with(chain);
    let (operator, backend) = boot(&app).await;

    chain.set_clone_owner(VACC_CLONE, CLONE_OWNER);
    chain.set_issuance_allowed(VACC_CLONE, CLONE_OWNER, true);

    let (s, b) = call(
        &app,
        "GET",
        "/issuer/issuance-allowed",
        Some(&operator),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let vacc = contract(&b, "VACCINATION");
    assert_eq!(
        vacc["read"]["activeSignerAllowed"], false,
        "an unadmitted signer was reported as able to issue"
    );
    assert!(
        vacc["read"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["address"] == backend),
        "our signer must have a row before it is admitted"
    );
}

/// `PROFILE_ISSUER_ADDR` is a real `DogTagIssuer` clone reached by the dog-tag bind, so it needs
/// layer 2 exactly as the record-type clones do. It is the half most likely to be forgotten, because
/// completing a bind needs the phone app and is therefore rarely walked - which is precisely why it
/// is asserted here.
#[tokio::test]
async fn the_dog_tag_profile_clone_is_covered_too_not_only_the_record_type_clones() {
    let chain = Arc::new(MemChain::new());
    let (app, chain) = app_with(chain);
    let (operator, _backend) = boot(&app).await;

    chain.set_clone_owner(PROFILE_ISSUER_ADDR, CLONE_OWNER);
    chain.set_clone_owner(VACC_CLONE, CLONE_OWNER);

    let (s, b) = call(
        &app,
        "GET",
        "/issuer/issuance-allowed",
        Some(&operator),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let types: Vec<&str> = b["contracts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["recordType"].as_str().unwrap())
        .collect();
    assert!(
        types.contains(&"DOG_PROFILE") && types.contains(&"VACCINATION"),
        "both anchoring contracts must be covered, got {types:?}"
    );
}

// -------------------------------------------------------------------------------------------------
// Honesty
// -------------------------------------------------------------------------------------------------

/// A LOG READ THAT FAILED IS NOT AN EMPTY LIST.
///
/// The switch fails only the `eth_getLogs` half, which is the realistic shape: a range-capping or
/// rate-limiting peer refuses the log while every `eth_call` beside it answers. A shared failure
/// switch would take the owner read down with it and collapse the route into a 502, so the arm this
/// surface exists to render would never be built.
#[tokio::test]
async fn an_unreadable_log_answers_unavailable_with_a_reason_and_no_entries() {
    let chain = Arc::new(MemChain::new());
    let (app, chain) = app_with(chain);
    let (operator, _backend) = boot(&app).await;

    chain.set_clone_owner(VACC_CLONE, CLONE_OWNER);
    chain.set_issuance_allowed(VACC_CLONE, CLONE_OWNER, true);
    chain.set_failing_issuance_allowed_log_reads(true);

    let (s, b) = call(
        &app,
        "GET",
        "/issuer/issuance-allowed",
        Some(&operator),
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "an unreadable list is a state to render, not a route failure: {b}"
    );
    let read = &contract(&b, "VACCINATION")["read"];
    assert_eq!(read["state"], "unavailable");
    assert!(
        read["reason"].as_str().is_some_and(|r| !r.is_empty()),
        "an unavailable read must say why: {read}"
    );
    assert!(
        read["entries"].is_null(),
        "an unavailable read carried an entries field a consumer could spread as empty: {read}"
    );
    assert!(
        read["activeSignerAllowed"].is_null(),
        "a read that never happened answered whether our signer may issue: {read}"
    );
}

/// An EMPTY resolved list is a different claim and must stay distinguishable from the one above:
/// the chain was asked, and this contract admits nobody.
#[tokio::test]
async fn an_empty_list_is_resolved_not_unavailable() {
    let chain = Arc::new(MemChain::new());
    let (app, chain) = app_with(chain);
    let (operator, _backend) = boot(&app).await;
    chain.set_clone_owner(VACC_CLONE, CLONE_OWNER);

    let (s, b) = call(
        &app,
        "GET",
        "/issuer/issuance-allowed",
        Some(&operator),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let read = &contract(&b, "VACCINATION")["read"];
    assert_eq!(read["state"], "resolved");
    assert_eq!(read["activeSignerAllowed"], false);
    // Our own signer still gets its row, so "admits nobody" is visible rather than blank.
    assert_eq!(read["entries"].as_array().unwrap().len(), 1);
}

/// Locked custody has no active address, so there is no signer to answer ABOUT. `null`, never
/// `false` - "this deployment's signer may not issue" would be a claim about an address we do not
/// know.
#[tokio::test]
async fn locked_custody_reports_no_active_signer_rather_than_an_unadmitted_one() {
    let chain = Arc::new(MemChain::new());
    chain.set_clone_owner(VACC_CLONE, CLONE_OWNER);
    // Deliberately NOT `boot_custody`: no genesis, so custody has no seal and no active address.
    // The operator session is minted directly, because this test is about the CUSTODY axis and an
    // operator who cannot log in would fail it for the wrong reason.
    let state = state_with(
        chain.clone(),
        "http://localhost:0".into(),
        REGISTRY.into(),
        VACC_CLONE.into(),
        "vet.example".into(),
        1,
    );
    let operator = mint_operator(&state).await;
    let app = vet_api::routes::router(state);

    let (s, b) = call(
        &app,
        "GET",
        "/issuer/issuance-allowed",
        Some(&operator),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(
        b["activeSigner"].is_null(),
        "a locked backend named a signer it cannot derive: {b}"
    );
    assert!(
        contract(&b, "VACCINATION")["read"]["activeSignerAllowed"].is_null(),
        "a locked backend answered whether its unknown signer may issue: {b}"
    );
}

// -------------------------------------------------------------------------------------------------
// Authority
// -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_roster_is_operator_gated() {
    let chain = Arc::new(MemChain::new());
    let (app, _chain) = app_with(chain);
    let (s, _b) = call(&app, "GET", "/issuer/issuance-allowed", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

/// THE SECURITY CLAIM OF THIS SLICE, asserted rather than described.
///
/// There is no backend route that writes the list, in either direction. Admitting is the clone
/// `owner()`'s alone (`DogTagIssuer.sol:336-346`, pinned against the REAL core by
/// `CustodialIssuance.t.sol::test_the_clones_own_list_is_what_admits_a_signer_and_only_its_owner_writes_it`),
/// and this backend is not any clone's owner - its custody signer is the address that needs
/// ADMITTING. It also could not authenticate one: an operator session proves "staff of this shop",
/// never "owner of this contract".
///
/// So a write route here would either be dead or would require giving the backend owner authority,
/// which is the second layer collapsing back into the first. The write is a wallet transaction.
///
/// Asserted through the ROUTER, so this fails if someone adds such a route later - a grep in a
/// comment would not.
#[tokio::test]
async fn there_is_no_backend_route_that_writes_the_list() {
    let chain = Arc::new(MemChain::new());
    let (app, _chain) = app_with(chain);
    let (operator, _backend) = boot(&app).await;

    for (method, path) in [
        ("POST", "/issuer/issuance-allowed"),
        ("PUT", "/issuer/issuance-allowed"),
        ("DELETE", "/issuer/issuance-allowed"),
        ("POST", "/issuer/issuance-allowed/grant"),
        ("POST", "/issuer/issuance-allowed/revoke"),
    ] {
        let (s, b) = call(
            &app,
            method,
            path,
            Some(&operator),
            Some(serde_json::json!({"signer": WITHDRAWN, "allowed": true})),
        )
        .await;
        assert!(
            s == StatusCode::METHOD_NOT_ALLOWED || s == StatusCode::NOT_FOUND,
            "{method} {path} is reachable ({s}) - admitting must never be a backend capability: {b}"
        );
    }
}
