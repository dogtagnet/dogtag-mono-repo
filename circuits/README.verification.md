# DogTag verification circuit (`verification.circom`)

Groth16 circuit proving a **consent-bound credential verification** (impl §11.8(d), §11.9(d),
§11.10). Built with circom 2.1.9 + snarkjs 0.7.6 + circomlib 2.0.5.

## What it proves

`DogTagVerification(N, depth)` (instantiated `main = DogTagVerification(8, 3)`):

- **(a)** Recomputes every leaf `Poseidon5([DS_LEAF, keyPathHash, salt, typeTag, value])`, proves
  the prover-supplied `sortedLeafHashes` is a genuine **permutation** of those leaves (via an
  N×N one-hot permutation matrix: each row and each column sums to 1) **and** is strictly
  ascending, then folds it with the SDK node rule `Poseidon3([DS_NODE, lo, hi])` (commutative,
  `lo<=hi` by a full-field comparator) to obtain the credential root `R`. This `R` equals the
  SDK `buildMerkle` root bit-for-bit (the §9 root-parity gate).
- **(b)** `leafValues[dogTagIdLeafIndex] == dogTagId`, `dogTagIdLeafIndex in [0,N)`, and the
  dogTagId leaf's keyPath-hash `== dogTagKeyPathField` (the bound `credentialSubject.dogTagId`
  keyPath field).
- **(c)** EdDSA-BabyJubjub verify `(Ax,Ay,S,R8x,R8y)` over `M = Poseidon6([dogTagId, purpose,
  relayer, subject, R, consentNonce])` (no domain tag) via circomlib `EdDSAPoseidonVerifier`.
- **(d)** `keyHash = Poseidon2([Ax, Ay])`.
- **(e)** `nullifier = Poseidon6([DS_NULLIFIER, dogTagId, purpose, relayer, subject, consentNonce])`.
- **(f)** range-checks `relayer`, `subject` `< 2^160`.

## Public-signal ordering (VERIFIED via `build/verification.sym`)

snarkjs orders public signals by wire index, and circom gives **OUTPUT signals the lowest wire
indices** — so outputs come *before* public inputs in the public-signal vector. The §11.9(e)
Solidity verifier hard-requires

```
pub = [dogTagId, purpose, relayer, subject, nullifier, keyHash, R]
```

To produce that exact order, **all seven public signals are declared as OUTPUTS** (in spec
order); `dogTagId/purpose/relayer/subject` are taken as private inputs and echoed to outputs
(`out* <== in*`). Every named circuit input is therefore PRIVATE. Confirmed by the test:
`public[i]` matches the independently-computed `[dogTagId, purpose, relayer, subject, nullifier,
keyHash, R]`.

## Constraints

`snarkjs r1cs info`: **31369 non-linear constraints**, 0 public inputs, 7 public outputs,
56 private inputs, 31362 wires.

## Build + test

```
npm run build-circuit   # compile + DEV trusted setup -> Groth16Verifier.sol
npm run test-circuit    # round-trip proof + R-parity + negative tests
```

## DEV-vs-PRODUCTION simplifications (be honest)

- **N=8, depth=3** (NOT production 24/5). N=8 is a power of two, so the Merkle fold is purely
  pairwise. **Odd-promotion** (`buildMerkle` promotes a lone odd node unchanged) is NOT
  implemented — `// TODO(production)` in the fold. Production non-power-of-2 N **requires**
  odd-promotion to match the SDK root.
- **DEV trusted setup only** (`scripts/setup.sh`): a *locally generated* power-of-tau (power 15)
  with a *single* contributor and a throwaway beacon. **Production REQUIRES** the public
  Hermez/Perpetual ptau + a **≥3-contributor** phase-2 ceremony ending in a **public verifiable
  beacon** so no party knows the toxic waste. The dev pipeline is for testing only.
- In-circuit, `leafKeyPathHashes/leafSalts/leafTypeTags/leafValues` are treated as
  **already-reduced field inputs** — the prover precomputes `fieldOf(keyPath)`, `fieldOf(salt)`,
  `fieldOf(value)` (the length-prefixed `bytesToField` fold) OUTSIDE the circuit. The value-field
  is bound by the issued root anyway, so the fold stays out of circuit.

## Validated vs stubbed

- VALIDATED end-to-end: compile, dev setup, witness → groth16 prove → verify, public-signal
  order, R-parity against the built SDK (`packages/dogtag-standard-ts/dist`), and three negative
  tests (tampered dogTagId leaf value, bad EdDSA signature, tampered nullifier public signal).
- The permutation/sortedness/full-field-comparator machinery is a complete, correct
  implementation (no stubs) for N=8. Odd-promotion is the only spec item deliberately deferred
  (TODO), and the ceremony is dev-grade by design.
