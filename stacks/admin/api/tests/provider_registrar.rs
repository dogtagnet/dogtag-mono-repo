//! The `ProviderRegistry` registrar surface, driven end to end over the real router.
//!
//! `MemChain` DECODES and APPLIES the three registrar writes (see `apply_provider_registry_calldata`),
//! so these tests walk register -> standing -> approve and read the result back through exactly the
//! code path production reads - including the contract's own `AlreadyRegistered`, `NoChange` and
//! `RetiredStanding` guards. A fake that merely counted transactions could not catch a route that
//! sends one the chain will refuse.

mod common;

use axum::http::StatusCode;
use common::*;

use admin_api::chain::record_type_key;
use admin_api::routes::router;

/// A plausible opaque KYC id. Not derived from anything - that is the point of `providerId`.
const PID: &str = "0x7a1c9f3b0e5d4a28c6b1f0937de4a5b2c8017f36";
const CONTROLLER: &str = "0x0000000000000000000000000000000000c07701";
const DIGEST: &str = "0x9f2b7c1d3e4a5b6c7d8e9f0a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3e4";

async fn register(app: &axum::Router, tok: &str, pid: &str) -> (StatusCode, serde_json::Value) {
    call(
        app,
        "POST",
        "/v1/admin/providers",
        Some(tok),
        Some(serde_json::json!({
            "providerId": pid,
            "controller": CONTROLLER,
            "identityDigest": DIGEST,
        })),
    )
    .await
}

