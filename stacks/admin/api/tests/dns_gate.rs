//! The DNS legitimacy gate: ADVISORY, with three real outcomes and a persistent trace.
//!
//! Two defects shaped this suite.
//!
//! 1. `main.rs` used to wire a `MockDnsChecker` returning an unconditional `Ok(true)` whenever
//!    `DNS_CHECK=skip` was set — a FABRICATED pass on the gate that decides whether an organisation is
//!    legitimate enough to be whitelisted, indistinguishable in the response from a domain that had
//!    really published the record.
//! 2. The gate then BLOCKED on a non-verified outcome, which is what makes operators reach for a
//!    bypass in the first place: an organisation is routinely KYC-approved days before its DNS team
//!    publishes anything.
//!
//! The design that replaces both: the lookup always runs for real and never blocks, but proceeding on a
//! non-verified observation requires the admin's EXPLICIT `proceedWithoutDns` and is RECORDED. An
//! override that leaves no trace would be the same fail-open with extra steps, so the trace is the part
//! under test here — see [`proceeding_records_an_immutable_trace_of_the_override`].

use axum::http::StatusCode;
use std::sync::Arc;

mod common;
use admin_api::dns::DnsChecker;
use common::{admin_token, call, hermetic_state_with_dns, MockDnsChecker};

const VET_ADDR: &str = "0x70997970c51812dc3a010c7d01b50e0d17dc79c8";

async fn submit(app: &axum::Router) -> String {
    let (s, b) = call(
        app,
        "POST",
        "/v1/issuer-applications",
        None,
        Some(serde_json::json!({
            "issuerEntityId": "bayview-vet",
            "addresses": [VET_ADDR],
            "recordTypes": ["VACCINATION"],
            "domain": "vet.example",
            "documentStore": "0x00000000000000000000000000000000000000cc",
            "usdaNan": "123456",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "submit application: {b}");
    b["applicationId"].as_str().unwrap().to_string()
}

/// Drive one approve attempt. `proceed` mirrors the admin ticking the explicit confirmation.
async fn approve_with(
    dns: Arc<dyn DnsChecker>,
    proceed: bool,
) -> (StatusCode, serde_json::Value, axum::Router, String) {
    let (state, _chain) = hermetic_state_with_dns(dns);
    let app = admin_api::router(state);
    let admin = admin_token(&app).await;
    let id = submit(&app).await;
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/issuer-applications/{id}/approve"),
        Some(&admin),
        Some(serde_json::json!({ "proceedWithoutDns": proceed })),
    )
    .await;
    (s, b, app, admin)
}

/// The application row as the dashboard reads it.
async fn application_row(app: &axum::Router, admin: &str) -> serde_json::Value {
    let (s, b) = call(app, "GET", "/v1/issuer-applications", Some(admin), None).await;
    assert_eq!(s, StatusCode::OK, "list applications: {b}");
    b["applications"][0].clone()
}

// -------------------------------------------------------------------------------------------------
// A clean pass needs no confirmation
// -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_published_record_approves_with_no_confirmation() {
    let (s, b, app, admin) = approve_with(Arc::new(MockDnsChecker::ok()), false).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["dnsState"], "verified");
    assert_eq!(
        b["dnsProceededUnverified"], false,
        "a clean pass is not an override"
    );
    assert!(!b["whitelistTxs"].as_array().unwrap().is_empty());

    let row = application_row(&app, &admin).await;
    assert_eq!(row["dnsState"], "verified");
    assert_eq!(row["dnsStateAtApproval"], "verified");
    assert_eq!(row["dnsProceededUnverified"], false);
}

// -------------------------------------------------------------------------------------------------
// Advisory: non-verified does not block, but needs a deliberate act
// -------------------------------------------------------------------------------------------------

/// Not a refusal — a request for confirmation, carrying what was OBSERVED so the UI can state the
/// observation rather than deliver a verdict about the organisation.
#[tokio::test]
async fn an_absent_record_asks_for_confirmation_rather_than_refusing() {
    let (s, b, _app, _admin) = approve_with(Arc::new(MockDnsChecker::not_published()), false).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["error"], "dnsConfirmationRequired");
    assert_eq!(b["dnsState"], "notListed");
    assert_eq!(b["domain"], "vet.example");
    assert!(
        b["expectedTxt"]
            .as_str()
            .unwrap()
            .starts_with("dogtag-verify="),
        "the prompt tells the admin exactly what the domain must publish: {b}"
    );
    assert_eq!(b["retryWith"]["proceedWithoutDns"], true);
}

