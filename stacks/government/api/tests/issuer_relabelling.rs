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
    chain.set_issuance_capability("0x5d86e4cf98a34ae0576f190f8d209c2943a9c79c", CLONE, &signer, true);
    // Declare the clone's own immutable `recordType()`, as the factory's `createIssuer` does on a real
    // clone. The mandatory issuer-whitelist pillar asks the RESOLVED clone which record type it issues
    // rather than trusting the envelope, so an undeclared clone leaves that pillar indeterminate — and
    // an indeterminate pillar is never a pass.
    chain.set_record_type(CLONE, &government_api::app::record_type_key("TRAVEL_CLEARANCE"));
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
    // A real `issue()` calls `registerRoot`, so the factory HAS a record of anything genuinely issued.
    // Modelling that explicitly keeps "issued, and the factory knows it" distinct from "the factory has
    // no record" — the latter is a state a fake must still be able to express (see
    // `an_unresolvable_root_falls_back_to_the_document_and_says_so`, which deliberately omits this).
    chain.set_root_issuer(FACTORY, root, CLONE);
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
    assert_eq!(b["issuerResolution"]["rootIssuerRead"], "resolved");
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
    // Deliberately NOT `seed()`: this test needs the factory to have NO record of the root, so it does
    // the issuance without the `registerRoot` mirror that `seed()` performs.
    issue_root(&chain, doc["signature"]["merkleRoot"].as_str().unwrap()).await;
    chain.set_factory_clone(FACTORY, CLONE, true);
    chain.set_onchain_name(CLONE, ONCHAIN_NAME);
    chain.set_claimed_domain(DOMAIN_REGISTRY, CLONE, "gov.example");
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    assert_eq!(b["issuerResolution"]["source"], "documentClaim");
    assert_eq!(b["issuerResolution"]["documentStoreDiffers"], false);
    assert_eq!(
        b["issuerResolution"]["rootIssuerRead"], "noRecord",
        "the factory answered, and its answer was 'no record of this root': {b}"
    );
}

// -------------------------------------------------------------------------------------------------
// (8) NO identity is read from a contract we have not proven the factory deployed
// -------------------------------------------------------------------------------------------------

/// The sharpest form of the relabelling attack, and the one a DNS binding alone cannot see.
///
/// Being on-chain is not the property that matters — being FACTORY-DESCENDED is. The attacker deploys
/// their own contract, makes its `name()` return "Ministry of Health of Singapore", and points the
/// document's `documentStore` at it. `data` is untouched so integrity passes; `data.issuer` and
/// `issuer.domain` are both left genuine so the DID assertion says `match`; the root really is issued on
/// that contract so `isValid` passes. Every pillar is green. If the server reads `name()` off it anyway,
/// the fabricated authority is rendered as the ON-CHAIN issuer, which is strictly worse than showing
/// nothing — the surface is asserting provenance it never established.
///
/// So the name read is gated on link 1. Nothing else in this response may carry that string as an
/// identity either.
#[tokio::test]
async fn an_identity_is_never_read_from_a_contract_the_factory_did_not_deploy() {
    const ATTACKER: &str = "0x00000000000000000000000000000000000000ee";
    const FABRICATED: &str = "Ministry of Health of Singapore";

    let (st, chain) = state();
    let doc = genuine_doc();
    let root = doc["signature"]["merkleRoot"].as_str().unwrap().to_string();

    // The attacker's contract answers every read a naive verifier would make.
    chain.issue(ATTACKER, &root).await.expect("emulated issue");
    chain.set_onchain_name(ATTACKER, FABRICATED);
    chain.set_claimed_domain(DOMAIN_REGISTRY, ATTACKER, "gov.example");
    // ...but the factory says it did not deploy it. Note there is deliberately NO `set_root_issuer`:
    // the factory has no record of this root, so `issuer_addr` falls back to the document's claim —
    // which is exactly how an attacker-controlled address gets read in the first place.
    chain.set_factory_clone(FACTORY, ATTACKER, false);

    let mut forged = doc.clone();
    forged["issuer"]["documentStore"] = json!(ATTACKER);
    forged["issuer"]["name"] = json!(FABRICATED);

    let app = government_api::router(st);
    let (s, b) = verify(&app, forged).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // The trap: everything the attacker controls checks out.
    assert_eq!(
        b["fragments"]["integrity"], true,
        "the issuer block is outside the Merkle root, so integrity is untouched: {b}"
    );
    assert_eq!(b["fragments"]["onchain"], true, "{b}");
    assert_eq!(b["fragments"]["issuerDidAssertion"], "match", "{b}");

    // The defence: no authoritative name is offered at all, and the response says WHY.
    assert_eq!(
        b["issuerIdentity"]["onchainNameAvailable"], false,
        "a name read off an unproven contract must never be reported as available: {b}"
    );
    assert!(
        b["issuerIdentity"]["onchainName"].is_null(),
        "no name may be carried at all: {}",
        b["issuerIdentity"]
    );
    assert_eq!(b["issuerIdentity"]["provenance"], "notFactoryDeployed");
    assert_eq!(b["issuerDomainBinding"]["state"], "notADogTagIssuer");

    // And the fabricated string appears NOWHERE except as the document's own (untrusted) claim, which
    // is the one place it is honest to show it.
    assert_eq!(b["issuerIdentity"]["documentName"], FABRICATED);
    let mut identity = b["issuerIdentity"].clone();
    identity["documentName"] = json!("");
    assert!(
        !serde_json::to_string(&identity)
            .unwrap()
            .contains(FABRICATED),
        "the fabricated name leaked into an identity field: {identity}"
    );
    assert!(
        !serde_json::to_string(&b["issuerDomainBinding"])
            .unwrap()
            .contains(FABRICATED),
        "the binding must not echo an unproven contract's self-description: {}",
        b["issuerDomainBinding"]
    );
}

