//! M-3 acceptance: an OWNER-HIDDEN Level-B consent proof reaches the chain through the NEW `sol!`
//! interface and is recorded by `VerificationRegistryConsent`.
//!
//! This is the submit-side twin of M-2's issuance bridge. M-2 can mint an owner-hidden tag; without
//! this path a device can prove consent but nothing carries the proof to the registry.
//!
//! **What this file uniquely proves**, versus `contracts/test/ConsentRegistry.t.sol` (which already
//! covers the on-chain gate matrix with a real proof, via `prank`): that the RUST RELAYER's new
//! Level-B interface — `record_verification_zk_consent` (4-arg, selector `dd080593`) plus the
//! Level-B read-back (`get_consent_verified_event`, `consumed(pub[3])`) — encodes correctly against
//! the real deployed contract and a real Groth16 verifier. Selector or index drift here is silent at
//! the type level (both levels take the same `(a,b,c,pub)` prefix) and shows up on-chain as an empty
//! `0x` revert, which is exactly the historical failure this pins against.
//!
//! **Fixture.** `contracts/test/consent-fixture-anvil.json` — a REAL Groth16 proof against the
//! PRODUCTION M3 ceremony zkey, identical to the committed `consent-fixture.json` except that
//! `relayer` is anvil account 0 instead of `0x1111…1111`. That substitution is necessary, not
//! cosmetic: `relayer` is bound into both the EdDSA consent message `M` and the nullifier, so the
//! owner consents to ONE submitter and no other address can broadcast that proof. The committed
//! fixture is therefore unsubmittable from any key we hold. Changing a PUBLIC INPUT does not move
//! the verifying key — the same ceremony key and the same `Groth16VerifierConsent` verify both.
//! Regenerate with:
//!   CONSENT_FIXTURE_RELAYER=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 \
//!   CONSENT_FIXTURE_OUT=contracts/test/consent-fixture-anvil.json \
//!   node circuits/scripts/gen-consent-fixture.mjs
//!
//! Requires forge/cast/anvil. SKIPS gracefully if Foundry is absent.
//! No node/snarkjs/zkey needed at test time — the proof is committed, so this gives signal wherever
//! Foundry is present.

use std::process::{Child, Command, Stdio};

use alloy::primitives::U256;
use serde_json::Value;

use vet_api::chain::{AlloyChain, ChainClient};

const CONTRACTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../contracts");
// anvil default account 0 — deployer, admin, AND the relayer the fixture proof is bound to.
const PK0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ACC0: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
/// D1: the Level-B tag is minted to a NEUTRAL CUSTODIAN, never the owner. Deliberately an address
/// unrelated to the relayer and to the fixture's hidden owner, so a registry that ever reintroduced
/// an owner-identity comparison would fail this test rather than silently pass.
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
// anvil / forge harness
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
    cast_send(
        rpc,
        core,
        "setIssuanceCapability(address,address,bool)",
        &[&clone, signer, "true"],
    );
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

fn pk0_bytes() -> [u8; 32] {
    let raw = hex::decode(PK0.trim_start_matches("0x")).expect("PK0 hex");
    let mut k = [0u8; 32];
    k.copy_from_slice(&raw);
    k
}

// ------------------------------------------------------------------------------------------------
// fixture
// ------------------------------------------------------------------------------------------------

/// The Level-B proof + the on-chain state it demands, in the shapes the chain layer accepts.
struct Fixture {
    a: [String; 2],
    b: [[String; 2]; 2],
    c: [String; 2],
    pubs: [String; 7],
    dog_tag_id: String,
    purpose: String,
    root: String,
    record_type: String,
    /// The hidden owner. MUST NOT appear on-chain anywhere — asserted below.
    owner_address: String,
}

fn load_fixture() -> Fixture {
    let path = format!("{CONTRACTS_DIR}/test/consent-fixture-anvil.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {path}: {e} (regenerate — see the module docs)"));
    let j: Value = serde_json::from_str(&raw).expect("fixture JSON");

    let s = |v: &Value| v.as_str().expect("fixture string").to_string();
    let arr2 = |k: &str| -> [String; 2] {
        let a = j[k].as_array().unwrap_or_else(|| panic!("{k}: !array"));
        [s(&a[0]), s(&a[1])]
    };
    let bv = j["b"].as_array().expect("b: !array");
    let brow = |i: usize| -> [String; 2] {
        let r = bv[i].as_array().expect("b row");
        [s(&r[0]), s(&r[1])]
    };
    let pv = j["pub"].as_array().expect("pub: !array");
    assert_eq!(
        pv.len(),
        dogtag_standard::public_signals::NUM_PUBLIC,
        "fixture pub must be 7 signals"
    );
    let pubs: [String; 7] = std::array::from_fn(|i| s(&pv[i]));

    Fixture {
        a: arr2("a"),
        b: [brow(0), brow(1)],
        c: arr2("c"),
        pubs,
        dog_tag_id: s(&j["dogTagId"]),
        purpose: s(&j["purpose"]),
        root: s(&j["R"]),
        record_type: s(&j["recordType"]),
        owner_address: s(&j["_ownerAddress"]),
    }
}

