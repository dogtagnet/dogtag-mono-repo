//! The record-type-keyed issuance-axis reads, migrated off `ISSUER_REGISTRY_ADDR` and off the
//! current-state getter.
//!
//! Three distinct claims are pinned here, and they fail for three different reasons.
//!
//! (1) THE AUTHORITY COMES OFF THE CLONE. `issuance_capability` reads the resolved clone's own
//!     `registry()` instead of this deployment's configured `ISSUER_REGISTRY_ADDR`. That is what
//!     removes the record-type key shape from that variable — one value was read by both the VERIFY
//!     key and the record-type key in the same process, so there was nothing to split at the config
//!     layer while a record-type caller still read it (`docs/CLIENT_REPOINT.md`).
//!
//! (2) THE RUNG IS `canIssue`, NOT `isRecognizedIssuer`. The ladder is
//!     `isRecognizedIssuer` ⊇ `canRevoke` ⊇ `canIssue`, and `DogTagIssuerV2.issue` is gated by
//!     `onlyIssuanceCapable` == `canIssue`. A preflight on a wider rung passes where the write
//!     reverts, which is the one thing a preflight exists to prevent.
//!
//! (3) CONFIRM ASKS THE PAST, NOT THE PRESENT. Delisting is forward-only, so a signer rotated
//!     between broadcast and confirm must still be able to confirm the issuance it already mined.
//!     This is #127's defect class reached from the issuance side rather than the verification side.
//!
//! MUTATION-CHECKED, each mutation reddening its own named test:
//!   * point `issuance_capability`'s call site back at
//!     `is_whitelisted_for(&st.cfg.issuer_registry_addr, ..)` in `prepare` ->
//!     `the_preflight_asks_the_clones_own_authority_not_the_configured_registry` and
//!     `a_generation_two_clone_issues_through_the_preflight` go red;
//!   * swap the MemChain/Alloy generation-2 arm from `can_issue` to `recognized` ->
//!     `the_preflight_uses_the_narrow_rung_so_it_refuses_what_the_write_would_refuse` goes red;
//!   * collapse `IssuanceCapability::Undetermined` into the FORBIDDEN arm ->
//!     `an_undeterminable_authority_is_not_reported_as_the_signers_fault` goes red;
//!   * restore the matrix's `.unwrap_or(false)` ->
//!     `the_signer_matrix_says_null_rather_than_false_when_it_could_not_check` goes red;
//!   * point confirm back at `is_whitelisted_for` ->
//!     `a_signer_delisted_after_it_anchored_can_still_confirm_its_own_issuance` goes red.
//!
//! The delisted-BEFORE half is asserted as a REQUIREMENT beside the delisted-AFTER half, never as an
//! observation, so a future change that softened the refusal into "could not determine" would be
//! caught rather than silently accepted.

mod common;

