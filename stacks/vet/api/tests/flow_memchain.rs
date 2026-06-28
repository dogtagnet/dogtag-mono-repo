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
    let (s, b) = call(
        &app,
        "POST",
        &format!("/records/{record_id}/share"),
        Some(&op),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "share: {b}");
    assert_eq!(
        b["recordId"].as_str().unwrap(),
        record_id,
        "share still returns recordId"
    );
    let qr = b["qrUrl"].as_str().unwrap();
    // The QR is now a tiny `/r/<32hex>` path — NO embedded JWT, NO query string.
    assert!(
        !qr.contains("t="),
        "qrUrl must not carry a JWT query string: {qr}"
    );
    let token = extract_token(qr);
    assert_eq!(
        token.len(),
        32,
        "share token must be 32 hex chars (16 random bytes): {token}"
    );
    assert!(
        token.chars().all(|c| c.is_ascii_hexdigit()),
        "token must be hex: {token}"
    );
    assert!(
        qr.ends_with(&format!("/r/{token}")),
        "qrUrl path must be /r/<token>: {qr}"
    );

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
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "reused share token must be 404 (one-time)"
    );

    // --- revoke: re-verify issuance INVALID ---
    let (s, b) = call(
        &app,
        "POST",
        &format!("/records/{record_id}/revoke"),
        Some(&op),
        Some(serde_json::json!({})),
    )
    .await;
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
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "non-whitelisted signer must 403: {b}"
    );
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
    let (s, _b) = call(
        &app,
        "PUT",
        "/settings/signing-mode",
        Some(&op),
        Some(serde_json::json!({"mode":"wallet"})),
    )
    .await;
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
    let (s, _b) = call(
        &app,
        "POST",
        "/admin/accounts",
        Some(&op),
        Some(serde_json::json!({"label":"x"})),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "operator token must not pass admin gate"
    );

    // settings 409 when a prepared record is outstanding.
    let rt = record_type_key("VACCINATION");
    mem.whitelist(REGISTRY, &rt, &backend_addr);
    let (s, _b) = call(
        &app,
        "PUT",
        "/settings/signing-mode",
        Some(&op),
        Some(serde_json::json!({"mode":"wallet"})),
    )
    .await;
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
    let (s, b) = call(
        &app,
        "PUT",
        "/settings/signing-mode",
        Some(&op),
        Some(serde_json::json!({"mode":"backend"})),
    )
    .await;
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
    let (s, _b) = call(
        &app,
        "GET",
        &format!("/verify/session/{session_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "status read must be operator-gated"
    );

    // unknown session -> 404.
    let (s, _b) = call(
        &app,
        "GET",
        "/verify/session/does-not-exist",
        Some(&op),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "unknown session -> 404");

    // before submit: status pending, no txHash.
    let (s, b) = call(
        &app,
        "GET",
        &format!("/verify/session/{session_id}"),
        Some(&op),
        None,
    )
    .await;
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
    let (s, b) = call(
        &app,
        "GET",
        &format!("/verify/session/{session_id}"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "status recorded: {b}");
    assert_eq!(b["status"], "recorded");
    assert_eq!(
        b["txHash"].as_str().unwrap(),
        submit_tx,
        "txHash persisted on the session"
    );
    assert_eq!(
        b["nullifier"], "0x2222222222222222222222222222222222222222222222222222222222222222",
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

    // The subject is the owner's secp256k1 wallet; it must SIGN the BindConsentKey EIP-712 digest so
    // the backend's defensive pre-check (recover ownerSig == subject) passes before the relayer
    // broadcasts bindConsentKeyFor. Derive `subject` from a real signer instead of a fixed string.
    use alloy::signers::local::PrivateKeySigner;
    let subject_signer = PrivateKeySigner::random();
    let subject = format!("{:#x}", subject_signer.address());
    let subject = subject.as_str();
    let rt = record_type_key("VACCINATION");
    let credential_root = "0x1111111111111111111111111111111111111111111111111111111111111111";
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
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "unbound + no bind block must 400: {b}"
    );
    assert!(
        b["error"]
            .as_str()
            .unwrap_or("")
            .contains("consent key not bound"),
        "clear 400 message: {b}"
    );

    // (b) with a bind block -> bindConsentKeyFor + record. keyOf(subject) must end == keyHash.
    // The owner signs the BindConsentKey EIP-712 digest the backend's pre-check recovers against:
    // verifyingContract = the ConsentKeyRegistry (CONSENT_KEYS), nonce = the live on-chain bindNonce
    // (0 for this still-unbound subject), chainId = DOGTAG_CHAIN_ID.
    let owner_sig = {
        use alloy::primitives::B256;
        use alloy::signers::SignerSync;
        let mut ckr = [0u8; 20];
        ckr.copy_from_slice(&hex::decode(CONSENT_KEYS.trim_start_matches("0x")).unwrap());
        let mut kh = [0u8; 32];
        kh.copy_from_slice(&hex::decode(key_hash.trim_start_matches("0x")).unwrap());
        let mut wallet = [0u8; 20];
        wallet.copy_from_slice(subject_signer.address().as_slice());
        let nonce = [0u8; 32]; // unbound subject -> bindNonce == 0.
        let digest = dogtag_standard::consent::bind_consent_key_digest(
            ckr,
            &kh,
            &wallet,
            &nonce,
            dogtag_standard::consent::DOGTAG_CHAIN_ID,
        );
        let sig = subject_signer
            .sign_hash_sync(&B256::from(digest))
            .expect("sign bind");
        format!("0x{}", hex::encode(sig.as_bytes()))
    };
    let bind =
        serde_json::json!({ "subject": subject, "keyHash": key_hash, "ownerSig": owner_sig });
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
    // The record is now ASYNC: submit returns `{status:"recording"}` immediately, then the spawned
    // task runs bindConsentKeyFor + recordVerificationZK and flips the session to `recorded`.
    assert_eq!(s, StatusCode::OK, "bind + record ack must succeed: {b}");
    assert_eq!(
        b["status"], "recording",
        "submit must ack `recording` immediately: {b}"
    );
    assert!(b["txHash"].is_null(), "no txHash on the recording ack: {b}");
    // poll the session until the background task finishes (MemChain returns immediately).
    for _ in 0..100 {
        let (_s, sb) = call(
            &app,
            "GET",
            &format!("/verify/session/{session_id}"),
            Some(&op),
            None,
        )
        .await;
        if sb["status"] == "recorded" {
            break;
        }
        assert_ne!(sb["status"], "error", "async record must not error: {sb}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        mem.consent_key_of(CONSENT_KEYS, subject)
            .await
            .unwrap()
            .to_lowercase(),
        key_hash.to_lowercase(),
        "keyOf(subject) bound to keyHash after the async record"
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
    assert_eq!(
        s,
        StatusCode::OK,
        "already-bound submit ack must succeed without bind block: {b}"
    );
    assert_eq!(
        b["status"], "recording",
        "already-bound submit must ack `recording`: {b}"
    );
    // poll until the async record lands.
    let mut recorded2 = false;
    for _ in 0..100 {
        let (_s, sb) = call(
            &app,
            "GET",
            &format!("/verify/session/{session_id2}"),
            Some(&op),
            None,
        )
        .await;
        if sb["status"] == "recorded" {
            recorded2 = true;
            break;
        }
        assert_ne!(sb["status"], "error", "async record must not error: {sb}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        recorded2,
        "already-bound async record must complete -> session recorded"
    );
}

/// ASYNC on-chain record (the export-timeout fix): a CLIENT-PROOF ZK consent submit returns
/// `{status:"recording"}` IMMEDIATELY (no txHash), then a spawned task records on-chain and flips the
/// session to `recorded` with a txHash + nullifier — and the one-time export token is consumed only on
/// that success. A forced record FAILURE leaves the session `error` and the token NOT consumed
/// (retryable QR). Mirrors the phone: authenticate with the export token, poll GET /verify/session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zk_client_proof_records_async_and_consumes_token_on_success() {
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

    let subject = "0x00000000000000000000000000000000000000de";
    let rt = record_type_key("VACCINATION");
    let credential_root = "0x1111111111111111111111111111111111111111111111111111111111111111";
    let key_hash = "0x3333333333333333333333333333333333333333333333333333333333333333";
    let purpose_b32 = vet_api::verify::purpose_key(purpose);
    // pre-bind the consent key so the submit takes the no-bind-block path (focus on the record).
    mem.set_consent_key(CONSENT_KEYS, subject, key_hash);

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
    let nullifier = "0x4444444444444444444444444444444444444444444444444444444444444444";
    let proof = serde_json::json!({
        "a": ["1", "2"],
        "b": [["1", "2"], ["3", "4"]],
        "c": ["5", "6"],
        "pubSignals": [
            "42", purpose_b32, addr_field_dec(&backend_addr), addr_field_dec(subject),
            nullifier, key_hash, credential_root
        ]
    });

    // --- start an EXPORT session as the operator -> mints a one-time export token in the qrUrl ---
    let (s, b) = call(
        &app,
        "POST",
        "/verify/session/start",
        Some(&op),
        Some(serde_json::json!({"purpose": purpose, "recordType":"VACCINATION", "mode":"zk"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "export session start: {b}");
    let session_id = b["sessionId"].as_str().unwrap().to_string();
    let qr = b["qrUrl"].as_str().unwrap();
    let token = export_token_from_qr(qr);
    assert_eq!(
        token.len(),
        32,
        "export token must be 32 hex chars: {token}"
    );

    // the token resolves the session (non-consuming) -> still valid before submit.
    let (s, _b) = call(&app, "GET", &format!("/x/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "export token must resolve before submit");

    // --- the PHONE submits with the export token (no operator bearer) -> immediate `recording` ack ---
    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent",
        None,
        Some(serde_json::json!({
            "consent": consent, "sig": "0xstub", "mode":"zk",
            "proof": proof, "exportToken": token
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "consent submit ack: {b}");
    assert_eq!(
        b["status"], "recording",
        "submit must ack `recording` immediately: {b}"
    );
    assert_eq!(
        b["sessionId"].as_str().unwrap(),
        session_id,
        "ack carries the sessionId"
    );
    assert!(b["txHash"].is_null(), "no txHash on the recording ack: {b}");

    // --- poll the session until the async record lands. Poll as the OPERATOR (bearer): the export
    //     token is consumed the instant the record succeeds, after which a token-gated poll 401s — the
    //     phone detects completion via the on-chain nullifier / its token no longer resolving. ---
    let mut recorded = serde_json::Value::Null;
    for _ in 0..100 {
        let (s, sb) = call(
            &app,
            "GET",
            &format!("/verify/session/{session_id}"),
            Some(&op),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "session poll: {sb}");
        if sb["status"] == "recorded" {
            recorded = sb;
            break;
        }
        assert_ne!(sb["status"], "error", "async record must not error: {sb}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        recorded["status"], "recorded",
        "async record must complete -> session recorded"
    );
    assert!(
        recorded["txHash"].as_str().is_some(),
        "recorded session carries the record txHash"
    );
    assert_eq!(
        recorded["nullifier"].as_str().unwrap().to_lowercase(),
        nullifier.to_lowercase(),
        "recorded session surfaces the consumed nullifier (pub[4])"
    );
    // on-chain effect: the nullifier was consumed by recordVerificationZK.
    assert!(
        mem.consumed("0x0000000000000000000000000000000000000000", nullifier)
            .await
            .unwrap(),
        "nullifier must be consumed on-chain after the async record"
    );
    // the one-time export token was CONSUMED on the record success -> resolves to a 404 now.
    let (s, _b) = call(&app, "GET", &format!("/x/{token}"), None, None).await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "export token must be consumed after a successful record"
    );

    // ---------------------------------------------------------------------------------------------
    // FORCED RECORD FAILURE: a fresh session whose proof reuses the SAME nullifier -> MemChain's
    // recordVerificationZK returns "replayed". The async task must set status=error and NOT consume
    // the token (so the owner can retry the same QR).
    // ---------------------------------------------------------------------------------------------
    let (s, b) = call(
        &app,
        "POST",
        "/verify/session/start",
        Some(&op),
        Some(serde_json::json!({"purpose": purpose, "recordType":"VACCINATION", "mode":"zk"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "export session start 2: {b}");
    let session_id2 = b["sessionId"].as_str().unwrap().to_string();
    let token2 = export_token_from_qr(b["qrUrl"].as_str().unwrap());

    // reuse the consumed nullifier -> the record will fail on-chain.
    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent",
        None,
        Some(serde_json::json!({
            "consent": consent, "sig": "0xstub", "mode":"zk",
            "proof": proof, "exportToken": token2
        })),
    )
    .await;
    // validation still passes (the replay is only detectable on-chain) -> we still ack `recording`.
    assert_eq!(
        s,
        StatusCode::OK,
        "submit ack even when the record will fail: {b}"
    );
    assert_eq!(b["status"], "recording");

    // poll until the async task records the failure.
    let mut errored = serde_json::Value::Null;
    for _ in 0..100 {
        let (_s, sb) = call(
            &app,
            "GET",
            &format!("/verify/session/{session_id2}"),
            Some(&op),
            None,
        )
        .await;
        if sb["status"] == "error" {
            errored = sb;
            break;
        }
        assert_ne!(
            sb["status"], "recorded",
            "a replayed nullifier must NOT record: {sb}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        errored["status"], "error",
        "forced record failure -> session error"
    );
    // the export token was NOT consumed -> still resolves (the owner can retry the same QR).
    let (s, _b) = call(&app, "GET", &format!("/x/{token2}"), None, None).await;
    assert_eq!(
        s,
        StatusCode::OK,
        "export token must survive a failed record (retryable QR)"
    );
}

fn common_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Extract the 32-hex export token from an export qrUrl: `{deployment}/x/{token}?a={relayer}`.
fn export_token_from_qr(qr: &str) -> String {
    qr.split("/x/")
        .nth(1)
        .unwrap()
        .split('?')
        .next()
        .unwrap()
        .to_string()
}

fn extract_token(qr: &str) -> String {
    // qrUrl: .../r/<32hex>
    qr.rsplit('/').next().unwrap().to_string()
}
