//! Public provider-directory coverage over the real indexer router and a hermetic in-process admin
//! source. No node or Mongo is involved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::header::{ACCEPT_ENCODING, CONTENT_ENCODING};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tower::ServiceExt;

use indexer_api::app::{watch_generation, AppState, Config};
use indexer_api::chain::MemLogSource;
use indexer_api::directory::Directory;
use indexer_api::scope::ScopeRegistry;
use indexer_api::store::{MemStore, Store};

const FACTORY: &str = "0x00000000000000000000000000000000000fac70";
const REGISTRY: &str = "0x0000000000000000000000000000000000c0ce61";
const VREG: &str = "0x0000000000000000000000000000000000c05e61";

#[derive(Clone)]
struct AdminStubState {
    response: Arc<Mutex<Value>>,
    request_uris: Arc<Mutex<Vec<String>>>,
}

struct AdminStub {
    base: String,
    response: Arc<Mutex<Value>>,
    request_uris: Arc<Mutex<Vec<String>>>,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl AdminStub {
    async fn start(response: Value) -> Self {
        async fn businesses(
            State(state): State<AdminStubState>,
            OriginalUri(uri): OriginalUri,
        ) -> Json<Value> {
            state.request_uris.lock().unwrap().push(uri.to_string());
            Json(state.response.lock().unwrap().clone())
        }

        let response = Arc::new(Mutex::new(response));
        let request_uris = Arc::new(Mutex::new(Vec::new()));
        let state = AdminStubState {
            response: response.clone(),
            request_uris: request_uris.clone(),
        };
        let app = Router::new()
            .route("/v1/businesses", get(businesses))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind hermetic admin stub");
        let address = listener.local_addr().expect("stub address");
        let (shutdown, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .expect("serve hermetic admin stub");
        });
        Self {
            base: format!("http://{address}"),
            response,
            request_uris,
            shutdown: Some(shutdown),
            task,
        }
    }

    fn set_response(&self, response: Value) {
        *self.response.lock().unwrap() = response;
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.await.expect("join hermetic admin stub");
    }
}

fn cfg() -> Config {
    Config {
        rpc_url: "mem://".into(),
        generations: vec![watch_generation(FACTORY, REGISTRY, VREG, vec![])
            .expect("valid provider-directory fixture generation")],
        start_block: 0,
        confirmations: 0,
        chunk_size: 100,
        poll_interval_secs: 1,
        default_page_limit: 100,
        max_page_limit: 1000,
        explorer_base: "https://explorer.roax.net".into(),
    }
}

fn state(directory: Arc<Directory>) -> AppState {
    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    AppState {
        store,
        source: Arc::new(MemLogSource::new()),
        scopes: Arc::new(ScopeRegistry::default()),
        directory,
        cfg: Arc::new(cfg()),
    }
}

async fn request(state: &AppState, path: &str) -> (StatusCode, Value) {
    let response = indexer_api::router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

async fn post_nearest(
    state: &AppState,
    path: &str,
    position: Value,
) -> (StatusCode, HeaderMap, Value) {
    let response = indexer_api::router(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&position).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, body)
}

