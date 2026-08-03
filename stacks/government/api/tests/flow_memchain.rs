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
const FACTORY: &str = "0xed20269e3ebf0119739aab5258741f3aeb49f140";
/// A LAN IP, not `localhost`: a loopback name is one of the bases issuance REFUSES to stamp into a
/// credential (no phone can resolve it), so a localhost deployment would leave `statusBaseUrl` unset
/// and the receipt-QR assertions below would be exercising the degradation branch by accident.
const DEPLOYMENT_URL: &str = "http://192.168.1.20:44832";

fn demo_state() -> (AppState, MemChain) {
    let cfg = Config {
        deployment_url: DEPLOYMENT_URL.into(),
        rpc_url: "https://devrpc.roax.net".into(),
        chain_id: 135,
        issuer_registry_addr: REGISTRY_ADDR.into(),
        factory_addr: "0x00000000000000000000000000000000000000fa".into(),
        issuer_domain_registry_addr: "0x00000000000000000000000000000000000000dd".into(),
        dns_doh_endpoint: String::new(),
        verification_registry_addr: "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87".into(),
        travel_clearance_issuer_addr: ISSUER_ADDR.into(),
        eu_health_cert_issuer_addr: "0x0000000000000000000000000000000000000000".into(),
        issuer_name: "DogTag Government Authority".into(),
        issuer_domain: "gov.example".into(),
        demo: true,
        api_token: Some("dogtag-gov-demo-token".into()),
    };
    // Issuances register under this factory, as a real `issue()` does via `registerRoot`.
    let chain = MemChain::new().with_factory("0x00000000000000000000000000000000000000fa");
    // The demo clone really was deployed by the DogTag factory. Seeded because link-1 provenance is a
    // verdict pillar: an unseeded pair reads as a DEFINITE `notFactoryDeployed`, which fails the verdict
    // — correctly, but it would make this suite about provenance rather than about the flow.
    chain.set_factory_clone("0x00000000000000000000000000000000000000fa", ISSUER_ADDR, true);
    // Declare the clone's own immutable `recordType()`, mirroring what the factory's `createIssuer`
    // fixes on a real clone. The issuer pillar asks the RESOLVED clone which record type it issues, so
    // an undeclared clone leaves that pillar indeterminate for the whole harness.
    chain.set_record_type(ISSUER_ADDR, &government_api::app::record_type_key(TRAVEL_CLEARANCE));
    // whitelist the demo signer for TRAVEL_CLEARANCE so the issuer-identity pillar can be exercised.
    if let Some(signer) = chain.signer_address() {
        chain.set_issuance_capability(REGISTRY_ADDR, ISSUER_ADDR, &signer, true);
    }
    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    let state = AppState {
        store,
        chain: Arc::new(chain.clone()),
        cfg: Arc::new(cfg),
        dns: std::sync::Arc::new(dogtag_dns_rs::BindingResolver::production(String::new())),
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
        at_block: Option<u64>,
    ) -> Result<bool, government_api::chain::ChainError> {
        self.0.is_valid(issuer_addr, root, at_block).await
    }
    async fn issued_at(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<alloy::primitives::U256, government_api::chain::ChainError> {
        self.0.issued_at(issuer_addr, root, at_block).await
    }
    async fn block_number(&self) -> Result<u64, government_api::chain::ChainError> {
        Ok(1)
    }
    async fn root_issuer(
        &self,
        _factory_addr: &str,
        _root: &str,
        _at_block: Option<u64>,
    ) -> Result<Option<String>, government_api::chain::ChainError> {
        Ok(None)
    }
    async fn is_factory_clone(
        &self,
        _factory_addr: &str,
        _clone_addr: &str,
        _at_block: Option<u64>,
    ) -> Result<bool, government_api::chain::ChainError> {
        Ok(true)
    }
    async fn issuer_onchain_name(
        &self,
        _clone_addr: &str,
        _at_block: Option<u64>,
    ) -> Result<String, government_api::chain::ChainError> {
        Ok(String::new())
    }
    /// This harness's issuer claims no on-chain domain, which is the normal day-one state.
    async fn issuer_claimed_domain(
        &self,
        _domain_registry_addr: &str,
        _clone_addr: &str,
        _at_block: Option<u64>,
    ) -> Result<Option<government_api::chain::DomainClaim>, government_api::chain::ChainError> {
        Ok(None)
    }

    async fn issued_by(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, government_api::chain::ChainError> {
        self.0.issued_by(issuer_addr, root, at_block).await
    }
    async fn issuer_record_type(
        &self,
        issuer_addr: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, government_api::chain::ChainError> {
        self.0.issuer_record_type(issuer_addr, at_block).await
    }
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
        at_block: Option<u64>,
    ) -> Result<bool, government_api::chain::ChainError> {
        self.0
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
        self.0
            .whitelisted_at_issuance(issuer_addr, signer, root, at_block)
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
/// point it at a contract the attacker controls which DOES answer `isValid(root) == true`, AND names a
/// genuinely whitelisted signer as the issuer. Integrity passes (data untouched), the on-chain read
/// passes (the attacker's contract says yes), and asking that same contract who issued the root yields
/// an address the real registry really does authorize - so every question the verifier knew how to ask
/// was being answered by the suspect.
///
/// What refuses it is refusing to take the document's word for WHICH contract to ask: the clone is
/// resolved from the factory's write-once `rootIssuer[R]` index, which only a factory-deployed clone
/// can ever write to. The hostile contract is therefore absent from it, and the envelope naming a
/// contract other than the one the chain says issued the root is itself the definite failure.
///
/// The hostile contract's own answers are asserted first, so this cannot pass for an unrelated reason
/// (a contract that simply fails `isValid` would be refused by the issuance pillar and never reach
/// this one). Both attack shapes are covered: a genuine root with a swapped `documentStore` (resolves
/// to the REAL clone → definite false) and a root no clone ever issued (resolves to nothing →
/// indeterminate, which is equally not a pass).
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
    let honest_signer = genuine["signerAddr"].as_str().unwrap().to_string();

    // The attacker deploys a contract of their own that is NOT a factory clone, and makes it answer
    // every question favourably: valid, anchored, the right record type, and issued by the authority's
    // OWN genuinely-whitelisted signer.
    const ATTACKER_CLONE: &str = "0x00000000000000000000000000000000deadbeef";
    chain.with_hostile_clone(
        ATTACKER_CLONE,
        true,
        1_782_864_012,
        &honest_signer,
        &government_api::app::record_type_key(TRAVEL_CLEARANCE),
    );
    // The hostile contract really does answer the way the attack requires - asserted directly, so the
    // refusal below cannot be credited to a fixture that never posed the attack.
    assert!(
        chain.is_valid(ATTACKER_CLONE, &root, None).await.unwrap(),
        "the hostile contract must answer isValid=true - that is the attack"
    );
    assert_eq!(
        chain.issued_by(ATTACKER_CLONE, &root, None).await.unwrap(),
        Some(honest_signer.clone()),
        "the hostile contract names a genuinely whitelisted signer"
    );
    // ...and that signer really DOES hold the issuance capability on the genuine clone, so the
    // attack turns entirely on WHICH contract is asked rather than on an absent grant.
    assert_eq!(
        chain
            .whitelisted_at_issuance(ISSUER_ADDR, &honest_signer, &root, None)
            .await
            .unwrap(),
        government_api::chain::GrantAtIssuance::Authorized,
        "the signer really is authorised on the genuine clone"
    );
    // It can never appear in the factory index: only a clone may call `registerRoot`, and the genuine
    // clone already claimed this root write-once.
    assert_eq!(
        chain.root_issuer("0x00000000000000000000000000000000000000fa", &root, None).await.unwrap().as_deref(),
        Some(ISSUER_ADDR),
        "the factory still names the REAL issuing clone"
    );
    assert!(
        chain
            .clone()
            .with_signer("0x00000000000000000000000000000000000000ff")
            .issue(ATTACKER_CLONE, &root)
            .await
            .is_err(),
        "re-anchoring a claimed root must revert (`root taken`), as registerRoot does on chain"
    );

    let mut forged = issued["wrappedDoc"].clone();
    forged["issuer"]["name"] = json!("Ministry of Health of Singapore");
    forged["issuer"]["domain"] = json!("moh.gov.sg");
    forged["issuer"]["documentStore"] = json!(ATTACKER_CLONE);
    let (status, v) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": forged }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "verify: {v}");
    assert_eq!(v["fragments"]["integrity"], true, "data is untouched: {v}");
    // Every read was made against the clone the FACTORY named, so the hostile contract's answers were
    // never consulted at all.
    assert_eq!(v["issuerAddr"], json!(ISSUER_ADDR), "{v}");
    // The pillar therefore reports the REAL signer honestly - it passes, because the signer it found
    // genuinely is authorised. The forgery is caught one step earlier, at resolution: the envelope
    // names a different contract than the one the chain says issued this root.
    assert_eq!(v["fragments"]["issuerWhitelisted"], true, "{v}");
    assert_eq!(v["issuerResolution"]["documentStoreDiffers"], true, "{v}");
    assert_eq!(
        v["verdict"], false,
        "forged issuer clone must NOT verify: {v}"
    );

    // The other shape: a root NO clone ever issued, pointed at the same obliging contract. Nothing
    // resolves, so the pillar is indeterminate — which is equally never a pass.
    let mut fabricated = forged.clone();
    fabricated["signature"]["merkleRoot"] = json!(format!("0x{}", "ab".repeat(32)));
    let (_, v2) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": fabricated }),
    )
    .await;
    assert!(
        v2["fragments"]["issuerWhitelisted"].is_null(),
        "unclaimed root -> indeterminate: {v2}"
    );
    assert_eq!(v2["verdict"], false, "{v2}");
}

/// The boundary of this pillar, and the moment the gap beside it CLOSED.
///
/// Relabelling only `name`/`domain` leaves the real `documentStore` in place, so the issuer-whitelist
/// pillar still reports the real, whitelisted signer - it authenticates the issuing KEY, not the label
/// rendered next to it, and that remains true (`issuerWhitelisted: true` below).
///
/// This test previously asserted `verdict: true`, documenting that a name-only relabel slipped through,
/// and said in as many words: if a future change makes this false, that is the domain assertion landing
/// - update this test, do not treat it as a regression. That change has now landed (the root-covered
/// `data.issuer` DID assertion, audit rec 6, from the DNS issuer-binding work), so the verdict is now
/// correctly `false` on a MISMATCH while the whitelist pillar independently still passes.
///
/// Kept rather than deleted because it pins WHICH pillar catches WHICH forgery: the whitelist pillar
/// says nothing about the label, and the DID assertion says nothing about the key. Both are needed.
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

    // The whitelist pillar is untouched by a label swap - it resolved the real signer and passed.
    assert_eq!(v["fragments"]["issuerWhitelisted"], true, "{v}");
    // ...and the DID assertion is what now catches the relabel, so the overall verdict fails.
    assert_eq!(v["fragments"]["issuerDidAssertion"], "mismatch", "{v}");
    assert_eq!(v["verdict"], false, "{v}");
}

/// The forgery through the ABSENCE of the field: strip `issuer.documentStore` instead of pointing it
/// somewhere hostile.
///
/// Exempting a blank claim from the envelope-vs-factory comparison bought nothing - the factory
/// supplies the address either way - while letting anyone holding a genuine credential skip the
/// misrepresentation check entirely. The chain's answer was then backfilled into `issuerAddr`, so an
/// unauthenticated caller could launder a stripped envelope into a clean-looking `verdict: true` row
/// in the verifications audit log, naming an issuer the document itself never claimed.
///
/// Integrity is asserted TRUE first, so the refusal is provably the clone check and not the document
/// failing to parse or fold: the `issuer` block sits outside the Merkle root, so emptying a field in
/// it leaves the recompute untouched.
#[tokio::test]
async fn an_absent_document_store_is_a_mismatch_not_an_exemption() {
    let (state, _) = demo_state();
    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "17", "fields": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");

    // Baseline: untouched, this exact document passes with the pillar RESOLVED.
    let (_, genuine) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": issued["wrappedDoc"].clone() }),
    )
    .await;
    assert_eq!(genuine["verdict"], true, "genuine: {genuine}");

    let mut stripped = issued["wrappedDoc"].clone();
    stripped["issuer"]["documentStore"] = json!("");
    let (status, v) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": stripped }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "verify: {v}");
    assert_eq!(
        v["fragments"]["integrity"], true,
        "the issuer block is outside R, so the recompute is untouched: {v}"
    );
    // The pillar itself resolved fine and passes honestly - it found the real clone via the factory
    // and the real, whitelisted signer. The REFUSAL comes from the clone-agreement check: an absent
    // `documentStore` is a mismatch like any other, not an exemption.
    assert_eq!(v["fragments"]["issuerWhitelisted"], true, "{v}");
    assert_eq!(v["fragments"]["issuerWhitelistState"], "passed", "{v}");
    assert_eq!(v["issuerResolution"]["documentStoreDiffers"], true, "{v}");
    assert_eq!(
        v["verdict"], false,
        "a stripped documentStore must not verify: {v}"
    );
}

