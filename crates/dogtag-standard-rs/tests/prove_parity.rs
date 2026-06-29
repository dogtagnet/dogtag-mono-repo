//! On-device prover parity test (Workstream A / E) — runs ONLY with `--features prover`.
//!
//! 1. Builds a deterministic WrappedDoc (fixed salts) including a `credentialSubject.dogTagId` leaf.
//! 2. Derives a consent whose fields satisfy the circuit's bindings (credentialRoot == the doc's
//!    merkle root; dogTagId == fieldOf(the dogTagId leaf value), which is what the circuit asserts),
//!    and signs the EdDSA consent message M with the crate's own BabyJubjub signer.
//! 3. Calls `prover_ffi::prove_verification(...)` with `circuits/build/verification_final.zkey`.
//! 4. Independently VERIFIES the returned proof against `circuits/build/verification_key.json`
//!    (NOT the zkey), reconstructing the ark proof from the Solidity-calldata strings (undoing the
//!    snarkjs->Solidity b-swap) — same harness pattern as dogtag-prover-rs/tests/prove.rs.
//! 5. Asserts the 7 public signals equal the independently-recomputed
//!    [dogTagId, purpose, relayer, subject, nullifier, keyHash, R].
//!
//! This is the success criterion: the on-device proof verifies under the SAME vkey the on-chain
//! Groth16Verifier was generated from, with matching public signals.
#![cfg(feature = "prover")]

use std::path::PathBuf;
use std::str::FromStr;

