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
const FACTORY: &str = "0x00000000000000000000000000000000000000fa";
/// The name the protocol multisig wrote into the clone at `createIssuer`.
const ONCHAIN_NAME: &str = "DogTag Government Authority";

fn state() -> (AppState, MemChain) {
    let chain = MemChain::new();
    let cfg = Config {
        deployment_url: "http://localhost:44832".into(),
        rpc_url: "https://devrpc.roax.net".into(),
        chain_id: 135,
        issuer_registry_addr: "0x5d86e4cf98a34ae0576f190f8d209c2943a9c79c".into(),
        factory_addr: FACTORY.into(),
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
    let signer = chain
        .signer_address()
        .expect("MemChain has an emulated signer");
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
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Seed the chain so the record is genuinely issued+valid and the clone carries its real name/domain.
/// Everything the attack relies on is therefore true: only the document is forged.
async fn seed(chain: &MemChain, doc: &Value) {
    let root = doc["signature"]["merkleRoot"].as_str().unwrap();
    issue_root(chain, root).await;
    // Link 1: this clone really was deployed by the DogTag factory.
    chain.set_factory_clone(FACTORY, CLONE, true);
    chain.set_onchain_name(CLONE, ONCHAIN_NAME);
    // Link 2: the clone's own on-chain domain claim.
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
    assert_eq!(
        b["issuerIdentity"]["documentName"],
        "Ministry of Health of Singapore"
    );
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
    chain.set_factory_clone(FACTORY, CLONE, true);
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

// -------------------------------------------------------------------------------------------------
// (5) link 1 — factory provenance
// -------------------------------------------------------------------------------------------------

/// The attack the DNS binding alone cannot see: deploy your OWN contract, claim a domain for it,
/// publish a matching TXT record. DNS agrees. The domain registry agrees. And none of it means anything,
/// because that contract never passed through the KYC-gated `createIssuer`.
///
/// This must be its own state, never rendered as merely "not listed in DNS" — it is a far stronger
/// statement than a missing record.
#[tokio::test]
async fn a_contract_not_deployed_by_the_factory_is_reported_as_not_a_dogtag_issuer() {
    let (st, chain) = state();
    let doc = genuine_doc();
    let root = doc["signature"]["merkleRoot"].as_str().unwrap();
    issue_root(&chain, root).await;
    chain.set_onchain_name(CLONE, ONCHAIN_NAME);
    // The attacker HAS a domain claim and WOULD have a matching TXT record...
    chain.set_claimed_domain(DOMAIN_REGISTRY, CLONE, "gov.example");
    // ...but the contract was NOT deployed by the DogTag factory.
    chain.set_factory_clone(FACTORY, CLONE, false);
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    let binding = &b["issuerDomainBinding"];
    assert_eq!(binding["state"], "notADogTagIssuer", "{binding}");
    assert_ne!(
        binding["state"], "notListed",
        "a non-clone must never be softened into a missing-DNS-record observation"
    );
    assert_ne!(binding["state"], "verified");
    assert_ne!(binding["state"], "noDomainClaimed");
}

/// Provenance is re-checked at verification time, so link 1 short-circuits BEFORE the domain claim is
/// even read. A stored binding is a claim; the app does not inherit trust it did not verify.
#[tokio::test]
async fn provenance_is_checked_before_the_domain_claim_is_trusted() {
    let (st, chain) = state();
    let doc = genuine_doc();
    let root = doc["signature"]["merkleRoot"].as_str().unwrap();
    issue_root(&chain, root).await;
    chain.set_factory_clone(FACTORY, CLONE, false);
    // A stored claim exists for a non-clone (e.g. written before the registry enforced provenance, or
    // by a laxer registry the app was pointed at). It must not be honoured.
    chain.set_claimed_domain(DOMAIN_REGISTRY, CLONE, "attacker.example.com");
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    assert_eq!(b["issuerDomainBinding"]["state"], "notADogTagIssuer");
    assert!(
        b["issuerDomainBinding"].get("domain").is_none(),
        "the unverifiable claim is not even echoed back: {}",
        b["issuerDomainBinding"]
    );
}

/// A failed provenance READ is not "not a DogTag issuer" — we simply do not know.
#[tokio::test]
async fn no_configured_factory_reports_unavailable_not_a_provenance_failure() {
    let (mut st, chain) = state();
    let mut cfg = (*st.cfg).clone();
    cfg.factory_addr = "0x0000000000000000000000000000000000000000".into();
    st.cfg = Arc::new(cfg);

    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    assert_eq!(b["issuerDomainBinding"]["state"], "unavailable");
    assert_ne!(b["issuerDomainBinding"]["state"], "notADogTagIssuer");
}

/// The full three-link chain, and the categorical distinctness of the failure modes. Any two of these
/// sharing a wire value would let a strong statement be mistaken for a weak one.
#[tokio::test]
async fn the_binding_states_are_all_categorically_distinct() {
    let observed = {
        let mut out: Vec<String> = Vec::new();

        // link 1 fails
        {
            let (st, chain) = state();
            let doc = genuine_doc();
            let root = doc["signature"]["merkleRoot"].as_str().unwrap().to_string();
            issue_root(&chain, &root).await;
            chain.set_factory_clone(FACTORY, CLONE, false);
            let app = government_api::router(st);
            let (_s, b) = verify(&app, doc).await;
            out.push(
                b["issuerDomainBinding"]["state"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }
        // link 1 holds, no claim
        {
            let (st, chain) = state();
            let doc = genuine_doc();
            let root = doc["signature"]["merkleRoot"].as_str().unwrap().to_string();
            issue_root(&chain, &root).await;
            chain.set_factory_clone(FACTORY, CLONE, true);
            let app = government_api::router(st);
            let (_s, b) = verify(&app, doc).await;
            out.push(
                b["issuerDomainBinding"]["state"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }
        // links 1+2 hold, DNS unreachable (no DoH endpoint configured in this harness)
        {
            let (st, chain) = state();
            let doc = genuine_doc();
            seed(&chain, &doc).await;
            let app = government_api::router(st);
            let (_s, b) = verify(&app, doc).await;
            out.push(
                b["issuerDomainBinding"]["state"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }
        out
    };

    assert_eq!(
        observed,
        ["notADogTagIssuer", "noDomainClaimed", "couldNotCheck"]
    );
    let unique: std::collections::HashSet<&String> = observed.iter().collect();
    assert_eq!(
        unique.len(),
        3,
        "no two failure modes may share a wire value"
    );
}

// -------------------------------------------------------------------------------------------------
// (6) block anchoring — what makes a verification auditable against a mutable world
// -------------------------------------------------------------------------------------------------

/// A verdict that says "verified" without saying WHEN is not auditable. Every response carries the block
/// its on-chain reads were pinned to.
#[tokio::test]
async fn every_verification_reports_the_block_it_read_at() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    assert!(
        b["blockNumber"].as_u64().is_some(),
        "the verification is anchored to a block: {b}"
    );
    assert!(
        b["issuerDomainBinding"]["blockNumber"].as_u64().is_some(),
        "so is the binding: {}",
        b["issuerDomainBinding"]
    );
}

/// The claim's OWN block anchor is distinct from the block it was read at: the first answers "when did
/// this issuer last change its domain", the second "which chain state is this answer from".
#[tokio::test]
async fn the_binding_carries_the_block_the_claim_was_written_at() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    let binding = &b["issuerDomainBinding"];
    assert!(
        binding["claimUpdatedAtBlock"].as_u64().is_some(),
        "the on-chain claim's own anchor: {binding}"
    );
    assert!(binding["claimUpdatedAt"].as_u64().is_some());
    assert!(binding["claimSetBy"].as_str().is_some());
}

/// THE ASYMMETRY. Chain state is reproducible at any block with an archive node; DNS has no history and
/// is only ever observable NOW. So the DNS half must be labelled as a live observation, never presented
/// as if it proved the past.
#[tokio::test]
async fn the_dns_half_is_labelled_as_a_live_observation_that_cannot_be_recomputed() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    let binding = &b["issuerDomainBinding"];
    assert_eq!(binding["dnsObservation"], "live");
    assert_eq!(
        binding["dnsHistorical"], false,
        "no DNS answer may ever claim to be historical: {binding}"
    );
    assert!(
        binding["checkedAt"].as_u64().is_some(),
        "the observation has its own wall-clock time, separate from the block anchor: {binding}"
    );
}

// -------------------------------------------------------------------------------------------------
// (7) old credentials resolve against the clone that ISSUED them
// -------------------------------------------------------------------------------------------------

/// `rootIssuer[R]` is write-once and names the clone that issued THIS root. Verification follows it
/// rather than the document's `issuer.documentStore`, so a superseded clone still verifies its own
/// credentials and a swapped documentStore cannot redirect the check.
#[tokio::test]
async fn verification_follows_the_root_issuer_not_the_document_claim() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let root = doc["signature"]["merkleRoot"].as_str().unwrap();
    chain.set_root_issuer(FACTORY, root, CLONE);
    let app = government_api::router(st);

    // The attacker points documentStore at a contract they control.
    let mut forged = doc.clone();
    forged["issuer"]["documentStore"] = json!("0x00000000000000000000000000000000000000ee");

    let (_s, b) = verify(&app, forged).await;
    assert_eq!(
        b["issuerResolution"]["source"], "rootIssuer",
        "the chain resolves the issuer, not the document: {b}"
    );
    assert_eq!(b["issuerAddr"].as_str().unwrap().to_lowercase(), CLONE);
    assert_eq!(
        b["issuerResolution"]["documentStoreDiffers"], true,
        "the swap is reported rather than silently followed"
    );
}

/// With no `rootIssuer` record the document's claim is the only thing available — and the response says
/// so, so a caller can tell an authoritative resolution from a fallback.
#[tokio::test]
async fn an_unresolvable_root_falls_back_to_the_document_and_says_so() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await; // deliberately NO set_root_issuer
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    assert_eq!(b["issuerResolution"]["source"], "documentClaim");
    assert_eq!(b["issuerResolution"]["documentStoreDiffers"], false);
}
