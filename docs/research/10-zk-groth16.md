# 10 — ZK / Groth16: Groomer-attested verification proof

> Status: v1 design. Companion to [`architecture.md`](../architecture.md) (§3 keccak salted-leaf
> Merkle standard, §4 contracts, §5 verification) and [`implementation.md`](../implementation.md)
> (§1 `hashLeaf`/`buildMerkle`, §11.x normative). Chain: **ROAX** (EVM, chainId **135**, gas token
> PLASMA). Curve for Groth16: **BN254 / alt_bn128** (the only pairing precompile available on an EVM
> chain). This document is **forward-looking / optional** — v1 verification is the three off-chain
> pillars (§5); this is the on-chain *attested-verification* path a groomer/relayer can opt into.

---

## 0. Problem framing

Today a groomer that imports a customer's vaccination credential verifies it **off-chain** (the three
authenticity pillars — integrity, issuance, identity — §5/§11.3) and leaves **no on-chain trace**. The
goal here: let a **groomer (verifier/relayer)** record **on-chain** that they validated a user's
credential (vaccination cert or DogTag SBT profile), **gated by the user's signed consent**, optionally
as a **Groth16 proof** so that the **raw credential data never goes on-chain**. The groomer's **Rust
backend** generates the proof; an EVM verifier contract checks it and the
`VerificationRegistry` records an event.

The three hard facts that drive every decision below:

1. The issuance Merkle root is **keccak256**-based and **EVM-anchored** (`DogTagIssuer.isValid(root)` —
   §4.4). We **must not** change that — it is the OpenAttestation-style trust anchor verified by
   mobile, portals, and Solidity alike.
