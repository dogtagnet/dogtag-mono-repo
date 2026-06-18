# Audit 11 — DogTag v4 Poseidon-Unification Contract Review

> Scope: the **v4 Poseidon-unification** delta only — collapse the dual credential commitment
> (`rKec`/`rZk`) to a **single Poseidon root `R`** anchored by `DogTagIssuer.issue(R)`, with the
> `VerificationRegistry` (both paths) re-checking `isValid(R)` **directly** on the public root; the
> deletion of `zkCommit`/`ZkCommitment`/`kecOf`/`zkIndex`/`cloneOf`/`issuerForAny` and the `0x02`
> binding leaf; the on-chain normal-path `PoseidonT7` nullifier; and the admin-set
> `purposeToRecordType → issuerFor[recordType]` clone resolution that replaced `zkIndex`/`issuerForAny`.
> EVM/BN254/Poseidon-accurate.
> Canonical artifact under audit: **`implementation.md §11.9`** (overrides §4.7/§11.8 on conflict),
> cross-referenced against `§11.1`/`§11.8`/`§11.2`, `architecture.md §4.1/§4.4/§4.7/§13.8`,
> `CHANGESPEC-v4.md`.
> Regression baselines: `audit-04-contracts-v2.md` (v1 C-1/C-2/H-1/H-2/H-3, M-1, M-4; v2 hardened
> §11.6 confirm), `audit-08-contracts-v3.md` (v3 consent/Groth16; v3.1 subject↔key/ownerOf/purpose).
> Date: 2026-06-17. Auditor: contract security review.

---

## Severity legend
- **Critical** — forged/false attestation, auth bypass, a false/missing `isValid` result that an
  external party can drive, cross-path double-attest, or a deterministic DoS of the headline flow.
- **High** — privilege escalation or a missing control that breaks a stated security property.
- **Medium** — exploitable under specific conditions, divergence that will cause real bugs.
- **Low** — hardening / defense-in-depth.
- **Info** — observations / confirmations.

---

## Executive position

The unification is a genuine **soundness simplification**: deleting the off-chain
`rKec↔rZk` binding (`zkCommit`/`kecOf`) and the undefined `issuerForAny()`/`zkIndex` lookups removes the
two v3 Criticals (audit-07 C-1, audit-08 C-2) **at the root** — there is no off-chain binding left to be
unsound, and the circuit now proves exactly the root that `issue(R)` anchored. The v1/v2/v3.1
remediations survive the change intact (full table below), and the surviving v3.1 ZK gates (subject↔key,
`ownerOf`, purpose binding, range-checks, nullifier-as-public-signal) are all still present in §11.9(e).

**However, the unification introduced one new Critical of its own.** The deleted clone-resolution
machinery (`zkIndex`/`issuerForAny`) was replaced by `issuerFor[purposeToRecordType(purpose)]` — a map
**keyed only by `recordType`**. But the protocol deploys **one `DogTagIssuer` clone per recordType _and
per business_** (arch §4.1 line 266, §4.4 line 333; `businesses.documentStores{recordType→addr}` is
per-business, impl §9.1). A single-valued `issuerFor[recordType]` **cannot identify which of the N
per-business clones actually holds `issuedAt[R]`**, so `isValid(R)` is queried against **one arbitrary
admin-pinned clone** — wrong for every credential issued by any other business of that recordType. This
is a **false-negative DoS for the common case** and, in the adversarial case, a **false-positive**: a
relayer can record a verification of root `R` against the pinned clone when in fact a _different_ root
`R'` is what is valid there, or when `R` was issued-then-revoked in the real clone but coincides with a
still-valid root in the pinned clone. **V4-C1 below.** This is the same class of bug as audit-08 V3-C2
(reading `isValid` from a clone not bound to the proof) — the unification deleted the *symptom*
(`issuerForAny`) but re-created the *disease* with `issuerFor[recordType]`.

Two further items: a **High** that the on-chain `PoseidonT7` nullifier must be CI-proven bit-identical to
the circuit or the shared `consumed` set is bypassable (carried forward from V3-C1(1) — unification does
**not** resolve it), and a **Medium** that `setZkVerifier`'s timelock is still only a comment.

Net: the deletions are clean and the regressions hold, but **V4-C1 (issuer-clone resolution) makes both
`isValid(R)` paths unsound as written** and must be fixed before deploy.

---

# CRITICAL

## V4-C1 — `issuerFor[recordType]` cannot resolve which per-business clone holds `issuedAt[R]` → `isValid(R)` queries the wrong clone (false-negative DoS; worse, false-positive against a different business's root)

