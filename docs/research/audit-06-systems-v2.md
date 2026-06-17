# Audit 06 — Systems / Security / Privacy-Regulatory / Cross-Doc Consistency (v2)

> **Scope.** v2 updates applied to `architecture.md`, `implementation.md`, `BUILD_PROMPT.md`
> by three independent agents, against `docs/research/CHANGESPEC-v2.md` (§0 canonical names),
> `07-legal-privacy.md`, `08-wallet-integration.md`, and the prior systems audit
> `audit-03-systems.md`. **In scope:** cross-doc consistency, dual-signing system security,
> mobile wallet security, privacy/regulatory enforceability, the on-chain-ownership-vs-privacy
> tension, buildability/phasing. **Not in scope:** smart-contract internals and Merkle/keccak
> crypto correctness (audit-01/02 cover those).
>
> Auditor: systems-architecture, security, privacy/regulatory. Date: 2026-06-17.
> Severity legend — **Critical** (auth bypass / spoof / regulator-fatal / normative self-contradiction
> that will be miscoded), **High** (exploitable, breaks a core flow, or a real compliance gap),
> **Medium** (correctness gap, recoverable), **Low** (polish), **Info** (note).

---

## 0. Executive verdict

The v2 merge is substantively coherent — `architecture.md §13.5`, `implementation.md §11.6`, and
CHANGESPEC §0 agree on the big-ticket items (dual-signing `prepare`/`confirm` with on-chain
re-verification, multi-address whitelist, the `ownership` pillar, salt-as-erasure-lever, MPC
wallet storage). But three independent edits left **load-bearing contradictions**, the most
serious being that **`BUILD_PROMPT.md` still describes a THREE-pillar verifier** while
`architecture.md`/`implementation.md`/CHANGESPEC all moved to **FOUR** (added `ownership`). A
coding agent told to "honor the source of truth" will read conflicting normative statements about
how many pillars gate `valid`. Separately, the v2 design contains an **unresolved core tension**:
it asserts "nothing personal on-chain — ever" while simultaneously putting
`DogTagSBT.ownerOf(dogTagId) == userWalletAddress` on-chain and *requiring* it in verification —
a wallet-address↔pet link that is pseudonymous personal data under GDPR/CCPA and is **never
acknowledged** in the §11 privacy model or the erasure flow. This must be flagged and mitigated
before the privacy posture can be called consistent.

---

## 1. Cross-doc consistency (3 agents, 3 files)

### 1.1 [Critical] BUILD_PROMPT still says THREE pillars; everything else says FOUR
- `BUILD_PROMPT.md` Mission (line 16): *"Verification is three-pillar: integrity + on-chain status
  + DNS identity."*
- `BUILD_PROMPT.md` Non-negotiable principle #3 (line 22): *"All three verification pillars
  required. `verify()` returns `valid` only if integrity **and** on-chain status **and**
  DNS-identity-cross-checked-against-registry are VALID."*
- vs. `architecture.md §1`, §5, §13.2 D1, §13.5; `implementation.md §1.7`, §11.3; CHANGESPEC §4 —
  all define **four** pillars, `valid` iff `integrity && issuance && identity && ownership`.

This is not cosmetic: principle #3 is explicitly normative and explicitly enumerates the gate
condition with the *old* three-term conjunction. BUILD_PROMPT Phase 6 (line 85) *does* describe the
4th `ownership` check and the tri-state fragment — so **BUILD_PROMPT contradicts itself** as well
as the other two docs. A build agent that treats principle #3 as the rule will ship a verifier that
returns `valid` without the ownership pillar.
**Fix:** rewrite BUILD_PROMPT line 16 and principle #3 to "four-pillar: integrity + on-chain status
+ DNS identity + ownership (`ownerOf(dogTagId)==userWalletAddress`), tri-state, all four required."

### 1.2 [High] `implementation.md §1.7` (legacy verify) still hard-codes the OLD 3-pillar valid and the wrong on-chain-read shape
`§1.7` says `valid = integrity && issuance && identity && ownership` in the body but the prose box
above it ("v2 adds an `ownership` fragment … use [§11.3] when coding") admits §1.7 is superseded.
The problem is §1.7 is **still present, still reads as four booleans (not tri-state)**, and the
canonical §11.3 differs in behaviour (tri-state ERROR, registry cross-check, empty-proof guard,
N-confirmations). Two `verify` definitions in one normative doc, one stale.
**Fix:** either delete §1.7's pseudocode and point to §11.3, or mark it `// SUPERSEDED — see §11.3`
inline on every divergent line (currently only a prose note flags it).

