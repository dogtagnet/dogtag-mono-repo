# Audit 04 — DogTag v2 Blockchain / Contract / Signing Changes

> Scope: BLOCKCHAIN / SMART-CONTRACT / SIGNING design ONLY, focused on the **v2** deltas:
> multi-address whitelist, dual signing (`prepare`/`confirm`), user-owned soulbound SBT +
> `ownerOf` ownership pillar, and burn-and-remint claim/transfer. ROAX specifics in scope.
> Out of scope except where they change an on-chain trust assumption: off-chain SDK internals,
> calendar, JWT/QR (covered by audits 02/03), mobile UI.
> Sources audited: `architecture.md` §4 (esp. §4.2/§4.3), §5, §6, §13 (esp. §13.5); `implementation.md`
> §2/§11.1 (contracts), §3.8 + §11.6 (prepare/confirm), §4.3 (whitelist), §11.3 (verify), §6.4/§6.5 (wallet/import).
> Regression baseline: `audit-01-contracts.md` (v1 C-1/C-2/H-1/H-2/H-3, M-1, M-4).
> Date: 2026-06-17. Auditor: contract/signing security review.

The canonical contract artifact is **`implementation.md §11.1`** (the corrected v2 Solidity); `§2`
is the pre-remediation body and is explicitly overridden by §11 ("overrides §1–§9 on conflict").
The canonical signing flow is **§11.6** (overrides §3.8 on conflict). Findings cite §11.x as the
artifact under audit, and flag where §2/§3.8 still contradict it (a real risk, since two versions
of every contract/endpoint ship in the same doc).

---

## Severity legend
- **Critical** — direct forgery/loss of credentials, auth bypass, or an attacker can make the
  system record a false "issued"/ownership fact without controlling the asserted identity.
- **High** — privilege escalation among scoped parties, or a missing control that breaks a stated
  security property (provenance, mode-independence, soulbound ownership).
- **Medium** — exploitable under specific conditions, DoS, or spec/impl divergence that will cause real bugs.
- **Low** — hardening / defense-in-depth.
- **Info** — observations, future-proofing.

---

## Executive position

The v2 design is **directionally strong**: the prepare/confirm split with server-side merkle
building and on-chain re-verification correctly defeats the "lying frontend" class, the per-record-type
multi-address whitelist is the right shape, and the v1 criticals (C-1/C-2/H-1/H-2/H-3) remain
**intact** in `§11.1`. However the **dual-signing confirm path has a Critical replay/cross-issuer
gap (V2-C1)** and a **High reorg-finality gap (V2-H1)**, and the **burn-and-remint "transfer" is a
Critical unspecified signature scheme (V2-C2)** that as written lets anyone direct a remint. The
soulbound + user-custody model also introduces a **High permanent-loss-of-credential-binding risk on
lost keys (V2-H2)**. Several Medium items (whitelist mode-switch window, mint-griefing, `signer`
trust at confirm, `issuedBy` propagation to remint) need closing before deploy.

---

# CRITICAL

## V2-C1 — `confirm` accepts a caller-supplied `signer` and a foreign/replayed `RootIssued` event → false "issued" + provenance forgery
**Where:** `implementation.md §11.6` / `§3.8` `POST /credentials/confirm {recordId, txHash, signer}`:
```
ev = findEvent(receipt.logs, issuerAddrFor(r.recordType), "RootIssued")
require ev.root==r.root && ev.by==signer
require rpc.call(issuerAddrFor(r.recordType), "issuedAt(bytes32)", r.root) != 0
```
Brief Q2(a)/(c)/(d).

**Why it matters:** `signer` is taken **from the request body**, not derived from the transaction.
The only cross-checks are (i) a log in `receipt` whose contract == the right issuer, `root == r.root`,
`by == signer`, and (ii) `issuedAt[root] != 0` on that issuer. Two concrete breaks:

