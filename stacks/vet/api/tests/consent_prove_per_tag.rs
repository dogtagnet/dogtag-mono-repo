//! Two tags of ONE wallet each produce a REAL consent proof the frozen VK accepts — and the two
//! verifications share nothing that could link them back to the same owner.
//!
//! This is the end of the chain the per-tag consent key exists to protect. `consent_prove.rs`
//! already proves that a single consent proof verifies against the frozen M3 VK; this one proves
//! the two claims the per-tag binding actually makes:
//!
//!   1. The derivation change is genuinely off-circuit — BOTH tags' proofs still verify against the
//!      SAME frozen `consent.circom` VK, so no circuit/VK/ceremony change was needed.
//!   2. The two verifications are unlinkable — different `(Ax, Ay)`, so a different committed
//!      `owner.consentKey` leaf, so a different `R` and a different `nullifier`. Nothing an observer
//!      sees on-chain for tag A appears in tag B's verification.
//!
//! Run with `-- --nocapture` to read the transcript. Self-skips when the committed consent
//! artifacts are absent, matching `consent_prove.rs`.
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

fn repo_root() -> PathBuf {
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

/// One recovery phrase, two pets. TEST MATERIAL ONLY, never holds value.
const SEED: &[u8] = b"per-tag consent prove wallet seed - TEST MATERIAL ONLY, never hold value";

fn owner_address() -> [u8; 20] {
    let mut a = [0u8; 20];
    a[16..].copy_from_slice(&0xdead_beefu32.to_be_bytes());
    a
}

fn attrs(pet_name: &str) -> Vec<AttributeLeaf> {
    vec![
        AttributeLeaf {
            key_path: "credentialSubject.name".to_string(),
            salt: [7u8; SALT_LEN],
            value: TypedScalar::Str(pet_name.to_string()),
        },
        AttributeLeaf {
            key_path: "credentialSubject.breedLabel".to_string(),
            salt: [9u8; SALT_LEN],
            value: TypedScalar::Str("Shiba Inu".to_string()),
        },
    ]
}

fn witness<'a>(handle: &'a str, attributes: &'a [AttributeLeaf]) -> ConsentWitness<'a> {
    ConsentWitness {
        seed: SEED,
        dog_tag_id_handle: handle,
        owner_address: owner_address(),
        attributes,
        // Same disclosed session params for both tags, so any linkage that shows up comes from the
        // owner-control core rather than from the verifier's own inputs.
        purpose: Fr::from(7u64),
        relayer: [0x11u8; 20],
        record_type: Fr::from(19u64),
        deadline: Fr::from(1_893_456_000u64), // 2030-01-01
        consent_nonce: Fr::from(99u64),
    }
}

const LABELS: [&str; 7] = [
    "dogTagId",
    "purpose",
    "relayer",
    "nullifier",
    "R",
    "recordType",
    "deadline",
];

#[test]
fn two_tags_of_one_wallet_both_verify_against_the_frozen_vk_and_are_unlinkable() {
    let build_dir = repo_root().join("circuits").join("build");
    if !build_dir.join("consent_final.zkey").exists() {
        eprintln!(
            "SKIP: consent artifacts absent (circuits/build/consent_final.zkey) — cannot exercise \
             the real consent prove/verify path"
        );
        return;
    }

    let prover = Prover::load_versioned(&build_dir, &artifact::LEVEL_B_V1_DESCRIPTOR)
        .expect("load consent prover");
    assert_eq!(prover.version(), artifact::LEVEL_B_V1);
    assert_eq!(prover.zkey_hash_hex(), artifact::LEVEL_B_V1_DESCRIPTOR.zkey.sha256);

    let mut proven = Vec::new();
    for (handle, pet) in [("424242", "Rex"), ("424243", "Milo")] {
        let a = attrs(pet);
        let inp = assemble_consent(&witness(handle, &a)).expect("assemble consent inputs");
        let inputs =
            ConsentProveInputs::from_circuit_input_json(&consent_circuit_input_value(&inp))
                .expect("parse consent circuit input");
        // `prove_consent_inputs` self-verifies against the zkey's embedded verifying key — the
        // FROZEN M3 ceremony VK. A returned proof IS a proof that verifies under it.
        let out = prover
            .prove_consent_inputs(inputs)
            .expect("consent prove (self-verifies against the frozen VK)");
        assert_eq!(out.public_signals[4], fe_to_dec(&inp.root), "pub[4] must be R");
        proven.push((pet, inp, out));
    }

    let (_, a_in, a_out) = &proven[0];
    let (_, b_in, b_out) = &proven[1];

    println!("\n=== Two tags of ONE wallet, each proved against the FROZEN consent VK ===");
    println!(
        "prover version {}  |  zkey sha256 {}\n",
        prover.version(),
        prover.zkey_hash_hex()
    );
    println!("{:<12} {:<26} {:<26}", "public signal", "Rex (tag 424242)", "Milo (tag 424243)");
    for (i, label) in LABELS.iter().enumerate() {
        let (x, y) = (&a_out.public_signals[i], &b_out.public_signals[i]);
        let verdict = if x == y { "same (disclosed session param)" } else { "unlinkable" };
        let trunc = |s: &String| if s.len() > 24 { format!("{}…", &s[..23]) } else { s.clone() };
        println!("pub[{i}] {label:<6} {:<26} {:<26} {verdict}", trunc(x), trunc(y));
    }
    println!(
        "\nconsent Ax   {:<26} {:<26} unlinkable",
        &a_in.ax[..a_in.ax.len().min(23)],
        &b_in.ax[..b_in.ax.len().min(23)]
    );
    println!("\nBoth proofs verified against the frozen VK: yes (2/2)\n");

    // 1. Off-circuit only: both verified under the SAME frozen VK (the proves above did not error).
    // 2. Unlinkable: the owner-control-derived signals all differ.
    assert_ne!(a_out.public_signals[0], b_out.public_signals[0], "dogTagId must differ");
    assert_ne!(
        a_out.public_signals[3], b_out.public_signals[3],
        "nullifier must differ — the two verifications must not be linkable on-chain"
    );
    assert_ne!(
        a_out.public_signals[4], b_out.public_signals[4],
        "R must differ — the committed owner.consentKey leaf differs per tag"
    );
    assert_ne!(a_in.ax, b_in.ax, "the proved consent Ax must differ per tag");
    assert_ne!(a_in.ay, b_in.ay, "the proved consent Ay must differ per tag");

    // The disclosed session params are shared by construction — that is the verifier's own input,
    // not owner-derived, and asserting it keeps the unlinkability claim honest about its scope.
    for i in [1usize, 2, 5, 6] {
        assert_eq!(
            a_out.public_signals[i], b_out.public_signals[i],
            "pub[{i}] ({}) is a disclosed session param and should be identical here",
            LABELS[i]
        );
    }
}
