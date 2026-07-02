//! Per-role persistence + CRUD over the vet's OWN records (MemChain + MemStore, no live node).
//!
//! Proves the management layer the DB task adds around the intact issuance/revocation crypto flow:
//!   1. issue → the record is persisted with its on-chain proof (tx hash, block number, contract
//!      address, and a ready-to-click `https://explorer.roax.net/tx/<hash>` link);
//!   2. GET /records lists it back from the backend (the source of truth, not a browser cache);
//!   3. PATCH updates OFF-CHAIN metadata but REJECTS any on-chain-derived field (immutability);
//!   4. revoke is a SOFT-invalidation: the record stays listed as `revoked`, keeps its original
//!      on-chain proof intact, and is still verifiable on-chain (`isValid` now reads false);
//!   5. the off-chain `expired` transition is likewise non-destructive.

mod common;

use axum::http::StatusCode;
use common::*;
use std::sync::Arc;
use vet_api::chain::{record_type_key, ChainClient, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";

/// Boot a whitelisted, unlocked vet in backend signing mode and return (app, mem, operator_token).
async fn booted() -> (axum::Router, MemChain, String) {
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
    mem.whitelist(REGISTRY, &record_type_key("VACCINATION"), &backend_addr);
    (app, mem, op)
}

/// Prepare + confirm (backend mode) a VACCINATION record, returning (record_id, merkle_root).
async fn issue_one(app: &axum::Router, op: &str, dog_tag_id: &str) -> (String, String) {
    let (s, b) = call(
        app,
        "POST",
        "/credentials/prepare",
        Some(op),
        Some(serde_json::json!({
            "recordType": "VACCINATION",
            "dogTagId": dog_tag_id,
            "fields": vaccination_fields()
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "prepare: {b}");
    assert_eq!(b["mode"], "backend");
    (
        b["recordId"].as_str().unwrap().to_string(),
        b["merkleRoot"].as_str().unwrap().to_string(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_persists_onchain_proof_and_lists_from_db() {
    let (app, mem, op) = booted().await;
    let (record_id, root) = issue_one(&app, &op, "42").await;

    // list is operator-gated.
    let (s, _b) = call(&app, "GET", "/records", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "records list must be gated");

    let (s, b) = call(&app, "GET", "/records", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "list: {b}");
    let records = b["records"].as_array().unwrap();
    assert_eq!(records.len(), 1, "the issued record is persisted + listed");
    let rec = &records[0];
    assert_eq!(rec["record_id"], record_id);
    assert_eq!(rec["status"], "issued");
    assert_eq!(rec["root"], root);
    // contract address == the DogTagIssuer clone the root anchored to.
    assert_eq!(rec["issuer_addr"].as_str().unwrap().to_lowercase(), ISSUER);
    // on-chain proof: tx hash + block number + a ready-to-click explorer link.
    let tx = rec["tx_hash"].as_str().expect("tx hash persisted");
    assert!(tx.starts_with("0x"));
    assert!(
        rec["block_number"].as_u64().is_some(),
        "block number persisted"
    );
    assert_eq!(
        rec["explorer_url"].as_str().unwrap(),
        format!("https://explorer.roax.net/tx/{tx}"),
        "explorer link built as https://explorer.roax.net/tx/<hash>"
    );
    // and the anchor is really on-chain valid.
    assert!(mem.is_valid(ISSUER, &root).await.unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn patch_updates_offchain_but_rejects_onchain_fields() {
    let (app, _mem, op) = booted().await;
    let (record_id, root) = issue_one(&app, &op, "7").await;
    let path = format!("/records/{record_id}");

    // update off-chain metadata — accepted.
    let (s, b) = call(
        &app,
        "PATCH",
        &path,
        Some(&op),
        Some(serde_json::json!({ "label": "Rex — booster", "notes": "annual" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "patch off-chain: {b}");
    assert_eq!(b["label"], "Rex — booster");
    assert_eq!(b["notes"], "annual");
    // on-chain fields untouched.
    assert_eq!(b["root"], root);

    // an absent key leaves the field unchanged.
    let (s, b) = call(
        &app,
        "PATCH",
        &path,
        Some(&op),
        Some(serde_json::json!({ "notes": "annual booster" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "patch notes only: {b}");
    assert_eq!(b["label"], "Rex — booster", "absent label key stays unchanged");
    assert_eq!(b["notes"], "annual booster");

    // an explicit JSON null clears the field.
    let (s, b) = call(
        &app,
        "PATCH",
        &path,
        Some(&op),
        Some(serde_json::json!({ "label": null })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "patch label null: {b}");
    assert!(b["label"].is_null(), "null label clears the stored value: {b}");
    assert_eq!(b["notes"], "annual booster", "notes untouched by label clear");

    // restore the label for the trailing proof-survival assertion below.
    let (s, b) = call(
        &app,
        "PATCH",
        &path,
        Some(&op),
        Some(serde_json::json!({ "label": "Rex — booster" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "patch label restore: {b}");
    assert_eq!(b["label"], "Rex — booster");

    // every on-chain-derived key is rejected (immutable chain state).
    for (k, v) in [
        ("txHash", serde_json::json!("0xdead")),
        ("tx_hash", serde_json::json!("0xdead")),
        ("blockNumber", serde_json::json!(999)),
        ("block_number", serde_json::json!(999)),
        ("issuerAddr", serde_json::json!("0xother")),
        ("contractAddress", serde_json::json!("0xother")),
        ("root", serde_json::json!("0xother")),
        ("wrappedDoc", serde_json::json!({})),
        ("explorerUrl", serde_json::json!("https://evil/tx/x")),
    ] {
        let (s, b) = call(
            &app,
            "PATCH",
            &path,
            Some(&op),
            Some(serde_json::json!({ k: v })),
        )
        .await;
        assert_eq!(
            s,
            StatusCode::BAD_REQUEST,
            "on-chain field '{k}' must be rejected: {b}"
        );
        assert!(
            b["error"].as_str().unwrap_or("").contains("immutable"),
            "rejection names immutability: {b}"
        );
    }

    // the on-chain proof survived every rejected edit.
    let (_s, b) = call(&app, "GET", "/records", Some(&op), None).await;
    let rec = &b["records"][0];
    assert_eq!(rec["root"], root);
    assert!(rec["tx_hash"].as_str().unwrap().starts_with("0x"));
    assert_eq!(rec["label"], "Rex — booster");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoke_is_soft_invalidation_keeping_history_and_proof() {
    let (app, mem, op) = booted().await;
    let (record_id, root) = issue_one(&app, &op, "99").await;
    assert!(mem.is_valid(ISSUER, &root).await.unwrap(), "issued");

    let (s, b) = call(
        &app,
        "POST",
        &format!("/records/{record_id}/revoke"),
        Some(&op),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "revoke: {b}");

    // on-chain: isValid flips to false (invalidated) but the historical anchor is untouched.
    assert!(
        !mem.is_valid(ISSUER, &root).await.unwrap(),
        "revoked on-chain"
    );
    assert!(!mem.issued_at(ISSUER, &root).await.unwrap().is_zero());

    // the record is NOT deleted — it stays listed as `revoked` with BOTH its original issuance proof
    // AND the revoke tx proof, still traceable to the explorer.
    let (_s, b) = call(&app, "GET", "/records", Some(&op), None).await;
    let records = b["records"].as_array().unwrap();
    assert_eq!(
        records.len(),
        1,
        "revoked record is retained (never deleted)"
    );
    let rec = &records[0];
    assert_eq!(rec["status"], "revoked");
    let orig_tx = rec["tx_hash"].as_str().unwrap();
    assert!(orig_tx.starts_with("0x"), "original issuance proof intact");
    assert_eq!(
        rec["explorer_url"].as_str().unwrap(),
        format!("https://explorer.roax.net/tx/{orig_tx}")
    );
    let revoke_tx = rec["revoked_tx_hash"].as_str().unwrap();
    assert_eq!(
        rec["revoke_explorer_url"].as_str().unwrap(),
        format!("https://explorer.roax.net/tx/{revoke_tx}")
    );
    assert!(rec["invalidated_at"].as_u64().is_some());

    // double-revoke is a conflict (already revoked).
    let (s, _b) = call(
        &app,
        "POST",
        &format!("/records/{record_id}/revoke"),
        Some(&op),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "already-revoked -> 409");

    // revoked is terminal: expiring a revoked record -> 409, status stays `revoked` (an off-chain
    // `expired` must never mask the on-chain revocation).
    let (s, b) = call(
        &app,
        "PATCH",
        &format!("/records/{record_id}"),
        Some(&op),
        Some(serde_json::json!({ "status": "expired" })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "expire-after-revoke -> 409: {b}");
    let (_s, b) = call(&app, "GET", "/records", Some(&op), None).await;
    assert_eq!(
        b["records"][0]["status"], "revoked",
        "never downgraded to expired"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_records_can_still_be_revoked_onchain() {
    let (app, mem, op) = booted().await;
    let (record_id, root) = issue_one(&app, &op, "11").await;

    let (s, b) = call(
        &app,
        "PATCH",
        &format!("/records/{record_id}"),
        Some(&op),
        Some(serde_json::json!({ "status": "expired", "reason": "validUntil lapsed" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "expire: {b}");
    assert!(
        mem.is_valid(ISSUER, &root).await.unwrap(),
        "expiry is off-chain only"
    );

    // an expired record can still be invalidated on-chain (e.g. compromised-but-expired credential).
    let (s, b) = call(
        &app,
        "POST",
        &format!("/records/{record_id}/revoke"),
        Some(&op),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "expired -> revoked: {b}");
    assert!(
        !mem.is_valid(ISSUER, &root).await.unwrap(),
        "isValid flips false"
    );

    // the row is retained with its ORIGINAL issuance proof plus the NEW revoke-tx proof.
    let (_s, b) = call(&app, "GET", "/records", Some(&op), None).await;
    let records = b["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    let rec = &records[0];
    assert_eq!(rec["status"], "revoked");
    let orig_tx = rec["tx_hash"].as_str().unwrap();
    assert_eq!(
        rec["explorer_url"].as_str().unwrap(),
        format!("https://explorer.roax.net/tx/{orig_tx}"),
        "original issuance proof intact"
    );
    let revoke_tx = rec["revoked_tx_hash"].as_str().unwrap();
    assert_eq!(
        rec["revoke_explorer_url"].as_str().unwrap(),
        format!("https://explorer.roax.net/tx/{revoke_tx}")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expire_is_offchain_soft_state_that_keeps_the_record() {
    let (app, mem, op) = booted().await;
    let (record_id, root) = issue_one(&app, &op, "5").await;

    let (s, b) = call(
        &app,
        "PATCH",
        &format!("/records/{record_id}"),
        Some(&op),
        Some(serde_json::json!({ "status": "expired", "reason": "validUntil lapsed" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "expire: {b}");
    assert_eq!(b["status"], "expired");

    // expiry is OFF-CHAIN: the anchor is untouched, the record retained + still verifiable on-chain.
    assert!(
        mem.is_valid(ISSUER, &root).await.unwrap(),
        "anchor untouched by expiry"
    );
    let (_s, b) = call(&app, "GET", "/records", Some(&op), None).await;
    let rec = &b["records"][0];
    assert_eq!(rec["status"], "expired");
    assert_eq!(rec["invalidation_reason"], "validUntil lapsed");
    assert!(
        rec["tx_hash"].as_str().unwrap().starts_with("0x"),
        "proof intact"
    );

    // arbitrary status transitions are rejected.
    let (s, _b) = call(
        &app,
        "PATCH",
        &format!("/records/{record_id}"),
        Some(&op),
        Some(serde_json::json!({ "status": "issued" })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "only 'expired' is allowed via patch"
    );
}
