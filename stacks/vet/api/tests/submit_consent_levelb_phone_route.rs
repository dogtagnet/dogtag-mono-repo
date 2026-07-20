//! The M-4 PHONE path for Level-B: `POST /v1/verify/consent/levelb` + the per-session opt-in.
//!
//! Companion to `submit_consent_levelb_route.rs`, which covers the operator-gated M-3 route and its
//! preflight. This file covers only what M-4 adds, and the two things it must NOT break:
//!
//!   1. the export-token gate that makes the route reachable by the phone at all,
//!   2. the `mode="levelb"` per-session opt-in that advertises Level-B in the convenience tier,
//!   3. **the DEFAULT staying Level-A** — the single most important assertion here. Making Level-B
//!      AVAILABLE must not make it the DEFAULT; flipping the default is M-5, not M-4. A regression
//!      that flipped it would be invisible in every Level-B test and would strand every shipped
//!      Level-A phone, so it is asserted explicitly rather than left implied.
//!
//! `MemChain` does not verify Groth16, so `a/b/c` are placeholders and the public signals do the
//! talking — the same division of labour as the M-3 route tests.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};

use dogtag_standard::public_signals::level_b as PB;
use vet_api::chain::{ChainClient, MemChain};

const REGISTRY: &str = "0x00000000000000000000000000000000000000aa";
const ISSUER: &str = "0x00000000000000000000000000000000000000bb";
const PURPOSE: &str = "GROOMING_INTAKE";

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

/// Built through the NAMED Level-B indices, so a circuit reordering moves this fixture rather than
/// silently producing a vector that means something else.
fn pubs_for(relayer: &str, purpose_label: &str) -> [String; 7] {
    let mut p: [String; 7] = std::array::from_fn(|_| "0".to_string());
    p[PB::DOG_TAG_ID] = "424242".to_string();
    p[PB::PURPOSE] = purpose_field(purpose_label);
    p[PB::RELAYER] = u256_dec_of_addr(relayer);
    p[PB::NULLIFIER] = "1111111111111111111111".to_string();
    p[PB::ROOT] = "2222222222222222222222".to_string();
    p[PB::RECORD_TYPE] = purpose_field("VACCINATION");
    p[PB::DEADLINE] = (now_secs() + 3600).to_string();
    p
}

fn proof_body(pubs: &[String; 7], export_token: Option<&str>) -> Value {
    let mut b = json!({
        "proof": {
            "a": ["1", "2"],
            "b": [["3", "4"], ["5", "6"]],
            "c": ["7", "8"],
            "pubSignals": pubs,
        }
    });
    if let Some(t) = export_token {
        b["exportToken"] = json!(t);
    }
    b
}

async fn boot(chain: Arc<MemChain>) -> (axum::Router, String, String) {
    let (app, op, relayer, _st) = boot_with_state(chain).await;
    (app, op, relayer)
}

/// `boot`, but keeping the `AppState` so a test can reach the STORE directly. Needed to construct
/// states the HTTP surface alone cannot produce — notably an export token mapped to a session row
/// that does not exist, which is what a `MongoStore` read failure looks like to the handler.
async fn boot_with_state(
    chain: Arc<MemChain>,
) -> (axum::Router, String, String, vet_api::app::AppState) {
    let st = state_with(
        chain.clone() as Arc<dyn ChainClient>,
        "memchain".to_string(),
        REGISTRY.to_string(),
        ISSUER.to_string(),
        "vet.example".to_string(),
        1,
    );
    let app = vet_api::router(st.clone());
    let (_admin, op, relayer) = boot_custody(&app).await;
    chain.whitelist(
        REGISTRY,
        &verify_key_word(&purpose_field(PURPOSE)),
        &relayer,
    );
    (app, op, relayer, st)
}

