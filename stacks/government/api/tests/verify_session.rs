//! Owner-hidden verify sessions — the government as a VERIFIER, driven from a QR the owner scans.
//!
//! The authority gets no shortcut: it goes through the same ZK consent path as a groomer, gated by the
//! same `VERIFY:<purpose>` whitelist, submitting to the same registry. These tests pin the parts the
//! phone depends on (route paths, the mandatory `unverifiedClaims` block, the dual auth gate) and the
//! preflight that must refuse BEFORE the owner spends tens of seconds proving.

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
const CONSENT_REGISTRY: &str = "0xaBFd6f6E31780EBcB7ABd28A2a9bCfc9C8e6A77B";
const API_TOKEN: &str = "dogtag-gov-demo-token";
const DEPLOYMENT_URL: &str = "http://192.168.1.20:44832";
const PURPOSE: &str = "travel_check";
const RECORD_TYPE: &str = "VACCINATION";

/// BN254 r — the field modulus every public signal must be below.
const BN254_R_DEC: &str =
    "21888242871839275222246405745257275088548364400416034343698204186575808495617";

fn cfg() -> Config {
    Config {
        deployment_url: DEPLOYMENT_URL.into(),
        rpc_url: "https://devrpc.roax.net".into(),
        chain_id: 135,
        issuer_registry_addr: REGISTRY_ADDR.into(),
        factory_addr: "0x00000000000000000000000000000000000000fa".into(),
        issuer_domain_registry_addr: "0x00000000000000000000000000000000000000dd".into(),
        dns_doh_endpoint: String::new(),
        verification_registry_addr: CONSENT_REGISTRY.into(),
        travel_clearance_issuer_addr: ISSUER_ADDR.into(),
        eu_health_cert_issuer_addr: "0x0000000000000000000000000000000000000000".into(),
        issuer_name: "DogTag Government Authority".into(),
        issuer_domain: "gov.example".into(),
        demo: true,
        api_token: Some(API_TOKEN.into()),
    }
}

/// A state whose signer is (or is not) whitelisted for `VERIFY:travel_check`.
fn state_with(whitelisted: bool) -> (AppState, String) {
    let chain = MemChain::new();
    let signer = chain.signer_address().unwrap();
    let c = cfg();
    if whitelisted {
        chain.whitelist(
            REGISTRY_ADDR,
            &government_api::verify::verify_key(PURPOSE),
            &signer,
        );
    }
    let state = AppState {
        store: Arc::new(MemStore::new()) as Arc<dyn Store>,
        chain: Arc::new(chain),
        cfg: Arc::new(c),
        dns: Arc::new(dogtag_dns_rs::BindingResolver::production(String::new())),
        feed: Arc::new(government_api::oversight::DisabledFeed),
    };
    (state, signer)
}

