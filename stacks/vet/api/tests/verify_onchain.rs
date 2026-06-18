//! Phase-3 ON-CHAIN verification acceptance: the REAL `/verify/*` leg recording `Verified` events on
//! the VerificationRegistry deployed to a local anvil (chainId 135).
//!
//! Two paths, both broadcasting a genuine tx (the relayer pays gas) and asserting a real `Verified`
//! event + a consumed nullifier on-chain:
//!
//!   NORMAL — issue a root R on a VACCINATION clone, mint the pet SBT to a subject whose secp256k1 key
//!     we control (alloy LocalSigner), whitelist the relayer (== backend custody signer) for
//!     keccak256(abi.encode("VERIFY:", purpose)), build the 9-field VerificationConsent, sign the
//!     EIP-712 digest with the subject key (reusing dogtag-standard `hash_typed_consent`), and call
//!     `ChainClient::record_verification` AS the relayer. Assert `Verified(nullifier)` where the
//!     nullifier matches `consent_nullifier` (the registry computes it via on-chain Poseidon6).
//!
//!   ZK — run the REAL ark-circom Groth16 prover (numLeaves=13, subject=vm.addr(1)) via the circuit
//!     input from `tests/gen_input.mjs`, set up on-chain state to match the produced `pub[7]` (issue
//!     root=pub[6], mint dogTagId=pub[0] to subject=address(pub[3]), bind keyHash=pub[5] for the
//!     subject via ConsentKeyRegistry.bindConsentKey, whitelist relayer=address(pub[2]) for the
//!     purpose), then broadcast `recordVerificationZK(a,b,c,pub)` AS the relayer and assert `Verified`
//!     + consumed[nullifier]. This is SLOW (real Groth16 proving, minutes).
//!
//! Requires forge/cast/anvil + node (for gen_input.mjs) + the circuits build artifacts. SKIPS
//! gracefully if Foundry is absent (mirrors flow.rs).

mod common;

use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use alloy::primitives::{keccak256, Address, U256};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use common::*;

use vet_api::chain::{record_type_key, AlloyChain, ChainClient, ConsentInput};
use vet_api::prover::{ArkProver, ProveInputs};

const CONTRACTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../contracts");
const REPO_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../..");
// anvil default account 0 (admin/deployer/funder).
const PK0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ACC0: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

const BN254_R: &str = "21888242871839275222246405745257275088548364400416034343698204186575808495617";

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

