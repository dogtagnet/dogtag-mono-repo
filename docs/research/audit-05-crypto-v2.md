# Audit 05 — Crypto / Data-Structure Determinism / Erasure (v2 schema + ownership + erasure)

> Scope: the **v2** deltas only — finalized schemas (CHANGESPEC §0/§1: microchip object,
> `weightHistory[]`, coded `vaccineProductCode`/`breedVbo`, VC 2.0 array `@context`/`type`,
> `credentialStatus`), the new **ownership** verification fragment (CHANGESPEC §4), and the
> **right-to-erasure** cryptography (arch §11.1, impl §4.5/§11.6). Audits CRYPTO + DATA-STRUCTURE
> DETERMINISM + ERASURE. Builds on `audit-02-crypto.md`; its fixes (A1 decimal grammar, A2 typed
> input, A3 NFC, F2a flatten, F2b parse, D1 all-pillars) are taken as the baseline and checked for
> regression under the new field shapes.
>
> Severity: **Critical** (breaks a security guarantee or guarantees cross-lang root divergence on
> realistic v2 inputs) · **High** (will diverge / weakens security for plausible v2 inputs) ·
> **Medium** (latent / edge / hygiene) · **Low** (informational / defense-in-depth).
>
> Auditor verdict + exec summary at the bottom (relayed separately).

---

## Section 1 — Finalized schema vs canonicalization (determinism under new field shapes)

### V1. `weightHistory[].value` is the float trap re-opened — **CRITICAL**

`weightHistory[]{value, unit, measuredOn}` is the single highest-risk v2 field. The validator (impl
§1.6) requires `isDecimalString(w.value)` — **good, it intends a string** — but two things make the
float trap re-appear in practice:

1. The §0/CHANGESPEC examples and arch §3.2 show `weightKg: 22.7` and `"…:4:22.7"`. `22.7` as written
   in JSON **is** an IEEE-754 double in every JSON parser (TS `JSON.parse`, Rust `serde_json` default).
   audit-02 A2 already flagged this and pinned the fix: **typed input at the wrap boundary, numbers
   carried as strings, `assertNotFloat` is a hard guard.** v2 *adds a new decimal field* (`weightKg`
   → `weightHistory[].value`) and the validator's `isDecimalString` will run **after** a JSON parse
   that has already lossily coerced `22.7`→`22.699999999999999…`. If any code path builds the leaf
   from the parsed number (or `to_string()`s an `f64`), TS and Rust diverge and the float trap bites.
2. `weightHistory[].value` now lives **inside an array of objects**, so its keyPath is
   `credentialSubject.weightHistory[0].value`. The A2 schema-driven `mapType(keyPath)` must resolve a
   type for a path *with an array index in it*. If `mapType` keys off the literal path (with `[0]`),
   array-element types are not in the static schema table → an impl may fall back to `typeof` →
   re-opens A2 exactly. This is a v2-specific regression vector for A2.

**Fix:**
- Restate A2 normatively for v2: `weightHistory[].value`, `titer.resultIUml`, and any future decimal
  enter `wrapDocument` as a **typed decimal string**, never a JSON number. Add `assertNotFloat` cover
  for array-element scalars.
- `mapType` must resolve types for **array-element paths** by stripping the `[i]` index to a schema
  template path: `credentialSubject.weightHistory[0].value` → schema key
  `credentialSubject.weightHistory[].value` (type `decimal`). Pin this index-erasure rule.
- Add cross-lang test vectors: `weightHistory:[{value:"22.7",unit:"kg",measuredOn:"…"},{value:"22.70",…}]`
  → assert `"22.70"` canonicalizes to `"22.7"` (A1) and both SDKs produce identical leaf hashes.

```
mapType(keyPath):
    template = replace_all(keyPath, /\[[0-9]+\]/, "[]")   # weightHistory[0].value -> weightHistory[].value
    return SCHEMA_TYPES[template]                          # decimal ; never typeof
```

### V2. `titer.resultIUml` is still validated as a native number — **HIGH**

impl §1.6 line: `require c.titer.resultIUml >= 0.5` and §11.5 `c.titer.value >= 0.5`. A `>=` numeric
comparison implies the value is held as a float at validation time. Same trap as V1: `0.5` is an f64.
Either (a) validate on the **string** with a decimal comparator, or (b) parse to a big-decimal, never
f64. Also note the **field-name drift** between §1.6 (`resultIUml`) and §11.5 (`value`) — pin one
(CHANGESPEC §0 says `resultIUml`); a validator keyed to the wrong name silently skips the check.

**Fix:** compare titer via `decimalGte(canonicalDecimal(s), "0.5")` on the string; canonical field is
`titer.resultIUml`; carry as decimal string end-to-end (same as weight).

### V3. Array `@context` / `type` flatten to indexed keyPaths — deterministic IF F2a holds, but introduces an **ordering trust** issue — **HIGH**

VC 2.0 makes `@context` and `type` **arrays**: `["https://www.w3.org/ns/credentials/v2","<dogtag>"]`,
`["VerifiableCredential","RabiesVaccinationCertificate"]`. Under the pinned F2a flatten grammar these
become leaves at `@context[0]`, `@context[1]`, `type[0]`, `type[1]`. This is **deterministic** — F2a
fixes array index as `[i]` base-10, and buildMerkle sorts leaves bytewise so element *emission* order
is irrelevant to the root. **No indexing ambiguity at the hash level.** Two real problems remain:

