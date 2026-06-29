# DogTag verification circuit (`verification.circom`)

Groth16 circuit proving a **consent-bound credential verification** (impl §11.8(d), §11.9(d),
§11.10). Built with circom 2.1.9 + snarkjs 0.7.6 + circomlib 2.0.5.

## What it proves

`DogTagVerification(N, depth)` (instantiated `main = DogTagVerification(24, 5)` — PRODUCTION:
N=24 max leaves, depth 5, with a **variable** actual leaf count `numLeaves` 1..24):

- **(a)** Recomputes every leaf `Poseidon5([DS_LEAF, keyPathHash, salt, typeTag, value])`, proves
  the prover-supplied `sortedLeafHashes` (the first `numLeaves` entries) is a genuine
  **permutation** of those leaves (via an N×N one-hot permutation matrix gated to the active
  prefix `[0,numLeaves)`: each row/column sums to 1, inactive rows pinned to their own diagonal,
  active rows forbidden from pointing at inactive columns) **and** that the active prefix is
  strictly ascending, then folds **exactly** those `numLeaves` leaves with the SDK node rule
  `Poseidon3([DS_NODE, lo, hi])` (commutative, `lo<=hi` by a full-field 254-bit comparator)
  **with ODD-PROMOTION** to obtain the credential root `R`. The fold is the EXACT static
  construction: `cnt[0]=numLeaves`, `cnt[l+1]=(cnt[l]+1)>>1`; per next-slot `k`,
  `hasPair_k = (2k+1 < cnt[l])` (parent = hashNode of the pair) and
  `promote_k = (2k+1 == cnt[l])` (lone odd node promoted unchanged); `next[k] = hasPair?paired:(promote?level[2k]:0)`.
  For `numLeaves==1` the cnt stays 1 at every level and slot 0 promotes each level, so the single
  leaf passes through unchanged. This `R` equals the SDK `buildMerkle` root bit-for-bit for ANY
  leaf count (the §9 root-parity gate).
- **(b)** `leafValues[dogTagIdLeafIndex] == dogTagId`, `dogTagIdLeafIndex in [0,numLeaves)`, and the
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

`snarkjs r1cs info`: **94459 non-linear constraints**, 0 public inputs, 7 public outputs,
157 private inputs, 93764 wires (N=24, depth=5). The DEV ptau is **power 17** (2^17 = 131072 ≥ 94459).

## Build + test

```
npm run compile-circuit # COMPILE ONLY -> build/verification.r1cs (+ wasm/sym); no trusted setup
npm run build-circuit    # compile + DEV trusted setup -> Groth16Verifier.sol
npm run test-circuit     # round-trip proof + R-parity + negative tests
```

> The PRODUCTION ceremony uses `compile-circuit` (it only needs the r1cs) followed by
> `scripts/ceremony.sh` — never `build-circuit`, which would overwrite the verifier/zkey with a
> forgeable dev key (see [`../docs/CEREMONY_RUNBOOK.md`](../docs/CEREMONY_RUNBOOK.md)).

## DEV-vs-PRODUCTION simplifications (be honest)

- **N=24, depth=5, variable `numLeaves`** (production sizing). The fold implements full
  **odd-promotion** (a lone odd node is promoted unchanged, never duplicated), matching the SDK
  `buildMerkle` for ANY leaf count 1..24. Verified against the SDK for counts {1,2,3,5,6,7,13,24}.
- **DEV trusted setup only** (`scripts/setup.sh`): a *locally generated* power-of-tau (power 17)
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
- The permutation/sortedness/full-field-comparator machinery and the variable-count
  odd-promotion fold are a complete, correct implementation (no stubs) for N=24/depth=5. R-parity
  is verified against the SDK for leaf counts {1,2,3,5,6,7,13,24}. Only the ceremony is dev-grade
  by design.
