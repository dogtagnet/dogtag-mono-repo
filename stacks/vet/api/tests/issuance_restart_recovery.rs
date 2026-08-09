//! Dog-tag issuance recovery ACROSS A BACKEND RESTART — the state that had no coverage.
//!
//! The stranded-root state (`issue(R)` landed, `mintCustodial` did not) can only ever be completed
//! through ITS OWN session: the retry re-arms the row so the device re-posts the same root, and the
//! vet-salted identity leaves the device's reuse guard compares against exist server-side nowhere
//! else. Under the default no-Mongo demo stack that row lived in the ephemeral `MemStore`, so the
//! restart that most often CAUSES the strand (every tunnel rotation forces one) erased the only
//! route back — measured on the captain's stack 2026-08-09: dog tags 5 and 6, roots anchored
//! on-chain since 2026-08-07, sessions gone, tags unrecoverable.
//!
//! These tests drive the journal-backed store (`ISSUANCE_JOURNAL_PATH` / `MemStore::
//! with_issuance_journal`) through a REAL restart shape: a fresh `MemStore` rebuilt from the same
//! journal file, a fresh router, custody re-hydrated from the same seal and re-unlocked — over the
//! SAME `MemChain`, because the chain is precisely what a backend restart does NOT reset.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::*;

use ark_bn254::Fr;
use dogtag_standard::field::to_hex32;
use dogtag_standard::leaf::field_of_value;
use dogtag_standard::profile_tree::{build_profile_tree, AttributeLeaf, SALT_LEN};
use dogtag_standard::types::TypedScalar;
use vet_api::chain::{ChainClient, MemChain};
use vet_api::store::{MemStore, Store};

/// Test-material wallet seed. Committed in the clear; never holds value.
const DEVICE_SEED: &[u8] = b"DogTag restart-recovery test seed - TEST MATERIAL ONLY";

fn start_body() -> serde_json::Value {
    serde_json::json!({
        // BLANK identity on purpose: this file tests SURVIVAL of the session row, not the D1
        // identity gate (which `custodial_bind_identity_gate.rs` owns). The row that survives is
        // the same row either way.
        "ownerIdentity": { "countryOfIdentification": "", "identification": "", "name": "" },
        "pet": { "name": "Rex" }
    })
}

/// The device's bind payload, folded by the REAL device-side tree builder — so "the phone re-posts
/// the same root after the restart" is the actual device computation, not a literal.
struct DeviceWire {
    root: String,
    leaves: serde_json::Value,
    reserved: serde_json::Value,
}

impl DeviceWire {
    fn bind_body(&self, token: &str) -> serde_json::Value {
        serde_json::json!({
            "token": token,
            "root": self.root,
            "leaves": self.leaves,
            "reservedLeafHashes": self.reserved,
        })
    }
}

fn device_wire(dog_tag_id_field: Fr) -> DeviceWire {
    let mut owner_address = [0u8; 20];
    owner_address[16..].copy_from_slice(&0xdead_beefu32.to_be_bytes());
    let attributes = vec![AttributeLeaf {
        key_path: "credentialSubject.name".to_string(),
        salt: [7u8; SALT_LEN],
        value: TypedScalar::Str("Rex".to_string()),
    }];
    let tree = build_profile_tree(DEVICE_SEED, dog_tag_id_field, &owner_address, &attributes)
        .expect("device-side build_profile_tree");
    DeviceWire {
        root: to_hex32(&tree.root),
        leaves: serde_json::json!([{
            "keyPath": "credentialSubject.name",
            "saltHex": format!("0x{}", hex::encode([7u8; SALT_LEN])),
            "tag": 2,
            "value": "Rex",
        }]),
        reserved: serde_json::json!([
            to_hex32(&tree.owner_address_leaf),
            to_hex32(&tree.consent_key_leaf),
            to_hex32(&tree.owner_secret_leaf),
        ]),
    }
}

fn canonical_field(handle: &str) -> Fr {
    field_of_value(&TypedScalar::Integer(handle.to_string())).expect("canonical field")
}

