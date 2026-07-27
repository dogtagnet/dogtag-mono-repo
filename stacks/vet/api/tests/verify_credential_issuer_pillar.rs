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
//! MUTATION-CHECKED, and each mutation kills a DIFFERENT test — the two halves of the fix are pinned
//! independently rather than by one test that would go red for either reason:
//!   * restoring `issuer_whitelisted.unwrap_or(true)` reds `a_forged_document_store_cannot_verify`;
//!   * restoring the old `body.issuer_addr`-first resolution order reds
//!     `an_operator_supplied_issuer_addr_cannot_select_the_answering_contract`.

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
