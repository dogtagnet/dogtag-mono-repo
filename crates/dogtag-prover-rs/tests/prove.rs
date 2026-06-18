//! Acceptance test for the DogTag Groth16 prover.
//!
//! 1. Builds the SAME circuit input object as `circuits/scripts/gen-zk-fixture.mjs`
//!    (numLeaves=13, dogTagId=424242, ...) by shelling out to `tests/gen_input.mjs`
//!    (it reuses the SDK's `buildMerkle` + poseidon + EdDSA so we don't re-derive
//!    poseidon in Rust).
//! 2. Calls `Prover::prove(inputs)`.
//! 3. Asserts the returned `pub[7]` equals the circuit's expected public signals.
//! 4. Independently VERIFIES the returned proof in-process with `ark_groth16::verify`,
//!    using the verifying key parsed from `circuits/build/verification_key.json`
//!    (NOT the zkey the prover used) and reconstructing the proof from the output's
//!    Solidity-calldata strings (undoing the b-coordinate swap).
//! 5. Cross-checks the produced `pub` against `contracts/test/zk-fixture.json` (proving
//!    our calldata formatting matches snarkjs).

use std::path::PathBuf;
use std::process::Command;

use ark_bn254::{Bn254, Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_crypto_primitives::snark::SNARK;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use dogtag_prover::{Groth16Output, ProveInputs, Prover, NUM_PUBLIC};
use num_bigint::BigUint;
use std::str::FromStr;

fn repo_root() -> PathBuf {
    // crate dir = <root>/crates/dogtag-prover-rs
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

/// Parse a snarkjs `verification_key.json` into an ark `VerifyingKey<Bn254>`.
///
/// G2 points in the JSON are `[[c0,c1],[c0,c1],[1,0]]`; ark `Fq2::new(c0, c1)`.
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

/// Reconstruct an ark `Proof<Bn254>` from the Solidity-calldata `Groth16Output`,
/// UNDOING the snarkjs->Solidity b-coordinate swap (`b[i] = [c1, c0]` -> `Fq2::new(c0, c1)`).
fn proof_from_output(o: &Groth16Output) -> Proof<Bn254> {
    let a = G1Affine::new(fq(&o.a[0]), fq(&o.a[1]));
    let c = G1Affine::new(fq(&o.c[0]), fq(&o.c[1]));
    // Output stored b[i] = [c1, c0]; ark wants Fq2::new(c0, c1).
    let bx = Fq2::new(fq(&o.b[0][1]), fq(&o.b[0][0]));
    let by = Fq2::new(fq(&o.b[1][1]), fq(&o.b[1][0]));
    let b = G2Affine::new(bx, by);
    Proof { a, b, c }
}

#[derive(serde::Deserialize)]
struct GenOutput {
    input: serde_json::Value,
    #[serde(rename = "pubDecimal")]
    pub_decimal: Vec<String>,
}

/// Run tests/gen_input.mjs to obtain the circuit input + expected pub (decimal).
fn gen_input(root: &PathBuf) -> GenOutput {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("gen_input.mjs");
    let out = Command::new("node")
        .arg(&script)
        .env("MONOREPO_ROOT", root)
        .current_dir(root.join("circuits"))
        .output()
        .expect("failed to spawn node (is node on PATH?)");
    assert!(
        out.status.success(),
        "gen_input.mjs failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("gen_input.mjs did not emit valid JSON")
}

#[test]
fn prove_verifies_and_pub_matches() {
    let root = repo_root();
    let build_dir = root.join("circuits").join("build");

    // 1. Build the same input object as gen-zk-fixture.mjs.
    let gen = gen_input(&root);
    let inputs = ProveInputs::from_circuit_input_json(&gen.input).expect("parse ProveInputs");

    // 2. Load + prove.
    let prover = Prover::load(&build_dir).expect("load prover artifacts");
    let output = prover.prove(inputs).expect("prove");

    // 3. pub[7] must equal the circuit's expected public signals (decimal).
    assert_eq!(output.public_signals.len(), NUM_PUBLIC);
    assert_eq!(
        output.public_signals.to_vec(),
        gen.pub_decimal,
        "public signals mismatch (decimal)"
    );

    // 4. Independently verify the proof against verification_key.json.
    let vk_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(build_dir.join("verification_key.json")).expect("read vk json"),
    )
    .expect("parse vk json");
    let vk = parse_vk(&vk_json);
    let pvk = Groth16::<Bn254>::process_vk(&vk).expect("process_vk");
    let proof = proof_from_output(&output);
    let public_inputs: Vec<Fr> = gen.pub_decimal.iter().map(|s| fr(s)).collect();
    let verified =
        Groth16::<Bn254>::verify_with_processed_vk(&pvk, &public_inputs, &proof).expect("verify");
    assert!(verified, "ark_groth16 verification of generated proof failed");

    // 5. Cross-check pub against the snarkjs-produced fixture (proves calldata formatting).
    let fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("contracts").join("test").join("zk-fixture.json"))
            .expect("read zk-fixture.json"),
    )
    .expect("parse zk-fixture.json");
    let fixture_pub_dec: Vec<String> = fixture["pub"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            let hs = h.as_str().unwrap().trim_start_matches("0x");
            BigUint::parse_bytes(hs.as_bytes(), 16).unwrap().to_string()
        })
        .collect();
    assert_eq!(
        output.public_signals.to_vec(),
        fixture_pub_dec,
        "pub differs from snarkjs fixture (contracts/test/zk-fixture.json)"
    );
}

#[test]
fn zkey_hash_is_stable_and_hex() {
    let build_dir = repo_root().join("circuits").join("build");
    let prover = Prover::load(&build_dir).expect("load");
    let h1 = prover.zkey_hash();
    let h2 = prover.zkey_hash();
    assert_eq!(h1, h2);
    assert_eq!(prover.zkey_hash_hex().len(), 64);
    assert_eq!(hex::encode(h1), prover.zkey_hash_hex());
}
