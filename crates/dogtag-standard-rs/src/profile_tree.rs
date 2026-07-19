//! Level-B per-tag profile tree - the **device-side** build (level-b-spec §"The per-tag Merkle
//! tree", §"Issuance (M5, custodial)").
//!
//! The owner's holder app builds this tree locally and hands the issuer only the root `R`, which the
//! issuer seals into `DogTagSBTConsent.profileRoot(dogTagId)` (write-once). The owner's wallet never
//! reaches the chain - not as `to`, not as `msg.sender`.
//!
//! **Why the owner-secret must be device-derived:** it is the nullifier's secret leaf. A
//! server-generated secret would let the server link nullifiers back to the owner, collapsing the
//! owner-unlinkable model. It is therefore derived here, on the device, from the wallet seed and
//! never transmitted.
//!
//! # Recoverability (captain's requirement)
//!
//! Every owner-control input is a pure function of `(wallet seed, dogTagId)`:
//!
//! | input          | derivation                                                    |
//! |----------------|---------------------------------------------------------------|
//! | owner-secret   | [`derive_owner_secret`] - BLAKE-512 KDF, wide-reduced mod r    |
//! | consent-key    | [`crate::eddsa::derive_babyjub_consent_key_per_tag`] - BLAKE-512 KDF -> `prv2pub` |
//! | reserved salts | [`derive_reserved_salt`] - BLAKE-512 KDF, per keyPath          |
//!
//! All three share the single `crate::kdf::kdf` preimage builder, so the `(seed, dogTagId)`
//! binding is uniform across the owner-control core rather than per-derivation folklore.
//!
//! Restoring the seed regenerates the identical owner-control core. Rebuilding the identical `R`
//! additionally requires the original owner address plus every attribute value and salt; those are
//! NOT seed-derivable and must be retained or recovered from the credential
//! (`docs/MOBILE_OWNER_SECRET.md`).
//!
//! Deriving from the seed does not weaken unlinkability: the seed never leaves the device, and
//! BLAKE-512 is one-way, so the secret stays as opaque to an observer as a random one - while a
//! random secret would be unrecoverable once the device is lost.

use ark_bn254::Fr;
use ark_ff::PrimeField;

use crate::field::{field_from_scalar_bytes, field_from_uint};
use crate::kdf::kdf;
use crate::leaf::{field_of_keypath, hash_leaf};
use crate::merkle::{build_merkle, MerkleTree};
use crate::poseidon::{poseidon, DS_LEAF};
use crate::types::{DogTagError, TypeTag, TypedScalar};

/// Reserved owner-control leaf keyPaths. The circuit PINS `fieldOfKeyPath` of each of these
/// (`circuits/consent.circom:50-52`); [`RESERVED_KEY_PATH_FIELDS`] is the drift guard.
pub const KP_OWNER_ADDRESS: &str = "owner.address";
pub const KP_CONSENT_KEY: &str = "owner.consentKey";
pub const KP_OWNER_SECRET: &str = "owner.secret";

/// typeTag pinned for all three reserved leaves (`consent.circom:54-56`).
pub const RESERVED_TYPE_TAG: TypeTag = TypeTag::Bytes;

/// The keyPath field constants pinned in `circuits/consent.circom:50-52`, as decimal strings.
///
/// These are NOT the source of truth - [`field_of_keypath`] is. They are duplicated here so a test
/// can assert the SDK still derives exactly what the circuit hard-codes: if either side drifts, the
/// device would build leaves the circuit cannot prove inclusion for. Mirrors the same guard in
/// `circuits/scripts/test-consent.mjs`.
pub const RESERVED_KEY_PATH_FIELDS: [(&str, &str); 3] = [
    (
        KP_OWNER_ADDRESS,
        "20593649144631820416234157596070441856608371338897391424937040814759273231214",
    ),
    (
        KP_CONSENT_KEY,
        "7822071287675030884271946396254564996644565056920260282559292033992393086992",
    ),
    (
        KP_OWNER_SECRET,
        "11172449362271989869407103131203633198993612309996015027844083581837121079156",
    ),
];

/// Domain for the owner-secret KDF. Versioned: bumping `v1` re-derives every secret (and thus every
/// `R`), so it must never change without a migration - `profileRoot` is write-once on-chain.
const OWNER_SECRET_DOMAIN: &[u8] = b"DogTag/owner-secret/v1";

