# Audit 01 — DogTag Smart Contracts & On-Chain Design

> Scope: SMART CONTRACT and on-chain design ONLY (Solidity, OZ v5, ROAX chainId 135).
> Out of scope: off-chain SDK, backends, custody, JWT/QR, calendar — except where they change an on-chain trust assumption.
> Sources audited: `docs/architecture.md` §4–§6, `docs/implementation.md` §2 (contract bodies), §3.3, §8, and `docs/research/03-chain-contracts.md`.
> Date: 2026-06-17. Auditor: contract security review.

The contract bodies under audit are the pseudocode/Solidity in **implementation.md §2.1–§2.5** (these are written as real Solidity and are the canonical artifact); research/03 is the design reference and differs from the implementation in several material ways (flagged below).

---

## Severity legend
- **Critical** — direct loss/forgery of credentials, auth bypass, or unrecoverable contract state reachable by an external party.
- **High** — privilege escalation among trusted-but-scoped parties, or a missing control that breaks a stated security property.
- **Medium** — exploitable under specific conditions, DoS, or spec/implementation divergence that will cause real bugs.
- **Low** — hardening / defense-in-depth.
- **Info** — observations, future-proofing, no action strictly required for v1.

---

## CRITICAL

### C-1 — `DogTagIssuer` implementation is never `_disableInitializers()`d → impl and re-init attack surface
**Where:** `implementation.md` §2.2 (`DogTagIssuer` has no constructor), §2.5 step 2 ("deploy DogTagIssuer impl (uninitialized)"), §4.5 architecture; research/03 §2.3 explicitly notes "clones have no constructor … guard `initialize()`".

**Why it matters:** The implementation contract is deployed bare. `initialize()` is guarded by OZ `initializer`, but `initializer` only prevents *re-entry within the same deployed instance*. Because the impl is never initialized at deploy time and `_disableInitializers()` is never called, **anyone can call `initialize()` directly on the implementation address** and set `registry` to an attacker-controlled contract. While the impl holds no per-credential value itself, this is the canonical EIP-1167/initializer footgun and has two concrete consequences here:
1. The impl becomes a fully-functional issuer pointing at an attacker registry; an indexer or explorer-driven integration that ever treats the verified implementation address as a live store (a realistic mistake, since the impl is the *only* contract verified on Blockscout — research/03 §5.4) will read attacker-controlled `isValid`.
2. It sets a bad precedent and leaves the door open if the impl is ever made `delegatecall`-reachable.

Note OZ v5 `Initializable` `initializer` does **not** auto-lock the implementation; you must call `_disableInitializers()` in a constructor.

**Fix:** Add to `DogTagIssuer`:
```solidity
constructor() { _disableInitializers(); }
```
This is constructor-only code, does not affect clone runtime, and is the OZ-canonical pattern. Add a Foundry test asserting `initialize()` on the impl reverts with `InvalidInitialization()`.

---

### C-2 — Any whitelisted issuer can issue/revoke roots in **every** clone and overwrite **any** pet's SBT profile (no scoping, no per-record-type or per-business authorization)
**Where:** `implementation.md` §2.2 `modifier onlyWhitelisted(){ require(registry.isWhitelisted(msg.sender)); }`; §2.4 `DogTagSBT.mint`/`setProfileRoot` use the *same* registry whitelist; architecture §4.3/§4.4 claims "each business's issuance is independently revocable/auditable" and §11 "only whitelisted addresses can issue/revoke". research/03 §3.3 even names the gap ("If you want per-record-type signer scoping…").

**Why it matters:** `IssuerRegistry.isWhitelisted` is a single global boolean per signer. Every `DogTagIssuer` clone and the `DogTagSBT` consult the *same* flag. Therefore **any** whitelisted signer (e.g. a small-town groomer approved for grooming records) can:
- call `issue()`/`revoke()`/`bulkRevoke()` on the **Vaccination** clone, the **DogProfile** clone, or any future clone — including **revoking a competitor vet's vaccination roots** (griefing/denial), or issuing roots they were never accredited for;
- call `DogTagSBT.mint(...)` minting arbitrary pet identities, and call `setProfileRoot(anyDogTagId, attackerRoot)` to **overwrite the on-chain profile anchor of any pet that exists**, breaking that pet's profile-integrity verification globally.

