//! `POST /verify/credential` must not believe a contract the document nominated.
//!
//! The `issuer` block sits OUTSIDE the Merkle root, so `issuer.documentStore` is attacker-controlled.
//! This endpoint used to read `isValid`/`issuedAt`/`isRevoked` against that address and to default the
//! issuer-whitelist pillar to a PASS (`issuer_whitelisted.unwrap_or(true)`) whenever no operator typed
//! a signer in. A forged document naming an obliging contract therefore returned `verdict: true` —
//! from an operator's own verification tool.
//!
//! Every read is now anchored to the factory's write-once `rootIssuer[R]`, and the pillar is mandatory:
//! only a definite `true` may contribute to a pass.
//!
//! MUTATION-CHECKED, and each mutation kills a DIFFERENT test — the independent halves of the fix
//! are pinned separately rather than by one test that would go red for any of them:
//!   * restoring `issuer_whitelisted.unwrap_or(true)` reds `a_forged_document_store_cannot_verify`;
//!   * restoring the old `body.issuer_addr`-first resolution order reds
//!     `an_operator_supplied_issuer_addr_cannot_select_the_answering_contract`;
//!   * deleting the `&& !issuer_store_differs` verdict term reds
//!     `a_rewritten_document_store_on_an_anchored_root_is_a_mismatch`;
//!   * folding the expected-signer assertion back inside the resolved branch — i.e. dropping the
//!     `(None, Some(want))` arm's registry read — reds
//!     `a_factoryless_deployment_still_fails_an_unwhitelisted_expected_signer`, and so does dropping
//!     the `is_none()` term from `issuer_pillar_ok` (which would let a factory-less `Some(false)`
//!     pass anyway);
//!   * breaking the tighten-only asymmetry — returning `Some(whitelisted)` from that arm instead of
//!     a failure-or-nothing — reds
//!     `a_factoryless_deployment_cannot_be_talked_into_a_pass_by_an_expected_signer`;
//!   * collapsing `expectedIssuerState` back into the bare `expectedIssuerDiffers` boolean reds
//!     `an_unevaluable_expected_clone_assertion_is_reported_not_swallowed`.
//!
//! The third exists because the first test cannot reach it: there the factory has NO record of the
//! root, so `resolved_issuer` is `None` and `issuer_store_differs` is `false`. The single-field
//! forgery against a GENUINELY anchored root is a different path, and it is the one where every other
//! pillar passes.
//!
//! The tighten-only test asserts on the STATE FIELDS, not on the verdict, and that is deliberate:
//! with no factory configured `issuer_pillar_ok` is satisfied by the unavailable branch either way,
//! so `verdict == true` holds under the mutation too and would pin nothing. `issuerWhitelistState`
//! and `issuerWhitelisted` are the only places the difference is observable.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::Value;
use std::sync::Arc;
use vet_api::chain::{record_type_key, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";
/// A contract the factory never deployed, answering exactly as an attacker would want. Reached only
/// by a document that names it in `issuer.documentStore`.
const HOSTILE: &str = "0x00000000000000000000000000000000000000c1";
/// An EOA the attacker controls. It is not whitelisted on the verifier's registry for anything.
const ATTACKER_SIGNER: &str = "0x00000000000000000000000000000000000000c2";

/// Boot a vet app on a MemChain that has NO automatic factory registration, so a test decides for each
/// root whether the factory has a record of it. Returns (app, operator token, backend signer, chain).
async fn boot() -> (axum::Router, String, String, MemChain) {
    let mem = MemChain::new();
    let state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let (_admin, op, backend) = boot_custody(&app).await;
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend);
    // The clone's own immutable `recordType()`, as the factory's `createIssuer` fixes it.
    mem.set_record_type(ISSUER, &rt);
    (app, op, backend, mem)
}

