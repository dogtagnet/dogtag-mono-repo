# 13 — Poseidon Unification: one root, one hash, no dual-root machinery

> **Corrections (audit-10):** the **nullifier arity is t=7** (`Poseidon(DS_NULLIFIER, dogTagId, purpose, relayer, subject, nonce)`) — ignore any contradictory t=6 nullifier prose below. CI parity requires **per-arity anchor vectors at t=2/3/6/7** (a single t=3 anchor is insufficient). All field reductions are pinned to the BN254 **scalar field `r`**. Normative source: `CHANGESPEC-v4` + `implementation.md §11.10` + `architecture.md §13.9`.

> Status: v4 design proposal (NORMATIVE once adopted; intended to override the dual-root §4.7 / §11.8 /
> §11.9 and the §3.3–§3.4 keccak leaf/Merkle standard **for the credential commitment only**).
> Companion to [`architecture.md`](../architecture.md) (§3 salted-leaf keccak Merkle standard, §4.7
> verification contracts, §13.2 determinism fixes, §13.8 ZK remediations) and
> [`implementation.md`](../implementation.md) (§1.1–§1.4 `encodeValue`/`hashLeaf`/`buildMerkle`, §11.2
> canonicalization fixes, §11.8/§11.9 ZK). Chain: **ROAX** (EVM, chainId **135**, gas token PLASMA).
> Curve: **BN254 / alt_bn128** (the only pairing precompile family on an EVM chain).

---

## 0. Thesis & scope

Today the design carries **two roots** for every credential:

- `rKec` — the keccak256 salted-leaf commutative Merkle root, anchored on-chain via `DogTagIssuer.issue(rKec)` and gated by `isValid(rKec)` (§3.3–§3.4, §1.2–§1.3).
- `rZk` — a *parallel* Poseidon Merkle root over the **same leaf set**, computed only for the Groth16 ZK path, bound back to `rKec` via `DogTagIssuer.zkCommit(rKec, rZk)` + a `kecOf[rZk] → rKec` mapping + a `zkIndex` (§11.8/§11.9).

This dual-root machinery exists **only** because keccak-in-circuit is ~151k constraints/hash and therefore unprovable (research/10 §0). The binding `rZk ↔ rKec` is the source of audit-07 C-1 / audit-08 C-2 (the keccak↔Poseidon binding is trusted off-chain, not proven in-circuit) and forces `kecOf`/`zkIndex`/`issuerForAny` plumbing.

**This spec unifies on Poseidon.** A single Poseidon root **R** becomes the credential commitment: it is the value computed off-chain by the SDK, anchored at issuance via `issue(R)` (the contract stores a `bytes32`, computes nothing), and the **same** root the Groth16 circuit proves over. There is no second root, no `kecOf`, no `zkIndex`, no `zkCommit`, no `rKec/rZk` duality.

### 0.1 What changes vs. what is untouched (the hard constraint)

**keccak256 MUST REMAIN, unchanged**, for everything that is not the credential commitment:

| keccak use | Why it stays |
|---|---|
| EIP-712 / ECDSA signature digests (`VerificationConsent`, SBT `recover()`, `bindConsentKey`) | EIP-712 `_hashTypedDataV4` is defined over keccak; wallets sign keccak digests. Poseidon is not an Ethereum signing primitive. |
| Ethereum address derivation | `keccak256(pubkey)[12:]` — protocol-level, immutable. |
| Namespacing keys | `recordType = keccak256(label)`, `keccak256("VERIFY:" ‖ purpose)` — pure off-chain/identifier hashing, no circuit, no field-element constraint, cheap. |
| EIP-1167 clone salt | `keccak256(recordType, business)` (§13.1 M-1). |

**Poseidon REPLACES keccak** for exactly three things, all of which become one unified primitive:

1. The **credential leaf hash** (was §3.3 keccak leaf).
2. The **credential Merkle tree** (was §3.4 keccak commutative tree).
3. The **verification nullifier** (already Poseidon in both paths; now the *only* hash, with circom == Solidity == Rust parity).

The single root **R** is anchored via `issue(R)` and re-checked via `isValid(R)` directly — the ZK circuit's public root output **is** R, so the registry calls `isValid(R)` with no mapping.

---

## 1. Poseidon over arbitrary bytes — the byte→field-element packing (the crux)

### 1.1 The problem

Poseidon's permutation operates on **BN254 scalar field elements**. The field modulus is

```
p = 21888242871839275222246405745257275088548364400416034343698204186575808495617   (≈ 2^254)
```

so a field element holds **< 254 bits ≈ 31.7 bytes**. Our leaf tuple `(keyPath, salt, typeTag, value)` is *bytes*, not field elements:

- `keyPath` — a variable-length NFC UTF-8 string (e.g. `credentialSubject.microchip.code`), can exceed 31 bytes.
- `salt` — exactly 16 raw bytes (fits in one field: 128 bits < 254).
- `typeTag` — one byte `uint8` (fits trivially).
- `value` — variable-length canonical bytes (§1.1 of impl): a 15-digit microchip (15 bytes, fits), a timestamp string (~20–25 bytes, fits), but a `distinctiveFeatures` string or a `taskDescription` can be hundreds of bytes (does **not** fit in one field).

