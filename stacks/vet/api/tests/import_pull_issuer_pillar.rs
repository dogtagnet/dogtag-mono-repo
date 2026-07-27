//! `POST /import/pull` — the SDK-backed third-party verify path — must not believe a contract or a
//! record-type label the document nominated.
//!
//! This route runs `crate::verify::third_party_verify`, which runs the SDK's
//! `dogtag_standard::verify::verify`. Before this change that orchestration made EVERY read against
//! `doc.issuer.document_store`, compared `issuedBy` to `doc.protocol.issuer_signer` — a SECOND
//! attacker-controlled field, since `protocol` is also outside the Merkle root — and fell open on a
//! read error (`Err(_) => FragmentState::Valid`). It had no factory anchor at all and no
//! issuer-whitelist pillar.
//!
//! **What the vet deployment was and was not exposed to, stated precisely.** Vet's `DnsAdapter` and
//! `RegistryAdapter` are config stand-ins pinned to this deployment's own `ISSUER_DOMAIN` and
//! `VACCINATION_ISSUER_ADDR` (`src/verify.rs`), and `identity` gates the verdict. That incidentally
//! refused the naive `documentStore` swap here — `a_forged_document_store_is_refused_by_identity_too`
//! pins that it is refused for BOTH reasons now, so nobody later reads this suite as evidence the
//! anchor is redundant. What got through was the RECORD-TYPE RELABEL: `issuer.recordType` sits
//! outside `R`, the SDK never read it, and this route had no coverage at all.
//!
//! MUTATION-CHECKED — each mutation was APPLIED, the named test observed going red, and the
//! mutation reverted. Each kills a DIFFERENT term:
//!   * dropping the `issuer_whitelist.permits_pass()` term from `credential_valid` reds
//!     `a_delisted_issuing_signer_cannot_import`;
//!   * folding `NoRecord` into the non-gating `UnavailableNoFactoryConfigured` branch reds
//!     `an_unanchored_root_cannot_import_but_a_factoryless_verifier_still_can`;
//!   * dropping the clone-vs-envelope `recordType` comparison reds
//!     `a_record_type_relabel_cannot_import`;
//!   * making `doc.issuer.document_store` the read target again — i.e. dropping the factory's
//!     preference in `issuer_addr` — reds `a_forged_document_store_is_refused_by_identity_too`;
//!   * returning `IssuerAnchor::NoFactoryConfigured` instead of `Err` for a MALFORMED
//!     `FACTORY_ADDR` reds `a_malformed_factory_addr_refuses_rather_than_degrading_to_fail_open`.
//!
//! **Recorded because it is the honest result, not the convenient one:** merely pointing the
//! `is_valid` CALL back at `doc.issuer.document_store` (while leaving `issuer_addr` resolved) does
//! NOT red anything in this file. On this deployment the forged document already fails `identity`,
//! and the genuine clone answers `isValid = true` for a genuine root, so the two read targets agree.
//! That term is pinned at the SDK level instead, by `a_forged_document_store_cannot_verify`, where
//! the fake can make the two contracts disagree. Claiming this file pinned it would have been
//! manufactured confidence.
//!
//! The factoryless half of the second test is the #96 regression pointed at this surface: our own
//! missing `FACTORY_ADDR` must not condemn a credential, while a chain that has never heard of the
//! root must.

mod common;

use axum::http::StatusCode;
use common::*;
use dogtag_standard::verify::{FragmentState, IssuerResolution, IssuerWhitelistState};
use serde_json::{json, Value};
use std::sync::Arc;
use vet_api::chain::{record_type_key, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";
/// A contract the factory never deployed, answering exactly as an attacker would want.
const HOSTILE: &str = "0x00000000000000000000000000000000000000c1";
/// This deployment's DOG_PROFILE anchor (`PROFILE_ISSUER_ADDR`) — a different clone from the
/// VACCINATION one, and deliberately NOT in `issuer_addrs`, exactly as on a real deployment.
const PROFILE_CLONE: &str = "0x00000000000000000000000000000000000000d1";

/// Everything a test needs to drive the route AND to call the verify path directly.
struct Deployment {
    app: axum::Router,
    op: String,
    /// The backend custody signer — the address that calls `issue(R)`, and therefore the address
    /// `issuedBy(R)` reports.
    backend: String,
    mem: MemChain,
    state: vet_api::app::AppState,
}

/// Boot a vet app whose MemChain registers every issuance under `FACTORY_ADDR`, so a genuine
/// credential resolves through the factory exactly as it does on a real deployment.
///
async fn boot_anchored() -> Deployment {
    let mem = MemChain::new().with_factory(FACTORY_ADDR);
    let state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state.clone());
    let (_admin, op, backend) = boot_custody(&app).await;
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend);
    mem.set_record_type(ISSUER, &rt);
    Deployment {
        app,
        op,
        backend,
        mem,
        state,
    }
}

