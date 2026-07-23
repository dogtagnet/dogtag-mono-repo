//! `ProfileDisclosure` - the ADDITIVE selective-disclosure envelope for profile-tree attribute
//! leaves (D1; f3 §2.3, q4 §1.3).
//!
//! The existing holder flow (`obfuscate`/`redact`) is subtractive: it starts from a full
//! `WrappedDoc` and removes cleartext. The profile tree has no document envelope at all - only `R`
//! ever leaves the device - so disclosure here is the OPPOSITE model: start from nothing and add
//! `(keyPath, salt, tag, value, proof)` tuples for exactly the leaves the owner chose.
//!
//! A disclosure is cryptographically INDEPENDENT of the consent proof; they share only `R`. The
//! consent circuit is leaf-blind and stays frozen - nothing here touches it. Anti-replay comes from
//! presenting the envelope ALONGSIDE a consent proof for the same `R` in the same verify submission,
//! inheriting that proof's relayer/deadline/nullifier binding (q4 §1.3.4); a bare envelope is a
//! replayable bearer credential and must not be accepted on its own.
//!
//! The verifier here is the PURE half (per-entry `verify_inclusion` folding to `R`). The on-chain
//! anchor half - `R == profileRoot(dogTagId)` and `rootIssuer[R]` resolving to a trusted issuer with
//! `isValid(R)` - needs chain reads and lives with the caller (the backend verify handler).

use ark_bn254::Fr;
use serde::{Deserialize, Serialize};

use crate::field::to_hex32;
use crate::leaf::{field_of_keypath, hash_leaf};
use crate::merkle::{merkle_proof, verify_inclusion, ProofStep};
use crate::profile_tree::{
    build_profile_tree, AttributeLeaf, OWNER_IDENTITY_PREFIX, OWNER_NAMESPACE_PREFIX,
};
use crate::types::{DogTagError, TypeTag, TypedScalar};
use crate::wrap::{from_hex32, scalar_from_packed};

/// One revealed leaf: the full opening plus its root-ward `Sibling | Promote` inclusion proof.
///
/// `proof` uses the same wire encoding as `verify_inclusion_proof_hex`: each step is either the
/// literal `"promote"` (a lone-odd-node pass-through) or a `0x..` 32-byte sibling hex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDisclosureEntry {
    pub key_path: String,
    pub salt_hex: String,
    pub tag: u8,
    pub value: String,
    pub proof: Vec<String>,
}

/// The disclosure envelope: `{ dogTagId, R, disclosures: [...] }` (f3 §2.3 / q4 §1.3.1).
///
/// `dog_tag_id` is the CANONICAL dogTagId field (`dog_tag_id_field_hex`) as `0x..` 32-byte hex -
/// the on-chain `profileRoot` key and the consent proof's `pub[0]`, never the raw decimal handle.
/// The pure verifier does not consume it; the backend binds it to the accompanying consent proof
/// and to the on-chain anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileDisclosure {
    #[serde(rename = "dogTagId")]
    pub dog_tag_id: String,
    #[serde(rename = "R")]
    pub root: String,
    pub disclosures: Vec<ProfileDisclosureEntry>,
}

/// Re-pack a [`TypedScalar`] into the `(tag, value)` wire pair [`scalar_from_packed`] parses.
fn packed_value_of(s: &TypedScalar) -> (u8, String) {
    match s {
        TypedScalar::Null => (TypeTag::Null as u8, String::new()),
        TypedScalar::Bool(b) => (TypeTag::Bool as u8, b.to_string()),
        TypedScalar::Str(v) => (TypeTag::String as u8, v.clone()),
        TypedScalar::Integer(v) => (TypeTag::Integer as u8, v.clone()),
        TypedScalar::Decimal(v) => (TypeTag::Decimal as u8, v.clone()),
        TypedScalar::Bytes(v) => (TypeTag::Bytes as u8, hex::encode(v)),
    }
}

/// Reject a keyPath the disclosure surface must never carry: the owner-CONTROL namespace (anything
/// under `owner.` outside `owner.identity.`). Those leaves are reserved-encoded and structurally
/// undisclosable through `verify_inclusion` anyway - `hash_leaf` can never reproduce a
/// `hash_reserved_leaf` image - but refusing them by NAME fails loudly instead of "false".
fn reject_owner_control(key_path: &str) -> Result<(), DogTagError> {
    let normalized = crate::encode::nfc(key_path);
    if normalized.starts_with(OWNER_NAMESPACE_PREFIX)
        && !normalized.starts_with(OWNER_IDENTITY_PREFIX)
    {
        return Err(DogTagError::Other(format!(
            "keyPath {key_path:?} is an owner-control leaf and can never be disclosed"
        )));
    }
    Ok(())
}

