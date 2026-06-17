# Audit 02 — Crypto / Canonicalization / Merkle Correctness & Determinism

> Scope: the salted-Merkle credential canonicalization in `architecture.md` §3/§5 and
> `implementation.md` §1 (`encodeValue`, `hashLeaf`, `buildMerkle`, `merkleProof`,
> `processProof`, `wrapDocument`, `obfuscate`, `verify`). **Crypto + canonicalization +
> Merkle only.** Goal: a spec tight enough that independent TS / Rust / Solidity
> implementations produce byte-identical roots and no forgery is possible.
>
> Auditor verdict and exec summary are at the bottom (and relayed separately).
>
> Severity scale: **Critical** (breaks security or guarantees cross-lang root divergence in
> realistic inputs) · **High** (will diverge or weaken security for plausible inputs) ·
> **Medium** (latent / edge-case divergence or hygiene) · **Low** (informational / defense-in-depth).

---

## A. Determinism gaps (TS / Rust / Solidity disagreement)

### A1. `canonicalDecimal` is unspecified — guaranteed cross-language divergence — **CRITICAL**

The spec says decimal is a "fixed decimal string … normalized (no trailing zeros beyond
significant, single canonical form)" and `encodeValue` calls `ascii(canonicalDecimal(value))`,
but **`canonicalDecimal` is never defined**. Every ambiguous case below produces a *different
leaf hash* depending on the implementation, so TS, Rust, and Solidity (and even two TS
versions) will disagree on the root for the same input. Decimals are core data (weight `"22.7"`,
titer `"0.5"`), so this is not theoretical.

Undefined cases, each of which must be pinned:

1. **Sign of zero / negatives:** is `-0` allowed? Is `-0.0` → `"0"`? Is a leading `-` placed where
   for `"-0.50"`? Integer rule bans `-0`; decimal rule is silent.
2. **Leading zero:** is it `"0.5"` or `".5"`? `"00.5"`? The §7.5 research note says "single leading
   zero `0.x`" but the normative spec (`implementation.md`) dropped that.
3. **Trailing zeros:** `"22.70"` → `"22.7"`, and `"22.0"` → `"22"`? Or `"22"`? Does a value that
   is integral after normalization stay typeTag `4` (DECIMAL) or could it be re-inferred as integer?
   If `"22.0"` decimal normalizes to `"22"` and an integer `22` also encodes `"22"`, then **only the
   type tag separates them** — fine, but the normalization rule must be explicit or one impl will
   keep `"22.0"`.
4. **Exponent / scientific notation:** is `1e3`, `1E3`, `2.5e-3` accepted on input? Must be rejected
   or expanded to a fixed-point form. JS `String(2.5e-3)` = `"0.0025"`, but `String(1e21)` =
   `"1e+21"` — a raw float path leaks exponent notation.
5. **Precision / max digits:** arbitrary precision like the integer rule? A decimal string is
   text, so arbitrary precision is feasible and should be mandated (no f64 round-trip).
6. **Internal whitespace / `+` sign / Unicode digits:** `"+22.7"`, `" 22.7"`, full-width digits.

**Fix — pin `canonicalDecimal` to a closed grammar over ASCII, operating on the input *string*
(never a float):**

```
canonicalDecimal(s) -> ascii string:
  # input MUST already be a string; assertNotFloat upstream guarantees no f32/f64 ever reaches here
  reject if s does not match:  ^-?(0|[1-9][0-9]*)(\.[0-9]+)?$
      # => no leading '+', no leading/trailing space, no exponent, no leading zeros in int part,
      #    no Unicode digits, fractional part (if '.' present) has >=1 digit
  strip trailing zeros in the fractional part
  if fractional part becomes empty, drop the '.'   # "22.0" -> "22", "22.70" -> "22.7"
  if result == "-0": result = "0"                  # no negative zero
  # NOTE: result may now be integral (e.g. "22"); the typeTag (4) still distinguishes it
  #       from an INTEGER leaf, because the tag byte is part of the preimage.
  return result
```

Add this exact regex and the strip/normalize steps to the normative spec, plus test vectors:
`"22.70" -> "22.7"`, `"22.0" -> "22"`, `"0.50" -> "0.5"`, `"-0.0" -> "0"`, `".5" -> reject`,
`"+1" -> reject`, `"1e3" -> reject`, `"00.5" -> reject`. Require all three SDKs to assert them.

> If you prefer to *forbid* integral decimals collapsing (keep `"22.0"` distinct), that is also
> valid — but **you must choose one** and write it down. The collapsing form above is recommended
> because it gives a single canonical representation per rational value at a given precision.

---

### A2. Wrap-time type inference from `typeof number` cannot distinguish int vs decimal — **CRITICAL**

`wrapDocument` does `typeTag = mapType(jsType, rawValue)` over a flattened credential. `mapType`
is never specified, and in JS/TS **`typeof 5 === typeof 5.0 === typeof 22.7 === "number"`** — there
is no way at runtime to tell an integer from a decimal, nor to recover the author's intent
(`5` vs `5.0`). Worse, `22.7` as an IEEE-754 double is `22.699999999999999…`; `String(22.7)`
happens to print `"22.7"` in JS but Rust `serde_json` may deserialize the same JSON number to
`f64` and a naive `to_string()` can differ, and **any value past 2^53 silently loses precision**
(microchip-style 15-digit IDs are < 2^53, but lot numbers or future fields may not be).