/// Issue a real credential through the ordinary flow and return its (record id, root, wrapped
/// document). The document is genuine in every respect — the attacks below only rewrite fields that
/// sit OUTSIDE the Merkle root, which is precisely why integrity keeps reconciling.
async fn issue_doc(app: &axum::Router, op: &str, dog_tag_id: &str) -> (String, String, Value) {
    let (s, b) = call(
        app,
        "POST",
        "/credentials/prepare",
        Some(op),
        Some(serde_json::json!({
            "recordType": "VACCINATION",
            "dogTagId": dog_tag_id,
            "fields": vaccination_fields()
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "prepare: {b}");
    let record_id = b["recordId"].as_str().unwrap().to_string();
    let root = b["merkleRoot"].as_str().unwrap().to_string();

    let (s, b) = call(
        app,
        "POST",
        &format!("/records/{record_id}/share"),
        Some(op),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "share: {b}");
    let token = b["qrUrl"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();

    let (s, doc) = call(app, "GET", &format!("/r/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "get shared: {doc}");
    (record_id, root, doc)
}

/// A deployment that never set `FACTORY_ADDR`, so the factory-anchored pillar cannot be evaluated at
/// all. Everything else is wired exactly as `boot()` wires it.
async fn boot_factoryless() -> (axum::Router, String, String, MemChain) {
    let mem = MemChain::new();
    let mut state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let mut cfg = (*state.cfg).clone();
    cfg.factory_addr = String::new();
    state.cfg = Arc::new(cfg);
    let app = vet_api::router(state);
    let (_admin, op, backend) = boot_custody(&app).await;
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend);
    mem.set_record_type(ISSUER, &rt);
    (app, op, backend, mem)
}

async fn verify(app: &axum::Router, op: &str, body: Value) -> Value {
    let (s, b) = call(app, "POST", "/verify/credential", Some(op), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "verify/credential: {b}");
    b
}

/// THE DEFECT. A document whose `issuer.documentStore` names a contract the attacker deployed, and
/// whose root the factory has no record of, must not verify — no matter how obligingly that contract
/// answers.
///
/// Pre-fix this returned `verdict: true`: `isValid` was read from HOSTILE (which says yes) and the
/// whitelist pillar was skipped entirely, then defaulted to a pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forged_document_store_cannot_verify() {
    let (app, op, _backend, mem) = boot().await;
    let (_id, root, mut doc) = issue_doc(&app, &op, "43").await;

    // The attacker's contract: valid, issued, never revoked, and claiming the right record type. The
    // factory's `rootIssuer` index deliberately does NOT point at it — nothing else could, since only
    // an `isClone` contract can write that index.
    mem.with_hostile_clone(
        HOSTILE,
        true,
        1_700_000_000,
        ATTACKER_SIGNER,
        &record_type_key("VACCINATION"),
    );
    // The `issuer` block is outside the Merkle root, so this rewrite leaves integrity intact — which
    // is the entire point of the attack.
    doc["issuer"]["documentStore"] = Value::String(HOSTILE.to_string());

    let b = verify(&app, &op, serde_json::json!({ "wrappedDoc": doc })).await;

    assert_eq!(
        b["verdict"], false,
        "a forged documentStore must not verify: {b}"
    );
    // Integrity genuinely still passes. That is what made this dangerous: the failure has to come from
    // the issuer pillar, not from a document that was mangled into failing.
    assert_eq!(b["fragments"]["integrity"], true, "{b}");
    // We ASKED and the chain has no record of this root. That is evidence, not a shrug.
    assert_eq!(b["issuerResolution"], "noRecord", "{b}");
    assert_eq!(b["fragments"]["issuerWhitelisted"], Value::Null, "{b}");
    assert_eq!(b["fragments"]["issuerWhitelistState"], "unresolved", "{b}");
    // `status` has to move with the verdict too, or "not evaluated" reads as "passed" one field over.
    assert_ne!(b["status"], "valid", "{b}");
    assert_eq!(b["status"], "issuer_unresolved", "{b}");
    // Sanity: the root the attacker wrapped is the one under test.
    assert_eq!(b["root"], root, "{b}");
}

/// An operator-supplied `issuerAddr` may only TIGHTEN. It must never select which contract answers.
///
/// Pre-fix `issuer_addr` sat at the FRONT of the resolution chain, so an operator who pasted an
/// attacker-supplied address — or an attacker with an operator session — nominated the contract that
/// vouches for the credential.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_operator_supplied_issuer_addr_cannot_select_the_answering_contract() {
    let (app, op, _backend, mem) = boot().await;
    let (record_id, root, doc) = issue_doc(&app, &op, "44").await;
    // This root IS genuinely anchored: the factory names the real clone.
    mem.set_root_issuer(FACTORY_ADDR, &root, ISSUER);
    // ...and the credential is then REVOKED, so the honest answer is "not valid".
    let (s, b) = call(
        &app,
        "POST",
        &format!("/records/{record_id}/revoke"),
        Some(&op),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "revoke: {b}");

    // The attacker's contract cheerfully reports the revoked root as valid and unrevoked.
    mem.with_hostile_clone(
        HOSTILE,
        true,
        1_700_000_000,
        ATTACKER_SIGNER,
        &record_type_key("VACCINATION"),
    );

    let b = verify(
        &app,
        &op,
        serde_json::json!({ "wrappedDoc": doc, "issuerAddr": HOSTILE }),
    )
    .await;

    // The override did not redirect a single read: the answers still come from the real clone.
    assert_eq!(
        b["issuerAddr"], ISSUER,
        "reads must go to the factory-resolved clone, never the caller's nomination: {b}"
    );
    assert_eq!(b["issuerResolution"], "resolved", "{b}");
    assert_eq!(
        b["fragments"]["revoked"], true,
        "the REAL clone's revocation must survive a hostile nomination: {b}"
    );
    assert_eq!(b["verdict"], false, "{b}");
    // And the disagreement is reported rather than silently ignored.
    assert_eq!(b["fragments"]["expectedIssuerDiffers"], true, "{b}");
}

/// The mandatory pillar must not condemn a genuine credential. This is the regression guard: the
/// pillar reads the record type from the CLONE and the signer from `issuedBy[R]`, so a drift in either
/// derivation would fail every honest document — the mirror of the defect being fixed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_genuine_credential_still_passes_the_mandatory_pillar() {
    let (app, op, backend, mem) = boot().await;
    let (_id, root, doc) = issue_doc(&app, &op, "45").await;
    mem.set_root_issuer(FACTORY_ADDR, &root, ISSUER);

    let b = verify(&app, &op, serde_json::json!({ "wrappedDoc": doc })).await;

    assert_eq!(b["verdict"], true, "genuine credential must verify: {b}");
    assert_eq!(b["status"], "valid", "{b}");
    assert_eq!(b["fragments"]["issuerWhitelisted"], true, "{b}");
    assert_eq!(b["fragments"]["issuerWhitelistState"], "passed", "{b}");
    // Resolved WITHOUT the caller naming anything: no `signerAddr`, no `issuerAddr` in the request.
    assert_eq!(b["signerAddr"], backend.to_lowercase(), "{b}");
    assert_eq!(b["issuerResolution"], "resolved", "{b}");
}

/// A supplied `signerAddr` tightens and nothing more. A correct one changes no verdict; a wrong one
/// fails the pillar even though the on-chain originator is genuinely whitelisted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expected_signer_can_only_tighten() {
    let (app, op, backend, mem) = boot().await;
    let (_id, root, doc) = issue_doc(&app, &op, "46").await;
    mem.set_root_issuer(FACTORY_ADDR, &root, ISSUER);

    // Asserting the true signer: still a pass, unchanged.
    let b = verify(
        &app,
        &op,
        serde_json::json!({ "wrappedDoc": doc.clone(), "signerAddr": backend }),
    )
    .await;
    assert_eq!(b["verdict"], true, "{b}");
    assert_eq!(b["fragments"]["issuerWhitelistState"], "passed", "{b}");
    assert_eq!(b["fragments"]["expectedSignerState"], "matched", "{b}");

    // Asserting a different signer: the pillar FAILS, even though `issuedBy[R]` is whitelisted.
    let b = verify(
        &app,
        &op,
        serde_json::json!({ "wrappedDoc": doc.clone(), "signerAddr": ATTACKER_SIGNER }),
    )
    .await;
    assert_eq!(
        b["verdict"], false,
        "a wrong expected signer must fail: {b}"
    );
    assert_eq!(b["fragments"]["issuerWhitelistState"], "failed", "{b}");
    assert_eq!(b["fragments"]["expectedSignerState"], "differs", "{b}");
    assert_eq!(b["status"], "issuer_not_whitelisted", "{b}");
    // The reported signer is still the CHAIN's answer, never the caller's claim.
    assert_eq!(b["signerAddr"], backend.to_lowercase(), "{b}");

    // An EMPTY signer is not an assertion — it must not be read as a mismatch.
    let b = verify(
        &app,
        &op,
        serde_json::json!({ "wrappedDoc": doc, "signerAddr": "   " }),
    )
    .await;
    assert_eq!(b["verdict"], true, "{b}");
    assert_eq!(b["fragments"]["expectedSignerState"], "notAsserted", "{b}");
}

/// Reporting the pillar UNAVAILABLE must not cost a capability that worked without a factory.
///
/// "We could not resolve the clone" is not "we stopped checking anything". Before this, the caller's
/// expected-signer assertion lived inside the resolved branch, so on a factory-less deployment it was
/// silently DISCARDED — an operator who explicitly named the signer they expected got `verdict: true`
/// with no check performed at all, which is strictly weaker than the endpoint was before the pillar
/// existed. It is exactly the defect class this whole change is about, turned on the caller's own
/// claim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_factoryless_deployment_still_fails_an_unwhitelisted_expected_signer() {
    let (app, op, _backend, _mem) = boot_factoryless().await;
    let (_id, _root, doc) = issue_doc(&app, &op, "51").await;

    let b = verify(
        &app,
        &op,
        serde_json::json!({ "wrappedDoc": doc, "signerAddr": ATTACKER_SIGNER }),
    )
    .await;

    assert_eq!(b["issuerResolution"], "noFactoryConfigured", "{b}");
    // The registry really was consulted, and it said no.
    assert_eq!(
        b["fragments"]["expectedSignerState"], "unanchoredNotWhitelisted",
        "the assertion must still reach this deployment's own registry: {b}"
    );
    assert_eq!(b["fragments"]["issuerWhitelisted"], false, "{b}");
    // A DEFINITE failure outranks "we never asked" — reporting it as unavailable would hide a real
    // failure behind our own configuration gap.
    assert_eq!(b["fragments"]["issuerWhitelistState"], "failed", "{b}");
    assert_eq!(
        b["verdict"], false,
        "a definite whitelist failure must fail the credential even with no factory: {b}"
    );
    assert_eq!(b["status"], "issuer_not_whitelisted", "{b}");
}

