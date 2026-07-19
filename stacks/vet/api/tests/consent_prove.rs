//! Ground-truth Level-B CONSENT prove→verify (M7 P0) — runs ONLY with `--features prover`.
//!
//! This is the single test that validates the whole consent proving path at once: it assembles the
//! `DogTagConsent(6)` inputs with the REAL Rust assembler (`consent_assemble` — canonical `dogTagId`
//! field + `build_profile_tree` + EdDSA-signed `M` + front-packed inclusion paths), then proves
//! against the COMMITTED frozen consent artifacts (`consent_final.zkey` / `consent.r1cs` /
//! `consent.wasm`). `Prover::prove_consent_inputs` self-verifies the proof in-process against the
//! zkey's embedded verifying key — which is the frozen M3 ceremony VK (sha256 `f83a111f…`, the same
//! VK snarkjs exported to `consent_verification_key.json`, sha256 `27879dd7…`) — so a returned proof
//! IS a proof that verifies against the frozen consent VK. It then asserts the seven public signals
//! in the FROZEN OUTPUT order `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`,
//! that `pub[0]` is the canonical `dogTagId` field, and that `pub[4]` (`R`) is the tree root the SBT
//! seals as `profileRoot(dogTagId)`.
//!
//! Hermetic: the consent artifacts are committed under `circuits/build`. Self-skips if absent (same
//! artifact-presence gate the Level-A `prove_verification` test uses), so it never reds an unbuilt
//! checkout. A piece-wise unit test can pass while a subtle encoding bug hides; a real proof against
//! the frozen VK cannot — this is the ground truth for input names, `M`/nullifier, path-packing, and
//! the canonical field, together.
#![cfg(feature = "prover")]

use std::path::PathBuf;

use ark_bn254::Fr;
use ark_ff::PrimeField;

use dogtag_prover::{artifact, ConsentProveInputs, Prover};
use dogtag_standard::consent_assemble::{
    assemble_consent, consent_circuit_input_value, ConsentWitness,
};
use dogtag_standard::profile_tree::{AttributeLeaf, SALT_LEN};
use dogtag_standard::types::TypedScalar;
// LEVEL-B indices: these exercise the consent circuit.
use dogtag_standard::public_signals::level_b as P;

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

fn fe_to_dec(f: &Fr) -> String {
    f.into_bigint().to_string()
}

const SEED: &[u8] = b"consent prove ground-truth wallet seed - TEST MATERIAL ONLY, never hold value";

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
        AttributeLeaf {
            key_path: "credentialSubject.dateOfBirth".to_string(),
            salt: [11u8; SALT_LEN],
            value: TypedScalar::Str("2021-04-02".to_string()),
        },
    ]
}

fn consent_witness(attributes: &[AttributeLeaf]) -> ConsentWitness<'_> {
    ConsentWitness {
        seed: SEED,
        dog_tag_id_handle: "424242",
        owner_address: owner_address(),
        attributes,
        purpose: Fr::from(7u64),
        relayer: [0x11u8; 20],
        record_type: Fr::from(19u64),
        deadline: Fr::from(1_893_456_000u64), // 2030-01-01
        consent_nonce: Fr::from(99u64),
    }
}

/// The success criterion: a real consent proof verifies under the frozen VK, with the seven public
/// signals in the frozen OUTPUT order and the canonical `dogTagId`/`R` bindings.
#[test]
fn consent_proves_and_verifies_against_the_frozen_vk() {
    let build_dir = repo_root().join("circuits").join("build");
    if !build_dir.join("consent_final.zkey").exists() {
        eprintln!(
            "SKIP: consent artifacts absent (circuits/build/consent_final.zkey) — cannot exercise \
             the real consent prove/verify path"
        );
        return;
    }

    // 1. Assemble the consent inputs with the REAL Rust assembler (canonical dogTagId field).
    let a = attrs();
    let inp = assemble_consent(&consent_witness(&a)).expect("assemble consent inputs");
    let circuit_input = consent_circuit_input_value(&inp);

    // 2. Load the version-keyed CONSENT prover (pins + parses the frozen consent artifacts).
    let prover = Prover::load_versioned(&build_dir, &artifact::LEVEL_B_V1_DESCRIPTOR)
        .expect("load consent prover");
    assert_eq!(prover.version(), artifact::LEVEL_B_V1);
    assert_eq!(prover.zkey_hash_hex(), artifact::LEVEL_B_V1_DESCRIPTOR.zkey.sha256);

    // 3. Prove — `prove_consent_inputs` self-verifies against the zkey's embedded (frozen) VK.
    let inputs = ConsentProveInputs::from_circuit_input_json(&circuit_input)
        .expect("parse consent circuit input");
    let out = prover
        .prove_consent_inputs(inputs)
        .expect("consent prove (self-verifies against the frozen VK)");

    // 4. The seven public signals, in the FROZEN OUTPUT order.
    let expected = vec![
        inp.dog_tag_id.clone(),   // pub[0] dogTagId  (canonical field)
        inp.purpose.clone(),      // pub[1] purpose
        inp.relayer.clone(),      // pub[2] relayer
        fe_to_dec(&inp.nullifier), // pub[3] nullifier (circuit output)
        fe_to_dec(&inp.root),      // pub[4] R          (circuit output; == profileRoot)
        inp.record_type.clone(),  // pub[5] recordType
        inp.deadline.clone(),     // pub[6] deadline
    ];
    assert_eq!(
        out.public_signals.to_vec(),
        expected,
        "consent pub[7] must match the frozen-order signals [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]"
    );

    // 5. The canonical-field discipline, visible on-chain: pub[0] is the canonical dogTagId field
    //    (the id the SBT must be minted with), and pub[4]/R is profileRoot(dogTagId).
    assert_eq!(out.public_signals[P::DOG_TAG_ID], fe_to_dec(&inp.dog_tag_id_field));
    assert_eq!(out.public_signals[P::ROOT], fe_to_dec(&inp.root));
    // The nullifier is a non-trivial circuit output (not echoed / zeroed).
    assert_ne!(
        out.public_signals[P::NULLIFIER], "0",
        "nullifier must be a real circuit output"
    );
}

/// Fail-closed at load: the consent prover refuses a zkey whose hash differs from the pinned one —
/// the same audit-M4 discipline the Level-A prover has, now for the consent artifact set.
#[test]
fn consent_prover_rejects_an_unexpected_zkey_hash() {
    let build_dir = repo_root().join("circuits").join("build");
    if !build_dir.join("consent_final.zkey").exists() {
        eprintln!("SKIP: consent artifacts absent");
        return;
    }
    let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
    let res = Prover::load_versioned_with_expected_zkey(
        &build_dir,
        &artifact::LEVEL_B_V1_DESCRIPTOR,
        wrong,
    );
    assert!(
        res.is_err(),
        "a consent zkey whose hash differs from the pin must be refused (fail-closed)"
    );
}
