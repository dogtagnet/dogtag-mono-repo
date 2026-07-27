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
    // Anchored to the SAME factory the harness config names, so `issue()` registers `rootIssuer[R]`
    // exactly as a real clone does (`DogTagIssuer.sol:56`). Without this the mandatory
    // issuer-whitelist pillar would resolve `unresolved` and the direct-verify assertions below would
    // silently stop testing a pass.
    let mem = MemChain::new().with_factory(FACTORY_ADDR);
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
    // The clone's own immutable `recordType()`, as the factory's `createIssuer` fixes it. The pillar
    // asks the CHAIN which record type a root belongs to rather than trusting the envelope, so a clone
    // that never declared one leaves the pillar indeterminate.
    mem.set_record_type(ISSUER, &rt);

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
    let shared_doc = doc.clone();

    // third-party verify of the returned doc: issuance pillar TRUE (root is issued on chain).
    assert!(
        mem.is_valid(ISSUER, &merkle_root).await.unwrap(),
        "issuance pillar: root must be valid on-chain after issue"
    );
    let (s, b) = call(
        &app,
        "POST",
        "/verify/credential",
        Some(&op),
        Some(serde_json::json!({
            "wrappedDoc": shared_doc.clone(),
            "signerAddr": backend_addr.clone(),
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "direct verify valid: {b}");
    assert_eq!(b["verdict"], true);
    assert_eq!(b["status"], "valid");
    assert_eq!(b["fragments"]["integrity"], true);
    assert_eq!(b["fragments"]["onchain"], true);
    assert_eq!(b["fragments"]["issued"], true);
    assert_eq!(b["fragments"]["revoked"], false);
    assert_eq!(b["fragments"]["issuerWhitelisted"], true);
    // The pillar was EVALUATED, not skipped. `issuerWhitelisted: true` alone cannot tell those apart.
    assert_eq!(b["fragments"]["issuerWhitelistState"], "passed");
    // Resolved through the factory's write-once index, never through the document's own claim.
    assert_eq!(b["issuerResolution"], "resolved");
    assert_eq!(b["issuerAddr"], ISSUER);
    // The signer is the one the CHAIN recorded in `issuedBy[R]`, not the one the caller asserted.
    assert_eq!(b["signerAddr"], backend_addr.to_lowercase());

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
    let (s, b) = call(
        &app,
        "POST",
        "/verify/credential",
        Some(&op),
        Some(serde_json::json!({
            "wrappedDoc": shared_doc,
            "signerAddr": backend_addr.clone(),
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "direct verify revoked: {b}");
    assert_eq!(b["verdict"], false);
    assert_eq!(b["status"], "revoked");
    assert_eq!(b["fragments"]["issued"], true);
    assert_eq!(b["fragments"]["revoked"], true);
    assert_eq!(b["fragments"]["onchain"], false);
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

fn extract_token(qr: &str) -> String {
    // qrUrl: .../r/<32hex>
    qr.rsplit('/').next().unwrap().to_string()
}