/// Start a session in `mode` and return `(sessionId, exportToken)`. The token is carried in the QR
/// URL (`/x/<token>?a=<relayer>`), which is exactly where the phone reads it from.
async fn start_session(app: &axum::Router, op: &str, mode: &str) -> (String, String) {
    let (s, b) = call(
        app,
        "POST",
        "/verify/session/start",
        Some(op),
        Some(json!({"purpose": PURPOSE, "recordType": "VACCINATION", "mode": mode})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "session start ({mode}): {b}");
    let session_id = b["sessionId"].as_str().expect("sessionId").to_string();
    let qr = b["qrUrl"].as_str().expect("qrUrl");
    let token = qr
        .rsplit('/')
        .next()
        .and_then(|t| t.split('?').next())
        .expect("token in qrUrl")
        .to_string();
    (session_id, token)
}

/// Poll an audit row to its terminal state. The handler acks `"recording"` and broadcasts from a
/// detached task, so the ack says nothing about the outcome — asserting on it would pass vacuously.
async fn settle(app: &axum::Router, op: &str, session_id: &str) -> Value {
    for _ in 0..200 {
        let (s, row) = call(
            app,
            "GET",
            &format!("/verify/session/{session_id}"),
            Some(op),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "session lookup: {row}");
        if row["status"] != "recording" {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("session {session_id} never left recording");
}

// ------------------------------------------------------------------------------------------------
// The per-session opt-in — and the default that must not move
// ------------------------------------------------------------------------------------------------

/// A `levelb` session advertises Level-B in the convenience tier, on the SAME resolve GET the phone
/// already hits for its QR. Both fields must move together: an app that resolves the anchor for
/// `dogtag-levelb/1` and finds the Level-A registry beside it fails closed with `RegistryMismatch`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn levelb_session_advertises_levelb_claims() {
    let chain = Arc::new(MemChain::new());
    let (app, op, _relayer) = boot(chain).await;
    let (_sid, token) = start_session(&app, &op, "levelb").await;

    let (s, b) = call(&app, "GET", &format!("/x/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "resolve: {b}");
    let claims = &b["unverifiedClaims"];
    assert_eq!(claims["protocolVersion"], "dogtag-levelb/1", "{b}");
    assert_eq!(
        claims["verificationRegistry"].as_str().unwrap(),
        common::VREG_CONSENT_ADDR,
        "the Level-B registry must accompany the Level-B version: {b}"
    );
    assert_eq!(b["mode"], "levelb", "{b}");
}

/// **The load-bearing regression guard.** M-4 makes Level-B AVAILABLE, not DEFAULT. A plain `zk`
/// session — what every shipped phone starts today — must still advertise Level-A, unchanged. If
/// this ever fails, the version-stamp flip (M-5) has leaked into M-4 and every installed Level-A
/// binary is stranded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_session_still_advertises_levela() {
    let chain = Arc::new(MemChain::new());
    let (app, op, _relayer) = boot(chain).await;
    let (_sid, token) = start_session(&app, &op, "zk").await;

    let (s, b) = call(&app, "GET", &format!("/x/{token}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "resolve: {b}");
    let claims = &b["unverifiedClaims"];
    assert_eq!(
        claims["protocolVersion"], "dogtag-levela/1",
        "the DEFAULT must stay Level-A — flipping it is M-5, not M-4: {b}"
    );
    assert_ne!(
        claims["verificationRegistry"].as_str().unwrap(),
        common::VREG_CONSENT_ADDR,
        "a `zk` session must not be pointed at the Level-B registry: {b}"
    );
}

/// An unrecognised mode is refused at session start. Before this it fell through the
/// `if mode == "normal"` dispatch into the LEVEL-A branch, so `"level-b"` or `"levelB"` would have
/// quietly produced a Level-A session advertising Level-A claims.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_mode_is_refused_at_session_start() {
    let chain = Arc::new(MemChain::new());
    let (app, op, _relayer) = boot(chain).await;
    for bad in ["level-b", "levelB", "LEVELB", "zk-levelb", ""] {
        let (s, b) = call(
            &app,
            "POST",
            "/verify/session/start",
            Some(&op),
            Some(json!({"purpose": PURPOSE, "recordType": "VACCINATION", "mode": bad})),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "mode {bad:?} should 400: {b}");
    }
}

// ------------------------------------------------------------------------------------------------
// The phone gate
// ------------------------------------------------------------------------------------------------

/// The whole point of the twin: the phone holds NO operator session, only its one-time export token,
/// and can submit with it. It also proves the row is the SESSION's own — not a second minted one —
/// so the operator trail holds one row per verification and the phone polls the id its QR named.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn phone_submits_with_export_token_and_settles_on_the_session_row() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain).await;
    let (sid, token) = start_session(&app, &op, "levelb").await;

    let (s, ack) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None, // no operator bearer — the token alone must satisfy the gate
        Some(proof_body(&pubs_for(&relayer, PURPOSE), Some(&token))),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "phone submit: {ack}");
    assert_eq!(ack["status"], "recording", "{ack}");
    assert_eq!(
        ack["sessionId"].as_str().unwrap(),
        sid,
        "the phone must get back the session its QR named, not a freshly minted one: {ack}"
    );

    let row = settle(&app, &op, &sid).await;
    assert_eq!(row["status"], "recorded", "row: {row}");
    // The E-1 guard: the row must key on pub[3] (the Level-B NULLIFIER), never pub[4] (which holds
    // R here). Reading the wrong slot compiles and yields a plausible field element, so assert the
    // recorded value is the nullifier's and specifically NOT the root's.
    let word = |dec: &str| {
        use alloy::primitives::U256;
        format!(
            "0x{}",
            hex::encode(U256::from_str_radix(dec, 10).unwrap().to_be_bytes::<32>())
        )
    };
    let pubs = pubs_for(&relayer, PURPOSE);
    assert_eq!(row["nullifier"], word(&pubs[PB::NULLIFIER]), "row: {row}");
    assert_ne!(
        row["nullifier"],
        word(&pubs[PB::ROOT]),
        "the row keyed on R (pub[4]) instead of the nullifier (pub[3]) — this is E-1: {row}"
    );
}

/// No operator session and no export token: refused. The twin must not be an ungated way to spend
/// the relayer's gas.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn phone_route_refuses_unauthenticated() {
    let chain = Arc::new(MemChain::new());
    let (app, _op, relayer) = boot(chain).await;
    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None,
        Some(proof_body(&pubs_for(&relayer, PURPOSE), None)),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{b}");
}

// ------------------------------------------------------------------------------------------------
// The two routes cannot be crossed
// ------------------------------------------------------------------------------------------------

/// A `zk` (Level-A) session's token must not buy a Level-B submission. Both routes read the SAME
/// export token, so without the mode check the token would be accepted by both.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn levela_session_is_refused_on_the_levelb_route() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain).await;
    let (_sid, token) = start_session(&app, &op, "zk").await;

    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None,
        Some(proof_body(&pubs_for(&relayer, PURPOSE), Some(&token))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"].as_str().unwrap_or_default().contains("level-b"),
        "the error should name the mode mismatch: {b}"
    );
}