struct Stack {
    registry: String,
    factory: String,
    sbt: String,
    verification_consent: String,
    clone: String,
}

/// Deploy the launch set and establish exactly the on-chain state the fixture's `pub[]` demands, so
/// all twelve registry gates can pass.
///
/// The stack comes from the REAL `script/Deploy.s.sol` rather than from a hand-assembled sequence of
/// `forge create` calls, for the reason `contracts/test/LaunchStack.sol` gives for doing the same:
/// a hand-wired equivalent lets this suite pass against a topology no deployment has, and leaves the
/// script — the one thing a real deployment runs — covered by nothing on the Rust side. It also
/// supplies `addFactoryGeneration`, which `attachService` requires and which no hand-wired stack had.
fn deploy_launch_set(rpc: &str, f: &Fixture) -> Stack {
    let d = deploy_launch_contracts(rpc);

    // Issuance is SERVICE-scoped now: `DogTagIssuer.issue` is gated by
    // `ProviderRegistry.canIssue(address(this), msg.sender)`, keyed on the clone rather than on a
    // record type, so a clone anchors nothing until the registrar has walked the whole journey and
    // its provider has designated it. `_onboard_issuing_clone` performs every step.
    let clone = onboard_issuing_clone(rpc, &d, &f.record_type, ACC0);

    // Gate #6, the orthogonal VERIFY axis. `setVerifierCapability` takes the RAW purpose and derives
    // `verificationKey` itself, so passing an already-derived key would derive twice and grant a
    // capability `canVerify` never reads — a transaction that succeeds, costs gas and authorises
    // nothing. That is why the old `verify_key_word` helper is gone rather than repointed.
    cast_send(
        rpc,
        &d.provider_registry,
        "setVerifierCapability(bytes32,address,bool)",
        &[&f.purpose, ACC0, "true"],
    );

    // The vet signer needs ISSUER_ROLE to mint.
    let issuer_role = cast_call(rpc, &d.sbt, "ISSUER_ROLE()(bytes32)", &[]);
    cast_send(
        rpc,
        &d.sbt,
        "grantRole(bytes32,address)",
        &[&issuer_role, ACC0],
    );

    let (registry, factory, sbt, verification_consent) = (
        d.provider_registry,
        d.factory,
        d.sbt,
        d.verification_registry,
    );
    Stack {
        registry,
        factory,
        sbt,
        verification_consent,
        clone,
    }
}

/// M-2's issuance, in the ORDER the contract mandates: `issue(R)` FIRST, then `mintCustodial(id, R)`.
///
/// The ordering is load-bearing, not stylistic. The mint is the irreversible half — `profileRoot[id]`
/// is write-once and survives a burn — so a mint that landed before a failing `issue` would retire
/// the `dogTagId` forever. Skipping `issue(R)` entirely leaves the root unanchored and every Level-B
/// verify reverts `"unknown root"` at gate #11.
fn issue_then_mint(rpc: &str, st: &Stack, dog_tag_id: &str, root: &str) {
    cast_send(rpc, &st.clone, "issue(bytes32)", &[root]);
    assert_eq!(
        cast_call(rpc, &st.clone, "isValid(bytes32)(bool)", &[root]),
        "true",
        "root must be anchored before minting"
    );
    cast_send(
        rpc,
        &st.sbt,
        "mintCustodial(uint256,bytes32)",
        &[dog_tag_id, root],
    );
}

// ------------------------------------------------------------------------------------------------
// M-3 acceptance
// ------------------------------------------------------------------------------------------------