2. **keccak256 inside Groth16 is ~151k constraints per hash** ([Electron-Labs](https://github.com/Electron-Labs/keccak-circom),
   [vocdoni](https://github.com/vocdoni/keccak256-circom)). A real credential tree (10–30 leaves) is
   ~20–40 keccak invocations → **3–6M constraints just for the tree**.
3. **secp256k1 ECDSA inside Groth16 is ~150k–1.5M constraints** depending on implementation
   ([circom-ecdsa](https://github.com/privacy-scaling-explorations/circom-ecdsa-p256),
   [BigWhaleLabs](https://blog.bigwhalelabs.com/private-ecdsa-verification-using-zk/)).

A naïve "prove keccak Merkle membership + verify an ECDSA consent signature" circuit is **5M–8M+
constraints** — minutes of proving, gigabytes of proving key, and a brutal phase-2 ceremony. **It is
not viable.** The whole design below is about avoiding both keccak-in-circuit and ECDSA-in-circuit
while keeping the keccak issuance root intact for EVM verification.

---

## 1. Circuit statement & signals

### 1.1 What the circuit proves

A single Groth16 proof attests, in zero knowledge over the private credential, that:

- **(a) Membership / well-formedness.** The private credential leaves hash (under the ZK-friendly
  commitment, §2) to a root **R_zk**, and that **R_zk is bound 1:1 to the EVM-anchored keccak root
  R_kec** that the issuer signed on-chain (binding mechanism in §2.3). I.e. "these private fields are
  exactly the fields the issuer committed to."
- **(b) Field equality.** The credential's `credentialSubject.dogTagId` leaf equals the **public**
  `dogTagId`. (The verifier learns *which pet* without learning any other field.)
- **(c) Consent.** The user (`userWallet`) authorized **this** groomer to record **this** verification:
  the circuit verifies an **EdDSA-BabyJubjub** signature (§2.4) by the user's ZK consent key over the
  message `H(dogTagId ‖ groomerRelayer ‖ R_zk ‖ consentNonce)`, and proves that ZK consent key is
  **bound to `userWallet`** (binding in §2.5).
- **(d) Nullifier.** The circuit derives and **exposes** a `nullifier = Poseidon(consentNonce,
  userWallet, dogTagId, groomerRelayer)` (§5); the registry rejects a repeated nullifier → no replay.

### 1.2 Signals

```
PUBLIC (5 signals — minimal; gas ≈ 181k + 6k·ℓ, so keep ℓ small):
  dogTagId          // uint256, the SBT tokenId being attested
  groomerRelayer    // address (as field element), the verifier/relayer recording the attestation
  userWalletAddress // address (as field element), whose consent gated this
  nullifier         // field element, derived in-circuit (replay guard) — OUTPUT signal
  Rzk               // field element, the ZK commitment root (see §1.3 — public, cross-checked on-chain)

PRIVATE (witness):
  leafValues[N]         // canonical field values of the credential
  leafSalts[N]          // per-field 16-byte salts (§3.2)
  leafKeyPathHashes[N]  // Poseidon(keyPath) — keyPath is fixed/known, hashed to a field
  leafTypeTags[N]       // uint8 per field
  merklePathElements[depth]  // ZK-tree sibling hashes (Poseidon)
  merklePathIndices[depth]   // 0/1 left/right bits (ZK tree is NOT commutative — see §2.2)
  dogTagIdLeafIndex     // which leaf is the dogTagId field
  consentNonce          // user's EIP-712 nonce (binds off-chain consent record — §5)
  consentSig            // EdDSA-BabyJubjub {R8x, R8y, S} over the consent message
  userPubKey            // {Ax, Ay} BabyJubjub pubkey of the user's ZK consent key
  userKeyBinding        // proof element binding userPubKey -> userWalletAddress (§2.5)
```

### 1.3 Public vs private decision: is **R** a public input?

**Yes — `Rzk` is a public input, and is cross-checked on-chain** rather than proven-in-circuit. Two
sub-decisions:

- **Do NOT prove `isValid(R)` inside the circuit.** Reaching into the issuer contract's storage from a
  circuit would require either an MPT/storage proof (huge) or trusting an oracle. Instead, the
  **`VerificationRegistry` reads `DogTagIssuer.isValid(R_kec)` on-chain at verify time** (a cheap
  `STATICCALL`, §6) and the circuit only proves the credential hashes to `R_zk` which is bound to
  `R_kec`. This keeps the circuit small and reuses the existing, audited issuance gate verbatim.
- **`R_zk` is public** so the registry can (i) recompute/lookup the bound `R_kec` and call `isValid`,
  and (ii) ensure the proof is about a *real, issued* credential — not a fabricated tree. The mapping
  `R_zk → R_kec` is published at issuance (§2.3); the registry stores it or accepts both roots as
  public inputs and checks the binding contract.

> **Privacy note:** exposing `R_zk` and `dogTagId` is acceptable — `dogTagId` is a non-personal random
> id (§11.7(a)), and `R_zk` is a salted commitment (the salts stay private). No raw field, no owner
> PII, no microchip, and no individual leaf hash ever leaves the witness. This satisfies "raw
> credential data must NOT go on chain."

---

## 2. The keccak256 problem (the critical decision)

### 2.1 The three options

| Option | Mechanism | Constraint ballpark | Verdict |
|---|---|---|---|
| **(i) Prove over keccak directly** | keccak256 Merkle in-circuit (matches §3.4 exactly) | **~151k constraints / hash** × ~20–40 hashes = **3–6M** for the tree alone, **+ECDSA** | ❌ Rejected — multi-minute proofs, >2GB zkey, infeasible phase-2 |
| **(ii) Parallel Poseidon commitment at issuance** | Issuer *also* computes a Poseidon Merkle root `R_zk` over the same leaves; anchors/links it to the keccak `R_kec` | **~210 constraints / Poseidon hash** × ~20–40 = **5k–10k** for the tree | ✅ **Recommended (with iii)** |
| **(iii) EdDSA-BabyJubjub consent** instead of ECDSA | User signs consent with a BabyJubjub EdDSA key, not their secp256k1 wallet key | **~4,018 constraints** for one EdDSA verify (circomlib) vs **~150k–1.5M** for ECDSA | ✅ **Recommended (with ii)** |

Sources: keccak ~151k ([Electron-Labs](https://github.com/Electron-Labs/keccak-circom),
[vocdoni](https://github.com/vocdoni/keccak256-circom)); Poseidon ~210 / EdDSA-BabyJubjub ~4,018
(circomlib, [iden3 EdDSA](https://iden3-docs.readthedocs.io/en/latest/iden3_repos/research/publications/zkproof-standards-workshop-2/ed-dsa/ed-dsa.html));
ECDSA-secp256k1 ~150k+ ([circom-ecdsa](https://github.com/privacy-scaling-explorations/circom-ecdsa-p256),
[BigWhaleLabs](https://blog.bigwhalelabs.com/private-ecdsa-verification-using-zk/)).

### 2.2 Recommendation: **(ii) + (iii)** — dual-root issuance + EdDSA consent

Add a **parallel Poseidon commitment to the issuance flow** purely for the ZK path, and use
**EdDSA-BabyJubjub** for the consent signature. Net circuit cost:

```
Poseidon Merkle membership  ~5k–10k   (tree of ~16–32 leaves, Poseidon 210/hash, with index bits)
dogTagId leaf equality      ~few hundred
EdDSA-BabyJubjub verify      ~4,018
consent message Poseidon    ~210
nullifier Poseidon          ~210
userPubKey -> wallet binding ~210 (Poseidon binding) — see §2.5
-------------------------------------------------------------
TOTAL                       ~12k–18k constraints  ✅  (sub-second proving)
```

That is **~300–500× smaller** than the keccak+ECDSA naïve circuit, and brings proving to **well under a
second** on the groomer's Rust backend, with a proving key in the low-MB range and a tractable phase-2.

> **ZK-tree note:** the in-circuit Poseidon Merkle tree uses **ordered (index-bit) hashing**, NOT the
> commutative `sortPair` of §3.4. Commutative sorting in-circuit costs comparators per level and adds
> ambiguity; instead the issuer fixes leaf order deterministically (e.g. sort the Poseidon leaf hashes
> ascending once at issuance, then the path is index-addressed). The ZK tree is an **independent
> structure** from the keccak tree — they share the same *leaf set*, not the same node hashing.

### 2.3 Reconciling with the existing keccak issuance root (the key move)

The keccak `R_kec` **stays exactly as specified in §3** and remains the *only* thing the off-chain
verifier (§5) and `DogTagIssuer.isValid` care about. We add a **parallel commitment** computed by the
**same SDK at wrap time**:

```
At issuance (wrapDocument, §1.4 — extended):
  for each field f:  leaf_kec[f] = keccak256(0x00 ‖ len(keyPath) ‖ keyPath ‖ ... ‖ value)   // §3.3, UNCHANGED
                     leaf_zk[f]  = Poseidon(Poseidon(keyPathHash, typeTag), saltField, valueField)  // NEW, ZK leaf
  R_kec = buildMerkle(leaf_kec[])    // §3.4 commutative keccak tree — anchored on-chain, UNCHANGED
  R_zk  = poseidonMerkle(sort(leaf_zk[]))   // NEW ordered Poseidon tree
  // BIND the two roots so a ZK proof about R_zk is provably about the SAME issued credential:
  bindingLeaf = keccak256(0x02 ‖ R_kec ‖ R_zk)     // 0x02 = new BINDING domain separator
```

How the binding is anchored (pick one; **A is recommended** as least-invasive):

- **A. Event-only (recommended for v1-opt-in).** When issuing, the issuer emits an extra event
  `ZkCommitment(R_kec, R_zk)` from a small `ZkCommitmentRegistry` (or extends `DogTagIssuer`). The
  `VerificationRegistry` (§6), given public `R_zk`, looks up the bound `R_kec` (from the indexed event
  / a `mapping(bytes32 R_zk => bytes32 R_kec)`) and calls `DogTagIssuer.isValid(R_kec)`. The circuit
  only proves "leaves → R_zk"; the chain proves "R_zk ↔ R_kec ↔ issued". **No change to the keccak
  tree, no change to `isValid`.**
- **B. Anchor R_zk too.** Issue *both* roots into the issuer (`issue(R_kec)` + a parallel
  `issueZk(R_zk)`); registry checks `isValidZk(R_zk)`. Cleaner provenance, but a contract change and a
  second SSTORE per issuance.
- **C. In-circuit keccak of the binding only.** Prove `keccak256(0x02‖R_kec‖R_zk)` in-circuit so
  `R_kec` can be a public input and checked against `isValid` with **no registry mapping**. Cost: **one
  keccak (~151k constraints)**. This is the *only* place keccak is even arguably worth it, and even
  then **A avoids it entirely** — so **C is documented but not recommended**.

**Decision: (ii) Poseidon parallel commitment + (iii) EdDSA-BabyJubjub consent, bound to the keccak
root via option A (event/mapping lookup).** The keccak `R_kec` and `isValid` are untouched; the SDK
gains a `poseidonMerkle` / `hashLeafZk` alongside `hashLeaf`/`buildMerkle` (both TS and Rust, asserted
in the shared `testvectors.json`).

### 2.4 Why EdDSA-BabyJubjub for consent (not ECDSA)

The consent signature is **a new, ZK-purposed credential**, not an existing on-chain artifact — so we
are free to choose the curve. BabyJubjub is the embedded curve of BN254; EdDSA over it verifies in
**~4,018 constraints** vs **~150k–1.5M** for secp256k1 ECDSA. The user's mobile wallet (§6.4) holds an
**MPC/BIP-39 secp256k1 key for the chain**, and **additionally derives a BabyJubjub EdDSA consent
key** (deterministically from the same seed, distinct derivation/domain). The EIP-712 consent UX
(§11.7(f)) is replaced/augmented by an in-app "approve this groomer to record verification" that
produces an EdDSA signature over the Poseidon consent message. This is the single most important cost
decision after the Poseidon tree.

### 2.5 Binding the EdDSA consent key to `userWallet`

The public signal is `userWalletAddress` (the secp256k1 EVM address), but the circuit verifies an
**EdDSA-BabyJubjub** signature. We must prove the BabyJubjub `userPubKey` belongs to the same user:

- **Recommended: registry-attested binding.** At wallet setup, the user (once) signs a secp256k1
  message binding `{babyJubPubKey}` to their wallet, and the central backend records
  `mapping(address userWallet => bytes32 babyJubPubKeyHash)` in a small on-chain `ConsentKeyRegistry`
  (or off-chain registry the VerificationRegistry trusts). The circuit exposes
  `Poseidon(userPubKey)` and the registry checks it equals the registered hash for `userWalletAddress`.
  This keeps **secp256k1 entirely out of the circuit** (the one-time bind is verified on-chain by
  `ecrecover`, which is a cheap precompile).
- Alternative (avoid): verify the secp256k1 binding signature in-circuit — reintroduces ~150k+
  constraints. Not worth it for a one-time binding.

---

## 3. Toolchain

### 3.1 Circuit + setup + verifier

- **circom 2.x** for the circuit; **circomlib** for `Poseidon`, `EdDSAPoseidonVerifier`, `comparators`,
  and Merkle/`SMTVerifier` gadgets. EdDSA-BabyJubjub verify ≈ 4,018 constraints; Poseidon ≈ 210/hash.
- **snarkjs (Groth16)** for setup and proof tooling, and to **export the Solidity verifier**
  (`snarkjs zkey export solidityverifier`) — the contract exposes
  `verifyProof(uint[2] a, uint[2][2] b, uint[2] c, uint[ℓ] input) view returns (bool)`
  ([snarkjs](https://github.com/iden3/snarkjs)). Curve = **BN254/alt_bn128** (mandatory — it is the EVM
  pairing precompile, EIP-196/197/1108).

### 3.2 Rust groomer-backend proving stack — comparison & recommendation

| Stack | Language | Witness gen | Perf | Maintenance / portability |
|---|---|---|---|---|
| **`ark-circom` + `ark-groth16` (`circom-compat`)** | Rust | **integrated** (loads `.wasm`/`.r1cs`, computes witness in-process) | "Fast" — slower than rapidsnark but ample for a ~15k-constraint circuit (sub-second) | **Pure Rust, no native deps (no gmp/cmake/nasm); works on all platforms; arkworks-maintained** |
| **rapidsnark (subprocess / FFI)** | C++ | **none** — needs `rust-witness`/`witnesscalc` separately | **Fastest (5–10× arkworks), multi-core** | Heavy C++/gmp/cmake/nasm build chain; FFI or subprocess glue; more ops burden |
| **snarkjs as subprocess** | JS/WASM | bundled | **Slowest** | Node runtime in a Rust service; brittle; only good for prototyping/CI |

Sources: [Mopro circom-prover comparison](https://zkmopro.org/blog/circom-comparison/),
[arkworks/circom-compat](https://github.com/arkworks-rs/circom-compat),
[rapidsnark](https://github.com/iden3/rapidsnark).

**Recommendation: `ark-circom` + `ark-groth16`.** For a **~12k–18k-constraint** circuit, arkworks
proves in **well under a second** — rapidsnark's 5–10× edge is irrelevant at this size and not worth
dragging gmp/cmake/nasm and a separate witness calculator into the otherwise-clean Alloy-based Rust
backend (§1.8). It is **pure Rust, integrated witness generation, single binary, cross-platform**, and
the same crate ecosystem the rest of the backend already trusts. Keep `rapidsnark` as a documented
escape hatch *only* if the circuit ever balloons past a few hundred k constraints. snarkjs-subprocess
is for local dev/CI parity only.

> Backend flow: load `verification.r1cs` + `verification.wasm` (or arkworks-native witness) + the
> phase-2 `verification.zkey` once at boot; per request, build the witness from the credential +
> consent sig, call `ark_groth16::create_random_proof`, serialize `(a,b,c,publicInputs)` for the
> Solidity call. Proving keys ship inside the backend image / a mounted volume.

---

## 4. Trusted setup

Groth16 needs a **per-circuit** trusted setup (the toxic-waste problem: anyone who knows the setup
secret can forge proofs; security holds if **≥1** participant destroyed their share).

### 4.1 Plan

1. **Phase 1 — Powers of Tau (circuit-independent): REUSE a public ceremony.** Use the
   **Perpetual Powers of Tau** / **Hermez** ceremony output (Hermez used `2^23` powers; 54+
   contributions) — do **not** run our own phase 1. Pick a `.ptau` with enough powers for ~15k
   constraints (`2^15` ≈ 32k is plenty; we take a published `powersOfTau28_hez_final_15.ptau` or
   larger). Sources: [Perpetual PoT](https://medium.com/coinmonks/announcing-the-perpetual-powers-of-tau-ceremony-to-benefit-all-zk-snark-projects-c3da86af8377),
   [Hermez selection](https://hackmd.io/@4sHVqkbyQnyF63sea5vFOg/S1XuzpJXw),
   [circom docs](https://docs.circom.io/getting-started/proving-circuits/).
2. **`prepare phase2`** on the chosen `.ptau` (snarkjs).
3. **Phase 2 — circuit-specific contribution ceremony.** `snarkjs zkey new` → one or more
   `zkey contribute` rounds → `zkey beacon` (verifiable random beacon as the final contribution) →
   `zkey export verificationkey` and `zkey export solidityverifier`.
4. **Distribute keys.** Publish the final `.zkey` hash + the `verification_key.json` + the verifier
   `.sol` source; ship the proving `.zkey` in the groomer backend image / mounted volume; pin its
   hash in CI. The verifier contract is deployed once to ROAX and address-pinned in config.

### 4.2 Security of a single-party phase 2

A **single-party** phase-2 means **one entity could retain the toxic waste and forge proofs** for this
circuit (a forged proof would falsely assert "a consented, issued credential was verified"). It does
**not** leak any user data (Groth16 zero-knowledge holds regardless), but it breaks **soundness**.

**Mitigations (do all):**
- Run a **multi-contributor phase-2** (≥3 independent parties: protocol multisig members + an external
  contributor), each publishing an attestation; security holds if **any one** is honest.
- End with a **public verifiable-delay/random beacon** contribution (e.g. drand / a future Ethereum
  block hash) so the final transcript is non-grindable.
- Publish the full transcript and let anyone `zkey verify` it against the reused `.ptau`.
- **Blast radius is contained:** this circuit is an **optional attestation path**; the **core trust
  model (the three keccak/DNS/issuance pillars, §5) does not depend on the ZK setup at all**, so even a
  fully compromised setup cannot forge a *credential* — only a spurious "I-verified-it" attestation,
  which the registry's nullifier + on-chain `isValid(R_kec)` re-check still constrains.

---

## 5. Nullifier / replay

```
nullifier = Poseidon(consentNonce, userWalletAddress, dogTagId, groomerRelayer)
```

- **Derived in-circuit, exposed as a public output signal.** The circuit *forces* the nullifier to be
  this exact hash of in-witness values, so a prover cannot choose it freely.
- The `VerificationRegistry` keeps `mapping(bytes32 => bool) usedNullifier`; `recordVerification`
  **reverts if already set**, then sets it. → the **same consent (same nonce) cannot be replayed**, and
  a different `(relayer, dogTagId)` produces a different nullifier (so one consent authorizes exactly
  one (groomer, pet) attestation).
- **Interplay with the EIP-712 / EdDSA consent nonce.** `consentNonce` is the same per-purpose nonce
  the user's consent record carries (mirrors the EIP-712 `nonce` pattern in §4.2 recover / §11.7). The
  user's app increments it per authorization; the off-chain `Consent`/`ConsentReceipt` row (§4.5)
  stores `{purpose:"groomer-verification", nonce, grantedAt, groomerRelayer, dogTagId}`. Withdrawing
  consent off-chain doesn't un-record an already-anchored verification (immutable), but prevents new
  proofs for that nonce. Because the nullifier binds `userWallet`+`dogTagId`+`relayer`+`nonce`, it is
  globally unique and double-recording is impossible.

---

## 6. On-chain verifier integration

### 6.1 Contracts

```solidity
// Generated by: snarkjs zkey export solidityverifier  (BN254; verifyProof is `view`)
interface IGroth16Verifier {
    function verifyProof(
        uint[2] calldata a, uint[2][2] calldata b, uint[2] calldata c,
        uint[5] calldata input            // [dogTagId, groomerRelayer, userWallet, nullifier, Rzk]
    ) external view returns (bool);
}

contract VerificationRegistry {
    IGroth16Verifier public verifier;
    IZkCommitmentRegistry public commitments;   // R_zk -> R_kec  (§2.3 option A)
    mapping(bytes32 => bool) public usedNullifier;

    event CredentialVerified(
        uint256 indexed dogTagId,
        address indexed groomerRelayer,
        address indexed userWallet,
        bytes32 nullifier,
        bytes32 Rzk
    );

    function recordVerification(
        uint[2] calldata a, uint[2][2] calldata b, uint[2] calldata c,
        uint256 dogTagId, address groomerRelayer, address userWallet,
        bytes32 nullifier, bytes32 Rzk
    ) external {
        require(msg.sender == groomerRelayer, "relayer must submit");   // bind tx sender to public signal
        require(!usedNullifier[nullifier], "replay");

        // 1) verify the SNARK over the 5 public signals
        uint[5] memory pub = [uint256(dogTagId), uint256(uint160(groomerRelayer)),
                              uint256(uint160(userWallet)), uint256(nullifier), uint256(Rzk)];
        require(verifier.verifyProof(a, b, c, pub), "bad proof");

        // 2) reconcile to the EVM-anchored keccak root and reuse the existing issuance gate (§4.4)
        bytes32 Rkec = commitments.kecRootFor(Rzk);
        require(Rkec != bytes32(0), "unknown commitment");
        address issuer = commitments.issuerFor(Rzk);        // the DogTagIssuer clone that anchored it
        require(IDogTagIssuer(issuer).isValid(Rkec), "not issued / revoked");

        usedNullifier[nullifier] = true;
        emit CredentialVerified(dogTagId, groomerRelayer, userWallet, nullifier, Rzk);
    }
}
```

Design points:
- The **public signals exactly match the prompt**: `dogTagId`, `groomerRelayer`, `userWalletAddress`
  (plus the derived `nullifier` and `Rzk` needed for replay + issuance reconciliation).
- `require(msg.sender == groomerRelayer)` binds the recorded relayer to the tx sender so a third party
  can't record a verification *as* someone else (the relayer pays PLASMA gas, like the §6 backend
  signing mode).
- **`isValid(R_kec)` is re-checked on-chain** — the circuit never trusts issuance; the chain does, via
  the existing, audited gate. Revoked credentials can't be freshly attested.
- The optional `ConsentKeyRegistry` check (§2.5) can be folded in here too (assert
  `commitments`/`keyRegistry` knows `Poseidon(userPubKey)` for `userWallet`) — or proven via the
  registry-attested binding being part of the public-input set.

### 6.2 Gas

Groth16 on-chain verification ≈ **(181 + 6·ℓ) kgas** for ℓ public inputs
([HackMD/nebra](https://hackmd.io/@nebra-one/ByoMB8Zf6)), dominated by the BN254 pairing precompile
(`45,000 + 34,000·k`, k=4 pairs, post-EIP-1108). With **ℓ = 5** public signals:
**≈ 181k + 30k ≈ 211k gas** for the pairing/verify, **+ ~30–50k** for the `isValid` STATICCALL, the
nullifier SSTORE, and the event. **Total ≈ 240k–270k gas per attestation.** On a low-fee EVM chain
like ROAX (PLASMA gas) this is inexpensive; the groomer relayer pays. **Sub-100k is not achievable for
a standalone Groth16 verify** ([7BlockLabs](https://www.7blocklabs.com/blog/whats-the-cleanest-way-to-optimize-an-on-chain-groth16-verifier-so-each-proof-costs-under-100k-gas))
— would require batching, out of scope for per-visit attestations.

---

## 7. Circom circuit sketch

```circom
pragma circom 2.1.6;
include "circomlib/circuits/poseidon.circom";
include "circomlib/circuits/eddsaposeidon.circom";
include "circomlib/circuits/comparators.circom";
include "circomlib/circuits/mux1.circom";

// Ordered Poseidon Merkle inclusion (NOT commutative — see §2.2)
template PoseidonMerkle(depth) {
    signal input leaf;
    signal input pathElements[depth];
    signal input pathIndices[depth];   // 0/1
    signal output root;
    component h[depth]; component mux[depth];
    signal cur[depth+1]; cur[0] <== leaf;
    for (var i = 0; i < depth; i++) {
        mux[i] = MultiMux1(2);
        mux[i].c[0][0] <== cur[i];           mux[i].c[1][0] <== pathElements[i];
        mux[i].c[0][1] <== pathElements[i];  mux[i].c[1][1] <== cur[i];
        mux[i].s <== pathIndices[i];
        h[i] = Poseidon(2);
        h[i].inputs[0] <== mux[i].out[0];
        h[i].inputs[1] <== mux[i].out[1];
        cur[i+1] <== h[i].out;
    }
    root <== cur[depth];
}

template DogTagVerification(N, depth) {
    // ---- public ----
    signal input  dogTagId;
    signal input  groomerRelayer;
    signal input  userWalletAddress;
    signal output nullifier;
    signal output Rzk;

    // ---- private ----
    signal input leafKeyPathHashes[N];
    signal input leafTypeTags[N];
    signal input leafSalts[N];
    signal input leafValues[N];
    signal input dogTagIdLeafIndex;
    signal input pathElements[depth];
    signal input pathIndices[depth];
    signal input consentNonce;
    signal input Ax; signal input Ay;          // user's BabyJubjub consent pubkey
    signal input R8x; signal input R8y; signal input S;  // EdDSA sig

    // (b) ZK leaf hashes + dogTagId equality
    component lh[N];
    for (var i = 0; i < N; i++) {
        lh[i] = Poseidon(4);
        lh[i].inputs[0] <== leafKeyPathHashes[i];
        lh[i].inputs[1] <== leafTypeTags[i];
        lh[i].inputs[2] <== leafSalts[i];
        lh[i].inputs[3] <== leafValues[i];
    }
    // assert the dogTagId leaf's value == public dogTagId (selected leaf)
    // (selector + IsEqual; omitted for brevity — constrains leafValues[dogTagIdLeafIndex] == dogTagId)

    // (a) membership -> Rzk
    component mk = PoseidonMerkle(depth);
    mk.leaf <== lh[dogTagIdLeafIndex].out;        // the dogTagId leaf is the membership leaf
    for (var i = 0; i < depth; i++) { mk.pathElements[i] <== pathElements[i];
                                      mk.pathIndices[i]  <== pathIndices[i]; }
    Rzk <== mk.root;

    // (c) consent message + EdDSA verify
    component msg = Poseidon(4);
    msg.inputs[0] <== dogTagId; msg.inputs[1] <== groomerRelayer;
    msg.inputs[2] <== Rzk;      msg.inputs[3] <== consentNonce;
    component sig = EdDSAPoseidonVerifier();
    sig.enabled <== 1; sig.Ax <== Ax; sig.Ay <== Ay;
    sig.R8x <== R8x; sig.R8y <== R8y; sig.S <== S; sig.M <== msg.out;

    // (key binding) expose Poseidon(Ax,Ay) for on-chain check vs userWalletAddress (§2.5)
    // component kb = Poseidon(2); kb.inputs[0] <== Ax; kb.inputs[1] <== Ay;  (public or registry-checked)

    // (d) nullifier
    component nf = Poseidon(4);
    nf.inputs[0] <== consentNonce; nf.inputs[1] <== userWalletAddress;
    nf.inputs[2] <== dogTagId;     nf.inputs[3] <== groomerRelayer;
    nullifier <== nf.out;
}

component main {public [dogTagId, groomerRelayer, userWalletAddress]} =
    DogTagVerification(24 /*N leaves*/, 5 /*depth*/);
```

> `nullifier` and `Rzk` are output signals (also public). The `dogTagId`-equality selector and the
> `Ax,Ay → wallet` binding are sketched in comments; both are standard `IsEqual`/`Mux`/`Poseidon`
> gadgets that add only a few hundred constraints.

---

## 8. Summary of decisions

| Question | Decision |
|---|---|
| Public signals | `dogTagId`, `groomerRelayer`, `userWalletAddress` (+ derived `nullifier`, `Rzk`) |
| R public or in-circuit `isValid`? | `R_zk` **public**; `isValid(R_kec)` **re-checked on-chain**, not in-circuit |
| keccak in-circuit? | **No** — parallel **Poseidon** commitment `R_zk` at issuance, bound to keccak `R_kec` |
| Consent signature | **EdDSA-BabyJubjub** (~4k constraints), not secp256k1 ECDSA (~150k–1.5M) |
| Keep keccak issuance root? | **Yes, untouched** — bound to `R_zk` via event/mapping (§2.3 option A) |
| Circuit size | **~12k–18k constraints** → sub-second proofs |
| Rust prover | **`ark-circom` + `ark-groth16`** (pure Rust, integrated witness, no native deps) |
| Trusted setup | Reuse **Perpetual/Hermez PoT** phase 1 + **multi-party phase-2 + beacon**; ship `.zkey` in backend |
| Nullifier | `Poseidon(consentNonce, userWallet, dogTagId, groomerRelayer)`, in-circuit, registry-tracked |
| Gas | **~240k–270k** per attestation (BN254 verify ~211k + isValid + SSTORE + event) |

---

## Sources

- keccak256 in circom (~151k constraints): [Electron-Labs/keccak-circom](https://github.com/Electron-Labs/keccak-circom), [vocdoni/keccak256-circom](https://github.com/vocdoni/keccak256-circom)
- Poseidon (~210) / EdDSA-BabyJubjub (~4,018) constraints (circomlib): [iden3 EdDSA](https://iden3-docs.readthedocs.io/en/latest/iden3_repos/research/publications/zkproof-standards-workshop-2/ed-dsa/ed-dsa.html), [zk-kit EdDSA Poseidon](https://zkkit.pse.dev/modules/_zk_kit_eddsa_poseidon.html), [Benchmarking ZK-Circuits in Circom (eprint 2023/681)](https://eprint.iacr.org/2023/681.pdf)
- secp256k1 ECDSA in circom (~150k–1.5M): [circom-ecdsa](https://github.com/privacy-scaling-explorations/circom-ecdsa-p256), [BigWhaleLabs private ECDSA](https://blog.bigwhalelabs.com/private-ecdsa-verification-using-zk/), [spartan-ecdsa](https://github.com/personaelabs/spartan-ecdsa)
- Rust prover comparison: [Mopro circom prover comparison](https://zkmopro.org/blog/circom-comparison/), [arkworks/circom-compat](https://github.com/arkworks-rs/circom-compat), [iden3/rapidsnark](https://github.com/iden3/rapidsnark)
- snarkjs + Solidity verifier + setup: [iden3/snarkjs](https://github.com/iden3/snarkjs), [circom 2 proving-circuits docs](https://docs.circom.io/getting-started/proving-circuits/)
- Groth16 gas: [Groth16 verification gas (HackMD)](https://hackmd.io/@nebra-one/ByoMB8Zf6), [Groth16 under 100k gas (7BlockLabs)](https://www.7blocklabs.com/blog/whats-the-cleanest-way-to-optimize-an-on-chain-groth16-verifier-so-each-proof-costs-under-100k-gas), [Groth16 vs FFLONK gas (HackMD)](https://hackmd.io/@Orbiter-Research/S1nat__m0)
- Trusted setup / Powers of Tau: [Perpetual PoT announcement](https://medium.com/coinmonks/announcing-the-perpetual-powers-of-tau-ceremony-to-benefit-all-zk-snark-projects-c3da86af8377), [Hermez PoT selection (HackMD)](https://hackmd.io/@4sHVqkbyQnyF63sea5vFOg/S1XuzpJXw), [RISC Zero trusted-setup security](https://dev.risczero.com/api/trusted-setup-ceremony)