fn have_node() -> bool {
    Command::new("node")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
    let out = cmd.current_dir(CONTRACTS_DIR).output().expect("run command");
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
    cmd.args(["create", "--rpc-url", rpc, "--private-key", PK0, "--broadcast", contract]);
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

/// Deploy the Poseidon6 contract from the raw circomlib initcode; return its address.
fn deploy_poseidon6(rpc: &str) -> String {
    let initcode = std::fs::read_to_string(format!("{CONTRACTS_DIR}/test/poseidon6.initcode"))
        .expect("read poseidon6.initcode");
    let initcode = initcode.trim();
    let mut cmd = Command::new("cast");
    cmd.args(["send", "--rpc-url", rpc, "--private-key", PK0, "--create", initcode, "--json"]);
    let out = run(&mut cmd);
    let v: serde_json::Value = serde_json::from_str(&out).expect("cast send --json");
    v["contractAddress"].as_str().expect("contractAddress").to_lowercase()
}

fn cast_send(rpc: &str, pk: &str, to: &str, sig: &str, args: &[&str]) {
    let mut cmd = Command::new("cast");
    cmd.args(["send", "--rpc-url", rpc, "--private-key", pk, to, sig]).args(args);
    run(&mut cmd);
}

fn cast_call(rpc: &str, to: &str, sig: &str, args: &[&str]) -> String {
    let mut cmd = Command::new("cast");
    cmd.args(["call", "--rpc-url", rpc, to, sig]).args(args);
    run(&mut cmd).trim().to_string()
}

/// Poll until `tx_hash` has a receipt; panic if it reverted.
fn wait_mined(rpc: &str, tx_hash: &str) {
    for _ in 0..100 {
        let out = Command::new("cast")
            .args(["receipt", "--rpc-url", rpc, tx_hash, "status"])
            .current_dir(CONTRACTS_DIR)
            .output()
            .expect("cast receipt");
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // cast prints "1 (success)" or "0 (failed)".
            assert!(s.starts_with('1'), "tx {tx_hash} reverted: {s}");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("tx {tx_hash} never mined");
}

fn fund(rpc: &str, to: &str, wei: &str) {
    let mut cmd = Command::new("cast");
    cmd.args(["send", "--rpc-url", rpc, "--private-key", PK0, to, "--value", wei]);
    run(&mut cmd);
}

fn hex32(u: U256) -> String {
    format!("0x{:064x}", u)
}

/// Deploy the full verification stack. Returns (registry, factory, sbt, consentKeys, verification).
struct Stack {
    registry: String,
    factory: String,
    sbt: String,
    consent_keys: String,
    verification: String,
}

fn deploy_stack(rpc: &str) -> Stack {
    let registry = forge_create(rpc, "src/IssuerRegistry.sol:IssuerRegistry", &[ACC0]);
    let impl_addr = forge_create(rpc, "src/DogTagIssuer.sol:DogTagIssuer", &[]);
    let factory = forge_create(
        rpc,
        "src/DogTagIssuerFactory.sol:DogTagIssuerFactory",
        &[&impl_addr, &registry, ACC0],
    );
    let sbt = forge_create(rpc, "src/DogTagSBT.sol:DogTagSBT", &[ACC0]);
    let consent_keys = forge_create(rpc, "src/ConsentKeyRegistry.sol:ConsentKeyRegistry", &[]);
    let verifier = forge_create(rpc, "src/Groth16Verifier.sol:Groth16Verifier", &[]);
    let poseidon6 = deploy_poseidon6(rpc);
    // VerificationRegistry(ir, sbt, zk, ck, ridx(=factory), pos6, admin)
    let verification = forge_create(
        rpc,
        "src/VerificationRegistry.sol:VerificationRegistry",
        &[&registry, &sbt, &verifier, &consent_keys, &factory, &poseidon6, ACC0],
    );
    Stack { registry, factory, sbt, consent_keys, verification }
}

/// node tests/gen_input.mjs -> the circuit input JSON + expected pub (decimal).
#[derive(serde::Deserialize)]
struct GenOutput {
    input: serde_json::Value,
    #[serde(rename = "pubDecimal")]
    pub_decimal: Vec<String>,
}
fn gen_input() -> GenOutput {
    let script = format!("{REPO_ROOT}/crates/dogtag-prover-rs/tests/gen_input.mjs");
    let out = Command::new("node")
        .arg(&script)
        .env("MONOREPO_ROOT", REPO_ROOT)
        .current_dir(format!("{REPO_ROOT}/circuits"))
        .output()
        .expect("spawn node gen_input.mjs");
    assert!(
        out.status.success(),
        "gen_input.mjs failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("gen_input.mjs JSON")
}

// ------------------------------------------------------------------------------------------------
// NORMAL path
// ------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn normal_path_records_verified_onchain() {
    if !have_foundry() {
        eprintln!("SKIP normal_path_records_verified_onchain: Foundry not on PATH");
        return;
    }
    run(Command::new("forge").args(["build"]));
    let anvil = start_anvil();
    let rpc = anvil.rpc();

    let stack = deploy_stack(&rpc);
    let vacc_rt = record_type_key("VACCINATION"); // bytes32

    // VACCINATION clone owned by ACC0; whitelist ACC0 to issue on it.
    cast_send(&rpc, PK0, &stack.factory, "createIssuer(string,bytes32,address)", &["VACC", &vacc_rt, ACC0]);
    let clone = cast_call(&rpc, &stack.factory, "predictIssuer(bytes32,address)(address)", &[&vacc_rt, ACC0]).to_lowercase();
    cast_send(&rpc, PK0, &stack.registry, "whitelistFor(bytes32,address)", &[&vacc_rt, ACC0]);

    // issue a credential root R on the clone.
    let root = "0x1100000000000000000000000000000000000000000000000000000000000011";
    cast_send(&rpc, PK0, &clone, "issue(bytes32)", &[root]);
    assert_eq!(cast_call(&rpc, &clone, "isValid(bytes32)(bool)", &[root]), "true");

    // subject — an alloy LocalSigner whose secp256k1 key we control.
    let subject_signer: PrivateKeySigner =
        "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d".parse().unwrap();
    let subject_addr = format!("{:#x}", subject_signer.address());

    // mint the pet SBT (dogTagId) to the subject (grant ACC0 the ISSUER_ROLE first).
    let issuer_role = cast_call(&rpc, &stack.sbt, "ISSUER_ROLE()(bytes32)", &[]);
    cast_send(&rpc, PK0, &stack.sbt, "grantRole(bytes32,address)", &[&issuer_role, ACC0]);
    let dog_tag_id: u64 = 4242;
    cast_send(
        &rpc,
        PK0,
        &stack.sbt,
        "mint(address,uint256,bytes32)",
        &[&subject_addr, &dog_tag_id.to_string(), &keccak_hex("profile")],
    );

    // purpose reduced mod r (the registry compares uint256(purpose) < r).
    let purpose = reduce_keccak_mod_r("GROOMING_INTAKE");
    let verify_key = verify_key_of(&purpose);

    // backend state: real AlloyChain + custody. The relayer == the backend custody signer.
    let chain = Arc::new(AlloyChain::new(rpc.clone()));
    let state = state_with_verify(
        chain.clone() as Arc<dyn ChainClient>,
        rpc.clone(),
        stack.registry.clone(),
        stack.verification.clone(),
        clone.clone(),
        "vet.example".to_string(),
        1,
        Arc::new(vet_api::prover::StubProver),
    );
    let app = vet_api::router(state);
    let (_admin, _op, relayer_addr) = boot_custody(&app).await;
    fund(&rpc, &relayer_addr, "1000000000000000000"); // 1 ETH for gas

    // whitelist the relayer for keccak256(abi.encode("VERIFY:", purpose)).
    cast_send(&rpc, PK0, &stack.registry, "whitelistFor(bytes32,address)", &[&verify_key, &relayer_addr]);
    assert_eq!(
        cast_call(&rpc, &stack.registry, "isWhitelistedFor(bytes32,address)(bool)", &[&verify_key, &relayer_addr]),
        "true"
    );

    // build the 9-field VerificationConsent.
    let nonce: u64 = 1;
    let deadline = U256::from(u64::MAX);
    let consent = ConsentInput {
        dog_tag_id: U256::from(dog_tag_id),
        record_type: vacc_rt.clone(),
        purpose: purpose.clone(),
        credential_root: root.to_string(),
        challenge: "0x00000000000000000000000000000000000000000000000000000000000000aa".to_string(),
        relayer: relayer_addr.clone(),
        subject: subject_addr.clone(),
        nonce: U256::from(nonce),
        deadline,
    };

    // sign the EIP-712 digest with the subject key (reuse dogtag-standard consent.rs parity).
    let sig = sign_consent(&subject_signer, &consent, &stack.verification);

    // broadcast recordVerification AS the relayer (backend signer index 0).
    let sent = chain
        .record_verification(0, &stack.verification, &consent, &sig)
        .await
        .expect("recordVerification broadcast");

    // ensure the tx is mined (anvil auto-mines; poll the receipt via cast for determinism).
    wait_mined(&rpc, &sent.tx_hash);

    // assert the on-chain Verified event with the expected nullifier (consent.rs parity).
    let ev = chain
        .get_verified_event(&sent.tx_hash, &stack.verification)
        .await
        .expect("Verified event present");
    let expected_nf = expected_nullifier(&consent);
    assert_eq!(ev.nullifier.to_lowercase(), expected_nf.to_lowercase(), "nullifier mismatch");
    assert_eq!(ev.dog_tag_id, U256::from(dog_tag_id));
    assert!(ev.relayer.eq_ignore_ascii_case(&relayer_addr));
    assert!(ev.subject.eq_ignore_ascii_case(&subject_addr));

    // consumed[nullifier] == true on-chain.
    assert!(
        chain.consumed(&stack.verification, &expected_nf).await.unwrap(),
        "nullifier must be consumed on-chain"
    );
}

// ------------------------------------------------------------------------------------------------
// ZK path (real Groth16 proof) — SLOW.
// ------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn zk_path_records_verified_onchain() {
    if !have_foundry() {
        eprintln!("SKIP zk_path_records_verified_onchain: Foundry not on PATH");
        return;
    }
    if !have_node() {
        eprintln!("SKIP zk_path_records_verified_onchain: node not on PATH");
        return;
    }
    let build_dir = format!("{REPO_ROOT}/circuits/build");
    if !std::path::Path::new(&format!("{build_dir}/verification_final.zkey")).exists() {
        eprintln!("SKIP zk_path_records_verified_onchain: circuits build artifacts missing");
        return;
    }

    run(Command::new("forge").args(["build"]));
    let anvil = start_anvil();
    let rpc = anvil.rpc();
    let stack = deploy_stack(&rpc);

    // 1. produce a REAL proof (a,b,c,pub) via the ark-circom prover from the gen_input fixture.
    let gen = gen_input();
    let inputs = ProveInputs::from_circuit_input_json(&gen.input).expect("ProveInputs");
    let prover = ArkProver::load(&build_dir).expect("load prover");
    eprintln!("zk_path: proving (this is slow — real Groth16)...");
    let out = prover.prove_inputs(inputs).await.expect("prove");
    let pub_ = &out.public_signals;
    assert_eq!(pub_.to_vec(), gen.pub_decimal, "pub mismatch vs fixture");

    let dog_tag_id = U256::from_str_radix(&pub_[0], 10).unwrap();
    let purpose_u = U256::from_str_radix(&pub_[1], 10).unwrap();
    let relayer_addr = format!("0x{:040x}", U256::from_str_radix(&pub_[2], 10).unwrap());
    let subject_addr = format!("0x{:040x}", U256::from_str_radix(&pub_[3], 10).unwrap());
    let nullifier = hex32(U256::from_str_radix(&pub_[4], 10).unwrap());
    let key_hash = hex32(U256::from_str_radix(&pub_[5], 10).unwrap());
    let root = hex32(U256::from_str_radix(&pub_[6], 10).unwrap());
    let purpose_b32 = hex32(purpose_u);
    let verify_key = verify_key_of(&purpose_b32);

    // subject == vm.addr(1): the key is 1.
    let subject_signer: PrivateKeySigner =
        "0x0000000000000000000000000000000000000000000000000000000000000001".parse().unwrap();
    assert!(
        format!("{:#x}", subject_signer.address()).eq_ignore_ascii_case(&subject_addr),
        "subject != vm.addr(1)"
    );

    // 2. on-chain state to satisfy recordVerificationZK:
    // issue root=pub[6] on a VACCINATION clone.
    let vacc_rt = record_type_key("VACCINATION");
    cast_send(&rpc, PK0, &stack.factory, "createIssuer(string,bytes32,address)", &["VACC", &vacc_rt, ACC0]);
    let clone = cast_call(&rpc, &stack.factory, "predictIssuer(bytes32,address)(address)", &[&vacc_rt, ACC0]).to_lowercase();
    cast_send(&rpc, PK0, &stack.registry, "whitelistFor(bytes32,address)", &[&vacc_rt, ACC0]);
    cast_send(&rpc, PK0, &clone, "issue(bytes32)", &[&root]);
    assert_eq!(cast_call(&rpc, &clone, "isValid(bytes32)(bool)", &[&root]), "true");

    // mint dogTagId=pub[0] to subject=address(pub[3]).
    let issuer_role = cast_call(&rpc, &stack.sbt, "ISSUER_ROLE()(bytes32)", &[]);
    cast_send(&rpc, PK0, &stack.sbt, "grantRole(bytes32,address)", &[&issuer_role, ACC0]);
    cast_send(
        &rpc,
        PK0,
        &stack.sbt,
        "mint(address,uint256,bytes32)",
        &[&subject_addr, &dog_tag_id.to_string(), &keccak_hex("profile")],
    );

    // bind keyHash=pub[5] for the subject via ConsentKeyRegistry.bindConsentKey (EIP-712, key=1).
    // We sign the bind digest with subject key, then broadcast the bind FROM the subject (funded).
    fund(&rpc, &subject_addr, "1000000000000000000");
    let bind_sig = sign_bind_key(&subject_signer, &stack.consent_keys, &key_hash, U256::ZERO);
    let bind_calldata = vet_api::chain::bind_consent_key_calldata(&key_hash, &bind_sig);
    let subject_chain = AlloyChain::new(rpc.clone());
    subject_chain
        .register_signer(7, key_bytes_one(), subject_addr.clone())
        .await;
    subject_chain
        .sign_and_send(7, &stack.consent_keys, &bind_calldata)
        .await
        .expect("bindConsentKey");
    assert_eq!(
        cast_call(&rpc, &stack.consent_keys, "keyOf(address)(bytes32)", &[&subject_addr]).to_lowercase(),
        key_hash.to_lowercase(),
        "consent key bound"
    );

    // whitelist relayer=address(pub[2]) for VERIFY:purpose.
    cast_send(&rpc, PK0, &stack.registry, "whitelistFor(bytes32,address)", &[&verify_key, &relayer_addr]);

    // 3. broadcast recordVerificationZK(a,b,c,pub) AS the relayer (pub[2]).
    // The relayer is fixed by the proof (0x1111...), whose key we do not hold — impersonate it on
    // anvil and broadcast the chain client's ABI-encoded calldata.
    fund(&rpc, &relayer_addr, "1000000000000000000");
    let zk_calldata =
        vet_api::chain::record_verification_zk_calldata(&out.a, &out.b, &out.c, &out.public_signals);
    let tx_hash = anvil_impersonate_send(&rpc, &relayer_addr, &stack.verification, &zk_calldata);

    // 4. assert the on-chain Verified event + consumed[nullifier], read via the chain client.
    let chain = AlloyChain::new(rpc.clone());
    let ev = chain
        .get_verified_event(&tx_hash, &stack.verification)
        .await
        .expect("Verified event present");
    assert_eq!(ev.nullifier.to_lowercase(), nullifier.to_lowercase(), "zk nullifier mismatch");
    assert_eq!(ev.dog_tag_id, dog_tag_id);
    assert!(ev.relayer.eq_ignore_ascii_case(&relayer_addr));
    assert!(ev.subject.eq_ignore_ascii_case(&subject_addr));
    assert!(
        chain.consumed(&stack.verification, &nullifier).await.unwrap(),
        "zk nullifier must be consumed on-chain"
    );
}

// ------------------------------------------------------------------------------------------------
// helpers
// ------------------------------------------------------------------------------------------------

fn key_bytes_one() -> [u8; 32] {
    let mut k = [0u8; 32];
    k[31] = 1;
    k
}

fn keccak_hex(s: &str) -> String {
    format!("0x{}", hex::encode(keccak256(s.as_bytes())))
}

/// keccak256(abi.encode("VERIFY:", purpose)) — the registry's `_verifyKey` (purpose is a 0x.. bytes32).
fn verify_key_of(purpose: &str) -> String {
    use alloy::sol_types::SolValue;
    let enc = (String::from("VERIFY:"), parse_b32(purpose)).abi_encode_params();
    format!("0x{}", hex::encode(keccak256(&enc)))
}

fn parse_b32(h: &str) -> alloy::primitives::FixedBytes<32> {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let mut out = [0u8; 32];
    let b = hex::decode(s).unwrap();
    out[32 - b.len()..].copy_from_slice(&b);
    alloy::primitives::FixedBytes::<32>::from(out)
}

/// keccak256(label) reduced mod the BN254 scalar field, as a 0x.. bytes32.
fn reduce_keccak_mod_r(label: &str) -> String {
    let k = U256::from_be_bytes::<32>(keccak256(label.as_bytes()).0);
    let r = U256::from_str_radix(BN254_R, 10).unwrap();
    hex32(k % r)
}

/// Build the dogtag-standard `VerificationConsent` from a `ConsentInput` (for digest/nullifier parity).
fn to_std_consent(c: &ConsentInput) -> dogtag_standard::consent::VerificationConsent {
    let b32 = |h: &str| parse_b32(h).0;
    let a20 = |h: &str| {
        let a: Address = h.parse().unwrap();
        let mut out = [0u8; 20];
        out.copy_from_slice(a.as_slice());
        out
    };
    let mut dog = [0u8; 32];
    dog.copy_from_slice(&c.dog_tag_id.to_be_bytes::<32>());
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&c.nonce.to_be_bytes::<32>());
    let mut deadline = [0u8; 32];
    deadline.copy_from_slice(&c.deadline.to_be_bytes::<32>());
    dogtag_standard::consent::VerificationConsent {
        dog_tag_id: dog,
        record_type: b32(&c.record_type),
        purpose: b32(&c.purpose),
        credential_root: b32(&c.credential_root),
        challenge: b32(&c.challenge),
        relayer: a20(&c.relayer),
        subject: a20(&c.subject),
        nonce,
        deadline,
    }
}

