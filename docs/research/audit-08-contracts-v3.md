# Audit 08 — DogTag v3 On-Chain Proof-of-Verification (consent + Groth16)

> Scope: the **NEW v3 contracts** — `VerificationRegistry`, `Groth16Verifier`, `ConsentKeyRegistry`,
> and the `DogTagIssuer.zkCommit`/`kecOf` additions — plus their interaction with the existing
> (audited) `IssuerRegistry` / `DogTagIssuer` / `DogTagSBT`. EVM-accurate, BN254/Poseidon-aware.
> Out of scope except where they change an on-chain trust assumption: SDK internals, prover-rs,
> mobile, backends, calendar.
> Canonical artifact under audit: **`implementation.md §11.8`** (the NORMATIVE corrected bodies),
> cross-referenced against `architecture.md §4.7/§13.6/§13.7`, `CHANGESPEC-v3.md §0–§5`, and
> `research/11-consent-attestation.md`. Where §11.8 (canonical) and research/11 (design sketch)
> diverge, **§11.8 is the artifact**; the divergence is itself flagged where it matters.
> Regression baselines: `audit-01-contracts.md` (v1 C-1/C-2/H-1/H-2/H-3, M-1, M-4),
> `audit-04-contracts-v2.md` (V2-C1/C2/H1/H2/H3, hardened §11.6 confirm).
> Date: 2026-06-17. Auditor: contract security review.

---

## Severity legend
- **Critical** — direct forgery of a verification attestation, auth bypass, a false "isValid" pass,
  cross-path double-attest, or an unrecoverable/unbound on-chain fact reachable by an external party.
- **High** — privilege escalation among scoped relayers, or a missing control that breaks a stated
  security property (relayer binding, purpose scoping, nullifier soundness, subject↔key binding).
- **Medium** — exploitable under specific conditions, DoS, or spec/impl divergence that will cause bugs.
- **Low** — hardening / defense-in-depth.
- **Info** — observations / confirmations.

---

## Executive position

The v3 design is **directionally strong**: the consent binds the relayer *into* the signed payload and
re-enforces `msg.sender == relayer` (the property EIP-2771 lacks), the nullifier is a **public signal**
(not derived from malleable proof bytes), the ZK path **range-checks all public signals**, the registry
re-checks `isValid` on-chain rather than trusting the circuit, and keccak issuance is genuinely untouched
(parallel Poseidon `rZk`). The v1 and v2 contract remediations are **intact** (full table at the end) and
the new contracts do not weaken them; verify submission correctly routes through the hardened §11.6
confirm.

**However**, the canonical §11.8 code has **two Critical** problems — a **broken normal-path nullifier
soundness / cross-path double-attest** (the on-chain Poseidon domain almost certainly does *not* match
the circuit's, *and* the normal nullifier omits `recordType` so one consent is replayable across
record-types), and an **unbound ZK `kecOf` / `issuerForAny()` lookup** that can pass `isValid` against
the wrong issuer or be wired to read `kecOf[0]` — plus **three High** issues (the `purpose`↔`recordType`
collapse defeats the headline "verify-capability scoped per purpose, separate from issuer roles"
property; the ZK whitelist uses `bytes32(0)` so a *single* whitelist entry authorizes a relayer for
**every** ZK purpose; and the subject↔BabyJubjub-key binding is asserted but **never actually checked**
on-chain or proven against `keyOf[subject]`). Detail below with concrete fixes.

---

# CRITICAL

## V3-C1 — Normal-path on-chain Poseidon nullifier is unsound for the shared `consumed` set: (a) domain/parameter mismatch vs the circuit, and (b) `recordType` is absent from the nullifier
**Where:** `implementation.md §11.8(a)` normal path:
```solidity
bytes32 nf = bytes32(Poseidon.hash4(c.dogTagId, uint160(c.relayer), uint160(c.subject), c.nonce));
```
vs the circuit `§11.8(d)`:
```
nullifier == Poseidon4(dogTagId, relayer, subject, consentNonce)   // "SAME formula as the normal path"
```
and the shared-set claim (CHANGESPEC §0 / arch §4.7): *"one `consumed` set across both paths ⇒ one
consent = one attestation."*

**Why it matters — two independent breaks:**

1. **Parameter/domain mismatch → the shared set is bypassable (cross-path double-attest).** The whole
   point of a *single* `consumed[nf]` set is that the normal-path Poseidon output for a logical event
   equals the ZK circuit's Poseidon output for the same event, so a verification recorded on one path
   cannot be recorded again on the other. This holds **only if the Solidity `Poseidon.hash4` is
   bit-identical to circomlib's `Poseidon(4)`** — same field (BN254 `r`), same round constants, same MDS
   matrix, same number of full/partial rounds, **same input arity and the same domain/capacity tag**,
   and the **same input encoding** (see point 2). On-chain Poseidon libraries differ materially here:
   several popular Solidity Poseidon implementations are generated for a *fixed* arity, use a different
   capacity constant, or (critically) are *not* the circomlib parameter set. If the lib chosen for
   `Poseidon.hash4` is not the **exact** circomlib `poseidon.circom` parameterization the circuit uses,
   the two paths produce **different** `nf` for the same event → the same consent can be recorded **once
   per path** (a Critical violation of the stated "one consent = one attestation," and a double-count /
   double-bill primitive). research/11 §5.2 itself flagged "if perfect coincidence is impractical … the
   two paths produce *different* nullifiers" — the canonical design *requires* coincidence but does not
   pin the library or assert the parameters.
