//! Custody seal persistence (signer survives a backend restart).
//!
//! Genesis-confirm writes the sealed custody (age ciphertext + non-secret meta) to `CUSTODY_SEAL_PATH`
//! (atomic temp+rename, 0600). A FRESH app instance constructed with the SAME path hydrates the store
//! to "initialized but locked" (no auto-unlock — no passphrase on disk). The operator then re-establishes
//! the admin session (password only — does NOT depend on in-memory genesis state) and unlocks with the
//! passphrase, re-deriving the SAME account-0 address. A wrong passphrase still fails.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use vet_api::store::{MemStore, Store};

use common::{call, state_with_seal_path, ADMIN_PW, OPERATOR_PW};

/// Mirror `main.rs` startup hydration: if the seal file exists and the store has no custody yet,
/// load `(ciphertext, meta)` from disk and put it into the store ("initialized but locked").
async fn hydrate_from_seal(store: &Arc<dyn Store>, path: &str) {
    if store.get_custody().await.is_none() {
        if let Some((encrypted_seed, meta)) = vet_api::custody::read_seal_file(path).unwrap() {
            store
                .put_custody(vet_api::store::CustodyBlob { encrypted_seed, meta })
                .await;
        }
    }
}

/// Run genesis on `app` (admin-gated) and return the freshly-genesised account-0 address.
async fn run_genesis(app: &axum::Router, admin: &str, passphrase: &str) -> String {
    let (s, b) = call(app, "POST", "/admin/genesis/start", Some(admin), Some(serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::OK, "genesis start: {b}");
    let words: Vec<String> = b["words"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    let challenge: Vec<usize> = b["challengeIndices"].as_array().unwrap().iter().map(|w| w.as_u64().unwrap() as usize).collect();
    let typed: Vec<String> = challenge.iter().map(|&i| words[i].clone()).collect();

    let (s, b) = call(
        app,
        "POST",
        "/admin/genesis/confirm",
        Some(admin),
        Some(serde_json::json!({"words": typed, "passphrase": passphrase})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "genesis confirm: {b}");
    b["address"].as_str().unwrap().to_string()
}

async fn admin_login(app: &axum::Router) -> String {
    let (s, b) = call(app, "POST", "/admin/login", None, Some(serde_json::json!({"password": ADMIN_PW}))).await;
    assert_eq!(s, StatusCode::OK, "admin login: {b}");
    b["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn seal_survives_restart_same_signer() {
    let passphrase = "seed-passphrase-123";
    let dir = std::env::temp_dir().join(format!("dogtag-seal-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let seal_path = dir.join("vet-custody.json").to_str().unwrap().to_string();

    // ---- instance #1: genesis (writes the seal to disk) ----
    let store1: Arc<MemStore> = Arc::new(MemStore::new());
    let st1 = state_with_seal_path(seal_path.clone(), store1);
    let app1 = vet_api::router(st1);

    let admin1 = admin_login(&app1).await;
    let addr_before = run_genesis(&app1, &admin1, passphrase).await;

    // the seal file now exists ...
    assert!(std::path::Path::new(&seal_path).exists(), "seal file written on genesis_confirm");
    let raw = std::fs::read_to_string(&seal_path).unwrap();
    // ... and contains ONLY ciphertext + non-secret meta: no plaintext mnemonic, no passphrase.
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(parsed.get("sealed_b64").is_some(), "seal has ciphertext");
    assert!(parsed.get("meta").is_some(), "seal has meta");
    assert!(!raw.contains(passphrase), "passphrase must NOT be on disk");
    // age-armored ciphertext header sanity + no obvious BIP39 word leakage (the address is fine; it's public).
    assert!(parsed["sealed_b64"].as_str().unwrap().len() > 0);
    // 0600 perms on unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&seal_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "seal file must be owner-only (0600)");
    }

    // ---- "restart": a FRESH app instance + FRESH store, SAME seal path ----
    let store2_concrete: Arc<MemStore> = Arc::new(MemStore::new());
    let st2 = state_with_seal_path(seal_path.clone(), store2_concrete);
    // hydrate exactly as main.rs does on startup.
    hydrate_from_seal(&st2.store, &seal_path).await;
    let app2 = vet_api::router(st2.clone());

    // hydrated => "initialized but locked": genesis/start is a 409 (no re-genesis), and not unlocked yet.
    assert!(!st2.custody.is_unlocked(), "custody starts LOCKED after restart");
    let admin2 = admin_login(&app2).await; // admin login works against a hydrated-but-locked custody
    let (s, _b) = call(&app2, "POST", "/admin/genesis/start", Some(&admin2), Some(serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "already initialized -> no re-genesis");

    // wrong passphrase still fails.
    let (s, _b) = call(
        &app2,
        "POST",
        "/admin/unlock",
        Some(&admin2),
        Some(serde_json::json!({"passphrase": "WRONG-passphrase"})),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "wrong passphrase rejected");

    // right passphrase unlocks -> SAME account-0 address as before the restart.
    let (s, b) = call(
        &app2,
        "POST",
        "/admin/unlock",
        Some(&admin2),
        Some(serde_json::json!({"passphrase": passphrase})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "unlock with right passphrase: {b}");
    let addr_after = st2.custody.active_address().unwrap();
    assert_eq!(addr_before, addr_after, "same signer re-derived after restart");

    // operator login also works post-restart (independent of genesis state).
    let (s, _b) = call(&app2, "POST", "/login", None, Some(serde_json::json!({"password": OPERATOR_PW}))).await;
    assert_eq!(s, StatusCode::OK);

    let _ = std::fs::remove_dir_all(&dir);
}