/// Domain for the reserved-leaf salt KDF. Distinct from [`OWNER_SECRET_DOMAIN`] so a salt can never
/// collide with the secret itself.
const RESERVED_SALT_DOMAIN: &[u8] = b"DogTag/reserved-leaf-salt/v1";

/// Salts are 16 bytes (`hash_leaf` enforces it; the spec calls for a fresh 16-byte salt).
pub const SALT_LEN: usize = 16;

/// Hash a **reserved** leaf: `Poseidon5(DS_LEAF, fieldOf(keyPath), fieldFromScalarBytes(salt),
/// typeTag, value)` with `value` written **raw** into the value slot.
///
/// This is the one place the reserved leaves diverge from disclosable attribute leaves, and it is
/// the sharpest M5 edge (AGENTS.md §"Reserved-leaf value encoding"): [`hash_leaf`] always runs the
/// value through `field_of_value` (the length-prefixed byte-fold), but the circuit feeds
/// `ownerAddress` / `keyHash` / `ownerSecret` in as bare field signals. Routing a reserved value
/// through `field_of_value` would build a leaf the circuit can never match.
pub fn hash_reserved_leaf(key_path: &str, salt: &[u8], value: Fr) -> Result<Fr, DogTagError> {
    if salt.len() != SALT_LEN {
        return Err(DogTagError::Other(format!(
            "salt must be {SALT_LEN} bytes (got {})",
            salt.len()
        )));
    }
    Ok(poseidon(&[
        Fr::from(DS_LEAF),
        field_of_keypath(key_path),
        field_from_scalar_bytes(salt),
        field_from_uint(RESERVED_TYPE_TAG as u64),
        value,
    ]))
}

fn check_seed(seed: &[u8]) -> Result<(), DogTagError> {
    if seed.is_empty() {
        return Err(DogTagError::Other("seed must be non-empty".to_string()));
    }
    Ok(())
}

/// Derive the owner-secret deterministically from the wallet seed, bound to `dog_tag_id`.
///
/// `Fr::from_be_bytes_mod_order` over the **full 64-byte** BLAKE-512 digest is a wide reduction: the
/// modular bias is ~2^-125, negligible. Reducing a bare 32-byte hash would be measurably biased.
///
/// Binding to `dog_tag_id` means one seed yields an independent secret per tag, so two tags owned by
/// the same wallet produce unlinkable nullifiers.
pub fn derive_owner_secret(seed: &[u8], dog_tag_id: Fr) -> Result<Fr, DogTagError> {
    check_seed(seed)?;
    Ok(Fr::from_be_bytes_mod_order(&kdf(
        OWNER_SECRET_DOMAIN,
        &dog_tag_id,
        &[],
        seed,
    )))
}

/// Derive a reserved leaf's 16-byte salt from the wallet seed, bound to `(dog_tag_id, key_path)`.
///
/// Salts only need to be unguessable to an attacker (they are the rainbow-resistance of the leaf
/// commitment); a KDF from a secret seed provides that just as a CSPRNG would, while additionally
/// being reproducible after a restore - which a random salt is not.
pub fn derive_reserved_salt(
    seed: &[u8],
    dog_tag_id: Fr,
    key_path: &str,
) -> Result<[u8; SALT_LEN], DogTagError> {
    check_seed(seed)?;
    let digest = kdf(RESERVED_SALT_DOMAIN, &dog_tag_id, key_path.as_bytes(), seed);
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&digest[..SALT_LEN]);
    Ok(salt)
}

/// A disclosable attribute leaf (pet name, breed, DOB, ...).
///
/// Unlike the reserved leaves, the value goes through `field_of_value`, and the salt is
/// caller-supplied: attribute values originate with the issuer, not the seed, so they cannot be
/// re-derived and must be retained to rebuild the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeLeaf {
    pub key_path: String,
    pub salt: [u8; SALT_LEN],
    pub value: TypedScalar,
}