/// The mirror: a `levelb` session must not be served by the LEVEL-A route. Without the guard it
/// falls through `if mode == "normal" { .. } else { <Level-A ZK> }` into the Level-A branch, which
/// reads `pub[SUBJECT]`/`pub[KEY_HASH]` and encodes the Level-A selector — i.e. it interprets an
/// owner-hidden proof with Level-A indices.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn levelb_session_is_refused_on_the_levela_route() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain).await;
    let (sid, _token) = start_session(&app, &op, "levelb").await;

    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/submit",
        Some(&op),
        Some(json!({
            "sessionId": sid,
            "consent": {
                "dogTagId": "42",
                "recordType": "0x00",
                "purpose": "0x00",
                "credentialRoot": "0x11",
                "challenge": "0x00",
                "relayer": relayer,
                "subject": "0x00000000000000000000000000000000000000cc",
                "nonce": "1",
                "deadline": (now_secs() + 300)
            },
            "sig": "0x00",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap_or_default()
            .contains("/v1/verify/consent/levelb"),
        "the error should route the caller to the Level-B path: {b}"
    );
}

/// The refusal above must read the session's STORED mode, never the request body's `mode` override.
/// A caller holding a `levelb` session's credentials can otherwise pass `"mode":"zk"` and land in
/// the Level-A ZK branch anyway — which reads `pub[SUBJECT]`/`pub[KEY_HASH]` and encodes the Level-A
/// selector, i.e. the exact E-1 misinterpretation the refusal exists to close.
///
/// The sibling test omits `mode` entirely, which is precisely why this gap survived it. `mode` keeps
/// its legitimate job (normal vs zk WITHIN a Level-A session); it just must not pick the LEVEL.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn levelb_session_is_refused_on_the_levela_route_even_with_a_mode_override() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain).await;
    let (sid, _token) = start_session(&app, &op, "levelb").await;

    for override_mode in ["zk", "normal"] {
        let (s, b) = call(
            &app,
            "POST",
            "/verify/consent/submit",
            Some(&op),
            Some(json!({
                "sessionId": sid,
                "mode": override_mode,
                "consent": {
                    "dogTagId": "42",
                    "recordType": "0x00",
                    "purpose": "0x00",
                    "credentialRoot": "0x11",
                    "challenge": "0x00",
                    "relayer": relayer,
                    "subject": "0x00000000000000000000000000000000000000cc",
                    "nonce": "1",
                    "deadline": (now_secs() + 300)
                },
                "sig": "0x00",
            })),
        )
        .await;
        assert_eq!(
            s,
            StatusCode::BAD_REQUEST,
            "mode override {override_mode:?} must not reach the Level-A branch: {b}"
        );
        assert!(
            b["error"]
                .as_str()
                .unwrap_or_default()
                .contains("/v1/verify/consent/levelb"),
            "the refusal must be the LEVEL-B routing error, not an incidental \
             downstream failure inside the Level-A branch: {b}"
        );
    }
}

