# audit-10 — Poseidon Unification (v4) crypto audit: determinism + soundness

> Scope: the v4 hash-unification (CHANGESPEC-v4 + research/13). The credential commitment
> (leaf + Merkle + nullifier) is unified on **one** Poseidon root `R` over BN254 (pinned
> circomlib instantiation); the Groth16 circuit proves `R` directly; the dual-root
> `rKec`/`rZk` machinery + `zkCommit` are **deleted**. keccak is retained only for
> EIP-712/ECDSA digests, address derivation, and pure namespacing keys.
>
> Audited against: arch §3.3/§3.4/§3.6/§5/§13.2/§13.8, impl §1.2/§1.3/§1.4/§11.2/§11.8/§11.9,
> CHANGESPEC-v4, research/13. Regression baselines: audit-02-crypto, audit-05-crypto-v2,
> audit-07-zk-v3, audit-08-contracts-v3.
>
> Severity legend: **Critical** (breaks soundness/determinism/safety) · **High** (exploitable
> or determinism-divergent under realistic conditions) · **Medium** · **Low/Note**.

---

## 0. One-line verdict

The unification is **structurally sound and a real soundness simplification** — packing is
injective and the leaf/node/byte/nullifier domains are non-confusable — **but it MUST NOT
ship until two Critical determinism gaps are closed: (P-C1) the cross-language Poseidon parity
gate needs per-arity anchor vectors (t=2,3,6,7), not just `poseidon([1,2])`; and (P-C2) the
`purpose = keccak % p` reduction and `dogTagId/nonce < p` range-checks must be byte-identically
enforced in circom AND Solidity AND the SDK, with a CI negative vector, or the shared `consumed`
set is bypassable.**

---

## 1. Critical findings (every Critical + fix)

### P-C1 (Critical) — single anchor vector `poseidon([1,2])` is INSUFFICIENT for 4-lang × 4-arity parity

CHANGESPEC §1/§2.2, impl §11.2(d), arch §13.2/§13.8 all gate CI on **one** vector:
`poseidon([1,2]) = 0x115cc0f5…189a`. That vector exercises **only t=3 (2 inputs)**. The system
actually uses **four arities**: t=2 (the `bytesToField` fold), t=3 (Merkle node + `[1,2]`),
t=6 (leaf: `[DS_LEAF, kp, salt, tag, val]`), and t=7 (nullifier:
`[DS_NULLIFIER, dogTagId, purpose, relayer, subject, nonce]`).

Poseidon's round constants, MDS matrix, and **partial-round count `R_P` are per-`t`** (impl §11.2(b)
correctly lists `t=2→56, t=3→57, t=5→60, t=6→60, t=7→…`). A library can be bit-identical at t=3
and **still diverge at t=6 or t=7** — the most common real drift modes are exactly the per-`t`
constant/`R_P` tables and the off-by-one in the t→nInputs convention (see P-C3). circomlibjs
issues #14 (hash changed) and #30 (JS vs Solidity) — cited in the spec itself — were *not* t=3
problems in general. A green `poseidon([1,2])` gate gives **false confidence** that t=6/t=7
match, which is precisely where the leaf root `R` and the nullifier live.

This is the single highest determinism risk because a t=6/t=7 mismatch is **silent**: issuance
(off-chain TS/Rust) and proving (circom/Rust) would all agree with each other, while the on-chain
`PoseidonT7` nullifier (Solidity) could diverge — breaking the shared `consumed` set exactly as
audit-07 H-3 / audit-08 V3-C1 warned, with no test catching it.

**Fix (NORMATIVE):** the CI parity gate MUST assert bit-identical output across circom /
poseidon-lite / light-poseidon(`new_circom`) / poseidon-solidity for **every arity actually used**,
with a pinned anchor per arity:
- t=2: `poseidon([1])` (the fold base case) — assert all 4 libs (note: `PoseidonT2` must exist in poseidon-solidity for the on-chain path if any on-chain byte-fold is ever added; otherwise t=2 parity is TS/Rust/circom only).
- t=3: `poseidon([1,2]) = 0x115cc0f5…189a` (existing).
- t=6: a pinned `poseidon([1,2,3,4,5])` vector (the leaf arity).
- t=7: a pinned `poseidon([1,2,3,4,5,6])` vector (the nullifier arity) — **this one is load-bearing
  for the cross-path `consumed` set and MUST be asserted against deployed `PoseidonT7`**.
