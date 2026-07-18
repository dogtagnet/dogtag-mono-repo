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
use dogtag_prover::{artifact, Groth16Output, ProveInputs, Prover, NUM_PUBLIC};
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

/// Audit M4: `Prover::load` enforces the pinned zkey hash — the bundled artifact matches the pinned
/// constant, so a swapped key would be the only way the hashes diverge.
#[test]
fn load_enforces_pinned_zkey_hash() {
    let build_dir = repo_root().join("circuits").join("build");
    let prover = Prover::load(&build_dir).expect("load must accept the pinned zkey");
    assert_eq!(
        prover.zkey_hash_hex(),
        dogtag_prover::EXPECTED_ZKEY_SHA256_HEX,
        "loaded zkey hash must equal the pinned constant"
    );
}

/// Audit M4: a zkey whose hash differs from the expected one is rejected (fail-closed), proving a
/// swapped/corrupt proving key cannot be loaded.
#[test]
fn load_rejects_unexpected_zkey_hash() {
    let build_dir = repo_root().join("circuits").join("build");
    let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
    match Prover::load_with_expected_zkey(&build_dir, wrong) {
        Ok(_) => panic!("load must reject a zkey whose hash differs from the pinned value"),
        Err(dogtag_prover::ProverError::ZkeyHashMismatch { expected, got }) => {
            assert_eq!(expected, wrong);
            assert_eq!(got, dogtag_prover::EXPECTED_ZKEY_SHA256_HEX);
        }
        Err(other) => panic!("expected ZkeyHashMismatch, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Version-keyed artifacts (M7 §3.2/§3.5)
// ---------------------------------------------------------------------------------------------

fn sha256_hex(path: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())));
    hex::encode::<[u8; 32]>(hasher.finalize().into())
}

/// Every hash the Level-A descriptor pins is the hash of the artifact actually in the tree — so the
/// pins are checked facts, not copied-in strings that could silently rot.
#[test]
fn level_a_descriptor_pins_match_the_real_artifacts() {
    let build_dir = repo_root().join("circuits").join("build");
    let d = artifact::current();

    assert_eq!(
        sha256_hex(&build_dir.join(d.zkey.rel_path)),
        d.zkey.sha256,
        "zkey pin does not match {}",
        d.zkey.rel_path
    );
    for f in [&d.r1cs, &d.wasm, &d.vk.verification_key_json] {
        let Some(expected) = f.sha256 else { continue };
        assert_eq!(
            sha256_hex(&build_dir.join(f.rel_path)),
            expected,
            "pin does not match {}",
            f.rel_path
        );
    }
}

/// PARITY, the acceptance bar: naming no version and naming the current one load the SAME artifacts.
/// `Prover::load` is the pre-M7 entry point, so this is what "back-compat" means concretely.
#[test]
fn load_and_load_versioned_current_are_equivalent() {
    let build_dir = repo_root().join("circuits").join("build");
    let default = Prover::load(&build_dir).expect("load (no version named)");
    let versioned = Prover::load_versioned(&build_dir, artifact::resolve(None).expect("resolve"))
        .expect("load_versioned(current)");

    assert_eq!(default.zkey_hash(), versioned.zkey_hash());
    assert_eq!(default.version(), versioned.version());
    assert_eq!(default.version(), artifact::LEVEL_A_V1);
    assert_eq!(default.descriptor(), versioned.descriptor());
}

/// The zkey's pin is enforced (`load_rejects_unexpected_zkey_hash`); this proves the SAME
/// fail-closed discipline now covers the witness artifacts. A tampered r1cs is rejected before it is
/// ever parsed, so a proof can never be built from a constraint system the version did not pin.
#[test]
fn load_rejects_tampered_witness_artifact() {
    let build_dir = repo_root().join("circuits").join("build");
    let d = artifact::current();

    // A build dir whose zkey/wasm are the real ones (symlinked — they are ~65 MB / ~2 MB) but whose
    // r1cs has been swapped for junk.
    let tmp = std::env::temp_dir().join(format!("dogtag-prover-tamper-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("verification_js")).expect("mkdir tmp build dir");
    for rel in [d.zkey.rel_path, d.wasm.rel_path] {
        std::os::unix::fs::symlink(build_dir.join(rel), tmp.join(rel)).expect("symlink artifact");
    }
    std::fs::write(tmp.join(d.r1cs.rel_path), b"not an r1cs").expect("write tampered r1cs");

    let result = Prover::load_versioned(&tmp, d);
    std::fs::remove_dir_all(&tmp).ok();

    match result {
        Ok(_) => panic!("load must reject an r1cs whose hash differs from the version's pin"),
        Err(dogtag_prover::ProverError::ArtifactHashMismatch {
            artifact, expected, ..
        }) => {
            assert_eq!(artifact, d.r1cs.rel_path);
            assert_eq!(Some(expected.as_str()), d.r1cs.sha256);
        }
        Err(other) => panic!("expected ArtifactHashMismatch, got {other:?}"),
    }
}

/// The load-time width guards reject a version this build cannot feed or format, BEFORE any artifact
/// is read.
///
/// Today's registry has one entry whose widths match, so `resolve` can never hand `load_versioned` a
/// mismatched descriptor — which is precisely why these guards need a direct test: nothing else
/// executes them, so a refactor could drop one and every other test would still pass. They exist for
/// the multi-version state M7 builds toward, where a descriptor's width is no longer guaranteed to be
/// this build's.
#[test]
fn load_rejects_a_version_whose_width_this_build_cannot_handle() {
    // Paths/pins are irrelevant: the width guards run before any file I/O, so a descriptor pointing
    // at nothing still reaches them. If a guard were removed the load would fail differently (a
    // missing-file `Load`), which the message assertions below catch.
    const BAD_LEAVES: artifact::ArtifactDescriptor = artifact::ArtifactDescriptor {
        max_leaves: Some(dogtag_prover::N + 1),
        ..artifact::LEVEL_A_V1_DESCRIPTOR
    };
    const BAD_PUBLIC: artifact::ArtifactDescriptor = artifact::ArtifactDescriptor {
        num_public: NUM_PUBLIC + 1,
        ..artifact::LEVEL_A_V1_DESCRIPTOR
    };

    match Prover::load_versioned("/nonexistent-build-dir", &BAD_LEAVES).map(|_| ()) {
        Err(dogtag_prover::ProverError::Load(msg)) => assert!(
            msg.contains("leaves") && msg.contains(&(dogtag_prover::N + 1).to_string()),
            "the error must name the width mismatch, got: {msg}"
        ),
        other => panic!("a wider-than-N version must be refused at load, got {other:?}"),
    }

    match Prover::load_versioned("/nonexistent-build-dir", &BAD_PUBLIC).map(|_| ()) {
        Err(dogtag_prover::ProverError::Load(msg)) => assert!(
            msg.contains("public signals"),
            "the error must name the public-signal mismatch, got: {msg}"
        ),
        other => panic!("a version with a different public count must be refused, got {other:?}"),
    }
}
