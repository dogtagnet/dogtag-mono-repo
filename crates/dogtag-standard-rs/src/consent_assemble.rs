//! Level-B CONSENT circuit-input ASSEMBLY (M7 P0) — the prover-independent half of the consent
//! proving path, gated behind the lightweight `assemble` feature.
//!
//! This is the consent analogue of [`crate::prover_assemble`]: it assembles the named inputs the
//! **frozen** `circuits/consent.circom` (`DogTagConsent(6)`) needs, from the owner's wallet seed +
//! the disclosed consent parameters, and emits them as decimal-string field elements. It is the SAME
//! assembly the on-device `prover_ffi::prove_consent` runs and the server `/prove-consent` proves
//! against; only decimal strings cross the crate boundary (the ark-0.5 SDK vs the ark-0.6 backend
//! stay disjoint, exactly as the Level-A path).
//!
//! It is built from `consent.circom`, NOT from the stale Level-A `consent.rs` (whose `M`/nullifier
//! carry `subject`+`R` and do NOT match the Level-B circuit — see the ZK cross-check §2). The
//! circuit's seven public OUTPUTS, in frozen declaration order, are
//! `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`; `nullifier` and `R` are
//! circuit OUTPUTS (we never feed them in — the circuit computes them), and `dogTagId <-> R` is bound
//! ON-CHAIN, never in-circuit.
//!
//! # The one canonical `dogTagId` field (ZK cross-check §2 — load-bearing)
//!
//! The off-chain handle (e.g. the vaccination-form id `424242`) is turned into the canonical field
//! `field_of_value(Integer(handle))` **exactly once** here ([`assemble_consent`]), and that identical
//! field element is used for BOTH (a) the circuit's `dogTagId` input and (b) the `build_profile_tree`
//! KDF binding that produces `R`. The on-chain mint (`mintCustodial(id, R)`) MUST use the SAME field
//! as its `id` ([`ConsentAssembledInputs::dog_tag_id_field`]). A mismatch between (a) and the mint id
//! fails closed at `R != profileRoot(dogTagId)`; a mismatch between (a) and (b) yields a valid proof
//! but a non-matching `R` — also fail-closed. We DELIBERATELY do NOT copy the fixture's raw-handle
//! shortcut (`gen-consent-fixture.mjs` / `DeviceRootFixtureWitness`, which bind the raw `424242`).

#[cfg(feature = "prover")]
use std::collections::HashMap;

use ark_bn254::Fr;
use serde_json::Value;

use crate::eddsa::sign_poseidon;
use crate::ffi::FfiError;
use crate::field::{field_from_scalar_bytes, field_from_uint};
use crate::leaf::field_of_value;
use crate::merkle::{merkle_proof, ProofStep};
use crate::poseidon::{poseidon, DS_NULLIFIER};
use crate::profile_tree::{build_profile_tree, AttributeLeaf};
use crate::prover_assemble::fe_to_dec;
use crate::types::TypedScalar;

/// Inclusion-path depth — `component main = DogTagConsent(6)`.
pub const DEPTH: usize = 6;

