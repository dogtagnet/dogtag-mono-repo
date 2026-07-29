//! Public provider-directory coverage over the real indexer router and a hermetic in-process admin
//! source. No node or Mongo is involved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::header::{ACCEPT_ENCODING, CONTENT_ENCODING};
use axum::http::{Request, StatusCode};
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
async fn bare_route_is_public_and_returns_the_whole_safe_admin_directory() {
    let (state, _directory, stub) = loaded_state(provider_fixture()).await;
    let (status, body) = request(&state, "/v1/businesses").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the discovery route takes no bearer"
    );
    let businesses = body["businesses"].as_array().expect("businesses array");
    assert_eq!(businesses.len(), 6, "bare GET is the whole source list");
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
        "source order and every provider kind are preserved"
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
async fn name_kind_and_chosen_search_area_filter_with_and_semantics() {
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

    let (status, body) = request(
        &state,
        "/v1/businesses?name=gulf&kind=vet&kind=groomer&searchCenterLat=0&searchCenterLng=0&searchRadiusKm=0",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["businesses"][0]["businessId"], "biz-gulf",
        "a real 0,0 location is measured normally and the radius boundary is inclusive"
    );

    let (status, body) = request(
        &state,
        "/v1/businesses?searchCenterLat=0&searchCenterLng=0&searchRadiusKm=1",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["businesses"].as_array().unwrap().len(), 1);
    assert_eq!(body["businesses"][0]["businessId"], "biz-gulf");
    assert!(
        body["businesses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["businessId"] != "biz-remote"),
        "geo:null cannot match a spatial filter"
    );

    let (status, body) = request(&state, "/v1/businesses?kind=boarding-kennel").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["businesses"],
        json!([]),
        "a valid no-match is authoritative empty"
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
async fn search_area_is_strict_and_current_position_aliases_are_not_a_loaded_path() {
    let (state, _directory, stub) = loaded_state(provider_fixture()).await;

    let incomplete = [
        "searchCenterLat=1",
        "searchCenterLng=2",
        "searchRadiusKm=3",
        "searchCenterLat=1&searchCenterLng=2",
        "searchCenterLat=1&searchRadiusKm=3",
        "searchCenterLng=2&searchRadiusKm=3",
    ];
    for query in incomplete {
        let (status, _) = request(&state, &format!("/v1/businesses?{query}")).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{query} must fail as a partial group"
        );
    }

    let invalid = [
        "searchCenterLat=91&searchCenterLng=0&searchRadiusKm=1",
        "searchCenterLat=0&searchCenterLng=181&searchRadiusKm=1",
        "searchCenterLat=0&searchCenterLng=0&searchRadiusKm=-1",
        "searchCenterLat=NaN&searchCenterLng=0&searchRadiusKm=1",
        "searchCenterLat=0&searchCenterLng=0&searchRadiusKm=inf",
        "name=",
        "kind=",
        "type=",
        "kind=vet&type=vet",
        "kind=vet&kind=",
        "type=vet&type=%20",
        "name=one&name=two",
        "searchCenterLat=0&searchCenterLat=1&searchCenterLng=0&searchRadiusKm=1",
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
        "limit=1",
        "offset=1",
    ];
    for query in prohibited_or_unknown {
        let (status, _) = request(&state, &format!("/v1/businesses?{query}")).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{query} must be rejected rather than ignored or treated as current-position search"
        );
    }

    stub.stop().await;
}

#[tokio::test]
async fn whole_directory_negotiates_gzip_in_the_library_router() {
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
        "the router itself, not only the standalone binary, must compress a full fetch"
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
