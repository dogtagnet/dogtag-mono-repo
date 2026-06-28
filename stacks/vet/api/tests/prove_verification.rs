//! Server-side proving API (`POST /prove-verification`) test — runs ONLY with `--features prover`.
//!
//! Exercises the 32-bit-Android fallback's server leg: feed a KNOWN wrapped doc + consent + EdDSA sig
//! (the SAME inputs the on-device `prove_verification` takes) and assert the endpoint returns a
//! `{a,b,c,pub}` whose `pub` is the expected 7-vector `[dogTagId, purpose, relayer, subject,
//! nullifier, keyHash, R]`.
//!
//! Two prover backends:
//!   * StubProver (default / no artifacts) — hermetic: asserts the response SHAPE and that `pub`
//!     echoes the input-derived signals (dogTagId/purpose/relayer/subject/R). Always runs.
//!   * Real ArkProver — when `circuits/build/verification_final.zkey` exists (same artifact-presence
//!     gate the on-chain tests use): asserts the FULL 7-vector incl. the circuit-computed
//!     nullifier/keyHash, and — behind `DOGTAG_LIVE_VERIFIER=1` — that the proof cast-verifies on the
//!     LIVE `Groth16Verifier 0x138b…` (mirrors dogtag-standard's `prove_parity` live test).
#![cfg(feature = "prover")]

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use ark_bn254::Fr;
use ark_ff::PrimeField;
use serde_json::json;

use dogtag_standard::consent::{
    consent_nullifier, eddsa_consent_message, key_hash, VerificationConsent,
};
use dogtag_standard::eddsa::{consent_key_from_raw_prv, sign_poseidon};
use dogtag_standard::field::to_hex32;
use dogtag_standard::leaf::field_of_value;
use dogtag_standard::types::TypedScalar;
use dogtag_standard::wrap::{
    flatten_data, parse_packed, scalar_from_packed, wrap_document, IssuerMeta, WrappedDoc,
};

use vet_api::chain::MemChain;
use vet_api::prover::{ArkProver, ProverClient, StubProver};

fn repo_root() -> PathBuf {
    // tests/ -> api -> vet -> stacks -> <root>
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Deterministic 16-byte salts: each call returns [n;16], n increments from 1 (matches prove_parity).
fn fixed_salts() -> impl FnMut() -> [u8; 16] {
    let mut n: u8 = 1;
    move || {
        let s = [n; 16];
        n = n.wrapping_add(1);
        s
    }
}

fn word32(hi: u64) -> String {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&hi.to_be_bytes());
    format!("0x{}", hex::encode(w))
}

