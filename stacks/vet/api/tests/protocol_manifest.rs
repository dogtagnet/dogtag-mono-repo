//! End-to-end HTTP test for the signed-manifest serving path (M7 P3 §5.1 1B).
//!
//! Boots the REAL `GET /protocol/manifest` route on an ephemeral TCP port and drives it with an HTTP
//! client — the faithful "an app fetches the dogtag-signed discovery manifest, then verifies it
//! OFFLINE" flow. It exercises every documented branch of the serving contract in one sequential test
//! (env var `DOGTAG_MANIFEST_SIGNING_KEY` is process-global, so a single test owns it, no sibling race):
//!   * key UNSET            -> 503 "not configured"  (feature off, fail-closed)
//!   * key SET-but-malformed -> 503 "misconfigured"  (the unset-vs-malformed distinction)
//!   * key VALID            -> 200 + a manifest that OFFLINE-VERIFIES against the paired public key
//!   * unknown version       -> 404                    (fail-closed on an unrecognized version)

use axum::{routing::get, Router};
use dogtag_prover::manifest::{verify, SignedManifest};
use ed25519_dalek::SigningKey;
use vet_api::protocol::SIGNING_KEY_ENV;

#[tokio::test]
async fn manifest_route_serve_and_verify_offline() {
    // Boot the real route on an ephemeral port.
    let app = Router::new().route("/protocol/manifest", get(vet_api::protocol::get_manifest));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}/protocol/manifest");
    let client = reqwest::Client::new();

    let get = |version: &str| {
        let req = client.get(&base).query(&[("version", version)]).send();
        async move { req.await.unwrap() }
    };

    // --- 1. key UNSET -> 503 (feature intentionally disabled, fail-closed) ---------------------
    std::env::remove_var(SIGNING_KEY_ENV);
    let r = get("dogtag-levelb/1").await;
    let status_unset = r.status();
    let body_unset = r.text().await.unwrap();
    assert_eq!(status_unset, 503, "unset key must 503");
    assert!(body_unset.contains("not configured"), "body: {body_unset}");

    // --- 2. key SET-but-malformed -> 503 "misconfigured" (surfaced, not masked as unset) --------
    std::env::set_var(SIGNING_KEY_ENV, "not-a-valid-ed25519-seed");
    let r = get("dogtag-levelb/1").await;
    let status_bad = r.status();
    let body_bad = r.text().await.unwrap();
    assert_eq!(status_bad, 503, "malformed key must 503");
    assert!(body_bad.contains("misconfigured"), "body: {body_bad}");

    // --- 3. key VALID -> 200 + a manifest that verifies OFFLINE against the paired pubkey --------
    let seed = [7u8; 32];
    let signing_key = SigningKey::from_bytes(&seed);
    std::env::set_var(SIGNING_KEY_ENV, hex::encode(seed));

    let mut served = Vec::new();
    for version in ["dogtag-levela/1", "dogtag-levelb/1"] {
        let r = get(version).await;
        assert_eq!(r.status(), 200, "known version {version} must 200");
        let raw = r.text().await.unwrap();
        let sm: SignedManifest = serde_json::from_str(&raw).expect("valid SignedManifest JSON");

        // THE faithful offline check: an app pins the dogtag PUBLIC key and verifies with no RPC.
        verify(&sm, &signing_key.verifying_key())
            .unwrap_or_else(|e| panic!("served {version} manifest must verify offline: {e:?}"));
        assert_eq!(sm.content.version, version);
        assert_eq!(sm.alg, "ed25519");
        served.push((version.to_string(), raw, sm));
    }

    // --- 4. unknown version -> 404 (fail-closed) ------------------------------------------------
    let r = get("dogtag-levelc/9").await;
    let status_unknown = r.status();
    let body_unknown = r.text().await.unwrap();
    assert_eq!(status_unknown, 404, "unknown version must 404");

    // --- Emit a human-readable transcript for the evidence artifact (captured with --nocapture) --
    println!("\n================ GET /protocol/manifest — live HTTP transcript ================");
    println!("server: http://{addr}/protocol/manifest\n");
    println!("[1] key UNSET      -> HTTP {} : {}", status_unset.as_u16(), body_unset);
    println!("[2] key MALFORMED  -> HTTP {} : {}", status_bad.as_u16(), body_bad);
    println!("[4] unknown version-> HTTP {} : {}", status_unknown.as_u16(), body_unknown);
    for (version, raw, sm) in &served {
        let pretty = serde_json::to_string_pretty(
            &serde_json::from_str::<serde_json::Value>(raw).unwrap(),
        )
        .unwrap();
        println!("\n[3] key VALID, version={version} -> HTTP 200, offline-verify: PASS");
        println!("    ed25519 pubkey (app pins this): {}", sm.public_key);
        println!("{pretty}");
    }
    println!("\n=============================================================================\n");
}