#[tokio::test]
async fn a_failed_lookup_asks_for_confirmation_and_names_itself_as_unreachable() {
    let (s, b, _app, _admin) =
        approve_with(Arc::new(MockDnsChecker::could_not_check()), false).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(
        b["dnsState"], "couldNotCheck",
        "a resolver failure is not evidence of absence"
    );
    assert_ne!(b["dnsState"], "notListed");
}

#[tokio::test]
async fn the_admin_may_proceed_and_whitelist_anyway() {
    let (s, b, _app, _admin) = approve_with(Arc::new(MockDnsChecker::not_published()), true).await;
    assert_eq!(s, StatusCode::OK, "DNS must never block onboarding: {b}");
    assert!(
        !b["whitelistTxs"].as_array().unwrap().is_empty(),
        "the whitelist writes actually happened: {b}"
    );
}

// -------------------------------------------------------------------------------------------------
// The trace is what makes the override safe
// -------------------------------------------------------------------------------------------------

/// THE load-bearing guard. An override that leaves no record is fail-open with extra steps: the
/// dashboard must be able to tell this issuer apart from one that passed cleanly.
#[tokio::test]
async fn proceeding_records_an_immutable_trace_of_the_override() {
    let (s, b, app, admin) = approve_with(Arc::new(MockDnsChecker::not_published()), true).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        b["dnsState"], "notListed",
        "the REAL observation, not a pass"
    );
    assert_ne!(b["dnsState"], "verified");
    assert_eq!(b["dnsProceededUnverified"], true);
    assert!(
        b["dnsCheckedAt"].as_u64().unwrap() > 0,
        "the observation is timestamped"
    );

    let row = application_row(&app, &admin).await;
    assert_eq!(row["status"], "approved");
    assert_eq!(
        row["dnsStateAtApproval"], "notListed",
        "what we knew at the moment of granting is preserved"
    );
    assert_eq!(
        row["dnsProceededUnverified"], true,
        "\"whitelisted while DNS was unverified\" is legible to the dashboard"
    );
}

#[tokio::test]
async fn an_unreachable_resolver_override_is_traced_as_such_not_as_absence() {
    let (s, b, app, admin) = approve_with(Arc::new(MockDnsChecker::could_not_check()), true).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let row = application_row(&app, &admin).await;
    assert_eq!(row["dnsStateAtApproval"], "couldNotCheck");
    assert_ne!(
        row["dnsStateAtApproval"], "notListed",
        "the trace records which of the two non-verified outcomes actually occurred"
    );
}

/// The re-check payoff: `dnsState` is the mutable latest observation and `dnsStateAtApproval` is
/// immutable history, so a future daily cron can flip a binding to verified without an admin redoing
/// anything, while the override still remains visible. This pins the two fields as SEPARATE — a single
/// collapsed field would force the cron to erase the history in order to record the good news.
#[tokio::test]
async fn the_latest_state_and_the_approval_time_state_are_separate_fields() {
    let (s, _b, app, admin) = approve_with(Arc::new(MockDnsChecker::not_published()), true).await;
    assert_eq!(s, StatusCode::OK);

    let row = application_row(&app, &admin).await;
    assert!(
        row.get("dnsState").is_some() && row.get("dnsStateAtApproval").is_some(),
        "both fields are exposed: {row}"
    );
    assert!(
        row.get("dnsCheckedAt").is_some(),
        "the latest observation is timestamped so staleness is visible: {row}"
    );
}

/// All three outcomes must stay mutually distinguishable on the wire. One collapsed pair is enough to
/// reintroduce the original bug.
#[tokio::test]
async fn the_three_outcomes_are_all_distinct_on_the_wire() {
    let (_s1, verified, _, _) = approve_with(Arc::new(MockDnsChecker::ok()), true).await;
    let (_s2, absent, _, _) = approve_with(Arc::new(MockDnsChecker::not_published()), true).await;
    let (_s3, unknown, _, _) =
        approve_with(Arc::new(MockDnsChecker::could_not_check()), true).await;

    let seen = [
        verified["dnsState"].as_str().unwrap(),
        absent["dnsState"].as_str().unwrap(),
        unknown["dnsState"].as_str().unwrap(),
    ];
    assert_eq!(seen, ["verified", "notListed", "couldNotCheck"]);
    let unique: std::collections::HashSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 3, "no two outcomes may share a wire value");
}