fn provider_fixture() -> Value {
    json!({
        "businesses": [
            {
                "businessId": "biz-avila",
                "type": "vet",
                "name": "Ávila Veterinary",
                "geo": { "lat": 1.3521, "lng": 103.8198 },
                "contact": {
                    "phone": "+65 6000 0001",
                    "whatsapp": "+65 6000 0002",
                    "telegram": "avila-vet",
                    "email": "hello@avila.example",
                    "website": "https://avila.example"
                },
                "services": ["vaccination", "microchip"],
                "apiBaseUrl": "https://api.avila.example",
                "domain": "avila.example",
                "documentStores": ["https://docs.avila.example"],
                "hmacKeyId": "hmac-avila",
                "hmacSecret": "must-never-cross-the-indexer"
            },
            {
                "businessId": "biz-seaport",
                "type": "groomer",
                "name": "The Seaport Paw",
                "geo": { "lat": 1.2644, "lng": 103.8222 },
                "contact": { "website": "https://seaport.example" },
                "services": ["grooming"],
                "apiBaseUrl": "https://api.seaport.example",
                "domain": "seaport.example",
                "documentStores": [],
                "hmacKeyId": "hmac-seaport"
            },
            {
                "businessId": "biz-remote",
                "type": "vet",
                "name": "Remote Veterinary Advice",
                "geo": null,
                "contact": {
                    "phone": "+65 6000 0003",
                    "email": "remote@example"
                },
                "services": ["telehealth"],
                "apiBaseUrl": "https://api.remote.example",
                "domain": "remote.example",
                "documentStores": [],
                "hmacKeyId": "hmac-remote"
            },
            {
                "businessId": "biz-gulf",
                "type": "vet",
                "name": "Gulf of Guinea Clinic",
                "geo": { "lat": 0.0, "lng": 0.0 },
                "contact": {},
                "services": ["consultation"],
                "apiBaseUrl": "https://api.gulf.example",
                "domain": "gulf.example",
                "documentStores": [],
                "hmacKeyId": "hmac-gulf"
            },
            {
                "businessId": "biz-admin",
                "type": "admin",
                "name": "DogTag Protocol Administration",
                "geo": null,
                "contact": { "email": "registry@dogtag.example" },
                "services": ["provider-registration"],
                "apiBaseUrl": "https://admin-api.dogtag.example",
                "domain": "admin.dogtag.example",
                "documentStores": [],
                "hmacKeyId": "hmac-admin"
            },
            {
                "businessId": "biz-government",
                "type": "government",
                "name": "National Animal Health Authority",
                "geo": { "lat": 1.2966, "lng": 103.7764 },
                "contact": { "website": "https://animal-health.gov.example" },
                "services": ["travel-clearance"],
                "apiBaseUrl": "https://animal-health-api.gov.example",
                "domain": "animal-health.gov.example",
                "documentStores": [],
                "hmacKeyId": "hmac-government"
            }
        ]
    })
}

fn located_provider(id: &str, name: &str, lat: f64, lng: f64) -> Value {
    json!({
        "businessId": id,
        "type": "vet",
        "name": name,
        "geo": { "lat": lat, "lng": lng },
        "contact": {},
        "services": ["consultation"],
        "apiBaseUrl": format!("https://api.{id}.example"),
        "domain": format!("{id}.example"),
        "documentStores": [],
        "hmacKeyId": format!("hmac-{id}")
    })
}

fn deep_ranked_fixture() -> Value {
    let businesses = (0..64)
        .map(|index| {
            located_provider(
                &format!("biz-{index:03}"),
                &format!("Provider {index:03}"),
                0.0,
                index as f64 / 1_000.0,
            )
        })
        .collect::<Vec<_>>();
    json!({ "businesses": businesses })
}

fn distance_boundary_fixture() -> Value {
    json!({
        "businesses": [
            located_provider("tie-first", "Tie First", 0.0, -179.999),
            located_provider("tie-second", "Tie Second", 0.0, -179.999),
            located_provider("antipode", "Antipode", 0.0, 0.0)
        ]
    })
}

async fn loaded_state(response: Value) -> (AppState, Arc<Directory>, AdminStub) {
    let stub = AdminStub::start(response).await;
    let directory = Arc::new(Directory::new(
        HashMap::new(),
        Some(stub.base.clone()),
        None,
    ));
    directory.refresh().await;
    (state(directory.clone()), directory, stub)
}