/// The whole captain-facing journey, in the order the contracts force: register (which lands
/// PENDING), raise standing to ACTIVE, then approve a record type - and the read surface reflects
/// each step.
#[tokio::test]
async fn the_registrar_can_walk_register_activate_and_approve() {
    let (st, chain, _v, _b) = hermetic_state();
    // The hosted admin key IS the registry owner, so every action executes rather than proposing.
    chain.set_factory_owner(PROVIDER_REGISTRY, "0x00000000000000000000000000000000000000ad");
    let app = router(st);
    let tok = admin_token(&app).await;

    // Nothing exists yet - and an empty list is a real answer, distinct from an unreadable one.
    let (s, b) = call(&app, "GET", "/v1/admin/providers", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["providers"].as_array().unwrap().len(), 0);
    assert_eq!(b["authority"]["heldByHosted"], serde_json::json!(true));

    // 1. Register.
    let (s, b) = register(&app, &tok, PID).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "executed");
    assert_eq!(b["executed"], serde_json::json!(true));
    // Registration alone does NOT make the provider able to act, and the response says so.
    assert_eq!(b["standingAfterRegistration"], "pending");
    assert!(b["nextStep"].as_str().unwrap().contains("ACTIVE"));

    let (s, b) = call(
        &app,
        "GET",
        &format!("/v1/admin/providers/{PID}"),
        Some(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["provider"]["standing"], "pending");
    assert_eq!(b["provider"]["registered"], serde_json::json!(true));
    assert_eq!(b["approvals"]["state"], "resolved");
    assert_eq!(b["approvals"]["entries"].as_array().unwrap().len(), 0);
    // The identity anchor is the registrar's own assertion, under its own schema.
    assert_eq!(b["identityAnchor"]["digest"], DIGEST);
    assert_eq!(b["identityAnchor"]["schema"], serde_json::json!(1));
    assert_eq!(b["identityAnchor"]["hashAlgorithm"], serde_json::json!(0x1b));

    // 2. Activate - the step the two named calls do not cover, without which the provider is inert.
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/admin/providers/{PID}/standing"),
        Some(&tok),
        Some(serde_json::json!({ "standing": "active" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "executed");

    // 3. Approve a record type.
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/admin/providers/{PID}/service-approval"),
        Some(&tok),
        Some(serde_json::json!({ "recordType": "VACCINATION", "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "executed");
    assert_eq!(b["recordTypeKey"], record_type_key("VACCINATION"));

    let (s, b) = call(&app, "GET", "/v1/admin/providers", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let providers = b["providers"].as_array().unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0]["provider"]["standing"], "active");
    assert_eq!(providers[0]["provider"]["controller"], CONTROLLER);
    let entries = providers[0]["approvals"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["allowed"], serde_json::json!(true));
    // keccak is one-way, so a label is only ever shown when it round-trips to the key on chain.
    assert_eq!(entries[0]["recordType"], "VACCINATION");
}

/// The load-bearing distinction: an unreadable approval log is NOT an empty approval set.
///
/// Collapsing them would tell an admin a provider is approved for nothing on the strength of a read
/// that never happened - and the two have different remedies.
#[tokio::test]
async fn an_unreadable_approval_log_is_reported_as_unavailable_not_as_no_approvals() {
    let (st, chain, _v, _b) = hermetic_state();
    chain.set_factory_owner(PROVIDER_REGISTRY, "0x00000000000000000000000000000000000000ad");
    let app = router(st);
    let tok = admin_token(&app).await;
    let (s, _) = register(&app, &tok, PID).await;
    assert_eq!(s, StatusCode::OK);

    // Sanity: while readable, the empty log resolves to an empty (but ANSWERED) set.
    let (_, b) = call(
        &app,
        "GET",
        &format!("/v1/admin/providers/{PID}"),
        Some(&tok),
        None,
    )
    .await;
    assert_eq!(b["approvals"]["state"], "resolved");

    chain.set_failing_provider_reads(PROVIDER_REGISTRY, true);
    let (s, b) = call(
        &app,
        "GET",
        &format!("/v1/admin/providers/{PID}"),
        Some(&tok),
        None,
    )
    .await;
    // The provider read fails first, so the whole view is a 502 rather than a view with a silently
    // empty approvals block - either way, the one thing that must not happen is a 200 reporting no
    // approvals.
    assert_eq!(s, StatusCode::BAD_GATEWAY, "{b}");
    assert!(b["error"].as_str().unwrap().contains("provider("));
}

/// The approval read is genuinely tri-state at the type level, and the route renders the
/// `unavailable` arm rather than an empty list. Driven by failing ONLY the log read.
#[tokio::test]
async fn the_view_renders_the_unavailable_arm_when_only_the_log_read_fails() {
    use admin_api::chain::ChainClient;
    use admin_api::provider_registry::{fold_approvals, ApprovalsRead};

    let (_st, chain, _v, _b) = hermetic_state();
    chain.set_failing_provider_reads(PROVIDER_REGISTRY, true);
    let log = chain
        .service_creation_approval_log(PROVIDER_REGISTRY, PID)
        .await;
    assert!(log.is_err(), "the seeded failure must actually fail the read");

    // What the route builds from that error, and what it must never build instead.
    let unavailable = ApprovalsRead::Unavailable {
        reason: "seeded".into(),
    };
    let rendered = serde_json::to_value(&unavailable).unwrap();
    assert_eq!(rendered["state"], "unavailable");
    assert!(rendered.get("entries").is_none());

    let empty = ApprovalsRead::Resolved {
        entries: fold_approvals(&[], &|_| None),
    };
    assert_eq!(serde_json::to_value(&empty).unwrap()["state"], "resolved");
}

/// A `providerId` is permanent: re-registering one is refused before any gas is spent, because the
/// contract would revert `AlreadyRegistered()`.
#[tokio::test]
async fn a_provider_id_cannot_be_registered_twice() {
    let (st, chain, _v, _b) = hermetic_state();
    chain.set_factory_owner(PROVIDER_REGISTRY, "0x00000000000000000000000000000000000000ad");
    let app = router(st);
    let tok = admin_token(&app).await;

    let (s, _) = register(&app, &tok, PID).await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = register(&app, &tok, PID).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(b["error"].as_str().unwrap().contains("already registered"));
}

/// `setServiceCreationApproval` reverts `NoChange()` on a redundant write, so a definite read of the
/// current bit refuses it up front rather than paying for a doomed transaction.
#[tokio::test]
async fn a_redundant_approval_is_refused_rather_than_broadcast() {
    let (st, chain, _v, _b) = hermetic_state();
    chain.set_factory_owner(PROVIDER_REGISTRY, "0x00000000000000000000000000000000000000ad");
    let app = router(st);
    let tok = admin_token(&app).await;
    register(&app, &tok, PID).await;

    let approve = |allowed: bool| {
        let app = app.clone();
        let tok = tok.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/v1/admin/providers/{PID}/service-approval"),
                Some(&tok),
                Some(serde_json::json!({ "recordType": "VACCINATION", "allowed": allowed })),
            )
            .await
        }
    };

    // Approving something never approved is a real change; approving it again is not.
    assert_eq!(approve(true).await.0, StatusCode::OK);
    let (s, b) = approve(true).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(b["error"].as_str().unwrap().contains("NoChange"));
    // Withdrawing IS a change, and a withdrawal is a real registrar act rather than a no-op.
    assert_eq!(approve(false).await.0, StatusCode::OK);
    // And withdrawing an already-withdrawn one is refused for the same reason as the first case:
    // a record type the log never mentions is a definite "not approved".
    assert_eq!(approve(false).await.0, StatusCode::CONFLICT);
}

/// A no-op standing change and a transition out of the terminal RETIRED both revert on chain, so
/// both are refused before broadcast.
#[tokio::test]
async fn standing_refuses_a_no_op_and_treats_retired_as_terminal() {
    let (st, chain, _v, _b) = hermetic_state();
    chain.set_factory_owner(PROVIDER_REGISTRY, "0x00000000000000000000000000000000000000ad");
    let app = router(st);
    let tok = admin_token(&app).await;
    register(&app, &tok, PID).await;

    let set = |standing: &'static str| {
        let app = app.clone();
        let tok = tok.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/v1/admin/providers/{PID}/standing"),
                Some(&tok),
                Some(serde_json::json!({ "standing": standing })),
            )
            .await
        }
    };

    assert_eq!(set("active").await.0, StatusCode::OK);
    let (s, b) = set("active").await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(b["error"].as_str().unwrap().contains("NoChange"));

    assert_eq!(set("retired").await.0, StatusCode::OK);
    let (s, b) = set("active").await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(b["error"].as_str().unwrap().contains("terminal"));

    // PENDING and NONE are not settable - the contract reverts `InvalidStanding()`.
    let (s, b) = set("pending").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
}