/// The disclosed + owner-private witness a consent proof is assembled from.
///
/// `dog_tag_id_handle` is the raw off-chain decimal handle; it is field-hashed to the canonical
/// `dogTagId` here (see the module docs). `purpose` / `record_type` / `consent_nonce` / `deadline`
/// are already field elements (a bytes32 label enters the circuit as `keccak256(label) % r`, a
/// `uint256` as itself); `relayer` and `owner_address` are the raw address bytes (packed to a field
/// via `field_from_scalar_bytes`, matching the circuit's uint160 relayer and `build_profile_tree`'s
/// owner-address leaf value).
pub struct ConsentWitness<'a> {
    /// Owner wallet seed — the owner-secret / consent-key / reserved salts derive from it on-device.
    pub seed: &'a [u8],
    /// Off-chain decimal handle (e.g. `"424242"`). Field-hashed to the canonical `dogTagId` here.
    pub dog_tag_id_handle: &'a str,
    /// Owner address (the owner-address reserved leaf's value slot).
    pub owner_address: [u8; 20],
    /// Disclosable credential attribute leaves — MUST match issuance so the rebuilt `R` == the
    /// minted `profileRoot` (`build_profile_tree` rejects an attribute on a reserved keyPath).
    pub attributes: &'a [AttributeLeaf],
    /// Disclosed purpose label as a field (`keccak256(label) % r`).
    pub purpose: Fr,
    /// Disclosed relayer address (range-checked `< 2^160` by the circuit).
    pub relayer: [u8; 20],
    /// Prover-asserted recordType label as a field (NOT consent-signed; `consent.circom` §recordType).
    pub record_type: Fr,
    /// Consent expiry (signed inside `M`; compared on-chain against `block.timestamp`).
    pub deadline: Fr,
    /// Per-consent nonce (the D5 replay scope).
    pub consent_nonce: Fr,
}

/// The assembled `DogTagConsent(6)` inputs (decimal-string field elements), plus the derived values a
/// caller needs to wire the proof on-chain and to assert the canonical-field discipline in tests.
///
/// Every string field is a base-10 field element carrying the circom signal of the same name. The
/// three inclusion paths are front-packed: `*_siblings[0..*_path_len]` are the real siblings (in
/// leaf→root order), the tail is zero-padded to [`DEPTH`], exactly as `consent.circom`'s
/// `MerkleInclusion` consumes them.
pub struct ConsentAssembledInputs {
    pub dog_tag_id: String,
    pub purpose: String,
    pub relayer: String,
    pub record_type: String,
    pub deadline: String,
    pub consent_nonce: String,
    pub owner_address: String,
    pub owner_secret: String,
    pub ax: String,
    pub ay: String,
    pub r8x: String,
    pub r8y: String,
    pub s: String,
    pub owner_salt: String,
    pub key_salt: String,
    pub secret_salt: String,
    pub owner_siblings: [String; DEPTH],
    pub owner_path_len: String,
    pub key_siblings: [String; DEPTH],
    pub key_path_len: String,
    pub secret_siblings: [String; DEPTH],
    pub secret_path_len: String,

    /// `R` — the per-tag Merkle root the proof binds. Equals the SBT's `profileRoot(dogTagId)`, i.e.
    /// what `mintCustodial(dog_tag_id_field, R)` seals. Also emitted as the circuit's `pub[4]`.
    pub root: Fr,
    /// The canonical `dogTagId` field = `field_of_value(Integer(handle))`, computed ONCE. This is the
    /// `dogTagId` circuit input AND the `uint256 id` the SBT must be minted with (ZK cross-check §2).
    pub dog_tag_id_field: Fr,
    /// The consent `nullifier` (`pub[3]`), recomputed here for on-chain wiring + test assertions.
    /// **NOT** fed to the circuit — the circuit derives it from `ownerSecret` proven ∈ `R`.
    pub nullifier: Fr,
}

/// Front-pack a `Sibling | Promote` inclusion proof into the circuit's `siblings[DEPTH]` + `pathLen`.
///
/// `consent.circom`'s `MerkleInclusion` folds `siblings[0..pathLen]` into the leaf and treats the
/// zero-padded tail as no-ops; `Promote` steps (the lone-odd-node pass-throughs) carry no sibling and
/// are dropped, so the front-packed fold reaches the same `R` as the full `Sibling | Promote` fold
/// (`process_proof` treats `Promote` as identity). Mirrors `gen-consent-fixture.mjs::padPath`.
fn front_pack(proof: &[ProofStep]) -> Result<([String; DEPTH], String), FfiError> {
    let siblings: Vec<String> = proof
        .iter()
        .filter_map(|step| match step {
            ProofStep::Sibling(h) => Some(fe_to_dec(h)),
            ProofStep::Promote => None,
        })
        .collect();
    if siblings.len() > DEPTH {
        return Err(FfiError::Invalid(format!(
            "inclusion path too deep: {} siblings > DEPTH={DEPTH}",
            siblings.len()
        )));
    }
    let path_len = siblings.len().to_string();
    let mut packed: [String; DEPTH] = Default::default();
    for (slot, sib) in packed.iter_mut().zip(siblings) {
        *slot = sib;
    }
    // Zero-pad the tail (Default::default() gives empty strings).
    for slot in packed.iter_mut() {
        if slot.is_empty() {
            *slot = "0".to_string();
        }
    }
    Ok((packed, path_len))
}