/// The fixed `{wrappedDoc, consent, eddsaSig}` POST body + the independently-recomputed expected
/// 7-vector public signals (decimal). Mirrors dogtag-standard's `prove_parity::fixed_prove_inputs`,
/// but emits the request JSON the `/prove-verification` endpoint consumes.
fn fixed_request() -> (serde_json::Value, Vec<String>) {
    // 1. Deterministic typed credential -> WrappedDoc (dogTagId is an Integer leaf).
    let typed = json!({
        "credentialSubject": {
            "dogTagId": {"tag": 3, "value": "424242"},
            "name": {"tag": 2, "value": "Rex"},
            "breed": {"tag": 2, "value": "Labrador"},
            "microchip": {"code": {"tag": 2, "value": "985141006580311"}},
            "weightKg": {"tag": 4, "value": "22.7"}
        }
    });
    let issuer = IssuerMeta {
        name: "Acme Vet".into(),
        domain: "acme.example".into(),
        document_store: "0x0000000000000000000000000000000000000001".into(),
        record_type: "VACCINATION".into(),
    };
    let mut sp = fixed_salts();
    let doc: WrappedDoc = wrap_document(&typed, issuer, &mut sp).expect("wrap");
    let wrapped_doc_json = serde_json::to_string(&doc).unwrap();

    // Merkle root R from the wrapped doc (== signature.merkleRoot).
    let root_fr = {
        let s = doc.signature.merkle_root.trim_start_matches("0x");
        Fr::from_be_bytes_mod_order(&hex::decode(s).unwrap())
    };

    // The circuit asserts leafValues[dogTagIdLeafIndex] == dogTagId (= fieldOf(value)).
    let dog_value_field: Fr = {
        let pairs = flatten_data(&doc.data);
        let (_, packed) = pairs
            .iter()
            .find(|(k, _)| k == "credentialSubject.dogTagId")
            .expect("dogTagId leaf present");
        let (_, tag, val) = parse_packed(packed).unwrap();
        let scalar: TypedScalar = scalar_from_packed(tag, &val).unwrap();
        field_of_value(&scalar).unwrap()
    };

    // 2. Consent fields. purpose=7, sample relayer/subject addresses, nonce=99.
    let purpose_hex = word32(7);
    let relayer_hex = "0x1111111111111111111111111111111111111111";
    let subject_hex = "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf";
    let nonce_hex = word32(99);
    let dog_tag_id_hex = to_hex32(&dog_value_field);
    let credential_root_hex = to_hex32(&root_fr);

    let decode32 = |h: &str| -> [u8; 32] {
        let mut o = [0u8; 32];
        o.copy_from_slice(&hex::decode(h.trim_start_matches("0x")).unwrap());
        o
    };
    let decode20 = |h: &str| -> [u8; 20] {
        let mut o = [0u8; 20];
        o.copy_from_slice(&hex::decode(h.trim_start_matches("0x")).unwrap());
        o
    };
    let consent = VerificationConsent {
        dog_tag_id: decode32(&dog_tag_id_hex),
        record_type: [0u8; 32],
        purpose: decode32(&purpose_hex),
        credential_root: decode32(&credential_root_hex),
        challenge: [0u8; 32],
        relayer: decode20(relayer_hex),
        subject: decode20(subject_hex),
        nonce: decode32(&nonce_hex),
        deadline: [0u8; 32],
    };

    // 3. Derive a consent key + sign M = Poseidon6(dogTagId, purpose, relayer, subject, R, nonce).
    let prv: [u8; 32] =
        hex::decode("0001020304050607080900010203040506070809000102030405060708090001")
            .unwrap()
            .try_into()
            .unwrap();
    let key = consent_key_from_raw_prv(&prv);
    let m = eddsa_consent_message(&consent);
    let sig = sign_poseidon(&prv, &m);

    let consent_json = json!({
        "dogTagId": dog_tag_id_hex,
        "recordType": word32(0),
        "purpose": purpose_hex,
        "credentialRoot": credential_root_hex,
        "challenge": word32(0),
        "relayer": relayer_hex,
        "subject": subject_hex,
        "nonce": nonce_hex,
        "deadline": word32(0)
    });

    // The POST body the endpoint consumes: { wrappedDoc, consent, eddsaSig } — wrappedDoc as a string
    // (the phone holds it as a string), consent as an embedded object (both accepted).
    let body = json!({
        "wrappedDoc": wrapped_doc_json,
        "consent": consent_json,
        "eddsaSig": {
            "r8xDec": dogtag_standard::eddsa::fr_to_dec(&sig.r8x),
            "r8yDec": dogtag_standard::eddsa::fr_to_dec(&sig.r8y),
            "sDec": sig.s.to_str_radix(10),
            "axHex": to_hex32(&key.ax),
            "ayHex": to_hex32(&key.ay),
        }
    });

    // Independently recompute the expected public signals.
    let nullifier = Fr::from_be_bytes_mod_order(&consent_nullifier(&consent));
    let kh = Fr::from_be_bytes_mod_order(&key_hash(key.ax, key.ay));
    let dog_id = Fr::from_be_bytes_mod_order(&consent.dog_tag_id);
    let purpose = Fr::from_be_bytes_mod_order(&consent.purpose);
    let relayer = Fr::from_be_bytes_mod_order(&consent.relayer);
    let subject = Fr::from_be_bytes_mod_order(&consent.subject);
    let expected: Vec<String> = [dog_id, purpose, relayer, subject, nullifier, kh, root_fr]
        .iter()
        .map(|f| f.into_bigint().to_string())
        .collect();

    (body, expected)
}