Plus the §8 leaf/Merkle/nullifier end-to-end vectors. Any lib failing any arity at its pinned
version is rejected at the lockfile gate. Compute the canonical values once from circomlib and
freeze them in `poseidon-vectors.json`.

### P-C2 (Critical) — the `purpose`/`dogTagId`/`nonce` field-reduction & range-check must be byte-identical in all 3 environments, with a negative vector, or the shared `consumed` set is bypassable

The nullifier is computed in **three** places that MUST agree bit-for-bit (impl §11.8(a),
§11.9(b), arch §4.7):
1. circom circuit (ZK path, public-signal output);
2. Solidity `PoseidonT7` (normal path, impl §11.8(a) line 1497);
3. Rust prover / SDK (witness generation).

Two of the six inputs can exceed `p` and require a reduction/range-check whose definition MUST be
identical everywhere:
- **`purpose`** is `keccak256(label)` → a full 256-bit value that **can be `> p`**. Solidity does
  `uint256(c.purpose) % SNARK_SCALAR_FIELD` (impl §11.8(a) line 1496). The circuit takes `purpose`
  as a **public input** already reduced (impl §11.8(d) "keccak label reduced mod p"). The Rust
  prover must feed the **same** reduced value. If any one of the three uses the raw 256-bit value,
  or reduces against the wrong modulus (BN254 **scalar** field `r`, not the base field `q` — both
  are ~254-bit and easy to confuse), the nullifiers diverge and a malicious relayer can record the
  **same logical attestation once per path** (defeating "one consent = one attestation"), or
  poison the set so a legitimate second purpose can't be recorded.
- **`dogTagId`** (uint256 token id) and **`nonce`** (uint256) must be `< p`. The spec says allocate
  `dogTagId < p` (§7.3) and range-check, but **the normal-path Solidity `recordVerification` does
  NOT range-check `c.dogTagId`/`c.nonce < p` before the `PoseidonT7` call** (impl §11.8(a) — only
  the ZK path range-checks all 7 public signals, line 1662). If a `dogTagId ≥ p` is ever minted (or
  a `nonce ≥ p` chosen), the Solidity `PoseidonT7` reduces it mod `p` internally (or reverts,
  library-dependent) while the EIP-712 digest and the SBT use the **full** uint256 — so
  `ownerOf(c.dogTagId)` is checked on the full id but the nullifier is computed on `id mod p`.
  Two distinct dogTagIds congruent mod `p` then share a nullifier (a cross-pet collision), and the
  normal-path nullifier can mismatch the circuit's range-checked value.