/// Assemble the `DogTagConsent(6)` inputs from the owner seed + disclosed consent parameters.
///
/// Builds the per-tag profile tree on-device (owner-secret + consent key + reserved salts derived
/// from `seed`, bound to the canonical `dogTagId`), EdDSA-signs `M = Poseidon5(dogTagId, purpose,
/// relayer, deadline, consentNonce)` with that tag's OWN consent key, and computes the three
/// reserved-leaf inclusion paths. The canonical `dogTagId` field is computed ONCE and used for both
/// the circuit input and the tree KDF (see the module docs).
pub fn assemble_consent(w: &ConsentWitness) -> Result<ConsentAssembledInputs, FfiError> {
    // (1) THE one canonical dogTagId field — computed once, used everywhere.
    let dog_tag_id_field = field_of_value(&TypedScalar::Integer(w.dog_tag_id_handle.to_string()))
        .map_err(FfiError::from)?;

    // (2) Rebuild the identical per-tag tree the issuer minted (same canonical id -> same R).
    let tree = build_profile_tree(w.seed, dog_tag_id_field, &w.owner_address, w.attributes)
        .map_err(FfiError::from)?;

    // Address / salt value slots as raw fields (matching `hash_reserved_leaf` / the circuit signals).
    let relayer_field = field_from_scalar_bytes(&w.relayer);
    let owner_address_field = field_from_scalar_bytes(&w.owner_address);

    // (3) EdDSA-BabyJubjub over M = Poseidon5(dogTagId, purpose, relayer, deadline, consentNonce).
    let m = poseidon(&[
        dog_tag_id_field,
        w.purpose,
        relayer_field,
        w.deadline,
        w.consent_nonce,
    ]);
    let sig = sign_poseidon(&tree.consent_prv, &m);

    // (4) The three reserved-leaf inclusion paths, front-packed for the circuit.
    let (owner_siblings, owner_path_len) =
        front_pack(&merkle_proof(&tree.tree.layers, tree.owner_address_leaf))?;
    let (key_siblings, key_path_len) =
        front_pack(&merkle_proof(&tree.tree.layers, tree.consent_key_leaf))?;
    let (secret_siblings, secret_path_len) =
        front_pack(&merkle_proof(&tree.tree.layers, tree.owner_secret_leaf))?;

    // (5) Nullifier = Poseidon6(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce).
    //     Recomputed for on-chain wiring + tests; the circuit derives its own from ownerSecret ∈ R.
    let nullifier = poseidon(&[
        field_from_uint(DS_NULLIFIER),
        tree.owner_secret,
        dog_tag_id_field,
        w.purpose,
        relayer_field,
        w.consent_nonce,
    ]);

    Ok(ConsentAssembledInputs {
        dog_tag_id: fe_to_dec(&dog_tag_id_field),
        purpose: fe_to_dec(&w.purpose),
        relayer: fe_to_dec(&relayer_field),
        record_type: fe_to_dec(&w.record_type),
        deadline: fe_to_dec(&w.deadline),
        consent_nonce: fe_to_dec(&w.consent_nonce),
        owner_address: fe_to_dec(&owner_address_field),
        owner_secret: fe_to_dec(&tree.owner_secret),
        ax: fe_to_dec(&tree.ax),
        ay: fe_to_dec(&tree.ay),
        r8x: fe_to_dec(&sig.r8x),
        r8y: fe_to_dec(&sig.r8y),
        s: sig.s.to_str_radix(10),
        owner_salt: fe_to_dec(&field_from_scalar_bytes(&tree.owner_salt)),
        key_salt: fe_to_dec(&field_from_scalar_bytes(&tree.key_salt)),
        secret_salt: fe_to_dec(&field_from_scalar_bytes(&tree.secret_salt)),
        owner_siblings,
        owner_path_len,
        key_siblings,
        key_path_len,
        secret_siblings,
        secret_path_len,
        root: tree.root,
        dog_tag_id_field,
        nullifier,
    })
}