### 1.3 [High] Endpoint set is consistent for `prepare`/`confirm`/`signing-mode` but the LEGACY `/records` issue path is a security back-door left undocumented for auth
`POST /credentials/prepare`, `/credentials/confirm`, `PUT|GET /settings/signing-mode` agree across
`architecture.md §6.2`, `implementation.md §3.8`/§11.6, CHANGESPEC §3, BUILD_PROMPT Phase 3. **But**
`implementation.md §3.3` keeps `POST /records` as "the `mode:"backend"` convenience shortcut" that
signs+broadcasts directly, and its inline `require` is only `unlocked && account whitelisted` — it
does **not** require an operator session (contrast §3.8 `prepare`, which adds `require ...operator
session`). So the v2 dual-signing path is auth-guarded but the retained legacy path is not, in the
text. See §2.4 — this is also a security finding.
**Fix:** either retire `/records` in v2 or make its guard read `require operator session && unlocked
&& whitelisted`, matching §11.4's "operator session guards all issuance routes."

### 1.4 [High] `SERVICE_ATTESTATION` rename is applied unevenly; the `assistanceType` enum disagrees with CHANGESPEC §0
- Record-type label `SERVICE_ATTESTATION` (renamed from a boolean "service dog") is consistent in
  `architecture.md §3.6` table, §3.6 field set, `implementation.md §1.6` validator, CHANGESPEC §0/§1.5.
  **Good.**
- But the **`recordType` enum value used in `implementation.md` A2.1 `PreparedCredential` comment**
  (research-08-sourced) lists `"VACCINATION | OWNERSHIP | LICENSE | ..."` — `OWNERSHIP`/`LICENSE` are
  **not** canonical record types (CHANGESPEC §0 enumerates `DOG_PROFILE, VACCINATION,
  SERVICE_ATTESTATION, TRAVEL_CLEARANCE, EU_HEALTH_CERT, DOT_SERVICE_FORM, CDC_IMPORT_FORM`). This
  comment leaked verbatim from research 08 into the SigningStrategy interface description; a coder
  may scaffold a non-existent `OWNERSHIP` record type.
  **Fix:** replace the placeholder enum in the `PreparedCredential.recordType` comment with the
  canonical CHANGESPEC §0 list.

### 1.5 [High] `microchip` object vs flat `microchipId` — the two validators disagree
- `architecture.md §3.6` and `implementation.md §1.6` use the **object** `microchip{code,standard,
  implantDate,bodyLocation}` and validate `m.code /^[0-9]{15}$/`, `m.implantDate <= vaccinationDate`.
  Matches CHANGESPEC §0/§1.2. **Good.**
- But the **corrected validator `implementation.md §11.5`** (which BUILD_PROMPT Phase 3 cites as the
  one to code, and which the doc says "use these versions when coding") still uses the **flat**
  `c.credentialSubject.microchipId` and `c.credentialSubject.microchipDate` — the *pre-v2* field
  names. §1.6 (object) and §11.5 (flat) are both normative, both about microchip, and they
  contradict on the canonical name. CHANGESPEC §1 item 2 mandates the object.
  **Fix:** rewrite §11.5 to operate on `microchip.code` / `microchip.implantDate` (the object), or
  add an explicit "§1.6 field names win; §11.5 only adds conditional/jurisdiction logic" note (the
  doc's closing paragraph gestures at this but §11.5's code still emits the old names — make the
  code consistent, not just the prose).

### 1.6 [Medium] Rabies field set: `§11.5` omits `vaccineProductCode` and `nextDueDate` that `§1.6`/§0 require
`§1.6` and CHANGESPEC §0 require `vaccineProductCode` (APHIS PCN) and `nextDueDate` as mandatory.
`§11.5`'s corrected presence list is `vaccineProductName, vaccineManufacturer, batchLotNumber,
vaccinationDate, validFrom, validUntil, authorizedVet` — **missing `vaccineProductCode` and
`nextDueDate`**. Since §11.5 is the "code-this-one" version, the coded-PCN and next-due
requirements silently drop.
**Fix:** add `vaccineProductCode` and `nextDueDate` to §11.5's required list (or, again, make §11.5
strictly additive over §1.6's field set).

### 1.7 [Medium] `signatureTrustTier` enum has TWO different value sets across the docs
- CHANGESPEC §0 and `architecture.md §3.6`/§11.1 + `implementation.md §1.6`: `signatureTrustTier in
  {accredited_authority, licensed_vet, self_attested}`.
- The **service-attestation** field `issuerTrustTier` (a *different* field) uses `{adi_accredited,
  licensed_pro, handler_self_attestation, unverified_registry}` — consistent across docs. **Good.**
- But `architecture.md §3.6` "Service/assistance attestation" prose and §11.1 conflate the two trust
  ladders in places; ensure a coder doesn't validate `issuerTrustTier` against the
  `signatureTrustTier` enum. They are distinct fields with distinct enums per §0.
**Fix:** add a one-line note in §3.6 that `signatureTrustTier` (envelope-wide) ≠ `issuerTrustTier`
(service-attestation-specific), with their respective §0 enums.

### 1.8 [Medium] `recordType` keccak mapping is consistent; the `OWNERSHIP`-as-recordType leak (1.4) is the only on-chain-name drift
`recordType` on-chain = `keccak256(label)` is consistent (arch §3.6, §13.4; impl §2.5 deploy; §0).
No new drift beyond 1.4. Confirmed consistent: ports, chainId 135, PLASMA, RPC, contract set,
the `https://<host>/r?t=&i=` deep-link, JWT `exp=180s` single-source (§13.4).