1. **Replay / confirm someone else's issuance (Q2-c).** `r.root` is server-computed and
   deterministic from `{recordType, dogTagId, fields}` and the per-field salts — but the salts are
   generated in `prepare` and stored server-side, so the root is unique to *this* prepared draft and
   an attacker cannot generally guess it. **However**, the confirm only requires that *some*
   transaction `txHash` produced a `RootIssued(r.root, signer)` log and that `issuedAt[r.root]!=0`.
   It does **not** require that `txHash` is the transaction the wallet was told to broadcast, nor
   that `receipt.from`/the actual tx sender equals `signer`, nor that `signer` is currently
   `isWhitelistedFor(recordType, signer)`. A malicious wallet-mode frontend can therefore submit a
   **stale or unrelated `txHash`** (any historical tx that emitted a matching `RootIssued`) — e.g.
   the *same root re-issued in a different clone of the same `recordType` for a different business*,
   or a backend-mode tx for an unrelated record — and have the draft flipped to `issued` even though
   the wallet user never paid for / signed this specific issuance. Because `issuedAt` semantics are
   per-clone and `issue` is idempotent-guarded only within one clone, the same `bytes32` root can be
   anchored in multiple clones (audit-01 I-3), so "a matching event exists somewhere with this root"
   is **not** equivalent to "this draft's issuance happened".

2. **Provenance forgery / mislabeled audit (Q2-b adjacent).** `r.audit.signerAddress = signer` is
   set from the unvalidated body. Even though the doc calls audit "metadata only", the whitelist
   model, delist-driven compromise response (§13.3 "mass-revoke the affected roots"), and the status
   UI all key off *which address issued what*. An attacker can attribute their issuance to an
   arbitrary whitelisted address, poisoning the forensic trail that compromise-response depends on.

The on-chain re-verification is doing **less** than the prose claims. "A lying/buggy frontend cannot
fake issuance" is true only for *content* (you can't anchor a root you don't have), **not** for
*"this specific prepared draft was issued by this whitelisted signer in this clone via this tx"* —
which is exactly what `confirm` purports to establish.

**Fix:**
1. **Derive `signer` from the transaction, never from the body.** Fetch the tx (not just the
   receipt), require `tx.to == issuerAddrFor(r.recordType)`, `tx.input == prepared.calldata`
   (the exact `issue(r.root)` calldata stored at prepare), `tx.value == 0`, `tx.chainId == 135`, and
   set `signer := tx.from` (recover from the signed tx / `receipt.from`). Then validate
   `registry.isWhitelistedFor(r.recordType, signer)` **at confirm time**.
2. **Bind the event to *this* transaction.** Require the matching `RootIssued` log to have
   `log.transactionHash == txHash` **and** `log.address == issuerAddrFor(r.recordType)` (explicitly
   assert the emitting contract — see V2-C3/H3) **and** `ev.by == tx.from`.
3. **Bind the root to the prepared draft, on-chain.** Adopt audit-01 M-3's leaf binding: include
   `(dogTagId, recordType, issuerEntityId)` in the leaf set so the root is unique to this draft and
   cannot collide with another business's anchoring of "the same" logical content. This closes the
   cross-clone replay at the cryptographic layer.
4. **Idempotency / single-use:** require `r.status == "prepared"` (present) **and** that no other
   record already claims `txHash`; store `confirmedTxHash` unique-indexed.

---