/// The other half of that ruling, and the one that must not be reintroduced as a bypass: the
/// expected-signer path is TIGHTEN-ONLY. It may contribute a definite failure and may NEVER
/// contribute a pass.
///
/// With no factory we still could not resolve the clone, so a signer that happens to be whitelisted
/// shows nothing about whether it issued THIS root. The pillar therefore stays unavailable.
///
/// Asserted on the STATE FIELDS, not the verdict: `issuer_pillar_ok` is satisfied by the unavailable
/// branch regardless, so `verdict == true` holds under the broken version too and would pin nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_factoryless_deployment_cannot_be_talked_into_a_pass_by_an_expected_signer() {
    let (app, op, backend, _mem) = boot_factoryless().await;
    let (_id, _root, doc) = issue_doc(&app, &op, "52").await;

    let b = verify(
        &app,
        &op,
        serde_json::json!({ "wrappedDoc": doc, "signerAddr": backend }),
    )
    .await;

    // The asserted signer IS whitelisted for this record type...
    assert_eq!(
        b["fragments"]["expectedSignerState"], "unanchoredUnconfirmed",
        "{b}"
    );
    // ...and that changes NOTHING about the pillar. Still unevaluated, still not a pass.
    assert_eq!(
        b["fragments"]["issuerWhitelisted"],
        Value::Null,
        "a whitelisted expected signer must never promote an unresolved pillar: {b}"
    );
    assert_eq!(
        b["fragments"]["issuerWhitelistState"], "unavailableNoFactoryConfigured",
        "{b}"
    );
    assert_eq!(b["issuerResolution"], "noFactoryConfigured", "{b}");
    // Unchanged from the no-assertion case: our own misconfiguration still does not condemn it.
    assert_eq!(b["verdict"], true, "{b}");
}

