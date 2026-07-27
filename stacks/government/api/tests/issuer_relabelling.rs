//! The relabelling attack from audit-m9, and the two independent things that now defeat it.
//!
//! # The attack, as demonstrated live in the audit
//!
//! `check_integrity` hashes only `data` (flattened) plus `privacy.obfuscated`. The top-level `issuer`
//! block — `name`, `domain`, `documentStore` — is OUTSIDE the Merkle root. So the auditor took a genuine
//! VALID credential, changed nothing in `data`, relabelled `issuer.name` to "Ministry of Health of
//! Singapore" and `issuer.domain` to `moh.gov.sg`, and `POST /v1/verify` still answered
//! `{"verdict":true}` — a fabricated issuing authority with a passing verdict.
//!
//! # Why ONE defence is not enough
//!
//! There are two separable relabels, and they need different evidence:
//!
//!   * **domain relabel** — caught by asserting `issuer.domain` against the root-covered `data.issuer`
//!     DID. Fails the verdict outright ([`relabelled_domain_fails_the_verdict`]).
//!   * **name-only relabel** — NOT caught by that assertion, because `data.issuer` is a `did:web:`
//!     value and carries a DOMAIN, nothing else. Leave the domain genuine and only rewrite the name and
//!     the DID assertion says `match`, the DNS binding says `verified` (the genuine domain really does
//!     publish the record), and a naive UI renders the fabricated authority beside a green check —
//!     strictly worse than showing nothing. The defence is to display the clone's own on-chain `name()`,
//!     written by the factory's `onlyOwner` `createIssuer` at KYC time
//!     ([`relabelled_name_only_is_exposed_and_never_displayed_as_the_issuer`]).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

use government_api::app::{AppState, Config};
use government_api::chain::{ChainClient, MemChain};
use government_api::store::{MemStore, Store};

const CLONE: &str = "0xb5d6654d8b29096c8fcf71d24bbe6f6de86c5f9f";
const DOMAIN_REGISTRY: &str = "0x00000000000000000000000000000000000000dd";
/// The name the protocol multisig wrote into the clone at `createIssuer`.
const ONCHAIN_NAME: &str = "DogTag Government Authority";

fn state() -> (AppState, MemChain) {
    let chain = MemChain::new();
    let cfg = Config {
        deployment_url: "http://localhost:44832".into(),
        rpc_url: "https://devrpc.roax.net".into(),
        chain_id: 135,
        issuer_registry_addr: "0x5d86e4cf98a34ae0576f190f8d209c2943a9c79c".into(),
        issuer_domain_registry_addr: DOMAIN_REGISTRY.into(),
        // No DoH endpoint: DNS therefore reports `couldNotCheck`, which is exactly the honest state for
        // a hermetic test and keeps this suite about the RELABELLING, not about resolution.
        dns_doh_endpoint: String::new(),
        verification_registry_addr: "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87".into(),
        travel_clearance_issuer_addr: CLONE.into(),
        eu_health_cert_issuer_addr: "0x0000000000000000000000000000000000000000".into(),
        issuer_name: ONCHAIN_NAME.into(),
        issuer_domain: "gov.example".into(),
        demo: true,
        api_token: Some("test-gov-token".into()),
    };
    let st = AppState {
        store: Arc::new(MemStore::new()) as Arc<dyn Store>,
        chain: Arc::new(chain.clone()),
        cfg: Arc::new(cfg),
        dns: Arc::new(dogtag_dns_rs::BindingResolver::production(String::new())),
        feed: Arc::new(government_api::oversight::DisabledFeed),
    };
    (st, chain)
}