/// The same forgery through the OTHER field. `POST /v1/verify` is unauthenticated, so `issuer_addr`
/// is attacker-supplied, not operator-supplied: if it were allowed to SELECT which contract answers,
/// the factory anchor would be bypassed without touching `documentStore` at all. It may only
/// TIGHTEN - exactly like `signer_addr`.
///
/// Both shapes are covered: a genuine root with a disagreeing override (resolves to the real clone →
/// definite false), and a fabricated root that no clone ever issued (nothing resolves, and no
/// caller-named address may stand in → indeterminate). Neither is a pass.
#[tokio::test]
async fn an_issuer_addr_override_can_only_tighten_never_select_the_contract() {
    let (state, chain) = demo_state();
    const HOSTILE: &str = "0x00000000000000000000000000000000deadbe01";
    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "16", "fields": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");
    let honest_signer = chain.signer_address().unwrap();

    // The obliging contract: valid, right record type, issued by a genuinely whitelisted signer.
    chain.with_hostile_clone(
        HOSTILE,
        true,
        1_782_864_012,
        &honest_signer,
        &government_api::app::record_type_key(TRAVEL_CLEARANCE),
    );

    // (a) genuine root, override pointing elsewhere -> the reads still go to the factory's clone.
    let (_, v) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": issued["wrappedDoc"].clone(), "issuer_addr": HOSTILE }),
    )
    .await;
    // The override is REPORTED (labelled `operatorOverride`) but never gets to answer: both
    // verdict-deciding reads went to the clone the FACTORY named, so the obliging contract's
    // attacker-chosen answers are not what produced this result.
    assert_eq!(v["issuerResolution"]["source"], "operatorOverride", "{v}");
    // The override never got to ANSWER: both verdict-deciding reads went to the clone the factory
    // named, so the pillar reports the real signer and passes honestly.
    assert_eq!(v["fragments"]["issuerWhitelisted"], true, "{v}");
    assert_eq!(v["fragments"]["onchain"], true, "{v}");
    // But an override that DISAGREES with the factory is a failed assertion, and tightens: the caller
    // asserted this credential came from a contract the chain says did not issue it.
    assert_eq!(v["verdict"], false, "{v}");

    // (b) fabricated root + the same override: nothing in the factory index, so nothing resolves.
    let mut fabricated = issued["wrappedDoc"].clone();
    fabricated["signature"]["merkleRoot"] = json!(format!("0x{}", "cd".repeat(32)));
    let (_, v2) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": fabricated, "issuer_addr": HOSTILE }),
    )
    .await;
    // THE point: the pillar refuses to resolve. An override cannot stand in for a clone the factory
    // never recorded, so the answer is INDETERMINATE - never a pass, and never borrowed from whatever
    // contract the caller nominated.
    assert!(
        v2["fragments"]["issuerWhitelisted"].is_null(),
        "an override must not stand in for a clone that never existed: {v2}"
    );
    // With no factory record there is no resolved clone, so the issuance read falls back to the
    // caller-nominated address and the obliging contract answers `true`. That is reported honestly
    // rather than hidden - `rootIssuerRead: noRecord` and `issuerProvenance: unknown` say the
    // provenance was never established, so nothing here claims that contract was vouched for.
    assert_eq!(v2["issuerResolution"]["rootIssuerRead"], "noRecord", "{v2}");
    assert_eq!(v2["fragments"]["issuerProvenance"], "unknown", "{v2}");
    // And the credential is refused regardless: a fabricated root cannot survive integrity.
    assert_eq!(v2["fragments"]["integrity"], false, "{v2}");
    assert_eq!(v2["verdict"], false, "{v2}");

    // An override that AGREES with the factory leaves a genuine credential passing.
    let (_, ok) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": issued["wrappedDoc"].clone(), "issuer_addr": ISSUER_ADDR }),
    )
    .await;
    assert_eq!(ok["fragments"]["issuerWhitelisted"], true, "{ok}");
    assert_eq!(ok["verdict"], true, "{ok}");
}