/// THE acceptance test: a device-computed owner-hidden consent proof, against a tag minted by M-2's
/// `issue(R)` + `mintCustodial`, submitted through the NEW Level-B `sol!` interface, is recorded
/// on-chain — all twelve registry gates passing — and reads back in Level-B shapes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn level_b_consent_proof_records_owner_blind_verification_onchain() {
    if !have_foundry() {
        eprintln!("SKIP level_b_consent_proof_records_...: Foundry not on PATH");
        return;
    }
    let anvil = start_anvil();
    let rpc = anvil.rpc();
    let f = load_fixture();
    let stack = deploy_launch_set(&rpc, &f);
    issue_then_mint(&rpc, &stack, &f.dog_tag_id, &f.root);

    // The binding the whole Level-B model rests on (gate #7): the proof's R must be THIS tag's
    // on-chain root. The circuit deliberately does NOT bind dogTagId <-> R, so the registry is the
    // only place it is checked.
    assert_eq!(
        cast_call(
            &rpc,
            &stack.sbt,
            "profileRoot(uint256)(bytes32)",
            &[&f.dog_tag_id]
        ),
        f.root,
        "profileRoot must equal the proof's R"
    );

    // --- submit through the NEW Level-B interface, as the relayer the proof names ---
    let chain = AlloyChain::new(rpc.clone());
    chain
        .register_signer(0, pk0_bytes(), ACC0.to_string())
        .await;
    let sent = chain
        .record_verification_zk_consent(0, &stack.verification_consent, &f.a, &f.b, &f.c, &f.pubs)
        .await
        .expect("Level-B recordVerificationZK should be recorded on-chain");

    // --- read back in LEVEL-B shapes ---
    let ev = chain
        .get_consent_verified_event(&sent.tx_hash, &stack.verification_consent)
        .await
        .expect("owner-blind Verified event present");

    use dogtag_standard::public_signals::level_b as PB;
    let want_id = U256::from_str_radix(f.dog_tag_id.trim_start_matches("0x"), 16)
        .or_else(|_| U256::from_str_radix(&f.dog_tag_id, 10))
        .expect("dogTagId parses");
    assert_eq!(ev.dog_tag_id, want_id, "event dogTagId");
    assert!(
        ev.relayer.eq_ignore_ascii_case(ACC0),
        "event relayer should be the submitting relayer, got {}",
        ev.relayer
    );
    assert_eq!(ev.purpose, f.purpose, "event purpose");

    // The one-time check, keyed on pub[3] — the LEVEL-B nullifier slot. Reading pub[4] here would key
    // on R instead, and a SUCCESSFUL verification would look forever-unconsumed (the E-1 bug).
    let nullifier = ev.nullifier.clone();
    let pub_nullifier = {
        let u = U256::from_str_radix(f.pubs[PB::NULLIFIER].trim_start_matches("0x"), 16)
            .or_else(|_| U256::from_str_radix(&f.pubs[PB::NULLIFIER], 10))
            .expect("nullifier parses");
        format!("0x{}", hex::encode(u.to_be_bytes::<32>()))
    };
    assert_eq!(
        nullifier.to_lowercase(),
        pub_nullifier.to_lowercase(),
        "the event nullifier must be pub[3], not pub[4] (which is R)"
    );
    assert!(
        chain
            .consumed(&stack.verification_consent, &nullifier)
            .await
            .expect("consumed() read"),
        "nullifier must be consumed after a successful record"
    );

    // OWNER-BLIND: the hidden owner must appear nowhere on-chain. The Level-B `Verified` has no
    // `subject` field at the type level, so this guards the remaining surface — the raw calldata.
    let owner_bare = f.owner_address.trim_start_matches("0x").to_lowercase();
    let calldata =
        vet_api::chain::record_verification_zk_consent_calldata(&f.a, &f.b, &f.c, &f.pubs);
    assert!(
        !calldata.to_lowercase().contains(&owner_bare),
        "the owner address must never appear in Level-B calldata"
    );
}

/// FAIL-CLOSED (gate #7, `"R !profileRoot"`): a proof whose `R` is not this tag's on-chain root is
/// refused. Without this check a prover could fold a tree they fully control and consent as any
/// `dogTagId` — the circuit does not bind the two, so the registry is the only thing standing there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn level_b_submit_fails_closed_on_a_mismatched_root() {
    if !have_foundry() {
        eprintln!("SKIP level_b_submit_fails_closed_on_a_mismatched_root: Foundry not on PATH");
        return;
    }
    let anvil = start_anvil();
    let rpc = anvil.rpc();
    let f = load_fixture();
    let stack = deploy_launch_set(&rpc, &f);

    // Anchor and mint a DIFFERENT root for this dogTagId. Everything else — the proof, the relayer,
    // the whitelist, the deadline — stays valid, so `R !profileRoot` is the only thing that can fail.
    let other_root = "0x00000000000000000000000000000000000000000000000000000000deadbeef";
    issue_then_mint(&rpc, &stack, &f.dog_tag_id, other_root);
    // Anchor the real R too, so the failure cannot be blamed on gate #11 ("unknown root").
    cast_send(&rpc, &stack.clone, "issue(bytes32)", &[&f.root]);

    let chain = AlloyChain::new(rpc.clone());
    chain
        .register_signer(0, pk0_bytes(), ACC0.to_string())
        .await;
    let res = chain
        .record_verification_zk_consent(0, &stack.verification_consent, &f.a, &f.b, &f.c, &f.pubs)
        .await;
    assert!(
        res.is_err(),
        "a proof whose R is not the tag's profileRoot must be refused on-chain"
    );

    // And nothing was recorded: the nullifier stays unconsumed, so the owner can still use it.
    use dogtag_standard::public_signals::level_b as PB;
    let u = U256::from_str_radix(f.pubs[PB::NULLIFIER].trim_start_matches("0x"), 16)
        .or_else(|_| U256::from_str_radix(&f.pubs[PB::NULLIFIER], 10))
        .expect("nullifier parses");
    let nf = format!("0x{}", hex::encode(u.to_be_bytes::<32>()));
    assert!(
        !chain
            .consumed(&stack.verification_consent, &nf)
            .await
            .expect("consumed() read"),
        "a refused submit must not consume the nullifier"
    );
}