/// A genuine credential whose integrity ACTUALLY passes: `data.issuer` (root-covered) is
/// `did:web:gov.example`, the displayed `issuer` block agrees with it, and `signature.merkleRoot` is the
/// real recomputed root. `data` is never touched by any test here — that is the point: every forgery
/// below leaves integrity valid, which is exactly why the audit's attack worked.
fn genuine_doc() -> Value {
    let mut doc = raw_doc();
    // Stamp the REAL root so `check_integrity` passes. `check_integrity` returns the recomputed root
    // even when it reports Invalid, so one pass is enough to learn it; a single-document credential has
    // an empty `proof`, so `targetHash == merkleRoot == R`.
    let parsed: dogtag_standard::wrap::WrappedDoc = serde_json::from_value(doc.clone()).unwrap();
    let (_state, recomputed) = dogtag_standard::verify::check_integrity(&parsed);
    let root = dogtag_standard::to_hex32(&recomputed);
    doc["signature"]["targetHash"] = json!(root);
    doc["signature"]["merkleRoot"] = json!(root);

    // Sanity: this suite is only meaningful if the baseline document's integrity REALLY passes.
    let reparsed: dogtag_standard::wrap::WrappedDoc = serde_json::from_value(doc.clone()).unwrap();
    assert_eq!(
        dogtag_standard::verify::check_integrity(&reparsed).0,
        dogtag_standard::verify::FragmentState::Valid,
        "baseline fixture must have valid integrity"
    );
    doc
}

fn raw_doc() -> Value {
    json!({
        "version": "1.0",
        "data": {
            "issuer": "99998888777766665555444433332222:2:did:web:gov.example",
            "credentialSubject": {
                "dogTagId": "11112222333344445555666677778888:2:DT-TEST-0001",
                "name": "eeeeffff00001111aaaabbbbccccdddd:2:Max"
            }
        },
        "signature": {
            "type": "DogTagMerkleRoot",
            "targetHash": "0x0000000000000000000000000000000000000000000000000000000000000001",
            "proof": [],
            "merkleRoot": "0x0000000000000000000000000000000000000000000000000000000000000002"
        },
        "privacy": { "obfuscated": [] },
        "issuer": {
            "name": ONCHAIN_NAME,
            "domain": "gov.example",
            "documentStore": CLONE,
            "recordType": "TRAVEL_CLEARANCE"
        }
    })
}

/// Anchor `root` on the emulated chain through the real `issue` write path, so `isValid` is genuinely
/// true. The attack must be defeated against a REAL valid record, not a conveniently invalid one.
async fn issue_root(chain: &MemChain, root: &str) {
    let signer = chain.signer_address().expect("MemChain has an emulated signer");
    chain.whitelist(
        "0x5d86e4cf98a34ae0576f190f8d209c2943a9c79c",
        &government_api::app::record_type_key("TRAVEL_CLEARANCE"),
        &signer,
    );
    chain.issue(CLONE, root).await.expect("emulated issue");
}

async fn verify(app: &axum::Router, doc: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/v1/verify")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "wrapped_doc": doc })).unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Seed the chain so the record is genuinely issued+valid and the clone carries its real name/domain.
/// Everything the attack relies on is therefore true: only the document is forged.
async fn seed(chain: &MemChain, doc: &Value) {
    let root = doc["signature"]["merkleRoot"].as_str().unwrap();
    issue_root(chain, root).await;
    chain.set_onchain_name(CLONE, ONCHAIN_NAME);
    chain.set_claimed_domain(DOMAIN_REGISTRY, CLONE, "gov.example");
}

// -------------------------------------------------------------------------------------------------
// (1) domain relabel — the audit's exact payload
// -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn relabelled_domain_fails_the_verdict() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    // The audit's payload: `data` untouched, only the issuer block rewritten.
    let mut forged = doc.clone();
    forged["issuer"]["name"] = json!("Ministry of Health of Singapore");
    forged["issuer"]["domain"] = json!("moh.gov.sg");

    let (s, b) = verify(&app, forged).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        b["verdict"], false,
        "the audit's relabelled document must no longer verify: {b}"
    );
    assert_eq!(b["fragments"]["issuerDidAssertion"], "mismatch");
    assert_eq!(b["issuerIdentity"]["rootCoveredDomain"], "gov.example");
    assert_eq!(b["issuerIdentity"]["documentDomain"], "moh.gov.sg");
    assert_eq!(b["issuerIdentity"]["relabelled"], true);
    // Integrity genuinely still passes — which is precisely why a separate assertion was needed.
    assert_eq!(
        b["fragments"]["integrity"], true,
        "the forgery is invisible to the Merkle root; the DID assertion is what catches it"
    );
}

// -------------------------------------------------------------------------------------------------
// (2) name-only relabel — invisible to the DID assertion AND to DNS
// -------------------------------------------------------------------------------------------------