/// A deployment whose factory is configured but has NO record of any issuance (`MemChain::new()`
/// without `with_factory`), or one that never set `FACTORY_ADDR` at all. The two look alike from the
/// outside and mean opposite things, which is exactly what
/// `an_unanchored_root_cannot_import_but_a_factoryless_verifier_still_can` pins.
async fn boot_unanchored(configure_factory: bool) -> Deployment {
    let mem = MemChain::new(); // issuances register under no factory
    let mut state = state_with(
        Arc::new(mem.clone()),
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    if !configure_factory {
        let mut cfg = (*state.cfg).clone();
        cfg.factory_addr = String::new();
        state.cfg = Arc::new(cfg);
    }
    let app = vet_api::router(state.clone());
    let (_admin, op, backend) = boot_custody(&app).await;
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend);
    mem.set_record_type(ISSUER, &rt);
    Deployment {
        app,
        op,
        backend,
        mem,
        state,
    }
}

/// Issue a real credential through the ordinary flow. The document is genuine in every respect — the
/// attacks below only rewrite fields OUTSIDE the Merkle root, which is why integrity keeps
/// reconciling and why those fields cannot be trusted.
async fn issue_doc(app: &axum::Router, op: &str, dog_tag_id: &str) -> (String, Value) {
    let (s, b) = call(
        app,
        "POST",
        "/credentials/prepare",
        Some(op),
        Some(json!({
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
        Some(json!({})),
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
    (root, doc)
}

/// Serve `doc` at `GET /share/{ref}` on an ephemeral loopback port, the way an owner's wallet API
/// does, so `POST /import/pull` performs its real outbound fetch. Bound to `127.0.0.1:0` — the port
/// is chosen by the OS for this test and the listener dies with it.
async fn serve_doc(doc: Value) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/share/:record_ref",
        axum::routing::get(move || {
            let doc = doc.clone();
            async move { axum::Json(doc) }
        }),
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

/// Drive the real route end-to-end. Returns (status, body).
async fn import_pull(app: &axum::Router, op: &str, doc: Value) -> (StatusCode, Value) {
    let base = serve_doc(doc).await;
    call(
        app,
        "POST",
        "/import/pull",
        Some(op),
        Some(json!({
            "userApiBase": base,
            "recordRef": "rec-1",
            "userJwt": "owner-token"
        })),
    )
    .await
}

// ---------------------------------------------------------------------------------------------

/// The baseline the rest of the suite is measured against: a genuine credential still imports, and
/// the new pillar reports a DEFINITE pass rather than an honest-looking non-answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_genuine_credential_still_imports() {
    let d = boot_anchored().await;
    let (app, op) = (&d.app, &d.op);
    let (_root, doc) = issue_doc(app, op, "1001").await;

    let (s, b) = import_pull(app, op, doc).await;
    assert_eq!(s, StatusCode::OK, "genuine import: {b}");
    assert_eq!(b["verdict"]["valid"], true);
    assert_eq!(b["verdict"]["issuerWhitelistState"], "passed");
    assert_eq!(b["verdict"]["issuerResolution"], "resolved");
    assert_eq!(
        b["verdict"]["issuerAddr"].as_str().unwrap().to_lowercase(),
        ISSUER.to_lowercase(),
        "reads are made against the clone the FACTORY named"
    );
}

/// **The live reproduction on this surface.**
///
/// `issuer.recordType` is outside the Merkle root, so relabelling a genuine credential across record
/// types is free — and the label is the entire point of the credential. A rabies certificate
/// presented as a travel clearance passed every pillar here, because the SDK never read `recordType`
/// at all, and vet's config-pinned identity check does not look at it either.
///
/// The clone's own immutable `recordType()` — fixed by the factory's `createIssuer` at KYC time — is
/// what refuses it.
#[tokio::test(flavor = "multi_thread")]
async fn a_record_type_relabel_cannot_import() {
    let d = boot_anchored().await;
    let (app, op) = (&d.app, &d.op);
    let (_root, doc) = issue_doc(app, op, "1002").await;

    // Unmodified, it imports.
    let (s, b) = import_pull(app, op, doc.clone()).await;
    assert_eq!(s, StatusCode::OK, "control: {b}");
    assert_eq!(b["verdict"]["valid"], true);

    // Relabel ONLY the record type. `documentStore` and `domain` are untouched, so vet's
    // config-pinned identity pillar still passes and cannot be credited with the refusal.
    let mut forged = doc.clone();
    assert_eq!(forged["issuer"]["recordType"], "VACCINATION");
    forged["issuer"]["recordType"] = json!("TRAVEL_CLEARANCE");
    assert_eq!(
        forged["issuer"]["documentStore"], doc["issuer"]["documentStore"],
        "the swap under test is the LABEL, not the contract"
    );

    let (s, b) = import_pull(app, op, forged).await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "relabelled import: {b}"
    );
}

/// The pillar is MANDATORY: a credential whose issuing signer is not authorised for the record type
/// cannot import, even though the chain reports the root genuinely issued and not revoked.
///
/// Mutation: drop `issuer_whitelist.permits_pass()` from `credential_valid` -> this goes red.
#[tokio::test(flavor = "multi_thread")]
async fn a_delisted_issuing_signer_cannot_import() {
    let d = boot_anchored().await;
    let (_root, doc) = issue_doc(&d.app, &d.op, "1003").await;

    // Control: while the issuer is authorised, it imports and the pillar is a DEFINITE pass.
    let (s, b) = import_pull(&d.app, &d.op, doc.clone()).await;
    assert_eq!(s, StatusCode::OK, "control: {b}");
    assert_eq!(b["verdict"]["issuerWhitelistState"], "passed");

    // Now delist the issuer. The credential is untouched and genuine in every other respect — the
    // root is anchored, the clone declares its record type, `isValid` is still true. Only the
    // authorisation is gone. It has to happen in this order because issuance is itself gated on the
    // whitelist, so "never authorised" is a state no real credential can be issued in.
    d.mem
        .delist(REGISTRY, &record_type_key("VACCINATION"), &d.backend);

    let (s, b) = import_pull(&d.app, &d.op, doc.clone()).await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unauthorised issuer must not import: {b}"
    );

    // A DEFINITE failure, not an honest-looking non-answer: that difference is the whole point.
    let wrapped: dogtag_standard::wrap::WrappedDoc = serde_json::from_value(doc).unwrap();
    let v = vet_api::verify::third_party_verify(&d.state, &wrapped).await;
    assert_eq!(
        v.issuance,
        FragmentState::Valid,
        "the chain still says the root is valid — only the pillar objects"
    );
    assert_eq!(v.issuer_whitelist, IssuerWhitelistState::Failed);
    assert!(!v.valid, "the pillar is mandatory");
}

/// `noRecord` (we asked, the chain has no record of this root) is evidence and must refuse.
/// `noFactoryConfigured` (we never asked) is OUR gap and must NOT condemn the credential.
///
/// This is the distinction #94 ruled on and #96 carried, and the half that matters most here is the
/// second: #96 regressed a capability on factory-less deployments and had to undo it. Unavailable
/// must not cost a capability that already worked.
///
/// Mutation: fold `NoRecord` into the non-gating branch -> the first half goes red.
#[tokio::test(flavor = "multi_thread")]
async fn an_unanchored_root_cannot_import_but_a_factoryless_verifier_still_can() {
    // (a) The factory IS configured, and it has no record of this root. We asked, and the answer is
    //     evidence about the credential.
    let asked = boot_unanchored(true).await;
    let (_root, doc) = issue_doc(&asked.app, &asked.op, "1005").await;
    let (s, b) = import_pull(&asked.app, &asked.op, doc).await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the chain has no record of this root: that IS evidence: {b}"
    );

    // (b) The same chain, but this verifier never configured a factory. We never asked, so we have
    //     learned nothing about the credential and must not fail it.
    let never = boot_unanchored(false).await;
    let (_r2, doc2) = issue_doc(&never.app, &never.op, "1006").await;
    let (s, b) = import_pull(&never.app, &never.op, doc2).await;
    assert_eq!(s, StatusCode::OK, "our own gap must not condemn: {b}");
    assert_eq!(b["verdict"]["valid"], true);
    assert_eq!(
        b["verdict"]["issuerWhitelistState"], "unavailableNoFactoryConfigured",
        "reported explicitly — never a silent absence, and never rendered as a pass"
    );
    assert_eq!(b["verdict"]["issuerResolution"], "noFactoryConfigured");
    // The two states are DIFFERENT on the wire. A caller that cannot tell them apart cannot tell a
    // check that never ran from one that succeeded.
    assert_ne!(
        b["verdict"]["issuerWhitelistState"], "passed",
        "not evaluated must never render as passed"
    );
}