use ark_bn254::{Bn254, Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_crypto_primitives::snark::SNARK;
use ark_ff::PrimeField;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use num_bigint::BigUint;

use dogtag_standard::consent::{
    consent_nullifier, eddsa_consent_message, key_hash, VerificationConsent,
};
use dogtag_standard::eddsa::{consent_key_from_raw_prv, sign_poseidon};
use dogtag_standard::field::to_hex32;
use dogtag_standard::leaf::field_of_value;
use dogtag_standard::poseidon::to_be_bytes32;
use dogtag_standard::prover_ffi::{prove_verification, EddsaSigInput};
use dogtag_standard::types::TypedScalar;
use dogtag_standard::wrap::{
    flatten_data, parse_packed, scalar_from_packed, wrap_document, IssuerMeta, WrappedDoc,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fq(s: &str) -> Fq {
    Fq::from(BigUint::from_str(s).expect("decimal Fq"))
}
fn fr(s: &str) -> Fr {
    Fr::from(BigUint::from_str(s).expect("decimal Fr"))
}

fn parse_vk(v: &serde_json::Value) -> VerifyingKey<Bn254> {
    let g1 = |key: &str| -> G1Affine {
        let a = &v[key];
        G1Affine::new(fq(a[0].as_str().unwrap()), fq(a[1].as_str().unwrap()))
    };
    let g2 = |key: &str| -> G2Affine {
        let a = &v[key];
        let x = Fq2::new(fq(a[0][0].as_str().unwrap()), fq(a[0][1].as_str().unwrap()));
        let y = Fq2::new(fq(a[1][0].as_str().unwrap()), fq(a[1][1].as_str().unwrap()));
        G2Affine::new(x, y)
    };
    let ic = v["IC"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| G1Affine::new(fq(p[0].as_str().unwrap()), fq(p[1].as_str().unwrap())))
        .collect::<Vec<_>>();
    VerifyingKey {
        alpha_g1: g1("vk_alpha_1"),
        beta_g2: g2("vk_beta_2"),
        gamma_g2: g2("vk_gamma_2"),
        delta_g2: g2("vk_delta_2"),
        gamma_abc_g1: ic,
    }
}

/// Reconstruct an ark Proof from the Solidity-calldata strings, UNDOING the b-swap
/// (output stored b[i] = [c1, c0]; ark wants Fq2::new(c0, c1)).
fn proof_from_parts(a: &[String], b: &[Vec<String>], c: &[String]) -> Proof<Bn254> {
    let pa = G1Affine::new(fq(&a[0]), fq(&a[1]));
    let pc = G1Affine::new(fq(&c[0]), fq(&c[1]));
    let bx = Fq2::new(fq(&b[0][1]), fq(&b[0][0]));
    let by = Fq2::new(fq(&b[1][1]), fq(&b[1][0]));
    let pb = G2Affine::new(bx, by);
    Proof {
        a: pa,
        b: pb,
        c: pc,
    }
}

/// Deterministic 16-byte salts: each call returns [n;16], n increments from 1.
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

/// Build the SAME fixed inputs the parity test uses, returning the
/// `(wrapped_doc_json, consent_json, eddsa_sig, zkey_path, graph_path)` tuple plus the
/// independently-recomputed expected public signals. Shared by the verify test and the
/// live-verifier dump test so both prove over identical inputs.
fn fixed_prove_inputs() -> (String, String, EddsaSigInput, PathBuf, PathBuf, Vec<String>) {
    let root_dir = repo_root();
    let build_dir = root_dir.join("circuits").join("build");
    let zkey = build_dir.join("verification_final.zkey");
    assert!(zkey.exists(), "missing zkey: {}", zkey.display());
    let graph = build_dir.join("verification.graph");
    assert!(graph.exists(), "missing witness graph: {}", graph.display());

    // 1. Deterministic typed credential -> WrappedDoc. dogTagId is an Integer leaf.
    let typed = serde_json::json!({
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

    // The merkle root R from the wrapped doc (== signature.merkleRoot).
    let root_fr = {
        let s = doc.signature.merkle_root.trim_start_matches("0x");
        Fr::from_be_bytes_mod_order(&hex::decode(s).unwrap())
    };

    // The circuit asserts leafValues[dogTagIdLeafIndex] == dogTagId, where leafValues = fieldOf(value).
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

    // 2. Consent fields. purpose=7 (label reduced), relayer/subject sample addresses, nonce=99.
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

    let eddsa_sig = EddsaSigInput {
        r8x_dec: dogtag_standard::eddsa::fr_to_dec(&sig.r8x),
        r8y_dec: dogtag_standard::eddsa::fr_to_dec(&sig.r8y),
        s_dec: sig.s.to_str_radix(10),
        ax_hex: to_hex32(&key.ax),
        ay_hex: to_hex32(&key.ay),
    };

    let consent_json = serde_json::json!({
        "dogTagId": dog_tag_id_hex,
        "recordType": word32(0),
        "purpose": purpose_hex,
        "credentialRoot": credential_root_hex,
        "challenge": word32(0),
        "relayer": relayer_hex,
        "subject": subject_hex,
        "nonce": nonce_hex,
        "deadline": word32(0)
    })
    .to_string();

    // Independently recompute expected public signals.
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

    (
        wrapped_doc_json,
        consent_json,
        eddsa_sig,
        zkey,
        graph,
        expected,
    )
}

/// LIVE-VERIFIER BISECT DUMP: generate a real `prove_verification` proof over the fixed inputs and
/// print `a`/`b`/`c`/`pub` as decimal arrays + a ready-to-run `cast call` against the deployed
/// Groth16Verifier. Run with:
///   cargo test -p dogtag-standard-rs --features prover dump_proof_for_live_verifier -- --nocapture
#[test]
fn dump_proof_for_live_verifier() {
    let (wrapped_doc_json, consent_json, eddsa_sig, zkey, graph, expected) = fixed_prove_inputs();

    let proof = prove_verification(
        wrapped_doc_json,
        consent_json,
        eddsa_sig,
        zkey.to_string_lossy().into_owned(),
        graph.to_string_lossy().into_owned(),
    )
    .expect("prove_verification");

    assert_eq!(proof.pub_signals.len(), 7, "expected 7 public signals");
    assert_eq!(
        proof.pub_signals, expected,
        "public signals mismatch (snarkjs order)"
    );

    let a = &proof.a;
    let b = &proof.b;
    let c = &proof.c;
    let p = &proof.pub_signals;

    println!("\n===PROOF_DUMP_BEGIN===");
    println!("a = [{}, {}]", a[0], a[1]);
    println!(
        "b = [[{}, {}], [{}, {}]]",
        b[0][0], b[0][1], b[1][0], b[1][1]
    );
    println!("c = [{}, {}]", c[0], c[1]);
    println!("pub = [{}]", p.join(", "));
    // Ready-to-run cast call (current prover_ffi formatting).
    println!(
        "\nCAST_CMD: cast call 0x138b433071Ad806E841B5AD53623290a9bf21761 \
         'verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[7])(bool)' \
         '[{},{}]' '[[{},{}],[{},{}]]' '[{},{}]' '[{},{},{},{},{},{},{}]' \
         --rpc-url https://devrpc.roax.net",
        a[0],
        a[1],
        b[0][0],
        b[0][1],
        b[1][0],
        b[1][1],
        c[0],
        c[1],
        p[0],
        p[1],
        p[2],
        p[3],
        p[4],
        p[5],
        p[6],
    );
    println!("===PROOF_DUMP_END===\n");
}

/// REGRESSION GUARD: the on-device prover_ffi proof, formatted as Solidity calldata, must verify on
/// the LIVE deployed Groth16Verifier. Gated behind the `live_verifier` feature-ish env so CI without
/// network access skips it; run explicitly with DOGTAG_LIVE_VERIFIER=1.
#[test]
fn on_device_proof_verifies_on_live_chain() {
    if std::env::var("DOGTAG_LIVE_VERIFIER").ok().as_deref() != Some("1") {
        eprintln!("skipping live-verifier test (set DOGTAG_LIVE_VERIFIER=1 to run)");
        return;
    }
    let (wrapped_doc_json, consent_json, eddsa_sig, zkey, graph, _expected) = fixed_prove_inputs();
    let proof = prove_verification(
        wrapped_doc_json,
        consent_json,
        eddsa_sig,
        zkey.to_string_lossy().into_owned(),
        graph.to_string_lossy().into_owned(),
    )
    .expect("prove_verification");

    let a = &proof.a;
    let b = &proof.b;
    let c = &proof.c;
    let p = &proof.pub_signals;
    let out = std::process::Command::new("cast")
        .args([
            "call",
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
        "live Groth16Verifier rejected prover_ffi proof: stdout={stdout:?} stderr={stderr:?}"
    );
}

#[test]
fn on_device_proof_verifies_and_pub_matches() {
    let root_dir = repo_root();
    let build_dir = root_dir.join("circuits").join("build");
    let zkey = build_dir.join("verification_final.zkey");
    assert!(zkey.exists(), "missing zkey: {}", zkey.display());
    let graph = build_dir.join("verification.graph");
    assert!(graph.exists(), "missing witness graph: {}", graph.display());

    // 1. Deterministic typed credential -> WrappedDoc. dogTagId is an Integer leaf.
    let typed = serde_json::json!({
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

    // The merkle root R from the wrapped doc (== signature.merkleRoot).
    let root_fr = {
        let s = doc.signature.merkle_root.trim_start_matches("0x");
        Fr::from_be_bytes_mod_order(&hex::decode(s).unwrap())
    };

    // The circuit asserts leafValues[dogTagIdLeafIndex] == dogTagId, where leafValues = fieldOf(value).
    // So the consent's dogTagId field element MUST equal fieldOf(the dogTagId leaf's scalar).
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

    // 2. Consent fields. purpose=7 (label reduced), relayer/subject sample addresses, nonce=99.
    let purpose_hex = word32(7);
    let relayer_hex = "0x1111111111111111111111111111111111111111";
    let subject_hex = "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf";
    let nonce_hex = word32(99);
    let dog_tag_id_hex = to_hex32(&dog_value_field);
    let credential_root_hex = to_hex32(&root_fr);

    // Build the VerificationConsent (matches the circuit/nullifier inputs).
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

    let eddsa_sig = EddsaSigInput {
        r8x_dec: dogtag_standard::eddsa::fr_to_dec(&sig.r8x),
        r8y_dec: dogtag_standard::eddsa::fr_to_dec(&sig.r8y),
        s_dec: sig.s.to_str_radix(10),
        ax_hex: to_hex32(&key.ax),
        ay_hex: to_hex32(&key.ay),
    };

    // consent JSON in the POSTed hex shape.
    let consent_json = serde_json::json!({
        "dogTagId": dog_tag_id_hex,
        "recordType": word32(0),
        "purpose": purpose_hex,
        "credentialRoot": credential_root_hex,
        "challenge": word32(0),
        "relayer": relayer_hex,
        "subject": subject_hex,
        "nonce": nonce_hex,
        "deadline": word32(0)
    })
    .to_string();

    // 4. Prove on device (graph-based witness calculator over the bundled verification.graph).
    let proof = prove_verification(
        wrapped_doc_json,
        consent_json,
        eddsa_sig,
        zkey.to_string_lossy().into_owned(),
        graph.to_string_lossy().into_owned(),
    )
    .expect("prove_verification");

    assert_eq!(proof.pub_signals.len(), 7, "expected 7 public signals");

    // The 32-bit-ARM regression this fix targets: wasm2c zeroed the last-computed output wires.
    // pub[4]=nullifier and pub[5]=keyHash are derived from the REAL (large Ax/Ay) consent key + a
    // realistic nonce above, so they must be NON-ZERO with the graph calculator.
    assert_ne!(
        proof.pub_signals[4], "0",
        "nullifier (pub[4]) must be non-zero"
    );
    assert_ne!(
        proof.pub_signals[5], "0",
        "keyHash (pub[5]) must be non-zero"
    );

    // 5. Recompute expected public signals independently.
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
    assert_eq!(
        proof.pub_signals, expected,
        "public signals mismatch (snarkjs order)"
    );

    // 6. Independently verify against verification_key.json.
    let vk_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(build_dir.join("verification_key.json")).unwrap(),
    )
    .unwrap();
    let vk = parse_vk(&vk_json);
    let pvk = Groth16::<Bn254>::process_vk(&vk).unwrap();
    let ark_proof = proof_from_parts(&proof.a, &proof.b, &proof.c);
    let public_inputs: Vec<Fr> = proof.pub_signals.iter().map(|s| fr(s)).collect();
    let verified =
        Groth16::<Bn254>::verify_with_processed_vk(&pvk, &public_inputs, &ark_proof).unwrap();
    assert!(
        verified,
        "on-device proof failed verification under verification_key.json"
    );

    // sanity: hashing round-trips
    let _ = to_be_bytes32(&root_fr);
}