1. **`@` and `[` in keyPaths.** F2a "reserved characters rejected" forbids `.`, `[`, `]` *inside
   keys*. The key `@context` contains `@` — is `@` reserved? It must be **explicitly allowed** (it is a
   legal JSON object key, not a path metacharacter), or an over-zealous validator rejects every VC.
   And `@context[0]` legitimately contains `[`/`]` as the **array-index delimiter**, not as part of the
   key — the "reject `[`/`]` inside keys" rule must apply to the *key segment only*, not the assembled
   path. Pin: reserved-char rejection is checked **per object-key segment before assembling the path**,
   and `@` (and other JSON-legal key chars except `.`/`[`/`]`) are allowed in a segment.

2. **Array order is semantically load-bearing but cryptographically unbound across the *set*.** Because
   leaves are sorted and each element is an independent leaf, the tree binds *the multiset of
   (keyPath,value) pairs* — i.e. it binds "`type[0]`==VerifiableCredential AND `type[1]`==Rabies…". An
   attacker cannot reorder without changing keyPaths (`type[0]`≠`type[1]`), so order **is** bound via
   the index in the keyPath. Good. **But** an attacker who controls a forged doc could *append*
   `type[2]="SomethingElse"` — that changes the leaf set → different root → fails issuance (pillar 2).
   So no root-preserving attack. The residual is a **policy** gap identical to audit-02 D2: a holder can
   **obfuscate** `type[1]` (the specific credential type) while keeping `type[0]`, and the doc still
   verifies VALID with a weaker asserted type. **`type[*]` and `@context[*]` MUST be in `requiredPaths`
   (non-obfuscatable) for every recordType.** Otherwise "valid" no longer implies "is a Rabies cert".

**Fix:** (a) per-segment reserved-char check, allow `@`; pin that `[i]` is path syntax not a key char;
(b) add `@context[*]`, `type[*]`, and `credentialStatus.*` to the per-recordType `requiredPaths`
(audit-02 D2 hook) so they cannot be silently obfuscated; (c) add VC-2.0 array vectors to
`testvectors.json`.

### V4. `microchip` as an OBJECT — determinism PASS, but the audit-02 A4 string-vs-int decision must be re-pinned for `microchip.code` — **MEDIUM**

The object flattens cleanly: `credentialSubject.microchip.code`, `.standard`, `.implantDate`,
`.bodyLocation` — all scalar leaves, deterministic under F2a. **However** audit-02 A4 left a live
ambiguity: is the chip number a **string (tag 2)** or **integer (tag 3)**? The §3.2 example used tag 2;
audit-02 recommended **keep it string** (fixed-width, leading-significant, a join key). v2 moved it
into `microchip.code` and the validator regex is `^[0-9]{15}$`. A 15-digit all-digit value is exactly
the case an impl might "helpfully" treat as an integer (tag 3) → different leaf hash than tag 2 → root
divergence, and a 15-digit ID `< 2^53` so it would even survive an f64 round-trip silently (worst kind
of bug — no crash, wrong hash). **Re-pin: `microchip.code` is `string` (tag 2), always.** A leading-zero
chip number (`042…`) is *also* why it must be a string — integer canonicalization strips leading zeros
and would corrupt the identifier.

**Fix:** schema type for `microchip.code` = `string` (tag 2), normatively, with a vector proving
`042000000000000` hashes as the 15-char string (not stripped to 14-digit integer).

### V5. `weightHistory[]` empty-array / empty-object structural invisibility — **MEDIUM**

