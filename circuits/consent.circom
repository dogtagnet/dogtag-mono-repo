pragma circom 2.1.9;

// DogTagConsent — Level-B owner-unlinkable consent circuit (redesign milestone M2).
//
// Proves, in zero-knowledge and revealing NOTHING about the pet owner, that the HIDDEN owner
// consented to a DISCLOSED relayer for a DISCLOSED purpose. The owner's wallet address, the
// owner's BabyJubJub consent pubkey, and the owner's nullifier secret are PRIVATE leaves of the
// per-tag Merkle tree; only their membership in the public root R is proven.
//
// This SUPERSEDES verification.circom (Level-A), which exposed `subject` (the owner) and
// `keyHash` as public signals. verification.circom is LEFT UNTOUCHED (frozen dev zkey).
//
// PUBLIC SIGNAL VECTOR (order is load-bearing for M4's recordVerificationZK calldata):
//   [ dogTagId, purpose, relayer, nullifier, R, recordType, deadline ]
// NO owner/subject, NO keyHash. (snarkjs lists OUTPUT signals first in declaration order; we
// declare ALL SEVEN public signals as OUTPUTS in this exact order — see the note in the template.)
//
// PROVES (level-b-spec.md §"Verify (M2 circuit + M4 registry)"):
//   1. owner-address leaf  ∈ R   (inclusion via the M1 sorted-hash fold, path is a PRIVATE input)
//   2. consent-key  leaf   ∈ R   (leaf value = Poseidon2(Ax,Ay), binding the in-tree key to the signer)
//   3. owner-secret leaf   ∈ R
//   4. the consent-key (BabyJubJub) SIGNED  M = Poseidon5(dogTagId, purpose, relayer, deadline, nonce)
//   5. nullifier = Poseidon6(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce)
//
// dogTagId <-> R is bound ON-CHAIN (profileRoot(dogTagId) == R), NOT in this circuit.
//
// RESERVED-LEAF SCHEMA — the three owner-control leaves are PINNED to fixed keyPath+typeTag
// constants (below). Pinning is LOAD-BEARING against keyPath SUBSTITUTION: if keyPath were a free
// input a prover could point the owner-secret inclusion proof at ANY other in-tree leaf (e.g. a
// disclosable attribute), set `ownerSecret` to that leaf's value, and mint a SECOND valid nullifier
// for the same signed consent - breaking replay protection (D5).
// Pinning does NOT defend against a DUPLICATE reserved triple. This template verifies three
// INDEPENDENT leaf inclusions, and neither the circuit nor the tree construction enforces
// per-keyPath uniqueness, so "ownerSecret / (Ax,Ay) are collision-bound to the genuine owner leaves"
// holds only under a PRECONDITION on R: exactly one reserved triple exists in it. That precondition
// is the normative P-e invariant - see docs/DELEGATION.md section 5.
// M5 issuance MUST build these three leaves with exactly these keyPath+typeTag values, and exactly
// one triple per tree (see AGENTS.md).
//
// CEREMONY: the real testnet-grade phase-2 ceremony (M3) is DONE. The committed production VK/zkey
// (build/consent_verification_key.json, build/consent_final.zkey) + the Groth16VerifierConsent verifier
// are its output - see docs/CEREMONY_TRANSCRIPT.consent.md. scripts/setup-consent.sh remains the
// DEV/throwaway setup (NON-PRODUCTION); the production key is regenerated with scripts/ceremony-consent.sh.

include "poseidon.circom";
include "bitify.circom";
include "eddsaposeidon.circom";
include "lib/merkle_inclusion.circom";

// Domain-separation tags (field.ts / poseidon.rs): DS_LEAF=1, DS_NODE=2, DS_NULLIFIER=4.