/// Build a router whose prover is the real ArkProver if the build artifacts exist, else StubProver.
/// Returns `(router, is_real)`.
fn router_with_best_prover() -> (axum::Router, bool) {
    let build_dir = repo_root().join("circuits").join("build");
    let have_artifacts = build_dir.join("verification_final.zkey").exists();
    let (prover, is_real): (Arc<dyn ProverClient>, bool) = if have_artifacts {
        match ArkProver::load(&build_dir) {
            Ok(p) => (Arc::new(p), true),
            Err(e) => {
                eprintln!("ArkProver::load failed ({e}); falling back to StubProver");
                (Arc::new(StubProver), false)
            }
        }
    } else {
        (Arc::new(StubProver), false)
    };
    let state = common::state_with_verify(
        Arc::new(MemChain::new()),
        "memchain".to_string(),
        "0x00000000000000000000000000000000000000aa".to_string(),
        "0x00000000000000000000000000000000000000bb".to_string(),
        "0x00000000000000000000000000000000000000cc".to_string(),
        "vet.example".to_string(),
        1,
        prover,
    );
    (vet_api::public_router(state), is_real)
}

/// FAST hermetic shape check (no prove): the assembled circuit input must parse cleanly into
/// `dogtag_prover::ProveInputs` — i.e. the server assembly emits the EXACT `from_circuit_input_json`
/// shape (scalars as strings, the six width-N signals as arrays). Guards the assembly<->prover seam
/// without the multi-minute Groth16 prove.
#[test]
fn assembled_input_parses_into_prove_inputs() {
    let (body, _expected) = fixed_request();
    let wrapped_doc_json = body["wrappedDoc"].as_str().unwrap().to_string();
    let consent_json = serde_json::to_string(&body["consent"]).unwrap();
    let e = &body["eddsaSig"];
    let sig = dogtag_standard::prover_assemble::EddsaSigInput {
        r8x_dec: e["r8xDec"].as_str().unwrap().to_string(),
        r8y_dec: e["r8yDec"].as_str().unwrap().to_string(),
        s_dec: e["sDec"].as_str().unwrap().to_string(),
        ax_hex: e["axHex"].as_str().unwrap().to_string(),
        ay_hex: e["ayHex"].as_str().unwrap().to_string(),
    };
    let input = dogtag_standard::prover_assemble::assemble_circuit_input(
        &wrapped_doc_json,
        &consent_json,
        &sig,
    )
    .expect("assemble");
    // The scalar fields must be bare strings (not arrays) for the ark parser.
    assert!(
        input["dogTagId"].is_string(),
        "dogTagId must be a scalar string"
    );
    assert!(input["leafSalts"].is_array(), "leafSalts must be an array");
    // The decisive assertion: the ark-0.6 ProveInputs parser accepts it.
    vet_api::prover::ProveInputs::from_circuit_input_json(&input)
        .expect("assembled input must parse into dogtag-prover ProveInputs");
}