This is the regression of audit-07 H-3 ("input packing unasserted") and audit-08 V3-C1 ("normal-path
nullifier domain/encoding mismatch") into the v4 design: unification removed the dual-root binding
but **re-located** the parity-of-encoding risk into the nullifier's field-reduction.

**Fix (NORMATIVE):**
1. Pin a single `toField(x)` reduction: `purpose_field = uint256(keccak_label) mod r` where `r` is
   the **BN254 scalar field** (`21888242871839275222246405745257275088548364400416034343698204186575808495617`).
   State explicitly it is `r` (scalar), never `q` (base). Define it once, reference from circom,
   Solidity, Rust.
2. Add `require(c.dogTagId < SNARK_SCALAR_FIELD && c.nonce < SNARK_SCALAR_FIELD)` to the **normal
   path** `recordVerification` *before* the `PoseidonT7` call (the ZK path already range-checks all
   7 signals). Mint-time `require(dogTagId < r)` in `DogTagSBT.mint` as defense-in-depth (it is
   currently only a prose "allocate < p", not an on-chain guard).
3. CI vector #5 (§8) MUST use a `purpose` label whose keccak **exceeds `r`** (forces the reduction)
   and assert circom-output == `PoseidonT7` == Rust are bit-identical, **and** a negative vector
   where two dogTagIds `id` and `id + r` produce the *same* nullifier — to prove the range-check
   rejects rather than silently collides.

---

## 2. High findings (every High + fix)

### P-H1 (High) — `bytesToField` length-prefix is capped at 8 bytes (2^64); document the cap is unreachable AND assert the limb count is bounded so the fold cannot be length-extended

`bytesToField` prepends `u64be(len(x))` (8 bytes). Injectivity of the *content* framing relies on
`len(x) < 2^64`. That is unreachable for any real credential field (no value approaches exabytes),
so the cap is **safe in practice** — but two things must be nailed down:

- **Right-padding of the final limb is only disambiguated by the length prefix, and the length
  prefix itself sits in the first limb.** Concretely: `framed = u64be(len) ‖ x`, split into 31-byte
  limbs, last limb right-zero-padded. Without the length prefix, `x="ab"` and `x="ab\x00…"` would
  fold identically (trailing-zero ambiguity). The 8-byte prefix fixes the true length, so the map
  is injective on `x`. **This is correct** — but only because the prefix is *inside* the folded
  region (limb 0), not a separate Poseidon input. Confirm in vectors: `""`, `"a"`, `"a\x00"`,
  exactly-31-bytes, exactly-32-bytes (forces 2 limbs), 248-byte and 249-byte strings (forces a new
  limb at the 31-byte * k + prefix boundary). The §8 vector list covers most of these; **add
  `"a"` vs `"a\x00"` explicitly** — it is the textbook trailing-zero case and the audit-02 B2
  `null` vs `""` analog under Poseidon.
- **Length-extension of the fold.** `acc = Poseidon2(acc, limb)` is a Merkle–Damgård-style chain
  seeded at `DS_BYTES`. A classic length-extension would let an attacker, knowing `bytesToField(x)`,
  compute `bytesToField(x ‖ y)` without `x`. Here that is **prevented** because the length prefix
  is committed in limb 0 (the total length is bound up front), so appending changes limb 0 and
  re-frames every subsequent limb. **PASS, but state it as an invariant** and vector a
  `bytesToField(x) ≠ bytesToField(x ‖ extra)` negative case, because the fold *looks* extendable.

**Fix:** add the three negative/edge vectors above; document the 8-byte cap as "schema-bounded,
unreachable" and that the in-circuit `BytesToField(maxChunks)` template MUST range-check the actual
length against `maxChunks` (audit-07 H-1 style) so a malicious prover cannot present extra limbs.

### P-H2 (High) — in-circuit Merkle tree vs SDK commutative tree: the `min/max` sort + odd-promotion equivalence is asserted but not proven; the single-leaf and promotion paths need explicit in-circuit vectors

arch §3.4 / §4.7(d) / impl §1.3 / §11.8(d) claim the in-circuit ordered tree (pathIndices + sortPair
mux) produces the **same** `R` as the SDK's commutative sorted-pair tree. The reduction is correct
**iff** three conditions hold, none of which is currently vector-locked:

1. **Same comparator.** SDK compares as integers in `[0, p)` (impl §1.3 `cmpField: a <= b`); the
   circuit's `sortPair` mux must use the **same** integer comparison (a `LessEqThan`/`LessThan` over
   the full field, not a truncated/bytewise compare). A signed or 252-bit-truncated comparator in
   circom would sort differently for two leaves differing only in the top bits — and since leaves
   are `< p < 2^254`, the top ~2 bits are reachable. This is the Poseidon analog of audit-02 C2's
   "one comparator everywhere" requirement, now spanning circom too. **Must be a normative single
   definition referenced from SDK + circuit, with an adversarially-close vector (two leaves
   differing only in bit 253).**
2. **Odd-promotion in-circuit.** The SDK promotes a lone trailing node unchanged. An ordered
   index-bit circuit with a fixed depth typically pads to a power of two — which is **NOT**
   promotion and yields a different root. The circuit MUST replicate promotion (a node with no
   sibling at a level passes through unhashed), which an index-bit Merkle template does **not** do
   by default. **This is the one genuine correctness subtlety and is under-specified.** If the
   implementer uses a stock `MerkleTreeChecker`, the roots will diverge for any leaf count that is
   not a power of two.
3. **Single-leaf root.** SDK: `R = the one leaf hash` (no node hashing). The circuit's path for a
   1-leaf tree must produce `R = leaf` (depth-0 / empty path), not `Poseidon(leaf, pad)`.

