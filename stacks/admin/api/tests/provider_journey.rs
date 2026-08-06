//! The rest of the provider journey: attach -> stand up -> grant issuance, plus the two orthogonal
//! levers, driven end to end over the real router.
//!
//! `MemChain` DECODES and APPLIES all five new registrar writes (see
//! `apply_provider_registry_calldata`), modelling the contract's own guards - `AlreadyRegistered`,
//! `NotFactoryClone`, `InvalidServiceMetadata`, `UnexpectedServiceOwner`, `NoChange`,
//! `RetiredStanding`. A fake that merely counted transactions could not catch a route that sends a
//! call the chain will refuse, which is the whole failure mode a preflight exists to prevent.
//!
//! ## Two of these cases are RE-HOMED, not new
//!
//! `dispatch_summary`'s tri-state `outcome` and the `ADMIN_PROPOSE_ONLY` declaration were pinned by
//! the deleted whitelist console's tests. That logic is SHARED - every registrar route reports
//! through it - so deleting those tests with the console would have been a silent coverage
//! regression on logic that outlived it. They live here now, on `setVerifierCapability`.
//!
//! That route is the host because it has NO prerequisite state: `setIssuanceCapability` 404s on a
//! service that was never attached, and under propose-only nothing executes - so the attach could
//! never land and the case under test would never be reached.

mod common;

use axum::http::StatusCode;
use common::*;

use admin_api::chain::record_type_key;
use admin_api::routes::router;

const PID: &str = "0x7a1c9f3b0e5d4a28c6b1f0937de4a5b2c8017f36";
const CONTROLLER: &str = "0x0000000000000000000000000000000000c07701";
const DIGEST: &str = "0x9f2b7c1d3e4a5b6c7d8e9f0a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3e4";
/// The provider-deployed contract being attached.
const SERVICE: &str = "0x00000000000000000000000000000000000005ac";
/// The key that will sign issuances on it - deliberately NOT the controller.
const SIGNER: &str = "0x00000000000000000000000000000000000000c3";
const GENERATION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const HOSTED: &str = "0x00000000000000000000000000000000000000ad";

