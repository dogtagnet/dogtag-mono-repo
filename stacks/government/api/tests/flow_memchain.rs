//! End-to-end government flow over the in-memory MemChain + MemStore (no live node, no gas):
//! issue a TRAVEL_CLEARANCE credential (anchored on the emulated chain) → verify it → confirm the
//! verdict, the persisted credential, and the audit log. This is the demoable "one real E2E action".

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use government_api::app::{AppState, Config, TRAVEL_CLEARANCE};
use government_api::chain::{ChainClient, MemChain};
use government_api::store::{MemStore, Store};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

const ISSUER_ADDR: &str = "0x1111111111111111111111111111111111111111";
const REGISTRY_ADDR: &str = "0x5d86e4cf98a34ae0576f190f8d209c2943a9c79c";
const API_TOKEN: &str = "dogtag-gov-demo-token";

fn demo_state() -> (AppState, MemChain) {
    let cfg = Config {
        deployment_url: "http://localhost:44832".into(),
        rpc_url: "https://devrpc.roax.net".into(),
        chain_id: 135,
        issuer_registry_addr: REGISTRY_ADDR.into(),
        verification_registry_addr: "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87".into(),
        travel_clearance_issuer_addr: ISSUER_ADDR.into(),
        eu_health_cert_issuer_addr: "0x0000000000000000000000000000000000000000".into(),
        issuer_name: "DogTag Government Authority".into(),
        issuer_domain: "gov.example".into(),
        demo: true,
        api_token: Some("dogtag-gov-demo-token".into()),
    };
    let chain = MemChain::new();
    // whitelist the demo signer for TRAVEL_CLEARANCE so the issuer-identity pillar can be exercised.
    if let Some(signer) = chain.signer_address() {
        chain.whitelist(
            REGISTRY_ADDR,
            &government_api::app::record_type_key(TRAVEL_CLEARANCE),
            &signer,
        );
    }
    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    let state = AppState {
        store,
        chain: Arc::new(chain.clone()),
        cfg: Arc::new(cfg),
        feed: Arc::new(government_api::oversight::DisabledFeed),
    };
    (state, chain)
}

/// A backend that is a real node as far as every honesty surface is concerned, but emulates the chain
/// underneath. It exists because "live behaviour is COMPLETELY unchanged" is the load-bearing half of
/// the simulated-provenance contract, and a real `AlloyChain` cannot be exercised without a node -
/// `ChainBackend::Live` is the only input that drives the suppression, so overriding just that (plus
/// the real EIP-155 id an `AlloyChain` would report) pins the live path hermetically.
#[derive(Clone)]
struct LiveLikeChain(MemChain);

#[async_trait::async_trait]
impl ChainClient for LiveLikeChain {
    fn chain_id(&self) -> u64 {
        government_api::chain::ROAX_CHAIN_ID
    }
    fn backend(&self) -> government_api::chain::ChainBackend {
        government_api::chain::ChainBackend::Live
    }
    fn can_sign(&self) -> bool {
        self.0.can_sign()
    }
    fn signer_address(&self) -> Option<String> {
        self.0.signer_address()
    }
    async fn is_valid(
        &self,
        issuer_addr: &str,
        root: &str,
    ) -> Result<bool, government_api::chain::ChainError> {
        self.0.is_valid(issuer_addr, root).await
    }
    async fn issued_at(
        &self,
        issuer_addr: &str,
        root: &str,
    ) -> Result<alloy::primitives::U256, government_api::chain::ChainError> {
        self.0.issued_at(issuer_addr, root).await
    }
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, government_api::chain::ChainError> {
        self.0
            .is_whitelisted_for(registry_addr, record_type, signer)
            .await
    }
    async fn issue(
        &self,
        issuer_addr: &str,
        root: &str,
    ) -> Result<government_api::chain::SentTx, government_api::chain::ChainError> {
        self.0.issue(issuer_addr, root).await
    }
    async fn revoke(
        &self,
        issuer_addr: &str,
        root: &str,
    ) -> Result<government_api::chain::SentTx, government_api::chain::ChainError> {
        self.0.revoke(issuer_addr, root).await
    }
    async fn record_verification_zk_consent(
        &self,
        registry_addr: &str,
        a: &[String; 2],
        b: &[[String; 2]; 2],
        c: &[String; 2],
        pub_signals: &[String; 7],
    ) -> Result<government_api::chain::SentTx, government_api::chain::ChainError> {
        self.0
            .record_verification_zk_consent(registry_addr, a, b, c, pub_signals)
            .await
    }
}