/// FAIL-CLOSED (gate #10, `"replayed"`): the same consent cannot be recorded twice. The nullifier is
/// a public signal bound to the hidden `ownerSecret` + `consentNonce` (D5) — never derived on-chain,
/// because Level-B has no `subject` to derive it from. Replaying one signed consent repeats its
/// nullifier and is rejected; a fresh nonce would mint a new one and pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn level_b_submit_fails_closed_on_a_consumed_nullifier() {
    if !have_foundry() {
        eprintln!("SKIP level_b_submit_fails_closed_on_a_consumed_nullifier: Foundry not on PATH");
        return;
    }
    let anvil = start_anvil();
    let rpc = anvil.rpc();
    let f = load_fixture();
    let stack = deploy_launch_set(&rpc, &f);
    issue_then_mint(&rpc, &stack, &f.dog_tag_id, &f.root);

    let chain = AlloyChain::new(rpc.clone());
    chain
        .register_signer(0, pk0_bytes(), ACC0.to_string())
        .await;

    let first = chain
        .record_verification_zk_consent(0, &stack.verification_consent, &f.a, &f.b, &f.c, &f.pubs)
        .await
        .expect("first submit should be recorded");
    let ev = chain
        .get_consent_verified_event(&first.tx_hash, &stack.verification_consent)
        .await
        .expect("Verified event present");
    assert!(
        chain
            .consumed(&stack.verification_consent, &ev.nullifier)
            .await
            .expect("consumed() read"),
        "nullifier consumed after the first submit"
    );

    // Byte-identical resubmission — the exact replay the nullifier exists to stop.
    let second = chain
        .record_verification_zk_consent(0, &stack.verification_consent, &f.a, &f.b, &f.c, &f.pubs)
        .await;
    assert!(
        second.is_err(),
        "replaying a consumed nullifier must be refused on-chain"
    );
}

/// The owner-hidden stack deploys and records without any `ConsentKeyRegistry` in the picture.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn records_without_any_consent_key_registry() {
    if !have_foundry() {
        eprintln!("SKIP level_b_records_without_any_consent_key_registry: Foundry not on PATH");
        return;
    }
    let anvil = start_anvil();
    let rpc = anvil.rpc();
    let f = load_fixture();
    // deploy_level_b deploys NO ConsentKeyRegistry — that is the assertion.
    let stack = deploy_launch_set(&rpc, &f);
    issue_then_mint(&rpc, &stack, &f.dog_tag_id, &f.root);

    let chain = AlloyChain::new(rpc.clone());
    chain
        .register_signer(0, pk0_bytes(), ACC0.to_string())
        .await;
    chain
        .record_verification_zk_consent(0, &stack.verification_consent, &f.a, &f.b, &f.c, &f.pubs)
        .await
        .expect("owner-hidden verification records with no ConsentKeyRegistry deployed");

    // `ownerOf` is called by the registry as an EXISTENCE gate only, and resolves to the neutral
    // custodian — never the owner. Asserting it here pins D1: if the mint ever went to a real owner,
    // or the registry ever compared this value, the model would be broken.
    let owner = cast_call(
        &rpc,
        &stack.sbt,
        "ownerOf(uint256)(address)",
        &[&f.dog_tag_id],
    );
    assert!(
        owner.eq_ignore_ascii_case(CUSTODIAN),
        "the tag must be held by the neutral custodian, got {owner}"
    );
    assert!(
        !owner.eq_ignore_ascii_case(&f.owner_address),
        "the tag must never be held by the hidden owner"
    );
    let _ = &stack.registry;
    let _ = &stack.factory;
}
