//! M-3 END-TO-END: the NETWORK LAYER, exercised the way a caller actually reaches it.
//!
//! The two existing M-3 suites each cover one half and neither bridges them:
//!   * `submit_consent_onchain.rs` drives `AlloyChain` DIRECTLY against a real registry — real
//!     Groth16 verify, real gates, but it never touches the HTTP handler.
//!   * `submit_consent_levelb_route.rs` drives the HTTP handler — real preflight, real audit row,
//!     but against `MemChain`, which verifies no proof and deploys no registry.
//!
//! M-3's stated purpose is "the network layer that carries an owner-hidden consent proof to the
//! chain". The endpoint IS that network layer, so this file runs ONE flow through BOTH halves, in
//! the order a real caller experiences it:
//!
//!   POST /v1/verify/consent      (real Groth16 proof, real operator session)
//!     -> ack `recording` + sessionId
//!     -> detached broadcast to the REAL `VerificationRegistryConsent` on anvil
//!     -> poll GET /verify/session/:id  until `recorded` + txHash
//!     -> GET /verify/history          the operator's audit trail
//!     -> the on-chain `Verified` log   OWNER-BLIND: no `subject` field exists to fill
//!
//! It runs with `--nocapture` as a readable transcript, because the headline property of Level-B is
//! not "a test is green" but "the event the chain emitted names no owner".
//!
//! Fixture + Foundry requirements are identical to `submit_consent_onchain.rs`; SKIPS if Foundry is
//! absent.

mod common;

use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{json, Value};

use vet_api::app::{AppState, Config};
use vet_api::auth::JwtKeys;
use vet_api::calendar::{MockCalendar, MockCentralClient};
use vet_api::chain::AlloyChain;
use vet_api::custody::Custody;
use vet_api::oversight::DisabledFeed;
use vet_api::store::MemStore;

const CONTRACTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../contracts");
const PK0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ACC0: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
/// anvil's well-known mnemonic — account 0 is [`ACC0`], the relayer the fixture's proof is bound to.
/// Custody MUST unlock to this exact phrase: the handler validates `pub[relayer]` against
/// `custody.active_address()`, so any other seed is a 403 preflight, not a chain revert.
const ANVIL_PHRASE: &str = "test test test test test test test test test test test junk";
/// The tag is minted to a neutral custodian, never the owner.
const CUSTODIAN: &str = "0x00000000000000000000000000000000c0ffee00";
/// Anvil account 1 — the PROVIDER's key, deliberately distinct from the registrar's `ACC0`.
const PK1: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const ACC1: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
/// A registrar-issued `bytes20` provider id. Arbitrary and permanent by design — it means nothing.
const PROVIDER_ID: &str = "0x00000000000000000000000000000000000000a1";
const IDENTITY_DIGEST: &str =
    "0x1111111111111111111111111111111111111111111111111111111111111111";
/// `ProviderRegistry.Standing.ACTIVE`.
const STANDING_ACTIVE: &str = "2";

// ------------------------------------------------------------------------------------------------
// anvil / forge harness (mirrors submit_consent_onchain.rs)
// ------------------------------------------------------------------------------------------------