This means the *same logical credential* wrapped by the TS SDK vs the Rust SDK can get different
type tags (3 vs 4) and/or different value bytes ⇒ **different roots**. The architecture's headline
promise ("identical results in TypeScript, Rust, and Solidity") fails at the very first step:
deciding the type.

`assertNotFloat` actively fights this: if the input arrives as a JSON number it *is* an f64 in
both JS and Rust, so `assertNotFloat` would reject every numeric field — yet the example data
(`weightKg: 22.7`, `score: 5`) are JSON numbers. The function and the examples contradict each
other.

**Fix — never infer numeric type from a runtime float. Require typed input at the wrap boundary:**

- The wrap API must accept **explicitly typed scalars**, not raw JSON numbers. Two acceptable forms:
  1. A tagged input value, e.g. `{ t: "integer", v: "985141006580311" }` /
     `{ t: "decimal", v: "22.7" }` / `{ t: "string", v: "5" }`, where `v` is **always a string**
     for integer/decimal; or
  2. Drive types from the **schema** (each record type already has a field table in research/01):
     `mapType(keyPath)` looks up the declared type, and integer/decimal values are carried as
     strings end-to-end.
- `assertNotFloat` then becomes a real guard: reject any `number`/`f32`/`f64` that reached the
  encoder, with the message already in the spec. JSON parsing for credential *input* should use a
  parser that surfaces numbers as strings (e.g. parse with a big-decimal/raw-number reviver in TS;
  `serde_json::Number` kept as its source text, or a custom `Deserialize` to `String`, in Rust)
  **before** they are coerced to f64.
- Document that the canonical `data` packed string (`salt:typeTag:value`) is itself the source of
  truth on `verify()` (it is — verify re-parses the tag from the packed string), so the *only* place
  the int/decimal decision is made is wrap-time. That makes pinning the wrap-time input contract
  sufficient: verify never re-infers.

Without this fix the system is not deterministic across languages for any numeric field.

---

### A3. NFC normalization placement & on-chain asymmetry — **HIGH**

`hashLeaf` NFC-normalizes both `keyPath` and (for strings) the value. Good. But:

1. **Solidity cannot NFC-normalize** (no Unicode tables in the EVM). The spec acknowledges this for
   leaves ("issuer is responsible for NFC; on-chain just hashes bytes it's given") — but the
   architecture also describes a `MerkleVerifierLib` and "Solidity uses native keccak256" for *leaf*
   agreement in the §9 testing plan ("Solidity test that recomputes a node hash"). For **node**
   hashing Solidity is fine (it only sees 32-byte hashes). For **leaf** hashing Solidity must
   **never** be asked to build a leaf from a raw string — it can only re-hash already-normalized
   bytes supplied to it. Make this explicit: *Solidity participates at the node level only; any
   on-chain leaf check takes pre-NFC-normalized `valueBytes`/`keyPathBytes` as `bytes`, it does not
   normalize.* Otherwise the §9 "recompute a node hash to confirm on-chain agreement" test could be
   mistakenly extended to leaves and diverge.

