//! On-device CONSENT prover parity test (M7 P0) — runs ONLY with `--features prover`.
//!
//! The consent analogue of `prove_parity.rs`:
//! 1. Assembles the `DogTagConsent(6)` inputs with the REAL Rust assembler (`consent_assemble`).
//! 2. Calls `prover_ffi::prove_consent(...)` over the GRAPH witness backend (`consent.graph`) + the
//!    frozen `consent_final.zkey`.
//! 3. Independently VERIFIES the returned proof against the committed frozen VK
//!    `circuits/build/consent_verification_key.json` (sha256 `27879dd7…`, the same VK the on-chain
//!    `Groth16VerifierConsent` was generated from), reconstructing the ark proof from the
//!    Solidity-calldata strings (undoing the snarkjs→Solidity b-swap).
//! 4. Asserts the seven public signals equal the frozen-order vector
//!    `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`.
//!
//! This is the success criterion for the on-device FFI: `prove_consent`'s proof verifies under the
//! SAME VK the on-chain verifier uses, with matching public signals. It self-skips when the graph or
//! zkey is absent (the graph is not committed — CI fetches it from DOGTAG_ARTIFACTS_URL), so it never
//! reds an unbuilt checkout.
#![cfg(feature = "prover")]

use std::path::PathBuf;
use std::str::FromStr;

use ark_bn254::{Bn254, Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_crypto_primitives::snark::SNARK;
use ark_ff::PrimeField;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use num_bigint::BigUint;

use dogtag_standard::consent_assemble::{assemble_consent, ConsentWitness};
use dogtag_standard::prover_ffi::prove_consent;
use dogtag_standard::profile_tree::{AttributeLeaf, SALT_LEN};
use dogtag_standard::types::TypedScalar;

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

/// Reconstruct an ark Proof from the Solidity-calldata strings, UNDOING the b-swap.
fn proof_from_parts(a: &[String], b: &[Vec<String>], c: &[String]) -> Proof<Bn254> {
    let pa = G1Affine::new(fq(&a[0]), fq(&a[1]));
    let pc = G1Affine::new(fq(&c[0]), fq(&c[1]));
    let bx = Fq2::new(fq(&b[0][1]), fq(&b[0][0]));
    let by = Fq2::new(fq(&b[1][1]), fq(&b[1][0]));
    let pb = G2Affine::new(bx, by);
    Proof { a: pa, b: pb, c: pc }
}

fn word32(hi: u64) -> String {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&hi.to_be_bytes());
    format!("0x{}", hex::encode(w))
}

const SEED: &[u8] = b"consent parity wallet seed - TEST MATERIAL ONLY, never hold value";

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
            key_path: "credentialSubject.breedLabel".to_string(),
            salt: [9u8; SALT_LEN],
            value: TypedScalar::Str("Shiba Inu".to_string()),
        },
    ]
}

/// The success criterion: the on-device `prove_consent` proof verifies under the frozen consent VK.
#[test]
fn on_device_consent_proof_verifies_and_pub_matches() {
    let build_dir = repo_root().join("circuits").join("build");
    let zkey = build_dir.join("consent_final.zkey");
    let graph = build_dir.join("consent.graph");
    if !zkey.exists() || !graph.exists() {
        eprintln!(
            "SKIP: consent zkey/graph absent ({} / {}) — the graph is not committed (CI fetches it)",
            zkey.display(),
            graph.display()
        );
        return;
    }

    let a = attrs();
    // Expected public signals from the REAL assembler (frozen OUTPUT order).
    let inp = assemble_consent(&ConsentWitness {
        seed: SEED,
        dog_tag_id_handle: "424242",
        owner_address: owner_address(),
        attributes: &a,
        purpose: Fr::from(7u64),
        relayer: [0x11u8; 20],
        record_type: Fr::from(19u64),
        deadline: Fr::from(1_893_456_000u64),
        consent_nonce: Fr::from(99u64),
    })
    .expect("assemble consent");
    let expected: Vec<String> = vec![
        inp.dog_tag_id.clone(),
        inp.purpose.clone(),
        inp.relayer.clone(),
        inp.nullifier.into_bigint().to_string(),
        inp.root.into_bigint().to_string(),
        inp.record_type.clone(),
        inp.deadline.clone(),
    ];

    // Build the SAME witness through the FFI's string params.
    let attributes_json = serde_json::to_string(&serde_json::json!([
        {"keyPath": "credentialSubject.name", "salt": format!("0x{}", hex::encode([7u8; SALT_LEN])), "tag": 2, "value": "Rex"},
        {"keyPath": "credentialSubject.breedLabel", "salt": format!("0x{}", hex::encode([9u8; SALT_LEN])), "tag": 2, "value": "Shiba Inu"},
    ]))
    .unwrap();

    let proof = prove_consent(
        format!("0x{}", hex::encode(SEED)),
        "424242".to_string(),
        format!("0x{}", hex::encode(owner_address())),
        attributes_json,
        word32(7),
        "0x1111111111111111111111111111111111111111".to_string(),
        word32(19),
        word32(99),
        "1893456000".to_string(),
        zkey.to_string_lossy().into_owned(),
        graph.to_string_lossy().into_owned(),
    )
    .expect("prove_consent");

    assert_eq!(proof.pub_signals, expected, "consent public-signal 7-vector mismatch");

    // Independently verify against the frozen VK json.
    let vk_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(build_dir.join("consent_verification_key.json")).expect("read vk"),
    )
    .expect("parse vk json");
    let vk = parse_vk(&vk_json);
    let ark_proof = proof_from_parts(&proof.a, &proof.b, &proof.c);
    let pvk = Groth16::<Bn254>::process_vk(&vk).unwrap();
    let public_inputs: Vec<Fr> = proof.pub_signals.iter().map(|s| fr(s)).collect();
    let ok = Groth16::<Bn254>::verify_with_processed_vk(&pvk, &public_inputs, &ark_proof).unwrap();
    assert!(ok, "on-device consent proof must verify under the frozen consent VK");
}