/// Build a [`ProfileDisclosure`] for exactly the owner-picked `reveal_key_paths`.
///
/// Rebuilds the tag's tree from the SAME witness issuance folded (`seed`, canonical `dog_tag_id`
/// field, `owner_address`, the full persisted `attributes` list) and emits one entry per revealed
/// leaf: its `(keyPath, salt, tag, value)` opening plus the `Sibling | Promote` path to `R`.
///
/// `reveal_key_paths` must be non-empty, free of duplicates, and every entry must name an existing
/// ATTRIBUTE leaf (matched on the derived keyPath field, so NFC aliases resolve). Owner-control
/// keyPaths are rejected by name.
pub fn build_profile_disclosure(
    seed: &[u8],
    dog_tag_id: Fr,
    owner_address: &[u8; 20],
    attributes: &[AttributeLeaf],
    reveal_key_paths: &[String],
) -> Result<ProfileDisclosure, DogTagError> {
    if reveal_key_paths.is_empty() {
        return Err(DogTagError::Other(
            "reveal_key_paths must not be empty".to_string(),
        ));
    }
    let tree = build_profile_tree(seed, dog_tag_id, owner_address, attributes)?;

    let mut seen: Vec<Fr> = Vec::with_capacity(reveal_key_paths.len());
    let mut disclosures = Vec::with_capacity(reveal_key_paths.len());
    for kp in reveal_key_paths {
        reject_owner_control(kp)?;
        let kp_field = field_of_keypath(kp);
        if seen.contains(&kp_field) {
            return Err(DogTagError::Other(format!(
                "duplicate reveal keyPath {kp:?}"
            )));
        }
        seen.push(kp_field);
        let attribute = attributes
            .iter()
            .find(|a| field_of_keypath(&a.key_path) == kp_field)
            .ok_or_else(|| {
                DogTagError::Other(format!("keyPath {kp:?} is not an attribute of this tree"))
            })?;
        let leaf = hash_leaf(&attribute.key_path, &attribute.salt, &attribute.value)?;
        let proof = merkle_proof(&tree.tree.layers, leaf)
            .iter()
            .map(|step| match step {
                ProofStep::Sibling(h) => to_hex32(h),
                ProofStep::Promote => "promote".to_string(),
            })
            .collect();
        let (tag, value) = packed_value_of(&attribute.value);
        disclosures.push(ProfileDisclosureEntry {
            key_path: attribute.key_path.clone(),
            salt_hex: format!("0x{}", hex::encode(attribute.salt)),
            tag,
            value,
            proof,
        });
    }

    Ok(ProfileDisclosure {
        dog_tag_id: to_hex32(&dog_tag_id),
        root: to_hex32(&tree.root),
        disclosures,
    })
}

/// Parse one wire-encoded proof step list into [`ProofStep`]s.
fn parse_proof_steps(steps: &[String]) -> Result<Vec<ProofStep>, DogTagError> {
    steps
        .iter()
        .map(|s| {
            if s.eq_ignore_ascii_case("promote") {
                Ok(ProofStep::Promote)
            } else {
                Ok(ProofStep::Sibling(from_hex32(s)?))
            }
        })
        .collect()
}

