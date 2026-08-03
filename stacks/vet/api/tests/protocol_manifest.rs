//! End-to-end HTTP test for the signed-manifest serving path (M7 P3 §5.1 1B).
//!
//! Boots the REAL `GET /protocol/manifest` route on an ephemeral TCP port and drives it with an HTTP
//! client — the faithful "an app fetches the dogtag-signed discovery manifest, then verifies it
//! OFFLINE" flow. It exercises every documented branch of the serving contract in one sequential test
//! (env var `DOGTAG_MANIFEST_SIGNING_KEY` is process-global, so a single test owns it, no sibling race):
//!   * key UNSET             -> 503 "not configured"  (feature off, fail-closed)
//!   * key SET-but-malformed -> 503 "misconfigured"   (the unset-vs-malformed distinction)
//!   * DEPLOYMENT unset      -> 503 naming the variable (the on-chain half is configuration too)
//!   * key VALID             -> 200 + a manifest that OFFLINE-VERIFIES against the paired public key
//!   * unknown version       -> 404                   (fail-closed on an unrecognized version)

use axum::{routing::get, Router};
use dogtag_prover::manifest::{verify, SignedManifest};
use ed25519_dalek::SigningKey;
use vet_api::protocol::{DEPLOYMENT_ENV, SIGNING_KEY_ENV};

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

    // --- 3. key VALID but DEPLOYMENT unset -> 503 NAMING the variable ---------------------------
    // The manifest's on-chain half is configuration exactly like the key: it MIRRORS chain state, so
    // the prover crate ships no deployment constant. It used to, and every address in it had gone
    // superseded - this route would have served a signed manifest disagreeing with the chain on every
    // member. A missing variable must therefore refuse and SAY WHICH, never fill in a guess.
    let seed = [7u8; 32];
    let signing_key = SigningKey::from_bytes(&seed);
    std::env::set_var(SIGNING_KEY_ENV, hex::encode(seed));
    for var in DEPLOYMENT_ENV {
        std::env::remove_var(var);
    }
    let r = get("dogtag-levelb/1").await;
    let status_nodeploy = r.status();
    let body_nodeploy = r.text().await.unwrap();
    assert_eq!(status_nodeploy, 503, "an unconfigured deployment must 503");
    assert!(
        body_nodeploy.contains("CHAIN_ID"),
        "the refusal must NAME the missing variable, or the operator cannot act on it: {body_nodeploy}"
    );

    // Configure it, one synthetic record. Distinct per member so a field-order mistake in the
    // assembly cannot pass on two slots sharing a value.
    for (var, value) in [
        ("CHAIN_ID", "135"),
        ("FACTORY_ADDR", "0x1C9Ac2eB3f1A2D4B5C6d7E8f90A1B2C3D4e5F607"),
        ("VERIFICATION_REGISTRY_CONSENT_ADDR", "0x2B4d6f8a0c1e3a5b7d9f0e2C4a6b8d0F1E3A5c70"),
        ("SBT_CONSENT_ADDR", "0x3c5e7A9b0D2F4a6c8E0b1d3F5A7c9e0B2D4F6a80"),
        ("GROTH16_VERIFIER_CONSENT_ADDR", "0x4d6F8B0C2E4A6b8d0F2c4e6a8b0D2F4c6e8A0b90"),
    ] {
        std::env::set_var(var, value);
    }

    // --- 4. key VALID + deployment set -> 200 + a manifest that verifies OFFLINE ----------------

    let version = dogtag_standard::wrap::LEVEL_B_VERSION;
    let r = get(version).await;
    assert_eq!(r.status(), 200, "unified version must 200");
    let raw = r.text().await.unwrap();
    let sm: SignedManifest = serde_json::from_str(&raw).expect("valid SignedManifest JSON");

    // THE faithful offline check: an app pins the dogtag PUBLIC key and verifies with no RPC.
    verify(&sm, &signing_key.verifying_key())
        .unwrap_or_else(|e| panic!("served manifest must verify offline: {e:?}"));
    assert_eq!(sm.content.version, version);
    assert_eq!(sm.alg, "ed25519");

    // --- 4. unknown version -> 404 (fail-closed) ------------------------------------------------
    let r = get("dogtag-levela/1").await;
    let status_unknown = r.status();
    let body_unknown = r.text().await.unwrap();
    assert_eq!(status_unknown, 404, "unknown version must 404");
    assert!(body_unknown.contains("unknown version"), "body: {body_unknown}");

    // --- 5. S-13: RECOGNIZED but NOT SERVEABLE -> 404, and NOT "unknown version" ----------------
    // Both absences fail closed and serve nothing; what must differ is the REASON, because the
    // remedies are unrelated. A typo is the caller's to fix; `dogtag-levelb/2` is fixed by publishing
    // a discovery set to the deployed `ProtocolRegistryV2`, which carries none. Reporting the second
    // as "unknown version" sends an operator hunting a misspelling that does not exist - and naming a
    // step S-14 has already run would send them to redo it, which is the same wrong-remedy defect.
    let r = get("dogtag-levelb/2").await;
    let status_pending = r.status();
    let body_pending = r.text().await.unwrap();
    assert_eq!(status_pending, 404, "a version with no on-chain record still serves nothing");
    assert!(
        body_pending.contains("not yet deployed"),
        "a version that cannot be served must say so: {body_pending}"
    );
    assert!(
        !body_pending.contains("unknown version"),
        "must NOT collapse into the typo case: {body_pending}"
    );
    // The keys themselves, so a payload rename is caught HERE rather than by a value that silently
    // stops appearing - which is exactly how this assertion went stale once.
    assert!(
        body_pending.contains("recordedBy") && body_pending.contains("outstandingSteps"),
        "the diagnosis must carry who fills it in and what remains: {body_pending}"
    );
    // It names the REMEDY rather than only the absence, and since S-14 that remedy is PUBLICATION to
    // the deployed registry - mirrors `manifest::tests::the_awaiting_record_names_what_is_outstanding
    // _without_inventing_an_address`, which pins the same two facts on the record itself.
    assert!(
        body_pending.contains("publish") && body_pending.contains("ProtocolRegistryV2"),
        "the remedy is publication to ProtocolRegistryV2: {body_pending}"
    );
    assert!(!body_pending.contains("0x"), "no address may be invented: {body_pending}");

    // --- Emit a human-readable transcript for the evidence artifact (captured with --nocapture) --
    println!("\n================ GET /protocol/manifest — live HTTP transcript ================");
    println!("server: http://{addr}/protocol/manifest\n");
    println!("[1] key UNSET      -> HTTP {} : {}", status_unset.as_u16(), body_unset);
    println!("[2] key MALFORMED  -> HTTP {} : {}", status_bad.as_u16(), body_bad);
    println!("[4] unknown version-> HTTP {} : {}", status_unknown.as_u16(), body_unknown);
    println!("[5] undeployed ver -> HTTP {} : {}", status_pending.as_u16(), body_pending);
    let pretty = serde_json::to_string_pretty(
        &serde_json::from_str::<serde_json::Value>(&raw).unwrap(),
    )
    .unwrap();
    println!("\n[3] key VALID, version={version} -> HTTP 200, offline-verify: PASS");
    println!("    ed25519 pubkey (app pins this): {}", sm.public_key);
    println!("{pretty}");
    println!("\n=============================================================================\n");
}