/// Serialize the assembled inputs into the circuit-input JSON the ark-0.6 backend
/// (`dogtag_prover::ConsentProveInputs::from_circuit_input_json`) consumes: scalars are bare decimal
/// STRINGS, the three sibling signals are decimal-string ARRAYS of length [`DEPTH`]. This is the seam
/// the server `/prove-consent` proves against and the on-device path cross-checks.
pub fn consent_circuit_input_value(inp: &ConsentAssembledInputs) -> Value {
    let arr = |a: &[String; DEPTH]| Value::Array(a.iter().map(|s| Value::String(s.clone())).collect());
    serde_json::json!({
        "dogTagId": inp.dog_tag_id,
        "purpose": inp.purpose,
        "relayer": inp.relayer,
        "recordType": inp.record_type,
        "deadline": inp.deadline,
        "consentNonce": inp.consent_nonce,
        "ownerAddress": inp.owner_address,
        "ownerSecret": inp.owner_secret,
        "Ax": inp.ax,
        "Ay": inp.ay,
        "R8x": inp.r8x,
        "R8y": inp.r8y,
        "S": inp.s,
        "ownerSalt": inp.owner_salt,
        "keySalt": inp.key_salt,
        "secretSalt": inp.secret_salt,
        "ownerSiblings": arr(&inp.owner_siblings),
        "ownerPathLen": inp.owner_path_len,
        "keySiblings": arr(&inp.key_siblings),
        "keyPathLen": inp.key_path_len,
        "secretSiblings": arr(&inp.secret_siblings),
        "secretPathLen": inp.secret_path_len,
    })
}

