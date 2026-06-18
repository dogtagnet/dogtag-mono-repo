//! Full HTTP flow against the in-memory `MemChain` stub (no external services). Exercises every
//! endpoint and the spec's negative assertions. Always runs in CI.

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;
use vet_api::chain::{record_type_key, ChainClient, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_issuance_share_revoke_flow() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let state = state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);

    // --- custody: genesis -> confirm -> unlock ---
    let (_admin, op, backend_addr) = boot_custody(&app).await;

    // admin whitelists the backend signer for VACCINATION on-chain (emulated).
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend_addr);

    // --- settings: backend mode (default), confirm GET ---
    let (s, b) = call(&app, "GET", "/settings/signing-mode", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["signingMode"], "backend");

    // --- prepare (backend mode): builds, broadcasts, confirms (on-chain re-verify) ---
    let (s, b) = call(
        &app,
        "POST",
        "/credentials/prepare",
        Some(&op),
        Some(serde_json::json!({
            "recordType": "VACCINATION",
            "dogTagId": "42",
            "fields": vaccination_fields()
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "prepare: {b}");
    assert_eq!(b["mode"], "backend");
    let record_id = b["recordId"].as_str().unwrap().to_string();
    let _root = b["merkleRoot"].as_str().unwrap().to_string();
    assert!(b["txHash"].as_str().is_some());

    // record is now ISSUED on-chain — isValid(root) is true via the chain client (issuance pillar).
    // --- share: mint a one-time record-JWT ---
    let (s, b) = call(&app, "POST", &format!("/records/{record_id}/share"), Some(&op), Some(serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::OK, "share: {b}");
    let qr = b["qrUrl"].as_str().unwrap();
    let token = extract_token(qr);

    // --- GET /records/{id} with the JWT: returns the wrapped doc; issuance verifies VALID ---
    let (s, doc) = call(&app, "GET", &format!("/records/{record_id}"), Some(&token), None).await;
    assert_eq!(s, StatusCode::OK, "get record: {doc}");
    assert_eq!(doc["version"], "dogtag/1.0");
    let merkle_root = doc["signature"]["merkleRoot"].as_str().unwrap().to_string();

    // third-party verify of the returned doc: issuance pillar TRUE (root is issued on chain).
    assert!(
        mem.is_valid(ISSUER, &merkle_root).await.unwrap(),
        "issuance pillar: root must be valid on-chain after issue"
    );

    // --- reused share-JWT => 401 (one-time jti) ---
    let (s, _b) = call(&app, "GET", &format!("/records/{record_id}"), Some(&token), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "reused share-JWT must be 401");

    // --- revoke: re-verify issuance INVALID ---
    let (s, b) = call(&app, "POST", &format!("/records/{record_id}/revoke"), Some(&op), Some(serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::OK, "revoke: {b}");
    assert!(
        !mem.is_valid(ISSUER, &merkle_root).await.unwrap(),
        "after revoke, issuance pillar must be INVALID"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_whitelisted_signer_fails_preflight() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let state = state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let (_admin, op, _backend_addr) = boot_custody(&app).await;

    // NO whitelist seeded -> backend-mode prepare must fail the preflight (403).
    let (s, b) = call(
        &app,
        "POST",
        "/credentials/prepare",
        Some(&op),
        Some(serde_json::json!({"recordType":"VACCINATION","dogTagId":"7","fields":vaccination_fields()})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "non-whitelisted signer must 403: {b}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirm_refuses_bogus_txhash() {
    // Confirm REFUSES to mark issued if the on-chain RootIssued/issuedAt check fails.
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let state = state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let (_admin, op, backend_addr) = boot_custody(&app).await;
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend_addr);

    // switch to WALLET mode so prepare returns an unsigned tx WITHOUT confirming (leaves it prepared).
    let (s, _b) = call(&app, "PUT", "/settings/signing-mode", Some(&op), Some(serde_json::json!({"mode":"wallet"}))).await;
    assert_eq!(s, StatusCode::OK);

    let (s, b) = call(
        &app,
        "POST",
        "/credentials/prepare",
        Some(&op),
        Some(serde_json::json!({"recordType":"VACCINATION","dogTagId":"9","fields":vaccination_fields()})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "wallet prepare: {b}");
    let record_id = b["recordId"].as_str().unwrap().to_string();
    assert!(b["unsignedTx"].is_object());

    // confirm with a bogus txHash -> must NOT mark issued.
    let (s, b) = call(
        &app,
        "POST",
        "/credentials/confirm",
        Some(&op),
        Some(serde_json::json!({"recordId": record_id, "txHash": "0xdeadbeef00000000000000000000000000000000000000000000000000000000"})),
    )
    .await;
    assert_ne!(s, StatusCode::OK, "bogus txHash must NOT confirm: {b}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_gates_and_settings_409() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let state = state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);

    // issuance routes require operator session.
    let (s, _b) = call(&app, "GET", "/settings/signing-mode", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "no session -> 401");

    // custody routes require admin session (operator token is NOT admin).
    let (_admin, op, backend_addr) = boot_custody(&app).await;
    let (s, _b) = call(&app, "POST", "/admin/accounts", Some(&op), Some(serde_json::json!({"label":"x"}))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "operator token must not pass admin gate");

    // settings 409 when a prepared record is outstanding.
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend_addr);
    let (s, _b) = call(&app, "PUT", "/settings/signing-mode", Some(&op), Some(serde_json::json!({"mode":"wallet"}))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _b) = call(
        &app,
        "POST",
        "/credentials/prepare",
        Some(&op),
        Some(serde_json::json!({"recordType":"VACCINATION","dogTagId":"1","fields":vaccination_fields()})),
    )
    .await;
    assert_eq!(s, StatusCode::OK); // wallet mode leaves a prepared record
    let (s, b) = call(&app, "PUT", "/settings/signing-mode", Some(&op), Some(serde_json::json!({"mode":"backend"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "prepared outstanding -> 409: {b}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_session_and_zk_consent_stub() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let state = state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let (_admin, op, backend_addr) = boot_custody(&app).await;

    // relayer must be whitelisted for keccak256("VERIFY:"||purpose).
    let purpose = "boarding-checkin";
    let vk = {
        use vet_api::verify::verify_key;
        verify_key(purpose)
    };
    mem.whitelist(REGISTRY, &vk, &backend_addr);

    let (s, b) = call(
        &app,
        "POST",
        "/verify/session/start",
        Some(&op),
        Some(serde_json::json!({"purpose": purpose, "recordType":"VACCINATION", "mode":"zk"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    let session_id = b["sessionId"].as_str().unwrap().to_string();

    // ZK consent submit (stub prover): records the attestation.
    let rt = record_type_key("VACCINATION");
    let consent = serde_json::json!({
        "dogTagId": "42",
        "recordType": rt,
        "purpose": "0x0000000000000000000000000000000000000000000000000000000000000007",
        "credentialRoot": "0x1111111111111111111111111111111111111111111111111111111111111111",
        "challenge": "0x00",
        "relayer": backend_addr,
        "subject": "0x00000000000000000000000000000000000000cc",
        "nonce": "1",
        "deadline": (common_now() + 300)
    });
    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/submit",
        Some(&op),
        Some(serde_json::json!({"sessionId": session_id, "consent": consent, "sig": "0xstub", "mode":"zk"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "zk consent submit: {b}");
    assert_eq!(b["recorded"], true);
    assert_eq!(b["mode"], "zk");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_session_status_polls_pending_to_recorded() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let state = state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);
    let (_admin, op, backend_addr) = boot_custody(&app).await;

    let purpose = "boarding-checkin";
    let vk = {
        use vet_api::verify::verify_key;
        verify_key(purpose)
    };
    mem.whitelist(REGISTRY, &vk, &backend_addr);

    // start a session.
    let (s, b) = call(
        &app,
        "POST",
        "/verify/session/start",
        Some(&op),
        Some(serde_json::json!({"purpose": purpose, "recordType":"VACCINATION", "mode":"zk"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    let session_id = b["sessionId"].as_str().unwrap().to_string();

    // status read is operator-gated: no session -> 401.
    let (s, _b) = call(&app, "GET", &format!("/verify/session/{session_id}"), None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "status read must be operator-gated");

    // unknown session -> 404.
    let (s, _b) = call(&app, "GET", "/verify/session/does-not-exist", Some(&op), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "unknown session -> 404");

    // before submit: status pending, no txHash.
    let (s, b) = call(&app, "GET", &format!("/verify/session/{session_id}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "status pending: {b}");
    assert_eq!(b["status"], "pending");
    assert_eq!(b["mode"], "zk");
    assert!(b["txHash"].is_null(), "no txHash while pending");

    // submit the ZK consent (MemChain) -> records the attestation.
    let rt = record_type_key("VACCINATION");
    let consent = serde_json::json!({
        "dogTagId": "42",
        "recordType": rt,
        "purpose": "0x0000000000000000000000000000000000000000000000000000000000000007",
        "credentialRoot": "0x1111111111111111111111111111111111111111111111111111111111111111",
        "challenge": "0x00",
        "relayer": backend_addr,
        "subject": "0x00000000000000000000000000000000000000cc",
        "nonce": "1",
        "nullifier": "0x2222222222222222222222222222222222222222222222222222222222222222",
        "deadline": (common_now() + 300)
    });
    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/submit",
        Some(&op),
        Some(serde_json::json!({"sessionId": session_id, "consent": consent, "sig": "0xstub", "mode":"zk"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "consent submit: {b}");
    let submit_tx = b["txHash"].as_str().unwrap().to_string();

    // after submit: status recorded + txHash + nullifier surfaced.
    let (s, b) = call(&app, "GET", &format!("/verify/session/{session_id}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "status recorded: {b}");
    assert_eq!(b["status"], "recorded");
    assert_eq!(b["txHash"].as_str().unwrap(), submit_tx, "txHash persisted on the session");
    assert_eq!(
        b["nullifier"],
        "0x2222222222222222222222222222222222222222222222222222222222222222",
        "nullifier surfaced",
    );
}

fn common_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

fn extract_token(qr: &str) -> String {
    // qrUrl: .../r?t=<jwt>&i=<id>
    let after = qr.split("t=").nth(1).unwrap();
    after.split("&i=").next().unwrap().to_string()
}