// ------------------------------------------------------------------------------------------------
// The session binding — the export token is a GAS-SPEND capability, scoped to its session
// ------------------------------------------------------------------------------------------------

/// A proof for a different purpose than the session's is refused. The token authorizes the relayer
/// to pay for the verification its QR named, and nothing else.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proof_purpose_must_match_the_session() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain).await;
    let (_sid, token) = start_session(&app, &op, "levelb").await;

    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None,
        Some(proof_body(
            &pubs_for(&relayer, "SOME_OTHER_PURPOSE"),
            Some(&token),
        )),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"].as_str().unwrap_or_default().contains("purpose"),
        "{b}"
    );
}

/// A proof asserting a record type the session never named is refused — the third binding axis.
///
/// `recordType` is PROVER-asserted, not consent-signed, so unlike purpose it is pinned by nothing
/// upstream: the on-chain whitelist key derives from `purpose`, so the chain would happily record a
/// verification of some other record type. Two costs, both real: the relayer pays gas for a proof
/// the operator did not request, and the audit row — which keeps the SESSION's human-readable label
/// — would then name a record type that was never proved.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proof_record_type_must_match_the_session() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain).await;
    // The session is started for VACCINATION (see `start_session`).
    let (_sid, token) = start_session(&app, &op, "levelb").await;

    let mut pubs = pubs_for(&relayer, PURPOSE);
    pubs[PB::RECORD_TYPE] = purpose_field("EU_HEALTH_CERT");

    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None,
        Some(proof_body(&pubs, Some(&token))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap_or_default()
            .contains("recordType"),
        "{b}"
    );
}

/// The binding must compare the REDUCED keccak, so the matching case has to actually PASS — a guard
/// built on the raw `rt_key` word would reject every record type whose keccak exceeds r, and this
/// test would catch that where the mismatch test above would not (it would "pass" vacuously).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn matching_record_type_passes_the_binding() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain).await;
    let (sid, token) = start_session(&app, &op, "levelb").await;

    let (s, ack) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None,
        Some(proof_body(&pubs_for(&relayer, PURPOSE), Some(&token))),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "a proof whose recordType matches the session must not be refused: {ack}"
    );
    let row = settle(&app, &op, &sid).await;
    assert_eq!(row["status"], "recorded", "{row}");
}

/// The phone route FAILS CLOSED when a token-authed session cannot be read.
///
/// Falling through to `session = None` would silently demote the request to the COLD path, skipping
/// the mode refusal, all three binding axes AND the status replay guard — and the cold path is the
/// one with no replay protection at all, since this route never consumes the token. That is not
/// hypothetical: `MongoStore::get_session` maps driver errors to `None` (`.ok().flatten()`), so a
/// transient DB blip is indistinguishable here from "no such session".
///
/// The orphan token is minted through the store directly because the HTTP surface cannot produce
/// this state — which is exactly why it went unnoticed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn token_authed_submit_fails_closed_when_the_session_cannot_be_read() {
    let chain = Arc::new(MemChain::new());
    let (app, _op, relayer, st) = boot_with_state(chain).await;

    let token = "orphan-export-token";
    st.store
        .put_export_token(token, "session-that-does-not-exist", now_secs() + 600)
        .await;

    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None,
        Some(proof_body(&pubs_for(&relayer, PURPOSE), Some(token))),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_GATEWAY,
        "an unreadable session must fail closed, never fall through to the cold mint: {b}"
    );
}