/// Relabelling the RECORD TYPE is inside this pillar's reach, unlike `name`/`domain`.
///
/// `issuer.recordType` picks which whitelist question gets asked, and it sits in the same
/// root-uncovered block as `documentStore`. An authority whitelisted for two record types - the demo
/// government authority is whitelisted for both - could otherwise have a credential relabelled from
/// one to the other and still pass, with the verify response echoing the forged type back. So the key
/// is taken from the RESOLVED clone's own immutable `recordType()`, and a document claiming a
/// different one is a definite failure rather than a differently-phrased question.
#[tokio::test]
async fn a_record_type_relabel_is_refused_by_the_clones_own_record_type() {
    let (state, chain) = demo_state();
    // The signer is whitelisted for BOTH types, so the forged label names a capability this authority
    // genuinely holds - the mismatch is what refuses it, not a missing grant.
    let signer = chain.signer_address().unwrap();
    chain.set_issuance_capability(REGISTRY_ADDR, ISSUER_ADDR, &signer, true);

    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "15", "fields": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");

    let mut relabelled = issued["wrappedDoc"].clone();
    relabelled["issuer"]["recordType"] = json!(government_api::app::EU_HEALTH_CERT);
    let (_, v) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": relabelled }),
    )
    .await;

    assert_eq!(v["fragments"]["integrity"], true, "data untouched: {v}");
    assert_eq!(v["fragments"]["issuerWhitelisted"], false, "{v}");
    assert_eq!(
        v["verdict"], false,
        "cross-type relabel must not verify: {v}"
    );
}