2. **`recordType` (purpose) is NOT in the normal-path nullifier → replay across record-types.** The
   nullifier is `hash4(dogTagId, relayer, subject, nonce)`. The signed consent and the event both carry
   `recordType`, but it is **excluded** from `nf`. Therefore a single signed `VerificationConsent`
   (fixed `dogTagId/relayer/subject/nonce`) yields **one** nullifier regardless of `recordType` — which
   is fine for "one consent = one attestation," **but** the same `(dogTagId, relayer, subject, nonce)`
   tuple reused under a *different* `recordType` collides on the **same** `nf` and is *blocked*, while a
   relayer who wants two *distinct* attestations (e.g. VACCINATION and DOG_PROFILE) for the same pet/visit
   must collect two consents with two nonces (acceptable). The real defect is the inverse, in concert
   with the §11.8 collapse of `purpose := recordType` (V3-H1): because the registry derives the
   VERIFY-whitelist key and the emitted `purpose` *from* `recordType`, but the nullifier ignores it, the
   nullifier no longer scopes "what was attested" — see also Q6 in the brief: yes, the same
   `(dogTagId, relayer, subject, nonce)` is reusable across purposes only by signing a fresh consent with
   a fresh nonce, and the nullifier cannot distinguish purposes. **The combination of V3-C1(2) + V3-H1 is
   the dangerous one: a consent the user signed believing it authorized purpose `GROOMING_INTAKE` is
   indistinguishable, at the nullifier and whitelist layer, from one for `AIRLINE_CHECKIN`.**

**EVM/crypto-accuracy note:** `Poseidon.hash4` is declared `internal view returns(uint256)` — a `view`
library call (Poseidon is pure arithmetic; some libs are deployed as a separate contract called via
`STATICCALL`, others are inlined). Either is fine for *gas/correctness*, but the *parameterization* is
the soundness issue, not the call mechanism. Also note the inputs `uint160(c.relayer)`/`uint160(c.subject)`
are passed as `uint256` field elements — these are `< 2^160 < r`, so they never overflow the BN254 field
(good), **but the circuit must encode the address identically** (as the low-160-bit field element, not
e.g. a 20-byte big-endian packed differently). See V3-C2 for the ZK-side address-encoding hazard.

**Fix:**
1. **Pin the Poseidon library to the circuit's exact circomlib parameter set** and assert it in CI:
   add a Foundry/integration test that computes `Poseidon.hash4(a,b,c,d)` on-chain and asserts it equals
   the circuit's `nullifier` output for the same inputs (run the prover, compare). Ship a `testvectors`
   entry (already mandated for `hashLeafZk`/`poseidonMerkle` in impl §1.4 — extend it to the nullifier).
   Use a vetted, circomlib-equivalent Solidity Poseidon (e.g. the iden3 `poseidon-solidity` generated
   from the same constants) and pin its address/bytecode hash.
2. **Define the nullifier domain explicitly and identically on both sides** — including arity tag and
   field. Document `nullifier = Poseidon([dogTagId, relayerField, subjectField, nonce])` with the precise
   capacity constant, and forbid any "hash4" that isn't that exact construction.
3. **Decide `recordType`/`purpose` in the nullifier deliberately.** If "one consent authorizes exactly
   one (pet, relayer, purpose)" is the intent (it is, per the consent UX), **include `recordType` in the
   nullifier on both paths** (`Poseidon5(dogTagId, relayer, subject, recordType, nonce)`), so a consent
   cannot be silently spent under a different purpose, and emit/whitelist on the *same* value. Then fix
   V3-H1 so `purpose` is a first-class signed field. Add a test that two consents differing only in
   `recordType` produce different nullifiers.
4. Until the library is pinned and CI-asserted equal to the circuit, treat the "shared `consumed` set"
   claim as **unverified**; do not ship the "one consent = one attestation across paths" guarantee.

---

## V3-C2 — ZK path `kecOf`/`isValid` is read from an **unbound** issuer (`issuerForAny()`), and `kecOf[rZk]==0`/`isValid(0)` handling is fragile → false "credential valid" / wrong-issuer pass
**Where:** `implementation.md §11.8(a)` ZK path:
```solidity
bytes32 rZk = bytes32(pub[4]);
bytes32 rKec = IDogTagIssuer(issuerForAny()).kecOf(rZk); require(rKec != bytes32(0), "unknown rZk");
require(IDogTagIssuer(issuerForAny()).isValid(rKec), "cred !valid");   // map rZk->rKec, REUSE isValid
// issuerForAny(): resolves the issuer clone holding kecOf[rZk] (recordType is private on the ZK path; ...)
```