/// A caller assertion that could not be evaluated must be VISIBLE, never silently discarded. The
/// boolean `expectedIssuerDiffers` spells "checked and held" and "never checked" the same way, which
/// is precisely how a dropped check comes to read as a satisfied one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unevaluable_expected_clone_assertion_is_reported_not_swallowed() {
    let (app, op, _backend, _mem) = boot_factoryless().await;
    let (_id, _root, doc) = issue_doc(&app, &op, "53").await;

    let b = verify(
        &app,
        &op,
        serde_json::json!({ "wrappedDoc": doc, "issuerAddr": HOSTILE }),
    )
    .await;

    assert_eq!(
        b["fragments"]["expectedIssuerState"], "notEvaluated",
        "an unevaluable clone assertion must say so: {b}"
    );
    // The bare boolean cannot tell this apart from a satisfied assertion — which is the whole reason
    // the state field exists beside it.
    assert_eq!(b["fragments"]["expectedIssuerDiffers"], false, "{b}");
    assert_eq!(b["expectedIssuerAddr"], HOSTILE, "{b}");
    // And it still did not select which contract answers.
    assert_ne!(b["issuerAddr"], HOSTILE, "{b}");
}

/// The stated repro in its most direct form: the root IS genuinely anchored to the real clone, and
/// the attacker rewrites ONLY `issuer.documentStore` — one field, outside the Merkle root.
///
/// Distinct from `a_forged_document_store_cannot_verify`, which exercises the `noRecord` path (there
/// `resolved_issuer` is `None`, so `issuer_store_differs` is false and the pillar fails instead). This
/// is the only case that pins the `!issuer_store_differs` verdict term: without it the credential
/// would pass, because the factory-resolved clone answers honestly and the pillar is satisfied — the
/// document simply lies about who issued it, and nothing else would notice.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rewritten_document_store_on_an_anchored_root_is_a_mismatch() {
    let (app, op, _backend, mem) = boot().await;
    let (_id, root, mut doc) = issue_doc(&app, &op, "49").await;
    // Genuinely anchored: the factory names the real clone, which really did issue this root, and its
    // originator really is whitelisted. Every pillar below is satisfied.
    mem.set_root_issuer(FACTORY_ADDR, &root, ISSUER);
    mem.with_hostile_clone(
        HOSTILE,
        true,
        1_700_000_000,
        ATTACKER_SIGNER,
        &record_type_key("VACCINATION"),
    );
    // The single-field forgery.
    doc["issuer"]["documentStore"] = Value::String(HOSTILE.to_string());

    let b = verify(&app, &op, serde_json::json!({ "wrappedDoc": doc })).await;

    // The pillar itself PASSES — the honest clone vouches for the root. The refusal has to come from
    // the envelope disagreeing with the chain about who issued it.
    assert_eq!(b["fragments"]["issuerWhitelistState"], "passed", "{b}");
    assert_eq!(b["fragments"]["onchain"], true, "{b}");
    assert_eq!(b["fragments"]["integrity"], true, "{b}");
    // ...and it does.
    assert_eq!(b["fragments"]["documentStoreDiffers"], true, "{b}");
    assert_eq!(
        b["verdict"], false,
        "a document that misnames its issuer must not verify: {b}"
    );
    assert_eq!(b["status"], "issuer_mismatch", "{b}");
    // Reads went to the real clone regardless of what the envelope named.
    assert_eq!(b["issuerAddr"], ISSUER, "{b}");
    assert_eq!(b["documentStore"], HOSTILE, "{b}");
}