/// The operator half of the same rule: naming a sessionId that does not exist is refused rather than
/// quietly minting an unrelated cold row. Cold entry stays available — it is simply the caller
/// OMITTING `sessionId` (the operator-gated `/verify/consent/levelb` route), not naming a bad one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_submit_naming_an_unknown_session_is_refused() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain).await;

    let mut body = proof_body(&pubs_for(&relayer, PURPOSE), None);
    body["sessionId"] = json!("no-such-session");
    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        Some(&op),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{b}");

    // The cold path is still reachable — by omitting sessionId entirely.
    let (s, b) = call(
        &app,
        "POST",
        "/verify/consent/levelb",
        Some(&op),
        Some(proof_body(&pubs_for(&relayer, PURPOSE), None)),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "the cold operator path must be intact: {b}"
    );
}

/// The other half of the one-time-token property, and the one that actually blocks replay: after a
/// SUCCEEDING submit, the SAME token must not buy a second one.
///
/// This is the discriminator for how the phone route mirrors Level-A. Level-A consumes the token in
/// its background task on record success; the Level-B route instead relies on the session row having
/// left `"pending"` (it is persisted `"recording"` BEFORE the broadcast is spawned, so the window is
/// closed synchronously). The two are equivalent in effect only if the guard actually holds — which
/// nothing asserted until this test. Without it, a replay would spend the relayer's gas a second
/// time on a proof the chain then reverts as `"replayed"`.
///
/// The equivalence also rests on a session outliving its token: a session that vanished while its
/// token was live would fall through to the COLD mint path, which has no status guard. Sessions are
/// never deleted (there is no delete/GC path in `store`) while tokens expire at 600s, so that cannot
/// happen — but it is the invariant this test's guarantee depends on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_successful_submission_cannot_be_replayed_with_the_same_token() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain).await;
    let (sid, token) = start_session(&app, &op, "levelb").await;

    let (s, ack) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None,
        Some(proof_body(&pubs_for(&relayer, PURPOSE), Some(&token))),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "first submit: {ack}");
    let row = settle(&app, &op, &sid).await;
    assert_eq!(
        row["status"], "recorded",
        "first submit did not record: {row}"
    );

    // Same token, same proof, second time.
    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None,
        Some(proof_body(&pubs_for(&relayer, PURPOSE), Some(&token))),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "a settled session must refuse a replay on the same token: {b}"
    );
}

/// A proof naming a different relayer than the session's is refused.
///
/// NOTE this is currently proved by the PREFLIGHT (`does not name this relayer`), not by the session
/// binding: `session.relayer` is set to the custody address at session start, and the preflight
/// already requires `pub[RELAYER] == custody`, so the two comparisons coincide. The session-binding
/// relayer branch is therefore defense-in-depth that no test can currently isolate — it would start
/// to bite only if sessions ever carried a relayer other than the active custody signer. Kept
/// deliberately; the purpose half of the binding is the non-redundant one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proof_relayer_must_match_the_session() {
    let chain = Arc::new(MemChain::new());
    let (app, op, _relayer) = boot(chain).await;
    let (_sid, token) = start_session(&app, &op, "levelb").await;

    let other = "0x00000000000000000000000000000000000000ff";
    let (s, b) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None,
        Some(proof_body(&pubs_for(other, PURPOSE), Some(&token))),
    )
    .await;
    // The relayer preflight (`does not name this relayer`) fires first and is itself fail-closed;
    // either it or the session binding must refuse, and neither may let it through.
    assert!(
        s == StatusCode::BAD_REQUEST || s == StatusCode::FORBIDDEN,
        "a foreign relayer must be refused, got {s}: {b}"
    );
}

/// A FAILED submission must not burn the owner's one-time token — they retry with the same QR. This
/// is why the gate PEEKS (`consume=false`), exactly as the Level-A alias does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_submission_does_not_burn_the_export_token() {
    let chain = Arc::new(MemChain::new());
    let (app, op, relayer) = boot(chain).await;
    let (sid, token) = start_session(&app, &op, "levelb").await;

    // First attempt fails the purpose binding.
    let (s, _) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None,
        Some(proof_body(&pubs_for(&relayer, "WRONG"), Some(&token))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // The SAME token still works.
    let (s, ack) = call(
        &app,
        "POST",
        "/v1/verify/consent/levelb",
        None,
        Some(proof_body(&pubs_for(&relayer, PURPOSE), Some(&token))),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "retry with the same token: {ack}");
    let row = settle(&app, &op, &sid).await;
    assert_eq!(row["status"], "recorded", "row: {row}");
}