**Why it matters:**
1. **`issuerForAny()` is undefined and unbindable from the proof.** On the ZK path `recordType` is
   **private** (folded into the circuit), so the registry cannot resolve `issuerFor[recordType]`. The
   code papers over this with `issuerForAny()` — "either a single protocol issuer or per-circuit
   recordType pinned in the verifying key." Both options are under-specified and dangerous:
   - *Single protocol issuer*: then `kecOf`/`isValid` are read from **one** clone, but `rZk→rKec` and the
     `issue`/`revoke` state for a VACCINATION credential live in the **VACCINATION clone**, not a single
     global one. A lookup against the wrong clone returns `kecOf[rZk]==0` for every legitimately-issued
     credential (DoS), or — if an attacker can get *any* `rZk→rKec` committed in that one clone and that
     `rKec` is `isValid` — passes the check for a credential the circuit never bound. The ZK proof binds
     `rZk` to the leaves and `dogTagId`, but the **registry** is what maps `rZk→rKec→isValid`; if that
     mapping is read from an issuer not bound to the proof, **the on-chain `isValid` re-check (the entire
     reason the circuit doesn't prove issuance) is decoupled from the credential the proof is about.**
   - *Per-circuit recordType pinned in the verifying key*: viable, but then there is **one circuit +
     verifier + `issuerFor` per recordType**, `issuerForAny()` must be `issuerFor[thatRecordType]`, and
     the contract must select the right verifier — none of which §11.8 specifies. As written it is a
     single `zkVerifier` and a single `issuerForAny()`.
2. **`kecOf` lookup returning 0 is checked, but `isValid(0)` and the binding to `dogTagId` are not
   tight.** `require(rKec != bytes32(0))` correctly rejects an unknown/uncommitted `rZk` (good — closes
   the "`kecOf` returns 0 → `isValid(0)`" hole the brief calls out; `isValid(bytes32(0))` would also be
   `false` since `issuedAt[0]==0` unless `issue(0)` is allowed — and audit-01 L-3 / impl §11.1
   `require(r!=bytes32(0))` in `issue` blocks anchoring the zero root, so `isValid(0)==false` is
   guaranteed — **confirm this guard is present**, it is the backstop). **But** nothing ties the resolved
   `rKec` back to `pub[0]=dogTagId`: the circuit proves `dogTagId` is the credential's dogTagId leaf and
   that the leaves hash to `rZk`, and the registry proves `kecOf[rZk]=rKec` and `isValid(rKec)` — so the
   chain `dogTagId → rZk → rKec → isValid` **does** hold *iff* `issuerForAny()` is the correct clone and
   `zkCommit` was honest (see V3-H2 on `zkCommit` originator binding). The weak link is purely
   `issuerForAny()`.

**Fix:**
1. **Make `recordType` (or `purpose`) recoverable on the ZK path so the issuer is bound to the proof.**
   Either (a) make `recordType` a **public signal** and resolve `issuerFor[recordType]` (drops the
   privacy of *which kind* of check — acceptable for many purposes; gate behind the purpose), or
   (b) ship **one circuit + one `Groth16Verifier` + one pinned `issuerFor` per recordType** and select
   the verifier/issuer by an explicit `recordType` argument the relayer passes *and* that is bound into
   the verifying key (so a wrong pairing fails `verifyProof`). Replace `issuerForAny()` with that
   explicit resolution. Never read `kecOf`/`isValid` from an issuer not bound to the verified proof.
2. Keep `require(rKec != bytes32(0))` **and** assert `isValid(0)` can never be true (CI test:
   `issue(0)` reverts, `isValid(0)==false`).
3. Add a Foundry test: a valid proof for a credential whose `rZk` was committed in clone A must **revert**
   if the registry resolves clone B; and a proof whose `rZk` was never `zkCommit`'d reverts `unknown rZk`.

---

# HIGH

## V3-H1 — `purpose` is collapsed to `recordType`, defeating the headline "verifier capability scoped per purpose, separate from issuer roles" property
**Where:** `implementation.md §11.8(a)` normal path: `bytes32 purpose = c.recordType;` then
`isWhitelistedFor(keccak256("VERIFY:"||purpose), msg.sender)` and `emit Verified(..., purpose, ...)`.
The `VerificationConsent` struct (CHANGESPEC §0, impl §1.10, arch §3.6) has **`recordType` but no
`purpose` field**. Yet CHANGESPEC §0/§4 + arch §4.3/§4.7 define `purpose = keccak256(label)` (e.g.
`GROOMING_INTAKE`, `AIRLINE_CHECKIN`, `VET_INTAKE`) as a **distinct** concept, the `/verify/session/start`
JWT carries `purpose` **and** `recordType` as **separate** fields (impl §11.8(g), §3.9), and the
`VERIFY:` namespace is explicitly *"keyed by `purpose`"*, separate from issuer roles.

**Why it matters:** The stated design grants verify-capability at the **purpose** granularity
(`VERIFY:GROOMING_INTAKE`) so that *"a groomer can be authorized to verify a given `purpose` without
holding any issuer role"* (arch §4.3). Collapsing `purpose := recordType` means:
1. The whitelist key becomes `keccak256("VERIFY:" || keccak256("VACCINATION"))`, i.e. capability is
   granted **per record-type, not per purpose**. A groomer whitelisted to *check vaccination status at
   grooming intake* is, on-chain, indistinguishable from one whitelisted for *airline check-in of a
   vaccination cert* — the two purposes the spec wants to separate share one whitelist entry and one
   emitted `purpose`. The `purpose` taxonomy (`GROOMING_INTAKE` vs `AIRLINE_CHECKIN`) **never reaches the
   chain**; the `Verified` event's `purpose` field is actually a `recordType`, so the off-chain
   `verification_records` mirror (§9) and any auditor reading the event get the wrong semantic.
2. **The user's consent is mis-scoped.** The owner signs `recordType` (what *kind* of credential), not
   `purpose` (*why* it is being checked). The session JWT names a `purpose`, but it is **not in the
   signed struct**, so the contract cannot bind the consent to the purpose the user approved. A relayer
   whitelisted for `VERIFY:VACCINATION` can spend a consent the user tapped "approve" for a grooming
   visit to record an *airline check-in* (or vice-versa) — the on-chain record will read whatever the
   relayer chose, and the user signature does not constrain it.

**Fix:** Add **`bytes32 purpose`** to the `VerificationConsent` struct (and the EIP-712 typehash — this
is a breaking typehash change, do it before any signature is ever collected). Sign it. In
`recordVerification`, use `c.purpose` (not `c.recordType`) for the `VERIFY:` whitelist key and the emitted
event; keep `c.recordType` for the `issuerFor`/`isValid` resolution. On the ZK path, make `purpose` a
public signal (or per-circuit pinned) so the whitelist key is purpose-specific (fixes V3-H3 too). Update
`hashTypedConsent` in both SDKs and the `testvectors.json` in lockstep. If `purpose` must stay private on
ZK, fold it into the circuit and expose a per-purpose verifying key.

## V3-H2 — ZK whitelist uses `keccak256("VERIFY:" || bytes32(0))` → one whitelist entry authorizes a relayer for **every** ZK purpose (and is shared by all ZK relayers)
**Where:** `implementation.md §11.8(a)` ZK path:
```solidity
require(issuerRegistry.isWhitelistedFor(keccak256(abi.encodePacked("VERIFY:", bytes32(0))), msg.sender)); // purpose private in ZK
```

**Why it matters:** Because `purpose`/`recordType` is private on the ZK path, the code whitelists against
the **constant** key `keccak256("VERIFY:" || 0x0)`. Consequences:
1. **No purpose scoping at all on the ZK path.** A relayer granted `whitelistFor(keccak256("VERIFY:"||0),
   relayer)` can record ZK verifications for **any** purpose/record-type — the privacy-maximal *default*
   path (CHANGESPEC §5: "ZK is the default for sensitive purposes") is precisely the path with the
   **weakest** capability scoping. A groomer authorized only for grooming intake can record ZK
   attestations the spec intends to reserve for airlines/vets.
2. **The whitelist namespace degenerates to a single global "is-a-verifier" boolean** on the ZK path —
   re-introducing exactly the kind of un-scoped global flag that v1 **C-2** eliminated for issuers. This
   is a regression-in-spirit of the C-2 remediation, applied to the verify namespace.

**Fix:** Tie to V3-H1/V3-C2: make `purpose` recoverable on the ZK path (public signal or per-circuit
verifying key) and whitelist against `keccak256("VERIFY:" || purpose)`. If a single all-purpose ZK
verifier role is *intended* for v1, name it explicitly (`keccak256("VERIFY:ZK_ANY")`), document that it
is deliberately coarse, and add an admin note — but do not silently key on `bytes32(0)`.

## V3-H3 — Subject↔BabyJubjub-key binding is claimed but **never enforced** on-chain or against `keyOf[subject]`; `ConsentKeyRegistry.keyOf` is unused by the registry
**Where:** `implementation.md §11.8(a)` ZK path comment:
`// subject<->BabyJubjub-key linkage already proven in-circuit & bound via ConsentKeyRegistry (see (d))`
— but the function body **never reads `consentKeys.keyOf(...)`**. The `IConsentKeyReg` interface and the
`consentKeys` immutable are declared and wired in the constructor, yet `recordVerificationZK` makes **no
call** to it. The circuit `§11.8(d)` says it *"+ expose Poseidon(Ax,Ay) for the on-chain
ConsentKeyRegistry.keyOf[subject] check (subject<->key)"* — i.e. the binding is supposed to be a
**registry-side check**, but that check is absent.

**Why it matters:** The security argument for the ZK path is: the consenter proved (in EdDSA, in-circuit)
that the BabyJubjub key signed `(dogTagId, relayer, rZk, nonce)`, **and** that BabyJubjub key is the one
the user bound to their secp256k1 `subject` wallet via `ConsentKeyRegistry`. If the registry never checks
`Poseidon(Ax,Ay) == keyOf[subject]`, then the circuit proves only *"some BabyJubjub key signed,"* not
*"the subject's bound key signed."* Combined with the fact that `subject` (`pub[2]`) is a free public
input the relayer chooses, **a relayer can supply any `subject` address and a self-controlled BabyJubjub
key** (which it makes sign the consent message), produce a valid proof, and record a verification
attributing consent to a `subject` who never consented — *unless* either (a) the circuit constrains
`Poseidon(Ax,Ay) == keyOf[subject]` using `keyOf` as a **public input** the registry also verifies, or
(b) the registry calls `consentKeys.keyOf(subject)` and the circuit exposes `Poseidon(Ax,Ay)` as a public
signal the registry compares. Neither is in the public-signal list `[dogTagId, relayer, subject,
nullifier, rZk]` (no `keyHash` signal) and neither call is in the contract. As written, the
**subject-impersonation guard is missing** on the ZK path. (The normal path is fine: `ECDSA.recover ==
c.subject` + `sbt.ownerOf == subject` are both enforced.)

Note also the ZK path **does not check `sbt.ownerOf(dogTagId) == subject`** (the normal path does). The
design (research/11 §6.2) intends ownership to be proven in-circuit for ZK — but the circuit
`§11.8(d)` proof list (a)-(d) does **not** include an `ownerOf` constraint, and the registry omits the
on-chain `ownerOf` call. So neither layer binds `subject` to the SBT owner on the ZK path.

**Fix:**
1. **Add `keyHash` (= `Poseidon(Ax,Ay)`) as a public signal** and in `recordVerificationZK` require
   `bytes32(keyHash) == consentKeys.keyOf(address(uint160(pub[2])))`. This binds the in-circuit consent
   key to the `subject`'s registered key and makes `subject` un-spoofable. (Or pass `keyOf[subject]` in as
   a public input the circuit constrains equal to `Poseidon(Ax,Ay)`; same effect, registry must still
   read `keyOf` to supply/verify it.)