async fn start_session(app: &axum::Router, op: &str) -> (String, String, String) {
    let (s, b) = call(
        app,
        "POST",
        "/profiles/issue/session/start",
        Some(op),
        Some(start_body()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "session start: {b}");
    (
        b["token"].as_str().unwrap().to_string(),
        b["dogTagId"].as_str().unwrap().to_string(),
        b["sessionId"].as_str().unwrap().to_string(),
    )
}

async fn await_settled(app: &axum::Router, op: &str, session_id: &str) -> serde_json::Value {
    for _ in 0..200 {
        let (s, b) = call(
            app,
            "GET",
            &format!("/profiles/issue/session/{session_id}"),
            Some(op),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        if b["status"] != "pending" && b["status"] != "minting" {
            return b;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("session {session_id} never left pending");
}

/// Mirror `main.rs` startup hydration of the custody seal (same helper as `custody_seal_persist`).
async fn hydrate_from_seal(store: &Arc<dyn Store>, path: &str) {
    if store.get_custody().await.is_none() {
        if let Some((encrypted_seed, meta)) = vet_api::custody::read_seal_file(path).unwrap() {
            store
                .put_custody(vet_api::store::CustodyBlob {
                    encrypted_seed,
                    meta,
                })
                .await;
        }
    }
}

async fn admin_login(app: &axum::Router) -> String {
    let (s, b) = call(
        app,
        "POST",
        "/admin/login",
        None,
        Some(serde_json::json!({"password": ADMIN_PW})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "admin login: {b}");
    b["token"].as_str().unwrap().to_string()
}

async fn operator_login(app: &axum::Router) -> String {
    let (s, b) = call(
        app,
        "POST",
        "/login",
        None,
        Some(serde_json::json!({"password": OPERATOR_PW})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "op login: {b}");
    b["token"].as_str().unwrap().to_string()
}

/// A scratch directory + the two persistence paths a restart preserves.
fn scratch() -> (std::path::PathBuf, String, String) {
    let dir = std::env::temp_dir().join(format!("dogtag-issuance-journal-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let journal = dir.join("vet-issuance.json").to_str().unwrap().to_string();
    let seal = dir.join("vet-custody.json").to_str().unwrap().to_string();
    (dir, journal, seal)
}

/// Boot instance #2 over the same journal + seal + chain, exactly as `main.rs` does: rebuild the
/// store from the journal, hydrate custody, settle interrupted mints, unlock. Returns
/// (router, admin_token, operator_token, settled_count).
async fn restart(
    chain: Arc<dyn ChainClient>,
    journal: &str,
    seal: &str,
) -> (axum::Router, String, String, usize) {
    let store: Arc<dyn Store> = Arc::new(
        MemStore::with_issuance_journal(journal).expect("journal reload must succeed on restart"),
    );
    hydrate_from_seal(&store, seal).await;
    let settled = vet_api::routes::settle_interrupted_issuances(&store).await;
    let st = state_with_store(chain, store, Some(seal.to_string()));
    let app = vet_api::router(st.clone());
    let admin = admin_login(&app).await;
    let (s, b) = call(
        &app,
        "POST",
        "/admin/unlock",
        Some(&admin),
        Some(serde_json::json!({"passphrase": "seed-passphrase-123"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "unlock after restart: {b}");
    let op = operator_login(&app).await;
    (app, admin, op, settled)
}

/// THE CAPTAIN'S CASE: a mint-stage failure strands an anchored root, the backend RESTARTS, and the
/// tag is still recoverable — the session survives in the journal, the operator finds it through
/// `GET /profiles/issue/sessions`, the retry re-arms it, the device re-posts the same root, and the
/// resume check completes the remaining mint. Before the journal, the restart erased the row and
/// with it the only path back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stranded_root_survives_a_backend_restart_and_completes() {
    let (dir, journal, seal) = scratch();
    let mem = MemChain::new();
    let chain: Arc<dyn ChainClient> = Arc::new(mem.clone());

    // ---- instance #1: strand a root (role missing, gate blind → issue lands, mint reverts) ----
    let store1: Arc<dyn Store> =
        Arc::new(MemStore::with_issuance_journal(&journal).expect("fresh journal store"));
    let app1 = vet_api::router(state_with_store(chain.clone(), store1, Some(seal.clone())));
    let (_admin1, op1, signer) = boot_custody(&app1).await;
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, false);
    mem.set_sbt_role_reads_failing(true);
    let (token, dog_tag_id, session_id) = start_session(&app1, &op1).await;
    let wire = device_wire(canonical_field(&dog_tag_id));
    let root = wire.root.clone();
    let (s, _b) = call(
        &app1,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let settled = await_settled(&app1, &op1, &session_id).await;
    assert_eq!(settled["status"], "error");
    assert_eq!(settled["errorStage"], "mint");
    // The strand is REAL on the chain the restart will preserve: anchored, not sealed.
    assert!(
        chain.is_valid(PROFILE_ISSUER_ADDR, &root).await.unwrap(),
        "issue(R) landed before the mint failed"
    );

    // ---- the restart: fresh store from the SAME journal, fresh router, SAME chain ----
    drop(app1);
    let (app2, _admin2, op2, settled_count) = restart(chain.clone(), &journal, &seal).await;
    assert_eq!(settled_count, 0, "an already-errored row needs no settling");

    // The operator's route back: the failed session is listed with what happened.
    let (s, b) = call(&app2, "GET", "/profiles/issue/sessions", Some(&op2), None).await;
    assert_eq!(s, StatusCode::OK, "session list: {b}");
    let rows = b["sessions"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|r| r["sessionId"] == session_id.as_str())
        .expect("the failed session survived the restart");
    assert_eq!(row["status"], "error");
    assert_eq!(row["errorStage"], "mint");
    assert_eq!(row["dogTagId"], dog_tag_id.as_str());
    assert_eq!(row["petName"], "Rex");

    // ---- the fix lands (admin grants the mint role), the retry re-arms THIS session ----
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, true);
    mem.set_sbt_role_reads_failing(false);
    let (s, b) = call(
        &app2,
        "POST",
        &format!("/profiles/issue/session/{session_id}/retry"),
        Some(&op2),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "retry after restart: {b}");
    assert_eq!(
        b["dogTagId"].as_str().unwrap(),
        dog_tag_id,
        "the SAME dogTagId — a fresh session would derive a DIFFERENT root and strand this one"
    );
    let retry_token = b["token"].as_str().unwrap().to_string();
    assert_ne!(retry_token, token);

    // The device re-scans and re-posts the SAME root; the resume check skips the already-landed
    // issue(R) (re-issuing would revert "root taken") and completes the mint.
    let (s, b) = call(
        &app2,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&retry_token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "post-restart retry bind: {b}");
    let settled = await_settled(&app2, &op2, &session_id).await;
    assert_eq!(
        settled["status"], "bound",
        "the stranded root completes across the restart: {settled}"
    );
    assert!(settled["txHash"].as_str().is_some());

    // Both on-chain conditions hold — the recovered tag is verifiable.
    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    assert_eq!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .unwrap()
            .to_lowercase(),
        root.to_lowercase()
    );
    assert!(chain.is_valid(PROFILE_ISSUER_ADDR, &root).await.unwrap());

    // The id counter also survived: a fresh registration on instance #2 does NOT re-allocate the
    // recovered tag's id (a reset counter would hand the stranded id to the next walk-in).
    let (_t, fresh_id, _sid) = start_session(&app2, &op2).await;
    assert!(
        fresh_id.parse::<u64>().unwrap() > dog_tag_id.parse::<u64>().unwrap(),
        "fresh dogTagId {fresh_id} must be beyond the recovered {dog_tag_id}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A restart DURING the chain writes ("minting") settles the row to a RETRYABLE error at boot —
/// without that, the journal preserves a row the retry route refuses forever, which is the same
/// locked-out state one step later. The retry's resume check then reads the chain and completes
/// only what remains.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_mid_minting_settles_to_a_retryable_error_and_completes() {
    let (dir, journal, seal) = scratch();
    let mem = MemChain::new();
    let chain: Arc<dyn ChainClient> = Arc::new(mem.clone());

    let store1: Arc<dyn Store> =
        Arc::new(MemStore::with_issuance_journal(&journal).expect("fresh journal store"));
    let app1 = vet_api::router(state_with_store(
        chain.clone(),
        store1.clone(),
        Some(seal.clone()),
    ));
    let (_admin1, op1, signer) = boot_custody(&app1).await;
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, true);

    let (_token, dog_tag_id, session_id) = start_session(&app1, &op1).await;
    let wire = device_wire(canonical_field(&dog_tag_id));
    let root = wire.root.clone();

    // Model the process dying between the two chain writes: issue(R) landed, the row says
    // "minting", and no task survives to settle it. (The row shape is exactly what the bind
    // persists before its spawn; the write goes through the journaled store.)
    chain
        .issue(0, PROFILE_ISSUER_ADDR, &root)
        .await
        .expect("issue(R) lands before the crash");
    let mut row = store1.get_profile_session(&session_id).await.unwrap();
    row.status = "minting".to_string();
    row.root = Some(root.clone());
    store1.update_profile_session(row).await;

    // ---- the restart: boot settles the interrupted row to a retryable error ----
    drop(app1);
    let (app2, _admin2, op2, settled_count) = restart(chain.clone(), &journal, &seal).await;
    assert_eq!(settled_count, 1, "the interrupted mint is settled at boot");

    let (s, b) = call(
        &app2,
        "GET",
        &format!("/profiles/issue/session/{session_id}"),
        Some(&op2),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["status"], "error");
    assert_eq!(b["errorStage"], "interrupted");
    assert!(
        b["error"].as_str().unwrap().contains("restarted"),
        "the reason names the restart: {b}"
    );

    // Retry → device re-posts the same root → resume check finds it anchored → mint-only completion.
    let (s, b) = call(
        &app2,
        "POST",
        &format!("/profiles/issue/session/{session_id}/retry"),
        Some(&op2),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "retry of interrupted session: {b}");
    let retry_token = b["token"].as_str().unwrap().to_string();
    let (s, b) = call(
        &app2,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire.bind_body(&retry_token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "bind after interrupted restart: {b}");
    let settled = await_settled(&app2, &op2, &session_id).await;
    assert_eq!(settled["status"], "bound", "{settled}");

    let onchain_id = vet_api::routes::onchain_dog_tag_id(&dog_tag_id).unwrap();
    assert_eq!(
        chain
            .profile_root_of(SBT_CONSENT_ADDR, &onchain_id)
            .await
            .unwrap()
            .to_lowercase(),
        root.to_lowercase()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The journal's one-time marks and the id counter survive a restart at the STORE level: a consumed
/// bind token can never bind again, and the counter never re-allocates. (Un-burning a token across
/// a restart would be a replay; a reset counter hands a stranded session's dogTagId to the next
/// fresh registration.)
#[tokio::test]
async fn the_journal_keeps_consumed_tokens_and_the_counter_across_a_restart() {
    let (dir, journal, _seal) = scratch();
    let exp = vet_api::auth::now() + 600;

    let store1 = MemStore::with_issuance_journal(&journal).unwrap();
    assert_eq!(store1.next_dog_tag_id().await, 1);
    assert_eq!(store1.next_dog_tag_id().await, 2);
    store1.put_bind_token("aa11", "sess-1", exp).await;
    store1.put_bind_token("bb22", "sess-2", exp).await;
    assert_eq!(
        store1.take_bind_token("aa11").await.as_deref(),
        Some("sess-1")
    );

    let store2 = MemStore::with_issuance_journal(&journal).unwrap();
    assert_eq!(
        store2.take_bind_token("aa11").await,
        None,
        "a consumed one-time token STAYS consumed across the restart"
    );
    assert_eq!(
        store2.take_bind_token("bb22").await.as_deref(),
        Some("sess-2"),
        "an unconsumed token is still redeemable"
    );
    assert_eq!(
        store2.next_dog_tag_id().await,
        3,
        "the dogTagId counter continues rather than re-allocating"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A corrupt journal REFUSES to load rather than silently starting empty — starting empty over a
/// corrupt file is exactly the data loss the journal exists to prevent (`main.rs` treats this as
/// fatal, mirroring the custody seal).
#[tokio::test]
async fn a_corrupt_journal_refuses_rather_than_starting_empty() {
    let (dir, journal, _seal) = scratch();
    std::fs::write(&journal, b"{ not json").unwrap();
    assert!(
        MemStore::with_issuance_journal(&journal).is_err(),
        "corrupt journal must refuse, not start empty"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A fresh dogTagId cannot complete ANOTHER session's anchored root. A shipped device can never
/// produce this bind (its tree is keyed on the fresh session's own id, so it posts a fresh root),
/// but a hand-crafted client could post the stranded root on a fresh session's token — and without
/// the cross-session guard the resume path would mint the fresh id onto a root whose KDF binds a
/// different id, retiring it onto a tag that can never prove consent. The refusal names the owning
/// session's dogTagId, and the stranded root stays completable by ITS OWN retry afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fresh_sessions_bind_cannot_adopt_another_sessions_anchored_root() {
    let (dir, journal, seal) = scratch();
    let mem = MemChain::new();
    let chain: Arc<dyn ChainClient> = Arc::new(mem.clone());
    let store1: Arc<dyn Store> =
        Arc::new(MemStore::with_issuance_journal(&journal).expect("fresh journal store"));
    let app = vet_api::router(state_with_store(chain.clone(), store1, Some(seal.clone())));
    let (_admin, op, signer) = boot_custody(&app).await;

    // Strand session A's root: anchored, unminted, row errored at "mint".
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, false);
    mem.set_sbt_role_reads_failing(true);
    let (token_a, dog_tag_a, session_a) = start_session(&app, &op).await;
    let wire_a = device_wire(canonical_field(&dog_tag_a));
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire_a.bind_body(&token_a)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let settled = await_settled(&app, &op, &session_a).await;
    assert_eq!(settled["errorStage"], "mint");
    mem.set_sbt_issuer_role(SBT_CONSENT_ADDR, &signer, true);
    mem.set_sbt_role_reads_failing(false);

    // Session B (fresh dogTagId) binds with SESSION A'S root and openings — hand-crafted, since a
    // real device would fold B's own tree. The guard must refuse; B's id must stay unsealed.
    let (token_b, dog_tag_b, session_b) = start_session(&app, &op).await;
    assert_ne!(dog_tag_a, dog_tag_b);
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire_a.bind_body(&token_b)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "the refusal settles in the background arm");
    let settled_b = await_settled(&app, &op, &session_b).await;
    assert_eq!(settled_b["status"], "error", "{settled_b}");
    assert!(
        settled_b["error"]
            .as_str()
            .unwrap()
            .contains(&format!("dogTagId {dog_tag_a}")),
        "the refusal names the owning session: {settled_b}"
    );
    let onchain_b = vet_api::routes::onchain_dog_tag_id(&dog_tag_b).unwrap();
    assert!(
        !matches!(chain.profile_root_of(SBT_CONSENT_ADDR, &onchain_b).await, Ok(r) if r.trim_start_matches("0x").bytes().any(|b| b != b'0')),
        "dogTagId {dog_tag_b} must NOT have been sealed onto session A's root"
    );

    // The stranded root is still completable by ITS OWN session — the guard cost the owner nothing.
    let (s, b) = call(
        &app,
        "POST",
        &format!("/profiles/issue/session/{session_a}/retry"),
        Some(&op),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "retry of the owning session: {b}");
    let retry_token = b["token"].as_str().unwrap().to_string();
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/custodial-bind",
        None,
        Some(wire_a.bind_body(&retry_token)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let settled_a = await_settled(&app, &op, &session_a).await;
    assert_eq!(settled_a["status"], "bound", "{settled_a}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The recovery surfaces stay operator-gated: an unauthenticated caller can neither enumerate
/// sessions nor drive someone else's retry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unrelated_caller_cannot_list_or_retry() {
    let mem = MemChain::new();
    let chain: Arc<dyn ChainClient> = Arc::new(mem.clone());
    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    let app = vet_api::router(state_with_store(chain, store, None));

    let (s, _b) = call(&app, "GET", "/profiles/issue/sessions", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _b) = call(
        &app,
        "POST",
        "/profiles/issue/session/some-id/retry",
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}
