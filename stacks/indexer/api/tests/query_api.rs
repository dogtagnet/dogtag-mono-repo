//! End-to-end coverage: script a chain history into `MemLogSource`, drive the real ingest loop, then
//! exercise the HTTP query API through the actual router — asserting the unscoped/scoped doctrine,
//! server-side scope enforcement, filters, auth, idempotent re-scan, and reorg rollback.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt; // oneshot

use indexer_api::app::{keccak_key, watch_generation, AppState, Config};
use indexer_api::chain::{emit, MemLogSource};
use indexer_api::directory::Directory;
use indexer_api::indexer::Indexer;
use indexer_api::scope::{ScopeConfig, ScopeRegistry};
use indexer_api::store::{MemStore, Store};

const FACTORY: &str = "0x00000000000000000000000000000000000fac70";
const REGISTRY: &str = "0x0000000000000000000000000000000000c0ce61";
const VREG: &str = "0x0000000000000000000000000000000000c05e61";
const SECOND_FACTORY: &str = "0x00000000000000000000000000000000000fac71";
const SECOND_REGISTRY: &str = "0x0000000000000000000000000000000000c0ce62";
const SECOND_VREG: &str = "0x0000000000000000000000000000000000c05e62";
const GOV_CLONE: &str = "0x0000000000000000000000000000000000c10e01";
const GOV_SIGNER: &str = "0x00000000000000000000000000000000516e0001";
const OTHER_CLONE: &str = "0x0000000000000000000000000000000000c10e02";
const OTHER_SIGNER: &str = "0x00000000000000000000000000000000516e0002";
const SECOND_CLONE: &str = "0x0000000000000000000000000000000000c10e03";

fn b32(seed: u8) -> String {
    format!("0x{}", (0..32).map(|_| format!("{seed:02x}")).collect::<String>())
}

fn cfg() -> Config {
    Config {
        rpc_url: "mem://".into(),
        generations: vec![
            watch_generation(FACTORY, REGISTRY, VREG, vec![])
                .expect("valid generation-one fixture"),
        ],
        start_block: 0,
        confirmations: 0, // scripted head; index everything immediately
        chunk_size: 100,
        poll_interval_secs: 1,
        default_page_limit: 100,
        max_page_limit: 1000,
        explorer_base: "https://explorer.roax.net".into(),
    }
}

/// Script: deploy gov clone → whitelist gov signer → issue → verify → revoke, then a *different*
/// issuer's deploy + issuance (for scope-exclusion assertions).
fn seed(mem: &MemLogSource, c: &Config) {
    let generation = &c.generations[0];
    let rt = keccak_key("TRAVEL_CLEARANCE");
    let purpose = keccak_key("boarding_intake");
    mem.push_empty_block("0x00", 1000); // genesis
    mem.push_events("0x01", 1012, vec![emit::issuer_created(&generation.factory, GOV_CLONE, &rt, "Gov Travel")]);
    mem.push_events("0x02", 1024, vec![emit::whitelisted(&generation.issuer_registry, &rt, GOV_SIGNER)]);
    mem.push_events("0x03", 1036, vec![emit::root_issued(GOV_CLONE, &b32(0x11), GOV_SIGNER, 1036)]);
    mem.push_events("0x04", 1048, vec![emit::verified(&generation.verification_registry, 42, GOV_SIGNER, &purpose, &b32(0x33), 1900000000, 1048)]);
    mem.push_events("0x05", 1060, vec![emit::root_revoked(GOV_CLONE, &b32(0x11), GOV_SIGNER, 1060)]);
    // a different issuer the gov-scoped token must NOT see
    mem.push_events("0x06", 1072, vec![emit::issuer_created(&generation.factory, OTHER_CLONE, &rt, "Other")]);
    mem.push_events("0x07", 1084, vec![emit::root_issued(OTHER_CLONE, &b32(0x22), OTHER_SIGNER, 1084)]);
}

fn scopes() -> ScopeRegistry {
    ScopeRegistry::from_configs(vec![
        ScopeConfig {
            token: "gov".into(),
            label: "government-oversight".into(),
            unscoped: true,
            signers: vec![],
            clones: vec![],
        },
        ScopeConfig {
            token: "vet".into(),
            label: "gov-issuer".into(),
            unscoped: false,
            signers: vec![GOV_SIGNER.into()],
            clones: vec![GOV_CLONE.into()],
        },
    ])
}

