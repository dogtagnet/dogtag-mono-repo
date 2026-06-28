//! VET-side DOG_PROFILE (SBT) issuance flow against the in-memory `MemChain` stub.
//!
//! The vet ISSUES dog tags: operator starts a session (allocating a dogTagId + a one-time QR token),
//! the device scans `/p/<token>`, signs the EIP-191 wallet-registration message, and POSTs it to
//! `/profiles/issue/bind`. The vet recovers the signer, mints the DOG_PROFILE SBT to the wallet, and
//! returns the wrapped doc. Asserts: a valid sig mints to the wallet and the wrappedDoc root ==
//! SBT profileRoot[dogTagId]; a bad sig is rejected; the token is one-time.

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;
use vet_api::chain::{ChainClient, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";

fn start_body() -> serde_json::Value {
    serde_json::json!({
        "ownerIdentity": {
            "countryOfIdentification": "GB",
            "identification": "PASSPORT-123",
            "name": "Alice Owner"
        },
        "pet": {
            "name": "Rex",
            "species": "Canis lupus familiaris",
            "breedVbo": "VBO:0200798",
            "breedLabel": "Labrador Retriever",
            "sex": "male",
            "neuterStatus": "neutered",
            "dateOfBirth": "2021-05-01",
            "weightHistory": [{ "unit": "kg", "value": "22.7", "measuredOn": "2026-01-10" }],
            "microchip": { "code": "985141006580319", "standard": "ISO_11784_11785", "implantDate": "2021-06-01" }
        }
    })
}

/// Sign the canonical wallet-registration message with a fresh secp256k1 key; returns
/// (wallet_address_lowercase, signature_hex).
fn sign_registration() -> (String, String) {
    use alloy::signers::local::PrivateKeySigner;
    use alloy::signers::SignerSync;
    let signer = PrivateKeySigner::random();
    let wallet = format!("{:#x}", signer.address());
    let message = vet_api::auth::register_message(&wallet);
    let sig = signer.sign_message_sync(message.as_bytes()).unwrap();
    let sig_hex = format!("0x{}", hex::encode(sig.as_bytes()));
    (wallet.to_lowercase(), sig_hex)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_issue_session_start_bind_mints_to_wallet() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let state = state_with(
        chain.clone(),
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(state);

    // custody: genesis -> confirm -> unlock (registers the vet signer at index 0 in MemChain).
    let (_admin, op, _backend_addr) = boot_custody(&app).await;

    // --- operator starts a profile-issue session ---
    let (s, b) = call(&app, "POST", "/profiles/issue/session/start", Some(&op), Some(start_body())).await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    let token = b["token"].as_str().unwrap().to_string();
    let dog_tag_id = b["dogTagId"].as_str().unwrap().to_string();
    let session_id = b["sessionId"].as_str().unwrap().to_string();
    let qr = b["qr"].as_str().unwrap();
    assert_eq!(token.len(), 32, "bind token must be 32 hex chars: {token}");
    assert!(qr.ends_with(&format!("/p/{token}")), "qr must be /p/<token>: {qr}");

    // --- device resolves the QR (non-consuming) ---
    let (s, b) = call(&app, "GET", &format!("/p/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "resolve: {b}");
    assert_eq!(b["dogTagId"].as_str().unwrap(), dog_tag_id);

    // --- portal polls status: still pending ---
    let (s, b) = call(&app, "GET", &format!("/profiles/issue/session/{session_id}"), Some(&op), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["status"], "pending");

    // --- device binds with a valid EIP-191 signature -> mint ---
    let (wallet, sig) = sign_registration();
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/bind",
        None,
        Some(serde_json::json!({ "token": token, "walletAddress": wallet, "signature": sig })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "bind: {b}");
    assert_eq!(b["dogTagId"].as_str().unwrap(), dog_tag_id);
    let root = b["root"].as_str().unwrap().to_string();
    // The bind no longer waits for the mint: it returns immediately with status "minting" (no txHash yet).
    assert_eq!(b["status"], "minting", "bind must respond immediately with status=minting");
    assert!(b["txHash"].is_null(), "bind must NOT return a txHash (mint is async)");
    assert!(b["wrappedDoc"].is_object(), "bind must return wrappedDoc");

    // wrappedDoc root == the returned root.
    let wd_root = b["wrappedDoc"]["signature"]["merkleRoot"].as_str().unwrap();
    assert_eq!(wd_root, root, "wrappedDoc root must equal the returned root");

    // --- drive the async mint: poll the portal status until it flips pending -> bound (mirrors how the
    //     phone polls the chain). The background tokio task mints into the MemChain then sets txHash. ---
    let mut bound = serde_json::Value::Null;
    for _ in 0..100 {
        let (s, b) = call(&app, "GET", &format!("/profiles/issue/session/{session_id}"), Some(&op), None).await;
        assert_eq!(s, StatusCode::OK);
        if b["status"] == "bound" {
            bound = b;
            break;
        }
        assert_ne!(b["status"], "error", "mint must not error: {b}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(bound["status"], "bound", "async mint must complete -> session bound");
    assert_eq!(bound["walletAddress"].as_str().unwrap().to_lowercase(), wallet);
    assert!(bound["txHash"].as_str().is_some(), "bound session must carry the mint txHash");

    // --- on-chain effect: SBT minted to the wallet, profileRoot == root ---
    // The SBT is keyed by the CANONICAL on-chain dogTagId (the field element the circuit emits as
    // pub[0]), NOT the raw operator handle returned by the API. Resolve it the same way the mint
    // route does (`routes::onchain_dog_tag_id`) before querying ownerOf/profileRoot.
    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    let owner = chain.owner_of(SBT_ADDR, &onchain_id).await.unwrap();
    assert_eq!(owner.to_lowercase(), wallet.to_lowercase(), "SBT ownerOf must be the device wallet");
    let sbt_root = chain.profile_root_of(SBT_ADDR, &onchain_id).await.unwrap();
    assert_eq!(sbt_root.to_lowercase(), root.to_lowercase(), "wrappedDoc root must == SBT profileRoot");

    // --- token is one-time: a second bind on the same token is 410/404 ---
    let (wallet2, sig2) = sign_registration();
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/bind",
        None,
        Some(serde_json::json!({ "token": token, "walletAddress": wallet2, "signature": sig2 })),
    )
    .await;
    assert!(
        s == StatusCode::GONE || s == StatusCode::NOT_FOUND,
        "second bind on a consumed token must be 410/404, got {s}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_issue_bind_rejects_bad_signature() {
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

    let (s, b) = call(&app, "POST", "/profiles/issue/session/start", Some(&op), Some(start_body())).await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    let token = b["token"].as_str().unwrap().to_string();

    // a signature by key A but claiming wallet B (a different fresh wallet) must be rejected.
    let (_wallet_a, sig_a) = sign_registration();
    let (wallet_b, _sig_b) = sign_registration();
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/bind",
        None,
        Some(serde_json::json!({ "token": token, "walletAddress": wallet_b, "signature": sig_a })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "mismatched sig must be 401");

    // the token was NOT consumed by the rejected bind: a valid bind still works.
    let (wallet, sig) = sign_registration();
    let (s, b) = call(
        &app,
        "POST",
        "/profiles/issue/bind",
        None,
        Some(serde_json::json!({ "token": token, "walletAddress": wallet, "signature": sig })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "valid bind after a rejected one must succeed: {b}");
}
