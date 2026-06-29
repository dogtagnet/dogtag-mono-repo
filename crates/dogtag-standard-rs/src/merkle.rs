//! Poseidon Merkle tree (impl §1.3, §11.2) — mirror of dogtag-standard-ts/src/merkle.ts.
use ark_bn254::Fr;

use crate::field::field_le;
use crate::poseidon::{poseidon, to_be_bytes32, DS_NODE};

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
    level.sort_by_key(to_be_bytes32);
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

/// merkleProof — the sibling set (unordered ok: commutative); skips a promoted node.
pub fn merkle_proof(layers: &[Vec<Fr>], leaf_hash: Fr) -> Vec<Fr> {
    let mut idx = layers[0]
        .iter()
        .position(|h| *h == leaf_hash)
        .expect("leaf not in tree");
    let mut proof = Vec::new();
    for layer in layers.iter().take(layers.len().saturating_sub(1)) {
        let sib = idx ^ 1;
        if sib < layer.len() {
            proof.push(layer[sib]);
        }
        idx >>= 1;
    }
    proof
}

/// processProof — recompute the root from a leaf + its sibling set.
pub fn process_proof(proof: &[Fr], leaf: Fr) -> Fr {
    let mut h = leaf;
    for s in proof {
        h = hash_node(h, *s);
    }
    h
}