/// Build the app + indexer. Returns the `MemLogSource` handle too (it is Arc-backed + Clone, so the
/// returned handle shares state with the one inside `AppState` — tests use it to set the finalized
/// watermark and script reorgs that the running indexer observes).
async fn build() -> (AppState, Arc<Indexer>, MemLogSource) {
    let c = cfg();
    let mem = MemLogSource::new();
    seed(&mem, &c);
    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    let directory = Arc::new(Directory::new(
        HashMap::from([(GOV_SIGNER.to_ascii_lowercase(), "DogTag Government Authority".to_string())]),
        None,
        None,
    ));
    let state = AppState {
        store,
        source: Arc::new(mem.clone()),
        scopes: Arc::new(scopes()),
        directory,
        cfg: Arc::new(c),
    };
    let indexer = Arc::new(Indexer::new(state.clone()));
    indexer.rebuild_known_clones().await;
    (state, indexer, mem)
}

async fn get(state: &AppState, path: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().uri(path).method("GET");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let resp = indexer_api::router(state.clone())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

#[tokio::test]
async fn ingest_then_unscoped_and_scoped_feeds() {
    let (state, indexer, _mem) = build().await;

    indexer.tick().await.expect("tick");

    // --- unscoped government feed: every event across both issuers ------------------------------
    let (st, body) = get(&state, "/v1/events", Some("gov")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["total"], 7, "unscoped sees all 7 events");
    assert_eq!(body["scope"]["unscoped"], true);
    // every event carries a finality annotation (all finalized here — no watermark set, confirmations=0)
    for e in body["events"].as_array().unwrap() {
        assert_eq!(e["finality"], "finalized");
        assert_eq!(
            e["generation"], FACTORY,
            "every decoded event must carry its admitting generation"
        );
    }
    // newest-first ordering
    let first = &body["events"][0];
    assert_eq!(first["type"], "rootIssued"); // block 7 is the other issuer's RootIssued
    // signer naming join present on gov-signer events
    let names: Vec<&str> = body["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["actorName"].as_str())
        .collect();
    assert!(names.contains(&"DogTag Government Authority"));
    // explorer link present
    assert!(first["txUrl"].as_str().unwrap().starts_with("https://explorer.roax.net/tx/"));

    // --- scoped feed: only the gov issuer's events ---------------------------------------------
    let (st, body) = get(&state, "/v1/events", Some("vet")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["total"], 5, "scoped sees only its own signer/clone (5), not the other issuer");
    assert_eq!(body["scope"]["unscoped"], false);
    // must NOT contain the other issuer's clone/signer
    for e in body["events"].as_array().unwrap() {
        assert_ne!(e["clone"].as_str(), Some(OTHER_CLONE));
        assert_ne!(e["actor"].as_str(), Some(OTHER_SIGNER));
    }
}

#[tokio::test]
async fn scope_cannot_be_widened_by_client_filter() {
    let (state, indexer, _mem) = build().await;
    indexer.tick().await.unwrap();
    // A scoped token asks to see the OTHER signer — server-side scope still excludes it.
    let (st, body) = get(&state, &format!("/v1/events?signer={OTHER_SIGNER}"), Some("vet")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["total"], 0, "a scoped token cannot reach another issuer via a filter");
}

#[tokio::test]
async fn filters_and_stats_and_auth() {
    let (state, indexer, _mem) = build().await;
    indexer.tick().await.unwrap();

    // type filter
    let (_, body) = get(&state, "/v1/events?type=rootIssued", Some("gov")).await;
    assert_eq!(body["total"], 2, "two RootIssued across both issuers");

    // recordType by human label (keccak'd server-side)
    let (_, body) = get(&state, "/v1/events?type=issuerCreated&recordType=TRAVEL_CLEARANCE", Some("gov")).await;
    assert_eq!(body["total"], 2, "two IssuerCreated for TRAVEL_CLEARANCE");

    // stats (unscoped)
    let (_, body) = get(&state, "/v1/stats", Some("gov")).await;
    assert_eq!(body["rootIssued"], 2);
    assert_eq!(body["rootRevoked"], 1);
    assert_eq!(body["activeCredentials"], 1);
    assert_eq!(body["verifications"], 1);
    assert_eq!(body["clones"], 2);
    assert_eq!(body["finalized"], 7, "all finalized (no watermark set, confirmations=0)");
    assert_eq!(body["pending"], 0);

    // stats (scoped) — only the gov issuer
    let (_, body) = get(&state, "/v1/stats", Some("vet")).await;
    assert_eq!(body["rootIssued"], 1);
    assert_eq!(body["clones"], 1);

    // issuers listing
    let (_, body) = get(&state, "/v1/issuers", Some("gov")).await;
    assert_eq!(body["issuers"].as_array().unwrap().len(), 2);

    // auth: no token -> 401; bad token -> 401
    let (st, _) = get(&state, "/v1/events", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = get(&state, "/v1/events", Some("nope")).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // health is open - and HONEST that this whole fixture is a simulated source.
    //
    // This assertion previously read `body["chainId"] == 135`, which encoded the defect: this state is
    // built on a `MemLogSource` whose blocks, roots and tx hashes are scripted, yet it inherited the
    // configured real ROAX id and was byte-identical to a live indexer's `/health`. A simulated source
    // now says so. See `tests/simulated_disclosure.rs` for the full contract.
    let (st, body) = get(&state, "/health", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["simulated"], true);
    assert_eq!(body["backend"], "simulated");
    assert!(
        body["chainId"].is_null(),
        "a simulated source must not report a real chainId, got {}",
        body["chainId"]
    );
}

#[tokio::test]
async fn rescan_is_idempotent() {
    let (state, indexer, _mem) = build().await;
    indexer.tick().await.unwrap();
    let (_, a) = get(&state, "/v1/events", Some("gov")).await;
    // Re-run the loop: same range, same ids -> no duplicates.
    indexer.tick().await.unwrap();
    let (_, b) = get(&state, "/v1/events", Some("gov")).await;
    assert_eq!(a["total"], b["total"], "re-scanning must not duplicate events");
    assert_eq!(b["total"], 7);
}

/// The captain-directed finality model: finalize the gov flow (blocks 1..5), leave the other issuer
/// (blocks 6..7) PENDING, then reorg the pending tail. Finalized events survive untouched; the
/// orphaned pending events are dropped and re-derived from the canonical chain.
#[tokio::test]
async fn finalized_survive_reorg_pending_reconciled() {
    let (state, indexer, mem) = build().await;
    // Finalize through block 5 (the government issue/verify/revoke flow); the other issuer's deploy +
    // issuance at blocks 6-7 stay pending.
    mem.set_finalized(5);
    indexer.tick().await.expect("tick");

    let (_, body) = get(&state, "/v1/stats", Some("gov")).await;
    assert_eq!(body["finalized"], 5, "gov flow (blocks 1-5) finalized");
    assert_eq!(body["pending"], 2, "other-issuer deploy + issuance (blocks 6-7) pending");

    // Capture a finalized gov event id so we can prove it is untouched by the reorg.
    let (_, feed) = get(&state, "/v1/events?type=rootIssued&finality=finalized", Some("gov")).await;
    assert_eq!(feed["total"], 1, "the gov RootIssued is finalized");
    let gov_issue_id = feed["events"][0]["id"].as_str().unwrap().to_string();

    // Reorg the PENDING tail: rewrite from block 6 with empty blocks (the other issuer disappears).
    // Blocks 0..5 (finalized) are retained; the finalized watermark hash at block 5 is unchanged.
    mem.reorg_from(
        6,
        vec![
            ("0xREORG6".into(), 1072, vec![]),
            ("0xREORG7".into(), 1084, vec![]),
        ],
    );
    indexer.tick().await.expect("reorg tick");

    // Orphaned pending other-issuer events are gone; the finalized gov history is intact.
    let (_, body) = get(&state, "/v1/events", Some("gov")).await;
    assert_eq!(body["total"], 5, "only the 5 finalized gov events remain");
    for e in body["events"].as_array().unwrap() {
        assert_ne!(e["clone"].as_str(), Some(OTHER_CLONE));
        assert_eq!(e["finality"], "finalized");
    }
    // the specific finalized event survived with the same id (never rewound/re-inserted)
    let (_, again) = get(&state, "/v1/events?type=rootIssued", Some("gov")).await;
    assert_eq!(again["total"], 1);
    assert_eq!(again["events"][0]["id"].as_str(), Some(gov_issue_id.as_str()));
}

/// Pending events are promoted to finalized once the watermark advances past their block.
#[tokio::test]
async fn promotion_at_watermark() {
    let (state, indexer, mem) = build().await;
    // Finalize only through block 3 (deploy + whitelist + first issue); blocks 4-7 pending.
    mem.set_finalized(3);
    indexer.tick().await.expect("tick");
    let (_, body) = get(&state, "/v1/stats", Some("gov")).await;
    assert_eq!(body["finalized"], 3);
    assert_eq!(body["pending"], 4);
    // status surfaces the finalized watermark + that it came from the (simulated) finality tag
    let (_, st) = get(&state, "/v1/status", Some("gov")).await;
    assert_eq!(st["finalitySource"], "finalized-tag");
    assert_eq!(st["finalizedBlock"], 3);

    // Advance finality to the head; every pending event promotes.
    mem.set_finalized(7);
    indexer.tick().await.expect("tick");
    let (_, body) = get(&state, "/v1/stats", Some("gov")).await;
    assert_eq!(body["finalized"], 7, "all promoted to finalized");
    assert_eq!(body["pending"], 0);
}

/// The sole owner-hidden `Verified` shape is decoded through the real ingest path and is gated to the
/// configured registry address. Its indexed row carries a deadline and has no subject field.
#[tokio::test]
async fn owner_hidden_verified_decode_and_anti_spoof() {
    let c = cfg();
    let generation = &c.generations[0];
    let mem = MemLogSource::new();
    let rt = keccak_key("TRAVEL_CLEARANCE");
    let purpose = keccak_key("boarding_intake");
    let stranger = "0x000000000000000000000000000000005778a06e"; // not a registry — must be dropped
    let deadline: u64 = 1_900_000_000;

    mem.push_empty_block("0x00", 1000); // genesis
    mem.push_events("0x01", 1012, vec![emit::issuer_created(&generation.factory, GOV_CLONE, &rt, "Gov Travel")]);
    mem.push_events("0x02", 1024, vec![emit::whitelisted(&generation.issuer_registry, &rt, GOV_SIGNER)]);
    // Owner-hidden Verified from the configured registry.
    mem.push_events(
        "0x03",
        1036,
        vec![emit::verified(&generation.verification_registry, 43, GOV_SIGNER, &purpose, &b32(0x44), deadline, 1036)],
    );
    // Anti-spoof: the same event shape from a stranger address must be dropped, not indexed.
    mem.push_events(
        "0x04",
        1048,
        vec![emit::verified(stranger, 44, GOV_SIGNER, &purpose, &b32(0x55), deadline, 1048)],
    );
    // Role confusion is still a spoof: being a configured FACTORY must not make the same address a
    // configured verification registry.
    mem.push_events(
        "0x05",
        1060,
        vec![emit::verified(
            &generation.factory,
            45,
            GOV_SIGNER,
            &purpose,
            &b32(0x56),
            deadline,
            1060,
        )],
    );

    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    let directory = Arc::new(Directory::new(HashMap::new(), None, None));
    let state = AppState {
        store,
        source: Arc::new(mem),
        scopes: Arc::new(scopes()),
        directory,
        cfg: Arc::new(c),
    };
    let indexer = Arc::new(Indexer::new(state.clone()));
    indexer.rebuild_known_clones().await;
    indexer.tick().await.expect("tick");

    // The configured-registry event is ingested; the spoofed one is dropped.
    let (st, body) = get(&state, "/v1/events?type=verified", Some("gov")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["total"], 1, "stranger-emitted Verified must be dropped");

    let events = body["events"].as_array().unwrap();
    let verified = &events[0];
    assert_eq!(verified["dogTagId"], "43");
    assert!(
        !verified.as_object().unwrap().contains_key("subject"),
        "owner-hidden Verified rows must not expose a subject field"
    );
    assert_eq!(verified["deadline"].as_u64(), Some(deadline));
    assert_eq!(verified["nullifier"].as_str().unwrap(), b32(0x44));
    assert_eq!(verified["actor"].as_str().unwrap(), GOV_SIGNER.to_ascii_lowercase());
    assert_eq!(verified["contract"].as_str().unwrap(), VREG.to_ascii_lowercase());
    assert_eq!(verified["generation"].as_str(), Some(FACTORY));

    let (_, stats) = get(&state, "/v1/stats", Some("gov")).await;
    assert_eq!(stats["verifications"], 1);
}

/// Reproduce S-4's operational defect: a valid event from a second, unconfigured deployment
/// generation is correctly rejected by the anti-spoof gate, but the status surface must make the
/// incomplete watch set visible so an empty feed cannot masquerade as "nothing happened".
#[tokio::test]
async fn missing_generation_is_visible_when_its_events_are_dropped() {
    let c = cfg();
    let mem = MemLogSource::new();
    let record_type = keccak_key("TRAVEL_CLEARANCE");
    let purpose = keccak_key("boarding_intake");

    mem.push_empty_block("0x00", 1000);
    mem.push_events(
        "0x01",
        1012,
        vec![emit::issuer_created(
            SECOND_FACTORY,
            SECOND_CLONE,
            &record_type,
            "Generation Two",
        )],
    );
    mem.push_events(
        "0x02",
        1024,
        vec![emit::root_issued(
            SECOND_CLONE,
            &b32(0x65),
            GOV_SIGNER,
            1024,
        )],
    );
    mem.push_events(
        "0x03",
        1036,
        vec![emit::verified(
            SECOND_VREG,
            99,
            GOV_SIGNER,
            &purpose,
            &b32(0x66),
            1_900_000_000,
            1036,
        )],
    );

    let state = AppState {
        store: Arc::new(MemStore::new()),
        source: Arc::new(mem),
        scopes: Arc::new(scopes()),
        directory: Arc::new(Directory::new(HashMap::new(), None, None)),
        cfg: Arc::new(c),
    };
    let indexer = Indexer::new(state.clone());
    indexer.tick().await.expect("tick");

    let (_, feed) = get(&state, "/v1/events", Some("gov")).await;
    assert_eq!(
        feed["total"], 0,
        "an event from an unknown generation must stay anti-spoof-dropped"
    );

    let (_, status) = get(&state, "/v1/status", Some("gov")).await;
    assert_eq!(
        status["lag"], 0,
        "the cursor can look fully caught up even though the unknown generation was dropped"
    );
    let watched = status["watchedGenerations"]
        .as_array()
        .expect("status must expose the exact generation watch set");
    assert_eq!(watched.len(), 1);
    assert_eq!(watched[0]["generation"], FACTORY);
    assert_eq!(watched[0]["factory"], FACTORY);
    assert_eq!(watched[0]["issuerRegistry"], REGISTRY);
    assert_eq!(watched[0]["verificationRegistry"], VREG);
    assert!(
        watched.iter().all(|generation| generation["verificationRegistry"] != SECOND_VREG),
        "status must show that generation two is missing from the watch set"
    );

    // The same log is admitted once generation two's exact role-specific triple is configured, and
    // the event is stamped with generation two's immutable factory-address id.
    let mut configured = cfg();
    configured.generations.push(
        watch_generation(SECOND_FACTORY, SECOND_REGISTRY, SECOND_VREG, vec![])
            .expect("valid generation-two fixture"),
    );
    configured.chunk_size = 1; // prove clone→generation discovery survives a chunk boundary
    let mem = MemLogSource::new();
    mem.push_empty_block("0x00", 1000);
    mem.push_events(
        "0x01",
        1012,
        vec![emit::issuer_created(
            SECOND_FACTORY,
            SECOND_CLONE,
            &record_type,
            "Generation Two",
        )],
    );
    mem.push_events(
        "0x02",
        1024,
        vec![emit::root_issued(
            SECOND_CLONE,
            &b32(0x65),
            GOV_SIGNER,
            1024,
        )],
    );
    mem.push_events(
        "0x03",
        1036,
        vec![emit::verified(
            SECOND_VREG,
            99,
            GOV_SIGNER,
            &purpose,
            &b32(0x66),
            1_900_000_000,
            1036,
        )],
    );
    let configured_state = AppState {
        store: Arc::new(MemStore::new()),
        source: Arc::new(mem),
        scopes: Arc::new(scopes()),
        directory: Arc::new(Directory::new(HashMap::new(), None, None)),
        cfg: Arc::new(configured),
    };
    Indexer::new(configured_state.clone())
        .tick()
        .await
        .expect("configured tick");

    let (_, feed) = get(&configured_state, "/v1/events", Some("gov")).await;
    assert_eq!(feed["total"], 3);
    for event in feed["events"].as_array().unwrap() {
        assert_eq!(event["generation"], SECOND_FACTORY);
    }

    let (_, status) = get(&configured_state, "/v1/status", Some("gov")).await;
    let watched = status["watchedGenerations"].as_array().unwrap();
    assert_eq!(watched.len(), 2);
    assert_eq!(watched[1]["generation"], SECOND_FACTORY);
    assert_eq!(watched[1]["factory"], SECOND_FACTORY);
    assert_eq!(watched[1]["issuerRegistry"], SECOND_REGISTRY);
    assert_eq!(watched[1]["verificationRegistry"], SECOND_VREG);
}

/// Removing a generation from configuration must also remove its historically discovered clones
/// from the live anti-spoof trust set after restart. Otherwise a stored IssuerCreated row would keep
/// admitting that generation forever even though `/v1/status` says it is no longer watched.
#[tokio::test]
async fn removed_generation_does_not_retrust_its_persisted_clone() {
    let mut two_generations = cfg();
    two_generations.generations.push(
        watch_generation(SECOND_FACTORY, SECOND_REGISTRY, SECOND_VREG, vec![])
            .expect("valid generation-two fixture"),
    );
    let record_type = keccak_key("TRAVEL_CLEARANCE");
    let shared_store: Arc<dyn Store> = Arc::new(MemStore::new());

    let first_source = MemLogSource::new();
    first_source.push_empty_block("0x00", 1000);
    first_source.push_events(
        "0x01",
        1012,
        vec![emit::issuer_created(
            SECOND_FACTORY,
            SECOND_CLONE,
            &record_type,
            "Generation Two",
        )],
    );
    let first_state = AppState {
        store: shared_store.clone(),
        source: Arc::new(first_source),
        scopes: Arc::new(scopes()),
        directory: Arc::new(Directory::new(HashMap::new(), None, None)),
        cfg: Arc::new(two_generations),
    };
    Indexer::new(first_state)
        .tick()
        .await
        .expect("initial discovery tick");

    // Restart against the same cursor/store but with only generation one configured. Block 1 must
    // have the same hash so this is an ordinary forward scan, not a reorg-rebuild path.
    let restarted_source = MemLogSource::new();
    restarted_source.push_empty_block("0x00", 1000);
    restarted_source.push_events(
        "0x01",
        1012,
        vec![emit::issuer_created(
            SECOND_FACTORY,
            SECOND_CLONE,
            &record_type,
            "Generation Two",
        )],
    );
    restarted_source.push_events(
        "0x02",
        1024,
        vec![emit::root_issued(
            SECOND_CLONE,
            &b32(0x77),
            GOV_SIGNER,
            1024,
        )],
    );
    let restarted_state = AppState {
        store: shared_store,
        source: Arc::new(restarted_source),
        scopes: Arc::new(scopes()),
        directory: Arc::new(Directory::new(HashMap::new(), None, None)),
        cfg: Arc::new(cfg()),
    };
    let restarted = Indexer::new(restarted_state.clone());
    restarted.rebuild_known_clones().await;
    restarted.tick().await.expect("restart tick");

    let (_, issued) = get(
        &restarted_state,
        "/v1/events?type=rootIssued",
        Some("gov"),
    )
    .await;
    assert_eq!(
        issued["total"], 0,
        "a removed generation's persisted clone discovery must not bypass the current gate"
    );
    let (_, status) = get(&restarted_state, "/v1/status", Some("gov")).await;
    assert_eq!(status["watchedGenerations"].as_array().unwrap().len(), 1);
    assert_eq!(status["watchedGenerations"][0]["generation"], FACTORY);
}