**Fix (NORMATIVE):** specify the circuit Merkle as a **promotion-aware** template (sibling-present
flag per level; when absent, the node passes through), pin the integer comparator shared with the
SDK, and add §8 vector #4 variants asserting the **circom in-circuit recomputed root == SDK R** for
leaf counts 1, 2, 3 (promotion at level 0), 5, 6, 7 (promotion at multiple levels). Without these,
"one root R across SDK and circuit" is an unproven claim, and a divergence makes every ZK proof fail
`isValid(R)` (availability break) — or worse, if the circuit's tree is more permissive, a soundness
break.

### P-H3 (High) — leaf and nullifier both reach arity t=6/t=7 with overlapping slot-0 semantics; the spec's own §5.1 reasoning is muddled — finalize and vector the leaf↔nullifier non-collision

research/13 §5.1 is internally contradictory: it spends a paragraph arguing a t=6 nullifier
`[dogTagId, …]` cannot collide with a t=6 leaf `[DS_LEAF=1, …]` "because credentials never set
dogTagId==1", then **reverses** to the NORMATIVE choice **t=7 with `DS_NULLIFIER=4` in slot 0**
(impl §11.9(b), arch §4.7 confirm t=7/`PoseidonT7`). The final choice (t=7) is correct and clean —
**different arity (t=7 vs t=6) makes leaf↔nullifier collision structurally impossible regardless of
slot values.** But the residual risks to lock:

- **DS_LEAF=1 leaf (t=6) vs DS_NODE=2 node (t=3) vs DS_BYTES=3 fold (t=2):** all distinct arity AND
  distinct slot-0 constant → non-confusable. **PASS.**
- **The `bytesToField` output (DS_BYTES fold) is consumed as an *input field* to the leaf**, never
  as a leaf/node itself. A `bytesToField` result could numerically equal some leaf hash (both are
  field elements) — but it is never *interpreted* as a leaf (it only ever sits in a leaf input
  slot), so there is no cross-domain second-preimage. **PASS, but the obfuscation check must still
  reject any `obfuscated[]` entry equal to a live leaf hash** (audit-02 D / audit-05 V12 carry over
  unchanged — these are field elements now, same logic).
- **Domain tags are first-*input* slots, not capacity IV.** For circomlib's fixed-arity compression
  function this is sound: a fixed constant in a fixed position with fixed arity is a complete domain
  separator (the security argument is identical to a capacity-lane IV for fixed-width Poseidon).
  **PASS.** The only thing a capacity IV would buy is domain separation across *variable* arities of
  the *same* `t`, which does not arise here. Confirmed correct.

**Fix:** delete the contradictory t=6 paragraph in research/13 §5.1 (it reads as if t=6 might be
chosen and invites an implementer to pick the unsound variant); state flatly: **nullifier = t=7 with
`DS_NULLIFIER` in slot 0; leaf = t=6 with `DS_LEAF` in slot 0; node = t=3 with `DS_NODE`; byte-fold =
t=2 chain seeded `DS_BYTES`** — non-collision is guaranteed by distinct arity. Add §8 vector #6
asserting a leaf-input-tuple and a nullifier-input-tuple with numerically identical payloads produce
different hashes (they will — different arity).

### P-H4 (High) — `light-poseidon` limb construction: the "never `from_be_bytes_mod_order`" rule is correct but the safe alternative is under-specified and easy to get wrong

impl §11.2(c) and research/13 §1.5 correctly forbid `Fr::from_be_bytes_mod_order` for limbs (it
would silently reduce a `≥ p` value). But the prescribed alternative — "build each `Fr` from a
≤31-byte big-endian limb provably `< p`" — is the **only** place the Rust side can silently diverge
from circom, because:

- A 31-byte limb is `< 2^248 < p`, so `Fr::from_be_bytes_mod_order` on a **31-byte** input is
  actually safe (no reduction can occur). The danger is only if someone pads a limb to 32 bytes and
  feeds 32 bytes (then the top byte can push `≥ p`). The rule should be stated as: **decode exactly
  the ≤31-byte limb; never widen to 32 before decoding.**
- The DS constants and the length-prefix limb must also be `Fr` values built the same way.