/// The other arm of the same gate: provenance that could not be READ is not provenance. It must yield
/// no on-chain name either — a failed read is not evidence, in whichever direction it would flatter.
#[tokio::test]
async fn an_unread_provenance_also_withholds_the_on_chain_name() {
    let (mut st, chain) = state();
    let mut cfg = (*st.cfg).clone();
    cfg.factory_addr = "0x0000000000000000000000000000000000000000".into();
    st.cfg = Arc::new(cfg);

    let doc = genuine_doc();
    seed(&chain, &doc).await; // the clone's `name()` IS seeded — it just cannot be authorised
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    assert_eq!(b["issuerIdentity"]["provenance"], "unknown");
    assert_eq!(
        b["issuerIdentity"]["onchainNameAvailable"], false,
        "without link 1 there is no authoritative name to offer: {b}"
    );
    assert!(b["issuerIdentity"]["onchainName"].is_null());
    // A withheld name is not evidence the document's name is wrong.
    assert_eq!(b["issuerIdentity"]["documentNameDiffers"], false);
    assert_eq!(b["issuerIdentity"]["relabelled"], false);
    // And the resolution reports that the factory was never asked, rather than implying it answered.
    assert_eq!(
        b["issuerResolution"]["rootIssuerRead"], "noFactoryConfigured",
        "{b}"
    );
}

// -------------------------------------------------------------------------------------------------
// (9) the block anchor is true — EVERY on-chain read in a verification is pinned to the same block
// -------------------------------------------------------------------------------------------------