2. **Bind `subject` to SBT ownership on the ZK path** — either add an in-circuit constraint proving
   `ownerOf(dogTagId)==subject` against an on-chain-anchored owner (hard without an owner Merkle proof),
   or simply call `require(sbt.ownerOf(pub[0]) == address(uint160(pub[2])))` in the registry (cheap
   STATICCALL, parallels the normal path; only mild privacy cost since `subject` is already emitted). The
   latter is the pragmatic v1 fix.
3. Remove the misleading "already proven … bound via ConsentKeyRegistry" comment until the call exists;
   add a Foundry test where a relayer-chosen `subject` with an unregistered/mismatched key **reverts**.

---

# MEDIUM

## V3-M1 — `recordVerification` (normal path) has no per-subject nonce / deadline-window beyond the consent's own `deadline`; relies solely on the Poseidon nullifier for replay
**Where:** `implementation.md §11.8(a)` normal path. The §11.8 canonical body **dropped** the
`mapping(address=>uint256) nonces` and the `require(c.nonce == nonces[c.subject])` /
`nonces[c.subject]=c.nonce+1` that research/11 §2.1/§5.1 had. Replay protection is now **only**
`consumed[nf]` + `block.timestamp <= c.deadline`.

**Why it matters:** This is arguably *cleaner* (a monotonic per-subject nonce forces strict ordering and
breaks if the user signs two consents out of order — a real UX footgun), and the nullifier does provide
single-use. **But:** (a) `nonce` is now a free-form salt the *client* picks; if two different consents
accidentally reuse the same `(dogTagId, relayer, subject, nonce)` they silently collide on `nf` and the
second is rejected as "replayed" (confusing, not exploitable). (b) There is **no lower bound** on
`deadline` — a relayer can hold a consent with a far-future `deadline` and submit it much later; the spec
intends "5-15 min" (research/11 §1.2) but the contract enforces only the upper bound. A long-lived
consent widens the window for the V3-H1/H3 mis-scoping abuses. (c) Removing `nonces` is an **undocumented
divergence** from research/11 — note it deliberately.

