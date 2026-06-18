# Audit 07 — Groth16 ZK + Consent Cryptographic Soundness (v3)

**Auditor scope:** zero-knowledge / cryptographic-protocol audit of the on-chain proof-of-verification
leg ONLY. Soundness and the binding between the off-chain Groth16 proof and on-chain state.
**Sources read in full:** architecture.md §3.6, §4.7, §5.5, §13.7; implementation.md §1.4, §1.10, §2.2/§2.6,
§11.3, §11.8 (normative ZK code); research/CHANGESPEC-v3.md, 10-zk-groth16.md, 11-consent-attestation.md.
**Date:** 2026-06-17. **Curve:** BN254/alt_bn128. **Proof system:** Groth16 (snarkjs verifier).

> **Out of scope (not re-audited here):** the keccak issuance standard (§3), DNS pillar, GDPR/privacy
> (covered by audit-02/07-legal), SBT lifecycle (audit-04). Where those touch ZK soundness they are
> flagged, not re-derived.

---

## 0. One-line verdict

**The ZK path is UNSOUND as specified: `zkCommit` is an unconstrained issuer-asserted mapping with no
proof that `rZk` and `rKec` commit to the same leaf set, so any whitelisted verifier-or-issuer key can
bind a fabricated Poseidon root to a legitimate issued keccak root and mint valid "I-verified-it"
attestations over data that was never issued. Do not deploy the ZK path until `zkCommit` binding is
made cryptographic (Finding C-1).**

---

## 1. The `zkCommit` dual-root binding — EXPLICIT VERDICT

**Verdict: BROKEN (Critical). The dual-root binding is an unauthenticated assertion, not a proof.**

### 1.1 What the design claims

architecture.md §4.7 / implementation.md §1.4 claim `wrapDocument` computes `rZk` as "a parallel Poseidon
Merkle root **over the same leaf set** as `rKec`," and the issuer "binds the two roots via
`DogTagIssuer.zkCommit(rKec, rZk)`." The registry then maps `rZk → rKec` via `kecOf[rZk]` and reuses
`isValid(rKec)`. The stated security story (research/10 §2.3, §4.2; architecture §13.7) is: "the circuit
only proves leaves→`rZk`; the chain proves `rZk ↔ rKec ↔ issued`."

### 1.2 What the code actually enforces (implementation.md §2.2, lines 357–360)

```solidity
function zkCommit(bytes32 rKec, bytes32 rZk) external onlyWhitelisted {
    require(issuedAt[rKec]!=0 && rZk!=bytes32(0) && kecOf[rZk]==bytes32(0),"bad");
    kecOf[rZk]=rKec; emit ZkCommitment(rKec,rZk);
}
```

`zkCommit` takes **two opaque `bytes32` values** and stores the mapping. It enforces only:
1. `rKec` was issued (`issuedAt[rKec]!=0`),
2. `rZk` is non-zero,
3. `rZk` not already mapped (one-time).

**It does NOT, and cannot, verify that `rZk` is the Poseidon Merkle root of the same leaf set that
produced `rKec`.** That claim — "same leaf set" — exists only as off-chain SDK convention. Solidity sees
two unrelated 32-byte words. There is **no keccak↔Poseidon cross-commitment anywhere in the system**:

- The **circuit** (§11.8(d)) proves `leaves → rZk` and never references `rKec`. It cannot — keccak in-circuit
  was deliberately rejected (research/10 §2.1) precisely to avoid binding the two hash families.
- The **registry** (§11.8(a) `recordVerificationZK`) trusts `kecOf[rZk]` blindly: `rKec = kecOf(rZk);
  require(isValid(rKec))`.
- research/10 §2.3 itself documents the gap: **Option C (in-circuit keccak of `0x02‖rKec‖rZk`) is the
  ONLY variant that actually binds the two roots cryptographically, and it was explicitly NOT chosen**
  ("documented but not recommended"). Option A (event/mapping lookup) — the one adopted — relies entirely
  on the issuer asserting the binding honestly.

### 1.3 The attack (concrete)

`zkCommit` is `onlyWhitelisted`, i.e. callable by any address `isWhitelisted` on that `DogTagIssuer`
clone (note: §2.2's `onlyWhitelisted` still uses the **global** `registry.isWhitelisted(msg.sender)`, not
the per-recordType `isWhitelistedFor` of §13.1 C-2 — see Finding H-3). So the attacker is any
whitelisted issuer/verifier key, OR anyone who compromises one such key (the realistic threat — there are
many per the one-to-many signer model, §4.3).

1. Attacker observes a **legitimate, issued** keccak root `rKec*` on a `DogTagIssuer` (public on-chain
   via `RootIssued`). `isValid(rKec*) == true`.
