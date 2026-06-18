// Poseidon Merkle tree (impl §1.3, §11.2). Commutative sorted-pair node hash, integer [0,P)
// comparator, odd-promotion (never duplicate), single-leaf root == that leaf. The in-circuit
// ordered tree applies the same sortPair+DS_NODE so the proven root == this R (§11.10(e)).
import {DS_NODE, poseidon, type Field} from "./field.js";

/** hashNode — commutative: sort the pair as integers in [0,P), then Poseidon(DS_NODE, lo, hi). */
export function hashNode(a: Field, b: Field): Field {
  const [lo, hi] = a <= b ? [a, b] : [b, a];
  return poseidon([DS_NODE, lo, hi]);
}

export interface MerkleTree {
  root: Field;
  layers: Field[][];
}

/** buildMerkle — sort leaves ascending, fold bottom-up, promote a lone odd node unchanged. */
export function buildMerkle(leafHashes: Field[]): MerkleTree {
  if (leafHashes.length === 0) throw new Error("buildMerkle: empty leaf set");
  let level = [...leafHashes].sort((x, y) => (x < y ? -1 : x > y ? 1 : 0));
  const layers: Field[][] = [level];
  while (level.length > 1) {
    const next: Field[] = [];
    for (let i = 0; i < level.length; ) {
      if (i + 1 < level.length) {
        next.push(hashNode(level[i]!, level[i + 1]!));
        i += 2;
      } else {
        next.push(level[i]!); // promote odd, no duplicate
        i += 1;
      }
    }
    level = next;
    layers.push(level);
  }
  return {root: level[0]!, layers};
}

/** merkleProof — the sibling set (unordered ok: commutative); skips a promoted (sibling-less) node. */
export function merkleProof(layers: Field[][], leafHash: Field): Field[] {
  let idx = layers[0]!.findIndex((h) => h === leafHash);
  if (idx < 0) throw new Error("leaf not in tree");
  const proof: Field[] = [];
  for (let l = 0; l < layers.length - 1; l++) {
    const sib = idx ^ 1;
    const layer = layers[l]!;
    if (sib < layer.length) proof.push(layer[sib]!);
    idx = idx >> 1;
  }
  return proof;
}

/** processProof — recompute the root from a leaf + its sibling set. */
export function processProof(proof: Field[], leaf: Field): Field {
  let h = leaf;
  for (const s of proof) h = hashNode(h, s);
  return h;
}
