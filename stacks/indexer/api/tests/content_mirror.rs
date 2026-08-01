//! The S-17 content-addressed mirror, driven end to end over the REAL indexer router.
//!
//! Hermetic: `MemContentMirror` + `MemStore` + `MemLogSource`, no node and no Mongo.
//!
//! What these cases exist to pin, beyond "it stores and serves":
//!   - content submitted under an address it does not hash to is REFUSED, and stores nothing;
//!   - an unknown address is 404 (a fact) while an unreadable store is 503 (could not check), and a
//!     store failure never arrives as a 404;
//!   - a row that no longer hashes to its key is refused on the way OUT rather than served;
//!   - SVG is refused however correct its address, and relabelling it as a PNG does not get it in.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use indexer_api::app::{watch_generation, AppState, Config};
use indexer_api::chain::MemLogSource;
use indexer_api::directory::Directory;
use indexer_api::mirror::{content_address, MemContentMirror, PROFILE_MEDIA_TYPE};
use indexer_api::scope::{ScopeConfig, ScopeRegistry};
use indexer_api::store::{MemStore, Store};

const FACTORY: &str = "0x00000000000000000000000000000000000fac70";
const REGISTRY: &str = "0x0000000000000000000000000000000000c0ce61";
const VREG: &str = "0x0000000000000000000000000000000000c05e61";
const TOKEN: &str = "publisher";

fn cfg() -> Config {
    Config {
        rpc_url: "mem://".into(),
        generations: vec![
            watch_generation(FACTORY, REGISTRY, VREG, vec![]).expect("valid fixture generation")
        ],
        start_block: 0,
        confirmations: 0,
        chunk_size: 100,
        poll_interval_secs: 1,
        default_page_limit: 100,
        max_page_limit: 1000,
        explorer_base: "https://explorer.roax.net".into(),
    }
}

fn state(mirror: MemContentMirror) -> AppState {
    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    AppState {
        store,
        source: Arc::new(MemLogSource::new()),
        scopes: Arc::new(ScopeRegistry::from_configs(vec![ScopeConfig {
            token: TOKEN.into(),
            label: "publisher".into(),
            unscoped: true,
            signers: vec![],
            clones: vec![],
        }])),
        directory: Arc::new(Directory::new(std::collections::HashMap::new(), None, None)),
        mirror: Arc::new(mirror),
        cfg: Arc::new(cfg()),
    }
}

/// A minimal but genuinely PNG-shaped body: the 8-byte signature the sniffer keys on plus a payload.
fn png(payload: &str) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.extend_from_slice(payload.as_bytes());
    v
}

async fn put(
    state: &AppState,
    address: &str,
    bytes: Vec<u8>,
    media_type: &str,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("PUT")
        .uri(format!("/v1/content/{address}"))
        .header("content-type", media_type);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let response = indexer_api::router(state.clone())
        .oneshot(builder.body(Body::from(bytes)).unwrap())
        .await
        .expect("router responds");
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
    )
}

/// Returns the raw bytes as well as the status, because the whole point of the read path is WHICH
/// bytes come back — a case that only inspected the status could not tell a correct object from a
/// substituted one.
async fn get(state: &AppState, address: &str) -> (StatusCode, Option<String>, Vec<u8>) {
    let response = indexer_api::router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/content/{address}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router responds");
    let status = response.status();
    let media_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, media_type, body.to_vec())
}

#[tokio::test]
async fn published_content_round_trips_under_its_own_address() {
    let state = state(MemContentMirror::new());
    let bytes = png("the-provider-logo");
    let address = content_address(&bytes);

    let (status, body) = put(&state, &address, bytes.clone(), "image/png", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["address"], address);

    let (status, media_type, served) = get(&state, &address).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(media_type.as_deref(), Some("image/png"));
    assert_eq!(served, bytes, "the mirror must serve exactly what was published");
    assert_eq!(
        content_address(&served),
        address,
        "and it must re-hash to the address it was requested under"
    );
}

#[tokio::test]
async fn content_that_does_not_hash_to_its_address_is_refused_and_stores_nothing() {
    let mirror = MemContentMirror::new();
    let state = state(mirror.clone());
    let honest = png("the-real-logo");
    let impostor = png("a-different-logo-entirely");
    let address = content_address(&honest);

    let (status, body) = put(&state, &address, impostor, "image/png", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap().contains("does not hash to the address"),
        "the refusal must name the reason: {body}"
    );
    assert!(mirror.is_empty(), "a refused ingest must leave the store untouched");

    let (status, _, _) = get(&state, &address).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "and the address must still name nothing"
    );
}

