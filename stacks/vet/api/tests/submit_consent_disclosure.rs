//! D1 - the OPTIONAL `profileDisclosure` alongside the owner-hidden consent submission
//! (`POST /v1/verify/consent`).
//!
//! The consent proof stays frozen and leaf-blind; the disclosure is an independent Merkle opening
//! of owner-picked `owner.identity.*` leaves against the SAME anchored `R`, bound to the proof it
//! rides with (same `R`, same `dogTagId`) so it inherits the proof's relayer/deadline/nullifier
//! anti-replay context. These tests drive the REAL envelope builder over a REAL device tree
//! (`MemChain` skips only the Groth16 check, which is not the surface under test) and pin the ways
//! the handler must fail closed BEFORE the relayer pays gas.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};

use ark_bn254::Fr;
use dogtag_standard::disclosure::build_profile_disclosure;
use dogtag_standard::field::to_hex32;
use dogtag_standard::leaf::field_of_value;
use dogtag_standard::profile_tree::{build_profile_tree, AttributeLeaf, SALT_LEN};
use dogtag_standard::public_signals::level_b as PB;
use dogtag_standard::types::TypedScalar;
use vet_api::chain::{ChainClient, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";
const DEVICE_SEED: &[u8] = b"DogTag disclosure test seed - TEST MATERIAL ONLY";
const HANDLE: &str = "424242";

fn owner_address() -> [u8; 20] {
    let mut a = [0u8; 20];
    a[16..].copy_from_slice(&0xdead_beefu32.to_be_bytes());
    a
}

fn attrs() -> Vec<AttributeLeaf> {
    vec![
        AttributeLeaf {
            key_path: "credentialSubject.name".to_string(),
            salt: [7u8; SALT_LEN],
            value: TypedScalar::Str("Rex".to_string()),
        },
        AttributeLeaf {
            key_path: "owner.identity.fullName".to_string(),
            salt: [21u8; SALT_LEN],
            value: TypedScalar::Str("Alice Owner".to_string()),
        },
        AttributeLeaf {
            key_path: "owner.identity.country".to_string(),
            salt: [22u8; SALT_LEN],
            value: TypedScalar::Str("GB".to_string()),
        },
    ]
}

fn canonical_field() -> Fr {
    field_of_value(&TypedScalar::Integer(HANDLE.to_string())).expect("canonical field")
}

/// Decimal string of a 0x.. field hex - the radix pubSignals usually arrive in.
fn dec_of_hex(h: &str) -> String {
    use alloy::primitives::U256;
    U256::from_str_radix(h.trim_start_matches("0x"), 16)
        .expect("field hex")
        .to_string()
}

fn purpose_field(label: &str) -> String {
    use alloy::primitives::{keccak256, U256};
    let r = U256::from_str_radix(
        "21888242871839275222246405745257275088548364400416034343698204186575808495617",
        10,
    )
    .unwrap();
    (U256::from_be_bytes::<32>(keccak256(label.as_bytes()).0) % r).to_string()
}

fn u256_dec_of_addr(a: &str) -> String {
    use alloy::primitives::U256;
    U256::from_str_radix(a.trim_start_matches("0x"), 16)
        .expect("addr hex")
        .to_string()
}

fn verify_key_word(purpose_dec: &str) -> String {
    use alloy::primitives::{keccak256, U256};
    let purpose = U256::from_str_radix(purpose_dec, 10).expect("purpose dec");
    let mut buf = Vec::with_capacity(128);
    let mut off = [0u8; 32];
    off[31] = 0x40;
    buf.extend_from_slice(&off);
    buf.extend_from_slice(&purpose.to_be_bytes::<32>());
    let mut len = [0u8; 32];
    len[31] = 7;
    buf.extend_from_slice(&len);
    let mut lit = [0u8; 32];
    lit[..7].copy_from_slice(b"VERIFY:");
    buf.extend_from_slice(&lit);
    format!("0x{}", hex::encode(keccak256(&buf).0))
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A consent public-signal vector whose `dogTagId`/`R` are the REAL device tree's values.
fn pubs_for(relayer: &str, root_hex: &str, nullifier: &str) -> [String; 7] {
    let mut p: [String; 7] = std::array::from_fn(|_| "0".to_string());
    p[PB::DOG_TAG_ID] = dec_of_hex(&to_hex32(&canonical_field()));
    p[PB::PURPOSE] = purpose_field("GROOMING_INTAKE");
    p[PB::RELAYER] = u256_dec_of_addr(relayer);
    p[PB::NULLIFIER] = nullifier.to_string();
    p[PB::ROOT] = dec_of_hex(root_hex);
    p[PB::RECORD_TYPE] = purpose_field("VACCINATION");
    p[PB::DEADLINE] = (now_secs() + 3600).to_string();
    p
}

fn body_for(pubs: &[String; 7], disclosure: Option<Value>) -> Value {
    let mut body = json!({
        "proof": {
            "a": ["1", "2"],
            "b": [["3", "4"], ["5", "6"]],
            "c": ["7", "8"],
            "pubSignals": pubs,
        }
    });
    if let Some(d) = disclosure {
        body["profileDisclosure"] = d;
    }
    body
}

fn mem_state(chain: Arc<dyn ChainClient>) -> vet_api::app::AppState {
    state_with(
        chain,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    )
}

/// Boot, unlock custody, whitelist the relayer, and ANCHOR the device tree on-chain exactly as
/// custodial issuance does (`mintCustodial` seal + `issue(R)` anchor). Returns
/// `(app, operator, relayer, root_hex)`.
async fn boot_with_anchored_tree(chain: Arc<MemChain>) -> (axum::Router, String, String, String) {
    let app = vet_api::router(mem_state(chain.clone() as Arc<dyn ChainClient>));
    let (_admin, op, relayer) = boot_custody(&app).await;
    chain.whitelist(
        REGISTRY,
        &verify_key_word(&purpose_field("GROOMING_INTAKE")),
        &relayer,
    );

    let tree = build_profile_tree(DEVICE_SEED, canonical_field(), &owner_address(), &attrs())
        .expect("device tree");
    let root_hex = to_hex32(&tree.root);
    let onchain_id = to_hex32(&canonical_field());
    chain
        .mint_custodial(0, SBT_CONSENT_ADDR, &onchain_id, &root_hex)
        .await
        .expect("seal profileRoot");
    chain
        .issue(0, PROFILE_ISSUER_ADDR, &root_hex)
        .await
        .expect("anchor R");
    (app, op, relayer, root_hex)
}

fn disclosure_json(reveal: &[&str]) -> Value {
    let reveal: Vec<String> = reveal.iter().map(|s| s.to_string()).collect();
    let d = build_profile_disclosure(
        DEVICE_SEED,
        canonical_field(),
        &owner_address(),
        &attrs(),
        &reveal,
    )
    .expect("build disclosure");
    serde_json::to_value(&d).expect("disclosure json")
}

async fn settle(app: &axum::Router, op: &str, session_id: &str) -> Value {
    for _ in 0..200 {
        let (ss, row) = call(
            app,
            "GET",
            &format!("/verify/session/{session_id}"),
            Some(op),
            None,
        )
        .await;
        assert_eq!(ss, StatusCode::OK, "session lookup: {row}");
        if row["status"] != "recording" {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("session {session_id} never left recording");
}

/// HAPPY PATH: a subset disclosure (country only) verifies, the revealed keyPaths are recorded,
/// and the un-revealed identity values never appear anywhere in the exchange.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_valid_subset_disclosure_is_accepted_and_recorded() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer, root_hex) = boot_with_anchored_tree(chain).await;
    let pubs = pubs_for(&relayer, &root_hex, "1111111111111111111111");

    let (s, ack) = call(
        &app,
        "POST",
        "/v1/verify/consent",
        Some(&op),
        Some(body_for(&pubs, Some(disclosure_json(&["owner.identity.country"])))),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "submit: {ack}");
    assert_eq!(
        ack["disclosedKeyPaths"],
        json!(["owner.identity.country"]),
        "the revealed keyPaths must be recorded: {ack}"
    );
    // Selective means selective: the OTHER identity value is nowhere in the response.
    assert!(
        !ack.to_string().contains("Alice Owner"),
        "unrevealed identity must not leak: {ack}"
    );

    let row = settle(&app, &op, ack["sessionId"].as_str().unwrap()).await;
    assert_eq!(row["status"], "recorded", "broadcast must succeed: {row}");
}

/// A TAMPERED value fails the pure fold and rejects the WHOLE submission before any gas is spent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tampered_disclosure_value_rejects_the_submission() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer, root_hex) = boot_with_anchored_tree(chain.clone()).await;
    let pubs = pubs_for(&relayer, &root_hex, "1111111111111111111112");

    let mut forged = disclosure_json(&["owner.identity.country"]);
    forged["disclosures"][0]["value"] = json!("US");

    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent",
        Some(&op),
        Some(body_for(&pubs, Some(forged))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "forged value must reject: {b}");
    assert!(
        b["error"].as_str().unwrap_or_default().contains("profileDisclosure"),
        "the rejection must name the disclosure: {b}"
    );
}

/// The envelope must be BOUND to the consent proof it rides with: a disclosure for a DIFFERENT `R`
/// (valid in itself) is refused - that binding is its only anti-replay context.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disclosure_for_a_different_root_than_the_proof_is_refused() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer, _root_hex) = boot_with_anchored_tree(chain).await;
    // The proof claims a DIFFERENT root than the (internally valid) disclosure envelope.
    let pubs = pubs_for(&relayer, &to_hex32(&Fr::from(999_999u64)), "1111111111111111111113");

    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent",
        Some(&op),
        Some(body_for(&pubs, Some(disclosure_json(&["owner.identity.country"])))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "unbound disclosure must reject: {b}");
    assert!(
        b["error"].as_str().unwrap_or_default().contains("consent proof"),
        "the rejection must name the binding: {b}"
    );
}

/// An UN-ANCHORED `R` is refused: the disclosure chain of trust runs disclosure -> R ->
/// profileRoot(dogTagId) -> trusted issuer, and the middle link is checked on the SBT.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disclosure_against_an_unanchored_root_is_refused() {
    let chain = Arc::new(MemChain::new());
    // Boot WITHOUT anchoring the tree: whitelist only.
    let app = vet_api::router(mem_state(chain.clone() as Arc<dyn ChainClient>));
    let (_admin, op, relayer) = boot_custody(&app).await;
    chain.whitelist(
        REGISTRY,
        &verify_key_word(&purpose_field("GROOMING_INTAKE")),
        &relayer,
    );
    let tree = build_profile_tree(DEVICE_SEED, canonical_field(), &owner_address(), &attrs())
        .expect("device tree");
    let pubs = pubs_for(&relayer, &to_hex32(&tree.root), "1111111111111111111114");

    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent",
        Some(&op),
        Some(body_for(&pubs, Some(disclosure_json(&["owner.identity.country"])))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "unanchored R must reject: {b}");
    assert!(
        b["error"].as_str().unwrap_or_default().contains("profileRoot"),
        "the rejection must name the anchor: {b}"
    );
}

/// WITHOUT a disclosure the submission behaves exactly as before D1 - the optional block is
/// genuinely optional, and the audit row records an empty reveal set.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_submission_without_a_disclosure_is_unchanged() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer, root_hex) = boot_with_anchored_tree(chain).await;
    let pubs = pubs_for(&relayer, &root_hex, "1111111111111111111115");

    let (s, ack) = call(
        &app,
        "POST",
        "/v1/verify/consent",
        Some(&op),
        Some(body_for(&pubs, None)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "plain submit must still work: {ack}");
    assert_eq!(ack["disclosedKeyPaths"], json!([]));
    let row = settle(&app, &op, ack["sessionId"].as_str().unwrap()).await;
    assert_eq!(row["status"], "recorded", "{row}");
}