**Fix:** state the Rust limb rule precisely — "decode the big-endian limb of length ≤31 directly;
the 31-byte bound guarantees `< 2^248 < p` so no modular reduction occurs; do NOT zero-extend to 32
bytes before decoding" — and add a Rust unit test feeding a 31-byte `0xFF…FF` limb and asserting it
equals the circom witness's field for the same limb. (This is the concrete realization of the §8
`bytesToField` cross-lang vectors; call it out so the Rust author doesn't reach for the convenient
`from_be_bytes_mod_order`.)

---

## 3. Medium / Note findings

- **P-M1 (Medium) — nullifier omits `recordType`; relies on `purpose` for cross-purpose separation.**
  audit-07 M-3 flagged that the old nullifier lacked purpose. v4 fixes this: `purpose` is now in the
  nullifier, signed, and a public signal. **Resolved.** Residual: `purpose` and `recordType` are now
  distinct (audit-08 V3-H1), and the nullifier binds `purpose` (the verify scope), not `recordType`.
  This is correct for the verify domain. Note only: the `purposeToRecordType` admin map (impl
  §11.9(e)) is the trust point resolving the clone for `isValid(R)` — a misconfiguration routes
  `isValid` to the wrong issuer clone. Keep it admin-governed and event-logged (out of crypto scope,
  but flag for audit-11 contracts).

- **P-M2 (Medium) — `obfuscated[]` entries are now field elements `< p`, serialized bytes32.** The
  audit-02 D / audit-05 V12 well-formedness checks ("32-byte hashes that don't overlap live leaves")
  carry over, but the SDK MUST now validate each `obfuscated[]` entry is a **valid field element
  `< p`** (a bytes32 with top bits set, `≥ p`, can never be a real leaf and must be rejected, else an
  attacker injects junk that the rebuild treats as a leaf). Add to the `verify` integrity fragment.

- **P-M3 (Medium) — bytes32 ↔ field serialization is asymmetric.** Every leaf/root/nullifier is
  `< p < 2^254`, so the top ~2 bits of the bytes32 are always zero. On-chain (`issue(R)` stores
  bytes32, `isValid(R)` compares bytes32) this is fine *because both sides serialize the same field
  big-endian*. **But** if any code path ever compares a bytes32 from the chain against a freshly
  computed field without canonical big-endian serialization (e.g. little-endian Rust `to_bytes()`),
  `isValid(R)` silently fails. Pin "field → bytes32 is big-endian, fixed 32 bytes, left-zero-padded"
  as the single serialization rule (it is implied in impl §1.4 but not stated as a cross-lang MUST).

- **P-M4 (Note) — Poseidon algebraic-attack maturity caveat is correctly flagged** (research/13 §7.1).
  For a **salted** commitment the hiding term is the 128-bit salt (hash-agnostic — audit-05 V11), so
  Poseidon's narrower cryptanalytic margin vs keccak is acceptable. The DPIA (arch §11.1) MUST note
  the hash change, as the spec says. **PASS, keep the caveat.**

