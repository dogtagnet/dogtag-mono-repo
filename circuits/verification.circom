pragma circom 2.1.9;

// DogTag consent-bound credential verification circuit (impl §11.8(d), §11.9(d), §11.10).
//
// Public signal vector (snarkjs order = declared public INPUTS first, then OUTPUTS in
// declaration order, as circom appends outputs after inputs):
//   [dogTagId, purpose, relayer, subject, nullifier, keyHash, R]
//
// PROVES (§11.9(d)):
//   (a) Each leaf = Poseidon5([DS_LEAF, keyPathHash, salt, typeTag, value]); the supplied
//       sortedLeafHashes (first numLeaves entries) is a true PERMUTATION of those leaves and
//       is strictly ascending; fold those numLeaves leaves with the SDK node rule
//       Poseidon3([DS_NODE, lo, hi]) (sorted pair) WITH ODD-PROMOTION to obtain the credential
//       root R == the SDK's buildMerkle root over exactly numLeaves leaves (1..N, any count).
//   (b) leafValues[dogTagIdLeafIndex] == dogTagId, dogTagIdLeafIndex in [0,numLeaves), and the
//       dogTagId leaf's keyPath-hash == the bound credentialSubject.dogTagId keyPath field.
//   (c) EdDSA-BabyJubjub verify (Ax,Ay,S,R8x,R8y) over M = Poseidon6([dogTagId, purpose,
//       relayer, subject, R, consentNonce]).
//   (d) keyHash = Poseidon2([Ax, Ay]).
//   (e) nullifier = Poseidon6([DS_NULLIFIER, dogTagId, purpose, relayer, subject, consentNonce]).
//   (f) range-check relayer, subject < 2^160.
//
// PRODUCTION: N=24 max leaves, depth=5, VARIABLE actual leaf count `numLeaves` (1..24), with
// correct ODD-PROMOTION matching the SDK's buildMerkle bit-for-bit for any leaf count.
//
// CEREMONY: still DEV trusted setup only (small ptau, single contributor) — see README/setup.sh.
// Production needs the Hermez ptau and a >=3-contributor phase-2 ceremony ending in a public beacon.

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
// NodeHash: commutative SDK node — Poseidon3([DS_NODE, lo, hi]) with lo<=hi by
// integer value (full-field 254-bit comparator, matching SDK hashNode).
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
// PermutationApply(N): given the matrix-free index list `perm`, prove that
// `out` is `in` permuted by a genuine permutation OVER THE ACTIVE PREFIX [0,numLeaves).
// We build an N×N 0/1 matrix sel[k][j] = (perm[k] == j). The permutation argument is
// gated by `active[i] = (i < numLeaves)`: for active rows/cols we force a bijection of
// the active index set onto itself; inactive rows/cols are forced to the identity
// (sel[i][i]=1) so they don't smuggle in extra/reordered leaves.
//   - row k active  : exactly one active column hit -> perm[k] in [0,numLeaves)
//   - col j active  : hit by exactly one active row
//   - row/col inactive: pinned to identity (sel[k][k]=1, all others 0)
// out[k] = sum_j sel[k][j] * in[j]  (= in[perm[k]] for active k).
// ---------------------------------------------------------------------------
template PermutationApply(N) {
    signal input in[N];        // computed leaf hashes (canonical order)
    signal input perm[N];      // prover-supplied: out[k] == in[perm[k]] for active k
    signal input active[N];    // active[i] = (i < numLeaves), boolean (asserted by caller)
    signal output out[N];      // == sortedLeafHashes (active prefix asserted ascending by caller)

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

    // For inactive rows k (k>=numLeaves), pin perm[k]==k so the row maps to its own
    // (inactive) column. This makes the active sub-block a self-contained bijection.
    for (var k = 0; k < N; k++) {
        // (1 - active[k]) * (1 - sel[k][k]) == 0  -> inactive row must hit its diagonal.
        (1 - active[k]) * (1 - sel[k][k]) === 0;
    }

    // Row sums: every row (active or inactive) hits exactly one column in [0,N).
    for (var k = 0; k < N; k++) {
        var rowSum = 0;
        for (var j = 0; j < N; j++) { rowSum += sel[k][j]; }
        rowSum === 1;
    }
    // Column sums: every column hit exactly once. Combined with row sums this forces a
    // permutation of [0,N); combined with the inactive-diagonal pin above, the active
    // rows must permute exactly among the active columns.
    for (var j = 0; j < N; j++) {
        var colSum = 0;
        for (var k = 0; k < N; k++) { colSum += sel[k][j]; }
        colSum === 1;
    }

    // An active row may NOT point at an inactive column (would let a real leaf land in a
    // padding slot, dropping it from the fold). For active row k: forbid sel[k][j]=1 with
    // j inactive. Equivalently the active rows hit exactly the active columns. Since row/col
    // sums already force a full permutation of [0,N) and inactive rows are pinned to their
    // own diagonal (active[k]==0 -> sel[k][k]=1), the active rows are exactly the complement
    // and must map onto the active columns. We additionally pin each active row to an active
    // column explicitly for defense in depth: active[k]*sel[k][j]*(1-active[j]) == 0.
    signal aMix[N][N];
    for (var k = 0; k < N; k++) {
        for (var j = 0; j < N; j++) {
            aMix[k][j] <== active[k] * sel[k][j];
            aMix[k][j] * (1 - active[j]) === 0;
        }
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
// Main circuit. N = max leaves (24), depth = 5 levels (24->12->6->3->2->1).
// VARIABLE leaf count `numLeaves` in [1,N]; only the first numLeaves sorted leaves are
// folded, with odd-promotion exactly matching the SDK buildMerkle.
// ---------------------------------------------------------------------------
template DogTagVerification(N, depth) {
    // ---- The 4 consent-context values (dogTagId, purpose, relayer, subject) ----
    // PUBLIC-SIGNAL ORDERING NOTE (verified via build/verification.sym): circom assigns
    // OUTPUT signals the lowest wire indices, so snarkjs lists OUTPUTS *before* public
    // INPUTS in the public-signal vector. The §11.9(e) Solidity verifier hard-requires
    //   pub = [dogTagId, purpose, relayer, subject, nullifier, keyHash, R].
    // To produce that EXACT order we declare ALL SEVEN as OUTPUTS in spec order, taking
    // dogTagId/purpose/relayer/subject as PRIVATE inputs and echoing them to outputs
    // (out* <== in*). The witness for an echoed output is fully constrained by the input,
    // so making them outputs does not weaken soundness.
    signal input dogTagId;   // echoed to outDogTagId
    signal input purpose;    // echoed to outPurpose
    signal input relayer;    // echoed to outRelayer
    signal input subject;    // echoed to outSubject

    // ---- PRIVATE inputs ----
    signal input numLeaves;            // actual leaf count, 1..N (private)
    signal input leafKeyPathHashes[N]; // fieldOf(keyPath) precomputed by prover
    signal input leafTypeTags[N];
    signal input leafSalts[N];         // fieldOf(salt)
    signal input leafValues[N];        // fieldOf(value)
    signal input dogTagIdLeafIndex;    // index (canonical order) of the dogTagId leaf
    signal input sortedLeafHashes[N];  // SDK ascending-sorted leaf order (first numLeaves meaningful)
    signal input perm[N];              // sortedLeafHashes[k] == leafHash[perm[k]]
    signal input dogTagKeyPathField;   // fieldOf("credentialSubject.dogTagId") — bound below
    signal input consentNonce;
    signal input Ax;
    signal input Ay;
    signal input R8x;
    signal input R8y;
    signal input S;

    // ---- OUTPUT signals — declaration order IS the public-signal order ----
    //   [outDogTagId, outPurpose, outRelayer, outSubject, nullifier, keyHash, R]
    signal output outDogTagId;
    signal output outPurpose;
    signal output outRelayer;
    signal output outSubject;
    signal output nullifier;
    signal output keyHash;
    signal output R;

    outDogTagId <== dogTagId;
    outPurpose  <== purpose;
    outRelayer  <== relayer;
    outSubject  <== subject;

    // ===================================================================
    // numLeaves in [1,N]: range-check with a SMALL comparator (numLeaves<=N<=255).
    // ===================================================================
    component nlLo = LessThan(8);   // 1 <= numLeaves  <=>  0 < numLeaves
    nlLo.in[0] <== 0;
    nlLo.in[1] <== numLeaves;
    nlLo.out === 1;
    component nlHi = LessThan(8);   // numLeaves <= N  <=>  numLeaves < N+1
    nlHi.in[0] <== numLeaves;
    nlHi.in[1] <== N + 1;
    nlHi.out === 1;

    // active[i] = (i < numLeaves), boolean. Small comparators (i,numLeaves <= N <= 255).
    signal active[N];
    component activeLt[N];
    for (var i = 0; i < N; i++) {
        activeLt[i] = LessThan(8);
        activeLt[i].in[0] <== i;
        activeLt[i].in[1] <== numLeaves;
        active[i] <== activeLt[i].out; // boolean
    }

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
    // (a.2) sortedLeafHashes is a true permutation of leafHash[] over the active prefix.
    // ===================================================================
    component pa = PermutationApply(N);
    for (var i = 0; i < N; i++) {
        pa.in[i] <== leafHash[i];
        pa.perm[i] <== perm[i];
        pa.active[i] <== active[i];
    }
    for (var i = 0; i < N; i++) {
        sortedLeafHashes[i] === pa.out[i]; // bind prover-supplied sorted array to the permutation
    }

    // ===================================================================
    // (a.3) sortedLeafHashes strictly ascending over the ACTIVE prefix [0,numLeaves).
    //       For each adjacent pair (i,i+1) that is BOTH active (i+1<numLeaves) we force
    //       sortedLeafHashes[i] < sortedLeafHashes[i+1]. Strict LessThan also rejects
    //       duplicate leaves, matching a set. Inactive pairs are unconstrained.
    // ===================================================================
    signal pairActive[N - 1]; // pairActive[i] = (i+1 < numLeaves)
    component asc[N - 1];
    for (var i = 0; i < N - 1; i++) {
        pairActive[i] <== active[i + 1]; // active[i+1] = (i+1 < numLeaves)
        asc[i] = LessThanField();
        asc[i].a <== sortedLeafHashes[i];
        asc[i].b <== sortedLeafHashes[i + 1];
        // require ascending only when both members of the pair are active:
        // pairActive * (1 - asc.out) == 0  <=>  pairActive=1 -> asc.out=1.
        pairActive[i] * (1 - asc[i].out) === 0;
    }

    // ===================================================================
    // (a.4) VARIABLE-COUNT fold with ODD-PROMOTION (matches SDK buildMerkle).
    //
    //   level0 = sortedLeafHashes (length N; only first cnt[0]=numLeaves meaningful).
    //   cnt[0] = numLeaves;  cnt[l+1] = (cnt[l] + 1) >> 1.
    //   For level l, next-slot k:
    //       hasPair_k = (2k+1 <  cnt[l])   -> parent = hashNode(level[2k], level[2k+1])
    //       promote_k = (2k+1 == cnt[l])   -> next   = level[2k] (lone odd node, unchanged)
    //       next[k]   = hasPair_k ? paired_k : (promote_k ? level[2k] : 0)
    //   R = level[depth][0]. For numLeaves==1: cnt stays 1 at every level; slot 0 promotes
    //   each level -> the single leaf passes through unchanged.
    // ===================================================================
    // maxlen per level: maxlen[0]=N; maxlen[l+1]=ceil(maxlen[l]/2). Compile-time.
    var maxlen[6];
    maxlen[0] = N;
    for (var l = 0; l < depth; l++) { maxlen[l + 1] = (maxlen[l] + 1) \ 2; }

    // cnt[l] as signals. cnt[l+1] = floor((cnt[l]+1)/2). Prover supplies the quotient via
    // a boolean parity bit and we constrain it; numLeaves<=N so values are tiny (<=24).
    signal cnt[depth + 1];
    cnt[0] <== numLeaves;
    signal cntParity[depth]; // boolean: (cnt[l]+1) is odd? i.e. cnt[l] even -> +1 odd. We derive lo bit of (cnt[l]+1).
    // We compute q = (cnt[l]+1) >> 1 and r = (cnt[l]+1) & 1, with cnt[l]+1 = 2q + r, r in {0,1}.
    // r is the prover-free low bit of (cnt[l]+1). Use Num2Bits on (cnt[l]+1) bounded to 6 bits
    // (cnt<=24 -> cnt+1<=25 < 64). q = sum of bits[1..] ; r = bits[0]. cnt[l+1] = q.
    component cntBits[depth];
    for (var l = 0; l < depth; l++) {
        cntBits[l] = Num2Bits(6); // cnt[l]+1 <= 25 < 64
        cntBits[l].in <== cnt[l] + 1;
        // q = (cnt[l]+1) >> 1 = sum_{b=1..5} bit[b] * 2^(b-1)
        var q = 0;
        for (var b = 1; b < 6; b++) { q += cntBits[l].out[b] * (1 << (b - 1)); }
        cnt[l + 1] <== q;
        cntParity[l] <== cntBits[l].out[0]; // (cnt[l]+1) low bit (unused but documents intent)
    }

    // level storage. level[0] = sortedLeafHashes. We size each level to maxlen[l].
    signal level[depth + 1][N]; // over-allocate width N; use maxlen[l] entries.
    for (var i = 0; i < N; i++) { level[0][i] <== sortedLeafHashes[i]; }

    // Per-level fold with promotion selectors.
    // Count node-hash components: sum over levels of next-slot count = sum maxlen[l+1].
    var totalNodes = 0;
    for (var l = 0; l < depth; l++) { totalNodes += maxlen[l + 1]; }
    component nh[totalNodes];
    // selectors: hasPair_k = (2k+1 < cnt[l]); promote_k = (2k+1 == cnt[l]).
    component hasPairLt[totalNodes];
    component promoteEq[totalNodes];
    signal hasPair[totalNodes];
    signal promote[totalNodes];
    signal pairedVal[totalNodes];
    signal pairedTerm[totalNodes];
    signal promoteTerm[totalNodes];
    var nodeIdx = 0;
    for (var l = 0; l < depth; l++) {
        var nextLen = maxlen[l + 1];
        for (var k = 0; k < nextLen; k++) {
            // hasPair_k = (2k+1 < cnt[l])  ->  both children real.
            hasPairLt[nodeIdx] = LessThan(8); // 2k+1 <= 2*(N-1)+1 < 256
            hasPairLt[nodeIdx].in[0] <== 2 * k + 1;
            hasPairLt[nodeIdx].in[1] <== cnt[l];
            hasPair[nodeIdx] <== hasPairLt[nodeIdx].out;
            // promote_k = (2k+1 == cnt[l])  ->  lone odd node at index 2k.
            promoteEq[nodeIdx] = IsEqual();
            promoteEq[nodeIdx].in[0] <== 2 * k + 1;
            promoteEq[nodeIdx].in[1] <== cnt[l];
            promote[nodeIdx] <== promoteEq[nodeIdx].out;

            // paired_k = hashNode(level[2k], level[2k+1]). Always computed (cheap relative
            // to selecting); only used when hasPair_k. Inactive slots feed 0/0 -> harmless.
            nh[nodeIdx] = NodeHash();
            nh[nodeIdx].a <== level[l][2 * k];
            nh[nodeIdx].b <== level[l][2 * k + 1];
            pairedVal[nodeIdx] <== nh[nodeIdx].out;

            // next[k] = hasPair ? paired : (promote ? level[2k] : 0)
            //         = hasPair*paired + promote*level[2k]   (hasPair,promote mutually excl.)
            // Split into two quadratic terms (each <==) then sum (linear).
            pairedTerm[nodeIdx]  <== hasPair[nodeIdx] * pairedVal[nodeIdx];
            promoteTerm[nodeIdx] <== promote[nodeIdx] * level[l][2 * k];
            level[l + 1][k] <== pairedTerm[nodeIdx] + promoteTerm[nodeIdx];
            nodeIdx++;
        }
        // zero-fill the unused tail of level[l+1] (slots >= nextLen) so they are defined.
        for (var k = nextLen; k < N; k++) { level[l + 1][k] <== 0; }
    }
    R <== level[depth][0];

    // ===================================================================
    // (b) dogTagId leaf binding: value == dogTagId, index in [0,numLeaves), keyPath bound.
    // ===================================================================
    // Range-check index in [0, numLeaves): require active[dogTagIdLeafIndex]==1 via the
    // one-hot selector below combined with a direct index<numLeaves comparator.
    component idxLt = LessThan(8); // numLeaves <= N <= 255
    idxLt.in[0] <== dogTagIdLeafIndex;
    idxLt.in[1] <== numLeaves;
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

// PRODUCTION instantiation: N=24 max leaves, depth=5 (24->12->6->3->2->1), variable numLeaves.
// No `public [...]` list: ALL public signals are OUTPUTS (declared in spec order), so the
// snarkjs public-signal vector is exactly [dogTagId, purpose, relayer, subject, nullifier,
// keyHash, R]. ALL named inputs are therefore PRIVATE.
component main = DogTagVerification(24, 5);