struct Anvil {
    child: Child,
    port: u16,
}
impl Anvil {
    fn rpc(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}
impl Drop for Anvil {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn have_foundry() -> bool {
    ["forge", "cast", "anvil"].iter().all(|b| {
        Command::new(b)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

fn pick_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}

fn cast_ok(rpc: &str, args: &[&str]) -> bool {
    Command::new("cast")
        .args(args)
        .args(["--rpc-url", rpc])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn start_anvil() -> Anvil {
    let port = pick_port();
    let child = Command::new("anvil")
        .args(["--chain-id", "135", "--port", &port.to_string(), "--silent"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn anvil");
    let anvil = Anvil { child, port };
    for _ in 0..50 {
        if cast_ok(&anvil.rpc(), &["block-number"]) {
            return anvil;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("anvil did not come up");
}

fn run(cmd: &mut Command) -> String {
    let out = cmd
        .current_dir(CONTRACTS_DIR)
        .output()
        .expect("run command");
    if !out.status.success() {
        panic!(
            "command failed: {:?}\nstdout: {}\nstderr: {}",
            cmd,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn forge_create(rpc: &str, contract: &str, args: &[&str]) -> String {
    let mut cmd = Command::new("forge");
    cmd.args([
        "create",
        "--rpc-url",
        rpc,
        "--private-key",
        PK0,
        "--broadcast",
        contract,
    ]);
    if !args.is_empty() {
        cmd.arg("--constructor-args").args(args);
    }
    let out = run(&mut cmd);
    out.lines()
        .find(|l| l.contains("Deployed to:"))
        .and_then(|l| l.split_whitespace().last())
        .unwrap_or_else(|| panic!("no Deployed to in: {out}"))
        .to_string()
        .to_lowercase()
}

fn cast_send(rpc: &str, to: &str, sig: &str, args: &[&str]) {
    cast_send_as(rpc, PK0, to, sig, args);
}

/// `cast send` from an explicit key.
///
/// The two-key split in the provider journey is REAL and is preserved rather than collapsed:
/// `createIssuer` and `repointService` are the provider's own transactions while every registrar
/// call is `onlyOwner`. Running both halves from one key would let this suite assert a journey no
/// provider can actually walk.
fn cast_send_as(rpc: &str, pk: &str, to: &str, sig: &str, args: &[&str]) {
    let mut cmd = Command::new("cast");
    cmd.args(["send", "--rpc-url", rpc, "--private-key", pk, to, sig])
        .args(args);
    run(&mut cmd);
}

/// The launch-set addresses, as the real deploy script reports them.
struct Deployed {
    provider_registry: String,
    factory: String,
    sbt: String,
    verification_registry: String,
}

/// Stand up the launch set, mirroring `script/Deploy.s.sol::deploy` — including
/// `addFactoryGeneration`, WITHOUT which `attachService` reverts on an unregistered generation.
///
/// Deliberately NOT driven by running that script. `Deploy.s.sol::run` ends in `_writeLedger`, which
/// writes `contracts/deployments/roax.json` unconditionally — the repo's single source of truth for
/// every address — so a test that ran it would overwrite the real ROAX ledger with anvil's
/// deterministic addresses on every run. Measured, not theorised: a probe run did exactly that.
/// The script itself is separately covered where it can be driven safely, by
/// `contracts/test/LaunchStack.sol`, which calls the ledger-free `deploy()` entry point directly and
/// backs `Deploy.t.sol`, `ConsentRegistry.t.sol` and `CustodialIssuance.t.sol` under `forge test`.
fn deploy_launch_contracts(rpc: &str) -> Deployed {
    let core = forge_create(rpc, "src/ProviderRegistry.sol:ProviderRegistry", &[ACC0]);
    let issuer_impl = forge_create(rpc, "src/DogTagIssuer.sol:DogTagIssuer", &[]);
    let factory = forge_create(
        rpc,
        "src/DogTagIssuerFactory.sol:DogTagIssuerFactory",
        &[&issuer_impl, &core],
    );
    // `mintCustodial` has no recipient — the tag goes to the immutable custodian.
    let sbt = forge_create(
        rpc,
        "src/DogTagSBTConsent.sol:DogTagSBTConsent",
        &[ACC0, CUSTODIAN],
    );
    let verifier = forge_create(
        rpc,
        "src/Groth16VerifierConsent.sol:Groth16VerifierConsent",
        &[],
    );
    // VerificationRegistryConsent(core, sbt, zk, rootIndex(=factory), admin).
    let verification_registry = forge_create(
        rpc,
        "src/VerificationRegistryConsent.sol:VerificationRegistryConsent",
        &[&core, &sbt, &verifier, &factory, ACC0],
    );

    // The wiring that makes the above a system, and the one step no previous version of this harness
    // had: a factory in no generation cannot have a service attached under it.
    cast_send(
        rpc,
        &core,
        "addFactoryGeneration(bytes32,address)",
        &[&factory_generation(), &factory],
    );
    assert_eq!(
        cast_call(
            rpc,
            &core,
            "generationOfFactory(address)(bytes32)",
            &[&factory]
        ),
        factory_generation(),
        "the core must carry this factory's generation"
    );

    Deployed {
        provider_registry: core,
        factory,
        sbt,
        verification_registry,
    }
}

/// Walk the provider journey end to end and return a clone that can actually anchor.
///
/// `canIssue` folds SIX registrar facts — a registered provider in ACTIVE standing, a service
/// attached under a registered factory generation, a confirmed live owner, an effective service
/// standing, that service being the provider's CURRENT pointer for its record type, and an issuance
/// grant for the signer. Missing any one of them reverts the anchor rather than failing a check the
/// test can see, so all of them are performed here. `repointService` is the one most easily missed:
/// a live ROAX walk recorded `canRevoke` and `isRecognizedIssuer` already true while `canIssue`
/// stayed false until the provider designated its clone.
fn onboard_issuing_clone(rpc: &str, d: &Deployed, record_type: &str, signer: &str) -> String {
    let core = &d.provider_registry;
    cast_send(
        rpc,
        core,
        "registerProvider(bytes20,address,bytes32,uint32,uint16,uint8,bytes)",
        &[PROVIDER_ID, ACC1, IDENTITY_DIGEST, "1", "0xe3", "0x12", "0x"],
    );
    cast_send(
        rpc,
        core,
        "setProviderStanding(bytes20,uint8)",
        &[PROVIDER_ID, STANDING_ACTIVE],
    );
    cast_send(
        rpc,
        core,
        "setServiceCreationApproval(bytes20,bytes32,bool)",
        &[PROVIDER_ID, record_type, "true"],
    );

    // The provider deploys its own clone, from its own key.
    cast_send_as(
        rpc,
        PK1,
        &d.factory,
        "createIssuer(bytes20,bytes32,uint96)",
        &[PROVIDER_ID, record_type, "0"],
    );
    let clone = cast_call(
        rpc,
        &d.factory,
        "predictIssuer(bytes32,address,uint96)(address)",
        &[record_type, ACC1, "0"],
    )
    .to_lowercase();

    cast_send(
        rpc,
        core,
        "attachService(bytes20,address,bytes32,address)",
        &[PROVIDER_ID, &clone, &factory_generation(), ACC1],
    );
    cast_send(
        rpc,
        core,
        "setServiceStanding(address,uint8)",
        &[&clone, STANDING_ACTIVE],
    );
    // The grant is on the SIGNER's ADDRESS and names no service - `RIGHT_ISSUE` is bit 0 of the
    // rights bitmask. The `repointService` below still matters: `canIssue` folds this clone's own
    // lifecycle terms, none of which the re-keying dropped.
    cast_send(rpc, core, "setRights(address,uint256)", &[signer, "1"]);
    // The provider's CURRENT pointer for this record type — the provider's own decision, so its own
    // transaction. Without it the clone is attached and granted and still anchors nothing.
    cast_send_as(rpc, PK1, core, "repointService(address)", &[&clone]);

    assert_eq!(
        cast_call(rpc, core, "canIssue(address,address)(bool)", &[&clone, signer]),
        "true",
        "the journey must leave the signer able to anchor on this clone"
    );
    clone
}

/// `keccak256("dogtag-issuer-factory/1")` — the generation `Deploy.s.sol` registers its factory under.
fn factory_generation() -> String {
    format!(
        "0x{}",
        hex::encode(alloy::primitives::keccak256(b"dogtag-issuer-factory/1").0)
    )
}


fn cast_call(rpc: &str, to: &str, sig: &str, args: &[&str]) -> String {
    let mut cmd = Command::new("cast");
    cmd.args(["call", "--rpc-url", rpc, to, sig]).args(args);
    run(&mut cmd).trim().to_string()
}

// ------------------------------------------------------------------------------------------------
// fixture
// ------------------------------------------------------------------------------------------------

struct Fixture {
    proof: Value,
    pubs: [String; 7],
    dog_tag_id: String,
    purpose: String,
    root: String,
    record_type: String,
    /// The hidden owner. MUST NOT appear on-chain anywhere.
    owner_address: String,
}

fn load_fixture() -> Fixture {
    let path = format!("{CONTRACTS_DIR}/test/consent-fixture-anvil.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("read {path}: {e} (regenerate — see submit_consent_onchain.rs)")
    });
    let j: Value = serde_json::from_str(&raw).expect("fixture JSON");
    let s = |v: &Value| v.as_str().expect("fixture string").to_string();
    let pv = j["pub"].as_array().expect("pub: !array");
    assert_eq!(
        pv.len(),
        dogtag_standard::public_signals::NUM_PUBLIC,
        "fixture pub must be 7 signals"
    );
    Fixture {
        // The exact JSON body a client posts: the snarkjs proof shape, verbatim from the fixture.
        proof: json!({ "a": j["a"], "b": j["b"], "c": j["c"], "pubSignals": j["pub"] }),
        pubs: std::array::from_fn(|i| s(&pv[i])),
        dog_tag_id: s(&j["dogTagId"]),
        purpose: s(&j["purpose"]),
        root: s(&j["R"]),
        record_type: s(&j["recordType"]),
        owner_address: s(&j["_ownerAddress"]),
    }
}

struct Stack {
    registry: String,
    sbt: String,
    verification_consent: String,
    clone: String,
}

/// Deploy the Level-B contract set and establish the on-chain state the fixture's `pub[]` demands,
/// then perform M-2's issuance (`issue(R)` FIRST, then `mintCustodial`).
fn deploy_and_issue(rpc: &str, f: &Fixture) -> Stack {
    let d = deploy_launch_contracts(rpc);

    // Issuance is SERVICE-scoped: `DogTagIssuer.issue` is gated by
    // `ProviderRegistry.canIssue(address(this), msg.sender)`, keyed on the clone rather than on a
    // record type, so a clone anchors nothing until the registrar has walked the whole journey and
    // its provider has designated it.
    let clone = onboard_issuing_clone(rpc, &d, &f.record_type, ACC0);

    // The relayer must hold the orthogonal VERIFY-axis capability for this purpose — the SAME read
    // the handler's preflight performs before it will spend gas. `setVerifierCapability` takes the
    // RAW purpose and derives `verificationKey` itself; handing it an already-derived key would
    // derive twice and grant a capability `canVerify` never reads.
    cast_send(
        rpc,
        &d.provider_registry,
        "setVerifierCapability(bytes32,address,bool)",
        &[&f.purpose, ACC0, "true"],
    );

    let (registry, sbt, verification_consent) =
        (d.provider_registry, d.sbt, d.verification_registry);
    let issuer_role = cast_call(rpc, &sbt, "ISSUER_ROLE()(bytes32)", &[]);
    cast_send(
        rpc,
        &sbt,
        "grantRole(bytes32,address)",
        &[&issuer_role, ACC0],
    );

    // M-2 issuance, in the mandated order.
    cast_send(rpc, &clone, "issue(bytes32)", &[&f.root]);
    cast_send(
        rpc,
        &sbt,
        "mintCustodial(uint256,bytes32)",
        &[&f.dog_tag_id, &f.root],
    );

    Stack {
        registry,
        sbt,
        verification_consent,
        clone,
    }
}

/// An `AppState` wired to the real owner-hidden stack on anvil.
fn state_for(rpc: &str, st: &Stack) -> AppState {
    let cfg = Config {
        deployment_url: "http://localhost:41874".to_string(),
        rpc_url: rpc.to_string(),
        issuer_registry_addr: st.registry.clone(),
        factory_addr: String::new(),
        verification_registry_consent_addr: st.verification_consent.clone(),
        issuer_addrs: std::collections::HashMap::new(),
        issuer_name: "DogTag Vet".to_string(),
        issuer_domain: "vet.example".to_string(),
        sbt_consent_addr: st.sbt.clone(),
        profile_issuer_addr: st.clone.clone(),
        vet_signer_index: 0,
        operator_password: common::OPERATOR_PW.to_string(),
        admin_password: common::ADMIN_PW.to_string(),
        confirmations: 1,
        business_id: common::BUSINESS_ID.to_string(),
        business_type: common::BUSINESS_TYPE.to_string(),
        central_hmac_secret: common::CENTRAL_HMAC_SECRET.to_string(),
        custody_seal_path: None,
    };
    AppState {
        store: Arc::new(MemStore::new()),
        chain: Arc::new(AlloyChain::new(rpc.to_string())),
        consent_prover: Arc::new(vet_api::prover::ConsentProver::disabled()),
        calendar: Arc::new(MockCalendar::new()),
        central: Arc::new(MockCentralClient::new()),
        custody: Custody::new(),
        jwt: JwtKeys::generate(),
        cfg: Arc::new(cfg),
        ratelimit: Arc::new(vet_api::auth::RateLimiter::new()),
        feed: Arc::new(DisabledFeed),
    }
}

fn hr(title: &str) {
    println!(
        "\n─── {title} {}",
        "─".repeat(64usize.saturating_sub(title.len()))
    );
}

// ------------------------------------------------------------------------------------------------

/// THE bridging acceptance: a real owner-hidden consent proof travels the whole network layer —
/// HTTP endpoint, preflight, audit row, detached broadcast — and lands as an OWNER-BLIND `Verified`
/// on a real `VerificationRegistryConsent`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_real_consent_proof_travels_the_endpoint_to_the_chain_owner_blind() {
    if !have_foundry() {
        eprintln!("SKIP a_real_consent_proof_travels_...: Foundry not on PATH");
        return;
    }
    let anvil = start_anvil();
    let rpc = anvil.rpc();
    let f = load_fixture();

    hr("1. deploy the Level-B stack + issue the tag (M-2)");
    let stack = deploy_and_issue(&rpc, &f);
    println!(
        "  VerificationRegistryConsent : {}",
        stack.verification_consent
    );
    println!("  DogTagSBTConsent            : {}", stack.sbt);
    println!("  DogTagIssuer clone          : {}", stack.clone);
    println!("  custodian (D1, NOT the owner): {CUSTODIAN}");
    let onchain_root = cast_call(
        &rpc,
        &stack.sbt,
        "profileRoot(uint256)(bytes32)",
        &[&f.dog_tag_id],
    );
    println!("  profileRoot(dogTagId)       : {onchain_root}");
    println!("  proof R (pub[4])            : {}", f.root);
    assert_eq!(onchain_root, f.root, "profileRoot must equal the proof's R");

    hr("2. boot vet-api, log in as operator, unlock relayer custody");
    let st = state_for(&rpc, &stack);
    // Seed the custody seal with anvil's own mnemonic so account 0 IS the relayer the proof names —
    // the handler validates `pub[relayer]` against `custody.active_address()`, so any other seed
    // would be a 403 preflight rather than a chain interaction. This is exactly the blob
    // `POST /admin/genesis/confirm` writes; the unlock below then goes through the REAL route,
    // which is also what hands the derived signer to the chain client.
    let signer0 = vet_api::custody::derive_account(ANVIL_PHRASE, 0).expect("derive account 0");
    let addr0 = format!("{:#x}", signer0.address());
    assert!(
        addr0.eq_ignore_ascii_case(ACC0),
        "custody account 0 must be the relayer the proof names, got {addr0}"
    );
    st.store
        .put_custody(vet_api::store::CustodyBlob {
            encrypted_seed: vet_api::custody::encrypt_seed(ANVIL_PHRASE, "unlock-pw")
                .expect("encrypt seed"),
            meta: vet_api::store::KeystoreMeta {
                accounts: vec![vet_api::store::AccountMeta {
                    index: 0,
                    address: addr0.clone(),
                    label: "account0".to_string(),
                }],
                state: "initialized".to_string(),
            },
        })
        .await;

    let app = vet_api::router(st);
    let (s, b) = common::call(
        &app,
        "POST",
        "/admin/login",
        None,
        Some(json!({"password": common::ADMIN_PW})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "admin login: {b}");
    let admin = b["token"].as_str().expect("admin token").to_string();
    let (s, b) = common::call(
        &app,
        "POST",
        "/admin/unlock",
        Some(&admin),
        Some(json!({"passphrase": "unlock-pw"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "custody unlock: {b}");
    println!(
        "  POST /admin/unlock -> {}",
        serde_json::to_string(&b).unwrap()
    );
    let relayer = b["accounts"][0]["address"]
        .as_str()
        .expect("unlocked relayer")
        .to_string();

    let (s, b) = common::call(
        &app,
        "POST",
        "/login",
        None,
        Some(json!({"password": common::OPERATOR_PW})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "operator login: {b}");
    let op = b["token"].as_str().expect("operator token").to_string();
    println!("  relayer (custody account 0) : {relayer}");
    println!("  operator session            : authenticated");

    hr("3. POST /v1/verify/consent  (real Groth16 proof)");
    println!(
        "  request  : {}",
        serde_json::to_string(&json!({
            "proof": {"a": "…", "b": "…", "c": "…", "pubSignals": f.pubs}
        }))
        .unwrap()
    );
    let (s, ack) = common::call(
        &app,
        "POST",
        "/v1/verify/consent",
        Some(&op),
        Some(json!({ "proof": f.proof })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "owner-hidden submit: {ack}");
    println!("  {s}");
    println!(
        "  response : {}",
        serde_json::to_string_pretty(&ack).unwrap()
    );
    assert_eq!(ack["status"], "recording", "the ack is not terminal");
    let session_id = ack["sessionId"].as_str().expect("sessionId").to_string();

    hr("4. GET /verify/session/{id}  — poll the detached broadcast to a terminal state");
    let mut row = Value::Null;
    for _ in 0..600 {
        let (ss, r) = common::call(
            &app,
            "GET",
            &format!("/verify/session/{session_id}"),
            Some(&op),
            None,
        )
        .await;
        assert_eq!(ss, StatusCode::OK, "session lookup: {r}");
        if r["status"] != "recording" {
            row = r;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    println!("  {}", serde_json::to_string_pretty(&row).unwrap());
    assert_eq!(
        row["status"], "recorded",
        "the submission must settle as recorded (tx_hash carries the revert reason on error)"
    );
    let tx_hash = row["txHash"].as_str().expect("txHash on a recorded row");

    hr("5. GET /verify/history  — the operator's audit trail");
    let (s, hist) = common::call(&app, "GET", "/verify/history", Some(&op), None).await;
    assert_eq!(s, StatusCode::OK, "history: {hist}");
    println!("  {}", serde_json::to_string_pretty(&hist).unwrap());
    let rows = hist["verifications"].as_array().expect("history rows");
    let mine = rows
        .iter()
        .find(|r| r["sessionId"] == session_id.as_str())
        .expect("this verification is in the operator's history");
    assert_eq!(mine["status"], "recorded");
    // Owner-blind by construction: there is no field a subject could occupy.
    assert!(
        mine.get("subject").is_none(),
        "an owner-hidden audit row must have no subject slot: {mine}"
    );

    hr("6. the on-chain Verified log — OWNER-BLIND");
    let receipt_logs = run(Command::new("cast").args(["receipt", tx_hash, "--rpc-url", &rpc]));
    let status_line = receipt_logs
        .lines()
        .find(|l| l.starts_with("status"))
        .unwrap_or("status<unknown>");
    println!("  tx {tx_hash}  {status_line}");

    // Fetch the raw log, then render it as the DECODED field list — that list is the artifact the
    // whole milestone is about: six fields, and none of them names an owner.
    const VERIFIED_SIG: &str = "Verified(uint256,address,bytes32,bytes32,uint256,uint256)";
    let decoded = run(Command::new("cast").args([
        "logs",
        "--rpc-url",
        &rpc,
        "--address",
        &stack.verification_consent,
        "--from-block",
        "0",
        VERIFIED_SIG,
    ]));
    let field = |k: &str| -> String {
        decoded
            .lines()
            .find(|l| l.trim_start().starts_with(k))
            .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
            .unwrap_or_default()
    };
    let topics: Vec<String> = decoded
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("topics"))
        .filter_map(|l| l.trim().strip_prefix("0x").map(|h| format!("0x{h}")))
        .collect();
    assert_eq!(
        topics.len(),
        3,
        "expected topic0 + 2 indexed args:\n{decoded}"
    );
    let data = field("data");
    let word = |i: usize| format!("0x{}", &data.trim_start_matches("0x")[i * 64..(i + 1) * 64]);
    println!("  owner-hidden {VERIFIED_SIG}");
    println!("    topic0    (event sig)  : {}", topics[0]);
    println!("    dogTagId  (indexed)    : {}", topics[1]);
    println!("    relayer   (indexed)    : {}", topics[2]);
    println!("    purpose                : {}", word(0));
    println!("    nullifier              : {}", word(1));
    println!("    deadline               : {}", word(2));
    println!("    ts                     : {}", word(3));
    println!("    subject                : <NO SUCH FIELD — owner remains hidden>");

    // The hidden owner appears nowhere: not in the log data, not in the submitted calldata.
    let owner_bare = f.owner_address.trim_start_matches("0x").to_lowercase();
    assert!(
        !decoded.to_lowercase().contains(&owner_bare),
        "the owner address must never appear in the on-chain Verified log"
    );
    let tx_input = run(Command::new("cast").args(["tx", tx_hash, "input", "--rpc-url", &rpc]));
    assert!(
        !tx_input.to_lowercase().contains(&owner_bare),
        "the owner address must never appear in the submitted calldata"
    );
    println!(
        "  hidden owner {} : absent from log AND from calldata ✓",
        f.owner_address
    );

    hr("7. the nullifier is consumed on-chain (pub[3], not pub[4]=R)");
    use dogtag_standard::public_signals::level_b as PB;
    let nullifier = row["nullifier"].as_str().expect("nullifier on the row");
    let consumed = cast_call(
        &rpc,
        &stack.verification_consent,
        "consumed(bytes32)(bool)",
        &[nullifier],
    );
    println!("  consumed({nullifier}) = {consumed}");
    assert_eq!(consumed, "true", "the nullifier must be consumed");
    // Guard the E-1 slot confusion from the far side: pub[4] is R, and it must NOT be consumed.
    let r_word = {
        use alloy::primitives::U256;
        let u = U256::from_str_radix(f.pubs[PB::ROOT].trim_start_matches("0x"), 16)
            .or_else(|_| U256::from_str_radix(&f.pubs[PB::ROOT], 10))
            .expect("R parses");
        format!("0x{}", hex::encode(u.to_be_bytes::<32>()))
    };
    let r_consumed = cast_call(
        &rpc,
        &stack.verification_consent,
        "consumed(bytes32)(bool)",
        &[&r_word],
    );
    println!("  consumed(R = pub[4])        = {r_consumed}  (must be false)");
    assert_eq!(
        r_consumed, "false",
        "R must never be treated as the nullifier (E-1)"
    );

    hr("8. replay through the endpoint is refused");
    let (s2, ack2) = common::call(
        &app,
        "POST",
        "/v1/verify/consent",
        Some(&op),
        Some(json!({ "proof": f.proof })),
    )
    .await;
    println!("  POST /v1/verify/consent (same proof again) -> {s2}");
    let mut row2 = Value::Null;
    if s2 == StatusCode::OK {
        let sid2 = ack2["sessionId"].as_str().expect("sessionId").to_string();
        for _ in 0..600 {
            let (_, r) = common::call(
                &app,
                "GET",
                &format!("/verify/session/{sid2}"),
                Some(&op),
                None,
            )
            .await;
            if r["status"] != "recording" {
                row2 = r;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        println!("  {}", serde_json::to_string_pretty(&row2).unwrap());
        assert_eq!(
            row2["status"], "error",
            "a replayed nullifier must settle as an error row"
        );
        let reason = row2["txHash"].as_str().unwrap_or_default().to_lowercase();
        assert!(
            reason.contains("replay"),
            "the audit row must carry the on-chain revert reason, got {reason}"
        );
    }

    println!("\n─── M-3 end-to-end: PASS ───────────────────────────────────────────────\n");
}
