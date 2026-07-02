//! Cross-role interop: the government stack VERIFIES a credential ISSUED by the VET role.
//!
//! This is the codified "vet ISSUES -> government VERIFIES" link of the three-role showcase. Because
//! every role stack builds/verifies through the SAME open standard (`dogtag-standard-rs`, single
//! Poseidon root `R`), a credential a vet issues is verifiable, unchanged, by the government verifier.
//! Here the vet's on-chain anchor (a real `DogTagIssuer.issue` on ROAX) is stood in for by MemChain
//! (the shared-chain state), so the whole cross-role flow runs deterministically with no node/gas.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dogtag_standard::wrap::{wrap_document, IssuerMeta};
use government_api::app::{AppState, Config};
use government_api::chain::{ChainClient, MemChain};
use government_api::store::{MemStore, Store};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

// The live ROAX VACCINATION clone + IssuerRegistry (contracts/deployments/roax.json). Used here only
// as stable identifiers; MemChain stands in for the chain state.
const VACC_CLONE: &str = "0x5c703910111f942ee0f47e02214291b5274cdb53";
const REGISTRY: &str = "0x5d86e4cf98a34ae0576f190f8d209c2943a9c79c";
const VET_SIGNER: &str = "0x00000000000000000000000000000000000000a1";

/// Build a VACCINATION wrapped credential exactly as the vet stack would: a typed-scalar VC wrapped
/// through the shared SDK's `wrap_document` (the same primitive `vet-api`'s `app::wrap` calls).
fn vet_issue_vaccination(dog_tag_id: &str) -> (String, Value) {
    // Typed-scalar leaves ({tag,value}) — tag 3 = integer, tag 2 = string (see dogtag-standard types).
    let vc = json!({
        "credentialSubject": {
            "dogTagId": { "tag": 3, "value": dog_tag_id },
            "vaccineProductName": { "tag": 2, "value": "Nobivac Rabies" },
            "batchLotNumber": { "tag": 2, "value": "A2201" },
            "vaccinationDate": { "tag": 2, "value": "2026-01-15" }
        }
    });
    let meta = IssuerMeta {
        name: "Seaport Vet".into(),
        domain: "vet.local".into(),
        document_store: VACC_CLONE.into(),
        record_type: "VACCINATION".into(),
    };
    let mut salt = || {
        let mut s = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut s);
        s
    };
    let doc = wrap_document(&vc, meta, &mut salt).expect("wrap");
    let root = doc.signature.merkle_root.clone();
    (root, serde_json::to_value(&doc).unwrap())
}

fn government_stack(chain: MemChain) -> AppState {
    let cfg = Config {
        deployment_url: "http://localhost:44832".into(),
        rpc_url: "https://devrpc.roax.net".into(),
        chain_id: 135,
        issuer_registry_addr: REGISTRY.into(),
        travel_clearance_issuer_addr: "0x1111111111111111111111111111111111111111".into(),
        eu_health_cert_issuer_addr: "0x0000000000000000000000000000000000000000".into(),
        issuer_name: "Example Competent Authority".into(),
        issuer_domain: "gov.example".into(),
        demo: true,
        api_token: Some("dogtag-gov-demo-token".into()),
    };
    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    AppState {
        store,
        chain: Arc::new(chain),
        cfg: Arc::new(cfg),
    }
}

async fn post(state: &AppState, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = government_api::router(state.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn government_verifies_a_vet_issued_credential() {
    // 1) VET role: issue a VACCINATION credential + anchor its root on the (emulated) chain, and
    //    whitelist the vet signer for VACCINATION (what the admin approve flow does on-chain).
    let (root, wrapped) = vet_issue_vaccination("7");
    let chain = MemChain::new();
    chain.issue(VACC_CLONE, &root).await.expect("vet anchors root");
    chain.whitelist(
        REGISTRY,
        &government_api::app::record_type_key("VACCINATION"),
        VET_SIGNER,
    );

    // 2) GOVERNMENT role: a SEPARATE stack, sharing only the chain, verifies the vet's credential.
    let gov = government_stack(chain);
    let (status, v) = post(
        &gov,
        "/v1/verify",
        json!({ "wrapped_doc": wrapped, "signer_addr": VET_SIGNER }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "verify: {v}");
    assert_eq!(v["recordType"], "VACCINATION");
    assert_eq!(v["fragments"]["integrity"], true, "integrity: {v}");
    assert_eq!(v["fragments"]["onchain"], true, "on-chain isValid: {v}");
    assert_eq!(v["fragments"]["issuerWhitelisted"], true, "issuer identity: {v}");
    assert_eq!(v["verdict"], true);
    assert_eq!(v["recomputedRoot"], root);
}

#[tokio::test]
async fn government_rejects_a_tampered_vet_credential() {
    // A vet credential whose cleartext was altered after issuance: integrity recompute != anchored R.
    let (root, mut wrapped) = vet_issue_vaccination("7");
    let chain = MemChain::new();
    chain.issue(VACC_CLONE, &root).await.unwrap();
    // Tamper: flip a leaf's cleartext value (keep the old root in the signature).
    wrapped["data"]["credentialSubject"]["batchLotNumber"] =
        json!(format!("{}:2:TAMPERED", "00000000000000000000000000000000"));

    let gov = government_stack(chain);
    let (status, v) = post(&gov, "/v1/verify", json!({ "wrapped_doc": wrapped })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["fragments"]["integrity"], false, "tamper must fail integrity: {v}");
    assert_eq!(v["verdict"], false);
}