use axum::http::StatusCode;
use common::*;
use dogtag_standard::verify::{GrantEvent, LogPoint};
use serde_json::Value;
use std::sync::Arc;
use vet_api::chain::{record_type_key, ChainClient, IssuanceCapability, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";
/// A second authority address, used as the clone's `registry()` while the deployment's configured
/// `ISSUER_REGISTRY_ADDR` stays `REGISTRY`. On a real chain these are the same for a matched pair;
/// separating them here is what makes "which one did we ask" observable at all.
const CLONE_OWN_REGISTRY: &str = "0x00000000000000000000000000000000000000a2";

/// Boot a vet backend in the default BACKEND signing mode. Returns (app, operator token, backend
/// signer address, chain).
async fn boot() -> (axum::Router, String, String, MemChain) {
    // `with_registry` and no grant: a real clone answers `registry()` whether or not anyone was ever
    // whitelisted, so without it every authority read reports "could not determine" where the chain
    // would report an authority whose mapping is simply empty.
    let mem = MemChain::new()
        .with_factory(FACTORY_ADDR)
        .with_registry(REGISTRY);
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
    mem.set_record_type(ISSUER, &record_type_key("VACCINATION"));
    (app, op, backend, mem)
}

async fn prepare(app: &axum::Router, op: &str, dog_tag_id: &str) -> (StatusCode, Value) {
    call(
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
    .await
}

// -------------------------------------------------------------------------------------------
// (1) the authority is the clone's, not the configuration's
// -------------------------------------------------------------------------------------------

/// The grant lives ONLY in the registry the clone names. The deployment's configured
/// `ISSUER_REGISTRY_ADDR` has no grant for this pair at all, so a preflight that still read the
/// config value would refuse a signer the chain authorises.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_preflight_asks_the_clones_own_authority_not_the_configured_registry() {
    let (app, op, backend, mem) = boot().await;
    let rt = record_type_key("VACCINATION");

    mem.whitelist(CLONE_OWN_REGISTRY, &rt, &backend);
    mem.set_governing_registry(ISSUER, CLONE_OWN_REGISTRY);

    // Precondition, asserted rather than assumed: the CONFIGURED registry really does answer `false`.
    // Without this the test would pass for a deployment where both registries happen to agree, which
    // is exactly the state that cannot distinguish the two reads.
    assert!(
        !mem.is_whitelisted_for(REGISTRY, &rt, &backend).await.unwrap(),
        "fixture is inert: the configured registry must NOT hold this grant",
    );

    let (s, b) = prepare(&app, &op, "1").await;
    assert_eq!(s, StatusCode::OK, "prepare must succeed: {b}");
}

/// A clone whose `registry()` is a `ProviderRegistry`. The legacy record-type key means nothing
/// there, so only a service-scoped read can authorise this issuance.
///
/// Asserted at the chain client rather than through `prepare`, because the preflight is not the only
/// authority read on the backend-mode issuance path — see the test below, which pins what the rest of
/// that path does. Routing this through `prepare` would conflate the two.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_generation_two_clone_passes_the_issuance_preflight() {
    let (_app, _op, backend, mem) = boot().await;

    mem.set_governing_registry(ISSUER, CLONE_OWN_REGISTRY);
    mem.set_provider_capability(CLONE_OWN_REGISTRY, ISSUER, &backend, true, true);

    assert_eq!(
        mem.issuance_capability(ISSUER, &record_type_key("VACCINATION"), &backend)
            .await
            .unwrap(),
        IssuanceCapability::Authorized,
        "a service-scoped grant must authorise a generation-2 issuance",
    );
}

/// FINDING, pinned rather than described: backend-mode issuance against a generation-2 clone gets
/// through the preflight, anchors, and then CANNOT BE CONFIRMED — because confirm asks the historical
/// question and this build cannot yet answer it for generation 2.
///
/// `whitelisted_at_issuance` reconstructs the grant from `Whitelisted`/`Delisted`, whose `topic1` is
/// the RECORD-TYPE key. `ProviderRegistry` records its grants as `IssuanceCapabilitySet(service,
/// signer, allowed)` — different name, different `topic0`, different argument shape — so that filter
/// matches nothing and the honest answer is "could not determine".
///
/// The failure is LOUD and the record stays `Prepared`, which is the correct degradation: the
/// alternative before this change was a definite 403 blaming the signer (the configured generation-1
/// registry answers the record-type key off `_verifierCapabilities`, a confident `false`). Both are
/// broken; only one of them is honest about which.
///
/// Closing it is the event-vocabulary migration recorded in `docs/ISSUER_V2_OWNERSHIP.md` §8, not a
/// getter swap: `isRecognizedIssuer` reads current storage and cannot answer an at-anchoring
/// question. This test is what makes that blocker fail visibly at the cutover instead of being
/// rediscovered from a stranded record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_generation_two_issuance_cannot_yet_be_confirmed_and_says_so() {
    let (app, op, backend, mem) = boot().await;

    mem.set_governing_registry(ISSUER, CLONE_OWN_REGISTRY);
    mem.set_provider_capability(CLONE_OWN_REGISTRY, ISSUER, &backend, true, true);

    let (s, b) = prepare(&app, &op, "2").await;
    assert_eq!(
        s,
        StatusCode::BAD_GATEWAY,
        "must fail loudly at the confirm-stage authority read: {b}"
    );
    assert_ne!(
        s,
        StatusCode::FORBIDDEN,
        "and must never blame the signer for a vocabulary this build cannot read"
    );
    assert!(
        b["error"]
            .as_str()
            .is_some_and(|e| e.contains("could not determine")),
        "the reason must name the inability: {b}"
    );
}

