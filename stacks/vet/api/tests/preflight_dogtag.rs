//! §7B-2 existence pre-flight: with `require_minted_dog_tag` ON, `POST /credentials/prepare` refuses to
//! anchor a record against a dogTagId whose DogTagSBT is not yet minted (returns 400 with a clear
//! message), and succeeds once the SBT is minted under the field-hashed on-chain id. Hermetic (MemChain).

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;
use vet_api::chain::{ChainClient, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepare_requires_minted_dog_tag() {
    let mem = MemChain::new();
    let chain = Arc::new(mem.clone());
    let state = with_require_minted_dog_tag(state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    ));
    let app = vet_api::router(state);
    let (_admin, op, _backend_addr) = boot_custody(&app).await;

    // wallet mode keeps the assertion on the pre-flight itself (no broadcast/whitelist machinery); the
    // pre-flight runs before the signing-mode branch, so this still exercises it.
    let (s, _b) = call(
        &app,
        "PUT",
        "/settings/signing-mode",
        Some(&op),
        Some(serde_json::json!({ "mode": "wallet" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let dog_tag_id = "42";
    let body = serde_json::json!({
        "recordType": "VACCINATION",
        "dogTagId": dog_tag_id,
        "fields": vaccination_fields(),
    });

    // (1) unminted dogTagId -> 400 with a clear message that accounts for the async mint window.
    let (s, b) = call(&app, "POST", "/credentials/prepare", Some(&op), Some(body.clone())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "unminted dogTagId must be rejected: {b}");
    assert!(
        b["error"].as_str().unwrap_or_default().contains("not confirmed on-chain"),
        "error must explain the tag is not confirmed on-chain yet: {b}"
    );

    // (2) mint the SBT under the FIELD-HASHED on-chain id (== the export circuit's pub[0]) — the same key
    //     the pre-flight queries — then prepare succeeds.
    let onchain = vet_api::routes::onchain_dog_tag_id(dog_tag_id).unwrap();
    mem.mint(
        0,
        SBT_ADDR,
        "0x0000000000000000000000000000000000000001",
        &onchain,
        &format!("0x{}", "0".repeat(64)),
    )
    .await
    .expect("seed mint");

    let (s, b) = call(&app, "POST", "/credentials/prepare", Some(&op), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "minted dogTagId must be accepted: {b}");
    assert!(b["unsignedTx"].is_object(), "wallet mode returns an unsignedTx: {b}");
}