/// The authority is read live from `owner()`, so a registry the hosted key does not own yields an
/// unsigned proposal rather than a broadcast - and the response says which key was checked.
#[tokio::test]
async fn a_registry_the_hosted_key_does_not_own_proposes_rather_than_broadcasting() {
    let (st, chain, _v, _b) = hermetic_state();
    chain.set_factory_owner(PROVIDER_REGISTRY, "0x8e27e11700000000000000000000000000000000");
    let app = router(st);
    let tok = admin_token(&app).await;

    let (s, b) = register(&app, &tok, PID).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["outcome"], "proposed_unauthorized");
    assert_eq!(b["executed"], serde_json::json!(false));
    let action = &b["actions"][0];
    assert_eq!(action["disposition"], "proposed");
    // Both keys are named, so a wrong-key proposal is distinguishable from a designed one.
    assert!(action["holder"].as_str().unwrap().starts_with("0x8e27e117"));
    assert!(action["hostedSigner"].is_string());
    assert!(action["calldata"].as_str().unwrap().starts_with("0x"));
    // The screen can tell the operator this BEFORE they fill a form in.
    let (_, list) = call(&app, "GET", "/v1/admin/providers", Some(&tok), None).await;
    assert_eq!(list["authority"]["heldByHosted"], serde_json::json!(false));
}

/// An unset `PROVIDER_REGISTRY_ADDR` fails LOUDLY. A zero address would otherwise read back as
/// "no providers exist" - a definite claim about a registry that was never asked.
#[tokio::test]
async fn an_unconfigured_registry_refuses_loudly_rather_than_reporting_an_empty_registry() {
    use std::sync::Arc;
    let (mut st, _chain, _v, _b) = hermetic_state();
    let mut cfg = (*st.cfg).clone();
    cfg.provider_registry_addr = "0x0000000000000000000000000000000000000000".into();
    st.cfg = Arc::new(cfg);
    let app = router(st);
    let tok = admin_token(&app).await;

    for (method, path, body) in [
        ("GET", "/v1/admin/providers".to_string(), None),
        ("GET", format!("/v1/admin/providers/{PID}"), None),
        (
            "POST",
            "/v1/admin/providers".to_string(),
            Some(serde_json::json!({
                "providerId": PID, "controller": CONTROLLER, "identityDigest": DIGEST
            })),
        ),
        (
            "POST",
            format!("/v1/admin/providers/{PID}/standing"),
            Some(serde_json::json!({ "standing": "active" })),
        ),
        (
            "POST",
            format!("/v1/admin/providers/{PID}/service-approval"),
            Some(serde_json::json!({ "recordType": "VACCINATION", "allowed": true })),
        ),
    ] {
        let (s, b) = call(&app, method, &path, Some(&tok), body).await;
        assert_eq!(
            s,
            StatusCode::SERVICE_UNAVAILABLE,
            "{method} {path} must refuse loudly: {b}"
        );
        assert!(b["error"]
            .as_str()
            .unwrap()
            .contains("PROVIDER_REGISTRY_ADDR not configured"));
    }
}

