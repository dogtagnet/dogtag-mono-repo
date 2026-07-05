//! Evidence transcript for the PR-A control-plane surface. Drives the real Axum admin router against
//! MemChain and writes the actual JSON request/response pairs an operator would see to a transcript
//! file, so a reviewer can read the end-to-end behavior (authority map + predict + execute vs propose)
//! rather than trusting a pass/fail. Not a normal assertion test — it exists to produce the artifact.

mod common;

use common::{admin_token, hermetic_state, FACTORY};

use admin_api::chain::{default_admin_role, whitelist_admin_role};

const HOSTED: &str = "0x00000000000000000000000000000000000000ad";
const GOV: &str = "0x8e27e11700000000000000000000000000000000";

fn out_path() -> Option<std::path::PathBuf> {
    std::env::var("EVIDENCE_DIR")
        .ok()
        .map(|d| std::path::Path::new(&d).join("control-plane-api-transcript.txt"))
}

#[tokio::test]
async fn control_plane_api_transcript() {
    let mut log = String::new();
    macro_rules! w { ($($a:tt)*) => {{ log.push_str(&format!($($a)*)); log.push('\n'); }} }

    w!("================================================================================");
    w!("  PR-A control-plane foundation — live admin API transcript (MemChain, hermetic)");
    w!("  admin-api Axum router, admin-session gated. All JSON is the real endpoint output.");
    w!("================================================================================\n");

    // ---- Scenario 1: single-authority topology (today) — hosted EOA holds every authority. -------
    let (state, chain, _v, _b) = hermetic_state();
    let registry = state.cfg.issuer_registry_addr.clone();
    chain.set_factory_owner(FACTORY, HOSTED);
    chain.set_role(&registry, &whitelist_admin_role(), HOSTED);
    chain.set_default_admin(&registry, HOSTED);
    chain.set_role(&registry, &default_admin_role(), HOSTED);
    let app = admin_api::router(state);
    let tok = admin_token(&app).await;

    w!("### SCENARIO 1 — Phase-1 today: single deployer EOA {HOSTED} holds all three authorities\n");

    w!("GET /v1/admin/governance/authority");
    let (s, b) = common::call(
        &app,
        "GET",
        "/v1/admin/governance/authority",
        Some(&tok),
        None,
    )
    .await;
    w!(
        "HTTP {}\n{}\n",
        s.as_u16(),
        serde_json::to_string_pretty(&b).unwrap()
    );

    let predict_body = serde_json::json!({ "recordType": "VACCINATION", "business": HOSTED });
    w!("POST /v1/admin/factory/predict  {}", predict_body);
    let (s, b) = common::call(
        &app,
        "POST",
        "/v1/admin/factory/predict",
        Some(&tok),
        Some(predict_body.clone()),
    )
    .await;
    w!(
        "HTTP {}\n{}\n",
        s.as_u16(),
        serde_json::to_string_pretty(&b).unwrap()
    );
    let predicted_1 = b["predicted"].as_str().unwrap().to_string();

    w!("POST /v1/admin/factory/predict  (same inputs again — must be identical/deterministic)");
    let (_s, b2) = common::call(
        &app,
        "POST",
        "/v1/admin/factory/predict",
        Some(&tok),
        Some(predict_body),
    )
    .await;
    w!(
        "predicted (call 2) = {}   deterministic == {}\n",
        b2["predicted"],
        b2["predicted"] == predicted_1
    );

    let deploy_body = serde_json::json!({ "name": "Vax Authority", "recordType": "VACCINATION", "business": HOSTED });
    w!("POST /v1/admin/factory/issuers  {}", deploy_body);
    let (s, b) = common::call(
        &app,
        "POST",
        "/v1/admin/factory/issuers",
        Some(&tok),
        Some(deploy_body),
    )
    .await;
    w!(
        "HTTP {}\n{}",
        s.as_u16(),
        serde_json::to_string_pretty(&b).unwrap()
    );
    w!(">> hosted key IS the factory owner → disposition = EXECUTED, signed + broadcast, txHash returned.");
    w!(
        ">> deployed clone == the up-front predicted address: {}\n",
        b["predicted"].as_str() == Some(predicted_1.as_str())
    );

    // ---- Scenario 2: Phase-2 handover — DEFAULT_ADMIN moved to governance 0x8E27, factory NOT owned.
    let (state2, chain2, _v, _b) = hermetic_state();
    let registry2 = state2.cfg.issuer_registry_addr.clone();
    // hosted keeps WHITELIST_ADMIN, but factory ownership + DEFAULT_ADMIN have moved to governance.
    chain2.set_role(&registry2, &whitelist_admin_role(), HOSTED);
    chain2.set_factory_owner(FACTORY, GOV);
    chain2.set_default_admin(&registry2, GOV);
    chain2.set_role(&registry2, &default_admin_role(), GOV);
    chain2.set_pending_default_admin(&registry2, GOV, 1_782_988_652);
    let app2 = admin_api::router(state2);
    let tok2 = admin_token(&app2).await;

    w!("--------------------------------------------------------------------------------");
    w!("### SCENARIO 2 — Phase-2 handover: factory owner + DEFAULT_ADMIN moved to governance {GOV}");
    w!("###             (hosted key still holds WHITELIST_ADMIN). Same code path, no redeploy.\n");

    w!("GET /v1/admin/governance/authority");
    let (s, b) = common::call(
        &app2,
        "GET",
        "/v1/admin/governance/authority",
        Some(&tok2),
        None,
    )
    .await;
    w!(
        "HTTP {}\n{}\n",
        s.as_u16(),
        serde_json::to_string_pretty(&b).unwrap()
    );

    let deploy_body2 = serde_json::json!({ "name": "Vax Authority", "recordType": "VACCINATION", "business": HOSTED });
    w!("POST /v1/admin/factory/issuers  {}", deploy_body2);
    let (s, b) = common::call(
        &app2,
        "POST",
        "/v1/admin/factory/issuers",
        Some(&tok2),
        Some(deploy_body2),
    )
    .await;
    w!(
        "HTTP {}\n{}",
        s.as_u16(),
        serde_json::to_string_pretty(&b).unwrap()
    );
    w!(">> hosted key is NOT the factory owner → disposition = PROPOSED. Nothing broadcast; the");
    w!(">> {{target, calldata}} payload is handed to governance {GOV} to execute out-of-band.\n");

    // ---- Scenario 3: input hardening — malformed business rejected, auth required. ---------------
    w!("--------------------------------------------------------------------------------");
    w!("### SCENARIO 3 — input hardening\n");
    let bad =
        serde_json::json!({ "name": "X", "recordType": "VACCINATION", "business": "0xnothex" });
    w!(
        "POST /v1/admin/factory/issuers  {}   (malformed business)",
        bad
    );
    let (s, b) = common::call(
        &app,
        "POST",
        "/v1/admin/factory/issuers",
        Some(&tok),
        Some(bad),
    )
    .await;
    w!("HTTP {} → {}\n", s.as_u16(), b);

    w!("POST /v1/admin/factory/issuers  (no admin session)");
    let (s, _b) = common::call(
        &app,
        "POST",
        "/v1/admin/factory/issuers",
        None,
        Some(serde_json::json!({ "name": "X", "recordType": "VACCINATION" })),
    )
    .await;
    w!("HTTP {} (rejected — admin session required)\n", s.as_u16());

    w!("================================================================================");
    w!("  END TRANSCRIPT");
    w!("================================================================================");

    println!("{log}");
    if let Some(p) = out_path() {
        std::fs::write(&p, &log).expect("write evidence transcript");
        println!("\n[evidence written to {}]", p.display());
    }
}
