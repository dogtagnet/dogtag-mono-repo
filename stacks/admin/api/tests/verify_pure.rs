//! Unit coverage for the pure credential-import verification helpers in `admin_api::verify`.
//!
//! `structural_valid` and `dog_tag_id_of` are otherwise exercised only via the happy path of the
//! end-to-end `central.rs` mint test. These tests pin their error/edge branches directly by wrapping
//! a real document with the SDK and then mutating it: tampered roots, obfuscated (partial) docs,
//! empty data, malformed leaves, and the colon-preserving / missing-field behaviour of the dogTagId
//! extractor. Behaviour-preserving — no source change.

use dogtag_standard::wrap::{obfuscate, wrap_document, IssuerMeta, WrappedDoc};
use serde_json::{json, Value};

/// Deterministic 16-byte salt provider: returns [1;16], [2;16], ... so wraps are reproducible.
fn fixed_salts() -> impl FnMut() -> [u8; 16] {
    let mut n: u8 = 1;
    move || {
        let s = [n; 16];
        n = n.wrapping_add(1);
        s
    }
}

fn issuer() -> IssuerMeta {
    IssuerMeta {
        name: "Acme Vet".to_string(),
        domain: "acme.example".to_string(),
        document_store: "0x0000000000000000000000000000000000000001".to_string(),
        record_type: "DOG_PROFILE".to_string(),
    }
}

fn wrap(credential: &Value) -> WrappedDoc {
    let mut sp = fixed_salts();
    wrap_document(credential, issuer(), &mut sp).expect("wrap_document")
}

/// A fully-disclosed dog profile credential whose dogTagId is the integer "42".
fn sample() -> Value {
    json!({
        "credentialSubject": {
            "dogTagId": {"tag": 3, "value": "42"},
            "name": {"tag": 2, "value": "Rex"}
        }
    })
}

#[test]
fn structural_valid_accepts_a_freshly_wrapped_doc() {
    let doc = wrap(&sample());
    assert!(
        admin_api::verify::structural_valid(&doc),
        "a fully-disclosed wrapped doc must verify against its own recomputed root"
    );
}

#[test]
fn structural_valid_is_case_insensitive_on_the_root() {
    // The recomputed root is compared with eq_ignore_ascii_case, so an upper-cased embedded
    // merkleRoot must still verify.
    let mut doc = wrap(&sample());
    doc.signature.merkle_root = doc.signature.merkle_root.to_uppercase();
    assert!(admin_api::verify::structural_valid(&doc));
}

#[test]
fn structural_valid_rejects_a_tampered_root() {
    let mut doc = wrap(&sample());
    doc.signature.merkle_root = format!("0x{}", "0".repeat(64));
    assert!(
        !admin_api::verify::structural_valid(&doc),
        "a root that does not match the disclosed leaves must be rejected"
    );
}

#[test]
fn structural_valid_rejects_a_partially_obfuscated_doc() {
    // Once any field is obfuscated we cannot reconstruct its leaf from cleartext, so the importer
    // treats the doc as un-importable regardless of whether the root would otherwise match.
    let doc = wrap(&sample());
    let hidden = obfuscate(&doc, &["credentialSubject.name".to_string()]).expect("obfuscate");
    assert!(!hidden.privacy.obfuscated.is_empty());
    assert!(!admin_api::verify::structural_valid(&hidden));
}

#[test]
fn structural_valid_rejects_empty_data() {
    let mut doc = wrap(&sample());
    doc.data = json!({});
    assert!(
        !admin_api::verify::structural_valid(&doc),
        "a doc with no disclosed leaves cannot prove integrity"
    );
}

#[test]
fn structural_valid_rejects_a_malformed_leaf() {
    // Corrupt one packed value's salt to non-hex so leaf_from_packed errors and the whole doc fails.
    let mut doc = wrap(&sample());
    let cs = doc.data["credentialSubject"].as_object_mut().unwrap();
    cs.insert("name".to_string(), json!("zz:2:Rex"));
    assert!(!admin_api::verify::structural_valid(&doc));
}

#[test]
fn dog_tag_id_of_extracts_the_cleartext_value() {
    let doc = wrap(&sample());
    assert_eq!(
        admin_api::verify::dog_tag_id_of(&doc).as_deref(),
        Some("42")
    );
}

#[test]
fn dog_tag_id_of_preserves_colons_in_the_value() {
    // packed is salt:tag:value and the splitn(3, ':') keeps everything after the second colon, so a
    // value that itself contains colons round-trips intact.
    let doc = wrap(&json!({
        "credentialSubject": {
            "dogTagId": {"tag": 2, "value": "a:b:c"}
        }
    }));
    assert_eq!(
        admin_api::verify::dog_tag_id_of(&doc).as_deref(),
        Some("a:b:c")
    );
}

#[test]
fn dog_tag_id_of_is_none_when_field_absent() {
    let doc = wrap(&json!({
        "credentialSubject": {
            "name": {"tag": 2, "value": "Rex"}
        }
    }));
    assert_eq!(admin_api::verify::dog_tag_id_of(&doc), None);
}