2. **NFC must be applied identically in TS and Rust.** Pin the Unicode version. `String.prototype
   .normalize("NFC")` uses the ICU/Unicode version bundled in the JS engine; Rust
   `unicode-normalization` crate uses its own. Two different Unicode versions can normalize newly
   assigned codepoints differently. **Fix:** pin a Unicode version in the spec (e.g. "NFC per Unicode
   15.1"), pin the Rust crate version, and add cross-lang test vectors with known tricky inputs
   (precomposed vs decomposed: `"é"` U+00E9 vs `"e"+U+0301`; Hangul L/V/T jamo composition;
   the Angstrom sign U+212B → U+00C5; full normalization-form singletons). Assert TS and Rust agree.

3. **`assertNotFloat` and NFC do not cover NaN of strings**: also normalize/forbid lone surrogates
   and require valid UTF-8/UTF-16 input (invalid surrogate pairs serialize differently). Add: reject
   strings containing unpaired surrogates before normalization.

4. **Should you reject non-NFC input or silently normalize?** The spec wavers ("reject … or
   normalize and record"). For determinism it does not matter (both ends normalize), but for the
   *human-inspectable* `data` packed string it does: if the issuer stores the **original** (non-NFC)
   text in `data[keyPath]` but hashes the **NFC** form, then `verify()` re-reads the stored text,
   re-normalizes, and still matches — OK. But if any consumer compares the stored text byte-for-byte
   to something else it will mismatch. **Fix:** store the **NFC-normalized** value in the packed
   `data` string so stored == hashed-preimage (modulo salt/tag framing). State this normatively.

---

### A4. Integer encoding underspecified at the edges — **MEDIUM**

`integer` = "decimal ASCII, no leading zeros, no `-0`". Mostly good, but pin:

- Negative numbers: research §7.2 says "leading `-` for negatives"; the normative `implementation.md`
  line only says "no `-0`", implying negatives are allowed. Confirm: `-` allowed, `+` forbidden,
  `0` is `"0"`, leading-zero forms (`"007"`) rejected, no Unicode digits, no whitespace, arbitrary
  precision (parse from string, never via i64/f64). Add the same closed regex as decimals:
  `^-?(0|[1-9][0-9]*)$` with the extra rule `"-0"` is illegal (the regex already forbids `-0`).
- Microchip IDs are `^[0-9]{15}$` (schema) but stored as **type 2 (string)** in the example
  (`"…:2:985141006580311"`). Decide and document whether microchip is string or integer; the leaf
  hash differs by tag. The §3.2 example uses string (tag 2); keep it string (it has a fixed-width,
  leading-significant format and is a join key compared as text) and note that explicitly so an impl
  doesn't "helpfully" treat it as integer.

---

### A5. `len(...)` width, endianness, salt-as-bytes — **LOW (already correct; lock it down)**

`u32be` (4-byte big-endian) length prefixes and **raw 16-byte salt** (not hex) are specified and
correct, and they kill intra-leaf ambiguity. Two hardening notes:

- **Bound lengths to `uint32`:** a value/keyPath ≥ 4 GiB would overflow the prefix. Add an explicit
  assert `len < 2^32` (realistically also cap keyPath/value to something sane, e.g. 64 KiB, at
  ingest) so no impl wraps the length silently.
- **Salt is hashed raw, transported as hex in `data`.** Confirmed correct (§7.3). Add a test vector
  proving that the hex in `data` decodes to exactly 16 bytes and the *bytes* (not the hex text) are
  hashed. Reject `data` entries whose salt hex is not exactly 32 lowercase hex chars.

---

## B. Leaf hashing — domain separation, length-prefix completeness, second-preimage

### B1. Domain separation present and correct — **PASS** (with one gap, B2)

Leaf preimage starts with `0x00`, node preimage with `0x01`. Because a leaf preimage is
`0x00 ‖ u32(len kp) ‖ kp ‖ u32(16) ‖ salt ‖ tag ‖ u32(len v) ‖ v` (length ≥ 1+4+4+1+4 = 14 bytes
minimum plus 16-byte salt) and a node preimage is `0x01 ‖ 64 bytes` (exactly 65 bytes), the leading
byte alone separates the two domains. A 32-byte internal node hash can never be confused with a leaf
preimage because their first bytes differ and their structures differ. **This correctly defeats the
classic leaf/internal-node second-preimage attack that OA omits.** Good.

### B2. Every field is length-prefixed *except the type tag* — that is fine, but verify the framing is total — **PASS / NOTE**

`typeTag` is a single fixed-width byte with no prefix — unambiguous because its width is constant and
its position is fixed (always immediately after the 16-byte salt, which is itself fixed-width). The
domain separator `0x00` is likewise fixed-width-and-position. So the preimage is fully self-delimiting:
no two distinct tuples `(keyPath, salt, tag, value)` can serialize to the same bytes. **Confirmed
second-preimage-safe within a leaf.** No unprefixed *variable-length* field exists.

One concrete check to add to vectors: the empty-value cases. `null` (tag 0) → `len(v)=0`, and an
empty string (tag 2, `""`) → `len(v)=0`. These two differ **only by the tag byte** (0 vs 2), which is
in the preimage, so they hash differently. Good — but add a vector to lock it (`null` vs `""` vs
`false`/`0x00` which is tag 1 len 1). Also `bool false` (tag 1, value `0x00`, len 1) vs an integer
`0` is tag 3 value `"0"` len 1 vs a 1-byte `bytes` `0x00` (tag 5 len 1) — all distinct by tag. Lock
with vectors.

### B3. Salt entropy / second-preimage across leaves — **PASS**

16 random bytes (128-bit) per field defeats hash-guessing of low-entropy values (the threat model in
§7.3 / question 7). Two leaves with different `(keyPath, value)` essentially never collide (keccak256
+ 128-bit salt). See C3 for what happens *if* two leaf hashes did collide. Confirm CSPRNG mandated:
`random16()` must be a cryptographic RNG (`OsRng`/`crypto.getRandomValues`), not `Math.random`/`rand`.
**Add this as a normative MUST** — it is currently only implied.

---

## C. Merkle build — odd-node promotion, commutative sortPair, collisions, empty/single

### C1. "Promote lone odd node unchanged" + commutative hashing creates *structural proof ambiguity* — **HIGH**

The promote-odd rule means a node value at level *k* can appear unchanged at level *k+1* (and
further). Combined with **commutative `sortPair`** and **no left/right position bits in the proof**,
this is the OpenAttestation tree shape, and it has a known property: **the same set of leaves can
admit more than one valid `(leaf, proof)` → root reduction**, and more importantly **a `processProof`
verifier accepts any ancestor chain regardless of tree shape.** Concretely:

- `processProof(proof, leaf)` just folds `h = hashNode(h, s)` over the sibling list. It does **not**
  know the tree's arity, which nodes were promoted, or the leaf's index. So a prover can present a
  `proof` whose *length and contents* differ from what `merkleProof` would produce, as long as the
  fold lands on the committed root.
- Because pairs are sorted, the fold is order-independent: `hashNode(hashNode(leaf, A), B)` and any
  permutation reaching the same root verify. This is the documented OA behavior and is generally
  considered acceptable **for batch inclusion proofs** (you only claim "this leaf is under this
  root"), but it is **not** a proof that the leaf is at a particular position or that the tree has a
  particular shape.

Why this matters here specifically: for **single-document** verification you do **not** use
`processProof` for inclusion at all — you *rebuild the whole tree* from the full leaf set and compare
to `targetHash` (pillar 1). That rebuild is unambiguous (see C2). So the ambiguity is **dormant in
v1**. It becomes a real concern **when batching is enabled** and `proof` is non-empty, because then
inclusion rests solely on `processProof`. See E2 for the batch-time forgery surface.

**Fix (now, cheap):** document that `processProof` is an *inclusion* check only (membership under a
root), never a structural/position proof, and that **single-doc verification MUST NOT rely on
`processProof`** — it must rebuild and compare `targetHash` (the spec already does this; make it a
stated invariant). **Fix (before batching):** see E2 — bind tree size/index, or accept the OA
second-preimage caveat explicitly and mitigate by domain-separating leaf vs node (already done, which
already blocks the *cross-type* forgery; the residual is intra-node reshaping, mitigated by also
encoding subtree size — see E2).

### C2. Single-doc tree rebuild is deterministic and unambiguous — **PASS**

For pillar-1 integrity the verifier sorts the full leaf set bytewise and rebuilds with the fixed
pairing rule (pairs left-to-right, promote trailing odd). Given a fixed multiset of leaf hashes and
the single documented comparator, the tree shape and root are **uniquely determined**. No ordering
dependence on original field order (leaves are sorted first). **This is correct and is the core
guarantee.** Two requirements to lock:

- **One comparator everywhere:** unsigned bytewise ascending over the raw 32 bytes, used for (a) leaf
  pre-sort, (b) `sortPair`, (c) any proof reduction. The spec says this; add it as a single
  normative definition referenced from all three call sites, and a vector with adversarially-close
  hashes (differing only in the last byte, and differing in the first byte) to catch a signed-compare
  bug (a classic Rust `i8`/`u8` or JS `Buffer.compare` vs `<` mistake).
- **Pairing direction:** "pairs are `(level[0],level[1]), (level[2],level[3]), …`, trailing odd
  promoted." This is the build order; because `sortPair` is commutative the *pairing grouping*
  (which two are siblings) is what matters, and it is fully determined by the post-sort index. Lock
  with a 3-leaf and 5-leaf vector (both exercise promotion).

### C3. Duplicate / colliding leaf hashes — **MEDIUM**

`buildMerkle` explicitly does **not** dedupe ("salts make them unique"). Two sub-cases:

1. **Accidental duplicate (same leaf hash twice).** With per-field random salts this requires either
   a salt collision (negligible) or **salt reuse** (a buggy `random16` or a copy-paste). If two leaves
   are byte-identical, after sorting they are adjacent and `hashNode(x, x) = keccak256(0x01 ‖ x ‖ x)`
   — a valid, deterministic node. No crash, root is well-defined. The risk is purely the *upstream*
   bug of salt reuse, which also weakens the hiding property of obfuscation (an attacker who learns
   one field's salt learns nothing about another only if salts are independent). **Fix:** mandate
   unique CSPRNG salt per field (restate B3) and optionally assert no duplicate leaf hashes at wrap
   time as a cheap salt-reuse canary: `assert leaves.len() == unique(leaves).len()` in `wrapDocument`.
2. **Adversarial duplicate to manipulate shape.** Because `verify()` rebuilds from the full set, an
   attacker cannot inject a duplicate without it being in `data ∪ obfuscated` and thus changing the
   recomputed root (unless it collides with keccak256 — negligible). No new attack beyond D/E.

### C4. Empty tree and single-leaf tree — **PASS, with one gap**

- **Empty tree:** `buildMerkle` errors on empty input. Good — never let the root be a predictable
  constant (e.g. `0x00…0`) that the contract might treat as "issued". `DogTagIssuer.issue` also
  rejects `issuedAt[0x0]` only implicitly (it would happily issue `0x0` — see note). **Fix:** add an
  explicit `require(root != bytes32(0))` in `issue`/`bulkIssue` on-chain *and* reject empty/zero leaf
  sets in the SDK, so a zero root can never be anchored even if some caller bypasses `buildMerkle`.
- **Single-leaf tree:** `targetHash = that one leaf hash` (the `while len(level) > 1` loop doesn't
  run). Correct and matches §3.4. **Gap:** a single-leaf doc's `targetHash` is *exactly a leaf hash*
  (domain `0x00`). For single-doc, `merkleRoot == targetHash`, so the **anchored root is a leaf
  hash**. Domain separation means it can't be confused with a *node*, but note that for a 1-field
  credential the on-chain root reveals nothing extra and is safe. When batching, a document
  `targetHash` (which may be a single leaf hash) becomes a *leaf of the batch tree* — and there the
  batch tree hashes it with node domain `0x01`, so a document-root (leaf-domain `0x00`) can never be
  reinterpreted as a batch-internal node (`0x01`). Domain sep saves you again. **Confirm and vector
  the 1-leaf case.**

### C5. `merkleProof` skip-on-promotion logic is subtly wrong vs the build — **MEDIUM**

`merkleProof` computes the sibling as `idx ^ 1` and pushes `layers[L][sib]` only if `sib <
len(layers[L])`, then `idx = idx >> 1`. This assumes a node's parent index is `idx >> 1` and its
sibling is `idx ^ 1`. **But the build does not always pair `(2i, 2i+1)` into parent `i` when an odd
promotion happened at a *lower* level**, because promotion changes the count but the *next level is
still indexed 0..n* — actually the build re-indexes each `next` level densely (0,1,2,…), so a promoted
node at the end of level L lands at the end of level L+1, and `idx>>1` tracking can desync from the
dense re-indexing. Walk it:

- Level L (5 nodes): pairs (0,1)→p0, (2,3)→p1, 4 promoted→p2. Level L+1 has 3 nodes [p0,p1,p2].
- A leaf at index 4 (the promoted one): `sib = 4^1 = 5`, `5 < 5` is false ⇒ no sibling pushed,
  `idx = 4>>1 = 2`. Next level it is index 2 = p2. Correct.
- A leaf at index 3: `sib = 2`, push `layers[L][2]`, `idx = 1`. p1 is at index 1. Correct.

For this particular dense re-indexing the `idx>>1` / `idx^1` arithmetic **does** stay aligned because
the promoted node is always the *last* one and `last_index >> 1` equals its new last index when the
level length is odd. So the logic is **correct for trailing-only promotion** — but it is fragile and
relies on promotion only ever occurring at the tail. **Fix:** add an assertion/test that
`merkleProof` ⊕ `processProof` round-trips to the root for **every** leaf of trees of size 1..9
(sizes 3,5,6,7,9 all exercise promotion at one or more levels). This is the cheapest guard against an
off-by-one in any of the three ports. Also note: `merkleProof` is only needed for batching (single-doc
proof is empty); still test it.

---

## D. Selective disclosure / obfuscation — forgery surface

### D1. An attacker can ADD fake obfuscated hashes to change the root — but the **anchored root won't match**, so it fails issuance — yet integrity-only checks are foolable — **HIGH**

`verify()` pillar 1 computes `leaves = (hashes of data fields) ∪ doc.privacy.obfuscated` and rebuilds.
Nothing constrains `privacy.obfuscated` to be hashes that were *actually in the original tree*. So:

- **Adding** an arbitrary 32-byte value to `privacy.obfuscated` changes the recomputed leaf set ⇒
  changes the rebuilt root ⇒ `root != targetHash` ⇒ **integrity fails.** Good — you cannot add junk
  and keep `targetHash`.
- **But** an attacker can *also* change `targetHash` and `merkleRoot` in the wrapped doc to the new
  rebuilt root. Then **pillar 1 passes** (it only checks internal consistency: rebuilt root ==
  `targetHash`, and empty-proof ⇒ `merkleRoot == targetHash`). The forgery is only caught by **pillar
  2 (issuance):** the new `merkleRoot` was never `issue()`d on-chain ⇒ `isValid` false ⇒ overall
  INVALID. **Security therefore depends entirely on pillar 2 binding the root.** This is by design and
  is OK **only if no consumer ever trusts pillar-1-alone.**

**Findings / fixes:**

1. **Document the invariant loudly:** *integrity (pillar 1) proves the document is internally
   self-consistent; it proves NOTHING about authenticity. Authenticity = pillar 2 (the on-chain root
   was issued by a whitelisted signer) + pillar 3 (DNS binds domain→contract). A verifier MUST treat
   a doc as valid only if all three pass.* The SDK's `verify()` returns the combined `valid` (good),
   but the per-fragment fields invite a caller to short-circuit on `integrity` — add a doc comment /
   API note that `fragments.integrity == true` alone is meaningless for trust.
2. **`obfuscated` must be well-formed:** each entry must be exactly 32 bytes / 64 lowercase hex.
   Reject malformed entries (otherwise a length-confused impl might hash hex text). Add to spec.
3. **De-dup vs real leaves:** an attacker could move a *real* current field's hash into `obfuscated`
   while ALSO leaving the cleartext in `data` — then that leaf appears twice in the recomputed set
   (once from data, once from obfuscated). After sort they're adjacent and the tree changes (now N+1
   leaves) ⇒ root changes ⇒ caught by `targetHash`/issuance. Not a root-preserving attack, but
   **reject it at verify time** for clarity: a hash in `obfuscated` that also equals a live data
   leaf's hash is malformed. (Cheap, optional.)

### D2. Removing a field to change meaning while keeping the root — **PASS (cannot keep root), but a *semantic* gap exists** — **MEDIUM**

To remove a field and keep the same root, you must put its leaf hash into `obfuscated` (that is the
legitimate selective-disclosure path; the root is preserved because the leaf set is unchanged). An
attacker who wants to *delete* a field outright (drop both the cleartext and its obfuscated hash)
changes the leaf set ⇒ different root ⇒ issuance fails. **So you cannot remove a field and keep a
valid root.** Good.

**However**, the legitimate obfuscation path means a holder *can* hide any field, including ones a
relying party assumed present. Example: a credential asserts `validUntil` and `titer ≥ 0.5`; the
holder obfuscates `titer`. The root still verifies and the doc is "valid", but the *semantics the
verifier sees* are weaker than what the issuer signed. This is inherent to OA-style selective
disclosure and is acceptable, **but the relying party must enforce a `required-fields` policy per
record type** (e.g. EU rabies requires product/manufacturer/batch present and NOT obfuscated).
**Fix:** the SDK `verify()` (or a thin policy layer above it) should accept a `requiredPaths` set per
`recordType` and return INVALID/INCOMPLETE if any required field is missing or obfuscated. Without
this, "valid" ≠ "compliant." This is a **policy/spec** gap, not a crypto break — flagging because the
schema invariants in §3.6/§1.6 are enforced at *issuance* but not re-checked against obfuscation at
*verify*.

### D3. Can someone swap a field's value if they know the salt? — **PASS**

If an attacker knows `(keyPath, salt, tag)` of a field and wants to substitute `value'`, they compute
a *new* leaf hash; the leaf set changes ⇒ rebuilt root changes ⇒ `targetHash`/`merkleRoot` change ⇒
issuance fails (new root not issued). Knowing the salt does **not** let you forge a value under the
same root — keccak256 preimage resistance + the on-chain root binding prevent it. The salt's only job
is to stop *guessing* the value of an **obfuscated** (hidden) field; it is intentionally stored in
cleartext for *disclosed* fields (the holder needs it to prove the field). **Confirmed correct
threat model.** One note: when a field is obfuscated, its salt is **removed** from `data` along with
the cleartext (it lives only inside the now-hidden leaf), which is exactly why the value stays
unguessable — verify this is what `obfuscate` does. `obfuscate` deletes `doc.data[kp]` (the whole
packed `salt:tag:value` string), so yes the salt is removed. **Good.**

---

## E. proof / targetHash / merkleRoot relationship & verification bypass

### E1. Empty proof always "passes" `processProof` — correct *because* root must still equal targetHash — **PASS**

`processProof([], leaf) = leaf`. So with an empty proof, pillar 1's second clause becomes
`targetHash == merkleRoot`. For single-doc that is the spec's invariant (`merkleRoot == targetHash`).
So an empty proof does not *bypass* anything: it forces `merkleRoot == targetHash`, and `targetHash`
is independently pinned by the full-tree rebuild. **No bypass.** The only way to make pillar 1 pass is
to have `data ∪ obfuscated` rebuild to `targetHash` AND `merkleRoot == targetHash`; then pillar 2
must still find `merkleRoot` issued on-chain. Good.

### E2. Commutative `processProof` lets a prover craft *a* proof to a different root — relevant only at batch time — **HIGH (for the future batch design; flag now)**

As noted in C1, `processProof` is a permissive inclusion fold. At batch time, inclusion of a document
in a batch rests **only** on `processProof(proof, targetHash) == merkleRoot`. Two known issues with
sorted-pair, position-free Merkle proofs:

1. **Second-preimage via node/leaf confusion** — already mitigated here by the `0x00`/`0x01` domain
   separation (a 64-byte concat of two child hashes can't be reinterpreted as a single leaf). OA does
   *not* domain-separate; you do. **This closes the most cited sorted-Merkle forgery.** Good.
2. **Shape malleability / forged inclusion within the node domain** — even with domain sep, a sorted,
   sizeless proof does not bind the *number of leaves* or the leaf's *position*. An attacker who
   controls some batch leaves could, in principle, present a crafted proof for a value that was not a
   real document leaf if they can find sibling hashes that fold to `merkleRoot`. With keccak256 this
   requires a preimage/collision and is infeasible *for random siblings*, but the lack of size-binding
   removes a defense-in-depth layer and is the reason production sorted-Merkle libs (and RFC 6962)
   either forbid promotion, bind tree size, or include index bits.

**Fixes for the batch design (do before enabling non-empty proofs):**

- **Bind subtree size into each node:** `hashNode(a, b) = keccak256(0x01 ‖ u32be(size) ‖ sortPair(a,b))`
  where `size` = number of leaves under this node — OR include the leaf **count** in the leaf domain
  and a level/index in the node domain. This makes a forged-shape proof require matching the committed
  size, eliminating promotion-based ambiguity. (RFC 6962 / certificate-transparency style.)
- **Alternatively, drop commutativity for the batch tree** and carry left/right position bits in the
  proof (standard ordered Merkle). You lose the "no left/right bit" convenience but gain unambiguous
  inclusion. Given the §3.4 design leans on commutativity for a tiny Solidity verifier, the
  size-binding option is the smaller change.
- **Keep single-doc as-is** (full rebuild, empty proof) — it is not exposed to E2.

Mark this clearly as **a v2/batch requirement**; v1 single-doc is safe because it never trusts
`processProof` for inclusion.

### E3. `processProof` / rebuild use the *same* `hashNode` — consistency — **PASS**

Both `buildMerkle` and `processProof` call `hashNode` (domain `0x01` + `sortPair`). So a proof
generated by `merkleProof` over the build layers reduces with identical hashing. No comparator/domain
mismatch between build and verify (the OA bug class of "different ordering at digest vs tree level"
called out in research §7.1 is **avoided** — there is one tree, one comparator). Good.

---

## F. `verify()` pillar logic

### F1. Integrity cannot pass with mismatched data — **PASS** (subject to A1/A2)

Pillar 1 re-derives every leaf from the packed `data` strings (re-parsing salt/tag/value) ∪
`obfuscated`, rebuilds, and compares to `targetHash`. If any disclosed field's value, salt, tag, or
keyPath differs from what produced `targetHash`, the rebuilt root differs ⇒ integrity fails. So
integrity **cannot** pass with mismatched disclosed data. **Provided** the encoder is deterministic —
which is exactly what A1 (decimal) and A2 (int/decimal typing) threaten. If those are unfixed, a
*correctly transcribed* document can fail integrity in one language while passing in another. So F1 is
correct **conditional on A1/A2 being fixed.**

### F2. Recomputed leaf set is exactly {data leaves} ∪ {obfuscated} with no ordering dependence — **PASS, lock the parse** 

`verify()` builds `leaves` from `flatten(doc.data)` then `++ doc.privacy.obfuscated`, and `buildMerkle`
sorts first — so **field/insertion order is irrelevant** (sorted bytewise). Good. Two locks:

- **`flatten` must be deterministic and total**, and must reproduce the *same keyPath strings* used at
  wrap time (the keyPath is hashed). Pin the path grammar (dot for object keys, `[i]` base-10 no
  leading zeros for arrays — research §7.5 #8) and **forbid `.`, `[`, `]` inside keys** in both
  `wrapDocument.flatten` and `verify.flatten`. A mismatch here changes the hashed keyPath ⇒ root
  mismatch. **This is a real cross-lang risk** (JS `flatten` libs vs a Rust impl differ on arrays,
  empty objects, numeric keys). Specify the exact flatten/unflatten algorithm and add nested + array
  + empty-container vectors. **Recommend HIGH attention** — call it out:

  > **F2a (HIGH): `flatten`/`unflatten` is as load-bearing as the hash and is currently unspecified.**
  > Two implementations that disagree on how arrays, empty objects/arrays, `null` holes, or numeric
  > string keys flatten will produce different keyPaths and different roots. Pin the grammar and
  > algorithm exactly; add vectors for `a.b`, `a[0]`, `a[0].b`, empty `{}`/`[]` (define: emit nothing,
  > so they contribute no leaf — and therefore are invisible to integrity; if structure must be bound,
  > add an explicit sentinel leaf), and keys containing reserved chars (reject at wrap).

- **`parse(packed)` must be strict:** split into exactly `salt(32 hex) : tag(1-2 digits) : value`,
  where **value may contain `:`** (e.g. a timestamp `2026-06-17T08:00:00`). The packed format
  `hex(salt) + ":" + typeTag + ":" + asString(value)` uses `:` as delimiter but values can contain
  `:`. **Fix:** parse by splitting on the **first two** colons only (`salt`, `tag`, then the
  remainder verbatim is `value`), exactly like OA's "join the rest back". State this normatively and
  vector a value containing colons. Otherwise a value with `:` round-trips wrong in one impl ⇒ root
  mismatch or parse error. **MEDIUM→HIGH** depending on how many fields are timestamps (several are).

### F3. Pillar combination — **PASS**

`valid = integrity && issuance && identity`. Matches the OA "all groups VALID, none INVALID" model.
No pillar can be skipped to reach `valid=true`. Note the function does no error/`SKIPPED`
state (OA distinguishes ERROR/SKIPPED) — if `rpc`/`dns` throw, the booleans are presumably false; make
sure a *network error* is not silently treated as `false`→INVALID in a way that lets a transient
outage mark a real credential invalid, nor (worse) caught-and-defaulted to `true`. **Fix:** define the
tri-state (VALID/INVALID/ERROR) for pillars 2/3 and require `valid` only on explicit VALID; surface
ERROR distinctly so the UI can say "couldn't check" rather than "forged." Crypto pillar 1 is
pure/offline so it is binary. **LOW (correctness/UX), but specify it.**

---

## G. Salt threat model (question 7) — **PASS, confirmed**

- **16 bytes (128-bit) is enough.** The salt's job is to make a *hidden* (obfuscated) field's value
  unguessable from its leaf hash. 128 bits defeats brute force over even tiny value domains
  (yes/no, dates) because the attacker must also guess 128 random bits. Equivalent to a UUIDv4's
  ~122 bits, slightly better. No need for 32 bytes.
- **Stored in cleartext in `data` — fine.** For *disclosed* fields the value is already visible, so
  the salt's secrecy is irrelevant; the holder needs the salt to let a verifier recompute the leaf.
  For *obfuscated* fields the salt is **removed** along with the value (D3), so it is not in cleartext
  where it matters. Correct design.
- **One caveat (restating B3/C3):** salts MUST be independent CSPRNG draws per field. Reuse breaks the
  hiding property (an attacker seeing one disclosed field's salt must learn nothing about another's
  hidden value) and risks duplicate leaves. Make CSPRNG + per-field uniqueness a normative MUST and
  add a wrap-time duplicate-leaf canary.

---

## H. Summary table

| ID | Area | Severity | One-line |
|----|------|----------|----------|
| A1 | `canonicalDecimal` undefined | **Critical** | Decimals will hash differently across TS/Rust/Sol; pin a closed grammar + normalization. |
| A2 | int vs decimal from `typeof number` | **Critical** | Cannot tell int from decimal at runtime; require typed/string numeric input, never f64. |
| A3 | NFC placement & Solidity asymmetry | High | Pin Unicode version; Solidity hashes at node-level only; store NFC form in `data`. |
| A4 | integer edge rules | Medium | Pin sign/leading-zero/precision regex; decide microchip is string. |
| A5 | u32 width / raw-salt | Low | Correct; add length bound + salt-hex validation. |
| B1/B2/B3 | leaf domain sep, framing, salt | Pass | Domain sep + length-prefix make leaves second-preimage safe; mandate CSPRNG salt. |
| C1 | odd-promotion + commutative ambiguity | High | `processProof` is inclusion-only; single-doc must rebuild (it does). Risk is batch-time. |
| C2 | single-doc rebuild determinism | Pass | Unique root per leaf multiset; lock one comparator + vectors. |
| C3 | duplicate/colliding leaves | Medium | No dedupe is fine; add salt-reuse canary. |
| C4 | empty / single-leaf | Pass | Reject empty; reject zero root on-chain; vector 1-leaf. |
| C5 | `merkleProof` skip-on-promotion | Medium | Correct only for trailing promotion; round-trip test sizes 1..9. |
| D1 | add fake obfuscated hashes | High | Can't keep root; but pillar-1-alone is foolable — document "integrity ≠ authenticity." |
| D2 | obfuscate to weaken semantics | Medium | Can't keep root by deletion; but holder can hide required fields — add `requiredPaths` policy. |
| D3 | swap value knowing salt | Pass | Cannot forge under same root; salt only hides obfuscated values. |
| E1 | empty proof "passes" | Pass | Forces `merkleRoot==targetHash`; no bypass. |
| E2 | crafted proof to different root (batch) | High | Sizeless sorted proof lacks size/position binding; bind subtree size before batching. |
| E3 | build vs verify hash consistency | Pass | Same `hashNode`; OA dual-ordering bug avoided. |
| F1 | integrity vs mismatched data | Pass* | Holds *iff* A1/A2 fixed. |
| F2a | `flatten`/`unflatten` unspecified | **High** | Path grammar/flatten is as load-bearing as the hash; pin it + vectors. |
| F2b | `parse(packed)` colon-splitting | High | Split on first two colons only; values contain `:`. |
| F3 | pillar tri-state | Low | Define VALID/INVALID/ERROR; don't treat network error as forged or as valid. |
| G | salt 16B / cleartext | Pass | Threat model correct; mandate per-field CSPRNG uniqueness. |

---

## I. Concrete normative additions to fold into the spec

1. **`canonicalDecimal` grammar** (A1) — verbatim regex + strip rules + vectors.
2. **Numeric input contract** (A2) — integer/decimal carried as **strings**; `assertNotFloat` rejects
   any float reaching the encoder; JSON input parsed with raw-number preservation.
3. **Unicode/NFC pin** (A3) — "NFC per Unicode X.Y", pinned Rust crate version, reject unpaired
   surrogates, store NFC form in `data`. Solidity = node-level only.
4. **One comparator** (C2) — "unsigned bytewise ascending over 32 bytes" defined once, referenced by
   leaf-sort, `sortPair`, proof reduction.
5. **`flatten`/`unflatten` + keyPath grammar** (F2a) — exact algorithm; reserved chars rejected;
   empty-container behavior defined; vectors.
6. **`parse` rule** (F2b) — split on first two `:` only.
7. **Salt MUST** (B3/G) — CSPRNG, 16 bytes, unique per field; wrap-time duplicate-leaf canary.
8. **`obfuscated` well-formedness** (D1) — each entry 32 bytes/64 lc hex; reject overlap with live
   leaves.
9. **`integrity ≠ authenticity`** API note (D1) — callers must require all three pillars.
10. **`requiredPaths` policy hook** (D2) — per-recordType fields that must be present & not obfuscated.
11. **On-chain zero-root reject** (C4) — `require(root != 0)` in `issue`/`bulkIssue`; SDK rejects
    empty/zero leaf sets.
12. **Batch-tree size binding** (E2) — `hashNode = keccak256(0x01 ‖ u32be(subtreeLeafCount) ‖
    sortPair)` OR ordered proofs, before enabling non-empty `proof`.
13. **Pillar tri-state** (F3) — VALID/INVALID/ERROR; network errors are ERROR, not INVALID/VALID.
14. **Length bound** (A5) — assert all `len < 2^32` (and sane ingest caps).
15. **Round-trip test matrix** (C5) — `merkleProof`∘`processProof` for every leaf, tree sizes 1..9.

These 15 items, plus the cross-language `testvectors.json` already planned in §9, are what make the
spec tight enough for three independent implementations to agree byte-for-byte and for no
root-preserving forgery to exist.