/// A device-built per-tag profile tree.
///
/// Carries the full witness the M2 `DogTagConsent` circuit needs: `root` is what the issuer seals as
/// `profileRoot`, and `tree.layers` feeds [`crate::merkle::merkle_proof`] for the three reserved
/// leaves' inclusion paths.
pub struct ProfileTree {
    /// `R` - the per-tag Merkle root; equals the contract's `profileRoot(dogTagId)`.
    pub root: Fr,
    pub tree: MerkleTree,
    /// The nullifier secret (circuit input `ownerSecret`). Never leaves the device.
    pub owner_secret: Fr,
    /// BabyJubJub consent pubkey, seed-derived (circuit inputs `Ax` / `Ay`).
    pub ax: Fr,
    pub ay: Fr,
    /// 32-byte BabyJubJub consent private key - signs the consent message `M`.
    pub consent_prv: [u8; 32],
    pub owner_address_leaf: Fr,
    pub consent_key_leaf: Fr,
    pub owner_secret_leaf: Fr,
    /// Seed-derived reserved-leaf salts (circuit inputs `ownerSalt` / `keySalt` / `secretSalt`).
    pub owner_salt: [u8; SALT_LEN],
    pub key_salt: [u8; SALT_LEN],
    pub secret_salt: [u8; SALT_LEN],
}

/// Build the per-tag tree on-device from the wallet seed.
///
/// Deterministic in `(seed, dog_tag_id, owner_address, attributes)`: same inputs → same `R`. That is
/// what makes the tag recoverable - see the module docs.
///
/// `dog_tag_id` is the **canonical dogTagId field** (`field_of_value(Integer(dec))`, what
/// `dog_tag_id_field_hex` returns), not the raw decimal id.
///
/// An attribute whose keyPath resolves to one of the reserved owner-control keyPaths
/// ([`RESERVED_KEY_PATH_FIELDS`]) is **rejected**: `TypeTag::Bytes` IS [`RESERVED_TYPE_TAG`], so such
/// an attribute would build a second leaf the circuit accepts as a reserved leaf, defeating the
/// keyPath pinning that makes the real one unique (`consent.circom` header; D5).
pub fn build_profile_tree(
    seed: &[u8],
    dog_tag_id: Fr,
    owner_address: &[u8; 20],
    attributes: &[AttributeLeaf],
) -> Result<ProfileTree, DogTagError> {
    check_seed(seed)?;

    let owner_secret = derive_owner_secret(seed, dog_tag_id)?;
    let consent = crate::eddsa::derive_babyjub_consent_key_per_tag(seed, dog_tag_id);

    let owner_salt = derive_reserved_salt(seed, dog_tag_id, KP_OWNER_ADDRESS)?;
    let key_salt = derive_reserved_salt(seed, dog_tag_id, KP_CONSENT_KEY)?;
    let secret_salt = derive_reserved_salt(seed, dog_tag_id, KP_OWNER_SECRET)?;

    // Reserved value slots - all three are RAW fields (see `hash_reserved_leaf`).
    let owner_address_field = field_from_scalar_bytes(owner_address);
    let key_hash = poseidon(&[consent.ax, consent.ay]);

    let owner_address_leaf = hash_reserved_leaf(KP_OWNER_ADDRESS, &owner_salt, owner_address_field)?;
    let consent_key_leaf = hash_reserved_leaf(KP_CONSENT_KEY, &key_salt, key_hash)?;
    let owner_secret_leaf = hash_reserved_leaf(KP_OWNER_SECRET, &secret_salt, owner_secret)?;

    let mut leaves = vec![owner_address_leaf, consent_key_leaf, owner_secret_leaf];
    for a in attributes {
        // Compared on the DERIVED field, not the raw string: the leaf commits to
        // `field_of_keypath(kp)`, and that NFC-normalizes, so two distinct strings reaching the same
        // field build the same leaf and a raw string compare would miss them.
        let key_path_field = field_of_keypath(&a.key_path);
        if RESERVED_KEY_PATH_FIELDS
            .iter()
            .any(|(reserved, _)| field_of_keypath(reserved) == key_path_field)
        {
            return Err(DogTagError::Other(format!(
                "attribute keyPath {:?} is reserved for the owner-control leaves",
                a.key_path
            )));
        }
        leaves.push(hash_leaf(&a.key_path, &a.salt, &a.value)?);
    }

    // `build_merkle` sorts leaf hashes ascending before folding, so insertion order does not affect
    // `R`; the reserved leaves are pushed first only for readability.
    let tree = build_merkle(&leaves);
    Ok(ProfileTree {
        root: tree.root,
        tree,
        owner_secret,
        ax: consent.ax,
        ay: consent.ay,
        consent_prv: consent.prv,
        owner_address_leaf,
        consent_key_leaf,
        owner_secret_leaf,
        owner_salt,
        key_salt,
        secret_salt,
    })
}

