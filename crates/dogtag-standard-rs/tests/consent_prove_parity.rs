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
//! zkey is absent, so it never reds an unbuilt checkout - `consent.graph` is gitignored, is not
//! committed, and nothing fetches it automatically.
//!
//! Because that skip is invisible from inside libtest (stdout is captured for PASSING tests), the
//! LOUD entry point is the shell wrapper `scripts/test-consent-parity.sh` (`make test-consent-parity`):
//! it checks the artifacts before cargo runs, emits a `::error::` annotation naming the missing one,
//! and exits non-zero. Prefer it over invoking this test directly.
#![cfg(feature = "prover")]

use std::path::PathBuf;
use std::str::FromStr;

use ark_bn254::{Bn254, Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_crypto_primitives::snark::SNARK;
use ark_ff::PrimeField;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use num_bigint::BigUint;

use dogtag_standard::consent_assemble::{assemble_consent, ConsentWitness};
use dogtag_standard::profile_tree::{AttributeLeaf, SALT_LEN};
use dogtag_standard::prover_ffi::prove_consent;
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
    Proof {
        a: pa,
        b: pb,
        c: pc,
    }
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

/// Pet attributes PLUS the D1 `owner.identity.*` identity leaves - the exact v1 issuance tree
/// shape. The circuit is leaf-blind (f3 §2.2), so the frozen VK must keep verifying with identity
/// leaves present; running the one slow Groth16 pass over this combined tree is the empirical
/// proof that D1 moved nothing.
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
        AttributeLeaf {
            key_path: "owner.identity.docNumber".to_string(),
            salt: [23u8; SALT_LEN],
            value: TypedScalar::Str("PASSPORT-123".to_string()),
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
        // This is the repo's ONLY empirical proof that the prover agrees with the frozen consent VK.
        // Skipping it silently made that proof optional: the run reported green whether or not the
        // one thing it exists to check had actually been checked.
        //
        // Two mechanisms make the skip visible, and only one of them can live here. Annotating from
        // inside a PASSING libtest cannot work - libtest captures stdout unless the runner passes
        // `--nocapture` - so the `::error::` annotation belongs to the shell wrapper
        // (`scripts/test-consent-parity.sh`), which checks the artifacts before cargo even starts.
        // What DOES work here is the hard-fail leg: `DOGTAG_REQUIRE_ZK_ARTIFACTS=1` turns the skip
        // into a panic, and libtest prints captured output for FAILING tests. Any environment that is
        // supposed to have the artifacts sets it - a missing artifact there means the fetch regressed.
        //
        // Local checkouts keep the plain skip (the graph is gitignored and not committed).
        // Name the artifact that is ACTUALLY missing. The old wording listed both paths whichever one
        // was absent, which reads as "neither was fetched" when usually only the graph is (the zkey is
        // committed; the graph is the one that is never committed).
        let missing: Vec<String> = [(&zkey, "consent_final.zkey"), (&graph, "consent.graph")]
            .iter()
            .filter(|(p, _)| !p.exists())
            .map(|(p, name)| format!("{name} ({})", p.display()))
            .collect();
        let msg = format!(
            "consent proving artifact(s) absent: {} - consent.graph is gitignored, is NOT committed, \
             and nothing fetches it automatically; build it locally from circuits/consent.circom with \
             iden3's build-circuit tool",
            missing.join(", ")
        );
        if std::env::var("DOGTAG_REQUIRE_ZK_ARTIFACTS").is_ok_and(|v| v == "1") {
            panic!(
                "DOGTAG_REQUIRE_ZK_ARTIFACTS=1 but {msg}. This environment is configured to REQUIRE \
                 the prove<->VK parity check; refusing to report green without running it."
            );
        }
        eprintln!("SKIP: {msg} - prove<->VK parity was NOT verified in this run");
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
        {"keyPath": "owner.identity.fullName", "salt": format!("0x{}", hex::encode([21u8; SALT_LEN])), "tag": 2, "value": "Alice Owner"},
        {"keyPath": "owner.identity.country", "salt": format!("0x{}", hex::encode([22u8; SALT_LEN])), "tag": 2, "value": "GB"},
        {"keyPath": "owner.identity.docNumber", "salt": format!("0x{}", hex::encode([23u8; SALT_LEN])), "tag": 2, "value": "PASSPORT-123"},
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

    assert_eq!(
        proof.pub_signals, expected,
        "consent public-signal 7-vector mismatch"
    );

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
    assert!(
        ok,
        "on-device consent proof must verify under the frozen consent VK"
    );
}