/// The naive forgery — point `issuer.documentStore` at a contract that answers however the attacker
/// likes — is now refused for TWO independent reasons on this deployment, and the test asserts both
/// so nobody later reads the anchor as redundant here.
///
/// Vet's identity pillar is a config stand-in pinned to this deployment's own clone address, which
/// already refused this shape before the anchor existed. The anchor is what makes the refusal a
/// property of the SDK rather than of one deployment's adapter wiring: `issuerAddr` proves the reads
/// went to the factory-named clone and not to the contract the document nominated.
#[tokio::test(flavor = "multi_thread")]
async fn a_forged_document_store_is_refused_by_identity_too() {
    let d = boot_anchored().await;
    let (root, doc) = issue_doc(&d.app, &d.op, "1007").await;

    // A hostile contract that answers `isValid = true` / `issuedAt != 0` for any root it is asked
    // about — the contract the forged document nominates.
    d.mem
        .with_hostile_clone(HOSTILE, true, 42, HOSTILE, &record_type_key("VACCINATION"));
    let mut forged = doc.clone();
    forged["issuer"]["documentStore"] = json!(HOSTILE);

    let (s, b) = import_pull(&d.app, &d.op, forged).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "forged store: {b}");

    // The hostile contract genuinely WOULD have answered `true`, so this is the attack rather than
    // an unrelated failure — and the anchor is what sent the question elsewhere.
    assert!(
        d.state.chain.is_valid(HOSTILE, &root).await.unwrap(),
        "the fake must model a lying contract, or this test proves nothing"
    );
    let mut wrapped: dogtag_standard::wrap::WrappedDoc = serde_json::from_value(doc).unwrap();
    wrapped.issuer.document_store = HOSTILE.to_string();
    let v = vet_api::verify::third_party_verify(&d.state, &wrapped).await;
    assert_eq!(
        v.issuer_addr.to_lowercase(),
        ISSUER.to_lowercase(),
        "every verdict-deciding read went to the FACTORY-named clone, not the nominated one"
    );
    assert!(!v.valid);
}