**Where:** `implementation.md §11.9(e)` (canonical ZK body) and its footnote, and §11.8(a)
(normal-path body), both resolving the clone via the per-`recordType` map:
```solidity
// ZK path (§11.9(e)):
address clone = issuerFor[purposeToRecordType(bytes32(pub[1]))]; require(clone != address(0));
require(DogTagIssuer(clone).isValid(bytes32(pub[6])));         // isValid(R) DIRECTLY on the public root
// normal path (§11.8(a)):
address iss = issuerFor[c.recordType]; require(iss != address(0), "no issuer");
require(IDogTagIssuer(iss).isValid(c.credentialRoot), "cred !valid");
```
with `mapping(bytes32 => address) public issuerFor;  // recordType => DogTagIssuer clone` (§11.8(a),
line 1469) and `purposeToRecordType` an admin-set `mapping(bytes32 purpose => bytes32 recordType)`
(§11.9(e) footnote, line 1673).

**Why it matters — the map arity is wrong for the deployment topology.** The design is explicit that
clones are **one per recordType _and per business_**:
- arch §4.1 line 266: *"deploys DogTagIssuer EIP-1167 clones (one per record type **/ per business**)."*
- arch §4.4 line 333: *"**One clone per record type** (and per business, so each business's issuance is
  independently revocable/auditable)."*
- impl §9.1 line 589: each business registry row carries its own
  `documentStores{recordType→addr}` — i.e. **N distinct VACCINATION clones**, one per vet.
