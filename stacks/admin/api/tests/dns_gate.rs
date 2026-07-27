//! The DNS legitimacy gate: three real outcomes, and a policy flag that changes only whether a
//! non-verified outcome BLOCKS.
//!
//! This suite exists because of a specific defect. `main.rs` used to wire a `MockDnsChecker` returning
//! an unconditional `Ok(true)` whenever `DNS_CHECK=skip` was set — a FABRICATED pass on the very gate
//! that decides whether an organisation is legitimate enough to be whitelisted, indistinguishable in
//! the response from a domain that had really published the record.
//!
//! The load-bearing guard is [`non_enforcing_reports_the_real_absence_instead_of_a_pass`]: a
//! non-enforcing deployment must report what DNS actually said. If someone reintroduces a synthesized
//! pass, that test fails rather than the behaviour silently shipping.

use axum::http::StatusCode;
use std::sync::Arc;

mod common;
use admin_api::dns::DnsChecker;
use common::{admin_token, call, hermetic_state_with_dns, MockDnsChecker};

const VET_ADDR: &str = "0x70997970c51812dc3a010c7d01b50e0d17dc79c8";

/// Submit a pending issuer-application and return its id.
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

async fn approve(dns: Arc<dyn DnsChecker>, enforce: bool) -> (StatusCode, serde_json::Value) {
    let (state, _chain) = hermetic_state_with_dns(dns, enforce);
    let app = admin_api::router(state);
    let admin = admin_token(&app).await;
    let id = submit(&app).await;
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/issuer-applications/{id}/approve"),
        Some(&admin),
        Some(serde_json::json!({})),
    )
    .await;
    (s, b)
}

// -------------------------------------------------------------------------------------------------
// Enforcing (the default, DNS_CHECK=doh)
// -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn enforcing_approves_when_the_record_is_published() {
    let (s, b) = approve(Arc::new(MockDnsChecker::ok()), true).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["dnsCheck"], "published");
    assert!(
        !b["whitelistTxs"].as_array().unwrap().is_empty(),
        "a published record whitelists the signer"
    );
}

#[tokio::test]
async fn enforcing_refuses_when_the_record_is_definitively_absent() {
    let (s, b) = approve(Arc::new(MockDnsChecker::not_published()), true).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert!(
        b.get("whitelistTxs").is_none(),
        "nothing is whitelisted when the gate refuses: {b}"
    );
}

/// A lookup that did not resolve must NOT be reported as an absence (403) — it is a gateway failure,
/// because the check did not run. Distinguishing these two is the whole point of the trait's contract.
#[tokio::test]
async fn enforcing_reports_a_failed_lookup_as_a_gateway_error_not_a_refusal() {
    let (s, b) = approve(Arc::new(MockDnsChecker::could_not_check()), true).await;
    assert_eq!(
        s,
        StatusCode::BAD_GATEWAY,
        "a non-answer is not the same as an absence: {b}"
    );
}

// -------------------------------------------------------------------------------------------------
// Non-enforcing (DNS_CHECK=observe / the former `skip`)
// -------------------------------------------------------------------------------------------------

/// THE regression guard. Before this change, a non-enforcing deployment answered as though the domain
/// had published the record. It must now say the record was absent and that it proceeded anyway.
#[tokio::test]
async fn non_enforcing_reports_the_real_absence_instead_of_a_pass() {
    let (s, b) = approve(Arc::new(MockDnsChecker::not_published()), false).await;
    assert_eq!(s, StatusCode::OK, "non-enforcing proceeds: {b}");
    assert_eq!(
        b["dnsCheck"], "notPublished",
        "the REAL outcome is reported; a synthesized \"published\" here is the defect this guards"
    );
    assert_ne!(b["dnsCheck"], "published");
}

#[tokio::test]
async fn non_enforcing_reports_a_failed_lookup_as_could_not_check() {
    let (s, b) = approve(Arc::new(MockDnsChecker::could_not_check()), false).await;
    assert_eq!(s, StatusCode::OK, "non-enforcing proceeds: {b}");
    assert_eq!(b["dnsCheck"], "couldNotCheck");
    assert_ne!(
        b["dnsCheck"], "notPublished",
        "a resolver failure is not evidence of absence"
    );
}

/// All three outcomes must be mutually distinguishable in the response. A single collapsed pair would
/// be enough to reintroduce the bug.
#[tokio::test]
async fn the_three_outcomes_are_all_distinct_on_the_wire() {
    let (_s1, published) = approve(Arc::new(MockDnsChecker::ok()), false).await;
    let (_s2, absent) = approve(Arc::new(MockDnsChecker::not_published()), false).await;
    let (_s3, unknown) = approve(Arc::new(MockDnsChecker::could_not_check()), false).await;

    let seen = [
        published["dnsCheck"].as_str().unwrap(),
        absent["dnsCheck"].as_str().unwrap(),
        unknown["dnsCheck"].as_str().unwrap(),
    ];
    assert_eq!(seen, ["published", "notPublished", "couldNotCheck"]);
    let unique: std::collections::HashSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 3, "no two outcomes may share a wire value");
}
