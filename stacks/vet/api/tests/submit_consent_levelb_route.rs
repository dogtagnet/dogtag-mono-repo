//! The Level-B submit HANDLER (`POST /verify/consent/levelb`, M-3) against `MemChain`.
//!
//! Companion to `submit_consent_onchain.rs`, which proves a real proof reaches a real registry.
//! These tests cover what that one cannot: the SERVER preflight, and the ways it must fail closed
//! before the relayer pays gas. They need no Foundry, so they run everywhere.
//!
//! The preflight deliberately mirrors the registry's own requires. It is NOT the security boundary —
//! the on-chain gates are, and they run again regardless (the trust root for Level-B is
//! `zkVerifier.verifyProof` on-chain). Its only job is to stop the relayer broadcasting a tx that
//! cannot mine. That is why every case below asserts a 4xx BEFORE any chain write, not a revert
//! after one.
//!
//! `MemChain` does not verify Groth16, so the `a/b/c` here are placeholders and the public signals do
//! the talking — which is exactly the surface these tests are about. Anything requiring real proof
//! verification or the on-chain gates (bad root, the 12-gate matrix) lives in
//! `submit_consent_onchain.rs`; `MemChain` would wrongly succeed on those.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};

use dogtag_standard::public_signals::level_b as PB;
use vet_api::chain::{ChainClient, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";

/// A well-formed Level-B public-signal vector, built through the NAMED indices so a reordering of the
/// circuit's outputs moves this fixture rather than silently producing a vector that means something
/// else.
fn pubs_for(relayer: &str, deadline: u64) -> [String; 7] {
    let mut p: [String; 7] = std::array::from_fn(|_| "0".to_string());
    p[PB::DOG_TAG_ID] = "424242".to_string();
    p[PB::PURPOSE] = purpose_field("GROOMING_INTAKE");
    p[PB::RELAYER] = u256_dec_of_addr(relayer);
    // Distinct, recognizable values so a test can prove WHICH slot was read.
    p[PB::NULLIFIER] = "1111111111111111111111".to_string();
    p[PB::ROOT] = "2222222222222222222222".to_string();
    p[PB::RECORD_TYPE] = purpose_field("VACCINATION");
    p[PB::DEADLINE] = deadline.to_string();
    p
}

/// keccak256(label) reduced mod BN254 r — the field-element form every label takes on this path.
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

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn body_for(pubs: &[String; 7]) -> Value {
    json!({
        "proof": {
            "a": ["1", "2"],
            "b": [["3", "4"], ["5", "6"]],
            "c": ["7", "8"],
            "pubSignals": pubs,
        }
    })
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

/// Boot the app, unlock custody, and whitelist the resulting relayer for the purpose so the
/// preflight's whitelist read passes. Returns `(app, operator_token, relayer_addr)`.
async fn boot(chain: Arc<MemChain>) -> (axum::Router, String, String) {
    let app = vet_api::router(mem_state(chain.clone() as Arc<dyn ChainClient>));
    let (_admin, op, relayer) = boot_custody(&app).await;
    // The whitelist lives on the ISSUER REGISTRY (`Config::issuer_registry_addr`), not on the issuer
    // clone — the registry is what `isWhitelistedFor` is called against.
    chain.whitelist(
        REGISTRY,
        &verify_key_word(&purpose_field("GROOMING_INTAKE")),
        &relayer,
    );
    (app, op, relayer)
}

/// `keccak256(abi.encode("VERIFY:", purpose))` from a decimal purpose field element.
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

// ------------------------------------------------------------------------------------------------

/// The happy path, and the E-1 guard that motivates the whole shared-constants module: the one-time
/// check must key on `pub[3]`, the LEVEL-B nullifier — not `pub[4]`, which is `R` here.
///
/// This is the single most valuable assertion in the file. Reading the wrong slot compiles, runs, and
/// returns a plausible field element; the visible symptom is a SUCCESSFUL verification that the app
/// polls as forever-unconsumed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_level_b_proof_records_and_consumes_the_pub3_nullifier() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain.clone()).await;
    let pubs = pubs_for(&relayer, now_secs() + 3600);

    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/levelb",
        Some(&op),
        Some(body_for(&pubs)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "level-b submit: {b}");
    assert_eq!(b["recorded"], true);
    assert_eq!(b["level"], "level-b");
    assert_eq!(b["protocolVersion"], "dogtag-levelb/1");
    assert_eq!(b["consumed"], true, "the nullifier must read back consumed");

    // The nullifier is pub[3]'s value, NOT pub[4]'s.
    let want_nf = {
        use alloy::primitives::U256;
        let u = U256::from_str_radix(&pubs[PB::NULLIFIER], 10).unwrap();
        format!("0x{}", hex::encode(u.to_be_bytes::<32>()))
    };
    let got_nf = b["nullifier"].as_str().expect("nullifier").to_lowercase();
    assert_eq!(got_nf, want_nf, "nullifier must come from pub[3]");
    let root_as_nf = {
        use alloy::primitives::U256;
        let u = U256::from_str_radix(&pubs[PB::ROOT], 10).unwrap();
        format!("0x{}", hex::encode(u.to_be_bytes::<32>()))
    };
    assert_ne!(
        got_nf, root_as_nf,
        "reading pub[4] as the nullifier is the E-1 bug"
    );

    // OWNER-BLIND: the response echoes the Level-B event, which has no `subject`. There must be no
    // owner-shaped key anywhere in it.
    let verified = &b["verified"];
    assert!(verified.is_object(), "verified event echoed: {b}");
    assert!(
        verified.get("subject").is_none(),
        "the Level-B response must never carry a subject"
    );
    assert_eq!(verified["deadline"], pubs[PB::DEADLINE].as_str());
}

/// Replaying the same consent is refused. `MemChain` mirrors the registry's
/// `require(!consumed[nf], "replayed")`, so this pins the server surfacing that rather than
/// reporting a second success.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replaying_a_consumed_nullifier_is_refused() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain.clone()).await;
    let pubs = pubs_for(&relayer, now_secs() + 3600);

    let (s, _) = call(
        &app,
        "POST",
        "/verify/consent/levelb",
        Some(&op),
        Some(body_for(&pubs)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s2, b2) = call(
        &app,
        "POST",
        "/verify/consent/levelb",
        Some(&op),
        Some(body_for(&pubs)),
    )
    .await;
    assert_ne!(s2, StatusCode::OK, "a replay must not succeed: {b2}");
    assert!(
        b2["error"]
            .as_str()
            .unwrap_or_default()
            .contains("replayed"),
        "the on-chain revert reason should surface, got {b2}"
    );
}

/// A proof that names a DIFFERENT relayer is refused before any broadcast.
///
/// This is the trust model, and it is cryptographic rather than configurable: `relayer` is bound into
/// both the EdDSA consent message and the nullifier, so the owner consents to ONE submitter. We
/// cannot submit on someone else's behalf even if we wanted to — the chain would reject it at
/// `"not relayer"` — so refusing early only saves gas.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_proof_naming_another_relayer_is_refused() {
    let chain = Arc::new(MemChain::new());
    let (app, op, _relayer) = boot(chain.clone()).await;
    let pubs = pubs_for(
        "0x00000000000000000000000000000000000000ee",
        now_secs() + 3600,
    );

    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/levelb",
        Some(&op),
        Some(body_for(&pubs)),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
}

