//! A SIMULATED indexer must be structurally impossible to mistake for a live one.
//!
//! The defect this file exists to prevent: in `INDEXER_DEMO_MODE` the indexer served a scripted
//! in-memory event log - fabricated block numbers, fabricated merkle roots, `txUrl` links to
//! transactions that do not exist - while `/health` answered `{"ok":true,"chainId":135}`, byte-identical
//! to a live indexer on ROAX. During the honesty audit it reported `headBlock: 8` against a real chain
//! head of ~282,800 and nothing in any response let an operator, or the admin Dashboard/Activity and
//! government Oversight consoles that consume it, tell the two apart.
//!
//! The standing rule: **a surface states an accurate observation, or says it could not check - and a
//! simulated mode must be structurally visible.** "Could not check" must never render as either
//! neighbour.
//!
//! These tests deliberately do NOT use `MemLogSource`. They define their own `LogSource` impls, so the
//! disclosure cannot be satisfied by special-casing one concrete type: it must be derived from what the
//! source in use DECLARES via `LogSource::backend()`. If someone later hardcodes `simulated: false`,
//! reads the flag from an `INDEXER_DEMO_MODE` config bool instead of from the source, or drops the key
//! entirely, `simulated_source_cannot_answer_without_disclosing` fails.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt; // oneshot

use indexer_api::app::{watch_generation, AppState, Config};
use indexer_api::chain::{ChainError, LogSource, SourceBackend, WatchContext, SIMULATED_CHAIN_ID};
use indexer_api::directory::Directory;
use indexer_api::mirror::MemContentMirror;
use indexer_api::events::IndexedEvent;
use indexer_api::scope::{ScopeConfig, ScopeRegistry};
use indexer_api::store::{MemStore, Store};

// ------------------------------------------------------------------------------------------------
// Two log sources that exist only here. Neither is `MemLogSource` - that is the point: the endpoints
// must disclose based on what a source DECLARES, not on which concrete type it happens to be.
// ------------------------------------------------------------------------------------------------

/// A source that declares itself simulated. Stands in for any scripted/in-memory/fixture source
/// anyone adds later.
struct SimulatedFake;

#[async_trait::async_trait]
impl LogSource for SimulatedFake {
    fn backend(&self) -> SourceBackend {
        SourceBackend::Simulated
    }
    fn chain_id(&self) -> u64 {
        SIMULATED_CHAIN_ID
    }
    async fn head_block(&self) -> Result<u64, ChainError> {
        // The audit's exact symptom: a scripted head of 8 while the real chain sat at ~282,800.
        Ok(8)
    }
    async fn finalized_block(&self) -> Result<Option<u64>, ChainError> {
        Ok(Some(6))
    }
    async fn fetch_events(
        &self,
        _from: u64,
        _to: u64,
        _ctx: &WatchContext,
    ) -> Result<Vec<IndexedEvent>, ChainError> {
        Ok(vec![])
    }
    async fn block_hash(&self, _block: u64) -> Result<Option<String>, ChainError> {
        Ok(None)
    }
}

/// A source that declares itself live, reporting a real chain id and a plausible real head.
struct LiveFake;

#[async_trait::async_trait]
impl LogSource for LiveFake {
    fn backend(&self) -> SourceBackend {
        SourceBackend::Live
    }
    fn chain_id(&self) -> u64 {
        135
    }
    async fn head_block(&self) -> Result<u64, ChainError> {
        Ok(282_800)
    }
    async fn finalized_block(&self) -> Result<Option<u64>, ChainError> {
        Ok(Some(282_795))
    }
    async fn fetch_events(
        &self,
        _from: u64,
        _to: u64,
        _ctx: &WatchContext,
    ) -> Result<Vec<IndexedEvent>, ChainError> {
        Ok(vec![])
    }
    async fn block_hash(&self, _block: u64) -> Result<Option<String>, ChainError> {
        Ok(None)
    }
}

// ------------------------------------------------------------------------------------------------

fn cfg() -> Config {
    Config {
        rpc_url: "mem://".into(),
        // `Config` carries NO chain id at all any more. That was the trap: the old code read `chainId`
        // from here, so a simulated indexer inherited the configured `135` and looked live. The id now
        // has exactly one home - the source object - and a simulated source has none to give.
        generations: vec![watch_generation(
            "0x00000000000000000000000000000000000fac70",
            "0x0000000000000000000000000000000000c0ce61",
            "0x0000000000000000000000000000000000c05e61",
            vec![],
        )
        .expect("valid simulated generation fixture")],
        start_block: 0,
        confirmations: 0,
        chunk_size: 100,
        poll_interval_secs: 1,
        default_page_limit: 100,
        max_page_limit: 1000,
        explorer_base: "https://explorer.roax.net".into(),
    }
}

fn state(source: Arc<dyn LogSource>) -> AppState {
    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    AppState {
        store,
        source,
        scopes: Arc::new(ScopeRegistry::from_configs(vec![ScopeConfig {
            token: "gov".into(),
            label: "government-oversight".into(),
            unscoped: true,
            signers: vec![],
            clones: vec![],
        }])),
        directory: Arc::new(Directory::new(HashMap::new(), None, None)),
        mirror: Arc::new(MemContentMirror::new()),
        cfg: Arc::new(cfg()),
    }
}

