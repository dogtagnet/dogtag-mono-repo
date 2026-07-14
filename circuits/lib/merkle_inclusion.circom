pragma circom 2.1.9;

// Shared Merkle-inclusion primitives for the Level-B owner-unlinkable consent circuit
// (M2, DogTagConsent). These fold a SINGLE leaf up its inclusion path to the tree root R,
// using the SAME commutative sorted-pair node rule as the SDK buildMerkle / hashNode
// (crates/dogtag-standard-rs/src/merkle.rs, packages/dogtag-standard-ts/src/merkle.ts) and
// the frozen Level-A circuit (verification.circom).
//
// WHY A SEPARATE FILE (not shared with verification.circom): verification.circom re-derives
// the WHOLE tree (permutation + sort + variable-count fold) and has a frozen dev zkey. M2's
// DogTagConsent proves membership via caller-supplied inclusion PATHS instead, so it only needs
// the node hash + comparators + a per-leaf fold. LessThanField / LessEqThanField / NodeHash are
// COPIED verbatim from verification.circom (kept bit-identical so both circuits agree on the
// integer sort inside hashNode) rather than refactoring the frozen circuit.

include "poseidon.circom";
include "comparators.circom";
include "bitify.circom";

// ---------------------------------------------------------------------------
// LessThanField: strict a < b over FULL field elements (genuine elements in
// [0,p), p<2^254). circomlib's LessThan asserts n<=252 and Poseidon outputs span
// the full field, so we bit-decompose a and b (254 bits each, Num2Bits enforces
// 254-bit range) and compare lexicographically from the MSB.
// (Copied verbatim from verification.circom.)
// ---------------------------------------------------------------------------
template LessThanField() {
    signal input a;
    signal input b;
    signal output out; // 1 iff a < b (as integers in [0,2^254))

    component ab = Num2Bits(254);
    component bb = Num2Bits(254);
    ab.in <== a;
    bb.in <== b;

    signal eqPrefix[255];
    signal ltAcc[255];
    signal aIsLt[254];
    signal aib[254];
    signal eqBit[254];
    eqPrefix[0] <== 1;
    ltAcc[0] <== 0;
    for (var k = 0; k < 254; k++) {
        var i = 253 - k; // current bit index, MSB first
        aIsLt[k] <== (1 - ab.out[i]) * bb.out[i];
        ltAcc[k + 1] <== ltAcc[k] + eqPrefix[k] * aIsLt[k];
        aib[k] <== ab.out[i] * bb.out[i];
        eqBit[k] <== 1 - ab.out[i] - bb.out[i] + 2 * aib[k];
        eqPrefix[k + 1] <== eqPrefix[k] * eqBit[k];
    }
    out <== ltAcc[254];
}

template LessEqThanField() {
    signal input a;
    signal input b;
    signal output out; // 1 iff a <= b
    component lt = LessThanField();
    lt.a <== b;
    lt.b <== a;
    out <== 1 - lt.out; // a<=b  <=>  NOT(b<a)
}

// ---------------------------------------------------------------------------
// NodeHash: commutative SDK node — Poseidon3([DS_NODE, lo, hi]) with lo<=hi by
// integer value (full-field 254-bit comparator, matching SDK hashNode).
// (Copied verbatim from verification.circom.)
// ---------------------------------------------------------------------------
template NodeHash() {
    signal input a;
    signal input b;
    signal output out;

    component le = LessEqThanField();
    le.a <== a;
    le.b <== b;
    signal aLeqB <== le.out; // boolean

    signal diff <== a - b;
    signal lo <== b + aLeqB * diff;
    signal hi <== a + b - lo;

    component h = Poseidon(3);
    h.inputs[0] <== 2;   // DS_NODE
    h.inputs[1] <== lo;
    h.inputs[2] <== hi;
    out <== h.out;
}

// ---------------------------------------------------------------------------
// MerkleInclusion(depth): fold ONE leaf up a caller-supplied inclusion path to the
// root, mirroring the SDK processProof (merkle.rs / merkle.ts).
//
// The SDK merkleProof returns a FRONT-PACKED sibling set: it skips levels where the
// node is odd-promoted (no sibling), so the proof has exactly `pathLen` entries where
// pathLen <= tree depth. processProof folds them sequentially: h = hashNode(h, sib).
// This template reproduces that exactly with a FIXED-width array + a length:
//
//   h[0] = leaf
//   for i in [0,depth):  enabled_i = (i < pathLen)
//                        h[i+1]    = enabled_i ? hashNode(h[i], siblings[i]) : h[i]
//   root = h[depth]
//
// SOUNDNESS: root is asserted == the public R on-chain-committed root. A prover who does
// not hold a genuine path cannot fold an arbitrary leaf to a fixed committed R without a
// Poseidon collision, so free `siblings`/`pathLen` inputs do not weaken membership — the
// only freedom is which real path the honest prover supplies. Odd-promotion levels are the
// skipped (enabled_i = 0) steps; the node passes through unchanged, matching the SDK.
//
// `depth` is the MAX inclusion-path length = the max tree depth the VK supports. Trees with
// up to 2^depth leaves are covered; a leaf's real path uses pathLen<=depth of the slots and
// pads the unused tail siblings with 0 (their NodeHash is computed but discarded).
// ---------------------------------------------------------------------------
template MerkleInclusion(depth) {
    signal input leaf;
    signal input siblings[depth];
    signal input pathLen;        // number of real siblings, 0..depth
    signal output root;

    // pathLen in [0,depth]: bound it small (depth<=8 here => <256) so the LessThan below is well-defined.
    component plBits = Num2Bits(8);
    plBits.in <== pathLen;
    component plHi = LessThan(8);      // pathLen <= depth  <=>  pathLen < depth+1
    plHi.in[0] <== pathLen;
    plHi.in[1] <== depth + 1;
    plHi.out === 1;

    signal h[depth + 1];
    h[0] <== leaf;

    component enabledLt[depth];
    component nh[depth];
    signal paired[depth];
    signal step[depth];
    for (var i = 0; i < depth; i++) {
        // enabled_i = (i < pathLen)
        enabledLt[i] = LessThan(8);    // i, pathLen <= depth <= 8 < 256
        enabledLt[i].in[0] <== i;
        enabledLt[i].in[1] <== pathLen;

        nh[i] = NodeHash();
        nh[i].a <== h[i];
        nh[i].b <== siblings[i];
        paired[i] <== nh[i].out;

        // h[i+1] = enabled ? paired : h[i]  = h[i] + enabled*(paired - h[i])
        step[i] <== enabledLt[i].out * (paired[i] - h[i]);
        h[i + 1] <== h[i] + step[i];
    }
    root <== h[depth];
}
