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
    async fn issued_by(
        &self,
        issuer_addr: &str,
        root: &str,
    ) -> Result<Option<String>, government_api::chain::ChainError> {
        self.0.issued_by(issuer_addr, root).await
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

/// AUDIT FINDING (2026-07-27): a credential relabelled to a different issuing authority returned
/// `verdict: true`.
///
/// `check_integrity` folds only `data` + `privacy.obfuscated`, so the whole `issuer` block - `name`,
/// `domain`, and critically `documentStore`, the address every `isValid()` is made against - sits
/// OUTSIDE the Merkle root. The live demonstration took a genuine credential, changed nothing in
/// `data`, relabelled the issuer to "Ministry of Health of Singapore / moh.gov.sg", and still got
/// `{"verdict":true,"fragments":{...,"issuerWhitelisted":null}}` - a pass riding on a pillar that
/// never ran.
///
/// The sharp version of that attack - the one this pillar exists for - is the `documentStore` swap:
/// point it at a contract the attacker controls which DOES answer `isValid(root) == true`. Integrity
/// passes (data untouched) and the on-chain read passes (the attacker's clone says yes), so before
/// this change nothing was left to object.
///
/// The clone here is therefore genuinely anchored, and `onchain` is asserted TRUE - otherwise the test
/// would pass for the wrong reason (a clone that simply has no such root fails the issuance pillar and
/// never exercises this one at all).
///
/// SCOPE: relabelling `name`/`domain` ALONE still verifies, and deliberately so. This pillar asks the
/// chain who issued the root; it never reads those two fields. Binding them to the root-covered
/// `data.issuer` DID is the separate issuer-identity assertion (audit rec 6) landing with the DNS
/// issuer-binding work - see `a_name_only_relabel_is_out_of_this_pillars_reach` below, which pins that
/// boundary honestly rather than letting this test imply coverage it does not have.
#[tokio::test]
async fn a_forged_issuer_clone_that_answers_isvalid_is_still_refused() {
    let (state, chain) = demo_state();
    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "11", "fields": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");
    let root = issued["root"].as_str().unwrap().to_string();

    // Baseline: the genuine document passes with the pillar RESOLVED - so a later refusal is the
    // pillar working, not the fixture being broken.
    let (_, genuine) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": issued["wrappedDoc"].clone() }),
    )
    .await;
    assert_eq!(genuine["verdict"], true, "genuine: {genuine}");
    assert_eq!(genuine["fragments"]["issuerWhitelisted"], true);

    // The attacker anchors the SAME root on their OWN clone, from their OWN signer. `.clone()` shares
    // the emulated chain state; `with_signer` changes who `msg.sender` is, so the forged clone records
    // `issuedBy[root] = ATTACKER` exactly as a real deployment would.
    const ATTACKER_CLONE: &str = "0x00000000000000000000000000000000deadbeef";
    const ATTACKER_SIGNER: &str = "0x00000000000000000000000000000000000000ff";
    chain
        .clone()
        .with_signer(ATTACKER_SIGNER)
        .issue(ATTACKER_CLONE, &root)
        .await
        .expect("attacker anchors the stolen root on their own clone");

    let mut forged = issued["wrappedDoc"].clone();
    forged["issuer"]["name"] = json!("Ministry of Health of Singapore");
    forged["issuer"]["domain"] = json!("moh.gov.sg");
    forged["issuer"]["documentStore"] = json!(ATTACKER_CLONE);
    let (status, v) = call(&state, "POST", "/v1/verify", json!({ "wrapped_doc": forged })).await;

    assert_eq!(status, StatusCode::OK, "verify: {v}");
    assert_eq!(v["fragments"]["integrity"], true, "data is untouched: {v}");
    assert_eq!(
        v["fragments"]["onchain"], true,
        "the attacker's clone DOES answer isValid - this is the whole point: {v}"
    );
    // The signer resolves, and is not one the registry authorizes for this record type.
    assert_eq!(v["signerAddr"], json!(ATTACKER_SIGNER), "{v}");
    assert_eq!(v["fragments"]["issuerWhitelisted"], false, "{v}");
    assert_eq!(v["verdict"], false, "forged issuer clone must NOT verify: {v}");
}