This directly contradicts the documented property that issuance is per-business and that the DOG_PROFILE mint is a "DogTag-protocol" function (architecture §3.6 table, §4.2 "Minting gated to whitelisted DogTag-protocol issuers"). A groomer's grooming key == the protocol mint key under this design.

**Fix (layered):**
1. **Separate the SBT-mint/profile authority from the issuer whitelist.** `DogTagSBT.mint`/`setProfileRoot`/`burn` must be gated by a dedicated role (e.g. `PROFILE_ISSUER_ROLE`) held only by protocol signers — not by the generic issuer whitelist. Either give `DogTagSBT` its own `AccessControl` role, or add `registry.isWhitelistedFor(DOG_PROFILE_TYPE, msg.sender)`.
2. **Scope issuer authorization per record type** in the registry: `mapping(bytes32 recordType => mapping(address => bool))` and `isWhitelistedFor(recordType, signer)`; each clone checks `registry.isWhitelistedFor(recordType, msg.sender)` (the clone already stores `recordType`). This makes a grooming key unable to touch vaccination/profile stores and restores the "independently revocable per business/type" property.
3. **`setProfileRoot` and `revoke` must additionally be authorized for the specific token/root's originator** (see H-1) so one issuer cannot overwrite/revoke another's data even within the same record type.

---

## HIGH

### H-1 — `revoke` / `setProfileRoot` have no ownership of the target → cross-issuer tampering within a record type
**Where:** `implementation.md` §2.2 `revoke(bytes32 r)` (only checks `issuedAt!=0 && revokedAt==0`); §2.4 `setProfileRoot(id, root)`.

**Why it matters:** Even after fixing C-2's global scoping, the clone does not record *who* issued a root. Any address authorized for that record type can revoke a root issued by a different business, and any profile-issuer can overwrite any `dogTagId`'s root. Revocation is meant to be the issuer's control over its own credentials; here it is a shared mutable namespace. There is no `issuedBy[root]` mapping, so revoke authority cannot be constrained and the events (`RootRevoked(root, msg.sender, ts)`) are the *only* forensic record of who clobbered what.

**Fix:** Store `mapping(bytes32 => address) issuedBy;` set in `issue()`. In `revoke()` require `msg.sender == issuedBy[root]` (or an admin override role for compromised-key recovery). For `DogTagSBT.setProfileRoot`, store the minting issuer per `dogTagId` and require the same originator, or restrict profile mutation to the protocol admin role.

---

### H-2 — `burn` is specified but **not implemented**; if implemented per the spec it is a griefing primitive
**Where:** architecture §4.2 `function burn(uint256 tokenId) external; // owner/admin`; **absent entirely** from `implementation.md` §2.4 `DogTagSBT`.

**Why it matters:** Two problems. (a) The implementation omits `burn`, so the documented death/error-correction path doesn't exist — a functional gap, and the `_update` burn branch (`to==0`) is unreachable in practice. (b) If `burn` is added as written ("owner/admin", `external`, no body) the access control is ambiguous: an `external` burn callable by the token *owner* lets a pet owner destroy their own SBT identity (and orphan every credential that references `dogTagId`), and "admin" is undefined since `DogTagSBT` has no admin role wired (its constructor only stores `registry`). A burn that anyone-but-not-really can call is a classic mis-scope.

**Fix:** Implement `burn` explicitly with a clear, single authority — recommend a `DEFAULT_ADMIN_ROLE`/protocol-multisig only (death & error correction are protocol operations), emit an event, and document that owner self-burn is *not* permitted (it would orphan referencing credentials). If owner-initiated retirement is desired, route it through the protocol backend, not a direct on-chain owner burn.

### H-3 — Single `DEFAULT_ADMIN_ROLE` controls the whole trust graph; delisting is an unbounded griefing/DoS lever
**Where:** `implementation.md` §2.1 `constructor(address admin){ _grantRole(DEFAULT_ADMIN_ROLE, admin); }`; architecture §4.3, §12 ("Multisig for DEFAULT_ADMIN_ROLE" still an *open item*).