/// The residual attack the domain assertion cannot see. The verdict is NOT what saves us here (the
/// document is consistent with everything the root and DNS can prove); the display is. The response must
/// carry the authoritative on-chain name and flag the document's as differing.
#[tokio::test]
async fn relabelled_name_only_is_exposed_and_never_displayed_as_the_issuer() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let mut forged = doc.clone();
    forged["issuer"]["name"] = json!("Ministry of Health of Singapore");
    // domain deliberately left genuine — this is what makes the attack survive the DID assertion.

    let (s, b) = verify(&app, forged).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // The DID assertion and DNS are BOTH satisfied. That is the trap.
    assert_eq!(
        b["fragments"]["issuerDidAssertion"], "match",
        "a name-only relabel is invisible to a domain assertion — this is the whole point"
    );

    // The defence: the authoritative name comes from the clone, and the document's is flagged.
    assert_eq!(b["issuerIdentity"]["onchainName"], ONCHAIN_NAME);
    assert_eq!(b["issuerIdentity"]["onchainNameAvailable"], true);
    assert_eq!(b["issuerIdentity"]["documentName"], "Ministry of Health of Singapore");
    assert_eq!(
        b["issuerIdentity"]["documentNameDiffers"], true,
        "the fabricated name is reported as a discrepancy, not rendered as the issuer"
    );
    assert_eq!(b["issuerIdentity"]["relabelled"], true);
}

// -------------------------------------------------------------------------------------------------
// (3) the genuine article must stay clean — no crying wolf
// -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_genuine_document_verifies_with_no_discrepancy() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let (s, b) = verify(&app, doc).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["fragments"]["issuerDidAssertion"], "match");
    assert_eq!(b["issuerIdentity"]["relabelled"], false);
    assert_eq!(b["issuerIdentity"]["documentNameDiffers"], false);
    assert_eq!(b["issuerIdentity"]["documentDomainDiffers"], false);
}

/// A free-form label differing only by padding or case is not evidence of anything. Flagging it would
/// cry wolf on legitimate credentials, which trains operators to ignore the flag.
#[tokio::test]
async fn whitespace_and_case_differences_in_the_name_are_not_a_discrepancy() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let mut d = doc.clone();
    d["issuer"]["name"] = json!("  dogtag   GOVERNMENT authority ");
    let (_s, b) = verify(&app, d).await;
    assert_eq!(b["issuerIdentity"]["documentNameDiffers"], false);
    assert_eq!(b["issuerIdentity"]["relabelled"], false);
}

// -------------------------------------------------------------------------------------------------
// (4) the binding states stay honest
// -------------------------------------------------------------------------------------------------

/// With no DoH endpoint configured the DNS half genuinely cannot be resolved. That must surface as
/// `couldNotCheck` — never as `verified` and never as `notListed`.
#[tokio::test]
async fn an_unresolvable_dns_half_reports_could_not_check() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    let binding = &b["issuerDomainBinding"];
    assert_eq!(binding["state"], "couldNotCheck", "{binding}");
    assert_ne!(binding["state"], "verified");
    assert_ne!(binding["state"], "notListed");
    assert_eq!(
        binding["domain"], "gov.example",
        "the domain queried is the ON-CHAIN claim, not the document's: {binding}"
    );
}

/// An issuer that has published no domain claim is a NORMAL day-one state, and must be distinguishable
/// from both a failed lookup and an absent DNS record.
#[tokio::test]
async fn an_issuer_with_no_domain_claim_reports_no_domain_claimed() {
    let (st, chain) = state();
    let doc = genuine_doc();
    let root = doc["signature"]["merkleRoot"].as_str().unwrap();
    issue_root(&chain, root).await;
    chain.set_onchain_name(CLONE, ONCHAIN_NAME);
    // deliberately NO set_claimed_domain
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    assert_eq!(b["issuerDomainBinding"]["state"], "noDomainClaimed");
}

/// With no registry configured we do not KNOW anything about the binding. That is `unavailable`, not
/// "this issuer claims no domain".
#[tokio::test]
async fn no_configured_registry_reports_unavailable_not_absence() {
    let (mut st, chain) = state();
    let mut cfg = (*st.cfg).clone();
    cfg.issuer_domain_registry_addr = "0x0000000000000000000000000000000000000000".into();
    st.cfg = Arc::new(cfg);

    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    assert_eq!(b["issuerDomainBinding"]["state"], "unavailable");
    assert_ne!(b["issuerDomainBinding"]["state"], "noDomainClaimed");
}
