# DogTag v4 Change-Spec — Unify the credential commitment on Poseidon

> NORMATIVE for the v4 doc-update pass. Source: research **13-poseidon-unification.md** (+ context 02/10). Update agents apply this to `architecture.md`, `implementation.md`, `BUILD_PROMPT.md` using §0. This **simplifies** prior work: it both *rewrites* the hash spec AND *deletes* the dual-root machinery. Where this conflicts with earlier docs, **this wins**; mark superseded items, don't leave contradictions.

## 0. The decision

**The credential commitment (leaf hash + Merkle + verification nullifier) becomes Poseidon — a SINGLE root `R`** anchored at issuance (`issue(R)`) and proven directly by the Groth16 circuit. **keccak is retained ONLY where the EVM/ECDSA standards mandate it:** EIP-712/ECDSA signature digests (normal-path `VerificationConsent`, `DogTagSBT.recover`), Ethereum address derivation, and pure namespacing keys (`recordType = keccak256(label)`, `VERIFY:` whitelist keys, clone `salt`). Everything that is part of the credential commitment or enters the circuit is Poseidon.

## 1. Poseidon canonicalization (replaces arch §3.3/§3.4, impl §1.1–§1.4/§11.2)

`encodeValue` (NFC, pinned decimal grammar, no-float guard, type tags, first-two-colons packed parse, 16-byte CSPRNG salts) is **REUSED VERBATIM** — only the final hashing changes from keccak to Poseidon.

**Byte→field packing** (Poseidon hashes BN254 field elements <254 bits, not byte strings; use the **component-hash** approach, not raw absorb, because circomlib Poseidon arity is compile-time-constant):
```
fieldOf(bytes x) -> field:                      # injective, length-bound, multi-limb
   b = u64be(len(x)) ‖ x                         # 8-byte big-endian length prefix
   limbs = split b into 31-byte big-endian limbs (each < 2^248 < p, no wraparound)
   acc = DS_BYTES; for L in limbs: acc = Poseidon(acc, fieldFromLimb(L))   # domain-sep fold
   return acc
fieldOf(scalar uint) -> field: the integer reduced into [0,p)   # 15-digit chip, timestamps, typeTag, addresses(uint160) all fit one field
```
**Leaf hash:** `hashLeaf = Poseidon(DS_LEAF, fieldOf(keyPath), fieldOf(salt16), fieldOf(typeTag), fieldOf(value))`  (5 inputs → circomlib Poseidon width t=6).
**Merkle** (commutative, sorted-pair, odd-promote, single-leaf root — all preserved): `hashNode = Poseidon(DS_NODE, min(a,b), max(a,b))` (3 inputs), compared as integers in `[0,p)`. The in-circuit ordered tree applies the same sortPair+mux so the proven root == the SDK's `R`.
**Domain tags** (replace the keccak `0x00`/`0x01` bytes — used as the first *input slot*, NOT capacity IV, to stay on the exact circomlib API in all 4 libs): `DS_LEAF=1, DS_NODE=2, DS_BYTES=3, DS_NULLIFIER=4`.

**Pinned instantiation:** ONE circomlib BN254 parameter set — x⁵ S-box, `R_F=8`, per-`t` `R_P`, seed `"poseidon"`, circomlib MDS/round-constants. **Libraries (pin versions):** circom → circomlib `Poseidon`; TS → **`poseidon-lite`**; Rust → **`light-poseidon`** (`new_circom(n)`, Veridise-audited); Solidity → **`poseidon-solidity`** (`PoseidonT3..T7`). **CI MUST assert bit-identical output across all four** against the anchor vector `poseidon([1,2]) = 0x115cc0f5...189a` (circomlibjs has historically drifted — pin + test).

## 2. Single Poseidon root → DELETE the dual-root machinery

Apply to arch §4.1/§4.7/§13.8, impl §1.4/§2.2/§2.6/§11.8/§11.9.

**DELETE entirely (no longer needed — the circuit proves the same root anchored on-chain):**
- `rKec`/`rZk` duality → one root **`R`** (Poseidon). `wrapDocument` returns a single `R`; remove the parallel `hashLeafZk`/`rZk` computation.
- `DogTagIssuer.zkCommit(rKec, rZk)`, the `ZkCommitment` event, the `kecOf[rZk]` mapping.
- `zkIndex` / `cloneOf` / the undefined `issuerForAny`, and the `0x02` binding leaf.
- The circuit's separate `rZk` public output → the public root is **`R`**, the actual issued root.

**Result:** `DogTagIssuer.issue(R)` stores the Poseidon root; `VerificationRegistry` (both paths) checks `DogTagIssuer.isValid(R)` **directly** on the public root `R`. Issuance adds **zero** on-chain hashing (still just stores a `bytes32`).