#[tokio::test]
async fn bare_route_is_public_and_pages_the_safe_admin_directory() {
    let (state, _directory, stub) = loaded_state(provider_fixture()).await;
    let (status, body) = request(&state, "/v1/businesses").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the discovery route takes no bearer"
    );
    let businesses = body["businesses"].as_array().expect("businesses array");
    assert_eq!(
        businesses.len(),
        6,
        "the fixture fits inside the configured default page"
    );
    assert_eq!(body["total"], 6);
    assert_eq!(body["limit"], 100);
    assert_eq!(body["offset"], 0);
    assert_eq!(body["hasMore"], false);
    assert_eq!(
        businesses
            .iter()
            .map(|row| row["businessId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "biz-avila",
            "biz-seaport",
            "biz-remote",
            "biz-gulf",
            "biz-admin",
            "biz-government"
        ],
        "source order and every provider kind are preserved when the page contains the fixture"
    );

    let avila = &businesses[0];
    assert_eq!(avila["contact"]["phone"], "+65 6000 0001");
    assert_eq!(avila["contact"]["whatsapp"], "+65 6000 0002");
    assert_eq!(avila["contact"]["telegram"], "avila-vet");
    assert_eq!(avila["contact"]["email"], "hello@avila.example");
    assert_eq!(avila["contact"]["website"], "https://avila.example");
    assert!(
        avila.get("hmacSecret").is_none(),
        "the source's server secret must never cross the indexer"
    );

    let remote = &businesses[2];
    assert!(
        remote["geo"].is_null(),
        "absence stays explicit null, never 0,0"
    );
    assert_eq!(remote["contact"]["phone"], "+65 6000 0003");
    assert_eq!(remote["contact"]["email"], "remote@example");
    assert!(
        remote["contact"].get("website").is_none(),
        "website absence must not be filled from domain or apiBaseUrl"
    );
    assert_eq!(
        stub.request_uris.lock().unwrap().as_slice(),
        ["/v1/businesses"],
        "the indexer itself must fetch the complete upstream set without narrowing"
    );

    stub.stop().await;
}