// -------------------------------------------------------------------------------------------
// (2) the rung
// -------------------------------------------------------------------------------------------

/// A SUPERSEDED clone: still a recognized issuer — its existing roots stay genuine and stay
/// revocable — but `canIssue` is false, so it may anchor nothing new.
///
/// This is the case that separates the two rungs. Both are seeded independently, so a preflight
/// built on `isRecognizedIssuer` would let this through and the write would then revert.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_preflight_uses_the_narrow_rung_so_it_refuses_what_the_write_would_refuse() {
    let (app, op, backend, mem) = boot().await;

    mem.set_governing_registry(ISSUER, CLONE_OWN_REGISTRY);
    mem.set_provider_capability(CLONE_OWN_REGISTRY, ISSUER, &backend, true, false);

    // The wide rung really is satisfied — otherwise this test would pass under either rung and pin
    // nothing about which one the preflight consults.
    assert_eq!(
        mem.issuance_capability(ISSUER, &record_type_key("VACCINATION"), &backend)
            .await
            .unwrap(),
        IssuanceCapability::NotAuthorized,
        "the narrow rung must refuse a superseded clone",
    );

    let (s, b) = prepare(&app, &op, "3").await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a superseded clone must not anchor a new root: {b}"
    );
}

// -------------------------------------------------------------------------------------------
// (3) could not determine is neither verdict
// -------------------------------------------------------------------------------------------

/// An authority that answers in no vocabulary this build knows. Refusing the issuance is right — we
/// will not spend gas on a write we cannot show will land — but it must not be reported as the
/// SIGNER's problem, which is what the 403 arm says.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_undeterminable_authority_is_not_reported_as_the_signers_fault() {
    let (app, op, backend, mem) = boot().await;
    let rt = record_type_key("VACCINATION");

    // Grant it on the configured registry too, so an implementation that fell back to the config
    // value would produce a PASS here and be visibly distinguishable from the honest refusal.
    mem.whitelist(REGISTRY, &rt, &backend);
    mem.set_governing_registry(ISSUER, CLONE_OWN_REGISTRY);
    mem.set_registry_unanswerable(CLONE_OWN_REGISTRY);

    let (s, b) = prepare(&app, &op, "4").await;
    assert_eq!(
        s,
        StatusCode::BAD_GATEWAY,
        "an unanswerable authority is our inability, not a permissions verdict: {b}"
    );
    assert_ne!(
        s,
        StatusCode::FORBIDDEN,
        "must never accuse the signer over a read that did not resolve"
    );
}

/// The operator console's issuance matrix. `whitelisted` was a bare bool defaulting to `false` on any
/// read failure — an RPC blip rendered as "this signer is not approved".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_signer_matrix_says_null_rather_than_false_when_it_could_not_check() {
    let (app, op, backend, mem) = boot().await;
    let rt = record_type_key("VACCINATION");

    mem.whitelist(REGISTRY, &rt, &backend);
    mem.set_governing_registry(ISSUER, CLONE_OWN_REGISTRY);
    mem.set_registry_unanswerable(CLONE_OWN_REGISTRY);

    let (s, b) = call(&app, "GET", "/issuer/signers", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let row = b["matrix"]
        .as_array()
        .and_then(|m| m.first())
        .expect("one configured record type");
    assert!(
        row["whitelisted"].is_null(),
        "could-not-check must be null, never false: {row}"
    );
}

/// The same matrix, on a working authority, still answers the two definite states — so the null above
/// is a third state rather than the field having become useless.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_signer_matrix_still_reports_both_definite_answers() {
    let (app, op, backend, mem) = boot().await;
    let rt = record_type_key("VACCINATION");

    let (s, b) = call(&app, "GET", "/issuer/signers", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        b["matrix"][0]["whitelisted"],
        Value::Bool(false),
        "ungranted signer must be a definite false, not null: {b}"
    );

    mem.whitelist(REGISTRY, &rt, &backend);
    let (s, b) = call(&app, "GET", "/issuer/signers", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        b["matrix"][0]["whitelisted"],
        Value::Bool(true),
        "granted signer must be a definite true: {b}"
    );
}