/// Build the circom-prover input map (signal name -> decimal-string values) for the on-device graph
/// witness calculator. ONLY the full `prover` feature (`prover_ffi::prove_consent`) consumes this;
/// the server path uses [`consent_circuit_input_value`] instead, so it is gated to `prover` to avoid
/// a dead-code warning in the `assemble`-only (backend) build. Mirrors `prover_assemble::input_map`.
#[cfg(feature = "prover")]
pub(crate) fn consent_input_map(inp: &ConsentAssembledInputs) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    m.insert("dogTagId".into(), vec![inp.dog_tag_id.clone()]);
    m.insert("purpose".into(), vec![inp.purpose.clone()]);
    m.insert("relayer".into(), vec![inp.relayer.clone()]);
    m.insert("recordType".into(), vec![inp.record_type.clone()]);
    m.insert("deadline".into(), vec![inp.deadline.clone()]);
    m.insert("consentNonce".into(), vec![inp.consent_nonce.clone()]);
    m.insert("ownerAddress".into(), vec![inp.owner_address.clone()]);
    m.insert("ownerSecret".into(), vec![inp.owner_secret.clone()]);
    m.insert("Ax".into(), vec![inp.ax.clone()]);
    m.insert("Ay".into(), vec![inp.ay.clone()]);
    m.insert("R8x".into(), vec![inp.r8x.clone()]);
    m.insert("R8y".into(), vec![inp.r8y.clone()]);
    m.insert("S".into(), vec![inp.s.clone()]);
    m.insert("ownerSalt".into(), vec![inp.owner_salt.clone()]);
    m.insert("keySalt".into(), vec![inp.key_salt.clone()]);
    m.insert("secretSalt".into(), vec![inp.secret_salt.clone()]);
    m.insert("ownerSiblings".into(), inp.owner_siblings.to_vec());
    m.insert("ownerPathLen".into(), vec![inp.owner_path_len.clone()]);
    m.insert("keySiblings".into(), inp.key_siblings.to_vec());
    m.insert("keyPathLen".into(), vec![inp.key_path_len.clone()]);
    m.insert("secretSiblings".into(), inp.secret_siblings.to_vec());
    m.insert("secretPathLen".into(), vec![inp.secret_path_len.clone()]);
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merkle::{hash_node, process_proof};
    use crate::profile_tree::SALT_LEN;
    use ark_ff::PrimeField;

    /// Decimal string -> `Fr` via the crate's canonical big-endian idiom (avoids relying on a
    /// `From<BigUint>` impl).
    fn fr_from_dec(dec: &str) -> Fr {
        let b = num_bigint::BigUint::parse_bytes(dec.as_bytes(), 10).unwrap();
        Fr::from_be_bytes_mod_order(&b.to_bytes_be())
    }

    const SEED: &[u8] = b"consent-assemble test wallet seed - TEST MATERIAL ONLY, never hold value";
    const HANDLE: &str = "424242";

    fn addr() -> [u8; 20] {
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

    fn witness<'a>(handle: &'a str, attributes: &'a [AttributeLeaf]) -> ConsentWitness<'a> {
        ConsentWitness {
            seed: SEED,
            dog_tag_id_handle: handle,
            owner_address: addr(),
            attributes,
            purpose: Fr::from(7u64),
            relayer: [0x11u8; 20],
            record_type: Fr::from(19u64),
            deadline: Fr::from(1_893_456_000u64),
            consent_nonce: Fr::from(99u64),
        }
    }

    /// THE canonical-field round-trip (ZK cross-check §2, acceptance): the handle is field-hashed
    /// ONCE and the SAME field is the circuit `dogTagId` input, the on-chain mint id, and the KDF
    /// binding that produces `R` — so `R == profileRoot(dogTagId)` and `dogTagId(input) == mint id`.
    /// The per-tag consent key must survive all the way into the ASSEMBLED PROVER INPUTS, not just
    /// into the tree: `Ax`/`Ay` are what the circuit actually consumes. One seed, two tags → two
    /// different `(Ax, Ay)` and therefore two different `owner.consentKey` leaves and two different
    /// `R`. This is the end of the chain the per-tag binding exists to protect.
    #[test]
    fn two_tags_of_one_wallet_assemble_different_consent_keys() {
        let a = attrs();
        let one = assemble_consent(&witness("1000000000000000001", &a)).unwrap();
        let two = assemble_consent(&witness("1000000000000000002", &a)).unwrap();

        assert_ne!(one.dog_tag_id, two.dog_tag_id, "precondition: distinct tags");
        assert_ne!(one.ax, two.ax, "assembled Ax must differ per tag");
        assert_ne!(one.ay, two.ay, "assembled Ay must differ per tag");
        assert_ne!(one.root, two.root, "R must differ per tag");
    }

    #[test]
    fn canonical_field_is_used_across_circuit_input_kdf_and_mint_id() {
        let a = attrs();
        let dog_tag_id_field =
            field_of_value(&TypedScalar::Integer(HANDLE.to_string())).unwrap();
        // (b) the KDF binding: a tree built with the canonical field.
        let tree = build_profile_tree(SEED, dog_tag_id_field, &addr(), &a).unwrap();

        let inp = assemble_consent(&witness(HANDLE, &a)).unwrap();

        // (a) circuit `dogTagId` input == the canonical field.
        assert_eq!(inp.dog_tag_id, fe_to_dec(&dog_tag_id_field));
        // (c) on-chain mint id == the SAME canonical field.
        assert_eq!(inp.dog_tag_id_field, dog_tag_id_field);
        // R the proof binds == the tree root the SBT seals as profileRoot(dogTagId).
        assert_eq!(inp.root, tree.root, "assembled R must equal the KDF-bound profileRoot");
    }

    /// The fixture's raw-handle shortcut is fail-closed: a tree KDF-bound to the RAW handle `424242`
    /// yields a DIFFERENT `R` than the canonical-field tree the assembler builds, so a proof carrying
    /// the raw-bound `R` reverts at `R != profileRoot(dogTagId)`. The assembler can only ever produce
    /// the canonical-bound `R`.
    #[test]
    fn raw_handle_shortcut_breaks_the_r_binding_fail_closed() {
        let a = attrs();
        let canonical = field_of_value(&TypedScalar::Integer(HANDLE.to_string())).unwrap();
        let raw = Fr::from(424_242u64);
        assert_ne!(canonical, raw, "the raw handle and its field-hash must differ");

        let tree_raw = build_profile_tree(SEED, raw, &addr(), &a).unwrap();
        let tree_canonical = build_profile_tree(SEED, canonical, &addr(), &a).unwrap();
        assert_ne!(
            tree_raw.root, tree_canonical.root,
            "the KDF binds dogTagId, so raw vs canonical must diverge R"
        );

        let inp = assemble_consent(&witness(HANDLE, &a)).unwrap();
        assert_eq!(inp.root, tree_canonical.root, "assembler binds the canonical field");
        assert_ne!(inp.root, tree_raw.root, "assembler can never produce the raw-bound R");
    }

    /// The front-packed inclusion paths fold to `R` exactly as `consent.circom`'s `MerkleInclusion`
    /// does (fold `hash_node` over `siblings[0..pathLen]`), for all three reserved leaves.
    #[test]
    fn front_packed_paths_fold_to_root_like_the_circuit() {
        let a = attrs();
        let inp = assemble_consent(&witness(HANDLE, &a)).unwrap();
        let tree = build_profile_tree(SEED, inp.dog_tag_id_field, &addr(), &a).unwrap();

        let fold = |leaf: Fr, siblings: &[String; DEPTH], path_len: &str| -> Fr {
            let n: usize = path_len.parse().unwrap();
            let mut h = leaf;
            for sib in siblings.iter().take(n) {
                h = hash_node(h, fr_from_dec(sib));
            }
            h
        };

        assert_eq!(
            fold(tree.owner_address_leaf, &inp.owner_siblings, &inp.owner_path_len),
            inp.root
        );
        assert_eq!(
            fold(tree.consent_key_leaf, &inp.key_siblings, &inp.key_path_len),
            inp.root
        );
        assert_eq!(
            fold(tree.owner_secret_leaf, &inp.secret_siblings, &inp.secret_path_len),
            inp.root
        );

        // Sanity: the SDK's own full-proof fold agrees (Promote steps are identity).
        assert_eq!(
            process_proof(&merkle_proof(&tree.tree.layers, tree.owner_secret_leaf), tree.owner_secret_leaf),
            inp.root
        );
    }

    /// The circuit-input JSON carries every `consent.circom` signal name, scalars as strings and the
    /// three sibling signals as length-`DEPTH` arrays — the exact shape the server backend parses.
    #[test]
    fn circuit_input_json_has_every_signal_in_circuit_shape() {
        let a = attrs();
        let inp = assemble_consent(&witness(HANDLE, &a)).unwrap();
        let v = consent_circuit_input_value(&inp);
        for k in [
            "dogTagId", "purpose", "relayer", "recordType", "deadline", "consentNonce",
            "ownerAddress", "ownerSecret", "Ax", "Ay", "R8x", "R8y", "S", "ownerSalt",
            "keySalt", "secretSalt", "ownerPathLen", "keyPathLen", "secretPathLen",
        ] {
            assert!(v[k].is_string(), "{k} must be a decimal string");
        }
        for k in ["ownerSiblings", "keySiblings", "secretSiblings"] {
            assert_eq!(v[k].as_array().unwrap().len(), DEPTH, "{k} must be DEPTH-wide");
        }
    }
}