#[tokio::test]
async fn an_unknown_address_is_a_fact_and_an_unreadable_store_is_not() {
    let mirror = MemContentMirror::new();
    let state = state(mirror.clone());
    let address = content_address(&png("never-published"));

    let (status, _, _) = get(&state, &address).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "nothing is mirrored here");

    mirror.set_fail_reads(true);
    let (status, _, _) = get(&state, &address).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a store that could not be read must never answer 404 — that would send a publisher to \
         re-publish content that may already be there while the real fault goes uninvestigated"
    );
}

#[tokio::test]
async fn a_corrupted_row_is_refused_on_the_way_out_rather_than_served() {
    let mirror = MemContentMirror::new();
    let state = state(mirror.clone());
    let honest = png("the-real-logo");
    let address = content_address(&honest);

    // Planted, because `put` cannot produce this state: the ingest check is what stops it. This is
    // storage corruption or a hostile store, which is exactly the case the read-side check exists
    // for.
    let mut tampered = honest.clone();
    *tampered.last_mut().unwrap() ^= 0xFF;
    mirror.plant_unchecked(&address, tampered.clone(), "image/png");

    let (status, _, served) = get(&state, &address).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_ne!(served, tampered, "the tampered bytes must not reach the caller");
}

#[tokio::test]
async fn a_valid_but_wrong_image_planted_at_an_address_is_still_refused() {
    // The discriminator that matters for the client rule. An unparseable blob would be dropped by
    // the browser's own decoder and prove nothing about the address check; a perfectly renderable
    // image at the wrong address is only caught by recomputation.
    let mirror = MemContentMirror::new();
    let state = state(mirror.clone());
    let requested = content_address(&png("the-genuine-provider-logo"));
    let substituted = png("a-completely-valid-but-different-logo");

    mirror.plant_unchecked(&requested, substituted.clone(), "image/png");

    let (status, _, served) = get(&state, &requested).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_ne!(
        served, substituted,
        "a valid image at the wrong address is a substitution, and must not be served"
    );
}

#[tokio::test]
async fn svg_is_refused_however_correct_its_address_and_relabelling_does_not_get_it_in() {
    let mirror = MemContentMirror::new();
    let state = state(mirror.clone());
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#.to_vec();
    let address = content_address(&svg);

    let (status, body) = put(&state, &address, svg.clone(), "image/svg+xml", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // The sniff is what stops the allowlist being bypassed by a lie about the type. Without it the
    // declared type would be the only gate, and the declaration is provider-supplied.
    let (status, body) = put(&state, &address, svg, "image/png", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(mirror.is_empty(), "neither attempt may store anything");
}

#[tokio::test]
async fn publishing_needs_a_token_while_reading_does_not() {
    let state = state(MemContentMirror::new());
    let bytes = png("some-logo");
    let address = content_address(&bytes);

    let (status, _) = put(&state, &address, bytes.clone(), "image/png", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "an open write surface is a free content host");

    let (status, _) = put(&state, &address, bytes.clone(), "image/png", Some("not-a-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = put(&state, &address, bytes, "image/png", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);

    // The read surface is public, like the rest of the provider directory: what it serves is a
    // provider's own publication-safe content, whose digest is already on chain for anyone to read.
    let (status, _, _) = get(&state, &address).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_profile_blob_round_trips_as_json() {
    let state = state(MemContentMirror::new());
    let blob = br#"{"schema":"dogtag/provider-profile/2","contact":{"phone":"+65 6123 4567"}}"#.to_vec();
    let address = content_address(&blob);

    let (status, body) = put(&state, &address, blob.clone(), PROFILE_MEDIA_TYPE, Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, media_type, served) = get(&state, &address).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(media_type.as_deref(), Some(PROFILE_MEDIA_TYPE));
    assert_eq!(served, blob);
}

#[tokio::test]
async fn a_malformed_address_names_nothing_and_cannot_be_written_to() {
    let state = state(MemContentMirror::new());
    let bytes = png("some-logo");

    let (status, _, _) = get(&state, "0xnot-an-address").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = put(&state, "0x1234", bytes, "image/png", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn the_served_response_forbids_content_type_sniffing() {
    // These bytes render inside operator portals holding an admin session. The type is allowlisted
    // and sniffed on ingest; this stops a browser overriding that on the way out.
    let state = state(MemContentMirror::new());
    let bytes = png("some-logo");
    let address = content_address(&bytes);
    put(&state, &address, bytes, "image/png", Some(TOKEN)).await;

    let response = indexer_api::router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/content/{address}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router responds");
    assert_eq!(
        response.headers().get("x-content-type-options").and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
}
