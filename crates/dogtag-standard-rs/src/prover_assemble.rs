//! Circuit-input ASSEMBLY (Workstream A, server-share) — the pure, prover-independent half of the
//! on-device proving path, gated behind the lightweight `assemble` feature.
//!
//! This module assembles the circuit's 19 named inputs from a `WrappedDoc` + the §1.10 consent JSON +
//! the pass-through EdDSA-BabyJubjub signature, and emits them as a name -> decimal-string map
//! (`input_map`) — exactly the shape `dogtag-prover-rs::ProveInputs::from_circuit_input_json`
//! consumes. It is the SAME assembly the on-device `prover_ffi::prove_verification` runs.
//!
//! It is split out from `prover_ffi` so it can be compiled WITHOUT the heavy ark-0.5 `circom-prover`
//! witness/proving stack. The assembly only needs the SDK's own `ark-bn254`/`ark-ff` field types +
//! `wrap`/`leaf`/`field`/`merkle` — it does NOT prove. This lets the 64-bit backend
//! (`vet-api`, which already links the ark-0.6 `dogtag-prover-rs`) reuse the assembly to drive the
//! SERVER PROVING API for 32-bit-only Android phones, while keeping the two ark majors disjoint:
//! only DECIMAL STRINGS cross the crate boundary (no ark types), so there is no version clash.
//!
//! `prover_ffi` re-exports `assemble` / `input_map` / `AssembledInputs` / `EddsaSigInput` from here
//! and layers the actual circom-prover proving on top.

// `HashMap` is only used by the on-device `input_map` (gated to the `prover` feature).
#[cfg(feature = "prover")]
use std::collections::HashMap;

use ark_bn254::Fr;
use ark_ff::PrimeField;
use num_bigint::BigInt;
use serde_json::Value;

use crate::consent::VerificationConsent;
use crate::ffi::FfiError;
use crate::field::{field_from_scalar_bytes, to_hex32};
use crate::leaf::{field_of_keypath, field_of_value, hash_leaf};
use crate::merkle::build_merkle;
use crate::types::{TypeTag, TypedScalar};
use crate::wrap::{flatten_data, parse_packed, scalar_from_packed, WrappedDoc};

/// `N` — the circuit's max leaf width (`DogTagVerification(24, 5)`).
pub(crate) const N: usize = 24;
/// The canonical key path of the dogTagId leaf (bound by the circuit via `dogTagKeyPathField`).
pub(crate) const DOG_TAG_KEY_PATH: &str = "credentialSubject.dogTagId";

/// The pass-through EdDSA-BabyJubjub consent signature + public key (decimal scalars + hex point).
///
/// `r8x_dec` / `r8y_dec` / `s_dec` come from `sign_consent_eddsa` (ffi.rs `EddsaSignatureFfi`);
/// `ax_hex` / `ay_hex` are the consent public point (0x.. 32-byte BE field hex).
#[derive(uniffi::Record)]
pub struct EddsaSigInput {
    pub r8x_dec: String,
    pub r8y_dec: String,
    pub s_dec: String,
    pub ax_hex: String,
    pub ay_hex: String,
}

pub(crate) fn err<E: std::fmt::Display>(e: E) -> FfiError {
    FfiError::Invalid(e.to_string())
}

/// Convert a field element to its base-10 decimal string.
pub(crate) fn fe_to_dec(f: &Fr) -> String {
    f.into_bigint().to_string()
}

/// Parse a 0x.. 32-byte BE hex into a field element.
fn field_from_hex(label: &str, h: &str) -> Result<Fr, FfiError> {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let bytes = hex::decode(s).map_err(|e| err(format!("bad {label} hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(FfiError::Invalid(format!(
            "{label} hex must be 32 bytes (got {})",
            bytes.len()
        )));
    }
    Ok(Fr::from_be_bytes_mod_order(&bytes))
}

