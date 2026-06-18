pragma circom 2.1.9;

// DogTag consent-bound credential verification circuit (impl §11.8(d), §11.9(d), §11.10).
//
// Public signal vector (snarkjs order = declared public INPUTS first, then OUTPUTS in
// declaration order, as circom appends outputs after inputs):
//   [dogTagId, purpose, relayer, subject, nullifier, keyHash, R]
//
// PROVES (§11.9(d)):
//   (a) Each leaf = Poseidon5([DS_LEAF, keyPathHash, salt, typeTag, value]); the supplied
//       sortedLeafHashes is a true PERMUTATION of those leaves and is strictly ascending;
//       fold sortedLeafHashes with the SDK node rule Poseidon3([DS_NODE, lo, hi]) (sorted pair)
//       to obtain the credential root R == the SDK's buildMerkle root.
//   (b) leafValues[dogTagIdLeafIndex] == dogTagId, dogTagIdLeafIndex in [0,N), and the
//       dogTagId leaf's keyPath-hash == the bound credentialSubject.dogTagId keyPath field.
//   (c) EdDSA-BabyJubjub verify (Ax,Ay,S,R8x,R8y) over M = Poseidon6([dogTagId, purpose,
//       relayer, subject, R, consentNonce]).
//   (d) keyHash = Poseidon2([Ax, Ay]).
//   (e) nullifier = Poseidon6([DS_NULLIFIER, dogTagId, purpose, relayer, subject, consentNonce]).
//   (f) range-check relayer, subject < 2^160.
//
// SIMPLIFICATIONS vs production (documented, see README):
//   - N=8 (power of two), depth=3. Production credential trees are larger (e.g. N up to 24,
//     depth 5). For N a power of two the fold is purely pairwise; ODD-PROMOTION for non-power-of-2
//     N is NOT implemented here (TODO below) — required for production sizes.
//   - Dev trusted setup only (small ptau, single contributor). Production needs the Hermez ptau
//     and a >=3-contributor phase-2 ceremony ending in a public beacon.

include "poseidon.circom";
include "comparators.circom";
include "bitify.circom";
include "eddsaposeidon.circom";

// Domain-separation tags (impl §1.2 / field.ts).
// DS_LEAF=1, DS_NODE=2, DS_NULLIFIER=4.

