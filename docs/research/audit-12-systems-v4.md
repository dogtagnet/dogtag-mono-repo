# Audit 12 — Systems + Cross-Doc Consistency (v4 Poseidon unification)

> Scope: consistency + completeness of the v4 Poseidon-unification pass across `architecture.md`,
> `implementation.md`, `BUILD_PROMPT.md` (with `research/CHANGESPEC-v4.md` as the normative source).
> NOT in scope: Poseidon math, circuit/contract internals (covered by audits 10/11; flagged where a
> systems-level coherence gap exists). Date: 2026-06-17.

## Verdict

**PASS with fixes.** The dual-root deletion landed cleanly — every `rKec`/`rZk`/`zkCommit`/`kecOf`/
`zkIndex`/`cloneOf`/`issuerForAny`/`hashLeafZk`/`0x02`-leaf reference in the three docs is inside a
DELETED / SUPERSEDED / RESOLVED-by-unification callout (no dangling active spec). Single-root coherence,
the 7-signal tuple, hash policy, and the 4-language determinism principle are consistent. Remaining
issues are **(1) a stale doc-inventory line in BUILD_PROMPT** (Medium), **(2) two BUILD_PROMPT/impl
privacy lines that dropped the v4 hash-agnostic `nor Poseidon(microchip)` qualifier** (Medium), and
**(3) the `purposeToRecordType` ZK clone-resolution exists only in impl §11.9 and is missing/elided in
architecture §4.7 + §13.8** (High — cross-doc coherence gap; also a systems flag for the contracts
auditor).

---

## Findings by severity

### HIGH

**H-1 — Issuer-clone resolution (`purposeToRecordType`) is impl-only; arch §4.7 elides it with a `...`
placeholder.** (Audit point 4.)
- `recordType` is **not** a ZK public signal (impl §11.9(d)/(e)). The ZK path must therefore map the
  public `purpose` → `recordType` → `issuerFor[recordType]` clone to run `isValid(R)`. impl §11.9(e)
  line 1667 does this via `issuerFor[purposeToRecordType(bytes32(pub[1]))]` and defines
  `purposeToRecordType` as an admin-set `mapping(bytes32 purpose => bytes32 recordType)` (impl §11.9
  note, line 1673).
- **architecture.md does not contain the string `purposeToRecordType` anywhere.** arch §4.7 ZK body
  (line 445) writes `require(DogTagIssuer(issuerFor(...)).isValid(bytes32(pub[6])))` — a literal `...`
  placeholder that does not show how `recordType` is recovered from the public `purpose`. A reader of
  arch alone sees `issuerFor(c.recordType)` for the normal path (line 429) but an undefined argument for
  ZK, and could wrongly infer `recordType` is available on the ZK path.
- arch §13.8 (line 747) says only "`issuerFor(recordType)` resolution remains for the normal path" — it
  does **not** state how the ZK path resolves the clone, leaving the impression the ZK clone-resolution
  is unspecified (the very gap audit-08 C-2 was about).