**Why it matters:** A single admin EOA (the deploy script passes `adminMultisig` but that is a deploy-time choice, not enforced) can whitelist/delist any issuer. Compromise of that one key = ability to whitelist a rogue issuer (forge any credential whose DNS the attacker also controls) **and** to delist every legitimate issuer at once (global DoS: all `issue`/`revoke` revert). There is no timelock, no two-step admin handoff, no separation between "can add issuers" and "can remove issuers". Delisting is `O(1)` global by design (a feature for compromise response) but the same property makes it a single-call kill switch.

**Fix:** (1) Require `DEFAULT_ADMIN_ROLE` to be a multisig at deploy and assert the admin is a contract in the deploy script. (2) Split duties: a `WHITELIST_ADMIN_ROLE` for routine onboarding, reserve `DEFAULT_ADMIN_ROLE` for role-admin changes; consider a timelock on grant/delist of issuers. (3) Use OZ v5 `AccessControlDefaultAdminRules` for a two-step, delayed admin transfer. (4) Treat delisting reachability as monitored (alert on `IssuerDelisted`).

---

## MEDIUM

### M-1 — `createIssuer` is permissionless + deterministic-salt front-running / address squatting
**Where:** `implementation.md` §2.3 `createIssuer(...) external returns(address)` (no access control) using `cloneDeterministic(salt)`; §4.5 architecture; research/03 §2.3.

**Why it matters:** `createIssuer` is `external` with no gating. `cloneDeterministic` reverts if the salt is already used (CREATE2 collision). An attacker watching the mempool can **front-run the protocol's `createIssuer(name, recordType, salt)`**, deploying a clone at the predicted address first with an attacker-chosen `name`/`recordType` (initialize args are caller-controlled). The deploy script (§2.5 step 5) then reverts (salt taken) or, worse, the protocol/indexer uses `predictIssuer(salt)` and trusts an address that was initialized by the attacker with the wrong `recordType`/`name`. Because authorization is enforced by the *registry* not the clone, the squatted clone is still a "valid" issuer shell. Also, anyone can spam-create clones, polluting the `IssuerCreated` event space the indexer relies on.

**Fix:** Gate `createIssuer` to the protocol admin (`onlyRole(DEFAULT_ADMIN_ROLE)` via the registry, or an `Ownable` factory). Derive the salt deterministically from `recordType` (and business id) inside the factory so callers cannot choose a colliding/garbage salt, and so the predicted address is bound to its semantic identity. Verify post-deploy that `DogTagIssuer(clone).recordType() == recordType`.

### M-2 — Spec/impl divergence in `revoke()` precondition, and `bulkIssue/bulkRevoke` all-or-nothing reverts
**Where:** `implementation.md` §2.2 `revoke` requires `issuedAt[r]!=0 && revokedAt[r]==0`; research/03 §4.1 `revoke` requires only `revokedAt==0` (allows pre-emptive revoke of an unissued root, OA-style). §2.2 `bulkIssue`/`bulkRevoke` loop calling `issue`/`revoke`.

**Why it matters:** (a) The two documents disagree on whether an unissued root can be revoked. The impl's choice (must be issued first) is defensible but means you **cannot pre-block a known-bad root**, and it differs from the OA semantics research/03 cites — pick one deliberately and write the test. (b) `bulkIssue` reverts the entire batch if *any* root is already issued (the `require(issuedAt[r]==0)` in `issue`). For a batched-anchoring future (the stated reason these exist, architecture §4.4), one duplicate in a 500-root batch fails all 500 and wastes gas — a practical DoS/foot-gun once batching ships. Same for `bulkRevoke` on any already-revoked/un-issued root.

**Fix:** Decide and document the revoke precondition (recommend allowing revoke of issued-or-not, matching OA, so bad roots can be blocked pre-issue). For bulk ops, either document atomic semantics explicitly or add `skipExisting` variants that `continue` instead of revert; emit per-root results.

### M-3 — Front-running / griefing of `issue(root)` (root squatting)
**Where:** `implementation.md` §2.2 `issue` (`require(issuedAt[r]==0)`), §3.3 backend builds root then broadcasts.