// -------------------------------------------------------------------------------------------
// (4) confirm asks the past
// -------------------------------------------------------------------------------------------

/// Put the backend into wallet mode, prepare a draft, then broadcast its calldata directly so the
/// root is genuinely anchored and its `RootIssued` takes a real position in the log stream. Returns
/// (record id, merkle root, tx hash).
async fn wallet_prepare_and_anchor(
    app: &axum::Router,
    op: &str,
    mem: &MemChain,
    dog_tag_id: &str,
) -> (String, String, String) {
    let (s, _) = call(
        app,
        "PUT",
        "/settings/signing-mode",
        Some(op),
        Some(serde_json::json!({"mode": "wallet"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, b) = prepare(app, op, dog_tag_id).await;
    assert_eq!(s, StatusCode::OK, "wallet prepare: {b}");
    let record_id = b["recordId"].as_str().unwrap().to_string();
    let root = b["merkleRoot"].as_str().unwrap().to_string();
    let calldata = b["unsignedTx"]["data"].as_str().unwrap().to_string();

    let sent = mem
        .sign_and_send(0, ISSUER, &calldata)
        .await
        .expect("anchor the root");
    (record_id, root, sent.tx_hash)
}

/// THE #127 DEFECT CLASS, REACHED FROM THE ISSUANCE SIDE.
///
/// The transaction has already mined, and `DogTagIssuer.issue` is `onlyWhitelisted`, so the chain
/// itself established this signer's authority at that moment. A current-state re-check at confirm
/// therefore cannot make the check stronger and can make it wrong: an ordinary key rotation — or the
/// C-12 cutover freeze, which delists every generation-1 signer — would strand a genuine, mined
/// issuance in `Prepared` with no way to advance it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signer_delisted_after_it_anchored_can_still_confirm_its_own_issuance() {
    let (app, op, backend, mem) = boot().await;
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend);

    let (record_id, _root, tx_hash) = wallet_prepare_and_anchor(&app, &op, &mem, "5").await;

    // Rotate the key AFTER the root was anchored. Call order is log order in this fake, so this
    // really does record a `Delisted` positioned after the anchoring.
    mem.delist(REGISTRY, &rt, &backend);
    assert!(
        !mem.is_whitelisted_for(REGISTRY, &rt, &backend).await.unwrap(),
        "fixture is inert: the signer must be delisted NOW",
    );

    let (s, b) = call(
        &app,
        "POST",
        "/credentials/confirm",
        Some(&op),
        Some(serde_json::json!({ "recordId": record_id, "txHash": tx_hash })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "a forward-only delisting must not invalidate an issuance that already mined: {b}"
    );
}

/// The other direction, stated as a REQUIREMENT rather than an observation: a signer with no grant in
/// force when the root was anchored must still be refused at confirm. Without this the test above
/// would be satisfied by a confirm path that checked nothing at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signer_not_whitelisted_when_it_anchored_cannot_confirm() {
    let (app, op, backend, mem) = boot().await;
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend);

    let (record_id, root, tx_hash) = wallet_prepare_and_anchor(&app, &op, &mem, "6").await;

    // "Delisted BEFORE the anchoring" cannot be driven through the honest path — issuance is gated on
    // the whitelist, so the preflight refuses it and the confirm path is never reached. Seed it, for
    // the same reason `set_grant_history` exists at all.
    let anchored = mem
        .root_issued_at(ISSUER, &root)
        .expect("the root was anchored");
    mem.set_grant_history(
        REGISTRY,
        &rt,
        &backend,
        vec![
            GrantEvent {
                at: LogPoint {
                    block_number: 1,
                    log_index: 0,
                },
                granted: true,
            },
            GrantEvent {
                at: LogPoint {
                    block_number: anchored.block_number - 1,
                    log_index: 0,
                },
                granted: false,
            },
        ],
    );

    let (s, b) = call(
        &app,
        "POST",
        "/credentials/confirm",
        Some(&op),
        Some(serde_json::json!({ "recordId": record_id, "txHash": tx_hash })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a root anchored while the signer held no grant must not confirm: {b}"
    );
}