audit-02 F2a defines empty `{}`/`[]` as emitting "a null-typed leaf at that path" (one revision) or
"nothing" (another revision in the same doc — arch §13 F2a says "empty containers defined"; impl §11.2
says "empty object/array -> a null-typed leaf at that path"). **These two are not the same and must be
reconciled.** For `weightHistory` this matters: a dog with no recorded weight has `weightHistory: []`.
- If empty array → null-typed leaf at `credentialSubject.weightHistory`, the empty state is **bound**
  (good — can't silently add a weight later under the same root; would need a new root anyway).
- If empty array → nothing, then `weightHistory:[]` and *absent* `weightHistory` hash **identically**
  → two semantically different credentials share a root. Minor, but it's a determinism+semantics gap.

The two docs currently disagree, which is itself a **cross-impl divergence risk** (a TS impl reading
impl §11.2 emits a leaf; a Rust impl reading arch §13 might not).

**Fix:** pin ONE rule in both docs. Recommended: **empty container → one null-typed (tag 0) leaf at the
container path.** Add vectors `{a:{}}`, `{a:[]}`, `{a:null}` — note all three are tag-0 leaves at path
`a` and therefore hash **identically**; if you need to distinguish them, that requires a structural tag,
which is out of scope — document that empty-object ≡ empty-array ≡ null under this grammar.

### V6. "Stop duplicating identity" means the vaccine root no longer binds the dog's identity fields — **HIGH (data-model correctness)**

CHANGESPEC §1.7 / arch §3.6: the vaccine credential references `dogTagId` only; it does **not** copy
name/breed/microchip. Cryptographically fine. But it changes the **trust model** of the `ownership`
fragment and the join: a verifier now learns "root R is issued for `dogTagId` D" and separately
"`ownerOf(D)`==myWallet". **Nothing in the vaccine credential cryptographically binds the *microchip*
to `dogTagId` D** — that binding lives only in the `DOG_PROFILE` credential. So a relying party
checking "is THIS physical chip vaccinated" must (a) verify the vaccine cred references D, AND (b)
verify the `DOG_PROFILE` cred for D binds chip↔D, AND (c) verify both roots on-chain. If a verifier
only checks the vaccine cred, the chip number it shows (if any) is **unbound to D**. This is a
correctness property the verifier MUST enforce, not a hash bug.

**Fix:** document the **two-credential join requirement**: chip→dogTagId binding is asserted *only* in
`DOG_PROFILE`; any flow that asserts "this chip is vaccinated" MUST verify both the vaccine cred and
the profile cred (both roots valid on-chain, both for the same `dogTagId`). Add `dogTagId` to the
vaccine cred's `requiredPaths` (non-obfuscatable) so the reference can't be hidden.

### V7. Coded `breedVbo` / `vaccineProductCode` — see Section 4 (string normalization). PASS-with-normalization.

---

## Section 2 — Ownership fragment (tri-state + cross-party verification)

### V8. Requiring `ownership==VALID` for ALL verifications breaks legitimate cross-party flows — **CRITICAL (correctness / availability of a core feature)**

impl §11.3 sets `valid = integrity && issuance && identity && ownership` and comments: *"if
userWalletAddress is absent (third-party verifier, no claimed owner) → ownership = ERROR"*. Combined
with D1 "all four pillars must be VALID", this means **any verification by someone who is not the
pet's on-chain owner returns INVALID** (because ERROR≠VALID and `valid` requires all four VALID).

This is wrong for the system's own use cases:

1. **Groomer importing a customer's vaccination record** (impl §3.5 `/import/pull`, §5.2 "verify on
   chain+DNS before accepting"). The groomer is **not** `ownerOf(dogTagId)` — the **customer** is. Under
   the all-four rule the groomer's verify *always fails ownership* → the legitimate record is rejected.
   The very flow CHANGESPEC §3/§5 describes (groomer pulls + verifies a shared vaccination status) is
   broken by making ownership mandatory.
2. **A vet verifying another clinic's record**, an **airline/border officer** verifying a travel cert,
   any **third-party relying party** — none of them own the SBT. All would get INVALID.
3. Even **`/import/pull` in impl §3.5 calls `verify(doc,{rpc,dns})` with NO `userWalletAddress`** — so
   per §11.3 ownership=ERROR → `valid=false` → `require verdict.valid` **rejects every business import**.
   This is a direct, shipped contradiction: §3.5 omits `userWalletAddress`, §11.3 then fails it.

The ownership pillar is meaningful for **exactly one** flow: the **mobile owner importing a record as
"mine"** (impl §6.5) — there, "is the SBT owned by the address I control" is the right gate. It is
**not** a universal credential-validity pillar.

**Fix — make ownership a *contextual* pillar, not a universal one:**

```
verify(doc, {rpc, dns, userWalletAddress?, mode}) -> Verdict:
   integrity / issuance / identity as before (these define CREDENTIAL VALIDITY)
   credentialValid = integrity==VALID && issuance==VALID && identity==VALID

   if mode == "self-import":           // mobile owner claiming a record (impl §6.5)
       require userWalletAddress present
       ownership = (ownerOf(dogTagId)==userWalletAddress) ? VALID : INVALID   // ERROR on RPC fail
       valid = credentialValid && ownership==VALID
   else:                               // third-party / cross-party verification (groomer, airline, vet)
       ownership = userWalletAddress present
                   ? (ownerOf(dogTagId)==userWalletAddress ? VALID : INVALID)   // informational
                   : NOT_APPLICABLE
       valid = credentialValid          // ownership does NOT gate cross-party validity
   return { valid, fragments:{ integrity, issuance, identity, ownership } }
```

- Credential **authenticity/integrity** = pillars 1–3 (unchanged from audit-02 D1; still all-required).
- **Ownership is a 4th fragment whose *requirement* depends on the verification purpose.** For
  self-import it is required and gating. For third-party verification it is `NOT_APPLICABLE` (or
  informational "this record's pet is/*isn't* owned by wallet X"), and MUST NOT force INVALID.
- D1's "all four required" must be **re-scoped**: it remains true for the *self-import* path; the
  *general* `verify` requires the **three authenticity pillars**. Update arch §5 / §13 D1 accordingly —
  as written, D1 over-claims and contradicts §3.5.

> Note this is consistent with audit-02 D1's *intent* (don't trust integrity alone; bind to the chain +
> DNS). Ownership binds *to a person*, which is only meaningful when a person is claiming the record.

### V9. Ownership tri-state: ERROR-vs-INVALID conflation for the absent-wallet case — **MEDIUM**