- **P-M5 (Note) — single-leaf root is a raw leaf hash (t=6, DS_LEAF).** As in audit-02 C4, a 1-field
  credential's `R` is a leaf hash. Under Poseidon this is still safe: DS_LEAF (t=6) can never be
  reinterpreted as a node (t=3) or a future batch-tree node. Vector the 1-leaf case (§8 #4). **PASS.**

---

## 4. Answers to the seven audited questions

### Q1 — byte→field packing injectivity + collision/second-preimage

**VERDICT: injective and second-preimage-safe, with the edge cases handled — conditional on the
P-H1 vectors and the 31-byte limb discipline (P-H4).** Reasoning:
- Each 31-byte limb decodes big-endian to `< 2^248 < p`, so the byte→limb map is **injective into
  the field with no modular wraparound** (this is the crux that keccak-style raw-absorb-mod-p would
  break). PASS.
- The 8-byte big-endian length prefix inside the folded region kills trailing-zero ambiguity
  (`"ab"` ≠ `"ab\x00"`) and length-extension (`x` ≠ `x‖y`) — the prefix is committed in limb 0, so
  the whole framing is self-delimiting (the Poseidon analog of audit-02 B1/B2). PASS.
- **Cross-component ambiguity (keyPath vs salt vs value)** is prevented **structurally** by the
  leaf's fixed arity: keyPath, salt, typeTag, value occupy **distinct fixed input slots** of a single
  `Poseidon6(DS_LEAF, …)` — there is no concatenation to be ambiguous, so the old per-component
  uint32 length prefixes are correctly subsumed. A `bytesToField` value can never be confused with a
  salt field (16-byte direct decode) because they sit in different, fixed slots. PASS.
- **Edge cases:** empty bytes → `framed = u64be(0)` (one limb, well-defined image) → distinct from
  any non-empty; 31 bytes → 1 content limb (+ prefix may share or spill — vector both); 32 bytes →
  spills to a 2nd limb (vector — the textbook case); very long strings → multi-limb fold, bounded by
  schema `maxChunks` in-circuit; 8-byte cap → unreachable, document it. All handled **given** the
  P-H1 vectors are added.

### Q2 — Poseidon parameter parity across 4 langs

**VERDICT: they CAN share one BN254 circomlib instantiation, but the spec's CI gate is insufficient
as written (P-C1) — per-arity anchors (t=2,3,6,7) are REQUIRED, not optional.** The four libraries
(circomlib / poseidon-lite / light-poseidon `new_circom` / poseidon-solidity) are the correct,
maintained, circomlib-compatible set and do share the x^5 S-box, `R_F=8`, per-`t` `R_P`, seed
`"poseidon"`, and circomlib MDS — **by reputation**. Real drift risks, all flagged:
- circomlibjs history (#14 hash changed, #30 JS≠Solidity) → pin versions, which the spec does.
- **arity/t vs nInputs convention** (`t = nInputs + 1`): poseidon-lite's `poseidon6` = 6 inputs =
  t=7; circomlib `Poseidon(6)` = 6 inputs = t=7; light-poseidon `new_circom(6)` = 6 inputs. An
  off-by-one here (calling `new_circom(7)` for a 6-input hash) silently produces a different,
  wrong-arity hash. **Per-arity vectors catch this; the single `[1,2]` vector does not.**
- per-`t` `R_P` table and per-`t` constants — the dominant silent-drift surface at t=6/t=7.
- `light-poseidon` MUST use `new_circom(n)`, not a generic constructor (spec says this — keep).
**The `poseidon([1,2])` anchor is necessary but NOT sufficient. Per-arity anchors at t=2, t=3, t=6,
t=7 ARE needed.** This is P-C1 (Critical).

### Q3 — domain separation (first input slot vs capacity IV)

**VERDICT: input-slot domain separation is sound for fixed-arity Poseidon; no cross-domain
collision.** For a fixed-width compression function (which circomlib `Poseidon` is — not a duplex
sponge), a distinct constant in a fixed slot with fixed arity is a complete domain separator,
cryptographically equivalent to a capacity-lane IV. Choosing input-slot tags to stay on the exact
circomlib API in all 4 libs is the right call (capacity-IV plumbing is where cross-lib mismatches
creep in). Cross-domain collisions:
- leaf (t=6, DS_LEAF=1) vs node (t=3, DS_NODE=2) vs byte-fold (t=2, DS_BYTES=3) vs nullifier
  (t=7, DS_NULLIFIER=4): **distinct arity AND distinct slot-0 constant → impossible to confuse.**
- The one muddle is the contradictory t=6/t=7 nullifier discussion in research/13 §5.1 (P-H3) — the
  final t=7 choice is sound; the contradictory prose must be deleted so no one implements the
  unsound t=6 variant. PASS with P-H3 cleanup.

### Q4 — in-circuit tree == SDK tree

**VERDICT: provably equal ONLY if the circuit replicates (a) the integer-`[0,p)` comparator, (b)
odd-promotion (NOT power-of-two padding), and (c) the single-leaf passthrough — none of which is
vector-locked today (P-H2, High).** The claim is correct in principle (one tree definition, node =
`Poseidon3(DS_NODE, min, max)`), but a stock index-bit Merkle template will diverge on non-power-of-2
leaf counts (it pads instead of promoting) and on the comparator. Must add promotion-aware circuit
template + shared comparator + in-circuit-root == SDK-root vectors for counts {1,2,3,5,6,7}.

### Q5 — nullifier parity

**VERDICT: the design is correct (same t=7 instantiation in circom + Solidity `PoseidonT7` + Rust,
purpose now included, address uint160→field is safe at 160<254), but parity is NOT guaranteed
until P-C1 (t=7 anchor vector) AND P-C2 (purpose mod-r + dogTagId/nonce range-check, byte-identical,
with a negative vector) are closed.** This directly resolves the OLD audit-07 H-3 / H-4 / audit-08
V3-C1 nullifier gaps **provided** those two gates land:
- addresses uint160→field: 160 < 254, no reduction, one field — safe. PASS.
- `purpose = keccak % r`: deterministic IF the modulus is pinned to the **scalar field r** (not base
  field q) and applied identically in all 3 environments (P-C2). The reduction itself is
  deterministic; the risk is environment disagreement and modulus confusion.
- shared `consumed` set integrity: holds iff the t=7 hash and the field-encodings are bit-identical
  across paths — i.e. iff P-C1 + P-C2 close. Otherwise audit-07 H-3's "shared set is an illusion"
  re-applies.

### Q6 — field-element range

**VERDICT: 15-digit chips / timestamps / typeTag / uint160 addresses all fit one field; the
256-bit keccak-derived `purpose`/`recordType` reduction is the live risk (P-C2).**
- 15-digit microchip (string tag 2, 15 ASCII bytes) → one 31-byte limb, `< 2^248`. Fits. PASS.
- timestamps (~20–25 bytes), typeTag (1 byte) → one limb / direct. Fits. PASS.
- addresses (160 bits) → one field, no reduction. Fits. PASS.
- `dogTagId`/`nonce` (uint256) → MUST be `< r`; range-check missing on the normal path (P-C2.2).
- `purpose`/`recordType` (256-bit keccak) → reduced mod **r**; must be identical on-chain AND
  in-circuit AND in Rust (P-C2.1). A `> r` value reduced differently (or against `q`) → nullifier /
  whitelist-key mismatch. This is the Critical to close.

### Q7 — regression of audit-02 / audit-05 fixes under Poseidon

**VERDICT: all determinism fixes SURVIVE because `encodeValue` is reused verbatim — the change is
purely the final hash over identical canonical bytes. C-1/C-2 (07/08) are genuinely ELIMINATED;
audit-07 C-2 / audit-08 H3 (subject↔key, ownerOf, purpose) still hold.** Detail:

| Prior fix | Survives under Poseidon? | Note |
|---|---|---|
| A1 decimal grammar | **PASS** | operates on the input string; feeds `encodeValue` unchanged; only the hash of those bytes changes. |
| A2 typed input / no f64 | **PASS** | `mapType`/typed-string input unchanged; tag still in the leaf (now slot 3 of Poseidon6). |
| A3 NFC + pinned Unicode | **PASS** | NFC applied to keyPath/value bytes before `bytesToField`; Solidity still never builds a leaf (it stores R / hashes only the nullifier). |
| F2a flatten/keyPath grammar | **PASS** | keyPath is `bytesToField(utf8(NFC(kp)))` — same canonical bytes, now folded to a field. Length-bound by the 8-byte prefix (replaces the old uint32). |
| F2b first-two-colons parse | **PASS** | parsing is pre-encode; unaffected by the hash. |
| requiredPaths / non-obfuscatable (`dogTagId`,`@context[*]`,`type[*]`) | **PASS** | obfuscation logic unchanged; add field-element well-formedness on `obfuscated[]` (P-M2). |
| Salts 16B CSPRNG unique-per-field | **PASS** | salt → one field (16 bytes < 2^128 < p), no reduction; the 128-bit hiding term is hash-agnostic (audit-05 V11). Erasure reasoning (destroy all salt copies) unchanged. |
| C2 single-doc rebuild / one comparator | **PASS, EXTENDED** | now the comparator is integer-`[0,p)` and MUST also match in-circuit (P-H2). |
| domain-sep / leaf framing / second-preimage (B1/B2) | **PASS** | input-slot DS tags + fixed-arity slots replace the `0x00`/`0x01` byte prefixes; stronger (structural, not byte-prefix). |

**C-1 / C-2 elimination — CONFIRMED:**
- **audit-07 C-1** (keccak↔Poseidon `rZk`↔`rKec` binding trusted off-chain): **genuinely
  eliminated.** There is one root `R`; the circuit proves leaves→`R`; `issue(R)` anchors that exact
  root; `isValid(R)` is checked directly. No off-chain binding exists, so there is nothing to be
  unsound. The `zkCommit`/`ZkCommitment`/`kecOf`/`0x02` binding leaf are deleted.
- **audit-08 C-2** (forgeable `zkCommit` / undefined `issuerForAny`): **genuinely eliminated.**
  `zkCommit`/`kecOf`/`zkIndex`/`cloneOf`/`issuerForAny` are deleted; clone resolution uses the
  existing per-`recordType` `issuerFor` (via the admin `purposeToRecordType` map, P-M1) and calls
  `isValid(R)` directly. The deletion is a strict reduction in trust surface — confirmed sound.

**Still-normative gates — CONFIRMED INTACT (not addressed by, and not weakened by, unification):**
- **audit-07 C-2 / audit-08 V3-H3 (subject↔key):** circuit signs `subject` into the EdDSA message
  `Poseidon(dogTagId, purpose, relayer, subject, R, nonce)`, outputs `keyHash = Poseidon(Ax,Ay)`,
  registry requires `keyOf[subject] == keyHash`. Present in impl §11.8(d)/§11.9(d)(e). INTACT.
- **ownerOf(dogTagId) == subject:** present on both paths (impl §11.8(a) line 1492, §11.9(e) line
  1664). INTACT.
- **purpose binding:** signed, public signal, in nullifier, keys the `VERIFY:` whitelist
  (`keccak256("VERIFY:"||purpose)` — keccak correctly retained for the namespacing key). INTACT.
- range-check all ZK public signals (#358) and nullifier-as-public-signal (#383): INTACT on the ZK
  path; **normal-path dogTagId/nonce range-check is the gap (P-C2.2).**

---

## 5. Summary table

| ID | Area | Severity | Fix (one-line) |
|---|---|---|---|
| P-C1 | CI parity gate single-arity | **Critical** | Add per-arity anchors t=2,3,6,7 across all 4 libs; `[1,2]` alone is insufficient. |
| P-C2 | nullifier field-reduction parity | **Critical** | Pin `purpose mod r` (scalar field) + normal-path `dogTagId/nonce < r` range-check, byte-identical in circom/Solidity/Rust + negative vector. |
| P-H1 | bytesToField edge/extension vectors | High | Add `""`/`"a"`/`"a\x00"`/31B/32B/extension negative vectors; range-check in-circuit limb count. |
| P-H2 | in-circuit tree == SDK tree | High | Promotion-aware circuit template + shared integer-`[0,p)` comparator + single-leaf passthrough + root-equality vectors for counts {1,2,3,5,6,7}. |
| P-H3 | leaf↔nullifier arity domain | High | Delete contradictory t=6 prose; pin t=7 nullifier / t=6 leaf / t=3 node / t=2 fold; vector non-collision. |
| P-H4 | Rust limb construction | High | Decode ≤31-byte limb directly (never widen to 32 / never `from_be_bytes_mod_order`); unit-test against circom witness. |
| P-M1 | purposeToRecordType map | Medium | Admin-governed + event-logged (contracts-audit scope). |
| P-M2 | obfuscated[] field validity | Medium | Reject any `obfuscated[]` entry `≥ p`. |
| P-M3 | field↔bytes32 serialization | Medium | Pin big-endian, fixed-32, left-zero-pad as the one cross-lang serialization rule. |
| P-M4 | Poseidon algebraic maturity | Note | Salt is the hash-agnostic hiding term; DPIA notes the hash change. PASS. |
| P-M5 | single-leaf root = leaf hash | Note | Safe under DS_LEAF/arity separation; vector it. PASS. |

**Net:** the unification is the right call — it deletes a whole class of binding-soundness bugs
(audit-07 C-1, audit-08 C-2) and keeps every prior determinism fix because `encodeValue` is
untouched. The new risk surface is entirely **cross-environment Poseidon determinism** (P-C1) and
**nullifier field-encoding parity** (P-C2); both are CI-vector-closable and MUST gate the build.
