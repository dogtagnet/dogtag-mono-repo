//! Phase-3 acceptance: the full backend flow driven against a REAL local anvil (chainId 135) with the
//! actual DogTag contracts deployed via forge/cast. Issuance is genuinely re-verified on-chain.
//!
//! Requires `forge`, `cast`, `anvil` on PATH (Foundry). The test compiles the contracts once and runs
//! against an ephemeral anvil on a random port (killed on drop). If Foundry is absent the test SKIPS
//! (prints a notice and returns) rather than failing — the MemChain flow in `flow_memchain.rs` is the
//! hermetic always-on equivalent.

mod common;

use axum::http::StatusCode;
use common::*;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use vet_api::chain::{record_type_key, AlloyChain, ChainClient};

const CONTRACTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../contracts");
// anvil default account 0 (admin/deployer/funder).
const PK0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ACC0: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
/// Anvil account 1 — the PROVIDER's key, deliberately distinct from the registrar's `ACC0`.
/// `createIssuer` and `repointService` are the provider's own transactions; every registrar call is
/// `onlyOwner`. Running both halves from one key would assert a journey no provider can walk.
const PK1: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const ACC1: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
/// A registrar-issued `bytes20` provider id. Arbitrary and permanent by design — it means nothing.
const PROVIDER_ID: &str = "0x00000000000000000000000000000000000000a1";
const IDENTITY_DIGEST: &str =
    "0x1111111111111111111111111111111111111111111111111111111111111111";
/// `ProviderRegistry.Standing.ACTIVE`.
const STANDING_ACTIVE: &str = "2";
/// The factory generation string `Deploy.s.sol` registers its factory under. `attachService`
/// requires the factory to carry it, so it is DERIVED from the same string the script hashes rather
/// than pasted as a digest — a literal here could drift from the protocol constant silently.
const FACTORY_GENERATION_LABEL: &str = "dogtag-issuer-factory/1";

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