/// The boundary of this pillar, pinned so nobody reads the test above as broader than it is.
///
/// Relabelling only `name`/`domain` leaves the real `documentStore` in place, so the chain still
/// reports the real, whitelisted issuing signer and the credential still verifies. That is correct for
/// THIS pillar - it authenticates the issuing key, not the label rendered next to it - and it is why
/// the separate `issuer.domain` ↔ root-covered `data.issuer` DID assertion (audit rec 6, shipping with
/// the DNS issuer-binding work) is still required before any surface displays that label as authority.
#[tokio::test]
async fn a_name_only_relabel_is_out_of_this_pillars_reach() {
    let (state, _) = demo_state();
    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "14", "fields": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");

    let mut relabelled = issued["wrappedDoc"].clone();
    relabelled["issuer"]["name"] = json!("Ministry of Health of Singapore");
    relabelled["issuer"]["domain"] = json!("moh.gov.sg");
    let (_, v) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": relabelled }),
    )
    .await;

    // Documents the CURRENT, intentional gap. If a future change makes this false, that is the domain
    // assertion landing - update this test, do not treat it as a regression.
    assert_eq!(v["verdict"], true, "{v}");
    assert_eq!(v["fragments"]["issuerWhitelisted"], true, "{v}");
}

/// A signer that DID issue the root but is not whitelisted for that record type is a resolved `false`,
/// not an indeterminate — and equally must not pass. (Whitelists can be revoked after issuance; the
/// authority is the registry now, not at mint time.)
#[tokio::test]
async fn a_resolved_but_unwhitelisted_issuer_fails_the_verdict() {
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
        api_token: Some(API_TOKEN.into()),
    };
    // Identical to `demo_state()` except the signer is NEVER whitelisted for TRAVEL_CLEARANCE.
    let state = AppState {
        store: Arc::new(MemStore::new()) as Arc<dyn Store>,
        chain: Arc::new(MemChain::new()),
        cfg: Arc::new(cfg),
        feed: Arc::new(government_api::oversight::DisabledFeed),
    };

    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "12", "fields": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");

    let (status, v) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": issued["wrappedDoc"].clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "verify: {v}");
    assert_eq!(v["fragments"]["integrity"], true);
    assert_eq!(v["fragments"]["onchain"], true, "the root IS anchored: {v}");
    assert_eq!(v["fragments"]["issuerWhitelisted"], false, "{v}");
    assert!(v["signerAddr"].is_string(), "signer resolved: {v}");
    assert_eq!(v["verdict"], false, "{v}");
}

/// AUDIT FINDING (2026-07-27): the receipt QR encoded `https://gov.example/r/<id>` — NXDOMAIN — because
/// three renderers built it from `issuer.domain`, a `did:web` IDENTITY, instead of a reachable host.
///
/// Issuance now stamps `protocol.statusBaseUrl` from `DEPLOYMENT_URL`, the same base the (already
/// correct) share QR uses. Renderers read that and nothing else, so the QR points where the phone can
/// actually go — while `issuer.domain` stays the stable identity it is supposed to be.
#[tokio::test]
async fn issuance_stamps_a_reachable_status_base_url_not_the_did_web_domain() {
    let (state, _) = demo_state();
    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "13", "fields": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");

    let doc = &issued["wrappedDoc"];
    let base = doc["protocol"]["statusBaseUrl"]
        .as_str()
        .unwrap_or_else(|| panic!("statusBaseUrl stamped: {doc}"));
    assert_eq!(base, "http://localhost:44832", "== DEPLOYMENT_URL: {doc}");
    assert!(!base.ends_with('/'), "no trailing slash → no `//r/`: {base}");

    // The identity field is untouched and is NOT the QR base. Corrupting a root-covered did:web with a
    // rotating deployment hostname was the tempting non-fix; this asserts we did not take it.
    assert_eq!(doc["issuer"]["domain"], "gov.example");
    assert_ne!(doc["issuer"]["domain"], json!(base));

    // What a renderer builds now resolves on THIS deployment.
    let receipt_id = issued["receiptId"].as_str().expect("receiptId");
    let (status, _) = call(&state, "GET", &format!("/r/{receipt_id}"), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "the stamped base serves /r/:id");
}