fn expected_nullifier(c: &ConsentInput) -> String {
    format!("0x{}", hex::encode(dogtag_standard::consent::consent_nullifier(&to_std_consent(c))))
}

/// EIP-712 sign the VerificationConsent digest with the subject key (verifyingContract = registry).
fn sign_consent(signer: &PrivateKeySigner, c: &ConsentInput, verifying_contract: &str) -> String {
    let addr: Address = verifying_contract.parse().unwrap();
    let mut vc = [0u8; 20];
    vc.copy_from_slice(addr.as_slice());
    let digest = dogtag_standard::consent::hash_typed_consent(&to_std_consent(c), vc, 135);
    let sig = signer.sign_hash_sync(&alloy::primitives::B256::from(digest)).expect("sign");
    format!("0x{}", hex::encode(sig.as_bytes()))
}

/// EIP-712 sign BindConsentKey(babyJubPubKeyHash, wallet, nonce) for ConsentKeyRegistry (chainId 135).
fn sign_bind_key(signer: &PrivateKeySigner, consent_keys: &str, key_hash: &str, nonce: U256) -> String {
    use alloy::primitives::B256;
    let domain_typehash = keccak256(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)".as_slice(),
    );
    let name_hash = keccak256(b"DogTag".as_slice());
    let version_hash = keccak256(b"1".as_slice());
    let ck: Address = consent_keys.parse().unwrap();
    let mut ds_buf = Vec::new();
    ds_buf.extend_from_slice(domain_typehash.as_slice());
    ds_buf.extend_from_slice(name_hash.as_slice());
    ds_buf.extend_from_slice(version_hash.as_slice());
    ds_buf.extend_from_slice(&U256::from(135u64).to_be_bytes::<32>());
    let mut ckw = [0u8; 32];
    ckw[12..].copy_from_slice(ck.as_slice());
    ds_buf.extend_from_slice(&ckw);
    let domain_sep = keccak256(&ds_buf);

    let bind_typehash =
        keccak256(b"BindConsentKey(bytes32 babyJubPubKeyHash,address wallet,uint256 nonce)".as_slice());
    let mut sh_buf = Vec::new();
    sh_buf.extend_from_slice(bind_typehash.as_slice());
    sh_buf.extend_from_slice(parse_b32(key_hash).as_slice());
    let mut wallet_word = [0u8; 32];
    wallet_word[12..].copy_from_slice(signer.address().as_slice());
    sh_buf.extend_from_slice(&wallet_word);
    sh_buf.extend_from_slice(&nonce.to_be_bytes::<32>());
    let struct_hash = keccak256(&sh_buf);

    let mut buf = Vec::new();
    buf.extend_from_slice(&[0x19, 0x01]);
    buf.extend_from_slice(domain_sep.as_slice());
    buf.extend_from_slice(struct_hash.as_slice());
    let digest = keccak256(&buf);
    let sig = signer.sign_hash_sync(&B256::from(digest)).expect("sign bind");
    format!("0x{}", hex::encode(sig.as_bytes()))
}

/// Impersonate `from` on anvil and broadcast `calldata` to `to`; return the tx hash.
fn anvil_impersonate_send(rpc: &str, from: &str, to: &str, calldata: &str) -> String {
    // anvil_impersonateAccount
    run(Command::new("cast").args(["rpc", "--rpc-url", rpc, "anvil_impersonateAccount", from]));
    let out = run(Command::new("cast").args([
        "send", "--rpc-url", rpc, "--from", from, "--unlocked", to, calldata, "--json",
    ]));
    let v: serde_json::Value = serde_json::from_str(&out).expect("cast send --json");
    let tx = v["transactionHash"].as_str().expect("transactionHash").to_string();
    run(Command::new("cast").args(["rpc", "--rpc-url", rpc, "anvil_stopImpersonatingAccount", from]));
    tx
}
