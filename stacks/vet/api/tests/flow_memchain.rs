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
    // --- share: mint a SHORT one-time share token (low-density QR) ---
    let (s, b) = call(&app, "POST", &format!("/records/{record_id}/share"), Some(&op), Some(serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::OK, "share: {b}");
    assert_eq!(b["recordId"].as_str().unwrap(), record_id, "share still returns recordId");
    let qr = b["qrUrl"].as_str().unwrap();
    // The QR is now a tiny `/r/<32hex>` path — NO embedded JWT, NO query string.
    assert!(!qr.contains("t="), "qrUrl must not carry a JWT query string: {qr}");
    let token = extract_token(qr);
    assert_eq!(token.len(), 32, "share token must be 32 hex chars (16 random bytes): {token}");
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()), "token must be hex: {token}");
    assert!(qr.ends_with(&format!("/r/{token}")), "qrUrl path must be /r/<token>: {qr}");

    // --- GET /r/<token>: returns the wrapped doc; issuance verifies VALID ---
    let (s, doc) = call(&app, "GET", &format!("/r/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "get shared: {doc}");
    assert_eq!(doc["version"], "dogtag/1.0");
    let merkle_root = doc["signature"]["merkleRoot"].as_str().unwrap().to_string();

    // third-party verify of the returned doc: issuance pillar TRUE (root is issued on chain).
    assert!(
        mem.is_valid(ISSUER, &merkle_root).await.unwrap(),
        "issuance pillar: root must be valid on-chain after issue"
    );

    // --- reused short token => 404 (one-time, deleted after first use) ---
    let (s, _b) = call(&app, "GET", &format!("/r/{token}"), None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "reused share token must be 404 (one-time)");

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

const CONSENT_KEYS: &str = "0x00000000000000000000000000000000000000cc";

/// Decimal field-element string for a 0x.. 20-byte address (uint160).
fn addr_field_dec(addr: &str) -> String {
    use alloy::primitives::U256;
    let b = hex::decode(addr.trim_start_matches("0x")).unwrap();
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(&b);
    U256::from_be_bytes(w).to_string()
}

/// The relayer-sponsored consent-key bind path on the client-supplied-proof ZK branch:
///   (a) not bound + no bind block -> 400; (b) bind block -> bindConsentKeyFor + record;
///   (c) already bound -> skip the bind tx and record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zk_client_proof_relayer_sponsored_bind() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let state = state_with_verify_keys(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        "0x0000000000000000000000000000000000000000".to_string(),
        CONSENT_KEYS.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
        Arc::new(vet_api::prover::StubProver),
    );
    let app = vet_api::router(state);
    let (_admin, op, backend_addr) = boot_custody(&app).await;

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

    let subject = "0x00000000000000000000000000000000000000dd";
    let rt = record_type_key("VACCINATION");
    let credential_root =
        "0x1111111111111111111111111111111111111111111111111111111111111111";
    let key_hash = "0x3333333333333333333333333333333333333333333333333333333333333333";
    // purpose as the registry's reduced bytes32 (purpose_key); pub[1] must equal it.
    let purpose_b32 = vet_api::verify::purpose_key(purpose);

    let consent = serde_json::json!({
        "dogTagId": "42",
        "recordType": rt,
        "purpose": purpose_b32,
        "credentialRoot": credential_root,
        "challenge": "0x00",
        "relayer": backend_addr,
        "subject": subject,
        "nonce": "1",
        "deadline": (common_now() + 300)
    });
    // pubSignals: [dogTagId, purpose, relayer, subject, nullifier, keyHash, credentialRoot].
    let proof = serde_json::json!({
        "a": ["1", "2"],
        "b": [["1", "2"], ["3", "4"]],
        "c": ["5", "6"],
        "pubSignals": [
            "42",
            purpose_b32,
            addr_field_dec(&backend_addr),
            addr_field_dec(subject),
            "0x4444444444444444444444444444444444444444444444444444444444444444",
            key_hash,
            credential_root
        ]
    });

    // (a) not bound + no bind block -> 400.
    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/submit",
        Some(&op),
        Some(serde_json::json!({
            "sessionId": session_id, "consent": consent, "sig": "0xstub",
            "mode":"zk", "proof": proof
        })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "unbound + no bind block must 400: {b}");
    assert!(
        b["error"].as_str().unwrap_or("").contains("consent key not bound"),
        "clear 400 message: {b}"
    );

    // (b) with a bind block -> bindConsentKeyFor + record. keyOf(subject) must end == keyHash.
    let bind = serde_json::json!({ "subject": subject, "keyHash": key_hash, "ownerSig": "0xowner" });
    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/submit",
        Some(&op),
        Some(serde_json::json!({
            "sessionId": session_id, "consent": consent, "sig": "0xstub",
            "mode":"zk", "proof": proof, "bind": bind
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "bind + record must succeed: {b}");
    assert_eq!(b["recorded"], true);
    assert_eq!(
        mem.consent_key_of(CONSENT_KEYS, subject).await.unwrap().to_lowercase(),
        key_hash.to_lowercase(),
        "keyOf(subject) bound to keyHash after submit"
    );

    // (c) already bound (set keyOf directly) -> a fresh session records WITHOUT a bind block.
    mem.set_consent_key(CONSENT_KEYS, subject, key_hash);
    let (s, b) = call(
        &app,
        "POST",
        "/verify/session/start",
        Some(&op),
        Some(serde_json::json!({"purpose": purpose, "recordType":"VACCINATION", "mode":"zk"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "session start 2: {b}");
    let session_id2 = b["sessionId"].as_str().unwrap().to_string();
    // a distinct nullifier so MemChain doesn't reject the replay.
    let mut proof2 = proof.clone();
    proof2["pubSignals"][4] =
        serde_json::json!("0x5555555555555555555555555555555555555555555555555555555555555555");
    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/submit",
        Some(&op),
        Some(serde_json::json!({
            "sessionId": session_id2, "consent": consent, "sig": "0xstub",
            "mode":"zk", "proof": proof2
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "already-bound submit must succeed without bind block: {b}");
    assert_eq!(b["recorded"], true);
}

fn common_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

fn extract_token(qr: &str) -> String {
    // qrUrl: .../r/<32hex>
    qr.rsplit('/').next().unwrap().to_string()
}