/// A signer that DID issue the root but whose grant the governing registry never recorded is a
/// resolved `false`, not an indeterminate — and equally must not pass.
///
/// The parenthetical this comment used to carry ("the authority is the registry NOW, not at mint
/// time") was the defect stated as a rule. It is the opposite of the standing ruling: delisting is
/// forward-only (`DogTagIssuer.sol:82`), and `adminRevoke` exists because `delistFor` does not reach
/// backwards. What still fails here is the genuinely unauthorised case — the registry answered, and
/// its own log holds no grant at or before the anchoring.
#[tokio::test]
async fn a_resolved_but_unwhitelisted_issuer_fails_the_verdict() {
    let cfg = Config {
        deployment_url: "http://localhost:44832".into(),
        rpc_url: "https://devrpc.roax.net".into(),
        chain_id: 135,
        issuer_registry_addr: REGISTRY_ADDR.into(),
        factory_addr: FACTORY.into(),
        verification_registry_addr: "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87".into(),
        travel_clearance_issuer_addr: ISSUER_ADDR.into(),
        eu_health_cert_issuer_addr: "0x0000000000000000000000000000000000000000".into(),
        issuer_name: "DogTag Government Authority".into(),
        issuer_domain: "gov.example".into(),
        issuer_domain_registry_addr: "0x00000000000000000000000000000000000000dd".into(),
        dns_doh_endpoint: String::new(),
        demo: true,
        api_token: Some(API_TOKEN.into()),
    };
    // Identical to `demo_state()` except the signer is NEVER whitelisted for TRAVEL_CLEARANCE. The
    // clone still declares its record type AND names its governing registry, so the pillar resolves
    // all the way to that registry's grant log and the `false` below is the log answering with
    // nothing, not a read that could not be made. Without `with_registry` the fake would have no
    // authority to name — a real clone always does — and the answer would degrade to `unresolved`,
    // which is a different claim and not the one this test is about.
    let chain = MemChain::new().with_factory(FACTORY).with_registry(REGISTRY_ADDR);
    chain.set_record_type(
        ISSUER_ADDR,
        &government_api::app::record_type_key(TRAVEL_CLEARANCE),
    );
    let state = AppState {
        store: Arc::new(MemStore::new()) as Arc<dyn Store>,
        chain: Arc::new(chain),
        cfg: Arc::new(cfg),
        feed: Arc::new(government_api::oversight::DisabledFeed),
        dns: std::sync::Arc::new(dogtag_dns_rs::BindingResolver::production(String::new())),
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

/// DELISTING IS FORWARD-ONLY, at the government verify route. Both directions.
///
/// The authority's own verifier is the surface where this matters most: it is what a border officer
/// reads, and an ordinary key rotation at the issuing practice must not turn every travel document
/// that practice ever issued into a forgery. `DogTagIssuer.sol:82` states the rule in the contract's
/// own source and `adminRevoke` is the retroactive lever, which was not used on this root.
///
/// Mutation: point this handler's arm back at `is_whitelisted_for` -> the AFTER half goes red.
#[tokio::test]
async fn a_signer_delisted_after_issuance_still_verifies_and_before_issuance_does_not() {
    use government_api::chain::{GrantEvent, LogPoint};
    let (state, chain) = demo_state();
    let rt = government_api::app::record_type_key(TRAVEL_CLEARANCE);
    let signer = chain.signer_address().expect("demo signer");

    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "77", "fields": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");
    let doc = issued["wrappedDoc"].clone();
    let root = doc["signature"]["merkleRoot"].as_str().unwrap().to_string();

    let verify = |doc: Value| {
        let state = state.clone();
        async move { call(&state, "POST", "/v1/verify", json!({ "wrapped_doc": doc })).await }
    };

    // Control: authorised, and it verifies.
    let (_, v) = verify(doc.clone()).await;
    assert_eq!(v["verdict"], true, "control: {v}");
    assert_eq!(v["fragments"]["issuerWhitelistState"], "passed", "{v}");

    // (a) DELISTED AFTER the anchoring — a key rotation at the issuing authority.
    chain.delist(REGISTRY_ADDR, &rt, &signer);
    let (_, v) = verify(doc.clone()).await;
    assert_eq!(
        v["verdict"], true,
        "delisting is forward-only, so a genuine travel document must still verify: {v}"
    );
    assert_eq!(v["fragments"]["issuerWhitelistState"], "passed", "{v}");

    // (b) DELISTED BEFORE it. `issue()` is `onlyWhitelisted`, so this anchoring cannot have happened
    // as the document describes. Seeded rather than driven: the honest path cannot reach this state.
    let anchored = chain
        .root_issued_at(ISSUER_ADDR, &root)
        .expect("anchoring point");
    chain.set_grant_history(
        REGISTRY_ADDR,
        ISSUER_ADDR,
        &signer,
        vec![
            GrantEvent {
                at: LogPoint {
                    block_number: anchored.block_number - 2,
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
    let (_, v) = verify(doc).await;
    assert_eq!(v["verdict"], false, "{v}");
    assert_eq!(v["fragments"]["issuerWhitelistState"], "failed", "{v}");
}


/// THE GUARD MUST NOT SOFTEN GENERATION 1. An `IssuerRegistry` whose own log records no grant for the
/// pair is a definite refusal and stays one — an honest `issue()` cannot pass `onlyWhitelisted` in
/// that state, so the emptiness is evidence about the credential.
///
/// Without this, the guard could be written to return `Undetermined` for EVERY empty history and the
/// test above would still pass while every never-granted signer quietly stopped being refused.
#[tokio::test]
async fn an_empty_history_is_a_definite_refusal() {
    let (state, chain) = demo_state();
    let rt = government_api::app::record_type_key(TRAVEL_CLEARANCE);
    let signer = chain.signer_address().expect("demo signer");

    let (status, issued) = call_auth(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "79", "fields": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");
    let doc = issued["wrappedDoc"].clone();

    chain.set_grant_history(REGISTRY_ADDR, ISSUER_ADDR, &signer, vec![]);

    let (_, v) = call(&state, "POST", "/v1/verify", json!({ "wrapped_doc": doc })).await;
    assert_eq!(
        v["fragments"]["issuerWhitelistState"], "failed",
        "an empty generation-1 log is evidence about the credential, not about us: {v}"
    );
    assert_eq!(v["fragments"]["issuerWhitelisted"], false, "{v}");
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
    assert_eq!(base, DEPLOYMENT_URL, "== DEPLOYMENT_URL: {doc}");
    assert!(
        !base.ends_with('/'),
        "no trailing slash → no `//r/`: {base}"
    );

    // The identity field is untouched and is NOT the QR base. Corrupting a root-covered did:web with a
    // rotating deployment hostname was the tempting non-fix; this asserts we did not take it.
    assert_eq!(doc["issuer"]["domain"], "gov.example");
    assert_ne!(doc["issuer"]["domain"], json!(base));

    // What a renderer builds now resolves on THIS deployment.
    let receipt_id = issued["receiptId"].as_str().expect("receiptId");
    let (status, _) = call(&state, "GET", &format!("/r/{receipt_id}"), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "the stamped base serves /r/:id");
}