/// Records the `at_block` every pinned read was asked for, delegating the answer to `MemChain`.
///
/// This proves the HANDLER threads one anchor through every read; it cannot prove `AlloyChain` attaches
/// `.block(...)` to the eth_call, because `MemChain` ignores the parameter. That is the ceiling of a
/// hermetic test here, and it is the half that regresses: a new read added without the parameter, or a
/// caller passing `None`, is exactly what makes the response's stated anchor a false claim.
/// (read name, the `at_block` it was asked for), in call order.
type PinnedReads = Vec<(&'static str, Option<u64>)>;

#[derive(Clone)]
struct PinRecordingChain {
    inner: MemChain,
    asked: Arc<std::sync::Mutex<PinnedReads>>,
}

impl PinRecordingChain {
    fn new(inner: MemChain) -> Self {
        PinRecordingChain {
            inner,
            asked: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
    fn record(&self, what: &'static str, at_block: Option<u64>) {
        self.asked.lock().unwrap().push((what, at_block));
    }
    fn reads(&self) -> PinnedReads {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl ChainClient for PinRecordingChain {
    fn chain_id(&self) -> u64 {
        self.inner.chain_id()
    }
    fn backend(&self) -> government_api::chain::ChainBackend {
        self.inner.backend()
    }
    fn can_sign(&self) -> bool {
        self.inner.can_sign()
    }
    fn signer_address(&self) -> Option<String> {
        self.inner.signer_address()
    }
    async fn is_valid(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<bool, government_api::chain::ChainError> {
        self.record("isValid", at_block);
        self.inner.is_valid(issuer_addr, root, at_block).await
    }
    async fn issued_at(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<alloy::primitives::U256, government_api::chain::ChainError> {
        self.record("issuedAt", at_block);
        self.inner.issued_at(issuer_addr, root, at_block).await
    }
    async fn block_number(&self) -> Result<u64, government_api::chain::ChainError> {
        self.inner.block_number().await
    }
    async fn root_issuer(
        &self,
        factory_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, government_api::chain::ChainError> {
        self.record("rootIssuer", at_block);
        self.inner.root_issuer(factory_addr, root, at_block).await
    }
    async fn is_factory_clone(
        &self,
        factory_addr: &str,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<bool, government_api::chain::ChainError> {
        self.record("isClone", at_block);
        self.inner
            .is_factory_clone(factory_addr, clone_addr, at_block)
            .await
    }
    async fn issuer_onchain_name(
        &self,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<String, government_api::chain::ChainError> {
        self.record("name", at_block);
        self.inner.issuer_onchain_name(clone_addr, at_block).await
    }
    async fn issuer_claimed_domain(
        &self,
        domain_registry_addr: &str,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<Option<government_api::chain::DomainClaim>, government_api::chain::ChainError> {
        self.record("domainOf", at_block);
        self.inner
            .issuer_claimed_domain(domain_registry_addr, clone_addr, at_block)
            .await
    }
    async fn issued_by(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, government_api::chain::ChainError> {
        // Recorded like every other verdict-deciding read: the issuer-whitelist pillar resolves its own
        // signer through this call, so if it were left unpinned the pillar could answer at a different
        // height than the block printed beside the verdict.
        self.record("issuedBy", at_block);
        self.inner.issued_by(issuer_addr, root, at_block).await
    }
    async fn issuer_record_type(
        &self,
        issuer_addr: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, government_api::chain::ChainError> {
        self.record("recordType", at_block);
        self.inner.issuer_record_type(issuer_addr, at_block).await
    }
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
        at_block: Option<u64>,
    ) -> Result<bool, government_api::chain::ChainError> {
        self.record("isWhitelistedFor", at_block);
        self.inner
            .is_whitelisted_for(registry_addr, record_type, signer, at_block)
            .await
    }
    async fn whitelisted_at_issuance(
        &self,
        issuer_addr: &str,
        signer: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<government_api::chain::GrantAtIssuance, government_api::chain::ChainError> {
        // Recorded like every other verdict-deciding read: this one resolves the pillar, so if it
        // ever stopped carrying the anchor the block printed beside the verdict would be a claim
        // about reads that did not all happen at that height.
        self.record("whitelistedAtIssuance", at_block);
        self.inner
            .whitelisted_at_issuance(issuer_addr, signer, root, at_block)
            .await
    }
    async fn issue(
        &self,
        issuer_addr: &str,
        root: &str,
    ) -> Result<government_api::chain::SentTx, government_api::chain::ChainError> {
        self.inner.issue(issuer_addr, root).await
    }
    async fn revoke(
        &self,
        issuer_addr: &str,
        root: &str,
    ) -> Result<government_api::chain::SentTx, government_api::chain::ChainError> {
        self.inner.revoke(issuer_addr, root).await
    }
    async fn record_verification_zk_consent(
        &self,
        registry_addr: &str,
        a: &[String; 2],
        b: &[[String; 2]; 2],
        c: &[String; 2],
        pub_signals: &[String; 7],
    ) -> Result<government_api::chain::SentTx, government_api::chain::ChainError> {
        self.inner
            .record_verification_zk_consent(registry_addr, a, b, c, pub_signals)
            .await
    }
}

/// A verdict reported under `blockNumber: N` must be reproducible at N. Anchoring the identity reads
/// while the two reads that DECIDE the answer run at `latest` is anchoring in name only: a revoke
/// landing between the head read and `isValid` yields a verdict that re-running at N contradicts, while
/// the response prints N beside it.
#[tokio::test]
async fn every_on_chain_read_in_a_verification_is_pinned_to_the_reported_block() {
    let (mut st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let root = doc["signature"]["merkleRoot"].as_str().unwrap();
    chain.set_root_issuer(FACTORY, root, CLONE);

    let recorder = PinRecordingChain::new(chain.clone());
    st.chain = Arc::new(recorder.clone());
    let app = government_api::router(st);

    // A signer is supplied so the whitelist pillar — the other verdict-deciding read — actually runs.
    let signer = chain.signer_address().unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/v1/verify")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "wrapped_doc": doc, "signer_addr": signer })).unwrap(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let b: Value = serde_json::from_slice(&bytes).unwrap();

    let anchor = b["blockNumber"]
        .as_u64()
        .unwrap_or_else(|| panic!("the response must carry the anchor it claims: {b}"));

    let reads = recorder.reads();
    assert!(
        !reads.is_empty(),
        "the verification must actually read the chain"
    );
    for (what, at) in &reads {
        assert_eq!(
            *at,
            Some(anchor),
            "`{what}` was not pinned to the reported anchor {anchor}; reads were {reads:?}"
        );
    }
    // The two that decide the verdict are the ones this test exists for. The pillar's read is the
    // HISTORICAL one - the current-state `isWhitelistedFor` no longer decides anything here, so
    // naming it would pin an anchor on a read the verdict does not rest on.
    let names: Vec<&str> = reads.iter().map(|(w, _)| *w).collect();
    assert!(names.contains(&"isValid"), "reads were {reads:?}");
    assert!(
        names.contains(&"whitelistedAtIssuance"),
        "reads were {reads:?}"
    );
}

// -------------------------------------------------------------------------------------------------
// (10) a PROVEN non-clone reaches the verdict
// -------------------------------------------------------------------------------------------------

/// Link 1 is a verdict pillar, not decoration.
///
/// `onchain_valid` is read from `issuer_addr`, which falls back to the document's own `documentStore`
/// whenever the factory has no record of the root — so the attacker's contract answers `isValid` however
/// it likes. Reporting `provenance: "notFactoryDeployed"` beside `verdict: true` is worse than not
/// checking: it is checked, failed, and passed anyway, and the portal renders its VALID badge with the
/// red provenance line beneath it.
#[tokio::test]
async fn a_proven_non_clone_fails_the_verdict() {
    const ATTACKER: &str = "0x00000000000000000000000000000000000000ee";

    let (st, chain) = state();
    let doc = genuine_doc();
    let root = doc["signature"]["merkleRoot"].as_str().unwrap().to_string();
    // Everything the attacker controls checks out: their contract says the root is valid, `data` is
    // untouched so integrity passes, and the issuer block is left genuine so the DID assertion matches.
    chain.issue(ATTACKER, &root).await.expect("emulated issue");
    chain.set_factory_clone(FACTORY, ATTACKER, false);

    let mut forged = doc.clone();
    forged["issuer"]["documentStore"] = json!(ATTACKER);

    let app = government_api::router(st);
    let (s, b) = verify(&app, forged).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["fragments"]["integrity"], true, "{b}");
    assert_eq!(b["fragments"]["onchain"], true, "{b}");
    assert_eq!(b["fragments"]["issuerDidAssertion"], "match", "{b}");
    // ...and it still does not verify, because the contract is provably not factory-descended.
    assert_eq!(
        b["verdict"], false,
        "a proven non-clone must not carry a passing verdict: {b}"
    );
    assert_eq!(
        b["fragments"]["issuerProvenance"], "notFactoryDeployed",
        "the response says WHICH pillar fell: {b}"
    );
}

/// The other arm, and the one that must NOT change: an UNKNOWN provenance is evidence of nothing.
///
/// A deployment running without `FACTORY_ADDR` would otherwise start failing every legitimate
/// credential — the same discipline as `couldNotCheck`, which is never rendered as `notListed`.
#[tokio::test]
async fn an_unknown_provenance_does_not_fail_the_verdict() {
    let (mut st, chain) = state();
    let mut cfg = (*st.cfg).clone();
    cfg.factory_addr = "0x0000000000000000000000000000000000000000".into();
    st.cfg = Arc::new(cfg);

    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    assert_eq!(b["fragments"]["issuerProvenance"], "unknown", "{b}");
    assert_eq!(
        b["verdict"], true,
        "a provenance we could not read is not a provenance failure: {b}"
    );
}

/// And the ordinary case stays green: a genuine, factory-deployed clone verifies.
#[tokio::test]
async fn a_factory_deployed_clone_still_verifies() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let (_s, b) = verify(&app, doc).await;
    assert_eq!(b["fragments"]["issuerProvenance"], "factoryDeployed", "{b}");
    assert_eq!(b["verdict"], true, "{b}");
}

// -------------------------------------------------------------------------------------------------
// (11) the DNS half's freshness is DERIVED, never asserted
// -------------------------------------------------------------------------------------------------

/// `BindingResolver` replays a cached answer keeping its ORIGINAL `checked_at`, deliberately, so a
/// surface can say how old the observation really is. That is only worth anything if the wire field is
/// derived from that timestamp: hardcoding `dnsObservation: "live"` printed "DNS checked just now" over
/// an observation up to `CacheTtl::answer_max` (15 min) old.
///
/// The threshold itself is unit-tested against a stale timestamp in `routes::tests`; what this pins is
/// the precondition — that a second verify inside the cache TTL really is a REPLAY of the first
/// observation rather than a fresh look, so the field cannot be a constant and stay true.
#[tokio::test]
async fn a_cached_dns_observation_is_replayed_with_its_original_timestamp() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let app = government_api::router(st);

    let (_s, first) = verify(&app, doc.clone()).await;
    let (_s, second) = verify(&app, doc).await;

    let seen_first = first["issuerDomainBinding"]["checkedAt"]
        .as_u64()
        .unwrap_or_else(|| panic!("the observation carries its own time: {}", first["issuerDomainBinding"]));
    let seen_second = second["issuerDomainBinding"]["checkedAt"].as_u64().unwrap();
    assert_eq!(
        seen_first, seen_second,
        "the second answer is a REPLAY of the first observation, not a fresh look"
    );
    // Both are seconds old here, so both are honestly live — the point is that the value tracks
    // `checkedAt` rather than being stamped on.
    assert_eq!(second["issuerDomainBinding"]["dnsObservation"], "live");
    assert_eq!(second["issuerDomainBinding"]["dnsHistorical"], false);
}

// -------------------------------------------------------------------------------------------------
// (12) one verification is ONE consistent snapshot — asserted against the shipped MemChain
// -------------------------------------------------------------------------------------------------

/// The same property `PinRecordingChain` above pins, but recorded by `MemChain` itself rather than by a
/// wrapper local to this file — so ANY government test can assert it, and a future suite that drops the
/// wrapper does not silently drop the guard with it.
///
/// A read taken at `latest` while the response prints `blockNumber: N` makes that anchor a false claim:
/// a revoke landing between the head read and `isValid` yields a verdict that re-running at N
/// contradicts.
#[tokio::test]
async fn memchain_records_that_every_read_used_the_reported_anchor() {
    let (st, chain) = state();
    let doc = genuine_doc();
    seed(&chain, &doc).await;
    let root = doc["signature"]["merkleRoot"].as_str().unwrap();
    chain.set_root_issuer(FACTORY, root, CLONE);
    // The seeding above issues through the write path, which reads nothing pinned; scope the assertion
    // to the verification itself.
    chain.clear_recorded_at_blocks();
    let app = government_api::router(st);

    let signer = chain.signer_address().unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/v1/verify")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "wrapped_doc": doc, "signer_addr": signer })).unwrap(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let b: Value = serde_json::from_slice(&bytes).unwrap();

    let anchor = b["blockNumber"]
        .as_u64()
        .unwrap_or_else(|| panic!("the response must carry the anchor it claims: {b}"));

    let reads = chain.recorded_at_blocks();
    assert!(
        !reads.is_empty(),
        "the verification must actually read the chain"
    );
    for (what, at) in &reads {
        assert_eq!(
            *at,
            Some(anchor),
            "`{what}` was not pinned to the reported anchor {anchor}; reads were {reads:?}"
        );
    }
    // The two that DECIDE the verdict are the ones this test exists for. The pillar's read is the
    // HISTORICAL one - the current-state `isWhitelistedFor` no longer decides anything here.
    assert!(reads.contains_key("isValid"), "reads were {reads:?}");
    assert!(
        reads.contains_key("whitelistedAtIssuance"),
        "reads were {reads:?}"
    );
}