async fn get(st: &AppState, path: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().uri(path).method("GET");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let resp = indexer_api::router(st.clone())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

/// THE REGRESSION GUARD. A source that declares itself simulated cannot answer either operator-facing
/// endpoint without saying so.
#[tokio::test]
async fn simulated_source_cannot_answer_without_disclosing() {
    let st = state(Arc::new(SimulatedFake));

    for (path, token) in [("/health", None), ("/v1/status", Some("gov"))] {
        let (code, v) = get(&st, path, token).await;
        assert_eq!(code, StatusCode::OK, "{path} must answer");

        assert_eq!(
            v["simulated"], true,
            "{path} served a SIMULATED source without `simulated:true` - an operator cannot tell this \
             feed's blocks, roots and tx links are fabricated. Body: {v}"
        );
        assert_eq!(
            v["backend"], "simulated",
            "{path} must name the backend explicitly, not only as a boolean. Body: {v}"
        );
        // `null`, not the configured 135: this source is on no network at all.
        assert!(
            v["chainId"].is_null(),
            "{path} reported chainId {} for a simulated source - this is the exact defect (config says \
             135, the source is on no network). It must be null.",
            v["chainId"]
        );
        // The key must be PRESENT and null, not absent. An absent key is indistinguishable from an
        // endpoint that never learned to answer, which is "could not check" wearing "live"'s face.
        assert!(
            v.get("chainId").is_some(),
            "{path} omitted chainId entirely; it must be present and explicitly null"
        );
    }
}

/// The disclosure must survive the scripted numbers being visibly absurd - `/v1/status` still reports
/// the fabricated head, because that IS what the source says; the flag is what makes it readable.
/// (Demo mode's behaviour is deliberately unchanged: this is disclosure, not correction.)
#[tokio::test]
async fn simulated_status_still_reports_its_scripted_progress_but_flagged() {
    let st = state(Arc::new(SimulatedFake));
    let (code, v) = get(&st, "/v1/status", Some("gov")).await;
    assert_eq!(code, StatusCode::OK);

    // Unchanged behaviour: the scripted head is reported as-is.
    assert_eq!(
        v["headBlock"], 8,
        "demo behaviour must not change - only its labelling"
    );
    assert_eq!(v["finalizedBlock"], 6);
    // ...but it is now unmistakably labelled.
    assert_eq!(v["simulated"], true);
    assert!(v["chainId"].is_null());
}

/// The live path is UNCHANGED from before this fix: same keys, same values. `simulated:false` and
/// `backend:"live"` are additive - every consumer reads these payloads as opaque JSON.
#[tokio::test]
async fn live_source_response_is_unchanged_and_says_it_is_live() {
    let st = state(Arc::new(LiveFake));

    let (code, health) = get(&st, "/health", None).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(health["ok"], true, "pre-existing key, unchanged");
    assert_eq!(
        health["chainId"], 135,
        "a live source reports its real chain id"
    );
    assert_eq!(health["simulated"], false);
    assert_eq!(health["backend"], "live");

    let (code, status) = get(&st, "/v1/status", Some("gov")).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(status["chainId"], 135);
    assert_eq!(status["headBlock"], 282_800, "pre-existing key, unchanged");
    assert_eq!(status["simulated"], false);
    assert_eq!(status["backend"], "live");
}

/// `/health` is the unauthenticated compose-healthcheck target, so the disclosure must be reachable
/// WITHOUT a bearer token - an operator curling the box is the primary reader.
#[tokio::test]
async fn health_disclosure_needs_no_token() {
    let st = state(Arc::new(SimulatedFake));
    let (code, v) = get(&st, "/health", None).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(v["simulated"], true);
}

/// `/v1/status` stays fail-closed: the disclosure did not accidentally open an authenticated endpoint.
#[tokio::test]
async fn status_remains_fail_closed_without_a_token() {
    let st = state(Arc::new(SimulatedFake));
    let (code, _) = get(&st, "/v1/status", None).await;
    assert_eq!(code, StatusCode::UNAUTHORIZED);
}

/// The shipped sources declare themselves correctly. `backend()` has no default body, so a future
/// source cannot silently inherit "live" - it will not compile until it declares. That compile-time
/// obligation is the structural half of this fix; this test pins the two declarations we ship.
#[tokio::test]
async fn shipped_sources_declare_their_backend() {
    use indexer_api::chain::{AlloyLogSource, MemLogSource};

    assert_eq!(
        MemLogSource::new().backend(),
        SourceBackend::Simulated,
        "the demo/test source must declare itself simulated"
    );
    assert_eq!(
        MemLogSource::new().chain_id(),
        SIMULATED_CHAIN_ID,
        "a simulated source must never report a real EIP-155 id"
    );
    // A live source's id is whatever the operator asserted via `CHAIN_ID`; nothing here (and nothing
    // in the handlers) checks it against the node. What the source object guarantees is the other
    // half - that a SIMULATED source cannot echo a real network id, whatever the operator configured.
    assert_eq!(
        AlloyLogSource::new("https://devrpc.roax.net")
            .with_chain_id(135)
            .backend(),
        SourceBackend::Live,
    );
    assert!(!SourceBackend::Live.is_simulated());
    assert!(SourceBackend::Simulated.is_simulated());
    assert_eq!(SourceBackend::Simulated.as_str(), "simulated");
    assert_eq!(SourceBackend::Live.as_str(), "live");
}