**Corrected ZK public signals:** `[dogTagId, purpose, relayer, subject, nullifier, keyHash, R]` (`R` replaces `rZk`).

## 3. Nullifier (Poseidon, pinned, shared set)

`nullifier = Poseidon(DS_NULLIFIER, dogTagId, purpose, relayer, subject, nonce)` (6 inputs; addresses as uint160→one field; `purpose` keccak-label reduced mod p). SAME pinned instantiation in circom (public-signal output, never derived from proof bytes) AND the Solidity normal-path computation (`PoseidonT7`). CI parity protects the shared `consumed` set against cross-path double-attest.

## 4. Soundness delta (apply to arch §13.8, impl §11.9)

- **ELIMINATED by unification:** audit-07 **C-1** (zkCommit keccak↔Poseidon binding) and audit-08 **C-2** (unbound `issuerForAny`/`kecOf`) — there is no off-chain binding left to be unsound. Mark them RESOLVED-by-unification in §13.8/§11.9.
- **STILL NORMATIVE (keep):** audit-07 **C-2** / audit-08 **H3** — the ZK path MUST still bind `subject` into the EdDSA message, expose `keyHash=Poseidon(Ax,Ay)` and require `keyHash==ConsentKeyRegistry.keyOf[subject]`, require `ownerOf(dogTagId)==subject`, and bind `purpose` (purpose-scoped `VERIFY:` whitelist, purpose in the nullifier). The hardened §11.6 confirm asserting `Verified`+`consumed[nf]`, the HMAC relay, Art. 9 exclusion, per-pet consent key + rotation, real `setZkVerifier` timelock, and BN254-precompile gating ALL remain.

## 5. Cross-cutting wording (apply everywhere)

- Privacy reasoning is **hash-agnostic**: "even a *salted* commitment is personal data; an unsalted hash of a low-entropy microchip is brute-forceable; the 16-byte salt is the hiding term." Replace keccak-specific phrasing. `dogTagId` MUST NOT be `keccak256(microchip)` **nor `Poseidon(microchip)`** — any hash of a low-entropy chip is brute-forceable; `dogTagId` is a random/sequential non-personal id.
- The §3 "keccak256 is the Ethereum hash" confirmation is **reframed**: Poseidon for the credential commitment; keccak retained only for EIP-712/ECDSA/addresses/namespacing (list them).
- Determinism principle now spans **four** environments (circom/TS/Rust/Solidity) with the pinned Poseidon + CI anchor vector.

## 6. Section-by-section change map

**architecture.md:** §1 (note single Poseidon commitment); §3 (reframe the hash confirmation); §3.3/§3.4 (Poseidon leaf + byte→field packing + domain tags + Poseidon Merkle); §4.1/§4.7 (drop `zkCommit`/`kecOf`/`zkIndex`; `isValid(R)` directly; public signal `R`); §5 (Poseidon root recompute in integrity pillar); §11.1/§13.2 (hash-agnostic privacy + 4-lang Poseidon determinism); §13.8 (mark C-1/C-2 RESOLVED-by-unification; keep subject↔key/ownerOf/purpose).

**implementation.md:** §0 (note circuits use the pinned Poseidon); §1.1–§1.4 (`fieldOf`/`bytesToField`, Poseidon `hashLeaf`/`buildMerkle`; `wrapDocument` returns single `R`, remove `hashLeafZk`/`rZk`); §2.2 (remove `zkCommit`/`ZkCommitment`/`kecOf` from `DogTagIssuer`); §2.6 (remove `zkIndex`; registry `isValid(R)` directly); §11.2 (Poseidon canonicalization + pinned libs + anchor vector); §11.8/§11.9 (public signals `R`; delete zkCommit/cloneOf/issuerForAny; KEEP keyHash/ownerOf/purpose/nullifier; nullifier Poseidon instantiation pinned); §9 (Poseidon 4-lang parity vectors).

**BUILD_PROMPT.md:** principle #1 (Poseidon determinism across circom/TS/Rust/Solidity + CI anchor vector + pinned libs); principle #9 (single Poseidon root `R`; drop rKec/rZk/`zkCommit` language; keep subject↔key/purpose/nullifier rules); Phase 1 (Poseidon trust core, `fieldOf` packing, lib pins, parity vectors); Phase 2 (DogTagIssuer without `zkCommit`/`kecOf`); Phase 2.5 (circuit proves the issued root `R` directly — no dual root, no `zkIndex`); acceptance updates (CI anchor vector `poseidon([1,2])` passes in all 4 langs).

## 7. Keep these EXACT keccak usages (do NOT convert)
EIP-712 `_hashTypedDataV4` digests (normal consent, SBT recover); `ECDSA.recover`; Ethereum address derivation; `recordType=keccak256(label)`; `VERIFY:` whitelist keys; clone `salt=keccak256(...)`. These never enter a circuit and ECDSA is keccak-defined.