- The clone is chosen at issue time by the issuing backend
  (`issuerAddrFor(recordType)` → that business's own clone, impl §3.3/§3.8) and is recorded **per
  credential** in `doc.issuer.documentStore` (arch §3.1). The off-chain `verify` 3-pillar check reads
  `isValid` from `doc.issuer.documentStore` — **the correct, per-credential clone** (impl §11.3 line
  1206). The on-chain registry has no such per-credential pointer.

So `R` for a VACCINATION credential issued by Vet A lives in Vet A's VACCINATION clone, and Vet B's
VACCINATION root `R_B` lives in Vet B's clone. A single global `issuerFor[keccak256("VACCINATION")] =
oneClone` collapses all of them to **one** admin-chosen address. Concrete breaks:

1. **False-negative DoS (common case).** For every credential issued by a business *other than* the one
   pinned in `issuerFor[recordType]`, `clone.isValid(R)` reads `issuedAt[R]==0` on the pinned clone
   (the root was anchored in a *different* clone) → `isValid` returns `false` → `recordVerification` /
   `recordVerificationZK` **reverts `cred !valid`** for a perfectly valid credential. With more than one
   business per recordType (the entire multi-tenant premise), proof-of-verification is broken for all
   but one business. This is a deterministic, unconditional DoS of the headline v4 flow.

2. **False-positive against a different business's root (worse).** `isValid` is true iff
   `issuedAt[R]!=0 && revokedAt[R]==0` **on the pinned clone**. Because the same `bytes32` root can be
   anchored in multiple clones (roots are not clone-scoped — audit-04 V2-C1 clause 1 / I-3, and
   unification did not add clone-binding to the leaf), a relayer presenting root `R` can be accepted
   against the pinned clone whenever *that* clone has `R` issued-and-not-revoked — **even though the
   credential the consent/proof is about was issued (or revoked!) in a different clone.** Two attack
   shapes:
   - **Revocation evasion.** Vet A issues `R`, later **revokes** `R` in Vet A's clone (e.g. the
     vaccination cert was withdrawn). If `R` was also anchored in the pinned clone (collusion, or the
     same logical content batch-anchored elsewhere) and not revoked there, `isValid(R)` still reads
     `true` → a verification is recorded for a **revoked** credential. The whole point of on-chain
     `isValid` (revocation is first-class, mirrors `credentialStatus`) is defeated, because the read is
     decoupled from the clone that owns the credential's lifecycle.
   - **Wrong-issuer pass.** The ZK proof binds `R` to the leaves and `dogTagId`, and the normal path
     binds `R` to the ECDSA consent — but **neither binds `R` to a particular clone.** The registry is
     the only component that maps "this credential → its issuer state," and it maps to the wrong issuer.
     This is **exactly audit-08 V3-C2's finding** ("reading `isValid` from an issuer not bound to the
     proof"): the unification deleted `issuerForAny()` but `issuerFor[recordType]` is the same defect
     with a different name — the on-chain `isValid` re-check (the entire reason the circuit doesn't
     prove issuance) is **decoupled from the credential the attestation is about.**

3. **`purposeToRecordType` adds a second admin-trusted indirection** with the same arity problem: even
   resolving `purpose → recordType` correctly, the result indexes a recordType-keyed map that still
   can't pick the business clone. (It also re-introduces an admin-governed mapping the audit-08 fixes
   tried to remove from the trust path — now the admin must keep *two* maps consistent.)

**Why the docs' "RESOLVED-by-unification" claim is incomplete.** arch §13.8 / impl §11.9 mark audit-08
C-2 (`issuerForAny`) RESOLVED because "the registry calls `isValid(R)` directly." Direct is necessary
but **not sufficient**: *direct on which clone?* The single root `R` removed the `rZk→rKec` hop, but the
clone-selection problem the `rZk→rKec` hop was *also* papering over (recordType private on ZK; multiple
clones per recordType) is unchanged. **`isValid(R)` "directly" still needs the right clone**, and
`issuerFor[recordType]` is not it.

**Fix (recommended — global root→clone map written at `issue`):**
1. **Add a protocol-global `rootIssuer[R] = clone` written atomically at `issue(R)`.** The natural,
   minimal fix is a single authoritative registry the issuer clones write to:
   ```solidity
   // RootRegistry (new, or a mapping on a shared singleton the factory wires into every clone)
   mapping(bytes32 => address) public rootIssuer;   // R => the clone that anchored it (write-once)
   function noteIssued(bytes32 R) external onlyClone { 
       require(rootIssuer[R] == address(0), "root taken"); rootIssuer[R] = msg.sender; 
   }
   ```
   `DogTagIssuer.issue(R)` calls `rootIssuer.noteIssued(R)` (gated so only factory-deployed clones can
   write — the factory records each clone, or the clone proves its `initialize` lineage). The
   `VerificationRegistry` then resolves `clone = rootRegistry.rootIssuer(R); require(clone != 0);
   require(DogTagIssuer(clone).isValid(R));` — **the clone is now derived from the root itself**, bound
   to wherever it was actually anchored, with no admin map and no recordType ambiguity. This also makes
   `purposeToRecordType` unnecessary for clone resolution (keep it only if `recordType` is needed for
   the event/whitelist semantics).
   - **Write-once `rootIssuer[R]`** prevents a second clone from re-anchoring the same `R` and hijacking
     resolution (closes the cross-clone replay at the resolution layer; complements leaf-binding).
   - `isValid(R)` then reads issued-and-not-revoked **on the clone that owns `R`'s lifecycle**, so
     revocation is honoured and a different business's root cannot pass.

2. **Alternative (if a new singleton is undesirable): make the verifier carry the clone.** Have the
   relayer pass the credential's `documentStore` (clone address) as an explicit argument, and have the
   registry verify it is a factory-known clone for `recordType` (factory emits/keeps a
   `isClone[recordType][addr]` set) **before** calling `isValid(R)` on it. On the ZK path, bind the
   clone (or `documentStore`) into the circuit as a public signal so a relayer can't substitute a clone
   where `R` happens to be valid. This is strictly more work than (1) and still needs a factory-side
   clone allow-list; prefer (1).

3. **Leaf-bind the root to the issuer (defense-in-depth, also audit-04 V2-C1 #3 / M-3).** Include
   `(dogTagId, recordType, issuerEntityId)` (or the clone address) among the Poseidon leaves so `R` is
   unique to one clone's issuance and cannot collide across businesses. Then even an arity bug cannot
   cause a cross-business false-positive, because `R` itself differs per issuer. (Independent of the
   resolution fix; do both.)

4. **CI/Foundry tests:** (a) two businesses each issue under VACCINATION; a verification of business B's
   root **must not** revert and **must not** resolve business A's clone; (b) a root issued in clone A
   then revoked in clone A reverts `cred !valid` even if the same `bytes32` is issued-and-valid in clone
   B; (c) `rootIssuer[R]` is write-once (second `issue(R)` from another clone reverts); (d) a relayer
   cannot pass an arbitrary/non-clone address as the issuer.

> **Verdict on the central question:** **NO — `issuerFor[recordType]` does NOT uniquely resolve the
> clone that issued `R`.** With multiple clones per recordType (one per business, by design), the
> recordType-keyed map points at a single admin-pinned clone, so `isValid(R)` queries the wrong clone:
> a false-negative DoS for every other business, and a false-positive (different/stale root valid in the
> pinned clone) in the adversarial case. The correct fix is a global `rootIssuer[R]=clone` written at
> `issue(R)`, deriving the clone from the root itself.

---

# HIGH

## V4-H1 — On-chain `PoseidonT7` nullifier must be CI-proven bit-identical to the circuit, or the shared `consumed` set is bypassable (cross-path double-attest)

**Where:** `implementation.md §11.9(b)` + §11.8(a) normal path:
```solidity
uint256 p = uint256(c.purpose) % SNARK_SCALAR_FIELD;
bytes32 nf = bytes32(PoseidonT7.hash(
    [uint256(DS_NULLIFIER), c.dogTagId, p, uint160(c.relayer), uint160(c.subject), c.nonce]));
```
vs the circuit output (§11.9(d)/§11.8(d)):
`nullifier == Poseidon(DS_NULLIFIER, dogTagId, purpose, relayer, subject, consentNonce)`.

**Why it matters.** The single `consumed` set only prevents recording the *same* logical event on both
paths if `PoseidonT7.hash(...)` on-chain equals the circom `Poseidon` output for identical inputs —
**same BN254 field, round constants, MDS, R_F/R_P, the domain tag in the same first-slot position, and
the same input encoding** (addresses as the low-160-bit field element; `purpose` reduced mod `p`
identically on both sides). This is the **unification-surviving half of audit-08 V3-C1**: V3-C1's
`recordType`-absent-from-nullifier sub-bug is fixed (purpose is now in the nullifier on both paths,
§11.9(b)), but **V3-C1(1) — the on-chain-Poseidon-vs-circuit parameter-match requirement — is not
resolved by unification; it is now the *only* thing standing between the two paths and a double-attest.**
If `poseidon-solidity PoseidonT7` is not the exact circomlib parameterization the circuit uses, the two
paths produce different `nf` for the same consent → one consent can be recorded **once per path**
(double-count / double-bill). §11.2 mandates the pinned libs + the `poseidon([1,2])` anchor vector, and
§11.9(b) says "CI asserts Solidity == circom == Rust" — **good, but it is a process control, not a code
control.** Until that CI gate is green, the "one consent = one attestation across paths" guarantee is
**unverified**.

**EVM-accuracy notes (confirmed, not findings):**
- Inputs `uint160(relayer)`, `uint160(subject)` are `< 2^160 < r` — never overflow BN254. `c.dogTagId`
  and `c.nonce` must be range-checked `< r`; the normal path does **not** range-check them on-chain
  (the ZK path does, §11.9(e) `for i<7`). `dogTagId` is allocated `< p` (§11.2 line 1188), and `nonce`
  is client-chosen — a `nonce ≥ r` would make `PoseidonT7` reduce it mod `r`, and a circom witness using
  the unreduced `nonce` would mismatch. **Minor: range-check `dogTagId`/`nonce < SNARK_SCALAR_FIELD` on
  the normal path too** (folds into V4-H1's parity contract; cheap).
- `t=7` (6 inputs + capacity) is the correct circomlib width for the 6-input nullifier; `PoseidonT7`
  matches. Domain tag `DS_NULLIFIER=4` in slot 0 matches §11.2.

**Fix:**
1. **Make the parity a Foundry/integration test, not just a doc mandate:** compute `PoseidonT7.hash(v)`
   on a deployed `poseidon-solidity` instance and assert equality with the circuit's `nullifier` output
   for the same `v` (run the prover), **and** against `light-poseidon (new_circom)` and `poseidon-lite`.
   Pin the `poseidon-solidity` deployed address + bytecode hash. Gate deploy on this test.
2. Add the `poseidon([1,2]) = 0x115cc0f5…189a` anchor vector to the same CI lane for `PoseidonT7`
   (§11.2(d)) — reject the lib at the lockfile if it drifts.
3. Add `require(c.dogTagId < SNARK_SCALAR_FIELD && c.nonce < SNARK_SCALAR_FIELD)` on the normal path so
   both paths feed identically-ranged field elements into Poseidon.
4. **Confirm `poseidon-solidity` compiles under `evm_version=paris`** (no `PUSH0`/`MCOPY` assumptions) —
   ROAX pins paris (arch §13.1/§12); a lib that emits PUSH0 will not run. Deploy-time pre-check.

---

# MEDIUM

## V4-M1 — `setZkVerifier` / `setIssuerFor` / `purposeToRecordType` are admin setters with no timelock (the comment still says "timelocked")

**Where:** `implementation.md §11.8(a)`:
```solidity
function setIssuerFor(bytes32 rt,address i) external onlyRole(DEFAULT_ADMIN_ROLE){ issuerFor[rt]=i; }
function setZkVerifier(address v) external onlyRole(DEFAULT_ADMIN_ROLE){ zkVerifier=IGroth16Verifier(v); } // timelocked
```
plus the new admin-set `purposeToRecordType` map (§11.9(e) footnote).

**Why it matters.** Carried forward from audit-08 V3-M5 and still unresolved in the canonical body.
`AccessControlDefaultAdminRules(2 days,…)` delays only *admin transfer*, not these role-gated setters. A
rogue/compromised admin can in **one tx**: (a) point `zkVerifier` at a contract whose `verifyProof`
returns `true` for any input (forge ZK attestations, subject only to the `isValid(R)`/keyOf/ownerOf
re-checks — satisfiable by choosing a real issued `R`); or (b) repoint `issuerFor[rt]` /
`purposeToRecordType` at an attacker clone whose `isValid` returns `true`. Under **V4-C1** the
`issuerFor` swap is *especially* potent — it is already the mis-resolution vector, and an unrestricted
setter lets the admin retarget it at will. impl §11.9(k) explicitly requires a **real timelock** on
`setZkVerifier`; the code does not implement it.

**Fix:** Route admin config through an OZ `TimelockController` holding `DEFAULT_ADMIN_ROLE`, or add a
propose/commit-with-delay in the contract, for `setZkVerifier`, `setIssuerFor`, and (if retained)
`purposeToRecordType`. Emit `ZkVerifierChanged(old,new)` / `IssuerForChanged(rt,old,new)`. If V4-C1 is
fixed with the `rootIssuer[R]` map, the `setIssuerFor` exposure shrinks (clone is derived from the root,
not an admin map) — but the verifier swap still needs a timelock. Reconcile the **2-day** vs
`IssuerRegistry` **3-day** delays.

---

# LOW

## V4-L1 — `Verified.purpose` event topic on the ZK path now carries the real `purpose` (good) — confirm normal path is consistent and Art. 9 labels are non-sensitive
Unification + v3.1 made `purpose` a real public signal, so the §11.9(e) event emits `bytes32(pub[1])`
(the actual purpose), not the old `bytes32(0)`. This **resolves audit-08 V3-L2** (purpose=0 conflation)
for the ZK path. Confirm the normal path emits `c.purpose` (it does, §11.8(a) line 1499). Keep §11.9(h)'s
rule that `purpose` labels are non-sensitive (no Art. 9 leak in the cleartext topic). No code change;
add a test that the emitted `purpose` round-trips to the off-chain `verification_records.purpose`.

## V4-L2 — `purposeToRecordType` is dead weight if V4-C1 is fixed with `rootIssuer[R]`
If the recommended `rootIssuer[R]=clone` fix lands, clone resolution no longer needs `recordType`, so
`purposeToRecordType` exists only to (optionally) label the event/whitelist. Prefer signing/exposing
`recordType` where actually needed rather than maintaining a second admin map (one fewer admin-trusted
indirection — see V4-M1). Defense-in-depth, not a vuln.

---

# INFO / CONFIRMATIONS

- **I-1 — Deletion cleanup is complete in the canonical artifacts.** `zkCommit`, `ZkCommitment`,
  `kecOf`, `zkIndex`, `cloneOf`, `issuerForAny`, the `0x02` binding leaf, and the parallel
  `hashLeafZk`/`rZk` are removed from the code-bearing sections (§11.1 `DogTagIssuer` has no `kecOf`/
  `zkCommit`; §11.9(c)/(e) delete the lookups; §1.4 returns a single `R`). **Residual doc hygiene
  (not a contract bug):** §2.2's `DogTagIssuer` body and §11.8's ZK body are explicitly retained as
  pre-unification drafts with "CODE §11.1/§11.9" banners (lines 376-381, 1431-1433, 1502-1509). They
  are correctly marked superseded, but shipping both still invites a coder copying the wrong one —
  recommend deleting the stale bodies or fencing them in a clearly non-normative appendix (echoes
  audit-04's standing caveat about §2 vs §11.1).

- **I-2 — `issue(R)` is pure storage; zero on-chain hashing.** §11.1 `issue` is
  `require(r!=0 && issuedAt[r]==0); issuedAt[r]=block.timestamp; issuedBy[r]=msg.sender; emit`. No
  Poseidon, no keccak — a single `bytes32` SSTORE (~20k cold) + event. Confirmed; the only on-chain
  Poseidon is the normal-path `PoseidonT7` nullifier (V4-H1) and the optional future `MerkleVerifierLib`
  (§5.4, not in v1). **No new precompile dependency from issuance** (see ROAX below).

- **I-3 — ZK public signals are the unified 7-tuple.** `[dogTagId, purpose, relayer, subject,
  nullifier, keyHash, R]` across the circuit (§11.8(d) `main`), `IGroth16Verifier.verifyProof(...,
  uint[7])`, the prover (§3.10), and `recordVerificationZK` (§11.9(e)). `R` (pub[6]) replaced `rZk`.
  All 7 are range-checked `< SNARK_SCALAR_FIELD` before use (#358). Consistent.

- **I-4 — R lifecycle / originator binding intact (audit-04 H-1).** `issue(R)` guards `R!=bytes32(0)`
  (so `isValid(0)==false` always — the audit-08 backstop holds) and `issuedAt[R]==0` (issue-once per
  clone). `revoke(R)` requires `issuedAt[R]!=0 && revokedAt[R]==0` and `msg.sender==issuedBy[R] ||
  admin` (H-1 originator binding). `isValid(R) = issuedAt!=0 && revokedAt==0`. **Two credentials
  colliding on the same `R`:** requires a Poseidon second-preimage/collision over BN254 (≈2^128,
  negligible — and the per-field 16-byte salts make leaves unique, §3.2). The real `R`-collision risk is
  **not cryptographic but topological** — the *same* `bytes32 R` anchored in two clones — which is the
  V4-C1 cross-clone vector, addressed there (write-once `rootIssuer[R]` + leaf-binding). `R!=0` guard and
  issue-once guard both confirmed present.

- **I-5 — Normal-path on-chain Poseidon (`PoseidonT7`) is pure EVM arithmetic — no BN254 precompile.**
  The nullifier hash is field multiplies/adds mod `r` in `uint256`; it needs **no** `ecAdd/ecMul/
  ecPairing`. Gas is non-trivial — a `t=7` circomlib Poseidon on the EVM is tens of thousands of gas
  (round constants + MDS multiplies; commonly ~30-70k for this arity), on top of `ECDSA.recover`
  (~3k), three STATICCALLs (`ownerOf`, `isValid`, `isWhitelistedFor`), the `consumed` SSTORE (~20k
  cold), and the event — call it ~80-130k for the normal path. Bounded (one fixed-arity hash) → not a
  DoS. **Confirm `poseidon-solidity` compiles/runs under `paris`** (V4-H1 fix #4). Document per-attestation
  gas so relayers fund correctly.

- **I-6 — Reentrancy posture unchanged and safe.** Both paths make only STATICCALLs (`ownerOf`,
  `isValid`, `keyOf`, `isWhitelistedFor`, `verifyProof`, `PoseidonT7` view) and set `consumed[nf]` with
  checked-then-set, no interleaved mutating external call. No reentrancy. (Same as audit-08 I-5.)

- **I-7 — Relayer binding, signature/proof malleability, EIP-712 domain — all still sound.** Normal:
  `msg.sender==c.relayer` + `relayer` inside the signed struct; OZ `ECDSA.recover` rejects high-`s`/
  bad-`v`. ZK: `relayer`=pub[2] bound `==msg.sender`; nullifier is a public signal (#383) so malleated
  `(a,b,c)` yields the same `nf`, still blocked by `consumed`. EIP-712 domain
  `{DogTag,1,135,VerificationRegistry}`; the `VerificationConsent` typehash now includes `purpose` +
  `challenge` (§11.9(a)) and the SDK typehash (§1.10) must match in lockstep (breaking typehash change —
  do before any signature is collected). Unchanged by unification except the typehash addition.

---

# Regression review — v1 / v2 / v3.1 after the v4 unification

| Finding | Status in v4 | Evidence |
|---|---|---|
| **v1 C-1** `_disableInitializers()` on `DogTagIssuer` impl | **INTACT** | §11.1 `constructor(){ _disableInitializers(); }`; unification removed code (`zkCommit`), added no new init surface. |
| **v1 C-2** per-recordType + dedicated `PROFILE_ISSUER_ROLE` scoping | **INTACT** | §11.1 `isWhitelistedFor(rt,s)`; `DogTagIssuer.onlyWhitelisted(recordType,...)`; SBT `onlyProfileIssuer`. *Verify namespace:* the ZK `VERIFY:` key is now **purpose-specific** (`keccak256("VERIFY:"||pub[1])`, §11.9(e)) — the v3 `bytes32(0)` global key (audit-08 V3-H2) is **fixed**, no longer a C-2-in-spirit regression. |
| **v1 H-1** originator binding (`issuedBy`, revoke guard) | **INTACT** | §11.1 `issuedBy[r]=msg.sender` on issue; revoke requires `msg.sender==issuedBy[r] \|\| admin`. The deleted `zkCommit` was the only place lacking an originator check (audit-08 V3-M2) — **deletion removes that gap entirely.** |
| **v1 H-2** admin-only burn | **INTACT** | §11.1/§11.7(a) `burn` is admin-gated; verification leg never burns. |
| **v1 H-3** admin hardening (`AccessControlDefaultAdminRules`, multisig, duty split) | **INTACT** | `IssuerRegistry`(3 days)+`WHITELIST_ADMIN`; `VerificationRegistry`(2 days). **Nit:** `setZkVerifier`/`setIssuerFor` still not timelocked (V4-M1); 2-vs-3-day delay mismatch unresolved. |
| **v1 M-1** permissioned factory + deterministic salt | **INTACT** | §11.1 `createIssuer onlyRole(ADMIN)`, `salt=keccak256(recordType,business)` — note the salt is **per (recordType, business)**, which is exactly why there are N clones per recordType and why **V4-C1's recordType-only map is wrong.** |
| **v1 M-4** `evm_version=paris` + N-confirmation reads | **INTACT, with carried gate** | §11.8/§11.9 pin paris. **New deploy gate:** `poseidon-solidity` (V4-H1) and `Groth16Verifier` must compile under paris (no PUSH0/MCOPY); ROAX must expose BN254 precompiles for Groth16 (ROAX item below). Not a regression. |
| **v2 hardened §11.6 confirm** (signer from tx, calldata/to/value/chainId bind, emitting-contract pin, N-confirmations, idempotency) | **INTACT & REUSED** | Verify submission routes through `submitViaPrepareConfirm` → §11.6 (§11.8(g)/§3.9). §11.9(f) **generalizes the confirm to assert the `Verified` event** (emitted by the registry address) + `consumed[nf]==true` at N confirmations — not `RootIssued` — closing the audit-08 confirm-path note. **Confirmed verify submission still routes through the hardened §11.6 confirm asserting `Verified`.** |
| **v2 V2-H2 / v3 lost-key** (ownership contextual; consent-key rotation) | **INTACT** | §11.3 ownership is contextual (3 pillars gate validity); §11.9(j) makes `ConsentKeyRegistry` per-pet + **rotation** (no one-time lockout). Note §11.8(b)'s body still shows the one-time `require(keyOf==0)` guard — §11.9(j) overrides it; ensure the shipped code uses the rotation variant. |
| **v3.1 subject↔key** (`keyOf[subject]==keyHash`) | **INTACT** | §11.9(e) `require(uint256(consentKeys.keyOf(subject)) == pub[5])`; `keyHash` is public signal pub[5]. |
| **v3.1 `ownerOf(dogTagId)==subject`** | **INTACT (both paths)** | normal §11.8(a) `require(sbt.ownerOf(c.dogTagId)==c.subject)`; ZK §11.9(e) `require(sbt.ownerOf(pub[0])==subject)`. |
| **v3.1 purpose binding** (signed, in nullifier, in whitelist key, EdDSA message) | **INTACT** | `purpose` in `VerificationConsent` (§11.9(a)), in the nullifier (§11.9(b)), keys `VERIFY:` (§11.9(e)/§11.8(a)), and in the circuit EdDSA message `Poseidon(dogTagId,purpose,relayer,subject,R,nonce)` (§11.8(d)). |
| **v3.1 purpose-scoped VERIFY whitelist** | **INTACT** | `keccak256("VERIFY:"||purpose)` both paths; the v3 `bytes32(0)` ZK global key is gone. |
| **v3.1 HMAC relay / one-time challenge / 5-min deadline** | **INTACT** | §11.9(g)/(a) — `/verify/consent/submit` HMAC-signed, challenge consumption, `deadline=now+5min`. |
| **v3.1 real `setZkVerifier` timelock** | **NOT DONE (carried)** | §11.9(k) requires it; §11.8(a) setter still has no delay (V4-M1). |
| **v3.1 Art. 9 exclusion** | **INTACT** | §11.9(h): `SERVICE_ATTESTATION` has no on-chain root → not verifiable on-chain (rejected at registry+backend); `purpose` labels non-sensitive. |
| **audit-07 C-1 / audit-08 C-2** (off-chain binding / `issuerForAny`) | **RESOLVED-by-unification (symptom) — but see V4-C1 (re-created)** | The off-chain `rKec↔rZk` binding and `issuerForAny()` are deleted (§11.9(c)). **However the clone-resolution problem `issuerForAny` was hiding is re-created by `issuerFor[recordType]` — V4-C1.** The Criticals are resolved in *form* (no off-chain binding) but the *clone-selection soundness* regressed into V4-C1. |

**Net:** No v4 edit **undoes** a v1/v2/v3.1 remediation; several are *improved* by the deletions (the
`zkCommit` originator gap V3-M2 and the `bytes32(0)` ZK whitelist V3-H2 both vanish; the
`Verified.purpose` topic is real; the §11.6 confirm is generalized to `Verified`). The unification's
own new surface introduces **one Critical (V4-C1, clone resolution)** and carries forward **one High
(V4-H1, on-chain Poseidon parity)** and **one Medium (V4-M1, verifier/issuerFor timelock)** that the
canonical code still does not implement.

---

# Recommended Foundry / integration tests (v4)
- **V4-C1:** two businesses issue under one recordType; verification of business B's root does not
  resolve/clone-query business A; a root issued-then-revoked in clone A reverts `cred !valid` even when
  the same `bytes32` is valid in clone B; `rootIssuer[R]` write-once (second `issue(R)` from another
  clone reverts); a relayer cannot supply a non-clone issuer address.
- **V4-H1:** on-chain `PoseidonT7.hash(v) == circuit nullifier(v) == light-poseidon(new_circom)(v) ==
  poseidon-lite(v)` for identical `v`; the `poseidon([1,2])` anchor vector passes for the deployed
  `PoseidonT7`; a normal-path and a ZK attestation for the same logical event share one `nf` and the
  second reverts `replayed`; normal path reverts on `dogTagId`/`nonce >= SNARK_SCALAR_FIELD`.
- **V4-M1:** `setZkVerifier`/`setIssuerFor`/`purposeToRecordType` are timelocked/multisig-gated; emit
  events.
- **Regression:** all audit-01 (C-1/C-2/H-1/H-2/H-3/M-1), audit-04 (V2 hardened §11.6 confirm), and
  audit-08 (v3.1 subject↔key/ownerOf/purpose/whitelist) Foundry tests still pass; verify submission uses
  §11.6 confirm with the **`Verified`-event** assertion (§11.9(f)).
- **Deploy pre-check:** ROAX exposes BN254 `ecAdd/ecMul/ecPairing` (Groth16 verify); `poseidon-solidity`
  + `Groth16Verifier` compile and run under `evm_version=paris` (no PUSH0/MCOPY); `setIssuerFor`/
  `rootIssuer` wiring done for every recordType (else both paths revert at clone lookup).

---

# ROAX-specific (Q6)
- **BN254 pairing precompiles (`ecAdd 0x06`, `ecMul 0x07`, `ecPairing 0x08`) remain REQUIRED** for the
  ZK path's `Groth16Verifier.verifyProof` (~211k gas). Unchanged by unification — the verifier is the
  snarkjs-generated alt_bn128 contract regardless of single-vs-dual root. Deploy-time pre-check, like the
  RPC-liveness note in the arch header. If ROAX lacks them, the ZK path does not work at all.
- **Issuance adds NO new precompile dependency** — `issue(R)` is a pure `bytes32` SSTORE (I-2).
- **The normal-path `PoseidonT7` nullifier needs NO precompile** — pure EVM arithmetic mod `r` (I-5).
  Its only ROAX constraint is `paris` compatibility of `poseidon-solidity` (V4-H1 #4).

---

# Verdict

**Not deployment-ready.** The Poseidon unification is a real soundness *improvement* — it deletes the
two v3 Criticals (`zkCommit` off-chain binding, `issuerForAny`) at the root, the v1/v2/v3.1 remediations
are all intact, and the surviving ZK gates (subject↔key, `ownerOf`, purpose binding, range-checks,
nullifier-as-public-signal) plus the §11.6 hardened confirm asserting `Verified` are present. **But the
replacement clone resolver `issuerFor[recordType]` does NOT uniquely identify the per-business clone that
holds `issuedAt[R]`** (there are N clones per recordType, one per business), so both `isValid(R)` paths
query the wrong clone — a false-negative DoS for every non-pinned business and a false-positive
(different/stale root valid in the pinned clone) in the adversarial case (**V4-C1**). Fix V4-C1 with a
global `rootIssuer[R]=clone` written at `issue(R)` (clone derived from the root, write-once) plus
issuer leaf-binding; ship the V4-H1 on-chain-Poseidon↔circuit parity test (or the shared `consumed` set
is bypassable); and implement the V4-M1 timelock on `setZkVerifier`/`setIssuerFor` before any ROAX deploy.