/// An expired (or about-to-expire) `deadline` is refused.
///
/// Level-B's deadline is `pub[6]` — PROOF-BOUND and device-chosen — so unlike Level-A the relayer
/// cannot widen it to buy broadcast time. Re-encoding cannot save the tx; only a fresher proof can.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_deadline_is_refused() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain.clone()).await;

    for deadline in [now_secs().saturating_sub(1), now_secs() + 5] {
        let pubs = pubs_for(&relayer, deadline);
        let (s, b) = call(
            &app,
            "POST",
            "/verify/consent/levelb",
            Some(&op),
            Some(body_for(&pubs)),
        )
        .await;
        assert_eq!(
            s,
            StatusCode::BAD_REQUEST,
            "deadline {deadline} should be refused: {b}"
        );
    }
}

/// Art. 9 (§11.9(h)): `SERVICE_ATTESTATION` has no on-chain root and is not verifiable on-chain. The
/// contract compares against the REDUCED keccak; so does the preflight.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_service_attestation_record_type_is_refused() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain.clone()).await;
    let mut pubs = pubs_for(&relayer, now_secs() + 3600);
    pubs[PB::RECORD_TYPE] = purpose_field("SERVICE_ATTESTATION");

    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/levelb",
        Some(&op),
        Some(body_for(&pubs)),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b["error"].as_str().unwrap_or_default().contains("art9"));
}

/// A public signal at or above BN254 r is refused rather than silently wrapped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_out_of_field_public_signal_is_refused() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain.clone()).await;
    let mut pubs = pubs_for(&relayer, now_secs() + 3600);
    // Exactly r — the smallest value the field cannot represent.
    pubs[PB::NULLIFIER] =
        "21888242871839275222246405745257275088548364400416034343698204186575808495617".to_string();

    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/levelb",
        Some(&op),
        Some(body_for(&pubs)),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b["error"].as_str().unwrap_or_default().contains("!field"));
}