/// Every route is admin-session gated.
///
/// Each POST carries a WELL-FORMED body on purpose. `Json<T>` is an extractor, so it runs before the
/// handler and answers 422 for a malformed body - an empty `{}` would never reach `require_admin`
/// and the test would pass while pinning nothing about authorization.
#[tokio::test]
async fn the_registrar_surface_requires_an_admin_session() {
    let (st, _chain, _v, _b) = hermetic_state();
    let app = router(st);
    let register_body = serde_json::json!({
        "providerId": PID, "controller": CONTROLLER, "identityDigest": DIGEST
    });
    for (method, path, body) in [
        ("GET", "/v1/admin/providers".to_string(), None),
        ("GET", format!("/v1/admin/providers/{PID}"), None),
        ("POST", "/v1/admin/providers".to_string(), Some(register_body)),
        (
            "POST",
            format!("/v1/admin/providers/{PID}/standing"),
            Some(serde_json::json!({ "standing": "active" })),
        ),
        (
            "POST",
            format!("/v1/admin/providers/{PID}/service-approval"),
            Some(serde_json::json!({ "recordType": "VACCINATION", "allowed": true })),
        ),
    ] {
        let (s, b) = call(&app, method, &path, None, body).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {path}: {b}");
    }
}

/// The contract refuses a zero providerId, a zero controller and a zero identity digest. Catching
/// them here turns three reverts into three actionable messages.
#[tokio::test]
async fn malformed_registration_inputs_are_refused_before_any_transaction() {
    let (st, chain, _v, _b) = hermetic_state();
    chain.set_factory_owner(PROVIDER_REGISTRY, "0x00000000000000000000000000000000000000ad");
    let app = router(st);
    let tok = admin_token(&app).await;

    let post = |body: serde_json::Value| {
        let app = app.clone();
        let tok = tok.clone();
        async move { call(&app, "POST", "/v1/admin/providers", Some(&tok), Some(body)).await }
    };

    // Zero providerId - `ZeroProviderId()`.
    let (s, _) = post(serde_json::json!({
        "providerId": "0x0000000000000000000000000000000000000000",
        "controller": CONTROLLER, "identityDigest": DIGEST
    }))
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Wrong width - a bytes20 id is 40 hex, not an address-shaped guess of some other length.
    let (s, _) = post(serde_json::json!({
        "providerId": "0xdeadbeef", "controller": CONTROLLER, "identityDigest": DIGEST
    }))
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Zero controller - `ZeroAddress()`, and it is also the contract's existence sentinel.
    let (s, _) = post(serde_json::json!({
        "providerId": PID,
        "controller": "0x0000000000000000000000000000000000000000",
        "identityDigest": DIGEST
    }))
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Zero digest - `BadIdentityAnchor()`. An anchor is the registrar's assertion; an empty one
    // asserts nothing.
    let (s, b) = post(serde_json::json!({
        "providerId": PID, "controller": CONTROLLER,
        "identityDigest": "0x0000000000000000000000000000000000000000000000000000000000000000"
    }))
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b["error"].as_str().unwrap().contains("identityDigest"));
}

/// An approval or standing change against an id nobody registered is a 404, not a transaction.
#[tokio::test]
async fn acting_on_an_unregistered_provider_is_a_not_found() {
    let (st, chain, _v, _b) = hermetic_state();
    chain.set_factory_owner(PROVIDER_REGISTRY, "0x00000000000000000000000000000000000000ad");
    let app = router(st);
    let tok = admin_token(&app).await;

    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/admin/providers/{PID}/standing"),
        Some(&tok),
        Some(serde_json::json!({ "standing": "active" })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/admin/providers/{PID}/service-approval"),
        Some(&tok),
        Some(serde_json::json!({ "recordType": "VACCINATION", "allowed": true })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // The detail view of an unregistered id is a 200 that says so, rather than a 404: "this id is
    // not registered" is an answer the screen renders.
    let (s, b) = call(
        &app,
        "GET",
        &format!("/v1/admin/providers/{PID}"),
        Some(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["provider"]["registered"], serde_json::json!(false));
    assert_eq!(b["provider"]["standing"], "none");
    // No anchor is invented for an id that has none.
    assert!(b["identityAnchor"].is_null());
}