2. Attacker fabricates an arbitrary credential leaf set of its choosing (e.g. a rabies cert for a pet it
   does not own, with `dogTagId` = a victim's pet, or any fields it likes), and computes the **Poseidon**
   root `rZk_evil` over those fake leaves. It generates a perfectly valid Groth16 proof for `rZk_evil`
   (the circuit is satisfied — the fake leaves really do hash to `rZk_evil`).
3. Attacker calls `zkCommit(rKec*, rZk_evil)`. Passes: `rKec*` is issued, `rZk_evil != 0`, unmapped.
   Now `kecOf[rZk_evil] = rKec*`.
4. Attacker calls `recordVerificationZK(a,b,c,[dogTagId, relayer, subject, nf, rZk_evil])`. The registry:
   range-checks pass; `verifyProof` passes (real proof over `rZk_evil`); `rKec = kecOf[rZk_evil] = rKec*`;
   `isValid(rKec*) == true` → **PASSES**. `Verified(...)` emitted.

**Result:** a ZK attestation over **fabricated credential data** is recorded against a legitimate issued
root. The entire "proof is about a real, issued credential" guarantee collapses. The reverse also holds:
an attacker can map a *legitimate* `rZk` to an unrelated `rKec` it controls, decoupling "what was proven"
from "what was issued." (research/10 §1.3 explicitly relied on `rZk` ensuring the proof "is about a real,
issued credential — not a fabricated tree." That property does not hold.)

### 1.4 Why "issuer-only" does not save it

The mitigation posture leans on `zkCommit` being `onlyWhitelisted` ("issuer-only"). This is insufficient:

- **It is the wrong actor.** In the verification flow the *relayer* (groomer) and the *issuer* (vet) are
  deliberately different parties (research/11 §2.2: "we do NOT require the relayer to be the issuer").
  But `zkCommit` is callable by **any** whitelisted signer on that clone, and the VERIFY: namespace and
  ISSUE namespace both grant on-chain addresses. A malicious/compromised whitelisted key — of which the
  one-to-many model (§4.3) guarantees there are many, including hot backend keys and browser EOAs — can
  bind arbitrary `rZk`. Soundness must not rest on every whitelisted key being honest *about a binding it
  is not forced to compute correctly*.
- **A correct binding is not verified even for an honest-but-buggy issuer.** Nothing forces the SDK's
  `rZk` to actually be over the same leaves; a bug in `hashLeafZk`/`poseidonMerkle` (a different leaf
  ordering, a missing field, a salt mismatch) silently produces a valid-but-wrong binding that the chain
  cannot detect. There are **no test vectors that cross-check `rKec` and `rZk` are over identical leaves**
  beyond each being individually correct (the `testvectors.json` assertion in §1.4 asserts each root, not
  their equivalence).

### 1.5 Fix (C-1) — make the binding cryptographic

Pick one; **(A) is recommended:**

- **(A) Bind `rZk` to `rKec` inside the issuance commitment and check it on-chain at `zkCommit`.** Have
  the issuer pass the keccak binding leaf and prove consistency cheaply: store
  `kecOf[rZk]` ONLY after verifying that the SAME wrapped document produced both. The minimal sound form:
  compute, at issuance, a **single keccak binding** `bind = keccak256(0x02 ‖ rKec ‖ rZk)` and **anchor
  `bind` (not the bare pair) as the issued root**, so `isValid` is over a value that commits to both
  roots; the circuit then takes `rKec` as a *public input* and proves `keccak(0x02‖rKec‖rZk)==bind`
  in-circuit (research/10 Option C). Cost: one in-circuit keccak (~151k constraints) — still ~165k total,
  sub-second, and it is the ONLY variant that closes this hole without trusting `zkCommit`.
- **(B) If keeping the no-keccak-in-circuit constraint is non-negotiable:** the SDK must additionally,
  at issuance, prove leaf-set equivalence by anchoring a Poseidon-of-keccak-leaves digest and asserting
  it equals a value derived from the keccak tree — but there is no cheap on-chain check that two
  *different hash families* over the same preimages agree without recomputing one of them on-chain.
  In practice (B) degenerates to (A). **There is no sound design that lets `zkCommit` accept two
  independent opaque roots.**
- **(C) Minimum interim hardening (does NOT fix soundness, reduces blast radius):** restrict `zkCommit`
  to the **original issuer of `rKec`** only — `require(issuedBy[rKec]==msg.sender)` (the §13.1 H-1
  originator binding already records `issuedBy[root]`). This removes the "any whitelisted key" surface
  but still trusts that issuer to bind correctly and does nothing against a compromised issuer key. Ship
  (A) for any real deployment.

**Severity: CRITICAL. This is the root soundness break of the ZK path.**

---

## 2. Circuit completeness / soundness

Public signals (circom §11.8(d)): `[dogTagId, relayer, subject]` declared public + outputs `nullifier, rZk`.
The circuit proves (a) leaves→`rZk`; (b) `leafValues[dogTagIdLeafIndex] == dogTagId`; (c) EdDSA over
`Poseidon4(dogTagId, relayer, rZk, nonce)` + exposes `Poseidon(Ax,Ay)`; (d)
`nullifier == Poseidon4(dogTagId, relayer, subject, nonce)`.

### Finding C-2 (Critical) — `subject` is an UNCONSTRAINED public input; consent key not bound to `subject` in-circuit

`subject` is a public input but the circuit body (§11.8(d)) **never constrains it**:
- The EdDSA consent message is `Poseidon4(dogTagId, relayer, rZk, nonce)` — **`subject` is NOT in the
  signed message** (architecture §4.7(c) and implementation §1.10 confirm `M = poseidon4(dogTagId,
  relayer, rZk, nonce)`).
- The nullifier uses `subject`, but the nullifier is an *output* — it is whatever the prover's `subject`
  input makes it; it does not constrain `subject` to anything real.
- The "subject↔key linkage" is described as `Poseidon(Ax,Ay) == keyOf[subject]` "checked on-chain" —
  **but `recordVerificationZK` (§11.8(a)) NEVER performs this check.** It is written as a comment
  ("subject<->BabyJubjub-key linkage already proven in-circuit & bound via ConsentKeyRegistry") and the
  circuit comment also defers it ("expose Poseidon(Ax,Ay) for the on-chain ... check"). **Neither side
  does it.** `Poseidon(Ax,Ay)` is not even a public output in the declared signal list
  (`{public [dogTagId, relayer, subject]}` + outputs `nullifier, rZk` — there is no `keyHash` output),
  so the registry has nothing to compare to `keyOf[subject]`.

**Consequence:** a prover can set `subject` to **any address** (e.g. a victim's per-pet address) and sign
the consent with **its own** BabyJubjub key `(Ax,Ay)`. The EdDSA check passes (it verifies the prover's
own key against a message that doesn't contain `subject`); the circuit is satisfied; on-chain there is no
binding between `(Ax,Ay)` and `subject`. The attacker forges "subject X was verified by relayer Y" without
X's consent. This **defeats the entire consent mechanism on the ZK path.** (Note the normal path is sound
here: it does `ECDSA.recover(...) == c.subject` and `sbt.ownerOf == c.subject` — the ZK path has neither.)

**Fix (C-2):**
1. **Put `subject` in the signed EdDSA message:** `M = Poseidon(dogTagId, relayer, rZk, subject, nonce)`
   (or include `Poseidon(Ax,Ay)`), so the consent is bound to a specific subject.
2. **Expose `keyHash = Poseidon(Ax,Ay)` as a public output** and, in `recordVerificationZK`, require
   `consentKeys.keyOf[address(uint160(pub[2]))] == bytes32(keyHash)`. This is the linkage both the circuit
   comment and §2.5 *describe* but the normative code omits. Without it the `ConsentKeyRegistry` is dead
   code on the ZK path.
3. **Add `sbt.ownerOf(dogTagId) == subject` to the ZK path** (the normal path has it; the ZK path dropped
   it). Otherwise a prover proves consent for a pet `subject` doesn't own.

**Severity: CRITICAL.**

### Finding H-1 (High) — `dogTagId` leaf binding is under-specified; the membership leaf selection is unsound as sketched

The circuit (§11.8(d), and research/10 §7) does `mk.leaf <== lh[dogTagIdLeafIndex]` and asserts
`leafValues[dogTagIdLeafIndex] == dogTagId`. Two gaps:

- **`dogTagIdLeafIndex` is a free private input with no range/selector constraint shown.** A prover
  picks which leaf is "the dogTagId leaf." Combined with the membership proof being computed over THAT
  leaf, the circuit only proves "*some* leaf at a prover-chosen index is included and has value
  `dogTagId`." It does NOT prove the credential actually contains a `keyPath == credentialSubject.dogTagId`
  field with that value. A prover can place `dogTagId` in any leaf slot (e.g. reuse a `weightHistory[0]`
  slot value) — the `keyPath` is private and unconstrained. So `dogTagId` is bound to *a* leaf value but
  **not to the dogTagId field of the credential.**
- **Only the dogTagId leaf is proven to be in the tree.** The Merkle membership runs once, on the
  dogTagId leaf. The other `N-1` leaves (`lh[i]`) are hashed but **never constrained to be in the tree
  that yields `rZk`** as sketched (research/10 §7 runs `PoseidonMerkle` only on `mk.leaf`). So a prover
  can supply arbitrary `leafValues[i]` for non-dogTagId leaves — they don't affect `rZk` and aren't
  checked. This is fine for `rZk` integrity *only if* `rZk` is computed purely from the path, but then
  the leaves array is decorative and the "leaves hash to rZk" claim (research/10 §1.1(a)) is **false** —
  only the single dogTagId leaf is bound to `rZk`.

**Fix (H-1):** (i) constrain `leafKeyPathHashes[dogTagIdLeafIndex] == Poseidon("credentialSubject.dogTagId")`
(a circuit constant) so the bound leaf is provably the dogTagId field, not an arbitrary slot; (ii) make
`dogTagIdLeafIndex` a `LessThan(N)`-range-checked selector with a proper multiplexer; (iii) decide and
document the actual `rZk` recomputation — if `rZk` must commit to ALL leaves (the "same leaf set as rKec"
claim in §1 depends on this), the circuit must build the full Poseidon tree from `lh[0..N-1]`, not just
prove single-leaf membership against a prover-supplied path. As sketched, `rZk` is **not** pinned to the
full leaf set, which compounds C-1.

**Severity: HIGH (interacts with C-1; together they let a prover choose `rZk`, `dogTagId`, and `subject`
nearly freely).**

### Finding M-1 (Medium) — typeTag / value range checks absent in-circuit

`leafTypeTags[N]`, `leafSalts[N]`, `leafValues[N]` are field elements with no range checks. BN254 field
elements are ~254 bits; a `uint256 dogTagId` or a 16-byte salt that the SDK treats as bounded can, in the
witness, take any field value. Without `LessThan` constraints, a prover has extra freedom in constructing
leaves that collide a target `rZk` or alias `dogTagId` modulo the field. Add explicit bit-length
constraints (`Num2Bits`) on every numeric leaf input and on `dogTagId` (≤ what the SBT allows). Severity:
Medium (becomes High absent the C-1/H-1 fixes).

---

## 3. ConsentKeyRegistry binding (BabyJubjub ↔ secp256k1 subject)

`bindConsentKey(babyJubPubKeyHash, ecdsaSig)` (§11.8(b)): one-time, `ECDSA.recover(EIP712(BIND, hash,
msg.sender), sig) == msg.sender`, then `keyOf[msg.sender] = babyJubPubKeyHash`.

### Finding H-2 (High) — the binding is never consumed by the ZK path (see C-2), making the registry vacuous

As shown in C-2, `recordVerificationZK` performs **no** `keyOf` lookup, and the circuit exposes no
`Poseidon(Ax,Ay)` public output to compare. So however correct `bindConsentKey` is, it is **not enforced**
anywhere in the proof-of-verification flow. Any BabyJubjub key works. **Fix:** the C-2 fix (expose
`keyHash`, check `keyOf[subject] == keyHash` on-chain). Until then the ConsentKeyRegistry provides zero
security to the ZK path. Severity: HIGH (this is the on-chain half of the C-2 break).

### Finding M-2 (Medium) — `bindConsentKey` self-binding only; cannot bind a key to an address you don't control (GOOD) — but no rotation / no domain vs SBT recover

Positives (sound as written): the EIP-712 domain (`name:"DogTag", version:"1"`, and the bind typehash
`BindConsentKey(bytes32 babyJubPubKeyHash,address wallet)`) plus `recover == msg.sender` mean an attacker
**cannot** bind a BabyJubjub key to a `subject` they don't control — they'd need that wallet's secp256k1
signature, and `msg.sender` must be the wallet. No replay across wallets (the message includes
`msg.sender`). No cross-contract replay vs the SBT `recover` (different typehash, and `ConsentKeyRegistry`
is a distinct `verifyingContract`).

Gaps:
- **`verifyingContract` is the default `address(this)`** in `ConsentKeyRegistry`'s `EIP712("DogTag","1")`,
  which is correct, but note `VerificationRegistry` ALSO uses `EIP712("DogTag","1")`. The two are
  distinguished only by `verifyingContract`. Confirm the deployed addresses differ (they will) and that
  the `BindConsentKey` typehash can never collide with `VerificationConsent` — it cannot (different type
  strings), so this is OK. Document it.
- **No rotation / one-time forever.** `require(keyOf[msg.sender]==0)` means a user whose BabyJubjub key is
  lost/compromised can **never rebind** — and a compromised consent key lets the holder forge ZK consent
  for that subject indefinitely (subject to C-2 being fixed; once fixed, the consent key is security-
  critical). Add an admin/owner-gated `rotateConsentKey(newHash, ecdsaSig)` with a nonce, mirroring the
  SBT `recover` precedent. **Also wire consent-key compromise into the same delist/mass-revoke response
  as issuer-key compromise (§13.3).** Severity: Medium.
- **Per-pet address model collision (Medium).** `subject` is mandated to be a **fresh per-pet derived
  address** (`m/44'/60'/0'/0/{petIndex}`, §11.1/§13.7). But `keyOf` is keyed by wallet address, and a
  BabyJubjub consent key is "derived from the same seed, distinct domain" (research/10 §2.4) — i.e. ONE
  consent key, but MANY per-pet subject addresses. Either (i) each per-pet address must run its own
  `bindConsentKey` (gas + UX per pet, and each needs its own secp256k1 sig from that derived address), or
  (ii) the binding is per-person but `subject` is per-pet — in which case `keyOf[subject]` will be empty
  for every per-pet address and the (missing) C-2 check would always fail. **This is an unresolved design
  contradiction** between the per-pet-address privacy mandate and the per-wallet ConsentKeyRegistry. Must
  be resolved when implementing the C-2 fix.

---

## 4. Nullifier

Shared `nullifier = Poseidon(dogTagId, relayer, subject, nonce)`; ZK = public signal `pub[3]`; normal =
on-chain `Poseidon.hash4(dogTagId, uint160(relayer), uint160(subject), nonce)` (§11.8(a)).

### Finding H-3 (High) — normal-path Solidity Poseidon vs circuit Poseidon: parameters/domain MUST match exactly, and the inputs are packed differently

The "one consent = one attestation across both paths" guarantee requires the two `nf` values to be
**bit-identical** for the same logical event. Risks:

- **Param/version match unverified.** The circuit uses circomlib `Poseidon` (specific round constants,
  MDS matrix, `t=5` for 4 inputs, BN254). The normal path uses "an audited Solidity Poseidon lib"
  (`library Poseidon { hash4(...) }`). circomlib Poseidon and common Solidity Poseidon libs (e.g.
  poseidon-solidity, the iden3 generated one) **must be the exact same instantiation** (same `t`,
  full/partial round counts, constants, and the same domain/`nRoundsF/P` for arity 4). There is **no test
  vector asserting `Poseidon_solidity.hash4(x) == circuit Poseidon(x)`**. If they differ, the two paths
  produce different nullifiers → **double-attest across paths** (see H-4) AND the "shared set" is an
  illusion. **Fix:** pin one Poseidon instantiation; add a CI cross-vector
  (`testvectors.json`) asserting Solidity == circom == Rust prover hash for the same 4-tuple.
- **Input packing mismatch.** Circuit inputs are native field elements: `dogTagId, relayer, subject` are
  field elements; the circuit nullifier is `Poseidon4(dogTagId, relayer, subject, consentNonce)`. The
  Solidity side does `Poseidon.hash4(c.dogTagId, uint160(c.relayer), uint160(c.subject), c.nonce)`. The
  address-as-field packing (`uint160`) must match how the circuit ingests `relayer`/`subject` as field
  elements (they are passed as `uint160`-valued field elements on-chain in `pub`, so this likely matches —
  but it is **unasserted** and `dogTagId` as a full `uint256` could exceed the field, see M-1). Pin and
  test. Severity: HIGH.

### Finding H-4 (High) — cross-path double-attestation IS possible despite the "shared nullifier" claim

Even if H-3 is fixed so both paths compute the same `Poseidon4(dogTagId, relayer, subject, nonce)`, the
two paths **consume different `nonce` spaces**:
- Normal path: requires/uses the **EIP-712 `nonce`** field of the consent (and the registry historically
  tracked `nonces[subject]` monotonic — note §11.8(a) DROPPED the monotonic-nonce check that research/11
  §2.1 had; it now relies ONLY on `consumed[nf]`).
- ZK path: the `consentNonce` is a **private** circuit input the prover chooses; the registry never sees
  the raw nonce, only `nf`.

For the SAME logical visit, the owner signs ONE consent with ONE nonce. If the owner signs an ECDSA consent
(normal) with nonce `k` AND an EdDSA consent (ZK) — these are **two different signatures the user may both
produce** (the mobile flow §10.1 signs whichever the verifier requests). If a malicious relayer requests
BOTH a normal and a ZK consent for the same `(dogTagId, subject, nonce)`, and the user approves both
(plausible — same "approve" tap, different mode), the nullifiers **coincide** and the second reverts —
GOOD. **But** research/11 §5.2 itself concedes "accept that the two paths produce different nullifiers" as
a fallback and suggests "indexer dedupe on `(dogTagId, relayer, day)`." The normative §11.8 claims perfect
coincidence; the research it derives from says coincidence may be impractical. **This contradiction must be
resolved.** If the prover can choose `consentNonce` freely in the ZK path (it is private and unconstrained
against any on-chain nonce counter), then a relayer holding one EdDSA consent could **mint multiple ZK
attestations with different `consentNonce` values** unless the EdDSA signature pins the nonce — it does:
`M = Poseidon4(dogTagId, relayer, rZk, nonce)`, so a different `nonce` needs a different signature. OK for
replay of a single consent. **The residual real gap:** because the normal path no longer enforces a
monotonic `nonces[subject]` (it was dropped in §11.8 vs research/11 §2.1), the **only** anti-replay on the
normal path is `consumed[nf]`. That is fine for exact replay, but it means a relayer can collect N distinct
consents (N nonces) and record N attestations — which is intended. **Fix:** (i) restore an explicit decision
on cross-path coincidence and document it as normative; (ii) if coincidence is required, the ZK
`consentNonce` must be tied to the same per-subject nonce space the normal path uses (hard, since ZK hides
it) — more realistically, **accept per-path one-time-ness and dedupe off-chain**, and DOCUMENT that a single
visit can be recorded once per path. Severity: HIGH (the spec currently over-claims a guarantee it does not
provide).

### Finding M-3 (Medium) — cross-purpose nullifier collision: `purpose`/`recordType` is NOT in the nullifier

`nullifier = Poseidon(dogTagId, relayer, subject, nonce)` omits `recordType`/`purpose`. So a single
nullifier covers ALL purposes for a `(dogTagId, relayer, subject, nonce)` tuple. Consequences:
- **Under-counting / griefing:** if a relayer legitimately needs to record TWO different verifications
  (e.g. `VET_INTAKE` and `TRAVEL_PRESENTATION`) for the same pet/subject under the same nonce, the second
  reverts as a "replay" though it is a distinct logical event. Forces a fresh nonce per purpose (extra
  consent signatures) — acceptable but should be documented.
- **No cross-purpose forgery** results (the nullifier is still one-time), so this is not a soundness break,
  but the omission means the on-chain `Verified.purpose` field (emitted `bytes32(0)` on ZK!) is NOT bound
  by the nullifier. On the ZK path `purpose` is emitted as `0x0` and `recordType` is private — so the event
  carries no purpose at all, and the whitelist check uses `keccak256("VERIFY:"‖bytes32(0))` (a SINGLE
  global VERIFY key), defeating the per-purpose VERIFY: namespace (§4.3). **Fix:** make `recordType` (or
  `purpose`) a public signal on the ZK path (or pin one purpose per circuit/verifying-key), include it in
  the nullifier AND in the EdDSA message, and use it for the whitelist key. Severity: Medium (privacy vs
  per-purpose-gating trade-off; currently the ZK path's VERIFY: gating is effectively a single boolean).

### Finding (confirm, OK) — nullifier as public signal, not proof-derived

**CONFIRMED CORRECT.** `nullifier` is `pub[3]`, a public signal, never derived from `(a,b,c)` bytes.
Groth16 proof malleability (snarkjs #383) yields the same public signals → same `nf` → `consumed[nf]`
still blocks. The design comment and code agree. No malleability double-spend on the nullifier. Good.

---

## 5. Public-signal range checks & field-element handling

### Finding (confirm, OK) — range checks present (snarkjs #358)

`recordVerificationZK` does `for (uint i; i<5; i++) require(pub[i] < SNARK_SCALAR_FIELD)` and pins the
correct BN254 `r`. This correctly addresses snarkjs #358 (a public signal ≥ field modulus aliasing to a
valid in-field value → double-spend). The normal path computes `nf` from in-field `uint160(relayer)` /
`uint160(subject)` (in-range by construction) — but `dogTagId` and `nonce` are `uint256` and could exceed
`r`; if the SBT can mint a `dogTagId ≥ r`, the on-chain `Poseidon.hash4` would reduce it mod `r` while the
circuit expects an in-field value → **nullifier mismatch / collision risk.** **Fix:** constrain
`dogTagId < SNARK_SCALAR_FIELD` (and `nonce < r`) at SBT mint or in `recordVerification`, and bit-constrain
`dogTagId` in-circuit (M-1). Severity: Medium (folded into M-1).

### Finding M-4 (Medium) — address-as-field-element packing unasserted

`relayer`/`subject` are 160-bit addresses cast to `uint160` then to field elements. 160 < 254 bits so no
field overflow — safe. But the **prover and the contract must agree** that `pub[1]`/`pub[2]` are exactly
`uint160(address)` with the high bits zero. `require(uint160(pub[1]) == uint160(msg.sender))` only checks
the low 160 bits; if the circuit allowed high bits in `relayer`, two distinct field values could map to the
same address on-chain while being distinct in the proof. **Fix:** bit-constrain `relayer`/`subject` to 160
bits in-circuit (`Num2Bits(160)`). Severity: Medium.

---

## 6. Trusted setup

`reuse Hermez/PPoT phase-1 + multi-party phase-2 (≥3) + public beacon`, publish transcript, pin `.zkey`
hash, ship in prover image.

### Finding M-5 (Medium) — plan is sound; blast-radius claim is OVERSTATED given C-1/C-2

The setup plan itself is correct ZK practice (reuse phase-1, ≥3-party phase-2, beacon, transcript,
`zkey verify`, pinned hash). A compromised phase-2 lets a holder of the toxic waste **forge proofs for
arbitrary public signals** — i.e. assert "consented, issued credential verified" for any
`[dogTagId, relayer, subject, nullifier, rZk]` **without a witness**. The docs claim the blast radius is
"contained" because the three-pillar trust model doesn't depend on ZK and the nullifier + `isValid` re-check
still constrain a forged attestation. **This is only partly true and is overstated:**
- A forged proof still needs `kecOf[rZk]` to map to an `isValid` `rKec`. With C-1 unfixed, the forger
  (who, with toxic waste, needs no real leaves) just `zkCommit`s any `rZk` to a real `rKec` first — so the
  `isValid` re-check is **not** an independent backstop; it is bypassable by the same actor.
- With C-2 unfixed, the forger doesn't even need toxic waste to forge consent (it can pick any `subject`).
- So the "contained blast radius" depends entirely on C-1 and C-2 being fixed. **After** those fixes, the
  claim holds: a phase-2 compromise forges *attestations* (a spurious "I-verified-it"), never *credentials*
  (issuance is keccak/DNS, ZK-independent) and never *data leakage* (Groth16 ZK holds regardless of setup).

**Fix (M-5):** (i) implement C-1/C-2 so the `isValid`/nullifier backstops are genuinely independent of the
prover; (ii) make the Groth16 verifier address `immutable` OR strictly timelocked+multisig — note §11.8 has
`setZkVerifier` as plain `onlyRole(DEFAULT_ADMIN_ROLE)` with a comment "timelocked" but **no timelock is
coded**; a compromised admin swapping in a malicious verifier forges all attestations. Code the timelock or
make it immutable. Severity: Medium (High if `setZkVerifier` ships un-timelocked).

---

## 7. Additional findings

### Finding H-5 (High) — `issuerForAny()` is undefined and breaks per-recordType `isValid` routing

`recordVerificationZK` calls `IDogTagIssuer(issuerForAny()).kecOf(rZk)` and `.isValid(rKec)`. `issuerForAny()`
is a comment-only stub ("resolves the issuer clone holding kecOf[rZk]... either a single protocol issuer or
per-circuit recordType pinned"). There is **one `DogTagIssuer` clone per record type per business** (§4.4),
each with its own `kecOf` mapping. With `recordType` private on the ZK path, the registry **cannot know
which clone holds `kecOf[rZk]`**. As written this either (a) assumes a single global issuer (contradicting
the per-type/per-business clone architecture) or (b) is unimplementable. Worse: if it scans/guesses the
wrong clone, `kecOf[rZk]==0 → revert`, but if an attacker can get `rZk` mapped in ANY clone it controls
(C-1), `issuerForAny` resolving to *that* clone makes the `isValid` check pass against an attacker clone.
**Fix:** make `rZk → (issuerClone, rKec)` a single authoritative mapping in the registry (or a dedicated
`ZkCommitmentRegistry` as research/10 §6.1 originally proposed with `commitments.issuerFor(Rzk)`), written
ONLY under the C-1 cryptographic binding. Do not leave clone resolution undefined. Severity: HIGH.

### Finding M-6 (Medium) — `Verified` event omits `rZk`/`credentialRoot` on ZK path but the prompt/§4.7 emit differs from §11.8

architecture §4.7 `recordVerificationZK` emits `Verified(pub[0], relayer, subject, purpose, nf, ts)` and
elsewhere implies `credentialRoot`/`rZk` are part of the record; §11.8(a) emits `purpose=0x0`,
`credentialRoot` absent. research/11 §2.1's event had a `bool zk` flag and `credentialRoot` field; §11.8
dropped both. Minor, but indexers/auditors cannot distinguish path or recover `rZk` from the event. Add
`rZk` (already public, non-personal) and a `zk` bool to the event for auditability. Severity: Low/Medium.

### Finding M-7 (Medium) — deadline absent on ZK path

The normal path enforces `block.timestamp <= c.deadline`. The ZK path has **no deadline** — `consentNonce`
and `nullifier` prevent replay, but a captured EdDSA consent + proof is **valid forever** until the
nullifier is consumed. A relayer that obtained consent months ago can still record it. **Fix:** add a
`deadline` public signal bound in the EdDSA message, range-checked, and `require(block.timestamp <=
deadline)` on-chain. Severity: Medium.

---

## 8. Findings summary

| ID | Severity | Title | Fix (one-line) |
|----|----------|-------|----------------|
| **C-1** | **Critical** | `zkCommit` binds two opaque roots with NO same-leaf-set proof | In-circuit keccak binding of `0x02‖rKec‖rZk` (research/10 Option C), make `rKec` a public input checked vs `isValid`; interim: restrict to original issuer |
| **C-2** | **Critical** | ZK path: `subject` unconstrained; consent key NOT bound to `subject` (registry check missing both in-circuit and on-chain) | Put `subject` in EdDSA message; expose `Poseidon(Ax,Ay)` public output; on-chain `keyOf[subject]==keyHash`; add `ownerOf==subject` |
| **H-1** | High | `dogTagIdLeafIndex` free + only one leaf bound to `rZk` | Constrain dogTagId leaf's keyPath; range-check index; build full tree so `rZk` commits all leaves |
| **H-2** | High | ConsentKeyRegistry vacuous (never consumed by ZK path) | Same as C-2 (the on-chain half) |
| **H-3** | High | Solidity vs circom Poseidon params/packing unverified | Pin ONE Poseidon instantiation; CI cross-vector Solidity==circom==Rust |
| **H-4** | High | "Shared nullifier across paths" over-claimed; cross-path double-attest possible/undocumented | Resolve coincidence decision normatively; document or enforce |
| **H-5** | High | `issuerForAny()` undefined; ZK `isValid` clone routing unimplementable/forgeable | Authoritative `rZk→(clone,rKec)` mapping written only under C-1 binding |
| **M-1** | Medium | No in-circuit range checks on typeTags/values/dogTagId | `Num2Bits` on numeric leaves + `dogTagId < r` |
| **M-2** | Medium | No consent-key rotation; per-pet vs per-wallet keyOf contradiction | Add gated rotate; resolve per-pet `subject` vs per-wallet `keyOf` |
| **M-3** | Medium | `purpose`/`recordType` not in nullifier; ZK emits purpose=0x0 → VERIFY: gating is a single boolean | Make recordType public (or per-circuit) + in nullifier + EdDSA + whitelist key |
| **M-4** | Medium | Address public signals not bit-constrained to 160b | `Num2Bits(160)` on relayer/subject in-circuit |
| **M-5** | Medium | Trusted-setup blast-radius overstated; `setZkVerifier` not actually timelocked | Fix C-1/C-2 so backstops are independent; code timelock or make verifier immutable |
| **M-6** | Low/Med | `Verified` event drops `rZk`/path flag → poor auditability | Emit `rZk` + `zk` bool |
| **M-7** | Medium | No `deadline` on ZK path → consent valid forever | Add deadline public signal, bound in EdDSA, checked on-chain |

**Confirmed-correct (no finding):** nullifier as public signal (anti-malleability, #383) ✅;
public-signal range check `< r` present (#358) ✅; relayer bound into consent (normal) / as public signal
(ZK) + `==msg.sender`, no EIP-2771/4337 ✅; `ConsentKeyRegistry.bindConsentKey` cannot bind a key to an
address you don't control (replay-safe) ✅; EIP-712 domain pins `chainId:135` + `verifyingContract` (no
cross-chain/cross-contract replay) ✅; keccak issuance untouched ✅.

---

## 9. Disposition

**Do NOT deploy the ZK path (`recordVerificationZK`) until C-1, C-2, H-1, H-2, H-5 are fixed.** They are
not independent hardening items — together they let an attacker choose `rZk`, `dogTagId`, AND `subject`
nearly freely and still pass every on-chain check, i.e. forge attestations over fabricated data without the
named subject's consent. The **normal (ECDSA) path is materially sounder** (it does `recover==subject`,
`ownerOf==subject`, `isValid(rKec)` directly on the issued root) and can ship independently once H-3/H-4
(nullifier params + cross-path semantics) and M-7-equivalents are settled. The keccak issuance core, DNS,
and three-pillar verification are unaffected by all of the above.