/// Verify the PURE half of a [`ProfileDisclosure`]: every entry's leaf, RECOMPUTED from its
/// `(keyPath, salt, tag, value)` opening under `DS_LEAF` (never trusting a caller-supplied hash),
/// must fold through its proof to the envelope's `R`.
///
/// Returns `Ok(false)` when any entry fails to fold; errors on a malformed envelope (empty
/// disclosure list, bad hex, unknown tag, owner-control keyPath). Callers MUST additionally bind
/// the envelope: `R == profileRoot(dogTagId)` on-chain, `rootIssuer[R]` resolving to a trusted
/// issuer with `isValid(R)`, and - when presented alongside a consent proof - `R == pub[4]` and
/// `dogTagId == pub[0]` of that proof.
pub fn verify_profile_disclosure(d: &ProfileDisclosure) -> Result<bool, DogTagError> {
    if d.disclosures.is_empty() {
        // A vacuous envelope must never read as "verified".
        return Err(DogTagError::Other(
            "disclosures must not be empty".to_string(),
        ));
    }
    let root = from_hex32(&d.root)?;
    for entry in &d.disclosures {
        reject_owner_control(&entry.key_path)?;
        let salt = hex::decode(entry.salt_hex.strip_prefix("0x").unwrap_or(&entry.salt_hex))
            .map_err(|e| DogTagError::Other(format!("bad salt hex: {e}")))?;
        let tag = TypeTag::from_u8(entry.tag)
            .ok_or_else(|| DogTagError::Other(format!("unknown tag {}", entry.tag)))?;
        let scalar = scalar_from_packed(tag, &entry.value)?;
        let steps = parse_proof_steps(&entry.proof)?;
        if !verify_inclusion(&entry.key_path, &salt, &scalar, &steps, root)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile_tree::SALT_LEN;

    const SEED: &[u8] = b"disclosure test wallet seed - TEST MATERIAL ONLY, never hold value";

    fn tag_id() -> Fr {
        Fr::from(424_242u64)
    }

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

    /// The selective-disclosure round-trip: a SUBSET of the identity leaves verifies against the
    /// same `R` issuance sealed, and the un-revealed leaves appear nowhere in the envelope.
    #[test]
    fn a_subset_disclosure_builds_and_verifies() {
        let reveal = vec!["owner.identity.country".to_string()];
        let d = build_profile_disclosure(SEED, tag_id(), &addr(), &attrs(), &reveal).unwrap();

        let tree = build_profile_tree(SEED, tag_id(), &addr(), &attrs()).unwrap();
        assert_eq!(d.root, to_hex32(&tree.root), "envelope must carry the issued R");
        assert_eq!(d.dog_tag_id, to_hex32(&tag_id()));
        assert_eq!(d.disclosures.len(), 1);
        assert_eq!(d.disclosures[0].key_path, "owner.identity.country");
        assert_eq!(d.disclosures[0].value, "GB");

        assert!(verify_profile_disclosure(&d).unwrap());

        // Selective means selective: the other identity values are NOT in the envelope.
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("Alice Owner"), "unrevealed fullName must not leak");
        assert!(!json.contains("PASSPORT-123"), "unrevealed docNumber must not leak");
    }

    /// Multi-leaf disclosure + JSON round-trip (the exact wire the backend deserializes).
    #[test]
    fn a_multi_leaf_disclosure_round_trips_through_json() {
        let reveal = vec![
            "owner.identity.fullName".to_string(),
            "owner.identity.docNumber".to_string(),
        ];
        let d = build_profile_disclosure(SEED, tag_id(), &addr(), &attrs(), &reveal).unwrap();
        let json = serde_json::to_string(&d).unwrap();
        // The wire names are frozen: dogTagId / R / disclosures[{keyPath,saltHex,tag,value,proof}].
        for key in ["\"dogTagId\"", "\"R\"", "\"disclosures\"", "\"keyPath\"", "\"saltHex\""] {
            assert!(json.contains(key), "wire must carry {key}: {json}");
        }
        let back: ProfileDisclosure = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
        assert!(verify_profile_disclosure(&back).unwrap());
    }

    /// A tampered VALUE must fail: the leaf recomputes differently and no longer folds to `R`.
    #[test]
    fn a_tampered_value_fails_verification() {
        let reveal = vec!["owner.identity.country".to_string()];
        let mut d = build_profile_disclosure(SEED, tag_id(), &addr(), &attrs(), &reveal).unwrap();
        d.disclosures[0].value = "US".to_string(); // forge the country
        assert!(!verify_profile_disclosure(&d).unwrap());
    }

    /// A tampered ROOT must fail: the honest opening folds to the true `R`, not the forged one.
    #[test]
    fn a_foreign_root_fails_verification() {
        let reveal = vec!["owner.identity.country".to_string()];
        let mut d = build_profile_disclosure(SEED, tag_id(), &addr(), &attrs(), &reveal).unwrap();
        d.root = to_hex32(&Fr::from(1234u64));
        assert!(!verify_profile_disclosure(&d).unwrap());
    }

    /// Owner-control leaves are undisclosable BY NAME, on both sides of the envelope.
    #[test]
    fn owner_control_key_paths_are_rejected() {
        for kp in ["owner.secret", "owner.address", "owner.consentKey", "owner.delegateKey"] {
            let reveal = vec![kp.to_string()];
            assert!(
                build_profile_disclosure(SEED, tag_id(), &addr(), &attrs(), &reveal).is_err(),
                "builder must reject {kp}"
            );
        }
        // ...and a hand-crafted envelope naming one is an error, not merely false.
        let reveal = vec!["owner.identity.country".to_string()];
        let mut d = build_profile_disclosure(SEED, tag_id(), &addr(), &attrs(), &reveal).unwrap();
        d.disclosures[0].key_path = "owner.secret".to_string();
        assert!(verify_profile_disclosure(&d).is_err());
    }

    /// Malformed requests fail loudly: empty reveal set, unknown keyPath, duplicate keyPath, and
    /// the vacuous empty envelope.
    #[test]
    fn malformed_disclosures_error() {
        assert!(build_profile_disclosure(SEED, tag_id(), &addr(), &attrs(), &[]).is_err());
        assert!(build_profile_disclosure(
            SEED,
            tag_id(),
            &addr(),
            &attrs(),
            &["owner.identity.nope".to_string()]
        )
        .is_err());
        assert!(build_profile_disclosure(
            SEED,
            tag_id(),
            &addr(),
            &attrs(),
            &[
                "owner.identity.country".to_string(),
                "owner.identity.country".to_string()
            ]
        )
        .is_err());

        let reveal = vec!["owner.identity.country".to_string()];
        let mut d = build_profile_disclosure(SEED, tag_id(), &addr(), &attrs(), &reveal).unwrap();
        d.disclosures.clear();
        assert!(
            verify_profile_disclosure(&d).is_err(),
            "an empty envelope must be an error, never a vacuous true"
        );
    }

    /// Pet attributes are disclosable through the same envelope (the surface is leaf-generic even
    /// though D1's product use is identity).
    #[test]
    fn pet_attributes_disclose_through_the_same_envelope() {
        let reveal = vec!["credentialSubject.name".to_string()];
        let d = build_profile_disclosure(SEED, tag_id(), &addr(), &attrs(), &reveal).unwrap();
        assert!(verify_profile_disclosure(&d).unwrap());
        assert_eq!(d.disclosures[0].value, "Rex");
    }
}