## V2-C2 — Burn-and-remint "transfer authorised by the user's signature proving control of the destination" is an unspecified signature scheme → anyone can direct a remint / steal an identity
**Where:** `architecture.md §4.2` (`burn` … `"transfer" = burn-and-remint to new owner's address,
authorised by the user's signature proving control of the destination`), §10.1, §13.5; `implementation.md
§6.4` ("admin burn-and-remint … authorised by the USER'S signature proving control of the destination
address"); contract `§11.1` `burn(uint256 id) external { require(admin) … }` then a later
`mint(to,...)`. Brief Q3.

**Why it matters:** The claim/transfer is the single most security-sensitive user-facing on-chain
operation (it moves the canonical pet identity and therefore the `ownership` verification pillar to a
new address), yet the **signature scheme is named but never specified**: no message format, no domain
separator, no chain-id binding, no nonce/replay protection, no statement of *who verifies the
signature* (the admin backend? the contract?), and no binding of the signature to the **specific
`dogTagId` and specific destination address**. The contract `burn`/`mint` are pure `admin`-gated — the
"user signature proving control of destination" lives **entirely off-chain in the admin backend** with
no defined verification. Concrete failures of an unspecified scheme:

- **No destination binding / replay:** if the signed message is e.g. "I authorise transfer of my
  DogTag" without `(dogTagId, newOwner, chainId, contract, nonce, expiry)`, the admin can be tricked
  (or a leaked signature reused) to remint to an address the *user does not control* — defeating the
  entire point ("proving control of the destination"). A signature that proves control of *some*
  address does not prove the *remint destination* is that address unless the destination is **inside
  the signed payload**.
- **Cross-chain / cross-contract replay:** without `chainId` (ROAX = 135) and the `DogTagSBT`
  contract address in the signed domain, a signature gathered for any other EVM context (or a future
  redeploy) replays.
- **Griefing / first-claim (Q3 last clause):** because `mint` is `onlyProfileIssuer` and microchip
  uniqueness is enforced **off-chain only** (arch §4.2: "microchip uniqueness enforced off-chain by
  central backend before mint; one SBT per microchip"), the *protocol* decides destination. If the
  claim flow trusts a user-supplied "this is my chip + here is my address" without verifying physical
  possession of the microchip, an attacker who learns a 15-digit microchip code can request a mint to
  **their** wallet, then `ownerOf` binds the pet to the attacker. The signature "proving control of
  destination" only proves the attacker controls the attacker's address — not that they own the pet.

**Fix:**
1. **Specify an EIP-712 typed-data signature** with a domain `{name:"DogTag", version, chainId:135,
   verifyingContract: DogTagSBT}` and a `Claim{dogTagId, newOwner, nonce, deadline}` struct. The
   user signs with the **destination** key (proving control of `newOwner`). Verify
   `ecrecover == newOwner` and `newOwner == to`.
2. **Enforce it on-chain, not just in the backend.** Add a single `claim(dogTagId, newOwner, nonce,
   deadline, sig)` (admin-gated *plus* signature-gated) that burns the old token and mints to
   `newOwner` atomically, checks a per-`dogTagId` nonce, and reverts past `deadline`. Don't leave the
   only enforcement in an undocumented backend path.
3. **Separate identity authority from possession.** The destination signature must not be the *only*
   gate for first-mint/claim. Bind initial mint to an off-chain proof-of-possession of the microchip
   (existing custodian/vet attestation, or a registration secret), and document that learning a chip
   code is **not** sufficient to mint/claim. Record `claimedFrom`/`claimedTo` events.
4. State explicitly **who calls burn**: it must be admin (already `require(admin)`), and the user
   signature is an *additional* required input to the same atomic operation — never a separate,
   replayable artifact handed to the admin.

---

# HIGH

## V2-H1 — `confirm` has no finality/reorg depth → a reorged-out issuance is permanently recorded as `issued`
**Where:** `implementation.md §11.6` `confirm`: `receipt = rpc.getTransactionReceipt(txHash);
require receipt.status==success` then flips `status="issued"` immediately. Contrast `§11.3 verify`
which reads `isValid(..., confirmations=5)` and `audit-01 M-4` (N-confirmation requirement).
Brief Q2 (reorg).

**Why it matters:** v1 M-4 was remediated for the *verify* read but the **same reorg risk was
re-introduced at the write/confirm boundary in v2**. `confirm` accepts a 0-confirmation receipt and
permanently marks the off-chain record `issued`. On a low-liveness chain that was returning 502s at
design time (arch header) and whose finality is unverified (arch §12 open item), a `RootIssued` tx
can be reorged out after confirm. The off-chain DB then says `issued`, the QR/JWT share path serves
the doc as issued, but pillar-2 `isValid(root)` reads **false** on chain → every downstream verifier
gets a hard INVALID for a record the issuing business believes is live. Worse, a malicious wallet-mode
frontend can deliberately confirm against a tx it knows is on a soon-orphaned fork.

**Fix:** require the same configurable `confirmations` depth (default 5, M-4) at `confirm` as at
`verify`: re-read `issuedAt[root] != 0` **at `block.number - N`**, or poll until the tx is N blocks
deep before flipping to `issued`. Until then keep the record in an intermediate `confirming` state.
Document reorg behaviour: if the tx disappears, revert the record to `prepared` and re-submit.

## V2-H2 — Soulbound + sole user-custody: lost key permanently severs the `ownership` pillar for every credential of that pet
**Where:** `architecture.md §4.2` (SBT minted to and owned by user's self-custodial wallet; soulbound,
`locked==true` permanently), §5 pillar 4 (`ownerOf(dogTagId)==userWalletAddress` is **required**),
§13.5 (all four pillars must be VALID); `implementation.md §11.3` (`ownership` required), §6.4 (MPC
default, BIP-39 advanced, biometric-gated `…ThisDeviceOnly`, **no auto-backup**). Brief Q3 (lost key).

**Why it matters:** The ownership pillar is **mandatory** — `valid` requires `ownership==VALID`, and
the import check (§6.5) refuses to import a record unless `ownerOf(dogTagId)==myWalletAddress`. The SBT
is soulbound (cannot be transferred to a recovery address) and stored with **no auto-backup,
`ThisDeviceOnly`, biometry-bound** keys. So if the user loses the device/key (lost phone, biometric
re-enrol invalidating the key per `biometryCurrentSet`, MPC provider loss), they **permanently lose the
on-chain owner address**, and *every* credential about that pet now fails the required ownership pillar
forever — the records become INVALID for the rightful owner. The only escape is the burn-and-remint
claim path (V2-C2), which is exactly the unspecified, risky operation. The MPC default (TSS, social/
passkey, provider can't sign alone) mitigates *casual* loss but the BIP-39 export and the
`ThisDeviceOnly`/no-backup posture mean recoverability is **not guaranteed by design** for the asset
that gates all verification.

**Fix:**
1. **Make `ownership` a non-fatal pillar for third-party verifiers.** A vet/border agent verifying a
   credential cares about integrity+issuance+identity; `ownership` answers "is this *my* pet" and is
   only meaningful for the holder. Per §11.3 it already returns `ERROR` when `userWalletAddress` is
   absent — but the overall verdict still requires all four. Re-scope: `ownership` is **required only
   for the holder's own import/binding context**, and for third-party verification it should be
   `N/A`/advisory, not a hard INVALID. Document the two verification contexts explicitly.
2. **Guarantee a recovery path** for the user-owned SBT before relying on it as a hard gate: rely on
   the MPC provider's social/passkey recovery as the *default* and document the burn-and-remint claim
   (V2-C2, hardened) as the sanctioned recovery for lost-key, gated by an off-chain identity proof to
   the protocol admin — not by possession of the (lost) key.
3. Warn in onboarding that BIP-39-export users who lose the seed lose the pet's on-chain binding and
   must go through admin-mediated re-mint.

## V2-H3 — `confirm` event match does not assert the emitting **contract address**, enabling a spoofed `RootIssued` from a look-alike/attacker contract
**Where:** `implementation.md §11.6` `findEvent(receipt.logs, issuerAddrFor(r.recordType),
"RootIssued")` — the prose passes the expected address, but the security property depends on the
implementation actually filtering `log.address == issuerAddrFor(r.recordType)`, **and** on
`issuerAddrFor` being resolved from the **trusted central registry**, not from anything in the
request. Brief Q2 ("wrong contract emitting RootIssued", "log spoofing"). Related to V2-C1.

**Why it matters:** `RootIssued(bytes32,address,uint256)` is a trivially-emittable event signature.
Any contract anyone deploys can emit an identical-topic log with arbitrary `root`/`by`. If `findEvent`
matches on topic/signature across **all** logs in the receipt without strictly pinning
`log.address == issuerAddrFor(recordType)` (the registry-known clone), a malicious wallet-mode tx can
call an attacker contract that emits a spoofed `RootIssued(r.root, victimSigner)` in the **same
transaction**, satisfying the event check. The `issuedAt[root] != 0` read is the backstop — but only
if `issuerAddrFor` is itself trusted and the `issuedAt` read targets that exact clone (it does in the
pseudocode). So this is a **defense-in-depth High**: the `issuedAt` read saves it *iff* implemented
exactly as written, but the event check is currently the weakest link and is the thing the prose
leans on ("RootIssued event present").

**Fix:** In `findEvent`, hard-require `log.address == issuerAddrFor(r.recordType)` (an address
resolved only from the admin-written central registry, §13.3 "registry self-write impossible") and
`log.transactionHash == txHash`. Treat the `issuedAt[root]!=0` read as the **primary** proof (it is
unspoofable — state on the trusted clone) and the event as a secondary cross-check. Never trust a log
whose `address` is not the registry-known clone.

---

# MEDIUM

## V2-M1 — Mode-switch onboarding has a window where the new signer is not yet whitelisted, and delist-old/whitelist-new are two unsynchronized admin txs
**Where:** `architecture.md §4.3` (onboarding: submit → admin verifies → `whitelistFor` per type →
poll until live; "Delist inactive-mode addresses"), §6.3 ("Switching affects only future signing …
in-flight prepared drafts are re-validated"); `implementation.md §3.8`/`§4.3`
(`whitelistFor`/`delistFor` are separate admin calls). Brief Q1 (mode-switch window).

**Why it matters:** A mode switch (wallet↔backend) or key rotation introduces a **new address** that
must be `whitelistFor`'d by the admin (a manual, off-chain-approval-gated, on-chain tx) before it can
issue, while the **old** address should be `delistFor`'d. These are **independent transactions with no
atomicity**:
- **Neither valid window:** if the operator flips the Settings radio to the new mode *before* the new
  address is whitelisted (approval queue latency), every `prepare`/`submit` reverts (`preflightWhitelist`
  fails). This is fail-*closed* (correct, no security loss) but is a real availability gap; the doc's
  "poll `isWhitelistedFor` until live" only helps if the UI **blocks the mode switch** until live.
- **Both valid window:** if the admin whitelists the new address but has not yet delisted the old, the
  issuer entity transiently has **two** live signing addresses for the same recordType. This is by
  design (multi-address) and **not** a privilege escalation, but it widens the compromised-key window
  and the audit ambiguity (which address "should" have signed). The doc says delist inactive-mode
  addresses but specifies no ordering/SLA.

**Forward-only delisting (Q1 last clause): CONFIRMED correct.** `isValid(root)` = `issuedAt && !revokedAt`
and **does not consult registry membership** (arch §13.3, `implementation.md §11.1` `isValid`). So
delisting a signer leaves its already-issued roots VALID — the intended forward-only property. Compromise
response therefore correctly requires the **admin mass-revoke** path (`adminRevoke(bytes32[])`, §11.1
comment) over affected roots, not just delisting. This is sound; just ensure `adminRevoke` is actually
implemented (it is only a comment today — see V2-M5).

**Fix:** (1) UI must **block the mode switch** (and block submit) until `isWhitelistedFor(newAddr)`
reads true — the doc says "poll until live" and "block switching while a submit is pending"; make
"block switch until new address live" explicit. (2) Specify ordering: whitelist-new **then** issue
under new, delist-old on a defined SLA after the switch is confirmed; never delist-old before
new-is-live (else neither-valid). (3) Record both addresses' status in the per-(address×recordType)
matrix (`GET /issuer/signers`) so the transient both-valid state is visible and bounded.

## V2-M2 — `prepare` mints a `recordId` + persists a `prepared` draft on every call → unbounded draft/root spam and root-squat surface
**Where:** `implementation.md §11.6`/`§3.8` `prepare` (`recordId = uuid(); save records{... status:"prepared"}`)
with `require unlocked && operator session`. Related to audit-01 M-3 (issue front-run).

**Why it matters:** In wallet mode `prepare` does the full wrap + persists a draft and returns an
unsigned tx, but nothing ties a prepared draft to ever being confirmed, nor rate-limits prepares.
A buggy/abusive operator-session frontend can generate unbounded `prepared` rows. More importantly,
because the unsigned tx `{to:issuerAddr, data:issue(root)}` is handed to the browser, a second
whitelisted signer (or anyone watching, post-broadcast) sees `root` and can **front-run `issue(root)`**
from a different whitelisted key in the same clone (audit-01 M-3 is *not* fixed in v2 — `issuedBy[root]`
records the front-runner). Combined with V2-C1's weak confirm, the honest user's draft then can't be
cleanly confirmed (their own tx reverts "issued"), and provenance is wrong.

**Fix:** Adopt audit-01 M-3 leaf-binding (also required by V2-C1 fix #3) so a front-run `issue(root)`
from another signer anchors a root **bound to a different `(dogTagId, recordType, issuerEntity)`** and
cannot impersonate this draft. Rate-limit/expire `prepared` drafts (TTL); garbage-collect unconfirmed
drafts. Make `confirm` tolerate "already issued by the expected signer in the expected clone via the
expected tx" vs. "issued by someone else" (the latter → surface a front-run error, re-prepare with
fresh salts).

## V2-M3 — Prepared `{value, chainId}` are not validated on confirm; `data` only implicitly validated via the event root
**Where:** `implementation.md §11.6` prepare returns `unsignedTx:{to, data, value:0, chainId:135}`;
confirm validates only `ev.root==r.root` + `issuedAt!=0`. Brief Q2 (is `{to,data,value,chainId}`
validated on confirm against what was prepared?).

**Why it matters:** The brief explicitly asks whether the prepared tx fields are re-validated on
confirm. Today they are **not** validated as a set:
- `data` is *indirectly* checked because `data == issue(r.root)` and the event carries `r.root` — OK
  in practice, but only because `issue` has one argument; brittle if calldata ever carries more.
- `to` is *indirectly* checked via the emitting-contract filter (and must be hardened per V2-H3).
- `value` and `chainId` are **not** checked at all. `issue()` is non-payable so a non-zero `value`
  reverts (no fund loss), and `chainId` is effectively fixed by which RPC the backend queries — so the
  practical risk is **low**, but the design claims confirm "re-verifies on-chain before marking
  issued" and the brief flags this gap directly.

**Fix:** At confirm, fetch the **transaction** (not just receipt) and assert the full prepared tuple:
`tx.to == prepared.to`, `tx.input == prepared.data`, `tx.value == 0`, `tx.chainId == 135`,
`tx.hash == txHash`. Store `prepared.{to,data,value,chainId}` at prepare time and diff on confirm.
This subsumes part of V2-C1 and makes the "re-verify what was prepared" claim literally true.

## V2-M4 — Mint griefing: `mint` reverts if `dogTagId` already exists, and microchip uniqueness is off-chain only
**Where:** `architecture.md §4.2` ("microchip uniqueness enforced off-chain"; `mint(to,dogTagId,root)`),
`implementation.md §4.1` (`/v1/pets/{id}/mint` → `DogTagSBT.mint(userWalletAddress, dogTagId, root)`),
`§11.1` `mint` (`_safeMint(to,id)` reverts on duplicate `id`). Brief Q3 (grief by claiming a microchip first).

**Why it matters:** `dogTagId` (tokenId) derivation isn't pinned in the contract — if `dogTagId` is
derived from the microchip code (a plausible design, since one SBT per microchip), then whoever mints
first for a given chip **claims that tokenId**, and a second mint reverts (`_safeMint` duplicate).
Because mint is `onlyProfileIssuer` (protocol-only), the attacker can't mint directly — but they can
**induce** the protocol to mint to the wrong owner by submitting a chip + their address through the
mobile API (`POST /v1/pets` → `/mint`) if the central backend does not verify physical possession of
the chip. First-claim then wins and the legitimate owner is locked out (and must use the V2-C2 claim
path). This is the same root cause as V2-C2 clause 3 but specific to **initial** mint.

**Fix:** (1) Pin `dogTagId` derivation and document it (recommend a protocol-assigned sequential or
`keccak(chip)`-with-collision-handling id, not user-supplied). (2) Gate initial mint on an off-chain
**proof-of-possession** of the microchip (vet/custodian attestation at first registration), not merely
a user claim. (3) Add a Foundry test that a duplicate `dogTagId` mint reverts and that the central
backend rejects a mint for an already-minted chip with a clear error (not a raw revert).

## V2-M5 — `adminRevoke` (mass-revoke for compromised signers) is only a comment, but compromise response depends on it
**Where:** `implementation.md §11.1` `// adminRevoke(bytes32[]) — protocol admin mass-revoke …
(delisting is forward-only)`; `architecture.md §13.3` ("Compromise response therefore requires an
**admin revoke path** over the affected roots (mass-revoke), not just delisting"). Brief Q1 (stale whitelist).

**Why it matters:** The forward-only delisting property (correctly confirmed in V2-M1) means delisting
a compromised signer **does not invalidate roots it already issued**. The documented remedy is admin
mass-revoke — but it exists only as a code comment, with no signature, no auth (it would need to be
admin/`DEFAULT_ADMIN_ROLE`-gated and exempt from the H-1 originator check), and no batching bound. Per
H-1, `revoke` requires `msg.sender == issuedBy[root] || registry.hasRole(0x00, msg.sender)` — so an
admin **can** already revoke via the existing `revoke`/`bulkRevoke` (the `hasRole(0x00,...)` branch).
That's good, but the dedicated `adminRevoke` is unspecified and the team should not ship a comment as
the compromise-response control.

**Fix:** Either delete the `adminRevoke` comment and document that **`bulkRevoke` by `DEFAULT_ADMIN_ROLE`
is the mass-revoke path** (it is, via the H-1 admin branch), or implement `adminRevoke(bytes32[])` as
admin-only and test it. Pick one and write the compromised-key runbook: delist (stop new) +
admin-bulk-revoke (kill issued) + re-issue under fresh signer.

---

# LOW

## V2-L1 — `confirm`'s `signer` param is redundant once `tx.from` is used; remove it to shrink trust surface
After the V2-C1 fix derives `signer` from the transaction, the request-body `signer` becomes
attacker-controlled dead input. Drop it from the `confirm` body entirely; the audit field is set from
`tx.from`. (Defense-in-depth: never accept a security-relevant field from the client that you can read
from a trusted source.)

## V2-L2 — `issuedBy[root]` provenance is lost across burn-and-remint and not surfaced to the ownership pillar
The SBT `profileRoot` and the issuer `issuedBy[root]` are independent. After a burn-and-remint
(V2-C2), the new SBT keeps the same `dogTagId`/`profileRoot`, but there is no on-chain link recording
the prior owner or the claim event beyond `Burned(id)`. For dispute/forensics, emit a
`Claimed(dogTagId, from, to, nonce)` event and retain ownership history off-chain (`ownershipHistory[]`
exists in §9 — bind it to the on-chain claim tx).

## V2-L3 — `preflightWhitelist` `eth_call` is advisory only; a TOCTOU delist between preflight and broadcast still wastes user gas
`preflightWhitelist` (`§11.6`) does `eth_call isWhitelistedFor` to "fail fast", but in wallet mode the
user broadcasts later; an admin `delistFor` between preflight and broadcast makes the on-chain `issue`
revert and the user eats gas. Low impact (rare, small gas), but document that preflight is best-effort
and that `issue`'s `onlyWhitelisted` is the real gate.

---

# INFO / CONFIRMATIONS

## I-1 — Mode-independence of the anchored root holds
`prepare` builds the wrapped doc + merkle root **server-side in both modes** (`§11.6`, arch §6 decisive
rule); only sign+broadcast differs. The `data:issue(root)` calldata is identical regardless of mode, so
"what gets anchored is provably mode-independent" is **true at the build layer**. The residual risk is
entirely in the *confirm* trust (V2-C1/H1/H3/M3), not in the build.

## I-2 — Multi-address whitelist has no cross-issuer confusion *on-chain*
`isWhitelistedFor(recordType, signer)` grants per-(recordType×address). The contract has no notion of
"the same vet" (arch §4.3, impl §11.1 note) — the issuer↔signers grouping is an off-chain view only.
Therefore **no on-chain privilege escalation or cross-issuer confusion** arises from one entity holding
many addresses: each address is independently scoped, and a groomer address whitelisted for grooming
cannot touch VACCINATION/DOG_PROFILE (C-2 scoping intact). The only cross-issuer concern is the
*off-chain* `issuer_signer` mapping integrity (who is allowed to add an address to an entity) — keep
that admin-gated (§13.3 registry-self-write-impossible).

## I-3 — ROAX specifics unchanged and consistent (Q5)
`chainId 135 / 0x87`, `wallet_addEthereumChain` with `chainId:'0x87'` + EIP-3085 params (PLASMA,
RPC, explorer) on error `4902`, EIP-1559-by-default with legacy `gas_price` fallback
(`with_eip1559_or_legacy`, arch §6.1, impl §1.8/§3.8), wallet-mode gas paid by the user / backend-mode
by the clinic key — all **consistent across arch §6 and impl §3.8/§11.6** and unchanged from v1.
`evm_version=paris` still pinned (M-4) in `foundry.toml` §8. No regression. (Reorg/finality at confirm
is the only ROAX-adjacent gap — V2-H1.)

---

# Regression review — v1 criticals/highs after v2 edits

| v1 finding | Status in v2 | Evidence |
|---|---|---|
| **C-1** `_disableInitializers()` on `DogTagIssuer` impl | **INTACT** | `§11.1` `constructor(){ _disableInitializers(); }`; `initialize` requires `reg!=address(0)`. arch §13.1 unchanged. |
| **C-2** per-recordType + dedicated profile role scoping | **INTACT & reinforced** | `§11.1` `IssuerRegistry._wl[rt][s]` + `isWhitelistedFor`; `DogTagIssuer.onlyWhitelisted` uses `isWhitelistedFor(recordType,...)`; `DogTagSBT.onlyProfileIssuer` uses dedicated `PROFILE_ISSUER_ROLE`. v2 multi-address whitelist **builds on** this (per-address×recordType), does not weaken it (see I-2). arch §13.1/§13.5. |
| **H-1** originator binding on revoke / profile writes | **INTACT** | `§11.1` `issuedBy[r]=msg.sender` on issue; `revoke` requires `msg.sender==issuedBy[r] || admin`. Profile writes are `onlyProfileIssuer`. *Caveat:* M-3 issue-front-run still unfixed (V2-M2) and `issuedBy` not carried across remint (V2-L2). |
| **H-2** admin-only burn, no owner self-burn | **INTACT** | `§11.1` `burn` requires `registry.hasRole(0x00, msg.sender)`; emits `Burned`. *New v2 surface:* the burn is now part of the unspecified claim/remint flow — see V2-C2 (the burn auth is fine; the *remint authorisation* is the gap). |
| **H-3** admin hardening (two-step + delay, duty split, multisig) | **INTACT** | `§11.1` `AccessControlDefaultAdminRules(3 days, adminMultisig)` + separate `WHITELIST_ADMIN`. arch §13.1. |
| **M-1** permissioned factory + deterministic salt | **INTACT** | `§11.1` `createIssuer(... ) onlyRole(ADMIN)`, `salt=keccak256(abi.encode(recordType,business))`. |
| **M-3** issue front-run / leaf-binding | **NOT fixed (pre-existing), now more reachable** | v2 hands the unsigned `issue(root)` tx to the browser (wallet mode), widening who sees `root` pre-broadcast. Adopt leaf-binding (V2-C1 #3 / V2-M2). |
| **M-4** evm_version=paris + N-confirmation verify | **PARTIAL REGRESSION** | `verify` keeps `confirmations=5` (`§11.3`), but the **new** `confirm` write path has **0-confirmation** finality (V2-H1). The finality discipline was not extended to the v2 endpoint. |

**Net:** No v2 edit **contradicts or undoes** the v1 C-1/C-2/H-1/H-2/H-3/M-1 remediations — they are
all present in the canonical `§11.1` artifact. Two regressions are *omissions in the new v2 surface*,
not reversals of v1 fixes: (a) M-4's confirmation-depth discipline was not applied to the new
`confirm` write (V2-H1), and (b) M-3's still-open front-run is now easier to trigger via the
wallet-mode unsigned-tx hand-off (V2-M2). **Caveat on the doc itself:** `implementation.md §2` still
ships the *pre-remediation* contract bodies (global `isWhitelisted`, no `_disableInitializers`, no
`issuedBy`, owner-callable-shaped `burn` absent) alongside the corrected `§11.1`. §11 declares it
overrides §1–§9, but shipping both invites a coder copying §2. Recommend deleting/clearly marking §2
as superseded.

---

# Recommended Foundry / integration test additions (v2)
- `confirm` rejects a `txHash` whose `tx.from` is not currently `isWhitelistedFor(recordType)` (V2-C1).
- `confirm` rejects a `RootIssued` log emitted by a non-registry contract address (V2-H3).
- `confirm` rejects a stale/foreign `txHash` whose root matches but `tx.input != prepared.data` /
  `tx.to != prepared.to` (V2-C1/M3).
- `confirm` waits N confirmations; a reorg drops the record back to `prepared` (V2-H1).
- `claim(dogTagId, newOwner, nonce, deadline, sig)`: EIP-712 sig by `newOwner` required; wrong-chain /
  replayed / non-destination sig reverts; nonce monotonic (V2-C2).
- duplicate `dogTagId` mint reverts; central rejects mint for already-minted chip (V2-M4).
- `bulkRevoke` by `DEFAULT_ADMIN_ROLE` revokes a compromised signer's roots (mass-revoke path, V2-M5).
- mode-switch: submit blocked until new address `isWhitelistedFor` true; old delisted only after
  new live (V2-M1).
- regression: C-1/C-2/H-1/H-2/H-3/M-1 Foundry tests from audit-01 still pass against `§11.1`.

---

# Verdict
**Not deployment-ready:** the v2 confirm path has a Critical replay/provenance gap (V2-C1) and the
burn-and-remint "transfer" is a Critical unspecified, replay-able signature scheme (V2-C2); fix these
plus the reorg-at-confirm High (V2-H1), the lost-key ownership-pillar High (V2-H2), and the
event-spoofing High (V2-H3) before any ROAX deploy — but the v1 C-1/C-2/H-1/H-2/H-3 remediations are
intact and the multi-address whitelist + server-side-build mode-independence are sound.