/// Decode a 0x.. hex string into exactly `M` bytes (BE word / address).
fn decode_word<const M: usize>(label: &str, h: &str) -> Result<[u8; M], FfiError> {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let bytes = hex::decode(s).map_err(|e| err(format!("bad {label} hex: {e}")))?;
    if bytes.len() != M {
        return Err(FfiError::Invalid(format!(
            "{label} must be {M} bytes (got {})",
            bytes.len()
        )));
    }
    let mut out = [0u8; M];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// Parse the consent JSON (same shape as ffi.rs `consent_from_hex` / the POSTed consent) into a
/// `VerificationConsent`. Fields are 0x.. hex: uint256/bytes32 are 32-byte BE, addresses 20-byte.
pub(crate) fn consent_from_json(consent: &Value) -> Result<VerificationConsent, FfiError> {
    let s = |k: &str| -> Result<String, FfiError> {
        consent
            .get(k)
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .ok_or_else(|| FfiError::Invalid(format!("consent.{k}: missing or not a string")))
    };
    Ok(VerificationConsent {
        dog_tag_id: decode_word::<32>("dogTagId", &s("dogTagId")?)?,
        record_type: decode_word::<32>("recordType", &s("recordType")?)?,
        purpose: decode_word::<32>("purpose", &s("purpose")?)?,
        credential_root: decode_word::<32>("credentialRoot", &s("credentialRoot")?)?,
        challenge: decode_word::<32>("challenge", &s("challenge")?)?,
        relayer: decode_word::<20>("relayer", &s("relayer")?)?,
        subject: decode_word::<20>("subject", &s("subject")?)?,
        nonce: decode_word::<32>("nonce", &s("nonce")?)?,
        deadline: decode_word::<32>("deadline", &s("deadline")?)?,
    })
}

/// The 19 assembled circuit inputs (all decimal-string field elements; arrays of width `N`).
/// Mirrors `dogtag-prover-rs::ProveInputs` field-for-field so the input contract is identical.
pub(crate) struct AssembledInputs {
    pub(crate) dog_tag_id: String,
    pub(crate) purpose: String,
    pub(crate) relayer: String,
    pub(crate) subject: String,
    pub(crate) num_leaves: String,
    pub(crate) leaf_key_path_hashes: [String; N],
    pub(crate) leaf_type_tags: [String; N],
    pub(crate) leaf_salts: [String; N],
    pub(crate) leaf_values: [String; N],
    pub(crate) dog_tag_id_leaf_index: String,
    pub(crate) sorted_leaf_hashes: [String; N],
    pub(crate) perm: [String; N],
    pub(crate) dog_tag_key_path_field: String,
    pub(crate) consent_nonce: String,
    pub(crate) ax: String,
    pub(crate) ay: String,
    pub(crate) r8x: String,
    pub(crate) r8y: String,
    pub(crate) s: String,
    /// The merkle root over the active leaves (== credentialRoot, asserted by caller). Read by the
    /// assembler parity test.
    #[allow(dead_code)]
    pub(crate) root: Fr,
}

/// Per-leaf assembled fields in canonical (flatten) order.
struct LeafFields {
    key_path_hash: Fr,
    type_tag: u8,
    salt_field: Fr,
    value_field: Fr,
    /// The full leaf hash = Poseidon(DS_LEAF, kp, salt, tag, value).
    hash: Fr,
}

/// Assemble all 19 circuit inputs from the wrapped doc, consent, and EdDSA signature.
///
/// MIRRORS `circuits/scripts/gen-zk-fixture.mjs` + `dogtag-prover-rs::push_named_inputs` ordering:
/// per-leaf fields in canonical flatten order; `sortedLeafHashes` = the ascending-sorted active
/// prefix (`merkle::build_merkle(active).layers[0]`); `perm[k]` = index in the canonical active
/// order of `sortedLeafHashes[k]`; padding slots `[numLeaves, N)` pinned diagonal (`perm[k]=k`,
/// dummy leaf). Asserts `root == credentialRoot` and `numLeaves <= N`.
pub(crate) fn assemble(
    doc: &WrappedDoc,
    consent: &VerificationConsent,
    sig: &EddsaSigInput,
) -> Result<AssembledInputs, FfiError> {
    // 1. Canonical-ordered (keyPath, packed) pairs (same order verify/wrap use).
    let pairs = flatten_data(&doc.data);
    let num_leaves = pairs.len();
    if num_leaves == 0 {
        return Err(FfiError::Invalid("wrapped doc has no leaves".into()));
    }
    if num_leaves > N {
        return Err(FfiError::Invalid(format!(
            "too many leaves: {num_leaves} > N={N}"
        )));
    }

    // 2. Per-leaf assembled fields in canonical order + the dogTagId leaf index.
    let mut leaves: Vec<LeafFields> = Vec::with_capacity(num_leaves);
    let mut dog_tag_id_leaf_index: Option<usize> = None;
    for (i, (key_path, packed)) in pairs.iter().enumerate() {
        let (salt_hex, tag, value_rest) = parse_packed(packed).map_err(FfiError::from)?;
        let salt = hex::decode(&salt_hex).map_err(|e| err(format!("bad salt hex: {e}")))?;
        if salt.len() != 16 {
            return Err(FfiError::Invalid(format!(
                "salt must be 16 bytes (got {})",
                salt.len()
            )));
        }
        let scalar: TypedScalar = scalar_from_packed(tag, &value_rest).map_err(FfiError::from)?;
        let hash = hash_leaf(key_path, &salt, &scalar).map_err(FfiError::from)?;
        leaves.push(LeafFields {
            key_path_hash: field_of_keypath(key_path),
            type_tag: tag as u8,
            salt_field: field_from_scalar_bytes(&salt),
            value_field: field_of_value(&scalar).map_err(FfiError::from)?,
            hash,
        });
        if key_path == DOG_TAG_KEY_PATH {
            dog_tag_id_leaf_index = Some(i);
        }
    }
    let dog_tag_id_leaf_index = dog_tag_id_leaf_index.ok_or_else(|| {
        FfiError::Invalid(format!("missing required leaf {DOG_TAG_KEY_PATH}"))
    })?;

    // 3. Merkle tree over the ACTIVE leaves (the canonical-order hashes). The SDK sorts ascending
    //    and folds bottom-up; layers[0] is the ascending-sorted leaf order = sortedLeafHashes.
    let active_hashes: Vec<Fr> = leaves.iter().map(|l| l.hash).collect();
    let tree = build_merkle(&active_hashes);
    let sorted_active = &tree.layers[0];
    let root = tree.root;

    // 3a. Assert the tree root == the consent's credentialRoot (defensive — circuit also binds R).
    let consent_root = Fr::from_be_bytes_mod_order(&consent.credential_root);
    if root != consent_root {
        return Err(FfiError::Invalid(format!(
            "merkle root {} != consent.credentialRoot {}",
            to_hex32(&root),
            to_hex32(&consent_root)
        )));
    }

    // 4. Build the N-wide arrays. Active prefix [0, numLeaves): canonical-order leaf fields, with
    //    sortedLeafHashes = sorted active order and perm[k] = canonical index of sortedActive[k].
    //    Padding slots [numLeaves, N): a dummy diagonal leaf (perm[k]=k, active[k]=0 in-circuit).
    let mut leaf_key_path_hashes: [String; N] = Default::default();
    let mut leaf_type_tags: [String; N] = Default::default();
    let mut leaf_salts: [String; N] = Default::default();
    let mut leaf_values: [String; N] = Default::default();
    let mut sorted_leaf_hashes: [String; N] = Default::default();
    let mut perm: [String; N] = Default::default();

    // A deterministic dummy leaf hash for the padding slots (value never affects the proof because
    // the circuit gates each slot by active[k] = (k < numLeaves); we just need a valid field el).
    let dummy = hash_leaf(
        "__pad__",
        &[0u8; 16],
        &TypedScalar::Integer("0".to_string()),
    )
    .map_err(FfiError::from)?;
    let dummy_kp = field_of_keypath("__pad__");
    let dummy_tag = TypeTag::Integer as u8;
    let dummy_salt = field_from_scalar_bytes(&[0u8; 16]);
    let dummy_val = field_of_value(&TypedScalar::Integer("0".to_string())).map_err(FfiError::from)?;

    for k in 0..N {
        if k < num_leaves {
            let lf = &leaves[k];
            leaf_key_path_hashes[k] = fe_to_dec(&lf.key_path_hash);
            leaf_type_tags[k] = (lf.type_tag as u64).to_string();
            leaf_salts[k] = fe_to_dec(&lf.salt_field);
            leaf_values[k] = fe_to_dec(&lf.value_field);

            let sorted = sorted_active[k];
            sorted_leaf_hashes[k] = fe_to_dec(&sorted);
            // perm[k] = canonical index of the sorted leaf (findIndex in the active prefix).
            let idx = active_hashes
                .iter()
                .position(|h| *h == sorted)
                .expect("sorted leaf must exist in active set");
            perm[k] = idx.to_string();
        } else {
            // Diagonal padding slot.
            leaf_key_path_hashes[k] = fe_to_dec(&dummy_kp);
            leaf_type_tags[k] = (dummy_tag as u64).to_string();
            leaf_salts[k] = fe_to_dec(&dummy_salt);
            leaf_values[k] = fe_to_dec(&dummy_val);
            sorted_leaf_hashes[k] = fe_to_dec(&dummy);
            perm[k] = k.to_string();
        }
    }

    // 5. Scalars from the consent (decimal field elements; addresses as uint160).
    let dog_tag_id = fe_to_dec(&Fr::from_be_bytes_mod_order(&consent.dog_tag_id));
    let purpose = fe_to_dec(&Fr::from_be_bytes_mod_order(&consent.purpose));
    let relayer = fe_to_dec(&Fr::from_be_bytes_mod_order(&consent.relayer));
    let subject = fe_to_dec(&Fr::from_be_bytes_mod_order(&consent.subject));
    let consent_nonce = fe_to_dec(&Fr::from_be_bytes_mod_order(&consent.nonce));

    // 6. dogTagKeyPathField — fieldOf(canonical dogTagId key path), bound by the circuit.
    let dog_tag_key_path_field = fe_to_dec(&field_of_keypath(DOG_TAG_KEY_PATH));

    // 7. EdDSA Ax/Ay (hex -> decimal field) + R8x/R8y/S (already decimal).
    let ax = fe_to_dec(&field_from_hex("Ax", &sig.ax_hex)?);
    let ay = fe_to_dec(&field_from_hex("Ay", &sig.ay_hex)?);
    // R8x/R8y/S are pass-through decimal strings; validate they parse as integers.
    let r8x = parse_dec("R8x", &sig.r8x_dec)?;
    let r8y = parse_dec("R8y", &sig.r8y_dec)?;
    let s = parse_dec("S", &sig.s_dec)?;

    Ok(AssembledInputs {
        dog_tag_id,
        purpose,
        relayer,
        subject,
        num_leaves: num_leaves.to_string(),
        leaf_key_path_hashes,
        leaf_type_tags,
        leaf_salts,
        leaf_values,
        dog_tag_id_leaf_index: dog_tag_id_leaf_index.to_string(),
        sorted_leaf_hashes,
        perm,
        dog_tag_key_path_field,
        consent_nonce,
        ax,
        ay,
        r8x,
        r8y,
        s,
        root,
    })
}

/// Validate that `value` is a base-10 integer string, returning it unchanged.
fn parse_dec(field: &str, value: &str) -> Result<String, FfiError> {
    value
        .parse::<BigInt>()
        .map_err(|e| FfiError::Invalid(format!("{field}: not a decimal integer: {e}")))?;
    Ok(value.to_string())
}

/// Build the circom-prover input map (signal name -> decimal-string values), matching the circuit's
/// named inputs and ordering in `push_named_inputs`. ONLY the on-device `prover_ffi` (the full
/// `prover` feature) consumes this — the server path uses `prover_input_value` instead — so it is
/// gated to that feature to avoid a dead-code warning in the `assemble`-only (backend) build.
#[cfg(feature = "prover")]
pub(crate) fn input_map(inp: &AssembledInputs) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    m.insert("dogTagId".into(), vec![inp.dog_tag_id.clone()]);
    m.insert("purpose".into(), vec![inp.purpose.clone()]);
    m.insert("relayer".into(), vec![inp.relayer.clone()]);
    m.insert("subject".into(), vec![inp.subject.clone()]);
    m.insert("numLeaves".into(), vec![inp.num_leaves.clone()]);
    m.insert("leafKeyPathHashes".into(), inp.leaf_key_path_hashes.to_vec());
    m.insert("leafTypeTags".into(), inp.leaf_type_tags.to_vec());
    m.insert("leafSalts".into(), inp.leaf_salts.to_vec());
    m.insert("leafValues".into(), inp.leaf_values.to_vec());
    m.insert(
        "dogTagIdLeafIndex".into(),
        vec![inp.dog_tag_id_leaf_index.clone()],
    );
    m.insert("sortedLeafHashes".into(), inp.sorted_leaf_hashes.to_vec());
    m.insert("perm".into(), inp.perm.to_vec());
    m.insert(
        "dogTagKeyPathField".into(),
        vec![inp.dog_tag_key_path_field.clone()],
    );
    m.insert("consentNonce".into(), vec![inp.consent_nonce.clone()]);
    m.insert("Ax".into(), vec![inp.ax.clone()]);
    m.insert("Ay".into(), vec![inp.ay.clone()]);
    m.insert("R8x".into(), vec![inp.r8x.clone()]);
    m.insert("R8y".into(), vec![inp.r8y.clone()]);
    m.insert("S".into(), vec![inp.s.clone()]);
    m
}