/// A `relayer` signal that fits the field but NOT 160 bits is refused (audit L1).
///
/// `uint160(pub[2])` on-chain truncates the high bits, so a signal of the form `relayer + k·2^160`
/// would narrow to our address and pass a naive comparison while reverting `"addr range"`. The
/// preflight therefore range-checks the FULL field element before narrowing — matching what the
/// proof actually witnessed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_relayer_signal_above_2_160_is_refused_even_when_it_truncates_to_us() {
    use alloy::primitives::U256;
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain.clone()).await;
    let mut pubs = pubs_for(&relayer, now_secs() + 3600);

    // relayer + 2^160 — narrows to exactly our address, but is out of range.
    let aliased: U256 = U256::from_str_radix(relayer.trim_start_matches("0x"), 16).unwrap()
        + (U256::from(1u8) << 160);
    pubs[PB::RELAYER] = aliased.to_string();

    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/levelb",
        Some(&op),
        Some(body_for(&pubs)),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap_or_default()
            .contains("addr range"),
        "got {b}"
    );
}

/// An unconfigured Level-B registry fails closed BEFORE anything is broadcast.
///
/// `chain::parse_addr` coerces an unparseable address to the zero address, and a tx to a codeless
/// address SUCCEEDS — so without this edge check a half-wired stack would spend gas and only discover
/// the problem at read-back. Mirrors M-2's `unconfigured_level_b_refuses_...`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unconfigured_level_b_registry_fails_closed() {
    let chain = Arc::new(MemChain::new());
    let mut st = mem_state(chain.clone() as Arc<dyn ChainClient>);
    let mut cfg = (*st.cfg).clone();
    cfg.verification_registry_consent_addr = String::new();
    st.cfg = Arc::new(cfg);
    let app = vet_api::router(st);
    let (_admin, op, relayer) = boot_custody(&app).await;

    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/levelb",
        Some(&op),
        Some(body_for(&pubs_for(&relayer, now_secs() + 3600))),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    assert!(b["error"]
        .as_str()
        .unwrap_or_default()
        .contains("not configured"));
}

/// The route is operator-gated. The on-chain gates decide whether a proof is VALID, but the relayer
/// pays gas to find out — so an open endpoint would be a free way to drain its balance on proofs that
/// revert. Note this is only a SPEND gate: it grants no authority over the submission, because the
/// proof names its own submitter.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_level_b_route_requires_an_operator() {
    let chain = Arc::new(MemChain::new());
    let app = vet_api::router(mem_state(chain as Arc<dyn ChainClient>));
    let pubs = pubs_for(
        "0x00000000000000000000000000000000000000ee",
        now_secs() + 3600,
    );

    let (s, _) = call(
        &app,
        "POST",
        "/verify/consent/levelb",
        None,
        Some(body_for(&pubs)),
    )
    .await;
    assert!(
        s == StatusCode::UNAUTHORIZED || s == StatusCode::FORBIDDEN,
        "unauthenticated submit should be refused, got {s}"
    );
}

/// A malformed proof body is rejected at the edge, not turned into a broadcast.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_proof_is_rejected() {
    let chain = Arc::new(MemChain::new());
    let (app, op, _relayer) = boot(chain.clone()).await;

    for bad in [
        json!({}),
        json!({"proof": {"a": ["1"], "b": [["3","4"],["5","6"]], "c": ["7","8"], "pubSignals": ["0","0","0","0","0","0","0"]}}),
        // six signals — the width is shared by BOTH circuits, so a length check is the one drift a
        // reader CAN catch cheaply.
        json!({"proof": {"a": ["1","2"], "b": [["3","4"],["5","6"]], "c": ["7","8"], "pubSignals": ["0","0","0","0","0","0"]}}),
    ] {
        let (s, b) = call(&app, "POST", "/verify/consent/levelb", Some(&op), Some(bad)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    }
}

/// The Level-A route is untouched and still mounted alongside. Guards the non-scope boundary: M-3
/// adds the Level-B path BESIDE Level-A, it does not cut over.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_level_a_consent_route_is_still_mounted() {
    let chain = Arc::new(MemChain::new());
    let (app, op, _relayer) = boot(chain.clone()).await;

    // An empty body on the Level-A route is a 4xx from ITS OWN handler — the point is that the route
    // still resolves (a removed route would 404).
    let (s, _) = call(
        &app,
        "POST",
        "/verify/consent/submit",
        Some(&op),
        Some(json!({})),
    )
    .await;
    assert_ne!(s, StatusCode::NOT_FOUND, "Level-A route must stay mounted");
}