**Fix:** Document the nonce as a client-chosen unique salt (not monotonic) and the nullifier as the
single-use guard. Optionally enforce a max consent lifetime (`require(c.deadline <= block.timestamp +
MAX_CONSENT_TTL)`), or rely on the off-chain session JWT's 180s `exp` (impl §11.8(g)) plus the
backend's `consent.deadline >= now` check — and state that on-chain only bounds the *upper* edge.

## V3-M2 — `zkCommit` originator/overwrite: cannot overwrite an existing `kecOf[rZk]` (good), but **any** whitelisted issuer for that recordType can be the committer; `rKec` need not be the committer's own issuance
**Where:** `implementation.md §2.2`:
```solidity
function zkCommit(bytes32 rKec, bytes32 rZk) external onlyWhitelisted {
    require(issuedAt[rKec]!=0 && rZk!=bytes32(0) && kecOf[rZk]==bytes32(0),"bad");
    kecOf[rZk]=rKec; emit ZkCommitment(rKec,rZk);
}
```
`onlyWhitelisted` = `registry.isWhitelistedFor(recordType, msg.sender)`.

**Why it matters:** Access control is **issuer-only** (correct — answers Q4: not the SBT, not the
registry, gated by the same per-recordType whitelist as `issue`), and `kecOf[rZk]==bytes32(0)` makes the
mapping **append-only / non-overwritable** (correct — answers Q4 "can it overwrite an existing
`kecOf[rZk]`": **no**, a second `zkCommit` for an already-mapped `rZk` reverts `"bad"`). It also requires
`issuedAt[rKec]!=0` (the keccak root must already be issued) and `rZk!=0`. This does **not** weaken the
audited issuer contract: `issue`/`revoke`/`isValid`/`issuedBy` are untouched, and `zkCommit` adds only a
forward map.

**Residual (Medium):** `zkCommit` checks `issuedAt[rKec]!=0` but **not** `issuedBy[rKec]==msg.sender` —
so any address whitelisted for that recordType (post-C-2 there can be several per entity, and several
entities per recordType) can commit an `rZk→rKec` mapping for a credential issued by a *different*
whitelisted issuer. Because `rZk` is attacker-chosen and append-only, a malicious co-whitelisted issuer
could **front-run/squat** the *honest* `rZk` for someone else's `rKec` with a **wrong** `rZk'` mapping —
no, it can't map a *wrong* `rZk` to a valid `rKec` and gain anything (the ZK proof binds the *real* `rZk`
to the leaves; a bogus `rZk'` has no valid proof). The real risk is the inverse: an issuer maps the
**honest** `rZk` (which it can compute, since `wrapDocument` is deterministic given the doc, but the doc
is off-chain) — generally it does **not** know the honest `rZk` without the salts, so squatting is
impractical. **Net: low-exploitability, but the missing originator check is inconsistent with H-1's
philosophy** and could let a co-issuer occupy an `rZk` slot to *deny* the legitimate `zkCommit` (DoS:
the legitimate commit then reverts `kecOf[rZk]!=0`).

**Fix:** Add `require(issuedBy[rKec]==msg.sender || registry.hasRole(0x00,msg.sender), "!owner")` to
`zkCommit` (mirror the H-1 revoke guard) so only the originator (or admin) can bind `rZk` for their own
`rKec`. Add a test that a co-whitelisted-but-non-originator `zkCommit` reverts, and that a duplicate
`zkCommit(rZk)` reverts.

## V3-M3 — `ConsentKeyRegistry.bindConsentKey` is one-time and irrevocable → lost/rotated BabyJubjub consent key permanently disables the user's ZK path
**Where:** `implementation.md §11.8(b)`:
```solidity
require(keyOf[msg.sender] == bytes32(0), "already bound");   // one-time
```
No rebind, no rotation, no admin override.

**Why it matters:** Binding integrity is **good** (Q3): `ecrecover(EIP-712 BindConsentKey digest) ==
msg.sender` proves the secp256k1 wallet authorizes that BabyJubjub key, the EIP-712 domain pins
`chainId:135` + `verifyingContract` (so no cross-chain/cross-contract replay), and `keyOf[wallet]` is
keyed by `msg.sender` (the binder binds **their own** wallet — nobody can bind on another's behalf). But
**one-time + irrevocable** mirrors the v2 **V2-H2** lost-key problem: if the user rotates devices, loses
the BabyJubjub key, or the derived consent key changes (re-derivation domain mismatch), they can **never
re-bind**, permanently losing ZK-path consent for that wallet. Because the SBT can be `recover()`'d to a
new owner address (a *different* wallet) but `ConsentKeyRegistry` keys on the **old** wallet, after a
`recover()` the new owner address has **no** bound consent key and (per the one-time guard) the *old*
address's binding is stranded — the ZK path is unusable for the recovered pet until the new owner binds
*their own* key (which they can, since `keyOf[newOwner]==0` — so this part self-heals), **but** any
consent referencing the old `subject` is dead.

**Fix:** Allow **rebind** gated by a fresh EIP-712 signature from `msg.sender` (the wallet always controls
its own binding) — drop the `== bytes32(0)` one-time guard or add a separate
`rebindConsentKey(newHash, sig)` that overwrites `keyOf[msg.sender]` after `ecrecover==msg.sender`. Bind a
nonce/deadline into the `BindConsentKey` struct to prevent signature replay across rebinds. Document that
`subject` in ZK consent is the **current** per-pet wallet and must have a current binding.

## V3-M4 — Normal-path gas: on-chain Poseidon hash per attestation is non-trivial and unbounded by the spec; ZK-path event/`purpose=0` indexing
**Where:** `implementation.md §11.8(a)` normal path `Poseidon.hash4(...)`; CHANGESPEC §7 flag #2
("On-chain Poseidon … small gas cost; audit to confirm lib choice").

**Why it matters (Q5 gas):** A circomlib-equivalent Poseidon(4) over BN254 on the EVM is **expensive** —
a Solidity Poseidon `hash4` is typically tens of thousands of gas (the round constants + MDS multiplies
in `uint256` mod `r`; widely reported ~20k-60k+ gas depending on arity/implementation), on top of the
`ECDSA.recover` (~3k), two STATICCALLs (`ownerOf`, `isValid`), the `isWhitelistedFor` STATICCALL, a
`consumed` SSTORE (~20k cold), and the event. This is a meaningful per-attestation cost on the normal
path. It is **bounded** (one hash, fixed arity) so not a DoS, but the "small gas cost" framing
understates it; if a separate Poseidon **contract** is used via STATICCALL, add ~2.6k cold-account
access. The ZK path's `verifyProof` is ~211k (BN254 pairing precompiles) + the same `kecOf`/`isValid`
reads.

**Fix:** Benchmark `Poseidon.hash4` on ROAX (gas + that the EVM version supports the precompiles/opcodes
the lib uses — `evm_version=paris`, no PUSH0; some Poseidon libs assume `MCOPY`/`PUSH0` — **verify the
chosen lib compiles under `paris`**, audit-01 M-4 / arch §13.1). Document the per-attestation gas for both
paths so relayers fund correctly. Confirm ROAX has the `ecAdd/ecMul/ecPairing` precompiles (BN254) the
`Groth16Verifier` needs — **if ROAX lacks them, the ZK path does not work at all** (deploy-time
pre-check, like the §architecture header RPC liveness note).

## V3-M5 — `zkVerifier` is admin-swappable (`setZkVerifier`) with the comment "timelocked" but **no timelock is implemented**; an admin can swap in a malicious verifier
**Where:** `implementation.md §11.8(a)`:
```solidity
IGroth16Verifier public zkVerifier;   // admin-swappable (timelocked) if the circuit is upgraded
function setZkVerifier(address v) external onlyRole(DEFAULT_ADMIN_ROLE){ zkVerifier=IGroth16Verifier(v); } // timelocked
```

**Why it matters:** The comment says "timelocked" but the function is a plain `onlyRole(DEFAULT_ADMIN_ROLE)`
setter with **no delay**. A compromised/rogue admin can instantly point `zkVerifier` at a contract whose
`verifyProof` returns `true` for any input → **forge arbitrary ZK attestations** (subject to the
`kecOf`/`isValid` re-check, which a forged proof can satisfy by choosing a real, issued `rZk`). The same
admin already controls `setIssuerFor` (could point `issuerFor`/`issuerForAny` at an attacker issuer whose
`isValid` returns true) and `setRelayerRestriction`. `AccessControlDefaultAdminRules` gives a **two-step
+ delayed admin transfer** (good, H-3 intact) but does **not** delay ordinary role-gated calls like
`setZkVerifier`. So the verifier swap is as fast as one admin tx.

**Fix:** Implement an actual timelock for `setZkVerifier` (and arguably `setIssuerFor`): either route
admin config through an OZ `TimelockController` that holds `DEFAULT_ADMIN_ROLE`, or add a
propose/commit-with-delay pattern in the contract. At minimum, make `DEFAULT_ADMIN_ROLE` the multisig
(it is, via `AccessControlDefaultAdminRules(2 days, admin)` — note the **2-day** delay here vs
`IssuerRegistry`'s **3-day**; reconcile) and document the verifier-swap as a privileged, monitored action
with an on-chain event. Emit `ZkVerifierChanged(old,new)`.

---

# LOW

## V3-L1 — EIP-712 domain `name/version` collision across `ConsentKeyRegistry` and `VerificationRegistry` (both `"DogTag","1"`); separated only by `verifyingContract`
Both deploy `EIP712("DogTag","1")` (§11.8(a)/(b)), and `DogTagSBT` also uses `EIP712` with a `Claim`
typehash (§11.7(a)). Separation relies entirely on **`verifyingContract`** (each computes a distinct
`domainSeparator`) and on distinct typehashes (`VerificationConsent` vs `BindConsentKey` vs `Claim`).
This is sound (research/11 §1.1 calls it out as intentional defense-in-depth), but the `BindConsentKey`
struct **omits a nonce/deadline** — fine while binding is one-time, but if V3-M3's rebind is added, a
nonce/deadline becomes mandatory to prevent rebind-signature replay. Flagging for the rebind fix.

## V3-L2 — `Verified` event emits `purpose=bytes32(0)` on the ZK path, conflating "ZK path" with "unknown purpose"
`emit Verified(..., bytes32(0), nf, ...)` on ZK. Indexers cannot tell ZK-with-real-private-purpose from a
malformed event, and the off-chain `verification_records` mirror loses `purpose` entirely for ZK. Once
V3-H1/H3 make `purpose` recoverable, emit it (or a commitment to it) and/or add a `bool zk` field (the
research/11 §2.1 event had `bool zk`; the canonical §11.8 event dropped it). Recommend restoring a `zk`
flag or a dedicated `VerifiedZK` event for unambiguous indexing.

## V3-L3 — `recordVerification` does not check SBT `status` (Deceased/Revoked) — a verification can be recorded against a terminal-status pet
The normal path checks `sbt.ownerOf(dogTagId)==subject` and `isValid(credentialRoot)`, but not
`sbt.status(dogTagId)`. A `Deceased`/`Revoked` pet (impl §11.7(a)) still has an owner and may still have a
valid (un-revoked) credential root, so a verifier could record an attestation for a deceased pet. Low
impact (the credential is genuinely valid; status is about the pet, not the cred), but if business logic
treats a recorded `Verified` as "this pet is currently presentable," add an optional
`require(sbt.status(dogTagId)==Active)` for purposes that need it. (The `IDogTagSBT` interface in §11.8
even dropped the `status()` accessor that research/11 §2.1 had — restore it if you adopt this.)

---

# INFO / CONFIRMATIONS

- **I-1 — Relayer binding is sound on both paths (Q1/Q2 relayer-forge/replay/non-relayer).** Normal:
  `require(msg.sender == c.relayer)` + `relayer` is inside the signed struct, so a different relayer's
  submission reverts and they cannot edit `relayer` without invalidating `userSig`. ZK: `relayer` is a
  public signal `pub[1]` bound `== msg.sender`, committed by the proof. **A non-relayer cannot call;
  an eavesdropper/forwarder cannot spoof `msg.sender`** (no EIP-2771/4337 — correct choice). Replay of a
  captured consent by the bound relayer is stopped by `consumed[nf]` (modulo V3-C1's soundness caveat).
- **I-2 — Signature malleability (Q1).** Normal path uses **OZ `ECDSA.recover`**, which rejects
  high-`s` signatures and `v∉{27,28}` (EIP-2 enforced since OZ ≥4.7), so classic secp256k1 malleability
  is not a replay vector here (and the nullifier doesn't depend on the sig bytes anyway). ZK Groth16
  proof malleability is correctly neutralized by putting the nullifier in the **public signals**
  (§11.8(d), snarkjs #383) — a malleated `(a,b,c)` yields the same `nf`, still blocked by `consumed`.
  **Confirmed handled.**
- **I-3 — Missing-domain-fields / deadline (Q1).** EIP-712 domain includes `name, version, chainId(135),
  verifyingContract(VerificationRegistry)` — all four present (research/11 §1.1, impl §11.8). The
  typehash field order matches the SDK (`§1.10`) exactly. `deadline` is checked (`block.timestamp <=
  c.deadline`). **No missing-field gap** (lower-bound on deadline is V3-M1).
- **I-4 — Range-check ALL public signals (Q2).** `for (i<5) require(pub[i] < SNARK_SCALAR_FIELD)` is
  present and covers **all five** signals incl. `nullifier(pub[3])` and `rZk(pub[4])` before use
  (snarkjs #358). **Confirmed.** The address packing `uint160(pub[1])==uint160(msg.sender)` is correct
  (truncating compare; `msg.sender` is 160-bit) — but see V3-C1(1)/V3-C2 that the *circuit* must encode
  `relayer`/`subject` as the identical low-160-bit field element for the nullifier and the relayer check
  to agree. No `uint160 vs field` confusion in the contract itself; the hazard is circuit↔contract
  encoding agreement (covered in C-1/C-2 fixes).
- **I-5 — Reentrancy.** `recordVerification`/`recordVerificationZK` make only **STATICCALL**s
  (`ownerOf`, `isValid`, `kecOf`, `isWhitelistedFor`, `verifyProof`, `Poseidon` view) — all read-only,
  no external call to an attacker-controllable contract that could reenter, and state (`consumed[nf]`) is
  set before the event. **No reentrancy.** (One nit: in the ZK path `consumed[nf]=true` is set *after*
  `verifyProof` and the `kecOf`/`isValid` reads — all views — so even a hypothetical reentrant view is
  harmless; the normal path sets `consumed` after all checks too. Setting `consumed` is checked-then-set
  with no interleaved mutating external call. Fine.)
- **I-6 — `VERIFY:` whitelist toggle bypass (Q1).** `restrictToWhitelistedRelayers` is `DEFAULT_ADMIN`-
  gated and **defaults to `true`** (CHANGESPEC §2, impl §11.8). When `true`, the whitelist check runs on
  both paths; when an admin sets it `false`, **only** the relayer-binding gates remain (any consent-bound
  relayer can record) — that is the documented "open ecosystem" mode (research/11 §2.2). No *bypass* of
  the toggle by a caller; the only weakness is the *scoping* of the key it checks (V3-H1/H2), not the
  toggle itself.
- **I-7 — keccak issuance untouched (Q4).** `issue/revoke/isValid/issuedAt/revokedAt/issuedBy` are
  byte-for-byte the §11.1 (C-1/H-1) bodies; `zkCommit`/`kecOf`/`ZkCommitment` are additive and read-only
  w.r.t. issuance state. The §3 keccak canonicalization and `rKec` are unchanged. **The audited issuer is
  not weakened** (modulo V3-M2's originator nit on the *new* `zkCommit`).

---

# Regression review — v1 & v2 remediations after v3

| Finding | Status in v3 | Evidence |
|---|---|---|
| **v1 C-1** `_disableInitializers()` on `DogTagIssuer` impl | **INTACT** | §11.1 `constructor(){ _disableInitializers(); }` unchanged; `zkCommit` is added to the same clone, no new constructor/init surface. |
| **v1 C-2** per-recordType + dedicated `PROFILE_ISSUER_ROLE` scoping | **INTACT** | §11.1 `isWhitelistedFor(rt,s)` unchanged; `zkCommit` reuses `onlyWhitelisted`(rt). **Caveat:** the *verify* namespace (`VERIFY:`) re-introduces an un-scoped global key on the ZK path (V3-H2) — a regression-in-spirit for the new namespace, not an undo of C-2 for issuers. |
| **v1 H-1** originator binding (`issuedBy`, revoke/profile) | **INTACT** | §11.1 `issuedBy`/revoke guard unchanged. **Caveat:** new `zkCommit` lacks the analogous originator check (V3-M2). |
| **v1 H-2** admin-only burn | **INTACT** | §11.7(a) `burn` is `onlyRole(DEFAULT_ADMIN_ROLE)`; verification leg never burns. |
| **v1 H-3** admin hardening (`AccessControlDefaultAdminRules`, two-step+delay, multisig, duty split) | **INTACT** | `IssuerRegistry`(3 days)+`WHITELIST_ADMIN`; `VerificationRegistry`(2 days). **Nit:** delay mismatch (2 vs 3 days) + `setZkVerifier`/`setIssuerFor` are *not* timelocked despite the comment (V3-M5). |
| **v1 M-1** permissioned factory + deterministic salt | **INTACT** | §11.1 `createIssuer onlyRole(ADMIN)`, `salt=keccak256(recordType,business)` unchanged. |
| **v1 M-4** `evm_version=paris` + N-confirmation reads | **INTACT, with new gate** | §11.8 pins `pragma 0.8.24 // evm_version=paris`. **NEW risk:** the Poseidon lib + Groth16 verifier must compile under `paris` (no PUSH0/MCOPY) and ROAX must have BN254 precompiles (V3-M4) — a *new* deploy-time gate, not a regression. |
| **v2 V2-C1/M3/H3** hardened `confirm` (signer from tx, calldata/to/value/chainId bind, emitting-contract pin, finality, idempotency) | **INTACT & REUSED** | Verify submission routes through the **same §11.6 prepare/confirm** (CHANGESPEC §3, impl §11.8(g) `submitViaPrepareConfirm`). The §11.6 confirm derives `signer` from `tx.from`, binds `tx.to/input/value:0/chainId:135`, pins the emitting contract, waits N confirmations, and is idempotent on `txHash`. **Confirmed the verify tx goes through the hardened confirm.** (For verify, `tx.to` is `VerificationRegistry` and the bound calldata is `recordVerification(...)`/`recordVerificationZK(...)` — the §11.6 generic checks apply; ensure the `RootIssued`-specific event assertion is generalized to `Verified` for the verify flow — see note below.) |
| **v2 V2-H2** lost-key ownership-pillar / recovery | **INTACT (ownership contextual)** | §11.3 ownership is contextual (3 pillars gate validity). **NEW analogous risk:** `ConsentKeyRegistry` one-time bind has the *same* lost-key shape for the ZK consent key (V3-M3). |

**Net:** No v3 edit **undoes** a v1/v2 remediation in the canonical artifacts. Two *new-surface*
regressions-in-spirit: (a) the `VERIFY:` namespace re-introduces an un-scoped global key on the ZK path
(V3-H2), echoing the pre-C-2 global flag; (b) the new `zkCommit` lacks H-1-style originator binding
(V3-M2). The hardened §11.6 confirm is correctly reused for verify submission.

> **Confirm-path note for the verify flow:** §11.6's confirm pins the **`RootIssued`** event +
> `issuedAt[root]!=0` read. For a `recordVerification*` tx there is no `RootIssued`; the analogous
> re-verification is the **`Verified`** event (pin `log.address==VerificationRegistry`,
> `log.transactionHash==txHash`) + a `consumed[nf]==true` read at N confirmations. Generalize §11.6's
> event/state assertion per submission type so the verify confirm is as tight as the issue confirm
> (otherwise the verify confirm degrades to receipt-status-only — a V2-C1-class gap on the new path).

---

# Recommended Foundry / integration tests (v3)
- **V3-C1:** on-chain `Poseidon.hash4(a,b,c,d)` == circuit `nullifier` output for identical inputs
  (run prover, compare); two consents differing only in `recordType`/`purpose` produce different
  nullifiers (after the fix); a normal-path attestation and a ZK attestation for the *same* logical event
  share one `nf` and the second reverts `replayed`.
- **V3-C2:** ZK proof for an `rZk` committed in clone A reverts if the registry resolves clone B; an
  `rZk` never `zkCommit`'d reverts `unknown rZk`; `isValid(0)==false` and `issue(0)` reverts.
- **V3-H1:** consent signed for `purpose=GROOMING_INTAKE` cannot be recorded under `AIRLINE_CHECKIN`
  (whitelist key + emitted purpose are purpose-specific after adding the `purpose` field).
- **V3-H2:** a relayer whitelisted only for purpose X cannot record a ZK verification for purpose Y.
- **V3-H3:** a relayer-chosen `subject` whose `keyOf[subject]` is unset or != `Poseidon(Ax,Ay)` reverts;
  ZK path reverts when `sbt.ownerOf(dogTagId) != subject` (after adding the owner/keyHash checks).
- **V3-M2:** non-originator (co-whitelisted) `zkCommit` reverts; duplicate `zkCommit(rZk)` reverts.
- **V3-M3:** rebind of a consent key by the wallet succeeds (after the fix) with a fresh nonce; replayed
  bind signature reverts.
- **V3-M5:** `setZkVerifier` is timelocked/multisig-gated; emits an event.
- **Regression:** all audit-01 (C-1/C-2/H-1/H-2/H-3/M-1) and audit-04 (V2-C1/H1/H3 hardened-confirm)
  Foundry tests still pass; verify submission uses §11.6 confirm with the `Verified`-event assertion.
- **Deploy pre-check:** ROAX exposes BN254 `ecAdd/ecMul/ecPairing` precompiles; Poseidon lib + verifier
  compile and run under `evm_version=paris`.

---

# Verdict

**Not deployment-ready.** The v3 verification leg has two Criticals — an unsound shared nullifier
(unpinned on-chain Poseidon vs the circuit's Poseidon, plus `recordType`/`purpose` absent from it →
cross-path / cross-purpose double-attest, V3-C1) and an unbound ZK `kecOf`/`isValid` resolution
(`issuerForAny()`, V3-C2) — plus three Highs that collectively gut the "verify-capability scoped per
purpose, separate from issuer roles" property (V3-H1/H2) and leave the ZK subject impersonable because
the `ConsentKeyRegistry` binding is declared but never checked (V3-H3). Fix V3-C1/C2/H1/H2/H3 (and wire
the generalized `Verified`-event assertion into the §11.6 confirm) before any ROAX deploy. The relayer
binding, signature/proof-malleability handling, range-checks, reentrancy posture, and the EIP-712 domain
are sound; keccak issuance is genuinely untouched; and **all v1/v2 remediations are intact** — the v3
contracts add new surface that needs hardening, they do not reverse prior fixes.