async fn call(
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
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn start(state: &AppState) -> Value {
    let (status, v) = call(
        state,
        "POST",
        "/verify/session/start",
        json!({ "purpose": PURPOSE, "recordType": RECORD_TYPE }),
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "session start: {v}");
    v
}

/// `.../x/<token>?a=<relayer>` → the token.
fn token_of(qr_url: &str) -> String {
    qr_url
        .rsplit('/')
        .next()
        .unwrap()
        .split('?')
        .next()
        .unwrap()
        .to_string()
}

/// A syntactically well-formed consent proof. MemChain cannot verify Groth16, so these tests exercise
/// the PREFLIGHT (the part this stack owns); the ZK soundness of the path is the registry's, pinned in
/// `contracts/test`.
fn proof(relayer: &str, purpose_word: &str, record_type_word: &str, deadline: u64) -> Value {
    let addr = format!("0x{}", relayer.trim_start_matches("0x"));
    json!({
        "a": ["1", "2"],
        "b": [["3", "4"], ["5", "6"]],
        "c": ["7", "8"],
        // frozen output order: [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
        "pubSignals": [
            "12345",
            purpose_word,
            addr,
            "99887766",
            "5566",
            record_type_word,
            deadline.to_string(),
        ],
    })
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test]
async fn start_refuses_honestly_when_the_signer_is_not_whitelisted_for_the_purpose() {
    let (state, _) = state_with(false);
    let (status, v) = call(
        &state,
        "POST",
        "/verify/session/start",
        json!({ "purpose": PURPOSE, "recordType": RECORD_TYPE }),
        Some(API_TOKEN),
    )
    .await;
    // The same honest failure the groomer portal surfaces — refused BEFORE any QR is shown, so the
    // owner never spends time proving into a dead end.
    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"], json!("relayer not whitelisted for this purpose"));
}

#[tokio::test]
async fn start_requires_the_operator_bearer() {
    let (state, _) = state_with(true);
    let (status, _) = call(
        &state,
        "POST",
        "/verify/session/start",
        json!({ "purpose": PURPOSE, "recordType": RECORD_TYPE }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn start_mints_the_groomer_shaped_export_qr() {
    let (state, signer) = state_with(true);
    let v = start(&state).await;
    let qr = v["qrUrl"].as_str().unwrap();
    // `<DEPLOYMENT_URL>/x/<token>?a=<relayer>` — the exact shape both phone apps parse.
    assert!(
        qr.starts_with(&format!("{DEPLOYMENT_URL}/x/")),
        "qrUrl must be <DEPLOYMENT_URL>/x/<token>: {qr}"
    );
    assert!(qr.ends_with(&format!("?a={signer}")), "qr must carry the relayer: {qr}");
    let token = token_of(qr);
    assert_eq!(token.len(), 32, "token must be 32 hex chars: {token}");
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(!v["sessionId"].as_str().unwrap().is_empty());
    // The export TTL is longer than a share token's: on-device Groth16 proving takes tens of seconds.
    assert_eq!(v["ttlSecs"], json!(600));
}

#[tokio::test]
async fn export_token_resolves_the_session_with_the_mandatory_discovery_claims() {
    let (state, signer) = state_with(true);
    let started = start(&state).await;
    let token = token_of(started["qrUrl"].as_str().unwrap());

    // Unauthenticated — the token IS the capability.
    let (status, v) = call(&state, "GET", &format!("/x/{token}"), Value::Null, None).await;
    assert_eq!(status, StatusCode::OK, "resolve: {v}");
    assert_eq!(v["sessionId"], started["sessionId"]);
    assert_eq!(v["relayer"], json!(signer));
    assert_eq!(v["purpose"], json!(PURPOSE));
    assert_eq!(v["recordType"], json!(RECORD_TYPE));
    assert!(v["challenge"].as_str().unwrap().starts_with("0x"));

    // `unverifiedClaims` is MANDATORY: both phone apps refuse the owner-hidden flow outright when the
    // block is missing ("session is missing its discovery claims"). The registry claim in particular is
    // what the app checks against the on-chain ProtocolRegistry anchor.
    let claims = &v["unverifiedClaims"];
    assert_eq!(claims["verificationRegistry"], json!(CONSENT_REGISTRY));
    // `chainId` is sourced from the CHAIN CLIENT, not `cfg.chain_id` (mirroring vet-api), so a
    // simulated backend claims `SIMULATED_CHAIN_ID` rather than ROAX's 135 even though the config
    // names 135. That honesty is load-bearing here, not incidental: the phone validates this claim
    // against the on-chain ProtocolRegistry anchor, so a simulated stack must fail that check closed
    // rather than steer an owner into proving against a chain it cannot actually reach.
    assert_eq!(
        claims["chainId"],
        json!(government_api::chain::SIMULATED_CHAIN_ID)
    );
    assert_eq!(claims["purpose"], json!(PURPOSE));
    assert_eq!(claims["protocolVersion"], json!(dogtag_standard::wrap::LEVEL_B_VERSION));

    // NON-consuming: the phone re-reads it while proving and polling.
    let (status, again) = call(&state, "GET", &format!("/x/{token}"), Value::Null, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["sessionId"], started["sessionId"]);

    // An unknown token is a 404, not a blank session.
    let (status, _) = call(&state, "GET", "/x/deadbeef", Value::Null, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn session_status_is_gated_by_the_operator_bearer_or_the_sessions_own_export_token() {
    let (state, _) = state_with(true);
    let started = start(&state).await;
    let session_id = started["sessionId"].as_str().unwrap();
    let token = token_of(started["qrUrl"].as_str().unwrap());

    // Operator bearer (the portal's poll).
    let (status, v) = call(
        &state,
        "GET",
        &format!("/verify/session/{session_id}"),
        Value::Null,
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["status"], json!("pending"));

    // The owner's phone, with `?token=` — the path/query shape both apps hard-code.
    let (status, v) = call(
        &state,
        "GET",
        &format!("/verify/session/{session_id}?token={token}"),
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["status"], json!("pending"));

    // No credential at all → 401.
    let (status, _) = call(
        &state,
        "GET",
        &format!("/verify/session/{session_id}"),
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A VALID token for a DIFFERENT session must not read this one.
    let other = start(&state).await;
    let other_token = token_of(other["qrUrl"].as_str().unwrap());
    let (status, v) = call(
        &state,
        "GET",
        &format!("/verify/session/{session_id}?token={other_token}"),
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(v["error"], json!("export token does not match session"));
}

#[tokio::test]
async fn consent_submit_records_and_the_session_reaches_recorded() {
    let (state, signer) = state_with(true);
    let started = start(&state).await;
    let session_id = started["sessionId"].as_str().unwrap().to_string();
    let token = token_of(started["qrUrl"].as_str().unwrap());

    let (status, v) = call(
        &state,
        "POST",
        "/v1/verify/consent",
        json!({
            "exportToken": token,
            "proof": proof(
                &signer,
                &government_api::verify::purpose_key(PURPOSE),
                &government_api::verify::purpose_key(RECORD_TYPE),
                now() + 600,
            ),
        }),
        None, // authed by the export token alone — the phone holds no operator bearer
    )
    .await;
    assert_eq!(status, StatusCode::OK, "submit: {v}");
    assert_eq!(v["status"], json!("recording"));
    assert_eq!(v["sessionId"], json!(session_id));
    assert_eq!(v["registry"], json!(CONSENT_REGISTRY));
    assert!(v["nullifier"].as_str().unwrap().starts_with("0x"));
    // Shape parity with vet-api even when nothing was disclosed.
    assert_eq!(v["disclosedKeyPaths"], json!([]));

    // Broadcast is detached; the phone polls until the session settles.
    let mut settled = Value::Null;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let (_, s) = call(
            &state,
            "GET",
            &format!("/verify/session/{session_id}?token={token}"),
            Value::Null,
            None,
        )
        .await;
        if s["status"] != json!("recording") {
            settled = s;
            break;
        }
    }
    assert_eq!(settled["status"], json!("recorded"), "settled: {settled}");
    assert!(settled["txHash"].as_str().unwrap().starts_with("0x"));

    // The operator-facing history shows the recorded verification (metadata only — no owner, no leaf).
    let (status, h) = call(&state, "GET", "/verify/history", Value::Null, Some(API_TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{h}");
    let rows = h["verifications"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|r| r["sessionId"] == json!(session_id))
        .expect("session in history");
    assert_eq!(row["status"], json!("recorded"));
    assert_eq!(row["purpose"], json!(PURPOSE));
    assert!(row["explorerUrl"].as_str().unwrap().contains("/tx/0x"));
}

#[tokio::test]
async fn a_settled_session_refuses_a_second_proof() {
    let (state, signer) = state_with(true);
    let started = start(&state).await;
    let token = token_of(started["qrUrl"].as_str().unwrap());
    let body = json!({
        "exportToken": token,
        "proof": proof(
            &signer,
            &government_api::verify::purpose_key(PURPOSE),
            &government_api::verify::purpose_key(RECORD_TYPE),
            now() + 600,
        ),
    });
    let (status, _) = call(&state, "POST", "/v1/verify/consent", body.clone(), None).await;
    assert_eq!(status, StatusCode::OK);

    // The session status is the replay guard — the export token itself is deliberately not consumed.
    let (status, v) = call(&state, "POST", "/v1/verify/consent", body, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], json!("session not pending"));
}

#[tokio::test]
async fn consent_preflight_refuses_before_spending_gas() {
    let (state, signer) = state_with(true);
    let purpose_word = government_api::verify::purpose_key(PURPOSE);
    let record_type_word = government_api::verify::purpose_key(RECORD_TYPE);

    // Each case starts its own session: a rejected submit must leave the session usable, and these
    // assertions should not depend on each other's ordering.
    let submit = |body: Value| {
        let state = state.clone();
        async move {
            let started = start(&state).await;
            let token = token_of(started["qrUrl"].as_str().unwrap());
            let mut b = body;
            b["exportToken"] = json!(token);
            call(&state, "POST", "/v1/verify/consent", b, None).await
        }
    };

    // A proof bound to a DIFFERENT relayer is refused (the registry also requires relayer==msg.sender).
    let (status, v) = submit(json!({
        "proof": proof(
            "0x00000000000000000000000000000000000baddd",
            &purpose_word,
            &record_type_word,
            now() + 600,
        )
    }))
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"], json!("pubSignals[relayer] does not name this relayer"));

    // A proof for a different purpose does not satisfy this session's binding.
    let (status, v) = submit(json!({
        "proof": proof(
            &signer,
            &government_api::verify::purpose_key("grooming_intake"),
            &record_type_word,
            now() + 600,
        )
    }))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(
        v["error"],
        json!("pubSignals.purpose != purpose_key(session.purpose)")
    );

    // A deadline inside the 120s broadcast margin is refused up front rather than reverting on-chain.
    let (status, v) = submit(json!({
        "proof": proof(&signer, &purpose_word, &record_type_word, now() + 30)
    }))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("deadline"));

    // An out-of-field public signal is refused (snarkjs #358 — a soundness hazard, not a format nit).
    let mut bad = proof(&signer, &purpose_word, &record_type_word, now() + 600);
    bad["pubSignals"][0] = json!(BN254_R_DEC);
    let (status, v) = submit(json!({ "proof": bad })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["error"], json!("pubSignals[0]: !field"));

    // Art. 9 special-category data is never verifiable on-chain. Submitted COLD (operator bearer, no
    // session): a session-bound submit would be refused a step earlier by the recordType binding, so the
    // art9 rule is only reachable — and only meaningful — on the unbound path.
    let (status, v) = call(
        &state,
        "POST",
        "/v1/verify/consent",
        json!({
            "proof": proof(
                &signer,
                &purpose_word,
                &government_api::verify::purpose_key("SERVICE_ATTESTATION"),
                now() + 600,
            )
        }),
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("art9"), "{v}");

    // ...and a session-bound submit rejects the same proof at the recordType binding instead.
    let (status, v) = submit(json!({
        "proof": proof(
            &signer,
            &purpose_word,
            &government_api::verify::purpose_key("SERVICE_ATTESTATION"),
            now() + 600,
        )
    }))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(
        v["error"],
        json!("pubSignals.recordType != purpose_key(session.recordType)")
    );

    // A missing proof is a 400, never a 500.
    let (status, v) = submit(json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["error"], json!("proof: missing"));
}

/// The `VERIFY:` whitelist is enforced on the PROOF's own purpose word, not the session's — so a cold
/// operator submit for an unwhitelisted purpose cannot slip past the session-start check.
#[tokio::test]
async fn a_cold_operator_submit_still_faces_the_verify_whitelist() {
    let (state, signer) = state_with(false);
    let (status, v) = call(
        &state,
        "POST",
        "/v1/verify/consent",
        json!({
            "proof": proof(
                &signer,
                &government_api::verify::purpose_key(PURPOSE),
                &government_api::verify::purpose_key(RECORD_TYPE),
                now() + 600,
            )
        }),
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"], json!("relayer not whitelisted for this purpose"));
}

/// The existing paste-a-wrappedDoc verify path is a FALLBACK that must survive alongside the QR flow.
#[tokio::test]
async fn the_paste_json_verify_path_is_untouched() {
    let (state, _) = state_with(true);
    // Issue through the normal path, then verify the returned doc by pasting it (no QR involved).
    let (status, issued) = call(
        &state,
        "POST",
        "/v1/travel-clearance/issue",
        json!({ "record_type": TRAVEL_CLEARANCE, "dog_tag_id": "7", "fields": {} }),
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issue: {issued}");
    let (status, verdict) = call(
        &state,
        "POST",
        "/v1/verify",
        json!({ "wrapped_doc": issued["wrappedDoc"] }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "verify: {verdict}");
    assert_eq!(verdict["verdict"], json!(true));
    assert_eq!(verdict["fragments"]["integrity"], json!(true));
}
