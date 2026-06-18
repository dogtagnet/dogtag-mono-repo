pragma circom 2.1.9;

// Reference-of-record test harness: circomlib Poseidon at every arity the DogTag
// commitment uses. nInputs = {2,3,5,6} -> circomlib state width t = {3,4,6,7}.
//   2 in (t=3): bytesToField fold + the canonical anchor poseidon([1,2])
//   3 in (t=4): Merkle node   Poseidon(DS_NODE, lo, hi)
//   5 in (t=6): leaf          Poseidon(DS_LEAF, kp, salt, tag, val)
//   6 in (t=7): nullifier     Poseidon(DS_NULLIFIER, dogTagId, purpose, relayer, subject, nonce)
include "poseidon.circom";

template PoseidonN(n) {
    signal input in[n];
    signal output out;
    component h = Poseidon(n);
    for (var i = 0; i < n; i++) { h.inputs[i] <== in[i]; }
    out <== h.out;
}