### 1.9 [Low] Erasure/consent endpoints consistent; one path-name nit
`POST /v1/consents`, `/v1/consents/{id}/withdraw`, `POST /v1/privacy/delete-request` are consistent
across `implementation.md §4.5`/§11.6, CHANGESPEC §2, BUILD_PROMPT Phase 4/8. `architecture.md §11.1`
describes them in prose without pinning the paths — fine, but add the canonical paths to arch §11.1
so all three docs name them identically. `consents` vs `consent_receipts` collection names match §9.

### 1.10 Cross-doc inconsistency summary (quick list)
1. **BUILD_PROMPT 3-pillar vs 4-pillar** (§1.1, Critical) — and BUILD_PROMPT internally self-contradicts.
2. **Two `verify()` defs** in impl: stale §1.7 (boolean) vs canonical §11.3 (tri-state) (§1.2).
3. **Legacy `/records`** lacks the operator-session guard the v2 `prepare` path has (§1.3, also §2.4).
4. **`OWNERSHIP`/`LICENSE`** non-canonical recordType leak in the SigningStrategy comment (§1.4).
5. **`microchip` object (§1.6) vs flat `microchipId` (§11.5)** — the "code-this" validator uses old names (§1.5).
6. **`§11.5` drops `vaccineProductCode` + `nextDueDate`** that §0/§1.6 mandate (§1.6).
7. **`signatureTrustTier` vs `issuerTrustTier`** two enums, conflated in §3.6 prose (§1.7).
8. **Erasure/consent endpoint paths** only in prose in arch §11.1, not pinned (§1.9).

---

## 2. Dual-signing security at the system level

### 2.1 [Info → confirmed sound] Mode-independence of what gets anchored
The decisive rule — wrap+merkle is **always server-side**, only sign+broadcast differs — is
consistently stated (arch §6, impl §3.8/§11.6, CHANGESPEC §3, BUILD_PROMPT principle #8). The
`confirm` re-verification (`RootIssued(root,signer)` event **and** `issuedAt[root]!=0`) means a
lying/buggy wallet-mode frontend cannot fake issuance. This closes the obvious attack and is
correctly specified. No finding on the core flow.

### 2.2 [Medium] Mid-flight mode switch — `confirm` does not re-check the signing mode against the persisted record's audit metadata
The settings toggle is persisted server-side and "switching affects only future signing; in-flight
prepared drafts are re-validated" (arch §6.3, impl §5.0). But `POST /credentials/confirm` (impl
§11.6) reads `signingMode` from `issuer_settings` *at confirm time* to write `r.audit`, while the
`unsignedTx` was built under whatever mode was active at `prepare` time. If an operator flips
`backend→wallet` between a backend-mode `prepare` (which already signed+broadcast inside `prepare`)
and… — actually backend mode confirms inside prepare, so the window is wallet-mode `prepare` →
(operator flips to backend) → wallet `confirm`. In that case `r.audit.signingMode` records
`"backend"` though a wallet EOA signed. The **record verifies fine** (audit-only fields are ignored
by verification — good), but the **audit trail is wrong**, which matters precisely after a key
compromise (research 08 A3.4: "persist signerAddress+mode so an auditor can see which key signed").
**Fix:** in `confirm`, derive the audit `signingMode` from the *recovered signer address* (is it the
backend-derived address or a wallet EOA?) or stamp the mode onto the prepared draft at `prepare`
time and read it back at `confirm`, rather than re-reading the live setting.

### 2.3 [Medium] "Block switching while a submit is pending" is asserted but the enforcement point is unspecified — concurrent inconsistent records are possible
Arch §6.3 and impl §5.0 say switching is blocked while a submit `isPending`. But `isPending` is a
*frontend* wagmi state; the **server-side** `PUT /settings/signing-mode` (impl §3.8) has no guard
against being called while a `status:"prepared"` record exists for that issuer. Two browser tabs, or
a frontend bug, can flip the persisted mode server-side while a prepared draft is mid-broadcast →
the draft's `unsignedTx` (mode-specific `to/data` is actually mode-independent, but the
**whitelist preflight** and the audit stamping are not). Inconsistent records aren't
*cryptographically* possible (root is mode-independent), but **audit metadata inconsistency and a
confusing UX** are. The "in-flight drafts get re-validated" promise has no server endpoint.
**Fix:** make the server reject `PUT /settings/signing-mode` (409) while any `status:"prepared"`
record is outstanding for the issuer, OR explicitly re-validate/re-`prepare` outstanding drafts on
switch server-side. Don't rely on a frontend flag for a server-persisted invariant.