- **Coherence question for the contracts auditor:** does `purposeToRecordType` actually resolve the
  *unique clone that issued R*? It resolves `purpose → recordType → issuerFor[recordType]`, i.e. the
  *canonical* clone for that record type. If multiple business clones exist per record type (arch §4.4:
  "one clone per record type **and per business**"; impl §11.8 `issuerFor` is `recordType => single
  address`), then `issuerFor[recordType]` is a single admin-pinned clone, not necessarily the clone that
  anchored this particular `R`. `isValid(R)` on the wrong clone returns false (or, if `R` was issued on a
  different business's clone, the verification reverts). **Flag for contracts auditor:** confirm whether
  `issuerFor` is intended as one-clone-per-recordType (protocol-wide) and that per-business clones do not
  break ZK `isValid(R)` resolution. This is a systems-level coherence gap regardless of the answer,
  because the two docs describe the resolution at different levels of completeness.
- **Fix:**
  1. In arch §4.7 ZK body, replace `issuerFor(...)` with the explicit resolution and add the mapping to
     the prose, e.g.:
     `require(DogTagIssuer(issuerFor[purposeToRecordType(bytes32(pub[1]))]).isValid(bytes32(pub[6])));  // purpose→recordType→clone`
     and add one sentence: "`purposeToRecordType` is an admin-set `mapping(bytes32 purpose => bytes32
     recordType)` (recordType is not a ZK public signal); it replaces the deleted `zkIndex`/`issuerForAny`
     clone lookup."
  2. In arch §13.8 line 747, broaden the parenthetical to: "(`issuerFor[recordType]` resolution remains;
     the ZK path resolves `recordType` from the public `purpose` via the admin-set `purposeToRecordType`
     map — see impl §11.9(e))."
  3. Confirm the one-clone-per-recordType assumption holds (contracts auditor) or document how
     per-business clones are disambiguated on the ZK path.

### MEDIUM

**M-1 — Stale doc inventory in BUILD_PROMPT.md Mission (line 12).** (Audit point 6.)
Current: `research briefs 01–12, audits 01–09, and CHANGESPEC-v2/-v3`. Stale on all three counts:
- briefs are now **01–13** (added `13-poseidon-unification`).
- CHANGESPEC set is now **v2/v3/v4**.
- the precedence note ends at "§13.8/§11.9 are the latest" but never mentions CHANGESPEC-v4 as the
  overriding hash/dual-root authority (arch §3 line 3 *does* — the two precedence notes disagree).
**Fix (line 12):** `research briefs `01`–`13`, audits `01`–`09` (+ in-progress v4 audits 10–12), and
`CHANGESPEC-v2`/`-v3`/`-v4`` and append to the precedence sentence:
"… and `CHANGESPEC-v4` (Poseidon unification) overrides all earlier hash/dual-root wording on conflict."
(Mirror the wording already in architecture.md line 3, which is correct.)
- Note: arch line 3 Status header is **already correct** (`briefs 01–13`, `CHANGESPEC-v2/-v3/-v4`, v4
  precedence). implementation.md has **no top-level Status/inventory header**, so nothing stale there.

**M-2 — `dogTagId` privacy wording dropped the v4 hash-agnostic qualifier in BUILD_PROMPT + impl.**
(Audit point 3.)
The v4 rule (CHANGESPEC §5; arch §4.2/§11.1/§13.5/§13.6 — lines 293, 647, 710, 724 all correct) is that
`dogTagId` must be **"neither `keccak256(microchip)` nor `Poseidon(microchip)`"** — hash-agnostic, because
any hash of a low-entropy chip is brute-forceable. Three lines still say only `keccak256(microchip)`:
- `BUILD_PROMPT.md:58` — "`dogTagId` is a non-personal random/sequential id (never `keccak256(microchip)`)".
- `BUILD_PROMPT.md:59` — Foundry test "`dogTagId != keccak256(microchip)`".
- `BUILD_PROMPT.md:116` — negative test "esp. `dogTagId != keccak256(microchip)`".
- `implementation.md:1385` — "MUST assert it is never `keccak256(microchip)` (audit-06 §4.2)".
(BUILD_PROMPT principle #7, line 30, is already hash-agnostic — "an unsalted hash of a low-entropy
microchip number is brute-forceable" — so the principle is fine; only the dogTagId test specs lag.)
**Fix:** in each of the four lines, change to "never any hash of the microchip (neither
`keccak256(microchip)` nor `Poseidon(microchip)`)"; for the test specs, assert `dogTagId` is not
`keccak256(microchip)` **nor** `Poseidon(microchip)`. Low severity for security (the value is meant to be
random/sequential regardless) but it is a v4 consistency miss the spec explicitly calls out.

### LOW / INFORMATIONAL

**L-1 — §11.8 retains the pre-unification draft with a clipped pointer sentence.** impl §11.8 header
(lines 1431–1433) reads: "The §11.8 bodies below are the pre-unification (dual-root) drafts retained for
diff context. **CODE §11.9** …" — the verb is missing ("CODE §11.9" should read "**Code from §11.9**" or
"**Use §11.9**"). The §11.8(a) `recordVerification` normal-path body, however, is **already the unified
version** (single `R`, `purpose`, `isValid(R)` direct, `PoseidonT7` nullifier — lines 1482–1500), while
the ZK body is correctly stubbed as `⚠️ SUPERSEDED — CODE §11.9(e)` (lines 1502–1509). The mixed state
(normal path = final, ZK path = superseded stub, header = "all pre-unification drafts") is internally
confusing though not contradictory. **Fix:** correct the clipped sentence to "**Use the code in §11.9**"
and narrow the header to "the **ZK** body below is the pre-unification stub; the normal-path body is
already unified — code §11.9 for the ZK path."

**L-2 — `VerificationConsent` struct field-order differs between arch §3.6 and impl §11.8/§11.9
(non-blocking, but note).** arch §3.6 (lines 236–244) lists the struct **without** `purpose`/`challenge`
(it predates §13.8) — `{dogTagId, recordType, credentialRoot, relayer, subject, nonce, deadline}` — while
impl §11.8 (line 1459) and §11.9(a) (lines 1642–1646) add `purpose` and `challenge`. This is governed by
within-doc precedence (§13.8/§11.9 win), and arch §13.8 line 745 explicitly adds `purpose`+`challenge`,
so it is not a v4 regression. But arch §3.6's struct still shows the single root correctly
(`credentialRoot = R`, both paths) — only the field set is pre-§13.8. Optional: add a one-line
"see §13.8 for the `purpose`/`challenge` additions" pointer at arch §3.6. Out of strict v4 scope.

**L-3 — `DogTagSBT` §2.4 body (impl lines 397–417) and Deploy step 6 (line 447) still use the old global
`isWhitelisted`/`whitelistIssuer` and lack granular roles/`issuerOf`.** This is pre-audit code superseded
by §11.7(a)/§11.1 (impl line 1687 explicitly marks §2.1–§2.4 superseded), so within-doc precedence covers
it. **Not a v4 issue** (v4 didn't touch SBT roles) — noted only because it co-located with the v4-edited
§2.2/§2.6 and a reader skimming §2.x could be misled. No fix required for v4.

---

## Checklist results (per audit instruction)

### 1. No dangling deleted-term references — PASS
Every occurrence of each deleted term is inside a deletion/superseded/resolved callout. No active spec.

| Term | architecture.md | implementation.md | BUILD_PROMPT.md |
|---|---|---|---|
| `rKec` | 274, 453, 466, 746 — all "deleted/removed/RESOLVED" callouts | 148, 376–381, 1424, 1502 (SUPERSEDED), 1631, 1652 — all deletion callouts | 20, 32 — "no rKec/rZk duality" / "deleted" |
| `rZk` | 274, 453, 466, 746 — callouts | 148, 378, 1424, 1502, 1631, 1652, 1654 — callouts | 20, 32, 50 ("no `hashLeafZk`/`rZk`"), 66 ("no `rZk`") |
| `zkCommit` | 274, 466, 746 — callouts | 378, 381, 1035, 1424, 1433, 1502, 1632, 1635, 1652 — callouts | 32, 61 — "no zkCommit" |
| `ZkCommitment` | 274 — callout | 378, 1652 — callouts | 61 — "NO … `ZkCommitment`" |
| `kecOf` | 274, 453, 466, 747 — callouts | 378–433, 1448, 1502–1668 — all "no kecOf" callouts | 32, 61 — "no … `kecOf`" |
| `zkIndex` | 274, 747 — callouts | 1035, 1425–1685 — all deletion callouts | 32, 66 — "no `zkIndex`" |
| `cloneOf` | 274, 747 — callouts | 1635, 1652 — deletion callouts | 66 — "no … `cloneOf`" |
| `issuerForAny` | 274, 747 — callouts | 1425–1673 — all "undefined/deleted" callouts | 66 — "no … `issuerForAny`" |
| `hashLeafZk` | (none) | 148 — "removed" callout | 50 — "no `hashLeafZk`/`rZk`" |
| `0x02` binding leaf | (none) | 1636, 1652 ("`keccak(0x02‖rKec‖rZk)` … removed") | (none) |
| "dual root" / "parallel Poseidon/keccak" | "dual-root machinery … deleted" (274); "no parallel keccak root" (466) | "dual-root … removed" (377, 1431) | "no `rKec`/`rZk` duality" (20, 32) |

Research/ note: `CHANGESPEC-v4.md` uses all these terms normatively (it is the deletion spec), which is
correct and expected — it is the source, not active spec. No other research file is cited normatively in a
way that re-introduces the terms.

### 2. Single-root coherence — PASS
- issuance `issue(R)`: arch §4.1/§4.4 (lines 274, 363), impl §2.2/§11.1 (lines 363–364), BUILD #9 — all one Poseidon `R`.
- wrapped-doc `signature`: impl §1.4 line 141 `targetHash:R, proof:[], merkleRoot:R`. arch §3.1 consistent.
- consent `credentialRoot`: arch §3.6 lines 240/248/249 "`credentialRoot = R` (both paths)"; impl §11.8 line 1459/1494. Consistent.
- ZK public signals: **`[dogTagId, purpose, relayer, subject, nullifier, keyHash, R]` (7 signals)** everywhere —
  arch §4.6 line 405, §4.7 lines 436/462, impl §2.6 line 427, §11.8(a) line 1444, §11.8(d) line 1557, §11.9(d) line 1654,
  BUILD Phase 2.5 line 66. **No leftover 5-signal `rZk` tuple as active spec.** (`pub[5]` at arch 443 / impl 1663 = `keyHash`,
  the legitimate index-5 signal; the only `pub[5]=[…,rZk]` reference is the SUPERSEDED stub at impl 1502.)
- `isValid(R)`: checked directly on the public root on both paths — arch §4.7 lines 429/445, impl §11.8 line 1494, §11.9(e) line 1668. Consistent.
- verification integrity pillar: arch §5 line 476 + impl §11.3 lines 1199–1202 recompute Poseidon leaves+Merkle → `targetHash`/`R`. Consistent.

### 3. Hash policy consistency — PASS (with M-2 caveat)
- "keccak retained ONLY for EIP-712/ECDSA/addresses/namespacing (recordType, VERIFY:, clone salt)" stated
  consistently: arch §3 hash-policy box (line 84) + §13.8; impl §0 box (lines 29–34) + §7-keep-list refs;
  BUILD line 20 + principle #1. CHANGESPEC §7 is the source. No credential-commitment path says keccak —
  the only keccak-in-commitment-context hits (arch 84, 174, 646, 710, 746; impl 148, 1652) are all the
  policy statement itself or deletion callouts.
- Privacy wording hash-agnostic: arch §11.1 lines 646 ("hash-agnostic … keccak or Poseidon") + 647/710/724
  + §4.2 line 293 — all say `dogTagId` is "neither `keccak256(microchip)` nor `Poseidon(microchip)`".
  **Caveat (M-2):** BUILD lines 58/59/116 and impl line 1385 still say only `keccak256(microchip)`.

### 4. Issuer-clone resolution coherence — FAIL (see H-1)
`purposeToRecordType`/`issuerFor[recordType]` is described in impl §11.9 only; arch §4.7 uses a `...`
placeholder and §13.8 omits the ZK resolution. Flagged for contracts auditor re: uniqueness of the clone.

### 5. Determinism principle — PASS
BUILD principle #1 (line 24) + impl §11.2 (lines 1157–1184) + §9 (line 1028) + arch §13.2 (line 673) +
§3.4 box (line 176) all agree: 4 langs (circom/TS/Rust/Solidity); pinned libs
circomlib / `poseidon-lite` / `light-poseidon` (`new_circom`) / `poseidon-solidity` (`PoseidonT3..T7`);
CI anchor `poseidon([1,2]) = 0x115cc0f5...189a` (full value 0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a,
impl 1029/1181). No mismatch.

### 6. Stale doc inventory — one hit (M-1)
BUILD_PROMPT line 12 stale (briefs 01–12 → 01–13; audits 01–09 → +10–12; CHANGESPEC-v2/-v3 → +v4; precedence note omits v4).
arch line 3 Status header already correct. impl has no inventory header.

---

## Concrete fix list (copy-ready)

1. **BUILD_PROMPT.md:12** — bump inventory to briefs `01–13`, audits `01–09` (+ v4 audits 10–12),
   CHANGESPEC `v2/v3/v4`; append CHANGESPEC-v4 precedence clause (mirror arch line 3).
2. **architecture.md §4.7 (line 445)** — replace `issuerFor(...)` with
   `issuerFor[purposeToRecordType(bytes32(pub[1]))]` and add the `purposeToRecordType` definition sentence.
3. **architecture.md §13.8 (line 747)** — state the ZK clone-resolution via `purposeToRecordType`.
4. **BUILD_PROMPT.md:58, 59, 116 + implementation.md:1385** — add `nor Poseidon(microchip)` to the
   `dogTagId` hash-ban (hash-agnostic, per CHANGESPEC §5).
5. **implementation.md §11.8 header (lines 1431–1433)** — fix the clipped "CODE §11.9" sentence; clarify
   that only the ZK body is the superseded stub.
6. (Optional) **architecture.md §3.6** — pointer note that `purpose`/`challenge` are added in §13.8.
