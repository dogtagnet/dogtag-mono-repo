//! Phase-4 acceptance (anvil): issuer-application approval writes the on-chain whitelist for EACH
//! (address, recordType) pair via the admin signer (multi-address one-to-many), with a mock DnsChecker
//! returning ok. Then delist one and assert `isWhitelistedFor` flips to false.
//!
//! Requires `forge`, `cast`, `anvil` (Foundry). SKIPS cleanly if absent (the hermetic `central.rs`
//! suite is the always-on equivalent) — harness mirrors `stacks/vet/api/tests/flow.rs`.

mod common;

use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use axum::http::StatusCode;
use common::*;

use admin_api::app::{AppState, Config};
use admin_api::auth::JwtKeys;
use admin_api::business::{BusinessClient, ReqwestBusinessClient};
use admin_api::chain::{record_type_key, AlloyChain, ChainClient};
use admin_api::crypto::{KeyVault, MemVault};
use admin_api::dns::{DnsChecker, MockDnsChecker};
use admin_api::store::MemStore;

const CONTRACTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../contracts");
// anvil default account 0 (admin/deployer == WHITELIST_ADMIN).
const PK0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ACC0: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
// two issuer signer addresses (anvil accounts 1 and 2) — one issuer ENTITY, many addresses.
const ADDR1: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const ADDR2: &str = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";

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
}

fn cast_call(rpc: &str, to: &str, sig: &str, args: &[&str]) -> String {
    let mut cmd = Command::new("cast");
    cmd.args(["call", "--rpc-url", rpc, to, sig]).args(args);
    run(&mut cmd).trim().to_string()
}

/// Build an AppState wired to a REAL AlloyChain on anvil with the admin signer registered at index 0
/// (ACC0 == the registry's WHITELIST_ADMIN), a MockDnsChecker(ok), MemStore + MemVault.
async fn anvil_state(rpc: &str, registry: &str, sbt: &str) -> AppState {
    let chain = AlloyChain::new(rpc.to_string());
    // register ACC0's key as the admin signer (index 0).
    let pk: [u8; 32] = hex::decode(PK0.trim_start_matches("0x"))
        .unwrap()
        .try_into()
        .unwrap();
    chain.register_signer(0, pk, ACC0.to_lowercase()).await;
    let cfg = Config {
        deployment_url: "http://localhost:39742".to_string(),
        rpc_url: rpc.to_string(),
        issuer_registry_addr: registry.to_lowercase(),
        sbt_addr: sbt.to_lowercase(),
        issuer_name: "DogTag Central".to_string(),
        issuer_domain: "dogtag.example".to_string(),
        profile_document_store: sbt.to_lowercase(),
        admin_password_hash: admin_api::auth::hash_password(ADMIN_PW),
        admin_signer_index: 0,
    };
    AppState {
        store: Arc::new(MemStore::new()),
        chain: Arc::new(chain) as Arc<dyn ChainClient>,
        dns: Arc::new(MockDnsChecker::ok()) as Arc<dyn DnsChecker>,
        business: Arc::new(ReqwestBusinessClient::new()) as Arc<dyn BusinessClient>,
        vault: Arc::new(MemVault::new()) as Arc<dyn KeyVault>,
        jwt: JwtKeys::generate(),
        cfg: Arc::new(cfg),
        ratelimit: Arc::new(admin_api::auth::RateLimiter::new()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn anvil_whitelist_multi_address() {
    if !have_foundry() {
        eprintln!("SKIP anvil_whitelist_multi_address: Foundry (forge/cast/anvil) not on PATH");
        return;
    }
    run(Command::new("forge").args(["build"]));

    let anvil = start_anvil();
    let rpc = anvil.rpc();

    // deploy the contract set (IssuerRegistry with ACC0 as admin multisig -> WHITELIST_ADMIN).
    let registry = forge_create(&rpc, "src/IssuerRegistry.sol:IssuerRegistry", &[ACC0]);
    let sbt = forge_create(&rpc, "src/DogTagSBT.sol:DogTagSBT", &[ACC0]);
    // (factory/impl are part of the issuance set; not needed for the whitelist assertions.)

    let rt_vacc = record_type_key("VACCINATION");
    let rt_titer = record_type_key("TITER");

    let state = anvil_state(&rpc, &registry, &sbt).await;
    let app = admin_api::router(state);
    let admin = admin_token(&app).await;

    // submit an issuer-application with MULTIPLE addresses + MULTIPLE recordTypes (one entity -> many).
    let (s, b) = call(
        &app,
        "POST",
        "/v1/issuer-applications",
        None,
        Some(serde_json::json!({
            "issuerEntityId": "vet-entity-1",
            "addresses": [ADDR1, ADDR2],
            "recordTypes": ["VACCINATION", "TITER"],
            "domain": "biz.example",
            "documentStore": "0x00000000000000000000000000000000000000cc",
            "usdaNan": "123456",
            "license": { "number": "L-1", "jurisdiction": "CA", "expiry": "2030-01-01" }
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "submit application: {b}");
    let app_id = b["applicationId"].as_str().unwrap().to_string();

    // approve (mock DnsChecker returns ok) -> admin signer writes whitelistFor for EACH pair.
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/issuer-applications/{app_id}/approve"),
        Some(&admin),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "approve: {b}");
    // 2 addresses x 2 recordTypes = 4 whitelist txs.
    assert_eq!(
        b["whitelistTxs"].as_array().unwrap().len(),
        4,
        "one tx per (address,recordType)"
    );

    // assert isWhitelistedFor == true ON-CHAIN for EACH (address, recordType) pair.
    for addr in [ADDR1, ADDR2] {
        for rt in [&rt_vacc, &rt_titer] {
            assert_eq!(
                cast_call(
                    &rpc,
                    &registry,
                    "isWhitelistedFor(bytes32,address)(bool)",
                    &[rt, addr]
                ),
                "true",
                "addr {addr} must be whitelisted on-chain for {rt}"
            );
        }
    }

    // delist the application -> admin signer delists EACH pair; one pair flips to false.
    let (s, b) = call(
        &app,
        "POST",
        &format!("/v1/issuer-applications/{app_id}/delist"),
        Some(&admin),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "delist: {b}");
    assert_eq!(
        cast_call(
            &rpc,
            &registry,
            "isWhitelistedFor(bytes32,address)(bool)",
            &[&rt_vacc, ADDR1]
        ),
        "false",
        "after delist, isWhitelistedFor must be false on-chain"
    );
    assert_eq!(
        cast_call(
            &rpc,
            &registry,
            "isWhitelistedFor(bytes32,address)(bool)",
            &[&rt_titer, ADDR2]
        ),
        "false",
        "all pairs delisted"
    );
}
