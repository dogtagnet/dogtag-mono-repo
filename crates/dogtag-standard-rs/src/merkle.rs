//! Poseidon Merkle tree + inclusion proofs (impl §1.3, §11.2; DSDP plan §2.3) — mirror of
//! dogtag-standard-ts/src/merkle.ts.
use ark_bn254::Fr;

use crate::field::field_le;
use crate::leaf::hash_leaf;
use crate::poseidon::{poseidon, to_be_bytes32, DS_NODE};
use crate::types::{DogTagError, TypedScalar};

/// hashNode — commutative: sort the pair as integers in [0,P), then Poseidon(DS_NODE, lo, hi).
pub fn hash_node(a: Fr, b: Fr) -> Fr {
    let (lo, hi) = if field_le(&a, &b) { (a, b) } else { (b, a) };
    poseidon(&[Fr::from(DS_NODE), lo, hi])
}

pub struct MerkleTree {
    pub root: Fr,
    pub layers: Vec<Vec<Fr>>,
}

/// buildMerkle — sort leaves ascending (integer order), fold bottom-up, promote a lone odd node.
pub fn build_merkle(leaf_hashes: &[Fr]) -> MerkleTree {
    assert!(!leaf_hashes.is_empty(), "build_merkle: empty leaf set");
    let mut level: Vec<Fr> = leaf_hashes.to_vec();
    level.sort_by_key(to_be_bytes32); // ascending integer order via canonical 32-byte BE encoding
    let mut layers = vec![level.clone()];
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        let mut i = 0;
        while i < level.len() {
            if i + 1 < level.len() {
                next.push(hash_node(level[i], level[i + 1]));
                i += 2;
            } else {
                next.push(level[i]); // promote odd, no duplicate
                i += 1;
            }
        }
        level = next;
        layers.push(level.clone());
    }
    MerkleTree {root: level[0], layers}
}

/// One root-ward step of a `Sibling | Promote` inclusion proof (DSDP plan §2.3).
///
/// `Sibling(hash)` — the node was paired with `hash` at this level (folded via the commutative
/// [`hash_node`]). `Promote` — the node was the lone odd node at this level and passed through
/// unchanged; it carries no hash. The ordered list of steps, applied leaf→root, exactly replays the
/// [`build_merkle`] fold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofStep {
    Sibling(Fr),
    Promote,
}

/// merkleProof — the ordered, root-ward `Sibling | Promote` list for `leaf_hash` (plan §2.3).
///
/// One step per tree level: `Sibling(neighbor)` when the node is paired with its neighbor, or
/// `Promote` when it is the lone odd node at that level (`idx ^ 1` out of range). Promotion is made
/// **explicit** — unlike the legacy bare-sibling list, which represented a promote by *omission* —
/// so a verifier reconstructs the exact tree shape (and hence the proof depth) from the proof alone,
/// without ever seeing the other leaves.
pub fn merkle_proof(layers: &[Vec<Fr>], leaf_hash: Fr) -> Vec<ProofStep> {
    let mut idx = layers[0]
        .iter()
        .position(|h| *h == leaf_hash)
        .expect("leaf not in tree");
    let mut proof = Vec::with_capacity(layers.len().saturating_sub(1));
    for layer in layers.iter().take(layers.len().saturating_sub(1)) {
        let sib = idx ^ 1;
        if sib < layer.len() {
            proof.push(ProofStep::Sibling(layer[sib]));
        } else {
            proof.push(ProofStep::Promote); // lone odd node at this level — pass-through
        }
        idx >>= 1;
    }
    proof
}

/// processProof — fold a leaf hash root-ward through a `Sibling | Promote` proof (plan §2.3).
///
/// NOTE: this is a fold **primitive**, NOT a membership check. It trusts the `leaf_hash` you hand
/// it, so on its own it proves nothing — an internal node folds just as happily as a real leaf
/// (the C1/E2 hazard: `processProof` is inclusion-only, position/shape unbound under the commutative
/// odd-promotion fold, and it is exactly the opaque-leaf hole the audit flagged). Membership MUST go
/// through [`verify_inclusion`], which recomputes the leaf from its fields under `DS_LEAF` first.
pub fn process_proof(proof: &[ProofStep], leaf_hash: Fr) -> Fr {
    let mut h = leaf_hash;
    for step in proof {
        match step {
            ProofStep::Sibling(s) => h = hash_node(h, *s),
            ProofStep::Promote => {} // lone odd node — pass-through
        }
    }
    h
}