/// A MALFORMED `FACTORY_ADDR` must not degrade into the fail-open unavailable state. A deployment
/// that set the value INTENDED to check, so one fat-fingered character silently disabling a security
/// pillar is misconfigure-to-bypass reached by accident.
///
/// #96 pins its half of this with HTTP 500 from `POST /verify/credential`. The SDK has no 500/502
/// channel — it returns a `Verdict` — so here the same posture arrives as `readFailed` →
/// `unresolved` → the credential is refused. Recorded as a deliberate divergence, and now measured
/// rather than asserted in a comment.
///
/// Mutation: return `IssuerAnchor::NoFactoryConfigured` instead of `Err` for the malformed branch
/// -> this goes red (it becomes a 200 with `unavailableNoFactoryConfigured`).
#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_factory_addr_refuses_rather_than_degrading_to_fail_open() {
    let d = boot_anchored().await;
    let (_root, doc) = issue_doc(&d.app, &d.op, "1008").await;

    // Control: with a well-formed factory it imports.
    let (s, _b) = import_pull(&d.app, &d.op, doc.clone()).await;
    assert_eq!(s, StatusCode::OK, "control");

    // 41 chars — one short. #96's fixture. Only the config changes: the store, custody and chain are
    // the SAME, so the operator session above stays valid and the credential stays byte-identical.
    let mut state = d.state.clone();
    let mut cfg = (*state.cfg).clone();
    cfg.factory_addr = "0x00000000000000000000000000000000000000f".to_string();
    state.cfg = Arc::new(cfg);
    let app = vet_api::router(state.clone());

    let (s, b) = import_pull(&app, &d.op, doc.clone()).await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a deployment that set the value intended to check: {b}"
    );

    let wrapped: dogtag_standard::wrap::WrappedDoc = serde_json::from_value(doc).unwrap();
    let v = vet_api::verify::third_party_verify(&state, &wrapped).await;
    assert_eq!(v.issuer_resolution, IssuerResolution::ReadFailed);
    assert_eq!(v.issuer_whitelist, IssuerWhitelistState::Unresolved);
    assert_ne!(
        v.issuer_whitelist,
        IssuerWhitelistState::UnavailableNoFactoryConfigured,
        "malformed must NOT be folded into the deliberately-absent fail-open state"
    );
    assert!(!v.valid);
}