### 2.4 [High] Operator-auth on the NEW endpoints — partially specified, with one real gap
audit-03 §9.1/H (carried into arch §13.3, impl §11.4: "operator session guards all
issuance/import/calendar routes; custody under `/admin`, localhost/session-bound") is the v1
remediation. v2 must apply it to the NEW endpoints:
- `POST /credentials/prepare` — impl §3.8 and §11.6 **do** say `require ... operator session`. **Good.**
- `POST /credentials/confirm` — impl §3.8/§11.6 pseudocode shows **no `require operator session`**.
  Confirm flips a draft to `issued` and is callable by whoever can reach the API. While it
  re-verifies on-chain (so it can't fake issuance), an unauthenticated caller could (a) confirm
  *another* operator's prepared draft once a real tx lands, or (b) probe record state. It mutates
  state and should be session-guarded.
- `PUT /settings/signing-mode` — impl §3.8 **does** `require operator session`. **Good.**
- `GET /settings/signing-mode`, `GET /issuer/signers` — unspecified auth; these leak the
  whitelist matrix / signer addresses. Should be operator-only.
- **Legacy `POST /records` (impl §3.3)** — guard is only `unlocked && whitelisted`, **no operator
  session** (see §1.3). This is the clearest "could an unauthenticated caller drive issuance?" gap:
  if `/records` is still mounted and the backend is unlocked, anyone reaching the public API port
  can POST a record and the backend will sign+broadcast with the clinic's funded key (operator pays
  gas, attacker chooses the payload). **This is High** — it's a remote, unauthenticated,
  gas-draining + spurious-issuance vector on a self-hosted box.
**Fix:** (a) add `require operator session` to `confirm`, `GET /settings/signing-mode`,
`GET /issuer/signers`; (b) either delete `/records` in v2 or gate it with operator session;
(c) state once, normatively, that **every** issuance/settings/signer route except
`GET /records/{id}` (record-JWT) and the HMAC cross-backend routes requires an operator session —
the v2 docs reference §11.4 but never re-list the new endpoints under it.

### 2.5 [Medium] Wallet-mode `confirm` trusts the frontend-supplied `signer` for the `ev.by == signer` check
`confirm {recordId, txHash, signer}` requires `ev.by == signer` — but `signer` is supplied by the
(untrusted, wallet-mode) frontend. The check is `RootIssued.by == signer`, i.e. it confirms the
frontend told the truth about who signed, not that the signer was authorized. The real
authorization is `issuedAt[root]!=0` (which only a whitelisted signer can have set, via
`onlyWhitelisted`) — so security holds. But `signer` should be **derived from the receipt/event**,
not taken from the request body, to avoid storing attacker-chosen audit metadata.
**Fix:** in `confirm`, set `signer = ev.by` (read from the on-chain event), ignore any client-supplied
`signer`. Tightens the audit trail (ties to §2.2).

---

## 3. Mobile self-custodial wallet security

### 3.1 [Info → sound] Embedded-MPC default + encrypt-then-store is correctly specified
Default = embedded MPC (MetaMask Embedded Wallets / Privy, real TSS, provider can't sign alone);
raw BIP-39 export advanced-only; encrypt-then-store (HW key in Secure Enclave/StrongBox encrypts the
seed, ciphertext in normal storage, biometric-gated, `…ThisDeviceOnly`, no auto-backup,
`biometryCurrentSet`-bound). Matches research 08 B2.2/B2.6 precisely (arch §10.1, impl §6.4,
CHANGESPEC §4, BUILD_PROMPT Phase 6). The "Enclave/StrongBox can't hold an arbitrary 256-bit seed
directly → encrypt-then-store" subtlety is captured. No finding on the storage model.

### 3.2 [High] Key-loss / recovery for non-crypto owners is UNSPECIFIED — and the SBT-ownership model makes loss catastrophic
The docs choose embedded MPC explicitly to avoid seed-phrase UX for non-crypto pet owners, but **none
of the three docs specify the recovery path** for the MPC default. Research 08 B2.3/B2.6 notes MPC
providers offer "email/social/passkey-linked shares" recovery — but the DogTag docs never adopt or
pin a recovery story. This is load-bearing because:
- The `DogTagSBT` is **owned by the user's wallet address** and `ownerOf(dogTagId)==userWalletAddress`
  is a **required verification pillar**. If the user loses the key (lost device + no backup, or a
  raw-BIP-39 user who didn't save the mnemonic), **the pet's on-chain identity is orphaned** — every
  record fails the ownership pillar, and the SBT is soulbound (can't be moved) and burnable only by
  protocol admin.
- The only recovery primitive specified is **admin burn-and-remint** "authorised by the user's
  signature proving control of the destination address." But a user who lost the key **cannot
  produce that signature** — the recovery primitive assumes the user still controls *a* key, which
  is exactly what's lost in the worst case.
**Fix:** (a) pin the MPC recovery path (e.g. Privy passkey/email-share recovery) as the default and
make it normative in arch §10.1 / impl §6.4; (b) specify a **key-loss recovery** for burn-and-remint
that does NOT require the lost key — e.g. account-level identity proof (the central account the SBT
was minted for) + admin attestation, since the central backend minted the SBT and knows
`userId↔dogTagId↔ownerAddress`. Document the trust trade-off (admin can re-home a pet identity → must
be gated/audited). Without this, "self-custody" becomes "lose your phone, lose your pet's identity."

### 3.3 [Medium] Burn-and-remint recovery path is named but not safely specified (replay / who-authorizes / what message)
`DogTagSBT.burn` is admin-only (good, audit-01 H-2). But the v2 "transfer = burn-and-remint
authorised by the user's signature proving control of the destination" is described in prose only.
Unspecified: what exactly is signed (EIP-191 vs EIP-712 typed data — research 08 B3.3 mentions both
but the docs don't pick), what's in the signed payload (must bind `dogTagId` + destination address +
a nonce/expiry to prevent replay across pets or re-use), who verifies the signature (central admin
backend before calling `burn`+`mint`), and how a prior owner is prevented from signing a re-home of a
pet they already sold.
**Fix:** specify the claim message as EIP-712 typed data binding `{dogTagId, newOwnerAddress,
nonce, expiry, chainId:135}`; verify server-side at the admin backend; require the *current* legal
owner relationship (ownershipHistory) as the authorization, not just any signature; one-time nonce.

### 3.4 [Medium] dApp-connect (Reown WalletKit) phishing surface is enabled but not risk-controlled
arch §10.1 / impl §6.4 enable `wc:`-URI dApp connect via Reown WalletKit. For a wallet that **owns a
soulbound pet-identity SBT and may hold PLASMA**, arbitrary dApp connection is a phishing vector:
a malicious dApp can request `eth_sendTransaction` (drain PLASMA, see §3.5) or `personal_sign`/EIP-712
of a crafted message — and §3.3's claim signature is exactly an EIP-712 message, so a phishing dApp
could try to harvest a burn-and-remint authorization. None of the docs specify connection-approval
UX, per-session scoping, transaction-simulation/clear-signing, or a warning when a dApp asks to sign
a DogTag-domain typed-data message.
**Fix:** specify (a) explicit session-approval UX with origin display; (b) a domain-separator check
so DogTag's own EIP-712 claim messages are only ever signed through the in-app claim flow, never via
a connected dApp (distinct `verifyingContract`/`domain` + an in-app guard); (c) clear-signing /
simulation for outbound txs. At minimum, document dApp-connect as advanced/optional and off by
default for non-crypto owners.

### 3.5 [High] The mobile wallet introduces custody of FUNDS (PLASMA) and an attendant support/loss burden that is NOT acknowledged
The wallet shows "address + PLASMA balance, send/receive" (arch §10.1, impl §6.4) and in **wallet
signing mode the user pays PLASMA gas** (arch §6) — so users must **hold and manage PLASMA**. This
silently turns a credentialing app into a **funds-custody product**:
- Lost key = lost funds (in addition to lost pet identity, §3.2).
- Support burden: "where's my PLASMA," failed-tx, wrong-network sends, dust/phishing token spam.
- Regulatory: a consumer wallet holding transferable value can implicate money-transmission /
  e-money considerations depending on jurisdiction and whether PLASMA has value — **none of the docs
  mention this.** (Contrast the careful GDPR analysis in research 07; there is no parallel analysis
  for hosting a consumer funds wallet.)
**Fix:** (a) explicitly acknowledge in arch §10.1 + §11 that the app custodies user funds and
enumerate the loss/support/regulatory burden; (b) consider gas-sponsorship / account-abstraction
(ERC-4337/7702, which research 08 B2.3 lists via Reown/Coinbase) so **pet owners never need to hold
PLASMA** — issuance gas is paid by the issuer's backend anyway, and the only user-side on-chain action
is import (read-only) + occasional claim. If owners never need PLASMA, send/receive of native funds
can be **omitted from v1**, eliminating most of this surface; (c) if funds custody stays, get a
money-transmission legal read analogous to the §07 privacy DPIA.

---

## 4. Privacy / regulatory (research 07) — enforceability of "nothing personal on-chain"

### 4.1 [Critical] The SBT owner address ↔ pet link is personal data on-chain and is NOT acknowledged in the v2 privacy model
This is the headline regulatory finding. The v2 design:
- mints `DogTagSBT` to and owns it at `userWalletAddress` (arch §4.2, impl §4.1), and
- **requires** `ownerOf(dogTagId)==userWalletAddress` as a verification pillar (arch §5, impl §11.3),

which writes an **owner-address ↔ specific-pet** association **permanently on an immutable,
globally-replicated ledger**. Under GDPR Art. 4(1) + Recital 26 and research 07 §4.1, **a wallet
address is at minimum pseudonymous personal data** the moment it is "reasonably linkable" to the
person — and DogTag's own central backend holds exactly that linkage (`users.selfCustodialWalletAddress`,
arch §9.1). CCPA §1798.140 ("reasonably capable of being associated… directly or indirectly") sweeps
it in too. Yet:
- `architecture.md §11.1` enumerates what's on-chain as "salted commitments, revocation/status,
  non-personal DIDs/keys, timestamps, schema/version, accreditation refs" — **the SBT owner address
  is conspicuously absent** from both the "permitted" and "never" lists.
- The §11.1 "Never on-chain (enumerated)" list does not mention wallet addresses at all.
- The erasure flow (impl §4.5/§11.6) destroys salts + off-chain records but **leaves the
  `ownerOf` link and the SBT itself on-chain** — `burn` isn't part of `erase()`.

audit-03 §8.2 already raised the linkability of the owner address (Medium); v2 **elevates it to a
hard problem** by making `ownerOf` a *required* verification input and by minting to the user's
*self-custodial* address (more likely reused across pets than a fresh custodial address). The whole
of a user's pet history becomes enumerable from one address via `ownerOf`/`Locked`/`profileRoot`
events.
**Fix (must-do):**
1. Add wallet addresses + the `ownerOf` link explicitly to the §11.1 on-chain inventory and
   **classify them as pseudonymous personal data** — do not let "nothing personal on-chain" stand
   unqualified (it is currently false). See §5 for the recommended resolution.
2. Make SBT `burn` part of the `erase()` flow (or document why the soulbound identity token survives
   erasure and how that's defensible — it generally is *not*, since the address↔pet link persists).
3. Refresh the **mandatory DPIA** scope to cover the `ownerOf` linkage (currently the DPIA text only
   contemplates salted commitments).

### 4.2 [High] Microchip-in-a-commitment: enforceability of "salt is the privacy mechanism" depends on an unstated salt-uniqueness/entropy invariant being testable, and on the microchip NEVER also appearing as a low-entropy on-chain value
The docs correctly state (arch §11.1, CHANGESPEC §2, research 07 §4.2) that an *unsalted* hash of a
15-digit microchip is brute-forceable, so per-field 16-byte salts are the privacy mechanism. This is
right. Two enforceability gaps:
- **No on-chain value other than the salted root carries the microchip.** Confirmed: only `bytes32`
  roots + `profileRoot` + owner address are on-chain (audit-03 §8.1). So the microchip is *not*
  directly on-chain. **Good** — but this invariant is only as strong as the guarantee that no future
  field (e.g. a `dogTagId` derived as `keccak256(microchipId)`, which audit-03 §1.4/§9.4 *recommends*
  for uniqueness) gets anchored. **`dogTagId = keccak256(microchipId)` would put a low-entropy,
  brute-forceable hash of the microchip on-chain as the SBT tokenId** — directly contradicting the
  privacy rule. The v2 docs leave `dogTagId` allocation unspecified (audit-03 §9.4 still open) while
  another audit recommends the microchip-derived form. **These two recommendations are in direct
  conflict and must not both be adopted.**
- The "salt is the privacy mechanism" claim is only enforceable if the build asserts salts are
  CSPRNG, unique-per-field, 16 bytes (arch §13.2 says so) AND the erasure flow actually destroys
  every copy. BUILD_PROMPT Phase 8 has an "erasure-unlinkability test" — good — but no test asserts
  the **negative**: that no low-entropy personal value is ever anchored.
**Fix:** (a) **forbid** `dogTagId = keccak256(microchipId)`; allocate `dogTagId` as a random/sequential
non-personal id with off-chain microchip→tokenId uniqueness (resolve audit-03 §9.4 the privacy-safe
way, and add a note that the microchip-derived option from audit-03 §1.4 is **rejected on privacy
grounds**); (b) add a PII-off-chain *negative* test to Phase 8 ("no anchored value is a hash of a
low-entropy identifier").

### 4.3 [Medium] Erasure flow completeness — consent receipts, retention, and 45-day SLA are present but incomplete
- Consent/`ConsentReceipt`, per-purpose lawful basis, withdrawal, `retention{basis,clock}`,
  `POST /v1/privacy/delete-request` with `dueBy: now+45d`, and `erase()` (destroy salts+keys+record)
  are all specified (impl §4.5/§11.6, arch §11.1). **Good coverage vs research 07 §7.4.**
- **Gaps:** (a) erasure does not burn the SBT / address link (§4.1); (b) the **business-side**
  erasure is unspecified — `erase()` lives only in the central backend (impl §4.5), but the *vet
  backend* is the GDPR **controller** for the record (research 07 §4.1) and holds the `records`
  collection with the wrapped doc + salts. A central delete-request must **propagate to every
  business backend** that holds copies, or erasure is incomplete. No cross-backend erasure
  propagation endpoint exists (contrast the appointment sync, which has one). (c) Consent
  withdrawal (`/v1/consents/{id}/withdraw`) "stops processing for that purpose" but doesn't trigger
  retention re-evaluation/erasure — the link to `erase()` is unwired. (d) The 45-day clock is
  asserted but no escalation/overdue handling.
**Fix:** add a central→business **erasure propagation** call (HMAC-signed, like appointment sync) so a
delete-request reaches every controller holding the record; wire consent withdrawal to retention/erase;
specify overdue handling on the 45-day SLA.

### 4.4 [Medium] DPIA is mandated but its scope/trigger is under-specified for v2's new on-chain fields
research 07 §4.2/§7.4 and arch §13.5 mandate a DPIA "refreshed on any change to on-chain fields or
chain topology." v2 **changed the on-chain fields** (SBT now at user address; `ownerOf` now a
verification input) and **changed chain usage** (mobile wallet, dApp connect), but the DPIA text was
not updated to enumerate these as in-scope. The DPIA as written contemplates only salted commitments.
**Fix:** explicitly list in the DPIA scope: (1) owner-address↔pet linkage and its retention/erasure;
(2) the mobile funds wallet (§3.5); (3) the public-vs-permissioned chain choice (research 07 prefers
permissioned; ROAX is an EVM devchain — state its node topology and Chapter V exposure).

### 4.5 [Info → confirmed sound] Art. 9 / service-data off-chain; evidentiary posture
Service/assistance attestation is correctly off-chain-only, Art. 9-flagged, `storage=="off_chain"`
validated (impl §1.6), never hashed on-chain (arch §3.6, CHANGESPEC §0/§1.5). Evidentiary (not
authoritative) posture, trust tiers, DOT-as-§1001-self-attestation, layered USDA→APHIS issuer chain —
all consistent with research 07 §3/§5/§7. No finding.

---

## 5. On-chain ownership vs privacy — the core tension (resolve / flag)

**The tension is real and currently unresolved in the docs.** v2 simultaneously asserts (A) "nothing
personal on-chain — ever" (arch §11.1, BUILD_PROMPT principle #7, CHANGESPEC §2) and (B) "the SBT is
owned by the user's self-custodial wallet and `ownerOf(dogTagId)==userWalletAddress` is a *required*
verification pillar" (arch §4.2/§5, impl §11.3, CHANGESPEC §4). (B) puts a pseudonymous personal
identifier (the wallet address) in a permanent on-chain association with a specific pet — and, via
that address, with the user's entire pet history. (A) is therefore **literally false as written**;
the honest statement is "no *cleartext or directly-identifying* personal data, and only *pseudonymous*
on-chain identifiers, accepted as residual risk." The docs never make this concession, which is both
a consistency defect (§1/§4.1) and a regulatory exposure.

### Recommendation (ranked)

1. **Accept-as-pseudonymous + minimize, and SAY SO (minimum viable, do this regardless).** Reclassify
   the wallet address as pseudonymous personal data in §11.1; record it in the DPIA as accepted
   residual risk; ensure the **only** on-chain personal datum is the address (no microchip-derived
   tokenId — §4.2); include SBT `burn` in the erasure flow so the link is severable. This is the
   floor; without it the privacy model is internally inconsistent.

2. **Fresh address per pet (recommended for v1).** Mint each pet's SBT to a **per-pet derived
   address** rather than one reused wallet address. This breaks cross-pet enumeration (audit-03 §8.2's
   exact recommendation) so an observer can no longer roll up "all of this person's pets" from one
   address. The embedded-MPC/HD wallet can derive `m/44'/60'/0'/0/{petIndex}` per pet at near-zero UX
   cost (the user still sees "one wallet"). Trade-off: gas/UX for funding multiple addresses — but if
   gas is sponsored (§3.5 / option 3) this is free. **This is the best cost/benefit and directly
   mitigates the §4.1 linkability.**

3. **Account abstraction (ERC-4337/7702).** Use smart-account SBT ownership with gas sponsorship
   (research 08 B2.3 lists Reown/Coinbase AA). Lets the protocol sponsor gas (so owners never hold
   PLASMA — also fixes §3.5) and enables rotating/contract-controlled ownership with social recovery
   (also fixes §3.2). Higher complexity; strongest long-term answer. Reasonable as a v2+ upgrade path
   on top of option 2.

**Verdict on the tension:** ship **option 1 (honest reclassification + burn-in-erasure + no
microchip-derived tokenId) as mandatory**, and **option 2 (fresh address per pet)** as the v1 design,
keeping option 3 (AA + sponsored gas) as the documented upgrade path. Do **not** ship the current
"nothing personal on-chain" wording unqualified — it will not survive a DPIA or a regulator's read.

---

## 6. Buildability & phasing

### 6.1 [High] Underspecified items that block implementation (v2-introduced)
- **MPC wallet recovery path** (§3.2) — Phase 6 says "default embedded MPC" but no recovery flow is
  specified; a coder cannot build account recovery from the docs. Blocking for a shippable wallet.
- **Burn-and-remint claim message** (§3.3) — the signed-payload format, verifier, and replay
  protection are unspecified; Phase 6 acceptance ("imports only when ownerOf==myAddress") doesn't
  exercise claim/transfer at all. Blocking for the transfer feature.
- **Cross-backend erasure propagation** (§4.3) — `erase()` is central-only; the controller (vet
  backend) erasure is unspecified. Phase 4/8 acceptance tests only the central side. Blocking for a
  defensible GDPR delete.
- **`dogTagId` allocation** (§4.2, still open from audit-03 §9.4) — unspecified, and one audit
  recommends a privacy-violating form. Must be pinned before Phase 4 (mint).
- **`ownership` pillar behaviour for third-party verifiers** — impl §11.3 says "if userWalletAddress
  is absent → ownership = ERROR," and since `valid` requires all four VALID, **a third-party verifier
  (a groomer importing, or a border officer) with no claimed owner address can NEVER get a `valid`
  verdict.** This breaks the groomer import flow (impl §3.5 `/import/pull` calls `verify` and
  `require verdict.valid`) — the groomer is not the owner, so ownership=ERROR → not valid → import
  rejected. **This is a functional contradiction introduced by making ownership mandatory.** Blocking
  for the import flow.
**Fix for the last one:** make the `ownership` pillar **conditional/contextual** — required for
*self-import* (mobile, "is this my pet"), but **informational (not gating)** for third-party
verification (groomer/border just need integrity+issuance+identity). Define the verdict for the
"no claimed owner" case as VALID-without-ownership, not ERROR. Update arch §5 / impl §11.3 and the
import flow (§3.5) accordingly. (This is arguably the most impactful buildability bug in v2.)

### 6.2 [Medium] BUILD_PROMPT phasing — mostly sound, two gaps
The phase plan correctly threads dual-signing into Phase 3, mobile wallet into Phase 6, portal
light/dark + wallet-connect into Phase 5, and privacy/erasure into Phase 4/8. Gaps:
- Phase 6 has no acceptance for **claim/transfer (burn-and-remint)** or **MPC recovery** — add them.
- Phase 8's "PII-off-chain audit" should add the **negative test** (§4.2: no low-entropy hash
  anchored) and the **owner-address pseudonymity acknowledgment / fresh-address-per-pet** check
  (§4.1/§5).
- The "three-pillar" wording in Mission/principle #3 (§1.1) must be fixed to four before Phase 1/6,
  or the SDK and mobile verifier get coded to the wrong gate.

### 6.3 [Low] Phasing still places auth model implicitly
audit-03 §9.7 asked to pull the operator-auth model forward into Phases 3–4; v2 references §11.4 but
the new endpoints (§2.4) aren't re-listed under it. Add an explicit "Phase 3 acceptance: confirm,
settings, signers, and any retained `/records` route are operator-session-gated; only `GET
/records/{id}` (record-JWT) and HMAC routes are unauthenticated."

---

## 7. Top fixes, prioritized

1. **(Critical, §1.1)** BUILD_PROMPT: change "three-pillar" / principle #3 to **four pillars**
   (add ownership, tri-state) — it currently contradicts itself and the other two docs.
2. **(Critical, §4.1)** Acknowledge the **owner-address↔pet link as on-chain pseudonymous personal
   data**; add it to §11.1 inventory; put SBT `burn` in `erase()`; refresh the DPIA scope.
3. **(High, §6.1)** Make the `ownership` pillar **non-gating for third-party verification** — as
   written it breaks the groomer/border import flow (ownership=ERROR ⇒ never `valid`).
4. **(High, §2.4)** Add operator-session auth to `confirm`, `GET /settings/signing-mode`,
   `GET /issuer/signers`; gate or retire legacy `/records` (unauth issuance + gas-drain vector).
5. **(High, §3.5)** Acknowledge **funds custody** (PLASMA) burden; strongly prefer gas-sponsorship/AA
   so owners hold no PLASMA (also removes most of the wallet attack surface) — get a
   money-transmission legal read.
6. **(High, §3.2)** Specify **MPC key-loss recovery** that doesn't require the lost key; otherwise
   losing the phone orphans the pet's on-chain identity.
7. **(High, §4.2)** **Forbid** `dogTagId = keccak256(microchipId)` (privacy); pin a non-personal
   `dogTagId` allocation; add a negative PII-off-chain test.
8. **(High, §1.3/§1.5)** Resolve the duplicate normative validators: pin `microchip` **object** field
   names in §11.5 (currently flat `microchipId`); restore `vaccineProductCode`/`nextDueDate`.
9. **(Medium, §4.3)** Add **cross-backend erasure propagation** (central→business) so the controller's
   copy is actually deleted; wire consent withdrawal to erase.
10. **(Medium, §3.3)** Specify the burn-and-remint **claim message** (EIP-712, bound dogTagId+dest+
    nonce+expiry), server-side verification, replay protection.
11. **(Medium, §2.2/§2.3/§2.5)** Tighten dual-signing audit metadata: derive `signer`/`mode` from the
    on-chain event, not the request/live-setting; server-side block on mode switch with outstanding
    prepared drafts.
12. **(Medium, §5)** Adopt **fresh address per pet** (option 2) to break cross-pet enumeration.

---

## 8. Bottom line

**A largely faithful v2 merge with sound dual-signing and wallet-storage cores, undermined by one
self-contradicting normative doc (BUILD_PROMPT's lingering three-pillar rule), an unacknowledged
on-chain owner-address↔pet linkage that makes "nothing personal on-chain" literally false, and a
mandatory `ownership` pillar that silently breaks third-party import — none unfixable, but the
privacy reclassification (+ fresh-address-per-pet), the four-pillar correction, the contextual
ownership pillar, and operator-auth on the new/legacy issuance endpoints are ship-blocking and must
land before any external deployment.**