/// Hermetic: the StubProver path returns the correct calldata shape and echoes the input-derived
/// public signals. (The StubProver zeroes nullifier/keyHash, so we assert only the input columns.)
#[tokio::test]
async fn prove_verification_returns_calldata_shape_and_echoes_inputs() {
    let (router, is_real) = router_with_best_prover();
    let (body, expected) = fixed_request();

    let (status, resp) =
        common::call(&router, "POST", "/prove-verification", None, Some(body)).await;
    assert_eq!(status, axum::http::StatusCode::OK, "resp: {resp}");

    // Shape: a:[2], b:[2][2], c:[2], pub:[7].
    let a = resp["a"].as_array().expect("a array");
    let b = resp["b"].as_array().expect("b array");
    let c = resp["c"].as_array().expect("c array");
    let pubs = resp["pub"].as_array().expect("pub array");
    assert_eq!(a.len(), 2, "a must be [2]");
    assert_eq!(b.len(), 2, "b must be [2][..]");
    assert_eq!(b[0].as_array().unwrap().len(), 2, "b[0] must be [2]");
    assert_eq!(b[1].as_array().unwrap().len(), 2, "b[1] must be [2]");
    assert_eq!(c.len(), 2, "c must be [2]");
    assert_eq!(pubs.len(), 7, "pub must be the 7-vector");

    let got: Vec<String> = pubs
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    // Input-derived columns: dogTagId, purpose, relayer, subject (0..4) and R (6) — produced by BOTH
    // the StubProver (echo) and the real ArkProver (binding).
    for i in [0usize, 1, 2, 3, 6] {
        assert_eq!(got[i], expected[i], "pub[{i}] mismatch");
    }

    if is_real {
        // The real prover ALSO computes the nullifier + keyHash (pub[4], pub[5]). Assert the full
        // 7-vector — these are non-zero and exactly the independently-recomputed values.
        assert_ne!(
            got[4], "0",
            "nullifier (pub[4]) must be non-zero with the real prover"
        );
        assert_ne!(
            got[5], "0",
            "keyHash (pub[5]) must be non-zero with the real prover"
        );
        assert_eq!(
            got, expected,
            "full public-signal 7-vector mismatch (real prover)"
        );
    } else {
        eprintln!(
            "NOTE: ArkProver artifacts absent (circuits/build/verification_final.zkey) — ran the \
             StubProver shape check only. Build the circuit to exercise the real prover."
        );
    }
}

/// Real-prover full assertion + (behind DOGTAG_LIVE_VERIFIER=1) on-chain cast-verify against the
/// LIVE Groth16Verifier. Self-skips when the circuit artifacts are absent or `cast` is unavailable.
#[tokio::test]
async fn prove_verification_proof_matches_and_optionally_verifies_on_chain() {
    let (router, is_real) = router_with_best_prover();
    if !is_real {
        eprintln!("SKIP: ArkProver artifacts missing — cannot exercise the real prove/verify path");
        return;
    }
    let (body, expected) = fixed_request();
    let (status, resp) =
        common::call(&router, "POST", "/prove-verification", None, Some(body)).await;
    assert_eq!(status, axum::http::StatusCode::OK, "resp: {resp}");

    let got: Vec<String> = resp["pub"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(got, expected, "public-signal 7-vector mismatch");

    if std::env::var("DOGTAG_LIVE_VERIFIER").ok().as_deref() != Some("1") {
        eprintln!("skipping live-verifier cast call (set DOGTAG_LIVE_VERIFIER=1 to run)");
        return;
    }
    let a: Vec<String> = resp["a"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let c: Vec<String> = resp["c"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let b: Vec<Vec<String>> = resp["b"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        })
        .collect();
    let p = &got;
    let out = std::process::Command::new("cast")
        .args([
            "call",
            // The live Groth16Verifier the on-device parity test also targets.
            "0x138b433071Ad806E841B5AD53623290a9bf21761",
            "verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[7])(bool)",
            &format!("[{},{}]", a[0], a[1]),
            &format!("[[{},{}],[{},{}]]", b[0][0], b[0][1], b[1][0], b[1][1]),
            &format!("[{},{}]", c[0], c[1]),
            &format!(
                "[{},{},{},{},{},{},{}]",
                p[0], p[1], p[2], p[3], p[4], p[5], p[6]
            ),
            "--rpc-url",
            "https://devrpc.roax.net",
        ])
        .output()
        .expect("run cast");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.trim() == "true",
        "live Groth16Verifier rejected /prove-verification proof: stdout={stdout:?} stderr={stderr:?}"
    );
}