template DogTagConsent(depth) {
    // === Reserved owner-leaf schema constants (PINNED — see header + AGENTS.md) ===
    //   fieldOfKeyPath(s) = bytesToField(nfc(s))  (leaf.ts). Recomputed & asserted in test-consent.mjs.
    var KP_OWNER_ADDRESS = 20593649144631820416234157596070441856608371338897391424937040814759273231214;
    var KP_CONSENT_KEY   = 7822071287675030884271946396254564996644565056920260282559292033992393086992;
    var KP_OWNER_SECRET  = 11172449362271989869407103131203633198993612309996015027844083581837121079156;
    // typeTag for all three reserved leaves = TypeTag.Bytes (5). Pinned for determinism.
    var TAG_OWNER_ADDRESS = 5;
    var TAG_CONSENT_KEY   = 5;
    var TAG_OWNER_SECRET  = 5;

    // ---- PRIVATE inputs echoed to public outputs (see ordering note) ----
    signal input dogTagId;     // -> outDogTagId
    signal input purpose;      // -> outPurpose
    signal input relayer;      // -> outRelayer (uint160 address; range-checked)
    signal input recordType;   // -> outRecordType (PROVER-ASSERTED label, NOT consent-signed — see AGENTS.md)
    signal input deadline;     // -> outDeadline (consent expiry; signed inside M)

    // ---- PRIVATE consent + owner inputs ----
    signal input consentNonce;
    signal input ownerAddress;  // owner-address leaf value slot (opaque field; app-supplied)
    signal input ownerSecret;   // owner-secret leaf value slot + nullifier secret (opaque field)
    signal input Ax;            // consent-key BabyJubJub pubkey x
    signal input Ay;            // consent-key BabyJubJub pubkey y
    signal input R8x;           // EdDSA signature R8.x
    signal input R8y;           // EdDSA signature R8.y
    signal input S;             // EdDSA signature scalar
    signal input ownerSalt;     // fieldFromScalarBytes(16B salt) — owner-address leaf
    signal input keySalt;       // fieldFromScalarBytes(16B salt) — consent-key leaf
    signal input secretSalt;    // fieldFromScalarBytes(16B salt) — owner-secret leaf

    // ---- PRIVATE inclusion paths (front-packed siblings + length; see MerkleInclusion) ----
    signal input ownerSiblings[depth];
    signal input ownerPathLen;
    signal input keySiblings[depth];
    signal input keyPathLen;
    signal input secretSiblings[depth];
    signal input secretPathLen;

    // ---- OUTPUT signals — declaration order IS the public-signal order ----
    //   [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
    // circom assigns OUTPUT signals the lowest wire indices, so snarkjs lists them (in this
    // declaration order) BEFORE any public input. We declare ALL seven as outputs to fix the
    // vector exactly; echoed outputs (out* <== in*) are fully constrained by their input, so
    // exposing them as outputs does not weaken soundness. (Verified via build/consent.sym.)
    signal output outDogTagId;
    signal output outPurpose;
    signal output outRelayer;
    signal output nullifier;
    signal output R;
    signal output outRecordType;
    signal output outDeadline;

    outDogTagId   <== dogTagId;
    outPurpose    <== purpose;
    outRelayer    <== relayer;
    outRecordType <== recordType;
    outDeadline   <== deadline;

    // ===================================================================
    // consent-key leaf value = Poseidon2(Ax, Ay) — binds the in-tree key to the EdDSA signer.
    // ===================================================================
    component kh = Poseidon(2);
    kh.inputs[0] <== Ax;
    kh.inputs[1] <== Ay;
    signal keyHash <== kh.out;

    // ===================================================================
    // (1..3) Recompute the three reserved leaf hashes = Poseidon5(DS_LEAF, kp, salt, tag, value)
    //        with PINNED keyPath+typeTag, then fold each up its inclusion path to a root.
    // ===================================================================
    component lhOwner = Poseidon(5);
    lhOwner.inputs[0] <== 1;                 // DS_LEAF
    lhOwner.inputs[1] <== KP_OWNER_ADDRESS;
    lhOwner.inputs[2] <== ownerSalt;
    lhOwner.inputs[3] <== TAG_OWNER_ADDRESS;
    lhOwner.inputs[4] <== ownerAddress;

    component lhKey = Poseidon(5);
    lhKey.inputs[0] <== 1;                    // DS_LEAF
    lhKey.inputs[1] <== KP_CONSENT_KEY;
    lhKey.inputs[2] <== keySalt;
    lhKey.inputs[3] <== TAG_CONSENT_KEY;
    lhKey.inputs[4] <== keyHash;

    component lhSecret = Poseidon(5);
    lhSecret.inputs[0] <== 1;                 // DS_LEAF
    lhSecret.inputs[1] <== KP_OWNER_SECRET;
    lhSecret.inputs[2] <== secretSalt;
    lhSecret.inputs[3] <== TAG_OWNER_SECRET;
    lhSecret.inputs[4] <== ownerSecret;

    component incOwner = MerkleInclusion(depth);
    incOwner.leaf <== lhOwner.out;
    incOwner.pathLen <== ownerPathLen;
    for (var i = 0; i < depth; i++) { incOwner.siblings[i] <== ownerSiblings[i]; }

    component incKey = MerkleInclusion(depth);
    incKey.leaf <== lhKey.out;
    incKey.pathLen <== keyPathLen;
    for (var i = 0; i < depth; i++) { incKey.siblings[i] <== keySiblings[i]; }

    component incSecret = MerkleInclusion(depth);
    incSecret.leaf <== lhSecret.out;
    incSecret.pathLen <== secretPathLen;
    for (var i = 0; i < depth; i++) { incSecret.siblings[i] <== secretSiblings[i]; }

    // All three leaves live in the SAME per-tag tree, so all three folds MUST reach one root R.
    R <== incOwner.root;
    incKey.root === R;
    incSecret.root === R;

    // ===================================================================
    // (4) EdDSA-BabyJubjub verify over M = Poseidon5(dogTagId, purpose, relayer, deadline, nonce).
    //     No DS tag (matches level-b-spec.md §Verify #4 and the Level-A no-tag convention).
    //     NOTE (M3 hygiene): M is Poseidon5 and so shares arity+first-slot with the leaf hash
    //     Poseidon5(DS_LEAF=1, ...) when dogTagId==1. No exploit exists (EdDSA needs the key,
    //     leaves aren't signed); the VK-freeze checkpoint reviewed M's preimage structure and
    //     confirmed no exploit, and the VK is now FROZEN against this circuit. See AGENTS.md.
    // ===================================================================
    component mh = Poseidon(5);
    mh.inputs[0] <== dogTagId;
    mh.inputs[1] <== purpose;
    mh.inputs[2] <== relayer;
    mh.inputs[3] <== deadline;
    mh.inputs[4] <== consentNonce;

    component sig = EdDSAPoseidonVerifier();
    sig.enabled <== 1;
    sig.Ax  <== Ax;
    sig.Ay  <== Ay;
    sig.S   <== S;
    sig.R8x <== R8x;
    sig.R8y <== R8y;
    sig.M   <== mh.out;

    // ===================================================================
    // (5) nullifier = Poseidon6(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce).
    //     ownerSecret is the SAME field proven ∈ R above, so the nullifier is bound to the genuine
    //     owner-secret leaf (see reserved-leaf soundness note). Scope: per (dogTagId,purpose,relayer)
    //     + nonce (D5) — replaying one signed consent => same nullifier (rejected on-chain); a fresh
    //     nonce => new nullifier (allowed).
    // ===================================================================
    component nf = Poseidon(6);
    nf.inputs[0] <== 4;            // DS_NULLIFIER
    nf.inputs[1] <== ownerSecret;
    nf.inputs[2] <== dogTagId;
    nf.inputs[3] <== purpose;
    nf.inputs[4] <== relayer;
    nf.inputs[5] <== consentNonce;
    nullifier <== nf.out;

    // ===================================================================
    // range-check relayer < 2^160 (public address; compared on-chain by the registry).
    // ===================================================================
    component relayerBits = Num2Bits(160);
    relayerBits.in <== relayer;
}

// PRODUCTION shape: depth = 6 (inclusion paths up to 6 siblings => trees up to 2^6 = 64 leaves;
// realistic Level-B tree is 3 owner leaves + credential attributes, well under 64). All named
// signals are PRIVATE inputs; the seven public signals are the declared OUTPUTS above.
component main = DogTagConsent(6);