// ---------------------------------------------------------------------------
// LessThanField: strict a < b over FULL field elements (genuine elements in
// [0,p), p<2^254). circomlib's LessThan asserts n<=252 and Poseidon outputs span
// the full field, so we bit-decompose a and b (254 bits each, Num2Bits enforces
// 254-bit range) and compare lexicographically from the MSB.
//   eq[i]   = prefix bits [254..i+1] equal
//   lt accumulates: at the highest differing bit, a's bit < b's bit  =>  a < b.
// ---------------------------------------------------------------------------
template LessThanField() {
    signal input a;
    signal input b;
    signal output out; // 1 iff a < b (as integers in [0,2^254))

    component ab = Num2Bits(254);
    component bb = Num2Bits(254);
    ab.in <== a;
    bb.in <== b;

    // Walk MSB->LSB. gt/lt set once at the first differing bit.
    // running "still equal so far" flag.
    signal eqPrefix[255]; // eqPrefix[k] = all bits strictly above index (253-k+1) equal
    signal ltAcc[255];
    signal aIsLt[254];
    signal aib[254];
    signal eqBit[254];
    eqPrefix[0] <== 1;
    ltAcc[0] <== 0;
    for (var k = 0; k < 254; k++) {
        var i = 253 - k; // current bit index, MSB first
        // aIsLt = (a_i == 0) && (b_i == 1) = (1 - a_i) * b_i
        aIsLt[k] <== (1 - ab.out[i]) * bb.out[i];
        // contribute to ltAcc only while prefix equal
        ltAcc[k + 1] <== ltAcc[k] + eqPrefix[k] * aIsLt[k];
        // eqBit_i = 1 - a_i - b_i + 2 a_i b_i  (a_i,b_i boolean)
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
// PermutationApply(N): given the matrix-free index list `perm`, prove that
// `out` is `in` permuted by a genuine permutation. We build an N×N 0/1 matrix
// sel[k][j] = (perm[k] == j). Constraining each row to sum to 1 and each column
// to sum to 1 (with sel binary) forces sel to be a permutation matrix, hence
// perm is a bijection on [0,N). out[k] = sum_j sel[k][j] * in[j].
// ---------------------------------------------------------------------------
template PermutationApply(N) {
    signal input in[N];     // computed leaf hashes (canonical order)
    signal input perm[N];   // prover-supplied: out[k] == in[perm[k]]
    signal output out[N];   // == sortedLeafHashes (asserted ascending by caller)

    // sel[k][j] in {0,1}, sel[k][j] == 1 iff perm[k] == j.
    signal sel[N][N];
    component eqc[N][N];
    for (var k = 0; k < N; k++) {
        for (var j = 0; j < N; j++) {
            eqc[k][j] = IsEqual();
            eqc[k][j].in[0] <== perm[k];
            eqc[k][j].in[1] <== j;
            sel[k][j] <== eqc[k][j].out; // IsEqual guarantees boolean output
        }
    }

    // Each row sums to exactly 1 (perm[k] hits exactly one column in [0,N)).
    for (var k = 0; k < N; k++) {
        var rowSum = 0;
        for (var j = 0; j < N; j++) { rowSum += sel[k][j]; }
        rowSum === 1;
    }
    // Each column sums to exactly 1 (distinctness: no index used twice).
    for (var j = 0; j < N; j++) {
        var colSum = 0;
        for (var k = 0; k < N; k++) { colSum += sel[k][j]; }
        colSum === 1;
    }

    // out[k] = sum_j sel[k][j] * in[j]  (= in[perm[k]]). Accumulate via partial
    // sums so each constraint stays quadratic.
    signal partial[N][N + 1];
    for (var k = 0; k < N; k++) {
        partial[k][0] <== 0;
        for (var j = 0; j < N; j++) {
            partial[k][j + 1] <== partial[k][j] + sel[k][j] * in[j];
        }
        out[k] <== partial[k][N];
    }
}

// ---------------------------------------------------------------------------
// NodeHash: commutative SDK node — Poseidon3([DS_NODE, lo, hi]) with lo<=hi by
// integer value. We pick the order with a comparator + mux (252-bit compare is
// safe: field elements are < 2^254 but distinct sortedLeafHashes are ascending,
// so a/b already ordered; we still sort defensively to match SDK hashNode).
// ---------------------------------------------------------------------------
template NodeHash() {
    signal input a;
    signal input b;
    signal output out;

    // aLeqB = (a <= b) over full field elements (SDK hashNode sorts by integer value).
    component le = LessEqThanField();
    le.a <== a;
    le.b <== b;
    signal aLeqB <== le.out; // boolean

    // lo = aLeqB ? a : b = b + aLeqB*(a-b) ; hi = a + b - lo
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
// Main circuit. N must be a power of two for this pairwise fold (N=8 -> depth 3).
// ---------------------------------------------------------------------------
template DogTagVerification(N, depth) {
    // ---- PUBLIC inputs (order is load-bearing) ----
    signal input dogTagId;
    signal input purpose;
    signal input relayer;
    signal input subject;

    // ---- PRIVATE inputs ----
    signal input leafKeyPathHashes[N]; // fieldOf(keyPath) precomputed by prover
    signal input leafTypeTags[N];
    signal input leafSalts[N];         // fieldOf(salt)
    signal input leafValues[N];        // fieldOf(value)
    signal input dogTagIdLeafIndex;    // index (canonical order) of the dogTagId leaf
    signal input sortedLeafHashes[N];  // SDK ascending-sorted leaf order
    signal input perm[N];              // sortedLeafHashes[k] == leafHash[perm[k]]
    signal input dogTagKeyPathField;   // fieldOf("credentialSubject.dogTagId") — bound below
    signal input consentNonce;
    signal input Ax;
    signal input Ay;
    signal input R8x;
    signal input R8y;
    signal input S;

    // ---- OUTPUT signals (appended to public vector after the 4 inputs) ----
    signal output nullifier;
    signal output keyHash;
    signal output R;

    // ===================================================================
    // (a.1) recompute each leaf hash = Poseidon5([DS_LEAF, kp, salt, tag, val])
    // ===================================================================
    signal leafHash[N];
    component lh[N];
    for (var i = 0; i < N; i++) {
        lh[i] = Poseidon(5);
        lh[i].inputs[0] <== 1; // DS_LEAF
        lh[i].inputs[1] <== leafKeyPathHashes[i];
        lh[i].inputs[2] <== leafSalts[i];
        lh[i].inputs[3] <== leafTypeTags[i];
        lh[i].inputs[4] <== leafValues[i];
        leafHash[i] <== lh[i].out;
    }

    // ===================================================================
    // (a.2) sortedLeafHashes is a true permutation of leafHash[]
    // ===================================================================
    component pa = PermutationApply(N);
    for (var i = 0; i < N; i++) {
        pa.in[i] <== leafHash[i];
        pa.perm[i] <== perm[i];
    }
    for (var i = 0; i < N; i++) {
        sortedLeafHashes[i] === pa.out[i]; // bind prover-supplied sorted array to the permutation
    }

    // ===================================================================
    // (a.3) sortedLeafHashes strictly ascending (SDK sorts ascending by integer
    //       value; leaf hashes are distinct for distinct credentials). Using
    //       strict LessThan also rejects duplicate leaves, matching a set.
    // ===================================================================
    component asc[N - 1];
    for (var i = 0; i < N - 1; i++) {
        asc[i] = LessThanField();
        asc[i].a <== sortedLeafHashes[i];
        asc[i].b <== sortedLeafHashes[i + 1];
        asc[i].out === 1;
    }

    // ===================================================================
    // (a.4) fold sortedLeafHashes bottom-up with the SDK node rule.
    //       N is a power of two -> pure pairwise fold, no odd-promotion.
    //       TODO(production): for non-power-of-2 N, PROMOTE a lone odd node
    //       unchanged (merkle.ts buildMerkle) instead of padding.
    // ===================================================================
    // level sizes: N, N/2, ..., 1   (depth = log2(N))
    var levelSize = N;
    // Flattened storage of node hashes per level.
    component nh[N]; // upper bound on nodes; we use fewer
    signal level[depth + 1][N];
    for (var i = 0; i < N; i++) { level[0][i] <== sortedLeafHashes[i]; }

    var nodeIdx = 0;
    for (var d = 0; d < depth; d++) {
        var half = levelSize \ 2;
        for (var i = 0; i < half; i++) {
            nh[nodeIdx] = NodeHash();
            nh[nodeIdx].a <== level[d][2 * i];
            nh[nodeIdx].b <== level[d][2 * i + 1];
            level[d + 1][i] <== nh[nodeIdx].out;
            nodeIdx++;
        }
        levelSize = half;
    }
    R <== level[depth][0];

    // ===================================================================
    // (b) dogTagId leaf binding: value == dogTagId, index in [0,N), keyPath bound.
    // ===================================================================
    // Range-check index in [0, N).
    component idxLt = LessThan(8); // N <= 255 here
    idxLt.in[0] <== dogTagIdLeafIndex;
    idxLt.in[1] <== N;
    idxLt.out === 1;

    // Select leafValues[dogTagIdLeafIndex] and leafKeyPathHashes[dogTagIdLeafIndex]
    // via a 0/1 selector vector (same trick as PermutationApply, one-hot row).
    signal isel[N];
    component iseleq[N];
    var iselSum = 0;
    for (var i = 0; i < N; i++) {
        iseleq[i] = IsEqual();
        iseleq[i].in[0] <== dogTagIdLeafIndex;
        iseleq[i].in[1] <== i;
        isel[i] <== iseleq[i].out;
        iselSum += isel[i];
    }
    iselSum === 1; // exactly one index matches (redundant with range-check but cheap)

    signal selValuePartial[N + 1];
    signal selKeyPathPartial[N + 1];
    selValuePartial[0] <== 0;
    selKeyPathPartial[0] <== 0;
    for (var i = 0; i < N; i++) {
        selValuePartial[i + 1] <== selValuePartial[i] + isel[i] * leafValues[i];
        selKeyPathPartial[i + 1] <== selKeyPathPartial[i] + isel[i] * leafKeyPathHashes[i];
    }
    selValuePartial[N] === dogTagId;            // (b) value binds public dogTagId
    selKeyPathPartial[N] === dogTagKeyPathField; // (b) keyPath binds the credentialSubject.dogTagId path

    // ===================================================================
    // (e) nullifier = Poseidon6([DS_NULLIFIER, dogTagId, purpose, relayer, subject, nonce])
    // ===================================================================
    component nf = Poseidon(6);
    nf.inputs[0] <== 4; // DS_NULLIFIER
    nf.inputs[1] <== dogTagId;
    nf.inputs[2] <== purpose;
    nf.inputs[3] <== relayer;
    nf.inputs[4] <== subject;
    nf.inputs[5] <== consentNonce;
    nullifier <== nf.out;

    // ===================================================================
    // (d) keyHash = Poseidon2([Ax, Ay])
    // ===================================================================
    component kh = Poseidon(2);
    kh.inputs[0] <== Ax;
    kh.inputs[1] <== Ay;
    keyHash <== kh.out;

    // ===================================================================
    // (c) EdDSA-BabyJubjub verify over M = Poseidon6([dogTagId, purpose, relayer,
    //     subject, R, consentNonce]). NO domain tag (per spec note).
    // ===================================================================
    component mh = Poseidon(6);
    mh.inputs[0] <== dogTagId;
    mh.inputs[1] <== purpose;
    mh.inputs[2] <== relayer;
    mh.inputs[3] <== subject;
    mh.inputs[4] <== R;
    mh.inputs[5] <== consentNonce;

    component sig = EdDSAPoseidonVerifier();
    sig.enabled <== 1;
    sig.Ax <== Ax;
    sig.Ay <== Ay;
    sig.S <== S;
    sig.R8x <== R8x;
    sig.R8y <== R8y;
    sig.M <== mh.out;

    // ===================================================================
    // (f) range-check relayer, subject < 2^160 (addresses).
    // ===================================================================
    component relayerBits = Num2Bits(160);
    relayerBits.in <== relayer;
    component subjectBits = Num2Bits(160);
    subjectBits.in <== subject;
}

// Dev/test instantiation: N=8, depth=3 (NOT production 24/5; see header + README).
component main {public [dogTagId, purpose, relayer, subject]} = DogTagVerification(8, 3);