fn start_anvil() -> Anvil {
    let port = pick_port();
    let child = Command::new("anvil")
        .args(["--chain-id", "135", "--port", &port.to_string(), "--silent"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn anvil");
    let anvil = Anvil { child, port };
    // wait until the RPC answers.
    for _ in 0..50 {
        if cast_ok(&anvil.rpc(), &["block-number"]) {
            return anvil;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("anvil did not come up");
}

fn pick_port() -> u16 {
    // bind to :0 to grab a free port, then release it for anvil.
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

/// `forge create ... --broadcast` and parse the "Deployed to:" address.
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

fn cast_send(rpc: &str, pk: &str, to: &str, sig: &str, args: &[&str]) {
    let mut cmd = Command::new("cast");
    cmd.args(["send", "--rpc-url", rpc, "--private-key", pk, to, sig])
        .args(args);
    run(&mut cmd);
}

fn cast_call(rpc: &str, to: &str, sig: &str, args: &[&str]) -> String {
    let mut cmd = Command::new("cast");
    cmd.args(["call", "--rpc-url", rpc, to, sig]).args(args);
    run(&mut cmd).trim().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn anvil_full_flow() {
    if !have_foundry() {
        eprintln!("SKIP anvil_full_flow: Foundry (forge/cast/anvil) not on PATH");
        return;
    }
    // compile contracts once.
    run(Command::new("forge").args(["build"]));

    let anvil = start_anvil();
    let rpc = anvil.rpc();
    let rt = record_type_key("VACCINATION"); // 0x6510...
    let generation = format!(
        "0x{}",
        hex::encode(alloy::primitives::keccak256(FACTORY_GENERATION_LABEL.as_bytes()).0)
    );

    // --- deploy the launch set ---
    //
    // Mirrors `script/Deploy.s.sol::deploy` for the contracts this flow needs. Deliberately not
    // driven by running that script: its `run()` entry ends in `_writeLedger`, which overwrites
    // `contracts/deployments/roax.json` — the repo's single source of truth for every address — with
    // whatever chain it just deployed to. The script is covered where it can be driven safely, by
    // `contracts/test/LaunchStack.sol` under `forge test`.
    let registry = forge_create(&rpc, "src/ProviderRegistry.sol:ProviderRegistry", &[ACC0]);
    let impl_addr = forge_create(&rpc, "src/DogTagIssuer.sol:DogTagIssuer", &[]);
    let factory = forge_create(
        &rpc,
        "src/DogTagIssuerFactory.sol:DogTagIssuerFactory",
        &[&impl_addr, &registry],
    );
    // A factory in no generation cannot have a service attached under it.
    cast_send(
        &rpc,
        PK0,
        &registry,
        "addFactoryGeneration(bytes32,address)",
        &[&generation, &factory],
    );

    // --- the provider journey, up to the point that needs a signer ---
    //
    // Issuance is SERVICE-scoped: `DogTagIssuer.issue` is gated by
    // `ProviderRegistry.canIssue(address(this), msg.sender)`, keyed on the clone rather than on a
    // record type. `canIssue` folds SIX registrar facts, and the issuance grant is the only one that
    // names a signer — so everything else happens here and the grant waits until custody genesis has
    // produced the backend address below.
    cast_send(
        &rpc,
        PK0,
        &registry,
        "registerProvider(bytes20,address,bytes32,uint32,uint16,uint8,bytes)",
        &[PROVIDER_ID, ACC1, IDENTITY_DIGEST, "1", "0xe3", "0x12", "0x"],
    );
    cast_send(
        &rpc,
        PK0,
        &registry,
        "setProviderStanding(bytes20,uint8)",
        &[PROVIDER_ID, STANDING_ACTIVE],
    );
    cast_send(
        &rpc,
        PK0,
        &registry,
        "setServiceCreationApproval(bytes20,bytes32,bool)",
        &[PROVIDER_ID, &rt, "true"],
    );
    // The provider deploys its own clone, from its own key.
    cast_send(
        &rpc,
        PK1,
        &factory,
        "createIssuer(bytes20,bytes32,uint96)",
        &[PROVIDER_ID, &rt, "0"],
    );
    let clone = cast_call(
        &rpc,
        &factory,
        "predictIssuer(bytes32,address,uint96)(address)",
        &[&rt, ACC1, "0"],
    );
    let clone = clone.to_lowercase();
    cast_send(
        &rpc,
        PK0,
        &registry,
        "attachService(bytes20,address,bytes32,address)",
        &[PROVIDER_ID, &clone, &generation, ACC1],
    );
    cast_send(
        &rpc,
        PK0,
        &registry,
        "setServiceStanding(address,uint8)",
        &[&clone, STANDING_ACTIVE],
    );
    // The provider's CURRENT pointer for this record type — its own decision, so its own
    // transaction. Without it the clone is attached and still anchors nothing.
    cast_send(&rpc, PK1, &registry, "repointService(address)", &[&clone]);

    // --- backend state: real AlloyChain on anvil + MemStore ---
    let chain = Arc::new(AlloyChain::new(rpc.clone()));
    let state = state_with(
        chain.clone() as Arc<dyn ChainClient>,
        rpc.clone(),
        registry.to_lowercase(),
        clone.clone(),
        "vet.example".to_string(),
        1, // low confirmations for tests
    );
    let app = vet_api::router(state);

    // --- custody: genesis/confirm/unlock (wires the backend signer into AlloyChain) ---
    let (_admin, op, backend_addr) = boot_custody(&app).await;

    // fund the backend signer from anvil acct 0, and whitelist it on-chain for VACCINATION.
    fund(&rpc, &backend_addr, "100000000000000000"); // 0.1 ETH
    // The grant is on the SIGNER's ADDRESS and names no service — `RIGHT_ISSUE` is bit 0 of the
    // rights bitmask. `canIssue` below still folds this clone's own lifecycle terms, which is why
    // that assertion still means something after the re-keying.
    cast_send(
        &rpc,
        PK0,
        &registry,
        "setRights(address,uint256)",
        &[&backend_addr, "1"],
    );
    assert_eq!(
        cast_call(
            &rpc,
            &registry,
            "canIssue(address,address)(bool)",
            &[&clone, &backend_addr]
        ),
        "true",
        "the journey must leave the backend signer able to anchor on this clone"
    );
    // LAYER 2: the clone's OWN list. `canIssue` above is only the AUTHORITY's half - the grant names
    // no service, so without this the clone refuses `NotLocallyAllowed`. Admitting is the clone
    // owner's call, and the owner here is the provider key that created it (PK1), never the
    // registrar - so this is deliberately not a PK0 transaction.
    cast_send(
        &rpc,
        PK1,
        &clone,
        "setIssuanceAllowed(address,bool)",
        &[&backend_addr, "true"],
    );
    assert_eq!(
        cast_call(&rpc, &clone, "issuanceAllowed(address)(bool)", &[&backend_addr]),
        "true",
        "both layers must hold before the backend can anchor"
    );

    // --- non-whitelisted signer fails preflight: query a random unwhitelisted addr ---
    let random_addr = "0x000000000000000000000000000000000000dead";
    assert_eq!(
        cast_call(
            &rpc,
            &registry,
            "isWhitelistedFor(bytes32,address)(bool)",
            &[&rt, random_addr]
        ),
        "false"
    );

    // --- prepare (backend mode): build + broadcast + hardened confirm (on-chain re-verify) ---
    let (s, b) = call(
        &app,
        "POST",
        "/credentials/prepare",
        Some(&op),
        Some(serde_json::json!({"recordType":"VACCINATION","dogTagId":"42","fields":vaccination_fields()})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "prepare(backend): {b}");
    assert_eq!(b["mode"], "backend");
    let record_id = b["recordId"].as_str().unwrap().to_string();
    let root = b["merkleRoot"].as_str().unwrap().to_string();
    let tx_hash = b["txHash"].as_str().unwrap().to_string();

    // independently re-verify ON CHAIN: isValid(root) && issuedAt(root) != 0.
    assert_eq!(
        cast_call(&rpc, &clone, "isValid(bytes32)(bool)", &[&root]),
        "true",
        "root must be issued+valid on-chain"
    );
    assert!(
        chain.is_valid(&clone, &root).await.unwrap(),
        "AlloyChain.isValid agrees"
    );

    // --- share -> GET /r/<token> with the SHORT one-time token -> doc returned; issuance VALID ---
    let (s, b) = call(
        &app,
        "POST",
        &format!("/records/{record_id}/share"),
        Some(&op),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "share: {b}");
    let qr = b["qrUrl"].as_str().unwrap();
    assert!(
        !qr.contains("t="),
        "qrUrl must not carry a JWT query string: {qr}"
    );
    let token = extract_token(qr);

    let (s, doc) = call(&app, "GET", &format!("/r/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "get shared: {doc}");
    let merkle_root = doc["signature"]["merkleRoot"].as_str().unwrap().to_string();
    assert_eq!(merkle_root, root);
    assert!(
        chain.is_valid(&clone, &merkle_root).await.unwrap(),
        "issuance pillar: returned doc's root is VALID on-chain"
    );

    // reused short token => 404 (one-time, deleted after first use).
    let (s, _b) = call(&app, "GET", &format!("/r/{token}"), None, None).await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "reused share token must be 404 (one-time)"
    );

    // --- confirm REFUSES a bogus txHash (independent prepared record) ---
    // prepare a wallet-mode record so confirm is callable separately.
    let (s, _b) = call(&app, "POST", "/credentials/confirm", Some(&op),
        Some(serde_json::json!({"recordId": record_id, "txHash": "0x0000000000000000000000000000000000000000000000000000000000000bad"}))).await;
    // already issued + idempotency: not OK with a different/bogus hash.
    assert_ne!(
        s,
        StatusCode::OK,
        "confirm of an issued record with a bogus hash must not succeed"
    );
    let _ = tx_hash;

    // --- revoke -> issuance INVALID on re-verify ---
    let (s, b) = call(
        &app,
        "POST",
        &format!("/records/{record_id}/revoke"),
        Some(&op),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "revoke: {b}");
    assert_eq!(
        cast_call(&rpc, &clone, "isValid(bytes32)(bool)", &[&root]),
        "false",
        "after revoke, isValid must be false on-chain"
    );
    assert!(
        !chain.is_valid(&clone, &root).await.unwrap(),
        "issuance pillar now INVALID"
    );
}

fn fund(rpc: &str, to: &str, wei: &str) {
    let mut cmd = Command::new("cast");
    cmd.args([
        "send",
        "--rpc-url",
        rpc,
        "--private-key",
        PK0,
        to,
        "--value",
        wei,
    ]);
    run(&mut cmd);
}

fn extract_token(qr: &str) -> String {
    // qrUrl: .../r/<32hex>
    qr.rsplit('/').next().unwrap().to_string()
}