**Why it matters:** `root` is a public `bytes32`. The vet backend computes the Merkle root off-chain, then broadcasts `issue(root)`. A different **whitelisted** issuer (post-C-2 fix: only one authorized for that record type, but still >1 vet) who observes the pending tx can front-run `issue(root)` from their own key in the same clone. The honest tx then reverts ("already issued"), and the root is now recorded as issued by the *attacker* (`RootIssued(root, attacker, ts)`), corrupting the audit/forensic trail and—combined with H-1—letting the attacker later `revoke` it. Because the same root is anchored either way, integrity-pillar verification still passes, but issuance provenance is wrong and the legitimate issuer's revoke is blocked.

**Fix:** Bind issuance to identity: store `issuedBy[root] = msg.sender` (see H-1) and, if provenance matters, include the issuer address / `dogTagId` / `recordType` in the leaf so an attacker cannot meaningfully anchor another issuer's root (research/03 §4.2 already suggests `leaf = keccak256(abi.encode(tokenId, recordType, payloadHash))` — adopt it). At minimum, treat front-run as detected via the `issuedBy` mismatch and re-issue under a fresh salt.

### M-4 — `evm_version` inconsistency (`paris` vs `shanghai`) and unverified ROAX fee/finality model gate deployment correctness
**Where:** `implementation.md` §2 header & §8 `evm_version = "paris"`; research/03 §5.1 `evm_version = "shanghai"` (then §6.1 recommends dropping to `paris`); architecture §12 open item; architecture header "RPC was returning 502 … treat liveness as a deploy-time pre-check".

**Why it matters:** The two docs ship different `evm_version` defaults. If `shanghai` is used and ROAX predates it, every deploy emits `PUSH0` and reverts with `invalid opcode` — bricking the deploy. Separately, the **verify read** (pillar 2, `isValid` over RPC, architecture §5) has **no finality/reorg handling**: a credential can read VALID against a freshly-mined `issue` tx that later reorgs out, or read VALID against a `revoke` that hasn't finalized. On an unknown low-liveness chain (502s at design time) this is a real correctness gap for acceptance decisions (e.g. a border/airline check). EIP-1559 vs legacy is handled (`--legacy` fallback) but is also unconfirmed.

**Fix:** Standardize on `evm_version = "paris"` in both docs until Shanghai/PUSH0 is smoke-tested on `devrpc.roax.net` (research/03 §6.3 checklist). For verification, require N-confirmation depth before treating `isValid==true` as final for high-stakes record types, and document the chosen confirmation count + reorg behavior; cache the block number read. Confirm `CREATE2` support (needed by `cloneDeterministic`).

### M-5 — `IssuerRegistry` impl (§2.1) bypasses `AccessControl` role plumbing — `isWhitelisted` and role state can desync
**Where:** `implementation.md` §2.1: extends `AccessControl` but uses a private `mapping(address=>bool) _whitelisted` written directly in `whitelistIssuer`/`delistIssuer`, and `isWhitelisted` reads that mapping. research/03 §3.2 instead uses `grantRole(ISSUER_ROLE, …)` and `hasRole`.

**Why it matters:** The impl inherits `AccessControl` (so `onlyRole(DEFAULT_ADMIN_ROLE)` gates the setters) but stores whitelist state in a *parallel* bespoke mapping rather than as a role. Consequences: (a) `grantRole(ISSUER_ROLE, x)` / `RoleGranted` events (which research/03, indexers, and the admin "whitelist viewer" §5.3 may rely on) do **not** affect `isWhitelisted` — two sources of truth that silently diverge; (b) you lose `AccessControl`'s audited revoke/renounce semantics and enumeration tooling; (c) the §2.1 contract declares `ISSUER_ROLE` nowhere yet research/03 builds the whole model on it. Pick one model. The bespoke-mapping version is simpler and fine *if* the rest of the system (admin viewer, indexer, tests) reads `isWhitelisted`, not `hasRole`.

**Fix:** Choose one source of truth. Either (preferred) implement whitelist *as* a role: `whitelistIssuer => grantRole(ISSUER_ROLE,s)`, `isWhitelisted => hasRole(ISSUER_ROLE,s)` and drop the bespoke mapping; or keep the mapping and stop pretending `ISSUER_ROLE` exists. Ensure the admin portal's "on-chain whitelist viewer" reads the same source.

---

## LOW

### L-1 — `approve` / `setApprovalForAll` not blocked on the soulbound token
**Where:** `implementation.md` §2.4 `DogTagSBT` (no approval overrides). research/03 §1.2 notes approvals are "harmless … but you may also override … for cleanliness".