/// `demo_state()` with the chain client swapped for one that reports itself LIVE.
fn live_like_state() -> (AppState, MemChain) {
    let (state, chain) = demo_state();
    let live = AppState {
        chain: Arc::new(LiveLikeChain(chain.clone())),
        ..state
    };
    (live, chain)
}

async fn call(state: &AppState, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    call_with_token(state, method, uri, body, None).await
}

/// GET a surface that renders HTML rather than JSON (the public `/r/:receiptId` receipt page).
async fn call_html(state: &AppState, uri: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = government_api::router(state.clone())
        .oneshot(req)
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// Issue an anchored TRAVEL_CLEARANCE and return its public receipt id.
async fn issue_receipt(state: &AppState, dog_tag_id: &str) -> String {
    let (status, issued) = call_auth(
        state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({
            "record_type": TRAVEL_CLEARANCE,
            "dog_tag_id": dog_tag_id,
            "fields": { "animalName": "Rex" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");
    assert_eq!(issued["anchored"], true, "issue must anchor: {issued}");
    issued["receiptId"]
        .as_str()
        .expect("receiptId minted")
        .to_string()
}

/// Same as `call` but presents the government operator bearer (the issue/mutation gate).
async fn call_auth(state: &AppState, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    call_with_token(state, method, uri, body, Some(API_TOKEN)).await
}

async fn call_with_token(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let req = builder.body(Body::from(body.to_string())).unwrap();
    let resp = government_api::router(state.clone())
        .oneshot(req)
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

/// `/health` must be HONEST that this stack is on a simulated chain.
///
/// This test previously asserted `canSign == true` here, which encoded the bug: with `chain_id: 135`
/// in config, a MemChain-backed stack reported `chainId:135, canSign:true` and was indistinguishable
/// from live ROAX with a funded signer. The contract now is that a simulated backend says so.
#[tokio::test]
async fn health_reports_ready_and_declares_the_simulated_backend() {
    let (state, _) = demo_state();
    let (status, v) = call(&state, "GET", "/health", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "ok");
    assert_eq!(v["service"], "government-api");

    // The backend is named explicitly, and not as a real chain.
    assert_eq!(v["backend"], "simulated");
    assert_eq!(v["simulated"], true);
    // chainId is null — NOT the configured 135 — because this backend is on no network.
    assert!(
        v["chainId"].is_null(),
        "a simulated backend must not report a real chainId, got {}",
        v["chainId"]
    );
    // No claim of real signing capability, and no real signer address.
    assert_eq!(v["canSign"], false);
    assert!(v["signer"].is_null(), "no real signer on a simulated chain");
    // The stand-in address is still visible, under an unmistakable name.
    assert!(v["simulatedSigner"].is_string());
}

#[tokio::test]
async fn issue_then_verify_end_to_end() {
    let (state, _) = demo_state();

    // ISSUE — build + anchor the TRAVEL_CLEARANCE credential on the emulated chain.
    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({
            "record_type": TRAVEL_CLEARANCE,
            "dog_tag_id": "7",
            "fields": { "animalName": "Rex", "countryOfDeparture": "CA" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");
    assert_eq!(issued["anchored"], true);
    assert!(issued["txHash"].is_string());
    // The receipt handle is minted + surfaced with its public lookup URLs.
    let receipt_id = issued["receiptId"]
        .as_str()
        .expect("receiptId minted")
        .to_string();
    assert_eq!(receipt_id.len(), 12, "12-char Crockford receipt id");
    assert_eq!(
        issued["statusUrl"],
        format!("/v1/receipts/{receipt_id}/status")
    );
    let root = issued["root"].as_str().unwrap().to_string();
    assert!(root.starts_with("0x") && root.len() == 66);
    let wrapped = issued["wrappedDoc"].clone();

    // VERIFY — integrity (offline) + on-chain isValid (MemChain) + issuer whitelist.
    let signer = state.chain.signer_address().unwrap();
    let (status, verdict) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": wrapped, "signer_addr": signer }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "verify: {verdict}");
    assert_eq!(verdict["verdict"], true);
    assert_eq!(verdict["fragments"]["integrity"], true);
    assert_eq!(verdict["fragments"]["onchain"], true);
    assert_eq!(verdict["fragments"]["issuerWhitelisted"], true);
    assert_eq!(verdict["recomputedRoot"], root);

    // audit + records surfaces reflect the flow (records carry the read-time effectiveStatus).
    let (_, records) = call_auth(&state, "GET", "/v1/records", Value::Null).await;
    assert_eq!(records["records"].as_array().unwrap().len(), 1);
    assert_eq!(records["records"][0]["effectiveStatus"], "VALID");
    assert_eq!(records["records"][0]["receiptId"], receipt_id);
    let (_, audit) = call(&state, "GET", "/v1/verifications", Value::Null).await;
    assert_eq!(audit["verifications"].as_array().unwrap().len(), 1);

    // PUBLIC receipt status (no auth): live on-chain read → VALID, PII-free (no importer section).
    let (status, st_json) = call(
        &state,
        "GET",
        &format!("/v1/receipts/{receipt_id}/status"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "public status: {st_json}");
    assert_eq!(st_json["effectiveStatus"], "VALID");
    assert_eq!(st_json["receiptId"], receipt_id);
    assert_eq!(st_json["root"], root);
    assert!(
        st_json["issuanceDate"].is_string(),
        "issuance date derived from chain"
    );
    assert!(
        st_json.get("importer").is_none() && st_json.get("subject").is_none(),
        "public status is PII-free: {st_json}"
    );
    // unknown receipt id -> 404.
    let (status, _) = call(
        &state,
        "GET",
        "/v1/receipts/ZZZZZZZZZZZZ/status",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn issue_stamps_m7_provenance_block_and_mirror_columns() {
    // M7 P2 (§4.2): a newly-issued credential carries a populated `protocol` block on the envelope
    // (BESIDE R) AND mirrored, queryable columns on the persisted record.
    let (state, _) = demo_state();
    let signer = state.chain.signer_address().unwrap();
    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "7", "fields": { "animalName": "Rex" } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");
    let root = issued["root"].as_str().unwrap().to_string();

    // Envelope carries the block, with the routing key + the issuer's own signer as the claim.
    //
    // `protocol.chainId` still carries the CONFIGURED id even on this simulated backend: it is a
    // non-optional `u64` in the shared standard crate (mirrored in the TS SDK and read by vet/admin/
    // mobile), so it has no null representation and cannot say "no real network" without inventing a
    // sentinel across every consumer. The honest, simulation-aware value lives on the persisted
    // column asserted below - and on the two PUBLIC receipt surfaces, which are what an outside party
    // reads (see `simulated_issuance_never_claims_a_real_chain_on_the_public_surfaces`).
    let block = &issued["wrappedDoc"]["protocol"];
    assert_eq!(block["chainId"], 135);
    assert_eq!(block["version"], "dogtag-levelb/1");
    assert_eq!(
        block["verificationRegistry"],
        "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87"
    );
    assert_eq!(block["issuerClone"], ISSUER_ADDR);
    assert_eq!(
        block["issuerSigner"].as_str().unwrap().to_lowercase(),
        signer.to_lowercase(),
        "issuerSigner is this authority's own signer (== on-chain issuedBy[R])"
    );

    // Persisted record mirrors the block into queryable columns (persist, don't just transmit).
    let cred = state
        .store
        .get_credential(&root)
        .await
        .expect("credential persisted");
    // NOT Some(135): the column is sourced from the CHAIN CLIENT, not from `Config::chain_id`, so a
    // simulated backend records "anchored on no real network" instead of inheriting the configured id.
    assert_eq!(
        cred.chain_id, None,
        "a simulated backend must not stamp a real chain id on the credential it issued"
    );
    assert_eq!(cred.protocol_version.as_deref(), Some("dogtag-levelb/1"));
    assert_eq!(
        cred.verification_registry.as_deref(),
        Some("0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87")
    );
    assert_eq!(
        cred.issuer_signer.map(|s| s.to_lowercase()),
        Some(signer.to_lowercase())
    );
    // issuer_addr (== issuerClone) is unchanged from before M7.
    assert_eq!(cred.issuer_addr, ISSUER_ADDR);
}

/// A SIMULATED backend must not pass for ROAX on the two PUBLIC, unauthenticated receipt surfaces.
///
/// These are the artifacts an OUTSIDE party trusts, so they are the worst place to inherit a
/// configured `chainId` and hand out `explorer.roax.net` links to txs that were never broadcast -
/// exactly the dishonesty `/health` was fixed for. The stored record keeps its links (the operator's
/// own audit trail is untouched); the suppression is on the public read.
#[tokio::test]
async fn simulated_issuance_never_claims_a_real_chain_on_the_public_surfaces() {
    let (state, _) = demo_state();
    let receipt_id = issue_receipt(&state, "11").await;

    // 1) public JSON status: no chain id, no explorer links, and it SAYS it is simulated rather than
    //    leaving a consumer to infer it from missing fields.
    let (status, st_json) = call(
        &state,
        "GET",
        &format!("/v1/receipts/{receipt_id}/status"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "public status: {st_json}");
    assert!(st_json["chainId"].is_null(), "got {}", st_json["chainId"]);
    assert_eq!(st_json["simulated"], true);
    assert!(
        st_json["explorerUrl"].is_null() && st_json["revokeExplorerUrl"].is_null(),
        "a simulated tx must not be advertised on a real block explorer: {st_json}"
    );
    // The verdict itself still works - honesty about the chain must not break the status read.
    assert_eq!(st_json["effectiveStatus"], "VALID");

    // 2) public HTML page: no ROAX claim, no explorer.roax.net anchor, and an explicit marker.
    let (status, html) = call_html(&state, &format!("/r/{receipt_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !html.contains("explorer.roax.net"),
        "public receipt page linked a block explorer for a tx that was never broadcast:\n{html}"
    );
    assert!(
        !html.contains("ROAX chainId"),
        "public receipt page claimed ROAX on a simulated backend:\n{html}"
    );
    assert!(
        html.contains("SIMULATED backend"),
        "the page must SAY why the provenance block is absent, not silently drop it:\n{html}"
    );
    // Still a working receipt page, not an error surface.
    assert!(html.contains(&receipt_id) && html.contains("VALID"));
}

/// The mirror image: on a LIVE backend every one of those surfaces is byte-for-byte what it was -
/// the real chain id, the real `explorer.roax.net` links, the "Anchored on ROAX chainId" row.
///
/// `LiveLikeChain` is the emulation reporting `ChainBackend::Live`, which is the ONLY difference that
/// drives the suppression, so this pins the live path hermetically (a real `AlloyChain` would need a node).
#[tokio::test]
async fn live_issuance_keeps_the_real_chain_id_and_explorer_links() {
    let (state, _) = live_like_state();
    let receipt_id = issue_receipt(&state, "12").await;

    let (_, st_json) = call(
        &state,
        "GET",
        &format!("/v1/receipts/{receipt_id}/status"),
        Value::Null,
    )
    .await;
    assert_eq!(st_json["chainId"], 135);
    assert_eq!(st_json["simulated"], false);
    assert!(
        st_json["explorerUrl"]
            .as_str()
            .unwrap_or_default()
            .starts_with("https://explorer.roax.net/tx/0x"),
        "live explorer link unchanged: {st_json}"
    );

    let (status, html) = call_html(&state, &format!("/r/{receipt_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("ROAX chainId 135"), "{html}");
    assert!(html.contains("https://explorer.roax.net/tx/0x"), "{html}");
    assert!(!html.contains("SIMULATED backend"), "{html}");

    // The queryable provenance column carries the real id (it is `None` only when simulated).
    let creds = state.store.list_credentials().await;
    assert_eq!(creds[0].chain_id, Some(135));
}

#[tokio::test]
async fn issue_requires_the_operator_bearer() {
    let (state, _) = demo_state();
    // No bearer -> 401; the credential is NOT built/persisted.
    let (status, body) = call(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "7" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "issue without token: {body}"
    );
    // Wrong bearer -> 401.
    let (status, _) = call_with_token(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "7" }),
        Some("wrong"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Nothing was persisted by the rejected issues.
    let (_, records) = call_auth(&state, "GET", "/v1/records", Value::Null).await;
    assert_eq!(records["records"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn verify_unanchored_root_is_invalid() {
    let (state, _) = demo_state();
    // Build (dry_run) but DON'T anchor — on-chain isValid must be false → verdict false.
    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "dog_tag_id": "9", "dry_run": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issued["anchored"], false);

    let (status, verdict) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": issued["wrappedDoc"].clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(verdict["fragments"]["integrity"], true);
    assert_eq!(verdict["fragments"]["onchain"], false);
    assert_eq!(verdict["verdict"], false);
}