§11.3 maps "no claimed owner" to **ERROR**. ERROR means "couldn't check" (transient; UI says "retry").
But "no wallet supplied" is not a transient failure — it is a **deliberate third-party verification**.
Conflating them means a groomer's UI shows "ownership check errored" on every single import, training
operators to ignore the ERROR state (alarm fatigue) — which then masks a *real* RPC ERROR. Introduce a
distinct **`NOT_APPLICABLE`/`SKIPPED`** state for "ownership not in scope for this verification", keep
**ERROR** strictly for RPC/network failure of an *in-scope* ownership check. (This mirrors OA's
ERROR vs SKIPPED distinction, which audit-02 F3 already recommended for pillars generally.)

### V10. `doc.dogTagId` location is unspecified — **LOW**

§11.3 reads `doc.dogTagId` but the schema puts it at `credentialSubject.dogTagId` (impl §1.6). Pin the
exact path the ownership fragment reads (`doc.data.credentialSubject.dogTagId`, parsed from the packed
leaf) and require it be present + non-obfuscated (it's in `requiredPaths`, V6) before the ownership
RPC. Otherwise a missing/obfuscated `dogTagId` → undefined → `ownerOf(undefined)` reverts → ERROR for a
structurally-bad doc that should be INVALID.

---

## Section 3 — Erasure cryptography (the central question)

### V11. Destroying the salt alone does NOT guarantee unlinkability when the value is low-entropy — the requirement is incompletely specified — **CRITICAL**

The claim (arch §11.1, impl §4.5/§11.6): *"delete off-chain record + destroy salt/key → on-chain
commitment becomes unlinkable."* This is **mostly right but the stated requirement is incomplete and
the residual risk is mis-scoped.** Rigorous analysis:

The on-chain commitment is a **Merkle root**, derived from leaves
`leaf = keccak256(0x00 ‖ u32(len kp) ‖ kp ‖ u32(16) ‖ salt ‖ tag ‖ u32(len v) ‖ v)`.

**What is public/structural (NOT secret) in the preimage:** the domain byte `0x00`, all length
fields, the **keyPath `kp`** (e.g. `credentialSubject.microchip.code` — fully predictable from the
schema), and the **typeTag**. The ONLY high-entropy unknowns are `salt` (128 bits) and possibly `value`.

**The adversary's confirmation attack** (against a *target* whose chip number they suspect): given a
candidate `value*` (a guessed 15-digit chip = ~50 bits, or a breed from ~400 VBO codes = ~9 bits, or
sex/neuter = 1–2 bits), can they confirm `value*` is committed under root R?

- **If the salt is also known/guessable:** YES — they recompute the leaf and check Merkle membership
  against R. So salt is load-bearing.
- **If the salt is truly destroyed and unknown (16 random bytes, 128-bit):** to confirm `value*` they
  must brute-force the salt: for each of 2^128 salts, compute the leaf, test membership. **2^128 is
  infeasible.** So **salt destruction DOES unlink, *even for a 1-bit value*, PROVIDED the salt is (a)
  truly random 128-bit, (b) not recoverable from any surviving copy, and (c) not the same salt reused
  on another, surviving leaf.**

**Therefore the precise erasure requirement is NOT "destroy the salt and ensure value is
unguessable."** Value-guessing is *irrelevant* once a 128-bit salt is genuinely destroyed — the salt is
the hiding term and 128 bits defeats any value-domain brute force. The real, and currently
**unspecified**, requirements are:

1. **ALL copies of the salt must be destroyed, everywhere.** The salt lives in cleartext in the
   wrapped-doc `data` packed string (`saltHex:tag:value`). That document is **distributed**: issuer
   MongoDB, the holder's mobile app, any business that imported it (`pets_cache`, impl §3.5), any
   verifier that cached it, QR-shared copies, and **backups/replicas/log lines**. impl §4.5 `erase()`
   only iterates `offchain_records(ownerId, scope)` on the **central/issuer** backend. **It does not
   reach the holder's device, importing businesses' caches, or backups.** A single surviving copy of
   the packed string yields the salt → the value is confirmable → **not unlinked.** This is the dominant
   real-world failure mode and is **not addressed**.
2. **Salts must be unique per field per document** (audit-02 B3/G). If a salt were reused on a leaf in a
   *surviving* document, destroying it in the erased document is moot. Restate as a MUST.
3. **The 16-byte salt is sufficient entropy** — confirmed. 128 bits defeats value-guessing regardless of
   value entropy. No need for 32 bytes. (So the prompt's worry "is 16-byte salt enough when value is
   low-entropy" → **yes, if and only if the salt is actually destroyed everywhere; entropy is not the
   weak link, copy-proliferation is.**)
4. **Belt-and-suspenders:** even with salt destroyed, an immutable ledger means the commitment is
   *forever*. Best practice is to treat the value-entropy as a **second** independent barrier: where
   feasible, also ensure the value is not independently confirmable (e.g. don't *also* leave the chip
   number in a surviving non-erased credential for the same dog). This is defense-in-depth, not the
   primary mechanism.

**Recommended precise erasure requirement (normative):**

> **Erasure = render every committed leaf's preimage unreconstructable.** Concretely: for every
> off-chain record in scope, (a) **destroy all copies of every per-field salt** — issuer DB, all
> importing-business caches, the holder's device copy, QR/JWT caches, **and backups/replicas/WAL/logs**
> — by overwriting/cryptographically erasing, not just `delete`; (b) destroy off-chain blob encryption
> keys; (c) delete the cleartext record. **A 128-bit CSPRNG salt, once all copies are destroyed, makes
> the leaf preimage infeasible to reconstruct *for any value entropy* — the salt, not the value, is the
> hiding term.** The residual risk is **copy-proliferation** (a surviving salt copy re-links), not
> brute force; therefore erasure MUST enumerate and prove destruction of **all** salt copies (a
> copy-tracking / "salt custody ledger" per record), and the DPIA MUST record copies the protocol
> cannot reach (the holder's own device, third-party importers) as residual risk. This is a
> **mitigation, not a regulator-blessed safe harbour** (already stated — keep it).

### V12. `obfuscated[]` stores leaf HASHES — these LEAK across erasure — **HIGH**

Selective disclosure (arch §3.5, impl §1.5) moves a redacted field's **leaf hash** into
`privacy.obfuscated[]` and deletes the cleartext+salt from `data`. Two erasure interactions:

1. **The obfuscated leaf hash is itself a low-cost confirmation oracle IF the salt survives.** The
   obfuscated entry is `keccak256(0x00 ‖ … ‖ salt ‖ tag ‖ … ‖ value)`. The salt was *deleted from
   `data`* (good — audit-02 D3) — but the **hash** persists in any copy of the wrapped doc that the
   holder shared with the obfuscated field hidden. After erasure deletes the issuer's record, **copies
   of the obfuscated wrapped-doc held by recipients still contain the leaf hash.** If the salt for that
   obfuscated field was *ever* exposed (e.g. it was disclosed in a different share, or recovered from a
   backup), the recipient can confirm the hidden value against the persisted hash. So **`obfuscated[]`
   hashes are personal data that survive erasure in distributed copies**, exactly like the on-chain
   commitment — and they are **off-chain and copyable**, so easier to retain than the chain root.
2. **`erase()` does not mention scrubbing distributed wrapped-doc copies containing `obfuscated[]`.**
   Same gap as V11.1.

**Fix:** treat every distributed wrapped doc (including its `obfuscated[]` hashes and any surviving
salts) as in-scope for erasure; document that obfuscated leaf hashes have the **same unlinkability
dependency** as the on-chain root (salt destruction), and the **same residual** (copy-proliferation).
Add to the DPIA. Where the protocol cannot reach a copy (third-party holder), record as residual risk.

### V13. On-chain `setProfileRoot` history + `RootIssued`/`RootRevoked` events are permanent linkage metadata — **MEDIUM**

Even with off-chain salt destruction, the chain retains: `profileRoot[dogTagId]` history (via
`setProfileRoot` writes), `RootIssued(root, by, ts)` / `RootRevoked` events, `issuedBy[root]`, and the
**SBT `ownerOf(dogTagId)` → user wallet address** binding. The wallet address is **pseudonymous personal
data** (ICO/EDPB: a salted hash is personal; a persistent wallet address tied to a person is more so).
Erasure cannot remove these. The SBT is soulbound and burnable (admin burn, H-2) — **burning the SBT on
erasure** removes the live `ownerOf` binding (though historical Transfer/Locked events persist).

**Fix:** the erasure flow for a *full* owner deletion should **also burn the dog's SBT(s)** (admin
burn-and-forget, not remint) to drop the live `ownerOf`→wallet linkage; document that event-log history
(`RootIssued`, `Locked`, `Transfer`) is immutable and is residual DPIA risk; reinforce CHANGESPEC's
"prefer a permissioned network" so logs aren't globally replicated.

### V14. `destroy_salts` / `destroy_encryption_keys` are unspecified primitives — **MEDIUM**

impl §11.6 `erase()` calls `destroy_salts(rec)` and `destroy_encryption_keys(rec)` but neither is
defined. On MongoDB + journaled storage + replica sets + filesystem snapshots, a logical field
`unset`/document `delete` does **not** physically erase bytes (oplog, WAL, backups, SSD wear-leveling
retain them). "Destroy the salt" must mean **cryptographic erasure**: encrypt salts at rest under a
per-record (or per-owner) key and **destroy the key** (crypto-shredding), which is the only tractable
way to "destroy" data across replicas/backups. Pin this.

**Fix:**
```
# crypto-shredding model (makes "destroy the salt" tractable across replicas/backups):
on wrap:   per-record DEK; store salts (and packed `data`) encrypted under DEK; DEK wrapped by owner KEK
on erase:  destroy the per-record DEK (and remove KEK access) -> all ciphertext copies (incl. backups,
           oplog, importer caches that hold ciphertext) become undecryptable == salts destroyed
           then best-effort delete plaintext/ciphertext rows
# distributed copies the protocol does NOT control (holder device, third-party importer) cannot be
# crypto-shredded -> enumerated as residual risk in the DPIA (V11.1).
```

---

## Section 4 — Coded values (string normalization)

### V15. APHIS PCN / VBO codes hash by raw UTF-8 bytes — case + whitespace unpinned → divergence — **HIGH**

`vaccineProductCode` (USDA APHIS Veterinary Biologics PCN) and `breedVbo` (e.g. `VBO:0200798`) are
strings → tag 2 → `utf8(NFC_normalize(value))`. NFC normalization (audit-02 A3) handles Unicode
composition but **does NOT normalize ASCII case or whitespace.** So:

- `"VBO:0200798"` vs `"vbo:0200798"` vs `" VBO:0200798"` vs `"VBO: 0200798"` → **four different leaf
  hashes.** If a vet portal lowercases, an importer uppercases, or one trims and another doesn't, the
  same logical breed produces different roots. The "EU DCC lesson — coded values hash identically across
  jurisdictions" goal (CHANGESPEC §1.3) is **defeated** unless code normalization is pinned.
- APHIS PCN format (e.g. `"1A91.20"`) — same case/whitespace risk; also leading/trailing dot or zero
  ambiguity.

NFC alone is insufficient for coded identifiers. The codes need a **pinned canonical form** *before*
they enter the leaf (or as part of `encodeValue` for these specific fields).

**Fix — pin a coded-value normalization, applied at the validator/wrap boundary, and stored in `data`
in canonical form (so stored==hashed, per audit-02 A3.4):**

```
canonicalCode(s):                      # for vaccineProductCode, breedVbo, batchLotNumber, usdaNan, etc.
    s = NFC(s)
    s = trim leading/trailing ASCII whitespace
    reject if s contains internal whitespace        # codes have none; reject rather than collapse
    s = uppercase(s) for case-insensitive code systems (VBO prefix, APHIS PCN)   # PIN per code system
    require s matches the code system's regex (e.g. VBO: /^VBO:[0-9]{7}$/ ; usdaNan: /^[0-9]{6}$/)
    return s
```

Decide **per code system** whether it is case-sensitive (most ontology CURIEs uppercase the prefix:
`VBO:` not `vbo:`; the numeric ID is digits). `batchLotNumber` is a manufacturer string — likely
**case-sensitive, no normalization beyond trim+NFC** (lot `"AB12c"` ≠ `"ab12c"`) — pin it explicitly as
NOT uppercased, to avoid corrupting a case-significant lot number. Add vectors:
`"vbo:0200798"`/`" VBO:0200798 "` → `"VBO:0200798"`; `"1a91.20"`→`"1A91.20"`;
`batchLotNumber "AB12c"` → unchanged.

### V16. `microchip.standard`, `sex`, `neuterStatus`, enum strings — pin case — **MEDIUM**

Enums (`"ISO_11784_11785"`, `"male"`, `"intact"`, `"kg"`, `"primary"`) are tag-2 strings. The
validator checks membership against exact-case literals (impl §1.6), which is good — but the validator
runs on the *credential*, and the leaf is hashed from the same value, so as long as **validation
rejects any non-canonical case, the hashed value is forced canonical.** Confirm the validator is
**case-sensitive and exact** (it appears to be) and that there is **no lowercasing/normalization step
that would let a non-canonical case slip past validation into the hash**. Add a vector: `"Male"` →
**rejected at validation** (not silently lowercased). Same for `unit:"KG"` → rejected (must be `"kg"`).

---

## Section 5 — Regression of v1 (audit-02) fixes under v2 edits

| v1 fix | Survives v2? | Notes |
|---|---|---|
| **A1 decimal grammar** | **At risk** | The grammar itself is unchanged and correct, but v2 adds `weightHistory[].value` + `titer.resultIUml` as new decimals and the validator uses **numeric `>=`** on titer (V2) and examples show bare `22.7` (V1). The grammar survives; its *application* regresses unless V1/V2 fixed. |
| **A2 typed input / no f64** | **REGRESSED** (V1) | Array-element decimals (`weightHistory[0].value`) break `mapType` if it keys off the literal indexed path; `titer` `>=` implies a float. Fix V1 (`mapType` index-erasure) + V2. |
| **A3 NFC + store NFC in data** | Survives, **incomplete for codes** (V15) | NFC unchanged and correct. But NFC ≠ case/whitespace; coded values (`breedVbo`, PCN) need additional pinned normalization (V15). `@` in `@context` keyPath must be allowed (V3). |
| **F2a flatten / keyPath grammar** | **At risk** (V3, V5) | Arrays-of-objects (`weightHistory[i].value`) and array `@context[i]`/`type[i]` are handled by the `[i]` rule (PASS), but: (a) `@`/`[`/`]` reserved-char check must be **per-segment** not on the assembled path (V3); (b) empty-container rule **contradicts itself** across arch §13 vs impl §11.2 (V5) — a live cross-impl divergence. |
| **F2b first-two-colons parse** | **Survives, PASS** | New values that contain `:` — `breedVbo "VBO:0200798"`, ISO timestamps `measuredOn`/`implantDate`/`vaccinationDate`, `microchip.standard` has none — are correctly handled by split-on-first-two-colons. `"VBO:0200798"` as a value round-trips iff parse takes `valueRest` verbatim. **Add a vector** for a `breedVbo` value with its embedded `:` (it's the textbook F2b case). |
| **D1 all-pillars-required** | **REGRESSED / over-extended** (V8) | v2's 4th pillar makes "all four required" **break cross-party verification** and even self-contradicts impl §3.5 (which calls verify with no wallet). D1 must be re-scoped: 3 authenticity pillars always required; ownership required **only** for self-import. |
| Salt 16B CSPRNG unique-per-field (B3/G) | Survives, now **load-bearing for erasure** (V11) | Unchanged crypto; v2 elevates it from anti-forgery to the **erasure hiding term**. Restate CSPRNG+uniqueness as a MUST and add the wrap-time duplicate-leaf canary; uniqueness now also matters for erasure (V11.2). |
| Domain sep / leaf framing / second-preimage (B1/B2) | **Survives, PASS** | New object/array shapes don't touch leaf framing; every scalar is still an independent length-prefixed leaf. |
| Single-doc rebuild / one comparator (C2) | **Survives, PASS** | More leaves (objects/arrays expand the leaf set) but rebuild+sort is unchanged and deterministic. |
| processProof inclusion-only / batch caveat (C1/E2) | **Survives, PASS** | v2 is still single-doc (`proof:[]`); batch size-binding still a future requirement, unchanged. |
| obfuscated[] 32-byte well-formedness, no overlap (D1) | **Survives**, extended by V12 | Well-formedness check unchanged; v2 adds the **erasure-leakage** dimension (V12) and the **requiredPaths** dimension for arrays (V3/V6). |

**Net regression verdict:** Two v1 fixes regress under v2: **A2** (via array-element decimals +
numeric titer compare) and **D1** (the new ownership pillar makes "all four required" wrong for
cross-party verification and self-contradictory with §3.5). A1/A3/F2a are *at risk* (application
regresses, grammar intact) and need the V1/V2/V3/V5/V15 fixes. F2b is clean.

---

## Section 6 — Summary table

| ID | Area | Severity | One-line |
|----|------|----------|----------|
| V1 | `weightHistory[].value` float trap + `mapType` on array paths | **Critical** | New array-element decimal re-opens A2; `mapType` must index-erase to schema type, never `typeof`/f64. |
| V2 | `titer.resultIUml` numeric `>=` compare + field-name drift | High | Validate titer on the decimal **string**; pin canonical name `resultIUml`. |
| V3 | array `@context`/`type` flatten + `@`/`[]` reserved-char + obfuscation | High | Per-segment reserved-char check (allow `@`); add `@context[*]`/`type[*]` to non-obfuscatable requiredPaths. |
| V4 | `microchip.code` string-vs-int re-pin | Medium | Pin tag 2 (string); 15 digits could be mis-typed as int (`<2^53`, silent) and leading zeros would corrupt. |
| V5 | empty `weightHistory[]`/container rule contradicts itself | Medium | arch §13 vs impl §11.2 disagree (leaf vs nothing) → cross-impl divergence; pin one (null leaf). |
| V6 | "stop duplicating identity" unbinds chip↔dog in vaccine cred | High | Vaccine root no longer binds identity; relying party MUST join with `DOG_PROFILE`; `dogTagId` non-obfuscatable. |
| V7→V15 | coded values | (see V15) | — |
| V8 | ownership required for ALL verify breaks cross-party flows | **Critical** | Groomer/airline/vet aren't `ownerOf` → INVALID; §3.5 calls verify w/o wallet → rejects every import. Make ownership contextual. |
| V9 | ownership ERROR vs NOT_APPLICABLE conflation | Medium | "no wallet" is not a transient error; add SKIPPED/NOT_APPLICABLE; keep ERROR for RPC failure. |
| V10 | `doc.dogTagId` path unspecified | Low | Pin `credentialSubject.dogTagId`, require present+non-obfuscated before ownerOf. |
| V11 | erasure requirement incomplete; salt-copy proliferation, not entropy, is the risk | **Critical** | 128-bit salt unlinks for ANY value entropy IF all salt copies destroyed; `erase()` misses holder/importer/backup copies. |
| V12 | `obfuscated[]` leaf hashes leak across erasure | High | Off-chain copyable hashes have same unlinkability dependence (salt) + residual (copies) as the on-chain root. |
| V13 | on-chain events + `ownerOf` binding persist | Medium | Burn the SBT on full erasure to drop live `ownerOf`→wallet; event history is immutable residual. |
| V14 | `destroy_salts`/`destroy_encryption_keys` undefined | Medium | Logical delete ≠ physical erase on Mongo/WAL/backups; pin **crypto-shredding** (destroy per-record DEK). |
| V15 | coded `vaccineProductCode`/`breedVbo` case+whitespace | High | NFC ≠ case/whitespace; `"vbo:…"`≠`"VBO:…"` → divergent roots; pin `canonicalCode` per code system. |
| V16 | enum string case | Medium | Validator must reject non-canonical case (no silent lowercasing) so the hash is forced canonical. |

---

## Section 7 — Concrete normative additions for v2

1. **A2-for-arrays (V1):** `mapType(keyPath)` strips `[i]`→`[]` and resolves from the schema; decimals
   (`weightHistory[].value`, `titer.resultIUml`) carried as **typed decimal strings**; `assertNotFloat`
   covers array-element scalars. Vectors for `weightHistory`.
2. **Titer on string (V2):** `decimalGte(value, "0.5")`; canonical name `titer.resultIUml`.
3. **Per-segment reserved-char + allow `@` (V3):** reserved-char check on each object-key segment before
   path assembly; `[i]` is path syntax. Add VC-2.0 array vectors.
4. **`microchip.code` = string/tag 2 (V4):** vector with leading-zero chip.
5. **Empty-container rule pinned once (V5):** empty `{}`/`[]` → one null (tag 0) leaf at the path;
   reconcile arch §13 and impl §11.2. Vectors `{a:{}}`/`{a:[]}`/`{a:null}` (all equal).
6. **Two-credential join (V6):** chip↔dogTagId binds only in `DOG_PROFILE`; "this chip is vaccinated"
   flows MUST verify both roots for the same `dogTagId`; `dogTagId` in requiredPaths.
7. **Contextual ownership pillar (V8/V9):** `verify(..., mode)`; ownership gates only `self-import`;
   third-party verification requires the 3 authenticity pillars; add `NOT_APPLICABLE`/`SKIPPED` state;
   re-scope arch §13 D1; fix impl §3.5 to pass `mode`/wallet or call a 3-pillar variant.
8. **requiredPaths per recordType (V3/V6):** `@context[*]`, `type[*]`, `credentialStatus.*`, `dogTagId`,
   plus EU rabies product/manufacturer/batch — non-obfuscatable; `verify` returns INVALID/INCOMPLETE if
   a required path is missing or in `obfuscated[]`.
9. **Precise erasure requirement (V11):** destroy **all copies** of every 128-bit CSPRNG salt (issuer
   DB, importer caches, holder device where reachable, backups/WAL/oplog/logs) via **crypto-shredding**;
   salt-copy custody ledger; 16 bytes sufficient regardless of value entropy; copy-proliferation (not
   brute force) is the residual; unreachable copies → DPIA residual risk; mitigation not safe harbour.
10. **Crypto-shredding primitive (V14):** per-record DEK; salts/`data` encrypted at rest; erase =
    destroy DEK; defines `destroy_salts`/`destroy_encryption_keys`.
11. **Obfuscated-hash erasure scope (V12):** distributed wrapped docs (incl. `obfuscated[]` + any
    surviving salts) are in-scope for erasure; same unlinkability dependence + residual as the chain root.
12. **SBT burn on full erasure (V13):** drop the live `ownerOf`→wallet binding; event history immutable;
    prefer permissioned chain.
13. **`canonicalCode` normalization (V15):** NFC + trim + reject internal whitespace + per-code-system
    case rule (uppercase VBO/PCN prefixes; `batchLotNumber` case-preserving) + format regex; store
    canonical form in `data`; vectors.
14. **Enum case-strict (V16):** validator rejects non-canonical case; no silent lowercasing; vectors.
15. **F2b coded-value vector:** `breedVbo "VBO:0200798"` round-trips through first-two-colons parse.

---

## Auditor verdict

The v2 hashing core is **structurally sound** — objects, arrays-of-objects, and array `@context`/`type`
all flatten to deterministic, length-prefixed, sorted leaves under the audit-02 grammar, and the
domain-separated second-preimage protection is untouched. But v2 **re-opens the A2 float trap** for the
new decimal array fields, **breaks the D1 all-pillars rule** by making ownership universally required
(which kills the system's own cross-party import flows and self-contradicts §3.5), **under-specifies
erasure** (the binding requirement is *destroy every salt copy everywhere*, not "destroy the salt and
hope the value is unguessable" — and the current `erase()` reaches only the central DB), and **leaves
coded-value case/whitespace unpinned**, defeating the "codes hash identically across jurisdictions"
goal. Two v1 fixes (A2, D1) regressed; A1/A3/F2a are application-at-risk; F2b is clean.

**On the prompt's specific erasure question:** with a genuinely-destroyed 16-byte (128-bit) CSPRNG salt,
a low-entropy value (15-digit chip, small breed set) is **still unconfirmable** — the adversary would
have to brute-force 2^128 salts, not the value. **The salt, not the value, is the hiding term, and 16
bytes is enough.** Destroying *only* the salt **does** unlink — *provided every copy of that salt is
destroyed*. The weak link is **copy-proliferation** (the salt sits in cleartext in every distributed
wrapped-doc `data`), not entropy and not the chain. So the requirement is: **destroy salt AND every copy
of it (crypto-shredding across DB/backups/replicas/importer caches/holder device-where-reachable);
ensuring the value is independently unknowable is useful defense-in-depth but not the primary lever.**

**1-line verdict:** Hashing of the new shapes is deterministic, but v2 must (1) re-pin A2 for
array-element decimals, (2) make the ownership pillar contextual (it currently breaks cross-party
verify), (3) redefine erasure as crypto-shredding of *all* salt copies (16-byte salt is sufficient;
copy-proliferation is the real risk), and (4) pin coded-value case/whitespace — **NOT production-ready
until V1, V8, and V11 (all Critical) are fixed.**