/// Serialize the assembled inputs into the `gen-zk-fixture.mjs` / `tests/gen_input.mjs` JSON shape
/// that `dogtag-prover-rs::ProveInputs::from_circuit_input_json` consumes: SCALAR signals are bare
/// decimal STRINGS, the six width-`N` signals are decimal-string ARRAYS.
///
/// NOTE: this is DIFFERENT from `input_map` (the circom-prover format, where every signal — even a
/// scalar — is a `Vec<String>`). The ark-0.6 `ProveInputs` parser requires scalars unwrapped, so the
/// server proving path uses THIS shape, not `input_map`.
fn prover_input_value(inp: &AssembledInputs) -> Value {
    let arr = |a: &[String; N]| Value::Array(a.iter().map(|s| Value::String(s.clone())).collect());
    serde_json::json!({
        "dogTagId": inp.dog_tag_id,
        "purpose": inp.purpose,
        "relayer": inp.relayer,
        "subject": inp.subject,
        "numLeaves": inp.num_leaves,
        "leafKeyPathHashes": arr(&inp.leaf_key_path_hashes),
        "leafTypeTags": arr(&inp.leaf_type_tags),
        "leafSalts": arr(&inp.leaf_salts),
        "leafValues": arr(&inp.leaf_values),
        "dogTagIdLeafIndex": inp.dog_tag_id_leaf_index,
        "sortedLeafHashes": arr(&inp.sorted_leaf_hashes),
        "perm": arr(&inp.perm),
        "dogTagKeyPathField": inp.dog_tag_key_path_field,
        "consentNonce": inp.consent_nonce,
        "Ax": inp.ax,
        "Ay": inp.ay,
        "R8x": inp.r8x,
        "R8y": inp.r8y,
        "S": inp.s,
    })
}