/// verifyInclusion — the NORMATIVE DSDP §2.3 disclosed-leaf check.
///
/// RECOMPUTES the leaf hash from `(key_path, salt, tag, value)` under `DS_LEAF` (Poseidon5) — it
/// never trusts a caller-supplied leaf hash — then folds the `Sibling | Promote` proof root-ward
/// (`Sibling` → commutative `Poseidon3([DS_NODE, min, max])`, `Promote` → pass-through) and returns
/// whether the result equals the anchored root `R`.
///
/// The arity/domain split is what makes this sound: a disclosed leaf can only ever be a Poseidon5
/// image under `DS_LEAF = 1`, while every internal node is a Poseidon3 image under `DS_NODE = 2`. To
/// pass an internal node off as a disclosed leaf, an attacker would need `(key_path, salt, tag,
/// value)` whose Poseidon5 equals that node — preimage-infeasible. Membership itself is unforgeable
/// for the same reason (folding a non-member to `R` needs a Poseidon collision/preimage).
///
/// Per the normative fold, `Promote` steps do not affect the arithmetic result (they are pass-
/// throughs); they carry tree-shape/depth information, not authentication. This function therefore
/// accepts any proof of a genuine member and rejects any leaf whose fields do not fold to `R`.
pub fn verify_inclusion(
    key_path: &str,
    salt: &[u8],
    scalar: &TypedScalar,
    proof: &[ProofStep],
    root: Fr,
) -> Result<bool, DogTagError> {
    let leaf = hash_leaf(key_path, salt, scalar)?;
    Ok(process_proof(proof, leaf) == root)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical (keyPath, salt, tag=Integer) leaf identity used by the shared vectors.
    fn int_leaf(i: usize) -> (String, Vec<u8>, TypedScalar) {
        (format!("leaf{i}"), vec![(100 + i) as u8; 16], TypedScalar::Integer(i.to_string()))
    }

    fn hash_of(rec: &(String, Vec<u8>, TypedScalar)) -> Fr {
        hash_leaf(&rec.0, &rec.1, &rec.2).unwrap()
    }

    #[test]
    fn verify_inclusion_accepts_every_leaf_across_audit_sizes() {
        for k in [1usize, 2, 3, 5, 6, 7, 13, 24, 34] {
            let recs: Vec<_> = (0..k).map(int_leaf).collect();
            let hashes: Vec<Fr> = recs.iter().map(hash_of).collect();
            let tree = build_merkle(&hashes);
            for (idx, rec) in recs.iter().enumerate() {
                let proof = merkle_proof(&tree.layers, hashes[idx]);
                // one step per level: the proof depth is the tree depth.
                assert_eq!(proof.len(), tree.layers.len() - 1, "size {k} leaf {idx} proof depth");
                assert!(
                    verify_inclusion(&rec.0, &rec.1, &rec.2, &proof, tree.root).unwrap(),
                    "size {k} leaf {idx} must verify"
                );
            }
        }
    }

    #[test]
    fn multi_level_promotion_is_exercised_and_pass_through() {
        // The sorted-last leaf of an odd tree is promoted at consecutive levels; verifying it walks
        // several `Promote` (pass-through) steps then a single `Sibling`.
        let recs: Vec<_> = (0..5).map(int_leaf).collect();
        let hashes: Vec<Fr> = recs.iter().map(hash_of).collect();
        let tree = build_merkle(&hashes);
        let mut saw_multi = false;
        for (idx, rec) in recs.iter().enumerate() {
            let proof = merkle_proof(&tree.layers, hashes[idx]);
            if proof.iter().filter(|s| matches!(s, ProofStep::Promote)).count() >= 2 {
                saw_multi = true;
            }
            assert!(verify_inclusion(&rec.0, &rec.1, &rec.2, &proof, tree.root).unwrap());
        }
        assert!(saw_multi, "a size-5 tree must exercise >= 2-level promotion");
    }

    #[test]
    fn process_proof_is_a_fold_primitive_not_a_membership_check() {
        // 4-leaf tree: sorted layer0 [a,b,c,d] -> [ab,cd] -> root = hashNode(ab,cd).
        let recs: Vec<_> = (0..4).map(int_leaf).collect();
        let hashes: Vec<Fr> = recs.iter().map(hash_of).collect();
        let tree = build_merkle(&hashes);
        let node = tree.layers[1][0]; // an INTERNAL node (a Poseidon3 image under DS_NODE)
        let sibling = tree.layers[1][1];
        // The permissive fold folds the internal node straight to the root — this is exactly the
        // opaque-leaf hole the audit flagged: `process_proof` alone proves nothing.
        assert_eq!(process_proof(&[ProofStep::Sibling(sibling)], node), tree.root);
        // But `verify_inclusion` RECOMPUTES the leaf under DS_LEAF (Poseidon5). No credential fields
        // can equal an internal node (the arity/domain split), so presenting the node's proof with
        // any concrete leaf fields is rejected.
        let atk = int_leaf(99);
        assert_ne!(hash_of(&atk), node, "a Poseidon5 leaf can never equal a Poseidon3 node");
        assert!(
            !verify_inclusion(&atk.0, &atk.1, &atk.2, &[ProofStep::Sibling(sibling)], tree.root).unwrap(),
            "recompute-from-fields + arity split must reject an internal-node-as-leaf forgery"
        );
    }

    #[test]
    fn tampered_value_and_wrong_root_are_rejected() {
        let recs: Vec<_> = (0..7).map(int_leaf).collect();
        let hashes: Vec<Fr> = recs.iter().map(hash_of).collect();
        let tree = build_merkle(&hashes);
        let proof = merkle_proof(&tree.layers, hashes[0]);
        // correct fields verify
        assert!(verify_inclusion(&recs[0].0, &recs[0].1, &recs[0].2, &proof, tree.root).unwrap());
        // tampered value -> different leaf -> does not fold to root
        let tampered = TypedScalar::Integer("12345".to_string());
        assert!(!verify_inclusion(&recs[0].0, &recs[0].1, &tampered, &proof, tree.root).unwrap());
        // wrong root -> rejected
        let other = build_merkle(&(0..8).map(int_leaf).map(|r| hash_of(&r)).collect::<Vec<_>>()).root;
        assert!(!verify_inclusion(&recs[0].0, &recs[0].1, &recs[0].2, &proof, other).unwrap());
    }
}