/// A signer the registry does not whitelist is a DEFINITE failure, distinct from an unresolved one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unwhitelisted_on_chain_signer_fails_the_pillar() {
    let (app, op, _backend, mem) = boot().await;
    let (_id, root, doc) = issue_doc(&app, &op, "47").await;
    // The factory names a clone that DID issue this root — but the originator it recorded is an
    // address the verifier's own registry knows nothing about.
    mem.set_root_issuer(FACTORY_ADDR, &root, HOSTILE);
    mem.with_hostile_clone(
        HOSTILE,
        true,
        1_700_000_000,
        ATTACKER_SIGNER,
        &record_type_key("VACCINATION"),
    );

    let b = verify(&app, &op, serde_json::json!({ "wrappedDoc": doc })).await;

    assert_eq!(b["verdict"], false, "{b}");
    assert_eq!(b["fragments"]["issuerWhitelisted"], false, "{b}");
    assert_eq!(b["fragments"]["issuerWhitelistState"], "failed", "{b}");
    // The envelope also names a different contract than the chain does, and that is reported.
    assert_eq!(b["fragments"]["documentStoreDiffers"], true, "{b}");
}

/// "We never asked" must be visible, and must not be spelled the same way as "we asked and it passed".
///
/// This is the deployment-misconfiguration state: no factory configured, so the pillar CANNOT be
/// evaluated. That is our own gap and is not evidence about the credential, so it does not fail it —
/// but it is reported explicitly, because a silently-absent pillar would make misconfigure-to-bypass a
/// real attack path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unconfigured_factory_reports_unavailable_and_never_passed() {
    let mem = MemChain::new();
    let mut state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    // A deployment that never set `FACTORY_ADDR`.
    let mut cfg = (*state.cfg).clone();
    cfg.factory_addr = String::new();
    state.cfg = Arc::new(cfg);
    let app = vet_api::router(state);
    let (_admin, op, backend) = boot_custody(&app).await;
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend);
    mem.set_record_type(ISSUER, &rt);

    let (_id, _root, doc) = issue_doc(&app, &op, "48").await;
    let b = verify(&app, &op, serde_json::json!({ "wrappedDoc": doc })).await;

    assert_eq!(b["issuerResolution"], "noFactoryConfigured", "{b}");
    assert_eq!(
        b["fragments"]["issuerWhitelistState"], "unavailableNoFactoryConfigured",
        "an unevaluated pillar must never render as passed: {b}"
    );
    // Unevaluated, so it is null — NOT `true`.
    assert_eq!(b["fragments"]["issuerWhitelisted"], Value::Null, "{b}");
    // And our own misconfiguration does not condemn the credential.
    assert_eq!(b["verdict"], true, "{b}");
}

