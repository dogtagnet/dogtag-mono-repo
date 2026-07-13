// Poseidon Merkle tree + inclusion proofs (impl §1.3, §11.2; DSDP plan §2.3). Commutative
// sorted-pair node hash, integer [0,P) comparator, odd-promotion (never duplicate), single-leaf
// root == that leaf. The in-circuit ordered tree applies the same sortPair+DS_NODE so the proven
// root == this R (§11.10(e)). Mirror of crates/dogtag-standard-rs/src/merkle.rs.
import {DS_NODE, poseidon, type Field} from "./field.js";
import {hashLeaf} from "./leaf.js";
import type {TypedScalar} from "./types.js";

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

/**
 * One root-ward step of a `Sibling | Promote` inclusion proof (DSDP plan §2.3).
 *
 * `{sibling}` — the node was paired with `sibling` at this level (folded via the commutative
 * {@link hashNode}). `{promote: true}` — the node was the lone odd node at this level and passed
 * through unchanged; it carries no hash. The ordered list, applied leaf→root, replays the
 * {@link buildMerkle} fold. (Serialized as `{"sibling":"0x…"}` / `{"promote":true}` — plan §4.4.)
 */
export type ProofStep = {sibling: Field} | {promote: true};

/**
 * merkleProof — the ordered, root-ward `Sibling | Promote` list for `leafHash` (plan §2.3).
 *
 * One step per tree level: `{sibling}` when the node is paired with its neighbor, or `{promote}`
 * when it is the lone odd node at that level (`idx ^ 1` out of range). Promotion is made **explicit**
 * — unlike the legacy bare-sibling list, which represented a promote by *omission* — so a verifier
 * reconstructs the exact tree shape (and hence the proof depth) from the proof alone.
 */
export function merkleProof(layers: Field[][], leafHash: Field): ProofStep[] {
  let idx = layers[0]!.findIndex((h) => h === leafHash);
  if (idx < 0) throw new Error("leaf not in tree");
  const proof: ProofStep[] = [];
  for (let l = 0; l < layers.length - 1; l++) {
    const layer = layers[l]!;
    const sib = idx ^ 1;
    if (sib < layer.length) proof.push({sibling: layer[sib]!});
    else proof.push({promote: true}); // lone odd node at this level — pass-through
    idx = idx >> 1;
  }
  return proof;
}

/**
 * processProof — fold a leaf hash root-ward through a `Sibling | Promote` proof (plan §2.3).
 *
 * NOTE: this is a fold **primitive**, NOT a membership check. It trusts the `leafHash` you hand it,
 * so on its own it proves nothing — an internal node folds just as happily as a real leaf (the
 * C1/E2 hazard: `processProof` is inclusion-only, position/shape unbound under the commutative
 * odd-promotion fold, and it is exactly the opaque-leaf hole the audit flagged). Membership MUST go
 * through {@link verifyInclusion}, which recomputes the leaf from its fields under `DS_LEAF` first.
 */
export function processProof(proof: ProofStep[], leafHash: Field): Field {
  let h = leafHash;
  for (const step of proof) {
    if ("sibling" in step) h = hashNode(h, step.sibling);
    // {promote}: lone odd node — pass-through, no hashing
  }
  return h;
}

/**
 * verifyInclusion — the NORMATIVE DSDP §2.3 disclosed-leaf check.
 *
 * RECOMPUTES the leaf hash from `(keyPath, salt, tag, value)` under `DS_LEAF` (Poseidon5) — it never
 * trusts a caller-supplied leaf hash — then folds the `Sibling | Promote` proof root-ward
 * (`{sibling}` → commutative `Poseidon3([DS_NODE, min, max])`, `{promote}` → pass-through) and
 * returns whether the result equals the anchored root `R`.
 *
 * The arity/domain split is what makes this sound: a disclosed leaf can only ever be a Poseidon5
 * image under `DS_LEAF = 1`, while every internal node is a Poseidon3 image under `DS_NODE = 2`. To
 * pass an internal node off as a disclosed leaf, an attacker would need `(keyPath, salt, tag, value)`
 * whose Poseidon5 equals that node — preimage-infeasible. Membership itself is unforgeable for the
 * same reason (folding a non-member to `R` needs a Poseidon collision/preimage). `{promote}` steps
 * do not affect the arithmetic result; they carry tree-shape/depth information, not authentication.
 */
export function verifyInclusion(
  keyPath: string,
  salt: Uint8Array,
  scalar: TypedScalar,
  proof: ProofStep[],
  root: Field,
): boolean {
  const leaf = hashLeaf(keyPath, salt, scalar);
  return processProof(proof, leaf) === root;
}