/// Seed a registered, ACTIVE provider plus a factory generation whose factory recognizes `SERVICE`,
/// and a `SERVICE` that answers `owner()` / `recordType()` - i.e. everything `attachService` reads.
async fn seeded() -> (axum::Router, String, admin_api::chain::MemChain) {
    let (state, chain, _v, _b) = hermetic_state();
    // The hosted key owns the registry, so registrar writes EXECUTE rather than propose. The
    // propose-only cases below deliberately do not do this.
    chain.set_factory_owner(PROVIDER_REGISTRY, HOSTED);
    chain.set_factory_generation(PROVIDER_REGISTRY, GENERATION, FACTORY, true);
    chain.set_clone(FACTORY, SERVICE);
    chain.set_service_metadata(SERVICE, CONTROLLER, &record_type_key("VACCINATION"));
    let app = router(state);
    let tok = admin_token(&app).await;
    let (s, b) = call(
        &app,
        "POST",
        "/v1/admin/providers",
        Some(&tok),
        Some(serde_json::json!({
            "providerId": PID, "controller": CONTROLLER, "identityDigest": DIGEST,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/admin/providers/{PID}/standing"),
        Some(&tok),
        Some(serde_json::json!({ "standing": "active" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    (app, tok, chain)
}

async fn attach(app: &axum::Router, tok: &str) -> (StatusCode, serde_json::Value) {
    call(
        app,
        "POST",
        &format!("/v1/admin/providers/{PID}/services"),
        Some(tok),
        Some(serde_json::json!({
            "serviceAddress": SERVICE,
            "generationId": GENERATION,
            "expectedOwner": CONTROLLER,
        })),
    )
    .await
}

/// The whole outstanding half of the journey, in the order the contracts force.
///
/// The step that is easiest to leave out is the SERVICE STANDING: attaching lands the service at
/// PENDING exactly as registration lands a provider there, and `canIssue` folds it - so a journey
/// that attaches and grants issuance without it produces a service that still issues nothing.
#[tokio::test]
async fn attach_then_stand_up_then_grant_issuance_completes_the_journey() {
    let (app, tok, chain) = seeded().await;

    let (s, b) = attach(&app, &tok).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "executed", "{b}");
    // Attaching alone grants nothing, and the response says so rather than reading as done.
    assert_eq!(b["standingAfterAttach"], "pending", "{b}");
    assert!(b["nextStep"].as_str().unwrap().contains("setServiceStanding"));

    let (s, b) = call(&app, "GET", &format!("/v1/admin/providers/{PID}/services"), Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let svc = &b["services"][0];
    assert_eq!(svc["service"]["standing"], "pending", "attach lands PENDING: {b}");
    assert_eq!(svc["service"]["recordType"], "VACCINATION", "read off the clone: {b}");
    // The RESOLVED owner is stored, never the caller's expected value.
    assert_eq!(svc["service"]["confirmedOwner"], CONTROLLER, "{b}");
    assert_eq!(svc["effective"]["serviceStanding"], "pending");
    assert_eq!(svc["effective"]["hasActiveIssuer"], false);
    assert_eq!(svc["issuance"]["state"], "resolved");
    assert!(svc["issuance"]["entries"].as_array().unwrap().is_empty());

    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/admin/services/{SERVICE}/standing"),
        Some(&tok),
        Some(serde_json::json!({ "standing": "active" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "executed", "{b}");

    let (s, b) = call(
        &app,
        "POST",
        // The grant is on the SIGNER's ADDRESS, and the path says so: no service appears in it,
        // because none appears in `setRights`. That is what lets an applicant be approved before it
        // has a clone.
        &format!("/v1/admin/rights/{SIGNER}/issue"),
        Some(&tok),
        Some(serde_json::json!({ "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "executed", "{b}");

    let (s, b) = call(&app, "GET", &format!("/v1/admin/providers/{PID}/services"), Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let svc = &b["services"][0];
    assert_eq!(svc["service"]["standing"], "active");
    assert_eq!(svc["issuance"]["entries"][0]["holder"], SIGNER);
    assert_eq!(svc["issuance"]["entries"][0]["allowed"], true);

    // The registrar has now done everything a registrar CAN do, and the service still cannot issue.
    // `hasActiveIssuer` re-folds the provider's current pointer, which only the provider's own
    // `repointService` writes - so the honest state here is four terms held and that one not.
    let e = &svc["effective"];
    assert_eq!(e["providerStanding"], "active", "{b}");
    assert_eq!(e["serviceStanding"], "active", "{b}");
    assert_eq!(e["factoryActive"], true, "{b}");
    assert_eq!(e["ownerConfirmed"], true, "{b}");
    assert_eq!(
        e["hasActiveIssuer"], false,
        "the pointer is unwritten, so the chain cannot answer true here: {b}"
    );
    assert_eq!(svc["currentPointer"]["isCurrent"], false, "{b}");

    // The provider's own repoint, the one step no registrar route performs.
    chain.set_current_service(PROVIDER_REGISTRY, PID, &record_type_key("VACCINATION"), SERVICE);

    let (s, b) = call(&app, "GET", &format!("/v1/admin/providers/{PID}/services"), Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let svc = &b["services"][0];
    assert_eq!(svc["currentPointer"]["isCurrent"], true, "{b}");
    let e = &svc["effective"];
    assert_eq!(e["providerStanding"], "active", "{b}");
    assert_eq!(e["serviceStanding"], "active", "{b}");
    assert_eq!(e["factoryActive"], true, "{b}");
    assert_eq!(e["ownerConfirmed"], true, "{b}");
    assert_eq!(
        e["hasActiveIssuer"], true,
        "the repoint is the difference, and nothing else moved: {b}"
    );
}

/// The preflight resolves the generation by PROBING each active factory's own `isClone`, so an admin
/// never types a bytes32 - and the expected owner is prefilled from the live `owner()`.
#[tokio::test]
async fn the_preflight_resolves_the_generation_and_owner_off_the_chain() {
    let (app, tok, _chain) = seeded().await;
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/admin/providers/{PID}/services/preflight"),
        Some(&tok),
        Some(serde_json::json!({ "serviceAddress": SERVICE })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["verdict"], "ready", "{b}");
    assert_eq!(b["generation"]["state"], "resolved");
    assert_eq!(b["generation"]["generationId"], GENERATION, "{b}");
    assert_eq!(b["metadata"]["state"], "resolved");
    assert_eq!(b["metadata"]["owner"], CONTROLLER, "{b}");
    assert_eq!(b["metadata"]["recordType"], "VACCINATION", "{b}");
    assert!(b["alreadyAttached"].is_null());
}

/// A generation-1 `DogTagIssuer` is `Initializable` only and has NO `owner()`, so it can never be
/// attached however correct the rest of the form is. That is the single most likely thing an admin
/// will try first, so the preflight must say it in words rather than let a raw
/// `InvalidServiceMetadata()` revert arrive from a send.
///
/// Note the verdict is `couldNotRun` rather than `refused`: not answering `owner()` is how our READ
/// failed, and the preflight is never STRICTER than the chain - the contract's own guard stays the
/// real gate. The reason names the cause so the admin is not sent hunting a form error.
#[tokio::test]
async fn a_contract_with_no_owner_is_named_rather_than_left_to_revert() {
    let (state, chain, _v, _b) = hermetic_state();
    chain.set_factory_owner(PROVIDER_REGISTRY, HOSTED);
    chain.set_factory_generation(PROVIDER_REGISTRY, GENERATION, FACTORY, true);
    // Recognized as a clone, but answers neither getter - exactly a generation-1 issuer.
    chain.set_clone(FACTORY, SERVICE);
    let app = router(state);
    let tok = admin_token(&app).await;
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/admin/providers/{PID}/services/preflight"),
        Some(&tok),
        Some(serde_json::json!({ "serviceAddress": SERVICE })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["metadata"]["state"], "unavailable", "{b}");
    let reason = b["metadata"]["reason"].as_str().unwrap();
    assert!(
        reason.contains("no owner at all"),
        "must name the permanent property rather than imply a fixable form error: {reason}"
    );
    assert_eq!(b["verdict"], "couldNotRun", "never stricter than the chain: {b}");
}

/// An address no active factory claims is a DEFINITE refusal - the probe answered, and its answer is
/// evidence about the address. Distinct from the case above, where the probe could not be made.
#[tokio::test]
async fn an_address_no_factory_deployed_is_a_definite_refusal() {
    let (state, chain, _v, _b) = hermetic_state();
    chain.set_factory_owner(PROVIDER_REGISTRY, HOSTED);
    chain.set_factory_generation(PROVIDER_REGISTRY, GENERATION, FACTORY, true);
    let app = router(state);
    let tok = admin_token(&app).await;
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/admin/providers/{PID}/services/preflight"),
        Some(&tok),
        Some(serde_json::json!({ "serviceAddress": SERVICE })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["generation"]["state"], "none", "{b}");
    assert_eq!(b["verdict"], "refused", "{b}");
}

/// `expectedOwner` is a transaction GUARD, never a selector: a wrong value refuses the send, and it
/// can never cause a different owner to be stored.
#[tokio::test]
async fn a_wrong_expected_owner_refuses_the_attach_rather_than_redirecting_it() {
    let (app, tok, _chain) = seeded().await;
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/admin/providers/{PID}/services"),
        Some(&tok),
        Some(serde_json::json!({
            "serviceAddress": SERVICE,
            "generationId": GENERATION,
            "expectedOwner": SIGNER, // not the live owner
        })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_GATEWAY, "the chain refuses it: {b}");
    assert!(
        b["error"].as_str().unwrap().contains("UnexpectedServiceOwner"),
        "{b}"
    );
    // And nothing was attached.
    let (_s, b) = call(&app, "GET", &format!("/v1/admin/providers/{PID}/services"), Some(&tok), None).await;
    assert!(b["services"].as_array().unwrap().is_empty(), "{b}");
}

/// A second attach of the same address is refused before any gas is spent, and the refusal names the
/// correction path rather than leaving the admin to guess.
#[tokio::test]
async fn attaching_the_same_address_twice_is_refused_with_the_correction_path() {
    let (app, tok, _chain) = seeded().await;
    let (s, _b) = attach(&app, &tok).await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = attach(&app, &tok).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(b["error"].as_str().unwrap().contains("reassignServiceProvider"), "{b}");
}

/// A SECOND grant of the issue right names the SCOPE, not just the contract's error.
///
/// FOUND ON A LIVE WALK. The grant is keyed on the ADDRESS and carries no service, so the control
/// appears on every attached service row and granting from one row has already covered the rest. A
/// captain pressed it on his second row and got a bare "the contract refuses a no-op with NoChange()",
/// which is accurate about the revert and says nothing about why the second press was pointless. The
/// refusal now states the useful fact - one grant, every service, from any row - and carries the scope
/// as data so a client need not parse the sentence.
#[tokio::test]
async fn a_second_issue_right_grant_explains_the_scope_rather_than_naming_noChange() {
    let (app, tok, _chain) = seeded().await;
    let grant = |allowed: bool| {
        let app = app.clone();
        let tok = tok.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/v1/admin/rights/{SIGNER}/issue"),
                Some(&tok),
                Some(serde_json::json!({ "allowed": allowed })),
            )
            .await
        }
    };

    let (s, b) = grant(true).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (s, b) = grant(true).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    let err = b["error"].as_str().unwrap();
    // The useful fact, in the operator's terms.
    assert!(err.contains("already holds the issue right"), "{b}");
    assert!(err.contains("every service in effective standing"), "{b}");
    assert!(err.contains("from any service row"), "{b}");
    // And NOT the contract's error name, which is what told the captain nothing.
    assert!(!err.contains("NoChange"), "{b}");
    // The scope as data, so a renderer does not have to read the prose.
    assert_eq!(b["scope"], "address", "{b}");
    assert_eq!(b["coversEveryService"], true, "{b}");

    // THE WITHDRAW DIRECTION GETS ITS OWN SENTENCE. "already granted" and "was never granted" are
    // different facts with different next moves, and a shared message would describe one of them
    // wrongly - the same collapse this whole surface refuses everywhere else.
    let (s, b) = grant(false).await;
    assert_eq!(s, StatusCode::OK, "the withdraw is a real change: {b}");
    let (s, b) = grant(false).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    let err = b["error"].as_str().unwrap();
    assert!(err.contains("does not hold the issue right"), "{b}");
    assert!(err.contains("nothing to withdraw"), "{b}");
    assert!(!err.contains("already holds"), "{b}");
}

/// A capability log that could not be READ is its own state with its reason - never an empty holder
/// set, which would say nobody may issue on the strength of a read that never happened.
///
/// This needs its OWN failure switch: the realistic failure is the service `eth_call` answering while
/// a range-capping peer refuses the capability `eth_getLogs`, and a shared switch would fail the
/// service read first and collapse the route into a 502 before the `unavailable` arm is ever built.
#[tokio::test]
async fn an_unreadable_capability_log_is_a_failure_not_an_empty_holder_set() {
    let (app, tok, chain) = seeded().await;
    let (s, _b) = attach(&app, &tok).await;
    assert_eq!(s, StatusCode::OK);

    chain.set_failing_capability_log_reads(PROVIDER_REGISTRY, true);
    let (s, b) = call(&app, "GET", &format!("/v1/admin/providers/{PID}/services"), Some(&tok), None).await;
    // A 200 whose ISSUANCE arm is unavailable - the rest of the view still resolved.
    assert_eq!(s, StatusCode::OK, "{b}");
    let issuance = &b["services"][0]["issuance"];
    assert_eq!(issuance["state"], "unavailable", "{b}");
    assert!(
        issuance.get("entries").is_none(),
        "an unavailable read must carry NO entries key - it cannot be spread into a list as []: {b}"
    );
    assert!(issuance["reason"].as_str().unwrap().contains("RightsSet"), "{b}");
}

/// The verify axis is keyed by PURPOSE and takes no service at all, so it is never reported as a
/// property of one. The contract derives `verificationKey(purpose)` itself, so the wire carries the
/// RAW purpose - passing an already-derived key would derive twice and grant nothing.
#[tokio::test]
async fn verify_capability_is_keyed_by_purpose_and_carries_the_raw_purpose_word() {
    let (app, tok, _chain) = seeded().await;
    let (s, b) = call(
        &app,
        "POST",
        "/v1/admin/verifier-capabilities",
        Some(&tok),
        Some(serde_json::json!({ "purpose": "travel_check", "relayer": SIGNER, "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "executed", "{b}");
    let raw = admin_api::chain::purpose_key("travel_check");
    assert_eq!(b["purposeKey"], raw, "the RAW purpose, never verify_key: {b}");
    assert_ne!(
        b["purposeKey"].as_str().unwrap(),
        admin_api::chain::verify_key("travel_check"),
        "verify_key is the DERIVED key; sending it here would double-derive and grant nothing"
    );

    let (s, b) = call(&app, "GET", "/v1/admin/verifier-capabilities", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let offered = b["purposes"].as_array().unwrap();
    let p = offered.iter().find(|p| p["purpose"] == "travel_check").unwrap();
    assert_eq!(p["relayers"]["entries"][0]["holder"], SIGNER, "{b}");
    // The console can only grant what it OFFERS, and the offered set is hand-mirrored from the
    // labels in circulation (`stacks/owner/web/src/lib/consents.ts`). A label that exists nowhere
    // else grants under a key `canVerify` is never asked about, and a real one that is missing
    // cannot be granted from here at all - so both directions are checked against those literals.
    let labels: Vec<&str> = offered.iter().map(|p| p["purpose"].as_str().unwrap()).collect();
    for real in ["boarding_intake", "travel_check", "grooming_intake", "daycare_access", "service_animal"] {
        assert!(labels.contains(&real), "{real} is in circulation and must be grantable: {b}");
    }
    assert_eq!(labels.len(), 5, "no label the portals never use may be offered: {b}");
    // Granting a verify capability grants NO issuance: the two axes are orthogonal.
    let (_s, sv) = call(&app, "GET", &format!("/v1/admin/providers/{PID}/services"), Some(&tok), None).await;
    assert!(sv["services"].as_array().unwrap().is_empty(), "{sv}");
}

/// An EXPLICIT purpose word is validated, never handed straight to `parse_b256`, which coerces a
/// malformed value to the ZERO word. The contract refuses only a zero relayer, so such a grant would
/// succeed and land under a purpose no verifier ever reads, while the response echoed the malformed
/// string back as `purposeKey`.
#[tokio::test]
async fn a_malformed_explicit_purpose_is_refused_rather_than_coerced_to_the_zero_word() {
    let (app, tok, _chain) = seeded().await;
    for bad in [format!("0x{}", "z".repeat(64)), format!("0x{}", "0".repeat(64))] {
        let (s, b) = call(
            &app,
            "POST",
            "/v1/admin/verifier-capabilities",
            Some(&tok),
            Some(serde_json::json!({ "purpose": bad, "relayer": SIGNER, "allowed": true })),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad} must be refused: {b}");
        assert!(b.get("actions").is_none(), "nothing may be dispatched for {bad}: {b}");
    }

    // ...and the guard is not over-broad: a well-formed explicit word still passes through as the
    // RAW purpose, unchanged.
    let explicit = admin_api::chain::purpose_key("travel_check");
    let (s, b) = call(
        &app,
        "POST",
        "/v1/admin/verifier-capabilities",
        Some(&tok),
        Some(serde_json::json!({ "purpose": explicit, "relayer": SIGNER, "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["purposeKey"], explicit, "an explicit word passes through unchanged: {b}");
}

/// Resolver approval is the fleet-wide half; the provider's SELECTION is the other, and the response
/// says so rather than reading as done.
#[tokio::test]
async fn resolver_approval_reports_that_selection_still_follows() {
    let (app, tok, chain) = seeded().await;
    // `setResolverApproved` refuses a non-contract; the fake's clone set stands in for "has code".
    chain.set_clone(FACTORY, SERVICE);
    let (s, b) = call(
        &app,
        "POST",
        "/v1/admin/resolvers",
        Some(&tok),
        Some(serde_json::json!({ "kind": "directory", "resolver": SERVICE, "approved": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "executed", "{b}");
    assert!(
        b["nextStep"].as_str().unwrap().contains("select"),
        "approval alone resolves nothing: {b}"
    );

    let (s, b) = call(&app, "GET", "/v1/admin/resolvers", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let dir = b["kinds"].as_array().unwrap().iter().find(|k| k["kind"] == "directory").unwrap();
    assert_eq!(dir["listing"]["resolvers"][0]["resolver"], SERVICE, "{b}");
    assert_eq!(dir["listing"]["resolvers"][0]["approved"], true);
}

/// A pulled resolver KEEPS its listing slot with `approved: false` - "approved then pulled" is a
/// different fact from "never approved", and only the flag separates them.
#[tokio::test]
async fn a_pulled_resolver_is_reported_rather_than_dropped() {
    let (app, tok, chain) = seeded().await;
    chain.set_clone(FACTORY, SERVICE);
    for approved in [true, false] {
        let (s, b) = call(
            &app,
            "POST",
            "/v1/admin/resolvers",
            Some(&tok),
            Some(serde_json::json!({ "kind": "domain", "resolver": SERVICE, "approved": approved })),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }
    let (_s, b) = call(&app, "GET", "/v1/admin/resolvers", Some(&tok), None).await;
    let dom = b["kinds"].as_array().unwrap().iter().find(|k| k["kind"] == "domain").unwrap();
    assert_eq!(dom["listing"]["resolvers"][0]["resolver"], SERVICE, "{b}");
    assert_eq!(dom["listing"]["resolvers"][0]["approved"], false, "kept, not dropped: {b}");
}

// ---------------------------------------------------------------------------------------------
// RE-HOMED from the deleted whitelist console: `dispatch_summary`'s tri-state `outcome`.
//
// The console is gone but the logic is not - every registrar route reports through it, and the two
// "nothing was broadcast" outcomes have entirely different remedies. Deleting these with the console
// would have been a silent coverage regression on shared logic.
// ---------------------------------------------------------------------------------------------

/// Propose-only is DECLARED, so a grant that broadcasts nothing is the CORRECT outcome. It must read
/// calmly and must never say the hosted key is wrong - that text belongs to the undeclared case only.
#[tokio::test]
async fn a_declared_propose_only_grant_is_reported_by_design() {
    let (state, _chain, _v, _b) = hermetic_state_propose_only();
    // Deliberately NO `set_factory_owner`: the hosted key does not own the registry.
    let app = router(state);
    let tok = admin_token(&app).await;
    let (s, b) = call(
        &app,
        "POST",
        "/v1/admin/verifier-capabilities",
        Some(&tok),
        Some(serde_json::json!({ "purpose": "travel_check", "relayer": SIGNER, "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["actions"][0]["disposition"], "proposed", "{b}");
    assert_eq!(b["outcome"], "proposed_by_design", "{b}");
    assert_eq!(b["executed"], false, "still nothing on-chain");
    let w = b["warning"].as_str().unwrap();
    assert!(
        !w.contains("wrong key"),
        "a declared propose-only deployment must not be told its key is wrong: {w}"
    );
    assert!(w.contains("EXTERNAL SIGNING"), "{w}");
}

/// Nothing reached the chain and propose-only was NOT declared: only THEN may the response say the
/// hosted key was expected to hold the authority and does not.
#[tokio::test]
async fn an_undeclared_proposal_is_reported_as_the_wrong_key_case() {
    let (state, _chain, _v, _b) = hermetic_state();
    let app = router(state);
    let tok = admin_token(&app).await;
    let (s, b) = call(
        &app,
        "POST",
        "/v1/admin/verifier-capabilities",
        Some(&tok),
        Some(serde_json::json!({ "purpose": "travel_check", "relayer": SIGNER, "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["actions"][0]["disposition"], "proposed", "{b}");
    assert_eq!(b["outcome"], "proposed_unauthorized", "{b}");
    assert_eq!(b["executed"], false);
    let w = b["warning"].as_str().unwrap();
    assert!(w.contains("UNCHANGED"), "must state nothing landed: {w}");
    assert!(w.contains("wrong key"), "{w}");
}

/// A landed tx OUTRANKS the propose-only declaration. Catches an implementation that consults the
/// declaration before checking whether anything was actually broadcast.
#[tokio::test]
async fn a_landed_tx_outranks_the_propose_only_declaration() {
    let (state, chain, _v, _b) = hermetic_state_propose_only();
    // Here the hosted key DOES own the registry, so the action executes despite the declaration.
    chain.set_factory_owner(PROVIDER_REGISTRY, HOSTED);
    let app = router(state);
    let tok = admin_token(&app).await;
    let (s, b) = call(
        &app,
        "POST",
        "/v1/admin/verifier-capabilities",
        Some(&tok),
        Some(serde_json::json!({ "purpose": "travel_check", "relayer": SIGNER, "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["actions"][0]["disposition"], "executed", "{b}");
    assert_eq!(b["outcome"], "executed", "a real tx landed: {b}");
    assert_eq!(b["executed"], true);
    assert!(
        b["warning"].is_null(),
        "must not claim on-chain state is unchanged when a tx landed: {b}"
    );
}

/// Every read and write on this surface requires an admin session.
///
/// Each POST sends a WELL-FORMED body on purpose. Axum runs the `Json<T>` extractor BEFORE the
/// handler, so a body that fails to deserialize answers 422 without the auth check ever running -
/// which would make this test pass on the extractor rather than on the gate it is named for.
#[tokio::test]
async fn the_journey_routes_require_an_admin_session() {
    let (state, _c, _v, _b) = hermetic_state();
    let app = router(state);
    let attach_body = serde_json::json!({
        "serviceAddress": SERVICE, "generationId": GENERATION, "expectedOwner": CONTROLLER,
    });
    for (method, path, body) in [
        ("GET", format!("/v1/admin/providers/{PID}/services"), serde_json::Value::Null),
        ("POST", format!("/v1/admin/providers/{PID}/services"), attach_body),
        (
            "POST",
            format!("/v1/admin/providers/{PID}/services/preflight"),
            serde_json::json!({ "serviceAddress": SERVICE }),
        ),
        (
            "POST",
            format!("/v1/admin/services/{SERVICE}/standing"),
            serde_json::json!({ "standing": "active" }),
        ),
        (
            "POST",
            format!("/v1/admin/rights/{SIGNER}/issue"),
            serde_json::json!({ "allowed": true }),
        ),
        ("GET", "/v1/admin/verifier-capabilities".to_string(), serde_json::Value::Null),
        (
            "POST",
            "/v1/admin/verifier-capabilities".to_string(),
            serde_json::json!({ "purpose": "travel_check", "relayer": SIGNER, "allowed": true }),
        ),
        ("GET", "/v1/admin/resolvers".to_string(), serde_json::Value::Null),
        (
            "POST",
            "/v1/admin/resolvers".to_string(),
            serde_json::json!({ "kind": "directory", "resolver": SERVICE, "approved": true }),
        ),
    ] {
        let payload = if body.is_null() { None } else { Some(body) };
        let (s, _b) = call(&app, method, &path, None, payload).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {path} must require admin");
    }
}

/// The deleted whitelist console's routes are GONE - not merely unlinked from the nav.
///
/// It called `isWhitelistedFor`, which the single authority answers off an orthogonal axis (a
/// definite `false` for every genuine issuer signer), and `whitelistFor`, which it does not
/// implement at all. There was nothing to repair, so the door is bricked up rather than left ajar.
#[tokio::test]
async fn the_whitelist_console_routes_no_longer_exist() {
    let (state, _c, _v, _b) = hermetic_state();
    let app = router(state);
    let tok = admin_token(&app).await;
    for path in ["/v1/admin/whitelist/grant", "/v1/admin/whitelist/revoke"] {
        let (s, b) = call(
            &app,
            "POST",
            path,
            Some(&tok),
            Some(serde_json::json!({ "signer": SIGNER, "recordType": "VACCINATION" })),
        )
        .await;
        // Axum's route-miss is a 404 with an EMPTY body, while every handler here answers
        // `{"error": ...}`. Asserting both is what distinguishes "not mounted" from "the handler
        // said 404" - the same rule `role_gating.rs` records for the vet stack.
        assert_eq!(s, StatusCode::NOT_FOUND, "{path} must not be routed: {b}");
        assert!(b.is_null() || b.get("error").is_none(), "route-miss, not a handler 404: {b}");
    }
}