Naively `Poseidon(keyPath, salt, typeTag, value)` is therefore impossible: you cannot hand Poseidon a 300-byte string as "one element," and packing many bytes into "one element mod p" silently reduces mod p (loses bits, breaks injectivity, enables collisions).

### 1.2 Decision: hybrid component-hash (option a), NOT raw multi-field absorb (option b)

Two candidates were considered:

- **(b) Pack all leaf bytes into a field array and Poseidon-absorb the whole array.** Rejected. circomlib `Poseidon(t)` has a **fixed arity** (state size t∈[2,16], so ≤ 15 inputs), and a circuit template's input count must be a **compile-time constant**. A variable-length value would mean a variable number of absorbed fields → an unstable circuit shape per credential → impossible (the prompt's "long strings need multi-field" pitfall). Sponge-style variable absorb is also not what circomlib `Poseidon` implements (it is a fixed-width compression function, not a duplex sponge).

- **(a) Hash each variable-length component to ONE field with a deterministic byte→field reducer, then Poseidon the fixed-arity tuple of single fields.** **CHOSEN.** Every component is pre-reduced to a single canonical field element by a length-bound chunk-and-fold, so the final Poseidon call has a **fixed arity (t=5, 4 inputs)** regardless of value length, giving a constant circuit shape. This matches research/10 §2.1's `Poseidon(Poseidon(keyPathHash, typeTag), saltField, valueField)` sketch, hardened with explicit length-binding and a domain tag.

### 1.3 `bytesToField` — the canonical byte→single-field reducer (length-bound)

The one primitive both the byte→field packing and circuit replicate. It is a **length-prefixed, 31-byte-chunked, Poseidon-folded** absorb:

```
const FIELD_CAPACITY_BYTES = 31          // floor(253/8); 31 bytes = 248 bits < p, never reduces mod p
const DS_BYTES = 3                         // domain tag for the byte-absorb sub-hash (see §3)

fn beU64(n) -> 8 bytes big-endian          // 64-bit length is ample (no value approaches 2^64 bytes)

fn bytesToField(b: bytes) -> Field:
    // 1. Length-bind: prepend an 8-byte big-endian length so b="ab" (2 bytes) and
    //    b="ab\x00...\x00" can never collide, and empty bytes has a defined image.
    framed = beU64(len(b)) ++ b
    // 2. Chunk into 31-byte big-endian limbs (last limb zero-PADDED on the RIGHT to 31).
    chunks = split_into_31byte_chunks(framed)          // each chunk -> a Field via big-endian decode
    fields = [ be_decode_to_field(c) for c in chunks ] // each < 2^248 < p, NO modular reduction
    // 3. Fold the limbs with a domain-separated Poseidon chain (t=2 compression).
    acc = DS_BYTES                                      // domain/IV (see §3.2)
    for f in fields:
        acc = Poseidon2(acc, f)                         // circomlib Poseidon, arity 2
    return acc
```

Properties:

- **No modular wraparound.** Each 31-byte limb decodes to `< 2^248 < p`, so the byte→limb map is injective into the field.
- **Length-binding twice.** The 8-byte length prefix kills the "trailing-zero padding" ambiguity at the *content* level; the per-limb chunking is collision-free because the length prefix fixes how many real bytes the final padded limb contains.
- **Fixed circuit shape per component.** For a given schema field the **max byte length is schema-bounded**, so the circuit instantiates `bytesToField` with a fixed `maxChunks` and range-checks the actual length (audit-07 H-1 style). `keyPath`s are known compile-time constants in the circuit (their `bytesToField` images are precomputed and constrained), so only `value` needs the in-circuit chunked absorb, bounded by the schema's max field length.
- **`salt` and `typeTag` skip the reducer** — they already fit one field (16 bytes / 1 byte), so they are big-endian-decoded directly to a Field (still length-implicit because their lengths are fixed by spec: 16 and 1).

### 1.4 `hashLeaf` (Poseidon) — the leaf commitment

```
const DS_LEAF = 1          // leaf domain tag (see §3)

fn fieldOfSalt(salt: bytes16)  -> Field: assert len(salt)==16; return be_decode_to_field(salt)   // < 2^128
fn fieldOfTag(typeTag: u8)     -> Field: return Field(typeTag)                                    // 0..5
fn fieldOfKeyPath(kp: string)  -> Field: return bytesToField(utf8(NFC(kp)))
fn fieldOfValue(tag,value)     -> Field: return bytesToField(encodeValue(tag, value))   // encodeValue == impl §1.1, UNCHANGED

fn hashLeaf(keyPath, salt, typeTag, value) -> Field:
    return Poseidon5( DS_LEAF,                  // arity-5 (t=6): domain tag + 4 components
                      fieldOfKeyPath(keyPath),
                      fieldOfSalt(salt),
                      fieldOfTag(typeTag),
                      fieldOfValue(typeTag, value) )
```

