//! The captain's actual record CONTENT, driven through assembly and a REAL on-device-path Groth16
//! prove — the harness the phone cannot be (the panic never reaches the iOS unified log).
//!
//! Context (live walk 2026-08-09): sharing a record crashed the on-device prover with
//! `rustPanic "called Result::unwrap() on an Err value" / "witness thread panicked"`, with the
//! environment ELIMINATED — all four artifact pins match the repo byte-for-byte, both clones
//! grant the signer, the bundle carries current addresses. The strongest remaining hypothesis was
//! the record's own content: the portal's demo vaccination values ("Dr. Casey Rivera, DVM",
//! "RABVAC 3 TF", "LOT-2026-A17", "Boehringer Ingelheim", microchip 985112255719994, product code
//! "1351.20"), purpose `boarding_intake`.
//!
//! This test drives EXACTLY those values — with the tags the iOS mapping really assigns
//! (`Net.swift profileAttributeValues`: everything tag 2/String except `weightHistory[i].value`,
//! which is tag 4/Decimal; the product code is exercised as Decimal too since "1351.20" is the
//! only other decimal-shaped value) — through `prove_consent` against the committed
//! `consent_final.zkey` + `consent.graph`. Two raw-keccak words are passed deliberately:
//! `keccak("VACCINATION")` and the raw form a naive on-device derivation would send BOTH EXCEED
//! the BN254 modulus, and `field_from_word` must reduce them rather than reject or panic.
//!
//! VERDICT of the first run (recorded here so the hypothesis stays settled): these values PROVE —
//! assembly, witness and Groth16 all pass, so the crash was not this content through this path.
//! The panic class that DID reproduce is a witness-calculator rejection unwrapped inside the
//! dependency's witness thread (see `prover_ffi.rs` `a_mismatched_witness_graph_is_a_named_error_
//! never_a_panic`), which the pre-flight + panic backstop now render as errors. This test stays
//! as the content-regression pin: if a future circuit or encoding change makes any of these
//! ordinary values unprovable, it goes red naming the stage.
#![cfg(feature = "prover")]

use dogtag_standard::prover_ffi::prove_consent;

const ZKEY: &str = "../../circuits/build/consent_final.zkey";
const GRAPH: &str = "../../circuits/build/consent.graph";

fn attr(key_path: &str, tag: u8, value: &str, salt_byte: u8) -> serde_json::Value {
    serde_json::json!({
        "keyPath": key_path,
        "salt": format!("0x{}", hex::encode([salt_byte; 16])),
        "tag": tag,
        "value": value,
    })
}

#[test]
fn the_captains_demo_record_content_proves_end_to_end() {
    // Committed artifacts; an absent one is an incomplete checkout, not a skip.
    for p in [ZKEY, GRAPH] {
        assert!(std::path::Path::new(p).exists(), "incomplete checkout: {p} missing");
    }

    // The register-pet demo profile as the iOS mapping really tags it, plus every string the
    // captain's vaccination used, plus D1-style identity leaves. Salts are fixed per leaf — the
    // prover rebuilds the same tree the device persisted, so determinism is the point.
    let attributes = serde_json::Value::Array(vec![
        attr("credentialSubject.name", 2, "Rex", 1),
        attr("credentialSubject.species", 2, "Canis lupus familiaris", 2),
        attr("credentialSubject.breedVbo", 2, "VBO:0200798", 3),
        attr("credentialSubject.breedLabel", 2, "Labrador Retriever", 4),
        attr("credentialSubject.sex", 2, "male", 5),
        attr("credentialSubject.neuterStatus", 2, "neutered", 6),
        attr("credentialSubject.dateOfBirth", 2, "2021-05-01", 7),
        attr("credentialSubject.weightHistory[0].unit", 2, "kg", 8),
        // Tag 4 (Decimal) — the one non-String tag the iOS profile mapping emits.
        attr("credentialSubject.weightHistory[0].value", 4, "22.7", 9),
        attr("credentialSubject.weightHistory[0].measuredOn", 2, "2026-01-10", 10),
        attr("credentialSubject.microchip.code", 2, "985112255719994", 11),
        attr("credentialSubject.microchip.standard", 2, "ISO_11784_11785", 12),
        attr("credentialSubject.microchip.implantDate", 2, "2021-06-01", 13),
        attr("credentialSubject.microchip.bodyLocation", 2, "left neck", 14),
        // The captain's vaccination-form values, verbatim.
        attr("credentialSubject.veterinarian.name", 2, "Dr. Casey Rivera, DVM", 15),
        attr("credentialSubject.vaccineProductName", 2, "RABVAC 3 TF", 16),
        attr("credentialSubject.lotNumber", 2, "LOT-2026-A17", 17),
        attr("credentialSubject.manufacturer", 2, "Boehringer Ingelheim", 18),
        attr("credentialSubject.productCode", 4, "1351.20", 19),
        // D1 identity leaves (vet-salted in production; fixed salts here).
        attr("owner.identity.fullName", 2, "Dr. Casey Rivera, DVM", 20),
        attr("owner.identity.country", 2, "SG", 21),
    ]);

    // purpose = keccak("boarding_intake") % r (what vet-api's purpose_key sends);
    // recordType = RAW keccak("VACCINATION"), which EXCEEDS the BN254 modulus — the shape a naive
    // on-device derivation would pass, and `field_from_word` must reduce, never panic or refuse.
    let purpose = "0x0d35de973921c6fca6d7ad626fe13c4017a093733a6a21689b631b2c61b1c18d";
    let record_type_raw = "0x6510790a1a3e04db26bd73ea6246e7e8defb25eb4281f709e29decd6b8ca0561";

    let seed = b"captain record content harness seed - TEST MATERIAL ONLY";
    let out = prove_consent(
        format!("0x{}", hex::encode(seed)),
        "7".into(),
        format!("0x{}", hex::encode([0xabu8; 20])),
        attributes.to_string(),
        purpose.into(),
        format!("0x{}", hex::encode([0x11u8; 20])),
        record_type_raw.into(),
        format!("0x{}", hex::encode([0x22u8; 32])),
        "1893456000".into(),
        ZKEY.into(),
        GRAPH.into(),
    );

    match out {
        Ok(proof) => {
            assert_eq!(proof.pub_signals.len(), 7, "frozen 7-signal order");
            // The reduced recordType must round-trip into pub[5] — proof the raw ≥ r word was
            // reduced exactly as the on-chain binding (purpose_key) reduces it.
            let expected_rt = num_bigint::BigUint::parse_bytes(
                b"6510790a1a3e04db26bd73ea6246e7e8defb25eb4281f709e29decd6b8ca0561",
                16,
            )
            .unwrap()
                % num_bigint::BigUint::parse_bytes(
                    b"21888242871839275222246405745257275088548364400416034343698204186575808495617",
                    10,
                )
                .unwrap();
            assert_eq!(
                proof.pub_signals[5],
                expected_rt.to_string(),
                "recordType reduced mod r into pub[5]"
            );
        }
        Err(e) => panic!(
            "the captain's record content must prove (or this names the field the circuit \
             cannot take): {e}"
        ),
    }
}