/// The fixed demo witness behind `contracts/test/device-profile-root.json`.
///
/// Shared by the `gen-device-profile-root` generator and the drift guard in
/// `tests/profile_tree_parity.rs` so the two can never disagree about what was generated.
///
/// **Test material only** - this seed is committed in the clear and must never hold value.
pub struct DeviceRootFixtureWitness {
    pub seed: Vec<u8>,
    /// The raw demo handle (424242) used DIRECTLY as a field element - **not** the canonical dogTagId
    /// field (`field_of_value(Integer(dec))`, what `dog_tag_id_field_hex` returns). Mirrors
    /// `gen-consent-fixture.mjs`, which also binds the raw handle. Production must pass the canonical
    /// field to `build_profile_tree` instead; do not copy this fixture's shortcut into the M7 issuance
    /// path.
    pub dog_tag_id: Fr,
    /// The same raw handle in decimal - both the KDF binding above and the uint256 the contract is
    /// minted with.
    pub dog_tag_id_dec: String,
    pub owner_address: [u8; 20],
    pub attributes: Vec<AttributeLeaf>,
}

/// See [`DeviceRootFixtureWitness`]. A plain function (not a test fixture) so the generator binary
/// and the drift-guard test share one definition.
pub fn device_root_fixture_witness() -> DeviceRootFixtureWitness {
    let mut owner_address = [0u8; 20];
    owner_address[16..].copy_from_slice(&0xdead_beefu32.to_be_bytes());

    DeviceRootFixtureWitness {
        seed: b"DogTag demo wallet seed - TEST MATERIAL ONLY, never hold value".to_vec(),
        dog_tag_id: Fr::from(424242u64),
        dog_tag_id_dec: "424242".to_string(),
        owner_address,
        attributes: vec![
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
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merkle::{merkle_proof, process_proof};
    use crate::poseidon::to_be_bytes32;
    use crate::util::be_bytes_to_dec;

    const SEED: &[u8] = b"test wallet seed material - 64 bytes of BIP39 seed would go here";
    fn tag_id() -> Fr {
        Fr::from(424242u64)
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
                key_path: "credentialSubject.breedLabel".to_string(),
                salt: [9u8; SALT_LEN],
                value: TypedScalar::Str("Shiba Inu".to_string()),
            },
        ]
    }

    /// Drift guard: the SDK must derive exactly the keyPath fields `consent.circom` hard-codes.
    /// Mirrors `circuits/scripts/test-consent.mjs`. If this fails, the device builds leaves the
    /// circuit cannot prove.
    #[test]
    fn reserved_key_path_fields_match_the_pinned_circuit_constants() {
        for (kp, want) in RESERVED_KEY_PATH_FIELDS {
            let got = be_bytes_to_dec(&to_be_bytes32(&field_of_keypath(kp)));
            assert_eq!(got, want, "fieldOfKeyPath({kp}) drifted from consent.circom");
        }
    }

    /// THE recoverability round-trip (captain's requirement): regenerate the owner-secret from the
    /// seed, rebuild the tree, and assert the SAME `R`.
    #[test]
    fn owner_secret_and_root_regenerate_from_the_seed() {
        let first = build_profile_tree(SEED, tag_id(), &addr(), &attrs()).unwrap();

        // Simulate device loss: nothing survives but the seed, the id, the address and the
        // attributes (the issuer's data, held in the local backup file).
        let recovered = build_profile_tree(SEED, tag_id(), &addr(), &attrs()).unwrap();

        assert_eq!(
            recovered.owner_secret, first.owner_secret,
            "owner-secret must regenerate from the seed"
        );
        assert_eq!(recovered.root, first.root, "same seed must rebuild the same R");
        // The whole owner-control core, not just the secret.
        assert_eq!(recovered.ax, first.ax);
        assert_eq!(recovered.ay, first.ay);
        assert_eq!(recovered.owner_salt, first.owner_salt);
        assert_eq!(recovered.key_salt, first.key_salt);
        assert_eq!(recovered.secret_salt, first.secret_salt);

        // And the standalone KDF agrees with what the builder embedded.
        assert_eq!(derive_owner_secret(SEED, tag_id()).unwrap(), first.owner_secret);
    }

    #[test]
    fn owner_secret_kdf_matches_known_answer() {
        let secret = derive_owner_secret(SEED, tag_id()).unwrap();
        assert_eq!(
            crate::field::to_hex32(&secret),
            "0x2b9e3bbc643ef7b5e78776470743d23745ca259e82458595862dcaafa16e5371"
        );
    }

    #[test]
    fn a_different_seed_yields_a_different_secret_and_root() {
        let a = build_profile_tree(SEED, tag_id(), &addr(), &attrs()).unwrap();
        let b = build_profile_tree(b"a completely different wallet seed", tag_id(), &addr(), &attrs())
            .unwrap();
        assert_ne!(a.owner_secret, b.owner_secret);
        assert_ne!(a.root, b.root);
    }

    /// Per-tag binding: one wallet, two tags → independent secrets (hence unlinkable nullifiers).
    #[test]
    fn the_same_seed_yields_independent_secrets_per_tag() {
        let a = derive_owner_secret(SEED, Fr::from(1u64)).unwrap();
        let b = derive_owner_secret(SEED, Fr::from(2u64)).unwrap();
        assert_ne!(a, b, "owner-secret must be bound to dogTagId");
    }

    /// The consent key's sibling of the test above: one wallet, two tags → independent BabyJubjub
    /// keys, so the raw `(Ax, Ay)` can never cross-link a wallet's tags if it is ever exposed
    /// off-chain. This is the invariant that was MISSING while the key was derived from the seed
    /// alone; if someone re-points `build_profile_tree` at the wallet-level derivation, this fires.
    #[test]
    fn the_same_seed_yields_independent_consent_keys_per_tag() {
        let a = crate::eddsa::derive_babyjub_consent_key_per_tag(SEED, Fr::from(1u64));
        let b = crate::eddsa::derive_babyjub_consent_key_per_tag(SEED, Fr::from(2u64));
        assert_ne!(a.prv, b.prv, "consent private key must be bound to dogTagId");
        assert_ne!(a.ax, b.ax, "consent Ax must be bound to dogTagId");
        assert_ne!(a.ay, b.ay, "consent Ay must be bound to dogTagId");

        // ...and the binding must reach all the way to the committed leaf, not just the key.
        let ta = build_profile_tree(SEED, Fr::from(1u64), &addr(), &attrs()).unwrap();
        let tb = build_profile_tree(SEED, Fr::from(2u64), &addr(), &attrs()).unwrap();
        assert_ne!(
            ta.consent_key_leaf, tb.consent_key_leaf,
            "owner.consentKey leaf must differ per tag"
        );
        assert_ne!(ta.root, tb.root, "R must differ per tag");
    }

    /// Drift guard: the tree MUST use the per-tag derivation. A regression to the wallet-level
    /// key would still produce a valid-looking tree, so assert the leaf against the per-tag key
    /// explicitly and assert it does NOT match the wallet-level one.
    #[test]
    fn the_consent_key_leaf_commits_the_per_tag_key_not_the_wallet_key() {
        let id = tag_id();
        let tree = build_profile_tree(SEED, id, &addr(), &attrs()).unwrap();

        let per_tag = crate::eddsa::derive_babyjub_consent_key_per_tag(SEED, id);
        assert_eq!(tree.consent_prv, per_tag.prv, "tree must use the per-tag consent key");

        let wallet_level = crate::eddsa::derive_babyjub_consent_key_from_seed(SEED);
        assert_ne!(
            tree.consent_prv, wallet_level.prv,
            "tree must NOT use the seed-only wallet-level consent key"
        );
    }

    #[test]
    fn reserved_salts_are_distinct_per_leaf_and_domain_separated() {
        let o = derive_reserved_salt(SEED, tag_id(), KP_OWNER_ADDRESS).unwrap();
        let k = derive_reserved_salt(SEED, tag_id(), KP_CONSENT_KEY).unwrap();
        let s = derive_reserved_salt(SEED, tag_id(), KP_OWNER_SECRET).unwrap();
        assert_ne!(o, k);
        assert_ne!(k, s);
        assert_ne!(o, s);
        // A salt must never collide with the secret's own derivation.
        let secret_be = to_be_bytes32(&derive_owner_secret(SEED, tag_id()).unwrap());
        assert_ne!(&o[..], &secret_be[..SALT_LEN]);
    }

    /// The reserved leaves must NOT run their value through `field_of_value`. If someone
    /// "simplifies" `hash_reserved_leaf` into `hash_leaf`, this fires.
    #[test]
    fn reserved_leaf_writes_a_raw_field_not_a_folded_value() {
        let salt = [3u8; SALT_LEN];
        let secret = derive_owner_secret(SEED, tag_id()).unwrap();
        let raw = hash_reserved_leaf(KP_OWNER_SECRET, &salt, secret).unwrap();
        let folded = hash_leaf(
            KP_OWNER_SECRET,
            &salt,
            &TypedScalar::Bytes(to_be_bytes32(&secret).to_vec()),
        )
        .unwrap();
        assert_ne!(
            raw, folded,
            "raw-field and field_of_value encodings must differ - the circuit consumes the raw one"
        );
    }

    #[test]
    fn rejects_a_bad_salt_length_and_an_empty_seed() {
        assert!(hash_reserved_leaf(KP_OWNER_SECRET, &[0u8; 15], Fr::from(1u64)).is_err());
        assert!(derive_owner_secret(b"", tag_id()).is_err());
        assert!(build_profile_tree(b"", tag_id(), &addr(), &attrs()).is_err());
    }

    /// An attribute must never be able to occupy a reserved keyPath. `TypeTag::Bytes` IS
    /// `RESERVED_TYPE_TAG` (5), so `hash_leaf(KP_OWNER_SECRET, salt, Bytes(v))` builds a leaf shaped
    /// exactly like the real owner-secret leaf, and the circuit takes `ownerSecret` as an opaque
    /// field - a prover could aim the inclusion proof at either leaf and mint two nullifiers from one
    /// signed consent (D5). `profileRoot` is write-once, so one such tree would be permanent.
    #[test]
    fn rejects_an_attribute_that_collides_with_a_reserved_key_path() {
        for kp in [KP_OWNER_ADDRESS, KP_CONSENT_KEY, KP_OWNER_SECRET] {
            let mut with_collision = attrs();
            with_collision.push(AttributeLeaf {
                key_path: kp.to_string(),
                salt: [5u8; SALT_LEN],
                value: TypedScalar::Bytes(vec![0xde, 0xad, 0xbe, 0xef]),
            });
            let err = match build_profile_tree(SEED, tag_id(), &addr(), &with_collision) {
                Err(e) => e,
                Ok(_) => panic!("keyPath {kp} must not be usable as an attribute"),
            };
            assert!(
                err.to_string().contains(kp),
                "the error must name the offending keyPath, got: {err}"
            );
        }
    }

    /// The guard must not fire on ordinary attributes - `R` must not move for any non-colliding
    /// input (`contracts/test/device-profile-root.json` and the consent-fixture parity test pin it).
    #[test]
    fn ordinary_attributes_are_unaffected_by_the_reserved_key_path_guard() {
        let t = build_profile_tree(SEED, tag_id(), &addr(), &attrs()).unwrap();
        // A keyPath that merely CONTAINS a reserved one is a different field, hence a different leaf.
        let mut near_miss = attrs();
        near_miss.push(AttributeLeaf {
            key_path: format!("credentialSubject.{KP_OWNER_SECRET}"),
            salt: [5u8; SALT_LEN],
            value: TypedScalar::Bytes(vec![0xde, 0xad, 0xbe, 0xef]),
        });
        let n = build_profile_tree(SEED, tag_id(), &addr(), &near_miss).unwrap();
        assert_ne!(t.root, n.root);
    }

    /// The three reserved leaves must be provable members of `R` - the circuit proves exactly this.
    #[test]
    fn all_three_reserved_leaves_are_included_in_the_root() {
        let t = build_profile_tree(SEED, tag_id(), &addr(), &attrs()).unwrap();
        for leaf in [t.owner_address_leaf, t.consent_key_leaf, t.owner_secret_leaf] {
            let proof = merkle_proof(&t.tree.layers, leaf);
            assert_eq!(process_proof(&proof, leaf), t.root);
        }
    }

    /// Changing an attribute changes `R` (the tree really commits to the attributes).
    #[test]
    fn attributes_are_committed_to_the_root() {
        let base = build_profile_tree(SEED, tag_id(), &addr(), &attrs()).unwrap();
        let mut other = attrs();
        other[0].value = TypedScalar::Str("Fido".to_string());
        let changed = build_profile_tree(SEED, tag_id(), &addr(), &other).unwrap();
        assert_ne!(base.root, changed.root);
    }
}