/// **Scope proof, not a behaviour change.** `POST /import/pull` is the same route for both buttons on
/// the groomer's import page — the `kind` toggle only changes labels — so a DOG_PROFILE document can
/// be pointed at it. #94 deliberately kept this pillar OFF the profile branch, on the grounds that
/// applying it there would resolve indeterminate and refuse every profile.
///
/// The measured answer on THIS deployment: a profile document was already refused before the pillar
/// existed, by `identity`. Vet's `DnsAdapter`/`RegistryAdapter` pin `documentStore` to
/// `issuer_addrs`, which `prepare` also sources — so every document this deployment can wrap has a
/// store in that map, and a profile document (anchored on `PROFILE_ISSUER_ADDR`, a different clone)
/// never did.
///
/// Proven WITHOUT a git checkout: the second half runs the same document through a FACTORY-LESS
/// verifier, where this pillar is `unavailableNoFactoryConfigured` and therefore contributes nothing
/// to the verdict. It is still refused there, so the refusal is not this change's doing.
#[tokio::test(flavor = "multi_thread")]
async fn a_profile_document_was_already_refused_by_identity_and_still_is() {
    let d = boot_anchored().await;
    let (_root, doc) = issue_doc(&d.app, &d.op, "1009").await;

    // Shape it like a DOG_PROFILE credential: anchored on a different clone, labelled accordingly.
    // Both fields are outside `R`, so integrity still reconciles.
    let mut profile = doc.clone();
    profile["issuer"]["recordType"] = json!("DOG_PROFILE");
    profile["issuer"]["documentStore"] = json!(PROFILE_CLONE);

    let (s, b) = import_pull(&d.app, &d.op, profile.clone()).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "refused: {b}");

    // The load-bearing half: on a factory-less verifier the pillar cannot contribute, so whatever
    // refuses it there is NOT this change.
    let never = boot_unanchored(false).await;
    let wrapped: dogtag_standard::wrap::WrappedDoc = serde_json::from_value(profile).unwrap();
    let v = vet_api::verify::third_party_verify(&never.state, &wrapped).await;
    assert_eq!(
        v.issuer_whitelist,
        IssuerWhitelistState::UnavailableNoFactoryConfigured,
        "the pillar is contributing nothing on this deployment"
    );
    assert_eq!(
        v.identity,
        FragmentState::Invalid,
        "identity is what refuses a profile document here, and it did so before this change"
    );
    assert!(!v.valid);
}

/// The SDK's `record_type_key` is the cross-boundary tie to the on-chain whitelist key. If it ever
/// drifted from the backend's, every whitelist question would ask about nothing and the pillar would
/// report an honest-looking `unresolved` forever — a security control silently switched off.
#[test]
fn record_type_key_matches_the_vet_backend() {
    for label in ["VACCINATION", "TRAVEL_CLEARANCE", "DOG_PROFILE", ""] {
        assert_eq!(
            dogtag_standard::verify::record_type_key(label),
            record_type_key(label),
            "record-type keying drifted for {label:?}"
        );
    }
}