#[tokio::test]
async fn name_and_kind_filters_compose_with_and_semantics() {
    let (state, _directory, stub) = loaded_state(provider_fixture()).await;

    let (status, body) = request(&state, "/v1/businesses?name=avila").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["businesses"].as_array().unwrap().len(), 1);
    assert_eq!(body["businesses"][0]["businessId"], "biz-avila");

    let (status, body) = request(&state, "/v1/businesses?name=REMOTE").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["businesses"][0]["businessId"], "biz-remote",
        "name-only search retains a contact-only provider"
    );
    assert!(body["businesses"][0]["geo"].is_null());

    let (status, body) = request(&state, "/v1/businesses?kind=VET").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["businesses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["businessId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["biz-avila", "biz-remote", "biz-gulf"]
    );

    let (status, body) = request(&state, "/v1/businesses?type=VET").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["businesses"].as_array().unwrap().len(),
        3,
        "type remains a compatibility alias for existing directory clients"
    );

    let (status, body) = request(&state, "/v1/businesses?kind=ADMIN").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["businesses"][0]["businessId"], "biz-admin");

    let (status, body) = request(&state, "/v1/businesses?kind=government").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["businesses"][0]["businessId"], "biz-government");

    let (status, body) = request(&state, "/v1/businesses?kind=vet&kind=GROOMER").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["businesses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["businessId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["biz-avila", "biz-seaport", "biz-remote", "biz-gulf"],
        "the owner caller can request vet OR groomer without the service hardcoding its policy"
    );
    assert!(
        body["businesses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| matches!(row["type"].as_str(), Some("vet" | "groomer"))),
        "an owner-selected kind set cannot leak admin or government rows"
    );

    let (status, body) = request(&state, "/v1/businesses?type=vet&type=groomer").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["businesses"].as_array().unwrap().len(),
        4,
        "the compatibility alias has the same repeatable set semantics"
    );

    let (status, body) = request(&state, "/v1/businesses?kind=vet&kind=VET").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["businesses"].as_array().unwrap().len(),
        3,
        "case-normalized duplicate kind members are harmlessly deduplicated"
    );

    let (status, body) = request(&state, "/v1/businesses?name=paw&kind=vet&kind=groomer").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["businesses"][0]["businessId"], "biz-seaport",
        "kind values are ORed while name remains an AND predicate"
    );

    let (status, body) = request(&state, "/v1/businesses?kind=boarding-kennel").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["businesses"],
        json!([]),
        "a valid no-match is authoritative empty"
    );

    let (status, body) =
        request(&state, "/v1/businesses?kind=vet&kind=groomer&limit=2&offset=1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 4);
    assert_eq!(body["limit"], 2);
    assert_eq!(body["offset"], 1);
    assert_eq!(body["hasMore"], true);
    assert_eq!(
        body["businesses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["businessId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["biz-seaport", "biz-remote"],
        "non-location pages preserve filtered source order"
    );

    let (_, whole_again) = request(&state, "/v1/businesses").await;
    assert_eq!(
        whole_again["businesses"].as_array().unwrap().len(),
        6,
        "a filtered read must never mutate the universal source snapshot"
    );

    stub.stop().await;
}

#[tokio::test]
async fn nearest_is_body_only_server_ordered_paged_and_distance_bearing() {
    let (state, _directory, stub) = loaded_state(provider_fixture()).await;
    let position = json!({ "approximateLat": 0.0, "approximateLng": 0.0 });

    let (status, headers, body) = post_nearest(
        &state,
        "/v1/businesses/nearest?kind=vet&kind=groomer&limit=2&offset=0",
        position.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("cache-control").unwrap(),
        "private, no-store",
        "a position-derived response must not be cached"
    );
    assert_eq!(body["total"], 3, "the contact-only vet has no distance");
    assert_eq!(body["limit"], 2);
    assert_eq!(body["offset"], 0);
    assert_eq!(body["hasMore"], true);
    let first_page = body["businesses"].as_array().unwrap();
    assert_eq!(first_page.len(), 2);
    assert_eq!(first_page[0]["businessId"], "biz-gulf");
    assert_eq!(first_page[0]["distanceKm"], 0.0);
    assert!(
        first_page[0]["geo"].is_object(),
        "the public provider destination remains available to the list"
    );
    let first_distances = first_page
        .iter()
        .map(|row| row["distanceKm"].as_f64().unwrap())
        .collect::<Vec<_>>();
    assert!(
        first_distances.windows(2).all(|pair| pair[0] <= pair[1]),
        "the server, not the device, orders the page by distance"
    );
    assert!(
        first_page.iter().all(|row| row["businessId"] != "biz-remote"),
        "geo:null is never given a fabricated distance"
    );

    let (status, _, second) = post_nearest(
        &state,
        "/v1/businesses/nearest?kind=vet&kind=groomer&limit=2&offset=2",
        position,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["total"], 3);
    assert_eq!(second["businesses"].as_array().unwrap().len(), 1);
    assert_eq!(second["hasMore"], false);
    assert!(
        second["businesses"][0]["distanceKm"]
            .as_f64()
            .unwrap()
            >= first_distances[1],
        "paging preserves the global nearest ordering"
    );

    let (status, _, named) = post_nearest(
        &state,
        "/v1/businesses/nearest?name=avila&kind=vet&kind=groomer&limit=25&offset=0",
        json!({ "approximateLat": 1.352, "approximateLng": 103.820 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(named["total"], 1);
    assert_eq!(named["businesses"][0]["businessId"], "biz-avila");
    assert!(
        named["businesses"][0]["distanceKm"]
            .as_f64()
            .unwrap()
            < 0.1,
        "distance is computed from the explicitly approximate body position"
    );
    assert_eq!(
        stub.request_uris.lock().unwrap().as_slice(),
        ["/v1/businesses"],
        "the caller position is neither persisted nor forwarded to the admin source"
    );

    stub.stop().await;
}

#[tokio::test]
async fn nearest_deep_offsets_match_the_global_order_and_out_of_range_is_empty() {
    let (state, _directory, stub) = loaded_state(deep_ranked_fixture()).await;
    let position = json!({ "approximateLat": 0.0, "approximateLng": 0.0 });

    let (status, _, deep) = post_nearest(
        &state,
        "/v1/businesses/nearest?kind=vet&limit=3&offset=60",
        position.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deep["total"], 64);
    assert_eq!(deep["offset"], 60);
    assert_eq!(deep["hasMore"], true);
    assert_eq!(
        deep["businesses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["businessId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["biz-060", "biz-061", "biz-062"],
        "nth-element page selection must equal the same window of a full distance sort"
    );

    let (status, _, at_total) = post_nearest(
        &state,
        "/v1/businesses/nearest?kind=vet&limit=3&offset=64",
        position.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(at_total["businesses"], json!([]));
    assert_eq!(at_total["total"], 64);
    assert_eq!(at_total["offset"], 64);
    assert_eq!(at_total["hasMore"], false);

    let (status, _, beyond_total) = post_nearest(
        &state,
        "/v1/businesses/nearest?kind=vet&limit=3&offset=999",
        position,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(beyond_total["businesses"], json!([]));
    assert_eq!(beyond_total["total"], 64);
    assert_eq!(
        beyond_total["offset"], 999,
        "a stateless offset page echoes the request instead of clamping and duplicating rows"
    );
    assert_eq!(beyond_total["hasMore"], false);

    stub.stop().await;
}

#[tokio::test]
async fn nearest_ties_keep_source_order_and_distance_is_total_at_world_boundaries() {
    let (state, _directory, stub) = loaded_state(distance_boundary_fixture()).await;
    let (status, _, body) = post_nearest(
        &state,
        "/v1/businesses/nearest?kind=vet&limit=3&offset=0",
        json!({ "approximateLat": 0.0, "approximateLng": 180.0 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let rows = body["businesses"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row["businessId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["tie-first", "tie-second", "antipode"],
        "equal-distance rows use source order as their deterministic tie-break"
    );
    let first = rows[0]["distanceKm"].as_f64().unwrap();
    let second = rows[1]["distanceKm"].as_f64().unwrap();
    let antipode = rows[2]["distanceKm"].as_f64().unwrap();
    assert_eq!(
        first, second,
        "identical destinations have identical distance"
    );
    assert!(
        (0.110..0.112).contains(&first),
        "crossing the antimeridian takes the short great-circle path"
    );
    assert!(
        (20_015.0..20_016.0).contains(&antipode),
        "the atan2 form remains finite at the antipode"
    );

    stub.stop().await;
}

#[tokio::test]
async fn nearest_rejects_precise_positions_location_queries_and_ambiguous_paging() {
    let (state, _directory, stub) = loaded_state(provider_fixture()).await;

    let (status, _, _) = post_nearest(
        &state,
        "/v1/businesses/nearest?kind=vet",
        json!({ "approximateLat": 1.3521, "approximateLng": 103.820 }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the server rejects a client that failed to coarsen before sending"
    );

    for body in [
        json!({ "approximateLat": 91.0, "approximateLng": 0.0 }),
        json!({ "approximateLat": 0.0, "approximateLng": 181.0 }),
        json!({ "approximateLat": 0.0 }),
        json!({ "approximateLat": 0.0, "approximateLng": 0.0, "radiusKm": 5.0 }),
    ] {
        let (status, _, _) =
            post_nearest(&state, "/v1/businesses/nearest?kind=vet", body).await;
        assert!(
            status.is_client_error(),
            "invalid or expanded position bodies must be rejected"
        );
    }

    for query in [
        "approximateLat=1.352&approximateLng=103.820",
        "searchCenterLat=1&searchCenterLng=2&searchRadiusKm=3",
        "near=1,2",
        "radius=5",
        "limit=2&limit=3",
        "offset=0&offset=1",
    ] {
        let (get_status, _) = request(&state, &format!("/v1/businesses?{query}")).await;
        assert_eq!(
            get_status,
            StatusCode::BAD_REQUEST,
            "{query} must not create a GET position path or ambiguous page",
        );
        let (nearest_status, _, _) = post_nearest(
            &state,
            &format!("/v1/businesses/nearest?{query}"),
            json!({ "approximateLat": 1.352, "approximateLng": 103.820 }),
        )
        .await;
        assert_eq!(
            nearest_status,
            StatusCode::BAD_REQUEST,
            "{query} must not create a nearest-query position path or ambiguous page",
        );
    }

    stub.stop().await;
}

#[tokio::test]
async fn directory_queries_reject_unknown_or_ambiguous_parameters() {
    let (state, _directory, stub) = loaded_state(provider_fixture()).await;

    let invalid = [
        "name=",
        "kind=",
        "type=",
        "kind=vet&type=vet",
        "kind=vet&kind=",
        "type=vet&type=%20",
        "name=one&name=two",
    ];
    for query in invalid {
        let (status, _) = request(&state, &format!("/v1/businesses?{query}")).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{query} must fail validation"
        );
    }

    let prohibited_or_unknown = [
        "searchCenterLat=1&searchCenterLng=2&searchRadiusKm=3",
        "searchCenterLat=1",
        "searchCenterLng=2",
        "searchRadiusKm=3",
        "near=1,2",
        "radius=5",
        "lat=1&lng=2",
        "currentLat=1&currentLng=2",
        "latitude=1&longitude=2",
        "bbox=0,0,1,1",
        "geohash=w21z",
        "placeQuery=singapore",
        "kinds=vet",
        "q=vet",
    ];
    for query in prohibited_or_unknown {
        let (status, _) = request(&state, &format!("/v1/businesses?{query}")).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{query} must be rejected rather than ignored or retained as dormant search surface"
        );
    }

    stub.stop().await;
}

#[tokio::test]
async fn public_filter_work_is_bounded_before_scanning_the_directory() {
    let (state, _directory, stub) = loaded_state(provider_fixture()).await;

    let sixteen_kinds = std::iter::repeat("kind=vet")
        .take(16)
        .collect::<Vec<_>>()
        .join("&");
    let (status, body) = request(&state, &format!("/v1/businesses?{sixteen_kinds}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 3, "the documented maximum remains usable");

    let seventeen_kinds = std::iter::repeat("kind=vet")
        .take(17)
        .collect::<Vec<_>>()
        .join("&");
    let (status, _) = request(&state, &format!("/v1/businesses?{seventeen_kinds}")).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "raw repeated members are capped before deduplication"
    );
    let (status, _, _) = post_nearest(
        &state,
        &format!("/v1/businesses/nearest?{seventeen_kinds}"),
        json!({ "approximateLat": 1.352, "approximateLng": 103.820 }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "nearest has the same bounded public query surface"
    );

    let kind_at_limit = "k".repeat(64);
    let (status, _) = request(&state, &format!("/v1/businesses?kind={kind_at_limit}")).await;
    assert_eq!(status, StatusCode::OK);
    let kind_over_limit = "k".repeat(65);
    let (status, _) = request(&state, &format!("/v1/businesses?kind={kind_over_limit}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let name_at_limit = "n".repeat(200);
    let (status, _) = request(&state, &format!("/v1/businesses?name={name_at_limit}")).await;
    assert_eq!(status, StatusCode::OK);
    let name_over_limit = "n".repeat(201);
    let (status, _) = request(&state, &format!("/v1/businesses?name={name_over_limit}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    stub.stop().await;
}

#[tokio::test]
async fn directory_page_negotiates_gzip_in_the_library_router() {
    let (state, _directory, stub) = loaded_state(provider_fixture()).await;
    let response = indexer_api::router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/businesses")
                .header(ACCEPT_ENCODING, "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(CONTENT_ENCODING).unwrap(),
        "gzip",
        "the router itself, not only the standalone binary, must compress directory pages"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        bytes.get(..2),
        Some([0x1f, 0x8b].as_slice()),
        "gzip framing is present"
    );

    stub.stop().await;
}

#[tokio::test]
async fn failed_refreshes_preserve_the_snapshot_and_successful_empty_replaces_it() {
    let (state, directory, stub) = loaded_state(provider_fixture()).await;

    stub.set_response(json!({}));
    directory.refresh().await;
    let (status, after_malformed) = request(&state, "/v1/businesses").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        after_malformed["businesses"].as_array().unwrap().len(),
        6,
        "a malformed source response cannot erase the last complete snapshot"
    );

    let mut invalid_geo = provider_fixture();
    invalid_geo["businesses"][0]["geo"]["lat"] = json!(91.0);
    stub.set_response(invalid_geo);
    directory.refresh().await;
    let (status, after_invalid_geo) = request(&state, "/v1/businesses").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        after_invalid_geo["businesses"][0]["geo"]["lat"],
        json!(1.3521),
        "an invalid coordinate rejects the entire new snapshot instead of partially swapping it"
    );

    stub.set_response(json!({ "businesses": [] }));
    directory.refresh().await;
    let (status, after_empty) = request(&state, "/v1/businesses").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        after_empty["businesses"],
        json!([]),
        "a successfully read empty directory is authoritative"
    );

    stub.stop().await;
}

#[tokio::test]
async fn never_loaded_is_unavailable_but_a_successfully_empty_source_is_empty() {
    let unavailable = Arc::new(Directory::new(HashMap::new(), None, None));
    let (status, body) = request(&state(unavailable), "/v1/businesses").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        body.get("businesses").is_none(),
        "could not read is not an empty provider list"
    );

    let (empty_state, _directory, stub) = loaded_state(json!({ "businesses": [] })).await;
    let (status, body) = request(&empty_state, "/v1/businesses").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["businesses"], json!([]));

    stub.stop().await;
}