**Why it matters:** Not exploitable (every transfer path hits `_update` and reverts `Soulbound()`), but a non-transferable token that emits `Approval`/`ApprovalForAll` events is misleading to indexers/wallets and could confuse a future integration that gates on approval. Defense-in-depth.

**Fix:** Override `approve`, `setApprovalForAll`, (and optionally `transferFrom`/`safeTransferFrom`) to `revert Soulbound()`. Keep `_update` as the real guard.

### L-2 — `mint` uses `_safeMint` → `onERC721Received` callback before `profileRoot`/`Locked` state settles
**Where:** `implementation.md` §2.4 `mint`: `_safeMint(to,dogTagId); profileRoot[dogTagId]=root; emit Locked(dogTagId);`.

**Why it matters:** `_safeMint` invokes `to.onERC721Received` *before* `profileRoot` is set and *before* `Locked` is emitted. A contract recipient can reenter `DogTagSBT` (e.g. `setProfileRoot`, or `mint` again under C-2) inside the callback, observing/acting on a half-initialized token (`ownerOf` set, `profileRoot==0`). Low impact for a soulbound token (can't transfer), but it's a reentrancy window and an ERC-5192 ordering nit (Locked should be emitted at/just after mint atomically).

**Fix:** Set `profileRoot[dogTagId]` and emit `Locked` **before** `_safeMint`, or use `_mint` (no callback) since recipients are typically EOAs/custodial wallets, or add `nonReentrant`. Re-evaluate after C-2 (per-role gating reduces the reentry surface).

### L-3 — No zero-value / sanity guards in the impl `issue`/`mint`
**Where:** `implementation.md` §2.2 `issue` (no `root != 0` check, unlike research/03 §4.1 which has `require(root != bytes32(0))`); §2.4 `mint` no `root != 0`, no `to != 0` (the latter is caught by ERC721 mint).

**Why it matters:** Anchoring the zero root, or minting a profile with a zero root, is a silent footgun (a buggy backend that sends `0x0` would mark the zero root issued/valid, and any doc that hashes to a missing/empty tree could match). research/03 included the guard; the impl dropped it.

**Fix:** Add `require(root != bytes32(0))` to `issue` and `mint`/`setProfileRoot`.

---

## INFO

### I-1 — ERC-5192 `supportsInterface` id is correct; `locked` impl is acceptable but unconventional
`supportsInterface` returns true for `0xb45a3c0e` — **correct** ERC-5192 id (matches research/03 §1.1 and the EIP). The impl `locked(uint256) external pure returns(bool){ return true; }` (§2.4) is `pure` and does **not** revert for non-existent tokens; the EIP says queries for unminted/zero-address tokens "throw" and research/03 §1.2 used `_requireOwned(tokenId)`. Returning `true` for a non-existent token is a minor spec deviation (not a vulnerability). Consider `_requireOwned(tokenId); return true;` for strict conformance. The `_update` override itself is **correct**: it permits mint (`from==0`) and burn (`to==0`) and reverts only on real transfers — no transfer-bypass via `safeTransferFrom`, since all paths funnel through `_update` in OZ v5.

### I-2 — Single-record `root == targetHash == leafHash`: batch-vs-single ambiguity is mitigated by domain separators, but verify the boundary
Architecture §3.3/§3.4 length-prefixes leaves (domain `0x00`) and prefixes nodes (`0x01`), and a single-leaf doc's root is the *leaf* hash (0x00-prefixed) while any batched root is a *node* hash (0x01-prefixed). This **prevents** a single-leaf root from ever colliding with a multi-leaf/batch node — good, and better than OA. One residual: the on-chain contract stores a bare `bytes32` with no notion of single vs batched; the *off-chain* verifier (impl §1.7) is solely responsible for `processProof(proof, targetHash) == merkleRoot`. When batching ships, ensure the contract still only ever sees the final root and that an empty-proof single-doc cannot be replayed as a batch leaf elsewhere (binding the leaf to `dogTagId`+`recordType` per M-3/research §4.2 closes this). No on-chain change needed for v1; add a test vector for the single↔batch boundary.

### I-3 — `revoke` is globally effective per clone (good); cross-clone replay of a root is possible but low-impact
Because each clone has independent `issuedAt`/`revokedAt`, the same `bytes32` root can be issued in multiple clones. With per-type scoping (C-2) and leaf-binding (M-3) this is benign, and a revoke in the correct clone *is* globally effective for that record type (verification reads `issuer.documentStore` from the doc). Without leaf-binding, a root revoked in clone A still reads VALID in clone B — but the doc names its own `documentStore`, so the verifier checks the right clone. Acceptable for v1; document that `documentStore` in the wrapped doc is trust-critical and must match the DNS-bound address (architecture §5 pillar 3 already enforces this).

### I-4 — Off-chain↔on-chain trust boundary is by-design open
Per the brief: nothing on-chain stops a correctly-whitelisted vet from anchoring a root for data it shouldn't — that is the intended trust model (accreditation is the off-chain gate, revocation the on-chain remedy). No finding; just confirming the chain provides *integrity + issuance status + revocability*, not *content correctness*. The leverage points that DO matter on-chain are who can be whitelisted (H-3) and that one whitelisting can't act everywhere (C-2/H-1).

---

## Test additions recommended (Foundry)
- `initialize()` on the impl reverts after `_disableInitializers()` (C-1).
- A signer whitelisted-but-not-scoped cannot `issue`/`revoke` on a foreign-type clone, cannot `mint`/`setProfileRoot` (C-2).
- `revoke` by non-originator reverts; `setProfileRoot` overwrite by non-originator reverts (H-1).
- `burn` authority is admin-only; owner self-burn reverts (H-2).
- `createIssuer` reverts for non-admin; salt front-run produces wrong `recordType` detection (M-1).
- `bulkIssue` with one duplicate — assert documented atomic-vs-skip behavior (M-2).
- `issue` front-run: second issuer cannot claim provenance after leaf-binding (M-3).
- `supportsInterface(0xb45a3c0e)==true`; all transfer/safeTransfer paths revert `Soulbound()`; mint+burn succeed (I-1).
- Zero-root `issue`/`mint` reverts (L-3).

---

## Executive summary — all Critical & High findings + fixes

- **C-1 (Critical):** `DogTagIssuer` implementation never calls `_disableInitializers()`, so anyone can `initialize()` the impl and point it at an attacker registry — and the impl is the only address verified on Blockscout. **Fix:** add `constructor(){ _disableInitializers(); }`.
- **C-2 (Critical):** The whitelist is a single global boolean, so *any* whitelisted issuer (even a groomer) can issue/revoke roots in *every* clone and `mint`/`setProfileRoot` on the SBT — overwriting any pet's profile and revoking competitors' credentials; the "protocol-only mint" and "per-business revocable" properties are false. **Fix:** give `DogTagSBT` mint/profile its own protocol-only role; add per-record-type scoping `isWhitelistedFor(recordType, signer)` checked by each clone.
- **H-1 (High):** `revoke`/`setProfileRoot` don't track the originator, so any authorized party can revoke another business's root or overwrite any profile. **Fix:** store `issuedBy[root]`/minting issuer and require originator (or admin) to mutate.
- **H-2 (High):** `burn` is specified but unimplemented, and as specced ("owner/admin", no body) would let an owner orphan their identity / "admin" is unwired. **Fix:** implement `burn` as protocol-admin-only with an event; forbid owner self-burn.
- **H-3 (High):** Single `DEFAULT_ADMIN_ROLE` can whitelist a rogue issuer or delist everyone (global DoS) in one call; multisig is only an open item. **Fix:** enforce multisig admin at deploy, use `AccessControlDefaultAdminRules` (two-step + delay), split whitelist vs role-admin duties, monitor `IssuerDelisted`.

**Overall verdict:** The design is directionally sound (OZ v5 `_update` soulbound is correct, ERC-5192 id is right, the root-only anchoring + off-chain proof model is clean and batch-ready), but the v1 contract bodies are **not deployment-ready**: two Critical authorization/initialization flaws (C-1, C-2) plus three High issues mean any single whitelisted business key currently has protocol-wide write power and the implementation contract is hijackable — remediate C-1, C-2, H-1, H-2, H-3 and add per-type scoping before any ROAX deploy.