/// PUBLIC server-side assembly entry point — assemble the circuit's 19 named inputs from the SAME
/// inputs the on-device `prover_ffi::prove_verification` takes (the stored `WrappedDoc` JSON, the
/// §1.10 consent JSON, and the EdDSA-BabyJubjub consent signature + public key) and return them as a
/// `serde_json::Value` in EXACTLY the shape `dogtag-prover-rs::ProveInputs::from_circuit_input_json`
/// consumes (scalars as strings, the six width-`N` signals as string arrays).
///
/// This is the seam the SERVER PROVING API uses to support 32-bit-only Android phones that cannot run
/// the on-device circom-prover: the phone POSTs `{wrappedDoc, consent, eddsaSig}`, the backend
/// assembles here (ark-0.5 field types stay internal — only decimal strings escape) and proves with
/// the ark-0.6 Arkworks prover, then returns the Groth16 calldata. The witness (the wrapped doc) is
/// seen ONLY by this assembly step on the trusted prover host — the proof it yields is then submitted
/// to the groomer, which never sees the witness.
pub fn assemble_circuit_input(
    wrapped_doc_json: &str,
    consent_json: &str,
    eddsa_sig: &EddsaSigInput,
) -> Result<Value, FfiError> {
    let doc: WrappedDoc = serde_json::from_str(wrapped_doc_json)
        .map_err(|e| err(format!("bad wrapped doc json: {e}")))?;
    let consent_v: Value =
        serde_json::from_str(consent_json).map_err(|e| err(format!("bad consent json: {e}")))?;
    let consent = consent_from_json(&consent_v)?;
    let inp = assemble(&doc, &consent, eddsa_sig)?;
    Ok(prover_input_value(&inp))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the SAME deterministic input the gen-zk-fixture / tests/gen_input.mjs reference uses
    /// (numLeaves=13, dogTagId=424242, ...) directly in Rust and assert the assembled
    /// sortedLeafHashes / perm / dogTagIdLeafIndex / R match the reference fixture's structure.
    ///
    /// We construct leaves the same way the fixture does — but the FIXTURE uses synthetic numeric
    /// keyPath/salt/value fields directly, whereas our assembler derives them from a WrappedDoc. So
    /// this unit test exercises the assembler's tree/perm logic against an independently-built tree
    /// using the SDK merkle, catching assembly drift without the slow prove.
    #[test]
    fn assembler_tree_and_perm_match_sdk_merkle() {
        use crate::merkle::build_merkle;

        // A small deterministic WrappedDoc with a dogTagId leaf + a few others.
        let doc_json = serde_json::json!({
            "version": "dogtag/1.0",
            "data": {
                "credentialSubject": {
                    "dogTagId": "00000000000000000000000000000001:3:424242",
                    "name": "00000000000000000000000000000002:2:Rex",
                    "breed": "00000000000000000000000000000003:2:Labrador",
                    "microchip": "00000000000000000000000000000004:2:985141006580311"
                }
            },
            "signature": {"type":"DogTagMerkleProof","targetHash":"0x00","proof":[],"merkleRoot":"0x00"},
            "privacy": {"obfuscated": []},
            "issuer": {"name":"V","domain":"v.example","documentStore":"0x0000000000000000000000000000000000000001","recordType":"VACCINATION"}
        });
        let doc: WrappedDoc = serde_json::from_value(doc_json).unwrap();

        // Recompute the active leaves + tree independently from flatten_data.
        let pairs = flatten_data(&doc.data);
        let mut active: Vec<Fr> = Vec::new();
        let mut dog_idx = None;
        for (i, (kp, packed)) in pairs.iter().enumerate() {
            let (salt_hex, tag, val) = parse_packed(packed).unwrap();
            let salt = hex::decode(&salt_hex).unwrap();
            let scalar = scalar_from_packed(tag, &val).unwrap();
            active.push(hash_leaf(kp, &salt, &scalar).unwrap());
            if kp == DOG_TAG_KEY_PATH {
                dog_idx = Some(i);
            }
        }
        let tree = build_merkle(&active);
        let root = tree.root;

        // Build a consent whose credentialRoot == the tree root so assemble() passes its assert.
        let consent_json = serde_json::json!({
            "dogTagId": "0x0000000000000000000000000000000000000000000000000000000000067932",
            "recordType": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "purpose": "0x0000000000000000000000000000000000000000000000000000000000000007",
            "credentialRoot": to_hex32(&root),
            "challenge": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "relayer": "0x1111111111111111111111111111111111111111",
            "subject": "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
            "nonce": "0x0000000000000000000000000000000000000000000000000000000000000063",
            "deadline": "0x0000000000000000000000000000000000000000000000000000000000000000"
        });
        let consent = consent_from_json(&consent_json).unwrap();
        let sig = EddsaSigInput {
            r8x_dec: "1".into(),
            r8y_dec: "2".into(),
            s_dec: "3".into(),
            ax_hex: to_hex32(&Fr::from(4u64)),
            ay_hex: to_hex32(&Fr::from(5u64)),
        };

        let asm = assemble(&doc, &consent, &sig).unwrap();

        // numLeaves / dogTagIdLeafIndex
        assert_eq!(asm.num_leaves, pairs.len().to_string());
        assert_eq!(
            asm.dog_tag_id_leaf_index,
            dog_idx.unwrap().to_string(),
            "dogTagIdLeafIndex must be the canonical flatten index"
        );

        // R == tree root
        assert_eq!(asm.root, root);

        // sortedLeafHashes (active prefix) == ascending-sorted active order = layers[0]
        let sorted_active = &tree.layers[0];
        for (k, sa) in sorted_active.iter().enumerate().take(pairs.len()) {
            assert_eq!(
                asm.sorted_leaf_hashes[k],
                fe_to_dec(sa),
                "sortedLeafHashes[{k}] mismatch"
            );
            // perm[k] maps back to the canonical leaf
            let p: usize = asm.perm[k].parse().unwrap();
            assert_eq!(
                active[p], *sa,
                "perm[{k}] must point at the canonical leaf equal to sortedActive[{k}]"
            );
        }
        // Padding slots pinned diagonal.
        for k in pairs.len()..N {
            assert_eq!(asm.perm[k], k.to_string(), "padding perm[{k}] must be diagonal");
        }
    }

    #[test]
    fn assemble_rejects_root_mismatch() {
        let doc_json = serde_json::json!({
            "version": "dogtag/1.0",
            "data": { "credentialSubject": { "dogTagId": "00000000000000000000000000000001:3:42" } },
            "signature": {"type":"DogTagMerkleProof","targetHash":"0x00","proof":[],"merkleRoot":"0x00"},
            "privacy": {"obfuscated": []},
            "issuer": {"name":"V","domain":"v.example","documentStore":"0x0000000000000000000000000000000000000001","recordType":"VACCINATION"}
        });
        let doc: WrappedDoc = serde_json::from_value(doc_json).unwrap();
        let consent_json = serde_json::json!({
            "dogTagId": "0x000000000000000000000000000000000000000000000000000000000000002a",
            "recordType": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "purpose": "0x0000000000000000000000000000000000000000000000000000000000000007",
            "credentialRoot": "0x0000000000000000000000000000000000000000000000000000000000000001",
            "challenge": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "relayer": "0x1111111111111111111111111111111111111111",
            "subject": "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
            "nonce": "0x0000000000000000000000000000000000000000000000000000000000000063",
            "deadline": "0x0000000000000000000000000000000000000000000000000000000000000000"
        });
        let consent = consent_from_json(&consent_json).unwrap();
        let sig = EddsaSigInput {
            r8x_dec: "1".into(), r8y_dec: "2".into(), s_dec: "3".into(),
            ax_hex: to_hex32(&Fr::from(4u64)), ay_hex: to_hex32(&Fr::from(5u64)),
        };
        let res = assemble(&doc, &consent, &sig);
        assert!(res.is_err(), "must reject when root != credentialRoot");
    }

    fn invalid_msg(e: &FfiError) -> String {
        match e {
            FfiError::Invalid(m) => m.clone(),
        }
    }

    #[test]
    fn field_from_hex_round_trips_and_strips_prefix() {
        // 0x.. and bare hex of the same 32-byte word decode to the same field element.
        let with = field_from_hex("x", &to_hex32(&Fr::from(7u64))).unwrap();
        let bare = field_from_hex("x", to_hex32(&Fr::from(7u64)).strip_prefix("0x").unwrap()).unwrap();
        assert_eq!(with, Fr::from(7u64));
        assert_eq!(bare, Fr::from(7u64));
    }

    #[test]
    fn field_from_hex_reduces_mod_order_instead_of_rejecting() {
        // Unlike wrap::from_hex32's strict canonical guard, field_from_hex silently reduces an
        // at-or-above-modulus input mod the BN254 scalar field (from_be_bytes_mod_order).
        let all_ones = field_from_hex("x", &format!("0x{}", "ff".repeat(32))).unwrap();
        // 2^256 mod r is a small, well-known reduced value; just assert it did NOT error and is a
        // canonical (already-reduced) element by re-encoding through to_hex32 round-trip stability.
        let re = field_from_hex("x", &to_hex32(&all_ones)).unwrap();
        assert_eq!(all_ones, re);
    }

    #[test]
    fn field_from_hex_rejects_bad_hex_and_wrong_length() {
        assert!(invalid_msg(&field_from_hex("sig", "0xzz").unwrap_err()).contains("bad sig hex"));
        let short = format!("0x{}", "00".repeat(31));
        assert!(invalid_msg(&field_from_hex("sig", &short).unwrap_err())
            .contains("sig hex must be 32 bytes (got 31)"));
    }

    #[test]
    fn decode_word_exact_length_and_errors() {
        // 20-byte address, with prefix.
        let addr = decode_word::<20>("relayer", "0x1111111111111111111111111111111111111111").unwrap();
        assert_eq!(addr, [0x11u8; 20]);
        // Bare hex (no 0x) of a 32-byte word.
        let w = decode_word::<32>("nonce", &"00".repeat(32)).unwrap();
        assert_eq!(w, [0u8; 32]);
        // Wrong length embeds the actual byte count.
        assert!(invalid_msg(&decode_word::<32>("nonce", "0x00").unwrap_err())
            .contains("nonce must be 32 bytes (got 1)"));
        // Bad hex.
        assert!(invalid_msg(&decode_word::<20>("relayer", "0xgg").unwrap_err())
            .contains("bad relayer hex"));
    }

    #[test]
    fn parse_dec_preserves_string_and_rejects_non_integers() {
        // Returned verbatim, leading zeros and sign preserved.
        assert_eq!(parse_dec("n", "12345").unwrap(), "12345");
        assert_eq!(parse_dec("n", "007").unwrap(), "007");
        assert_eq!(parse_dec("n", "-5").unwrap(), "-5");
        // Non base-10 integers rejected with the field label.
        assert!(invalid_msg(&parse_dec("Ax", "12a").unwrap_err())
            .contains("Ax: not a decimal integer"));
        assert!(parse_dec("Ax", "").is_err());
        assert!(parse_dec("Ax", " 5 ").is_err());
    }

    #[test]
    fn consent_from_json_validates_fields() {
        let base = serde_json::json!({
            "dogTagId": "0x".to_string() + &"00".repeat(32),
            "recordType": "0x".to_string() + &"00".repeat(32),
            "purpose": "0x".to_string() + &"00".repeat(32),
            "credentialRoot": "0x".to_string() + &"00".repeat(32),
            "challenge": "0x".to_string() + &"00".repeat(32),
            "relayer": "0x".to_string() + &"11".repeat(20),
            "subject": "0x".to_string() + &"22".repeat(20),
            "nonce": "0x".to_string() + &"00".repeat(32),
            "deadline": "0x".to_string() + &"00".repeat(32),
        });
        let c = consent_from_json(&base).unwrap();
        assert_eq!(c.relayer, [0x11u8; 20]);
        assert_eq!(c.subject, [0x22u8; 20]);

        // Missing field -> "consent.<k>: missing or not a string".
        let mut missing = base.clone();
        missing.as_object_mut().unwrap().remove("relayer");
        assert!(invalid_msg(&consent_from_json(&missing).unwrap_err())
            .contains("consent.relayer: missing or not a string"));

        // Non-string field is treated the same as missing.
        let mut non_str = base.clone();
        non_str["nonce"] = serde_json::json!(7);
        assert!(invalid_msg(&consent_from_json(&non_str).unwrap_err())
            .contains("consent.nonce: missing or not a string"));

        // 32-byte word supplied for the 20-byte relayer slot -> length error.
        let mut wrong_len = base.clone();
        wrong_len["relayer"] = serde_json::json!("0x".to_string() + &"11".repeat(32));
        assert!(invalid_msg(&consent_from_json(&wrong_len).unwrap_err())
            .contains("relayer must be 20 bytes (got 32)"));
    }
}