/// A MALFORMED `FACTORY_ADDR` is a configuration FAULT, not a deployment that chose to have no
/// factory. It must not degrade into the fail-open `unavailableNoFactoryConfigured` state: that would
/// turn one fat-fingered character into a silently disabled security pillar — the
/// misconfigure-to-bypass path these explicit states exist to prevent. A deployment that set the
/// value INTENDED to check, so failing to check is a fault worth refusing to answer over.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_factory_addr_fails_loudly_instead_of_open() {
    let mem = MemChain::new();
    let mut state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let mut cfg = (*state.cfg).clone();
    // Truncated by one character — exactly the mistake an operator actually makes.
    cfg.factory_addr = "0x00000000000000000000000000000000000000f".to_string();
    state.cfg = Arc::new(cfg);
    let app = vet_api::router(state);
    let (_admin, op, backend) = boot_custody(&app).await;
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend);
    mem.set_record_type(ISSUER, &rt);

    let (_id, _root, doc) = issue_doc(&app, &op, "50").await;
    let (s, b) = call(
        &app,
        "POST",
        "/verify/credential",
        Some(&op),
        Some(serde_json::json!({ "wrappedDoc": doc })),
    )
    .await;

    assert_eq!(
        s,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a malformed factory must not yield a verdict at all: {b}"
    );
    // Emphatically NOT a pass dressed up as an unavailable pillar.
    assert_eq!(b["verdict"], Value::Null, "{b}");
}