- **`encodeValue` is reused verbatim** from impl §1.1 (the typeTag-driven canonical byte encoding: NFC strings, pinned-grammar integers/decimals, bool 0x00/0x01, null = empty, raw bytes). Determinism rules (audit-02 A1/A2/A3, the pinned decimal grammar, NFC, no-float guard) are **byte-identical** — only the final hash over those canonical bytes changes from keccak to Poseidon-via-`bytesToField`.
- **Output is a Field**, not bytes32. On-chain and in storage we serialize it as `bytes32` big-endian (always `< p < 2^254 < 2^256`, fits bytes32). The SDK exposes both the Field and its 0x-hex bytes32 form.
- **Length-binding is preserved** end-to-end: keyPath via its 8-byte length prefix inside `bytesToField`; salt via fixed 16; typeTag fixed 1; value via its 8-byte length prefix inside `bytesToField`. There is no intra-leaf concatenation ambiguity (the old §3.3 `u32be` length prefixes are subsumed by `bytesToField`'s `beU64` prefix + the fixed-arity tuple separating the four components into distinct Poseidon inputs).

> **Why arity-5 (t=6) and not nest like research/10's `Poseidon(Poseidon(kp,tag),salt,val)`:** a single flat `Poseidon5` with a domain tag is one permutation call, has a cleaner second-preimage argument (all four components + domain occupy distinct, fixed input slots), and circomlib supports t=6 directly. Nesting saves nothing here and adds a second hash to audit.

### 1.5 Exact cross-language algorithm (reproducible in 4 languages)

The algorithm is **fully specified by**: (i) `encodeValue` (impl §1.1, unchanged); (ii) `bytesToField` (§1.3); (iii) `hashLeaf` (§1.4); (iv) the pinned Poseidon (§2); (v) the domain tags (§3). Each language:

- **circom** (`circuits/`): `bytesToField` = a `BytesToField(maxChunks)` template using circomlib `Poseidon(2)` over range-checked 31-byte limbs; `hashLeaf` = circomlib `Poseidon(6)` over `[DS_LEAF, kpField, saltField, tagField, valField]`. keyPath images are circuit constants. Bytes are presented as field-limbs by the witness generator.
- **TS** (`packages/dogtag-standard-ts`): `poseidon-lite` (`poseidon2`, `poseidon6`); big-endian `Buffer`→`BigInt` limb decode; `bytesToField`/`hashLeaf` as plain functions.
- **Rust** (`crates/dogtag-standard-rs`): `light-poseidon` (`Poseidon::<Fr>::new_circom(2)`, `new_circom(5)`) over `ark_bn254::Fr`; `Fr::from_be_bytes_mod_order` is **not** used for limbs (would reduce) — instead build each `Fr` from a ≤31-byte big-endian limb that is provably `< p`.
- **Solidity** (`contracts/`): only needed for the **nullifier** (§5) and any future on-chain Merkle verifier — issuance does NOT compute Poseidon on-chain. `poseidon-solidity` `PoseidonT3`/`PoseidonT6`. Leaf hashing on-chain is not required for v1 (the chain stores R; it never recomputes leaves).

---

## 2. Pinned Poseidon instantiation (the one parameter set)

**ONE** circomlib-compatible BN254 instantiation, used by every language. Parameters (the circomlib / iden3 standard):

| Parameter | Value |
|---|---|
| Field | BN254 / alt_bn128 scalar field, `p` as in §1.1 |
| S-box | `x^5` (α = 5) |
| State size `t` | `t = nInputs + 1` (capacity 1). We use **t=2** (1 input — internal fold), **t=3** (2 inputs — Merkle node + byte-fold compression), **t=6** (5 inputs — leaf), **t=6** also for the **5-input nullifier** |
| Full rounds `R_F` | **8** (4 + 4) for all `t` in circomlib |
| Partial rounds `R_P` | per-`t` from circomlib's table: `t=2 → 56`, `t=3 → 57`, `t=4 → 56`, `t=5 → 60`, `t=6 → 60` |
| Round constants `C` | circomlib's `poseidon_constants.circom` / iden3 `poseidonConstants` (seed string `"poseidon"`, Grain LFSR per the Poseidon paper) |
| MDS matrix `M` | circomlib's per-`t` MDS (Cauchy matrix from the same generation script) |
| Capacity / IV | initial state `[0, in_0, …, in_{t-2}]` (circomlib sets the capacity lane to 0; **our domain separation is a first message input**, not a capacity IV — see §3) |

This is the de-facto "circomlib Poseidon" that the entire iden3/Semaphore/MACI/Tornado ecosystem uses. **Security target: 128-bit** (the paper's parameters for BN254 `x^5` at these `R_F`/`R_P`).

### 2.1 The four named, maintained libraries (all share this parameter set)

| Language | Library | Notes / parity |
|---|---|---|
| **circom** | **circomlib** `circuits/poseidon.circom` (`Poseidon(nInputs)` template, supports t∈[2,16]) | The reference. All others are validated against it. |
| **TS / JS** | **poseidon-lite** (`chancehudson/poseidon-lite`, npm) — exports `poseidon1`…`poseidon16`; pure JS, no WASM, deterministic; tested against circomlibjs vectors. Use `poseidon2`, `poseidon5`. (Alternative: `circomlibjs` `buildPoseidon()`, WASM, same constants but heavier and historically had a version where the hash changed — pin a version.) | poseidon-lite is the recommended modern choice (smaller, no build step, drop-in for circomlib). |
| **Rust** | **light-poseidon** (`Lightprotocol/light-poseidon`, crates.io) — circom-compatible BN254 params via `Poseidon::new_circom(nInputs)`, S-box `x^5`, t=2..13; **audited by Veridise**; uses `ark-bn254`/`ark-ff`. (Alternative: `TaceoLabs/poseidon-rust` — explicitly "compatible with Circom," but currently only t=3/t=4.) | light-poseidon covers all our arities (2,3,5,6) and is audited → primary. ark-circom's bundled Poseidon is the same family for the prover. |
| **Solidity** | **poseidon-solidity** (`chancehudson/poseidon-solidity`, npm) — gas-optimized circomlib-compatible `PoseidonT2`…`PoseidonT7`; deployed via a deterministic-deployment proxy at a fixed address; **byte-identical to circomlibjs/poseidon-lite**. (Alternative: circomlibjs `poseidonContract` codegen — same constants, more gas: ~32k vs ~21k for T3.) | We need `PoseidonT6` (5 inputs) for the nullifier; `PoseidonT3` if an on-chain Merkle verifier is later added. |

### 2.2 Parity proof obligation (mismatch risk — flagged)

**This is the single highest-risk item.** Historical mismatches are real:

- **circomlibjs once changed its Poseidon** (iden3/circomlibjs #14 "Poseidon hash has changed") — *pin the library version*.
- **circomlibjs vs Solidity produced different outputs** at one version (iden3/circomlibjs #30) — solved by using `poseidon-solidity` (regenerated from the *current* constants) and CI vectors.
- **light-poseidon must be called via `new_circom(n)`** (the circom-compatible constructor), NOT a generic constructor with arbitrary params, or constants diverge.

**Mitigation (NORMATIVE, CI-asserted):** a single `poseidon-vectors.json` checked into the repo; a CI job runs the **same inputs** through circom (witness + a tiny test circuit), poseidon-lite, light-poseidon, and a deployed `PoseidonT*` (Foundry) and asserts **bit-identical** field outputs. The canonical anchor vector is:

```
poseidon([1, 2]) = 7853200120776062878684798364095072458815029376092732009249414926327459813530
                 = 0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a
```

If any library fails this vector at its pinned version, that version is rejected at the lockfile/CI gate. **No library is "compatible by reputation" — it is compatible only after the vector passes in CI.**

---

## 3. Domain separation in Poseidon (replacing 0x00 / 0x01 keccak bytes)

The keccak standard prepends a domain byte: `0x00` (LEAF), `0x01` (NODE), reserved `0x02` (binding). Poseidon has no byte string to prepend; domain separation is done by a **dedicated domain tag occupying the first input slot** (equivalently, mixed into the initial state). We use **distinct small-integer domain tags as the first absorbed field**:

| Domain | Tag | Used in |
|---|---|---|
| `DS_LEAF` | **1** | `hashLeaf` first input (§1.4) |
| `DS_NODE` | **2** | `hashNode` (Merkle parent) — see §4 |
| `DS_BYTES` | **3** | `bytesToField` fold IV (§1.3) |
| `DS_NULLIFIER` | **4** | nullifier (§5) — implicit via fixed distinct arity+inputs; see §5.1 |

Properties:

- **Leaf↔node confusion is impossible:** a leaf is `Poseidon6(DS_LEAF=1, …)` (5 inputs incl. tag), a node is `Poseidon3(DS_NODE=2, …)` (2 inputs incl. tag). Different arity **and** different first-field tag → no input string is valid in both.
- **`bytesToField` (DS_BYTES) vs leaf/node:** the byte-fold is a `Poseidon2` chain whose IV is `3`; it is never confused with a leaf (t=6) or node (t=3) because of arity and because its output is only ever consumed as an *input field* to a leaf, never as a leaf/node itself.
- **Domain tags are public constants**, pinned in the spec and all four implementations; they are **not** secret and add ~0 cost (a literal field input).

> **Why a first-field tag, not a capacity IV:** circomlib `Poseidon` fixes the capacity lane to 0 and is most portably called as "hash of N inputs." Using input-slot tags keeps us on the exact circomlib API in all four libs (no custom IV plumbing, which is where cross-lib mismatches creep in). The security argument (domain separation = distinct, fixed-position constant per domain) is identical.

---

## 4. Merkle tree with Poseidon

2-to-1 compression with **`PoseidonT3` (t=3, 2 inputs)** plus the domain tag — so a node is actually a **3-input Poseidon** `Poseidon3(DS_NODE, a, b)`... but that breaks the commutative `sortPair` symmetry if done naively. Resolution below.

### 4.1 Keep the commutative sorted-pair + odd-promotion rule (confirmed safe)

```
const DS_NODE = 2

fn cmpField(a, b) -> bool: a <= b    // compare as integers in [0, p)  (canonical, deterministic)

fn hashNode(a: Field, b: Field) -> Field:
    (lo, hi) = cmpField(a,b) ? (a,b) : (b,a)        // commutative: sort the pair
    return Poseidon3(DS_NODE, lo, hi)               // t=3: domain tag + sorted pair

fn buildMerkle(leafHashes: Field[]) -> { root, layers }:
    if leafHashes.empty: ERROR
    level = sort_ascending_by_integer_value(leafHashes)    // canonical leaf order
    layers = [level]
    while len(level) > 1:
        next = []
        i = 0
        while i < len(level):
            if i+1 < len(level): next.push(hashNode(level[i], level[i+1])); i += 2
            else:                next.push(level[i]); i += 1      // PROMOTE odd, never duplicate
        level = next; layers.push(level)
    return { root: level[0], layers }                            // single leaf -> root == that leaf
```

- **Commutative sorted-pair is safe with Poseidon domain sep.** The off-chain SDK Merkle proof can stay an *unordered sibling set* (no left/right bits), exactly as in §3.4, because `hashNode` sorts. The `DS_NODE` tag prevents leaf/node confusion; sorting prevents order ambiguity. This preserves the §3.5 selective-disclosure / obfuscation property (`privacy.obfuscated[]` holds leaf Fields; rebuild the sorted set → same root).
- **Single-leaf document:** `root = the one leaf hash` (no node hashing), same as §3.4.
- **Comparison is by integer value in `[0, p)`** (not raw bytes32), the canonical ordering of field elements. (Since every leaf `< p < 2^254`, bytewise bytes32 comparison and integer comparison coincide — but integer-in-`[0,p)` is the normative definition.)

### 4.2 The ZK circuit Merkle (the one subtlety — ordered, not commutative, in-circuit)

Inside Groth16, an **ordered (index-bit) Merkle path** is cheaper and avoids in-circuit sorting (research/10 §1.3 ZK-tree note). To keep **one root R** across SDK and circuit, the rule is:

- The SDK is the **canonical root producer**: it sorts leaves ascending and builds the commutative tree (§4.1). This R is what `issue(R)` anchors.
- The circuit proves membership using **the SDK's fixed, sorted leaf order**: `pathIndices` encode the position in the *sorted* tree, and the in-circuit node hash applies the **same `sortPair` (via a comparator + mux) + `DS_NODE`** so the in-circuit recomputed root equals the SDK's R bit-for-bit. (This is a few extra constraints per level — a `LessThan` + `Mux` — and is the correct, audited way to make an in-circuit tree match a commutative off-chain tree. Alternatively, since the SDK already sorts, the issuer can fix a *stable* leaf order and the circuit uses ordered hashing with that same `sortPair` semantics. Either way the **single rule** is: node = `Poseidon3(DS_NODE, min(a,b), max(a,b))`.)

This eliminates the old "two different trees (commutative keccak vs ordered Poseidon) → two roots" problem: **there is one tree definition; the circuit just proves it.**

---

## 5. Nullifier

```
const DS_NULLIFIER = 4
fn fieldOfAddr(a: address) -> Field: return Field(uint160(a))      // 160 bits < p, one field, no reduction

nullifier = Poseidon6( DS_NULLIFIER,                 // t=6, 5 inputs (incl. domain tag)
                       fieldOf(dogTagId),            // uint256 -> MUST be < p (range-checked) — see §7
                       fieldOf(purpose),             // bytes32 keccak label -> reduce/range-check (see §5.2)
                       fieldOfAddr(relayer),
                       fieldOfAddr(subject),
                       fieldOf(nonce) )
```

This matches §11.9(b) `nullifier = Poseidon(dogTagId, purpose, relayer, subject, nonce)` (5 logical inputs), realized as a **t=6** circomlib Poseidon with `DS_NULLIFIER` in slot 0.

### 5.1 Arity & parity

- **Arity: t=6** (5 inputs: the 4 logical + domain tag, OR if we treat the domain implicitly, exactly 5 logical inputs in t=6). NORMATIVE: 6-lane state, inputs = `[DS_NULLIFIER, dogTagId, purpose, relayer, subject, nonce]` → **t=7? No.** Clarify: 6 input slots = t=7 in circomlib (t = nInputs+1). **To keep the §11.9 spec of 5 logical inputs without inflating arity, the domain is folded as: nullifier = `Poseidon6(dogTagId, purpose, relayer, subject, nonce)` with t=6 (5 inputs) and NO separate tag** — the nullifier's distinct *semantic* (these 5 specific signals) plus its dedicated `PoseidonT6` call site is its domain. Leaf uses t=6 with `[DS_LEAF, kp, salt, tag, val]` (also 5 inputs). To avoid leaf↔nullifier collision at the same arity, **the leaf keeps `DS_LEAF=1` in slot 0**; the nullifier inputs are 5 distinct semantic fields that can never equal `[1, kpField, saltField, tagField, valField]` for any real credential (slot 0 = `dogTagId` is a token id, slot-collision with the constant `1` is a non-issue and is additionally prevented by `dogTagId ≥ 1` allocation). **Cleaner normative choice: use t=6 for the nullifier with inputs `[dogTagId, purpose, relayer, subject, nonce]` and t=6 for the leaf with inputs `[DS_LEAF, …]`; they share arity but differ because the leaf's slot-0 is the reserved constant `1` and credentials never set `dogTagId == 1`-as-domain.** (If absolute domain rigor is preferred, bump the nullifier to t=7 with `DS_NULLIFIER` in slot 0 — a small, documented gas/constraint cost. Recommended: **t=7 with `DS_NULLIFIER`** for zero ambiguity; finalize in CI vectors.)

> **Resolution (NORMATIVE):** nullifier = **`Poseidon7(DS_NULLIFIER, dogTagId, purpose, relayer, subject, nonce)`** (t=7, 6 inputs). Unambiguous domain separation from the t=6 leaf; circomlib supports t≤16; `poseidon-solidity` ships `PoseidonT7`; light-poseidon supports it; poseidon-lite exports `poseidon6`. Gas/constraints marginally higher than t=6 but correctness-first.

- **circom == Solidity parity (audit-08 / §13.8 flag):** the **same** `Poseidon7` instantiation computes the nullifier as a **circuit output public signal** (ZK path, never derived from proof bytes — snarkjs #383) AND on-chain in `recordVerification` (normal path) via `poseidon-solidity` `PoseidonT7`. CI asserts circom == Solidity == Rust on nullifier vectors, so the **shared `consumed` set actually blocks cross-path double-attestation** (the whole point of §13.8's "pinned Poseidon" item).

### 5.2 Field-encoding of inputs

- **addresses (160 bits):** `uint160(addr)` → one field, no reduction (160 < 254).
- **`dogTagId` (uint256):** allocated as a non-personal id (§4.2). MUST be `< p` (enforced at mint and range-checked on-chain / in-circuit — §7). A sequential or `< 2^160` random id trivially satisfies this.
- **`purpose` / `recordType` (bytes32 = keccak label):** a full 256-bit keccak output can exceed `p`. **NORMATIVE: reduce once via `uint256(purpose) % p`** at the field boundary (both on-chain before the Poseidon call and in-circuit as a public-signal range/reduce), OR define `purpose` labels to be `< p` by construction (e.g. take the low 248 bits of the keccak label). We pin **"reduce mod p"** and treat the reduced value as the canonical purpose field; the on-chain `Verified.purpose` event still carries the full bytes32 label for readability (the *reduced* value is only the Poseidon input). CI vector covers a label whose keccak exceeds `p`.
- **`nonce` (uint256):** range-checked `< p` (the consent nonce is freely chosen `< p`).

---

## 6. What gets DELETED (the unification dividend)

With one root R, the following machinery is **removed entirely**:

| Deleted | Where it lived | Why it's gone |
|---|---|---|
| `rKec` / `rZk` duality | §3.3–§3.4 (rKec), §1.4/§11.8 (rZk) | One root R. The credential commitment is Poseidon; there is no parallel keccak credential root. |
| `DogTagIssuer.zkCommit(rKec, rZk)` + originator-gated body | §4.1, §2.2, §11.8/§11.9(c) | No two roots to bind. Issuance is just `issue(R)`. |
| `ZkCommitment(rKec, rZk)` event | impl §11.8 | — |
| `kecOf[rZk] → rKec` mapping | §4.1, impl §2.2/§11.8 | The ZK circuit's public root output **is** R; registry calls `isValid(R)` directly. |
| `zkIndex` / `cloneOf(rZk)` global index | §11.9(c)(e), `zkIndex.register` | The registry resolves the clone via `issuerFor[recordType]` (already present) and calls `isValid(R)`; no `rZk→clone` index, no `issuerForAny()`. |
| The circuit's separate `rZk` output | impl §11.8(d), §11.9(d) | Replaced by a single public root output `R` (== the issued root). |
| Registry's `kecOf` lookup before `isValid` | impl §11.8(a), §11.9(e) | `require(DogTagIssuer(clone).isValid(R))` directly on the public signal. |
| `keccak256(0x02 ‖ rKec ‖ rZk)` binding leaf + `0x02` domain | research/10 §2.3 | No binding needed; no third keccak domain. |
| `hashLeafZk` / `poseidonMerkle` as a *parallel* path | impl §1.4 | They become the *primary, only* `hashLeaf`/`buildMerkle`. The keccak `hashLeaf`/`buildMerkle` for the credential commitment are **retired** (keccak survives only for §0.1's non-commitment uses). |

### 6.1 ZK soundness Criticals — eliminated vs. still-apply

- **ELIMINATED by unification:**
  - **audit-07 C-1** (keccak↔Poseidon binding trusted off-chain, not proven in-circuit) — **gone**: there is no keccak credential root to bind to. The circuit proves leaves → R, and R is the anchored root. No binding step exists to be unsound.
  - **audit-08 C-2** (`zkCommit` forgeable / `issuerForAny()` undefined-forgeable / the binding is the trust gap) — **gone**: `zkCommit`/`kecOf`/`zkIndex`/`issuerForAny` are deleted. The registry uses the existing per-`recordType` `issuerFor` and calls `isValid(R)` on the circuit's public root. (Note: the circuit still does **not** prove `isValid` — the registry re-checks it on-chain, now directly on R, which is *strictly simpler and safer* than the mapping.)

- **STILL APPLY — must stay (NOT addressed by hash unification):**
  - **audit-07 C-2 / audit-08 H3 — subject↔key binding + purpose binding.** The circuit MUST still: verify the EdDSA-BabyJubjub consent signature over `Poseidon(dogTagId, purpose, relayer, subject, R, nonce)` (binds `subject` + `purpose`), output `keyHash = Poseidon(Ax, Ay)`, and the registry MUST still require `consentKeys.keyOf[subject] == keyHash` AND `sbt.ownerOf(dogTagId) == subject`. Unifying the hash does nothing for these; they are the real ZK soundness guarantees and remain **NORMATIVE** (§11.9(d)(e)).
  - **`ownerOf(dogTagId) == subject`** (pet belongs to the consenter) — stays.
  - **purpose-scoped `VERIFY:` whitelist** (`isWhitelistedFor(keccak256("VERIFY:" ‖ purpose), relayer)`) — stays (and still uses keccak for the *namespacing key*, per §0.1).
  - **Range-check ALL public signals `< p`** (snarkjs #358) and **nullifier-as-public-signal-not-proof-bytes** (snarkjs #383) — stay.
  - **`relayer == msg.sender`** binding on both paths — stays.

The public-signal vector becomes: `[dogTagId, purpose, relayer, subject, nullifier, keyHash, R]` (was `…, rZk]`; only the last name changes from `rZk` to the unified `R`).

---

## 7. Security & cost

### 7.1 Poseidon vs keccak as a *commitment*

- **Maturity:** keccak256 (SHA-3 family) is NIST-standardized and battle-tested; Poseidon is newer but is the **de-facto ZK commitment hash** (Tornado Cash, Semaphore, MACI, zkSync, Polygon ID, Mina) with the original Poseidon paper's 128-bit BN254 parameters widely deployed and independently analyzed (and our chosen lib, light-poseidon, is **Veridise-audited**). For a **salted commitment** (not a password hash, not a long-lived secret), Poseidon at 128-bit security is appropriate. The privacy mechanism is the **16-byte (128-bit) random salt** (§11.1), unchanged — the salt, not the hash's collision margin, is what makes a low-entropy microchip number non-brute-forceable, and that argument is hash-agnostic.
- **Algebraic-attack caveat (flag):** Poseidon's security rests on algebraic (Gröbner-basis) cryptanalysis margins rather than decades of differential/linear analysis like Keccak. We accept this because (a) it is the standard ZK choice, (b) the commitment's secrecy is salt-driven, and (c) the on-chain trust is the *issuance gate* (`isValid`) + DNS pillar, not the hash's pre-image hardness alone. The mandatory DPIA (§11.1) should note the hash change.

### 7.2 On-chain gas

- **Issuance: zero on-chain hashing.** `issue(R)` stores a `bytes32` (one `SSTORE`), exactly as today. The contract **never** computes Poseidon (or keccak) for the credential commitment — R is computed off-chain by the SDK. **No gas regression at issuance**; in fact the design is unchanged at the issuance call site (still a `bytes32` root).
- **Normal-path nullifier: one on-chain Poseidon.** `recordVerification` computes `nullifier = PoseidonT7(...)` once. `poseidon-solidity` `PoseidonT3` ≈ **21k gas**; `PoseidonT7` (6 inputs) is larger but still on the order of a few tens of thousands of gas (each extra lane adds round-MixLayer work; budget ~40–70k gas for T7, to be measured in CI). This is the **only** on-chain Poseidon in the system. Total `recordVerification` ≈ ECDSA `ecrecover` (3k) + `ownerOf` STATICCALL + `isValid` STATICCALL + PoseidonT7 + nullifier `SSTORE` + event — well within a normal tx.
- **ZK path:** Groth16 `verifyProof` (~211k gas) is unchanged; the nullifier is a public signal (no on-chain Poseidon). Total ~240–270k (unchanged).
- **Deployment:** `poseidon-solidity` deploys `PoseidonT3`/`PoseidonT7` once via the deterministic-deployment proxy; the registry references the fixed library addresses. (Gate Phase 2.5 on ROAX supporting BN254 pairing precompiles for the ZK path — §13.8(k) — unaffected by hash unification, since the normal path's Poseidon is pure EVM, no precompile.)

### 7.3 BN254 field-element pitfalls

- **`value >= p`:** never an issue for our *value* encoding — `bytesToField` chunks into 31-byte limbs each `< 2^248 < p`, so no value ever needs reduction; injectivity holds.
- **`dogTagId` (uint256):** could exceed `p` if allocated as a full 256-bit random. **NORMATIVE: allocate `dogTagId < p`** (sequential or ≤160-bit random per §4.2) and **range-check `< p`** on every Poseidon input on-chain and in-circuit.
- **`purpose`/`recordType` (bytes32):** a 256-bit keccak label can exceed `p` → **reduce mod p once** at the field boundary (§5.2), canonical and CI-tested.
- **15-digit microchip & timestamps:** fit in one 31-byte limb (15 bytes / ~25 bytes) — single-field, no multi-field needed. **Long strings** (`distinctiveFeatures`, `taskDescription`) span multiple 31-byte limbs via `bytesToField`'s chunked fold — handled, with a schema-bounded `maxChunks` for the circuit.
- **Serialization:** R and every leaf are `< p < 2^254`, so they fit `bytes32` big-endian with the top ~2 bits always zero; never truncate.

---

## 8. Determinism test vectors (new, cross-language)

Extend `testvectors.json` (impl §1, CI-asserted across TS/Rust + a circom witness + a Foundry `PoseidonT*` deploy). New vectors required:

1. **`poseidon([1,2])` anchor** — the §2.2 value, asserted bit-identical in poseidon-lite, light-poseidon, circom, `PoseidonT3`. (Gate: any lib failing → version rejected.)
2. **`bytesToField` vectors** — empty bytes; `"a"` (1 byte); exactly 31 bytes; exactly 32 bytes (forces 2 limbs + verifies the 8-byte length prefix); a multi-hundred-byte string (`taskDescription`); and a UTF-8/NFC edge (combining accent → must equal its NFC form's image). Assert the same Field in TS/Rust/circom.
3. **`hashLeaf` vectors per typeTag** — null (tag 0), bool true/false (tag 1), string with NFC normalization (tag 2), big integer `985141006580311` (microchip, tag 3 — but note microchip.code is tag 2 string per impl §11.2(d)), decimal `22.7` and `0.5` (tag 4, pinned grammar), raw bytes (tag 5). Each asserts the same leaf Field across all langs and that `tag 2 "5" != tag 3 5`.
4. **`hashNode` / `buildMerkle` vectors** — single-leaf (root == leaf); two leaves (commutativity: swap order → same root); three leaves (odd promotion); selective-disclosure (drop a leaf's cleartext, keep its Field in the obfuscated set → same R). Assert R identical in TS/Rust and that the circom in-circuit recomputed root == SDK R.
5. **`nullifier` (PoseidonT7) vector** — a fixed `(dogTagId, purpose, relayer, subject, nonce)` with `purpose`'s keccak label > p (forces the mod-p reduction), asserted **identical in circom (as output signal), Solidity `PoseidonT7`, and Rust** — this is the audit-08/§13.8 parity gate that protects the shared `consumed` set.
6. **Domain-separation negative vectors** — assert `hashLeaf(...)` with components equal to a `hashNode` input's `(lo,hi)` does **not** collide (different arity + tag), and that a `bytesToField` output is never a valid leaf/node.
7. **Range-check vectors** — a `dogTagId`/`purpose` chosen `>= p`: the SDK/circuit/contract must reject or reduce canonically and identically.

CI fails the build on any cross-language divergence in any vector.

---

## Sources

- circomlib Poseidon circuit & parameters (t∈[2,16], R_F=8, per-t R_P, x^5, seed "poseidon"): https://github.com/iden3/circomlib/blob/master/circuits/poseidon.circom , https://docs.taceo.io/docs/examples/poseidon/
- poseidon-lite (TS, poseidon1..poseidon16, circomlibjs-compatible): https://github.com/chancehudson/poseidon-lite
- poseidon-solidity (PoseidonT2..T7, ~21k gas T3, circomlib-compatible, deterministic deploy): https://github.com/chancehudson/poseidon-solidity , https://www.npmjs.com/package/poseidon-solidity
- light-poseidon (Rust, BN254 circom-compatible `new_circom`, t=2..13, Veridise-audited): https://github.com/Lightprotocol/light-poseidon , https://docs.rs/light-poseidon/latest/light_poseidon/
- TaceoLabs poseidon-rust (Rust, circom-compatible, t=3/t=4): https://github.com/TaceoLabs/poseidon-rust
- circomlibjs (reference JS, version-pinning caveats): https://github.com/iden3/circomlibjs , issue #14 (hash changed), #30 (JS vs Solidity mismatch)
- circomlib test vectors (`poseidon([1,2]) = 0x115cc0f5…189a`): https://git.arnaucube.com/arnaucube/circomlib-testvectors/commit/e4217687a6208ece7d42ba474ee09e89e81ea32e , https://github.com/iden3/circuits/blob/master/test/poseidon.test.ts
- poseidon-in-circomlib cross-check (PSE): https://github.com/privacy-scaling-explorations/poseidon_in_circomlib_check
- Poseidon paper parameters (BN254, 128-bit, x^5): https://www.poseidon-hash.info/
- Prior DogTag research: research/10-zk-groth16.md (§2 parallel Poseidon leaf, EdDSA-BabyJubjub), research/02-attestation.md (OA salted-leaf lineage)
