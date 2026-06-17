# 09 — SBT Lifecycle & Granular Authorization (DogTagSBT)

Research to validate the DogTag contract design: a non-transferable ERC-721 + ERC-5192 soulbound
token that is the canonical on-chain identity of a pet, owned by the pet owner's self-custodial
wallet, minted by an accredited issuer (vet/protocol). Goals: break a single whitelist into
granular per-action authorization (CREATE/mint, UPDATE, REVOKE, STATUS), record the issuer of each
token on-chain, and decide when the *original issuer* vs a *different authority/admin* may mutate —
because, e.g., a pet's death may be reported by a vet other than the minter.

Date: 2026-06-17

---

## 1. ERC-5484 — Consensual Soulbound Tokens

**Status:** Final. The core idea: burn authorization is agreed by issuer + receiver *before*
issuance and is **immutable** thereafter, recorded per token.

Exact interface (verbatim from the EIP):

```solidity
interface IERC5484 {
    /// A guideline to standardize burn-authorization's number coding
    enum BurnAuth {
        IssuerOnly, // 0
        OwnerOnly,  // 1
        Both,       // 2
        Neither     // 3
    }

    /// Emitted when a soulbound token is issued.
    /// from = issuer, to = receiver, tokenId, burnAuth chosen at issuance.
    event Issued (
        address indexed from,
        address indexed to,
        uint256 indexed tokenId,
        BurnAuth burnAuth
    );

    /// provides burn authorization of the token id.
    /// @dev unassigned tokenIds are invalid, and queries do throw
    function burnAuth(uint256 tokenId) external view returns (BurnAuth);
}
```

Key rules from the spec:
- `burnAuth` **SHALL be presented to receiver before issuance** and **SHALL be Immutable after
  issuance**.
- Burn rights by enum: `IssuerOnly` (loan-record-like), `OwnerOnly` (paid-membership-like), `Both`
  (credentials), `Neither` (credit-history — nobody can burn).
- SBTs are a strict subset of ERC-721, so existing NFT infra works.

**Notable gap:** ERC-5484 standardizes the *Issued* event (which carries `from` = issuer) but does
**not** define an `issuerOf(tokenId)` view. If you want on-chain readable issuer identity you must
add your own `issuerOf` mapping/getter (ERC-5727 *does* define one — see below). ERC-5484 also has
no profile-update or status concept; it is purely about who can burn.

**Relevance to DogTag:** the BurnAuth-at-mint model is elegant but *too rigid* for us. We need
revocation/decease to potentially be performed by an authority who is **not** the original issuer,
and the set of authorized actors can legitimately change over time (a clinic closes, a regulator
takes over). A value frozen at mint (`IssuerOnly`/`Both`/...) cannot express "original issuer OR any
current authority." We will keep the *spirit* — emit an `Issued`-style event recording the issuer
and the chosen authorization at mint — but back it with a mutable role/registry model.

Source: https://eips.ethereum.org/EIPS/eip-5484

---

## 2. ERC-5192 — Minimal Soulbound NFTs

**Status:** Final. Minimal soulbinding layer over ERC-721. This is the non-transferability
primitive DogTag is built on.

```solidity
interface IERC5192 {
    /// Emitted when the locking status is changed to locked.
    event Locked(uint256 tokenId);
    /// Emitted when the locking status is changed to unlocked.
    event Unlocked(uint256 tokenId);

    /// Returns the locking status of an Soulbound Token
    /// @dev SBTs assigned to zero address are considered invalid, and queries about them do throw.
    function locked(uint256 tokenId) external view returns (bool);
}
```

- Interface id: `0xb45a3c0e`.
- A locked token MUST revert on transfer. Mint locked => emit `Locked(tokenId)`.

**Relevance:** DogTag is permanently locked (`locked()` always true). All transfer functions
(`transferFrom`, `safeTransferFrom`, `approve`, `setApprovalForAll`) revert. Owner-key recovery
therefore cannot be a normal transfer — see §3 and the recommendation.

Source: https://eips.ethereum.org/EIPS/eip-5192

---

## 3. ERC-5727 — Semi-Fungible Soulbound Token (and siblings)

**Status:** Draft / stagnant (Standards Track ERC). Not Final, so treat as a *design vocabulary*,
not a compatibility target. It is the most complete SBT lifecycle model and explicitly composes
ERC-5192 (locking) + ERC-5484 (BurnAuth) + ERC-3525 (slots).

Core lifecycle surface (signatures as published):

```solidity
// issuance (NFT form + credit/fungible form)
function issue(address to, uint256 tokenId, uint256 slot, BurnAuth burnAuth,
               address verifier, bytes calldata data) external;
function issue(uint256 tokenId, uint256 amount, bytes calldata data) external;

// revocation
function revoke(uint256 tokenId, bytes calldata data) external;
function revoke(uint256 tokenId, uint256 amount, bytes calldata data) external;

// verification & authority
function verify(uint256 tokenId, bytes calldata data) external returns (bool);
function verifierOf(uint256 tokenId) external view returns (address);
function issuerOf(uint256 tokenId)  external view returns (address);

// events
event Revoked(address indexed from, uint256 indexed tokenId);
event Verified(address indexed by, uint256 indexed tokenId, bool result);
```

Key reusable concepts:
- **`issuerOf(tokenId)`** — on-chain record of who issued each token. **This is exactly the
  primitive DogTag needs** and is missing from ERC-5484. We adopt it.
- **Issuer / verifier separation** — the party that *creates* a token differs from the party that
  *attests/checks* it. Maps to vet-issuer vs. regulator/authority verifier.
- **Slots** (from ERC-3525) — a category/grouping dimension. Could model species/breed/clinic but
  is overkill for one-token-per-pet; we skip slots.

Extension interfaces worth borrowing ideas from:
- **IERC5727Expirable** — `setExpiration(uint256 tokenId, uint64 expiration, bool isRenewable)`.
  Pattern for time-bounded validity (we likely don't expire a pet identity, but useful for
  authority delegations / guardian grants).
- **IERC5727Recovery** — `recover(address owner, bytes memory signature)`: **owner-signature-driven
  account recovery**. Directly relevant to the lost-key path: the *user* authorizes migration to a
  new address via a signature rather than a transfer.
- **IERC5727Delegate** — `delegate()/undelegate()`: operator permissions scoped per slot. A model
  for scoped delegation of mutation rights.
- **IERC5727Governance** — `requestApproval()/voteApproval()`: multi-party approval before issuance.
- **IERC5727Enumerable** — slot/owner enumeration.

**Relevance:** We reuse `issuerOf` and the issuer/verifier mental model and the
signature-based recovery idea; we do **not** inherit the full (stagnant, heavy, semi-fungible)
interface. DogTag is one-NFT-per-pet, not semi-fungible credit.

Sources: https://eips.ethereum.org/EIPS/eip-5727 ,
https://github.com/ethereum/ercs/blob/master/ERCS/erc-5727.md

---

## 4. ERC-6147 — Guard of NFT/SBT (lost-key / recovery)

**Status:** Final. Extends ERC-721 by separating *holding right* from *transfer right* via a new
**guard** role with an expiry.

```solidity
interface IERC6147 {
    /// @param newGuard can not be zero address (use removeGuard to clear)
    /// @param expires UNIX timestamp; guard may manage the token before expires
    event UpdateGuardLog(uint256 indexed tokenId, address indexed newGuard,
                         address oldGuard, uint64 expires);

    function changeGuard(uint256 tokenId, address newGuard, uint64 expires) external;
    function removeGuard(uint256 tokenId) external;
    function transferAndRemove(address from, address to, uint256 tokenId) external;
    function guardInfo(uint256 tokenId) external view returns (address, uint64);
}
```

Mechanics:
- When **no valid guard** exists: the owner, approved address, or operator may set a guard.
- When a **valid guard** exists: only the guard can change/remove the guard or transfer. The
  owner/operators/approved addresses are *locked out* of changing the guard and transferring while a
  guard is active.
- `expires` is a UNIX timestamp; after it passes the guard reverts to `(address(0), 0)`.
- Use case the EIP explicitly cites: *"When the wallet where SBT is located is stolen or
  unavailable, SBT should be able to be recoverable."* The guard can be a social-recovery /
  multisig / third-party protocol address.

**Critical caveat for soulbound tokens:** ERC-6147's recovery hinges on the guard's
*transfer* power. But a strict ERC-5192 SBT reverts on transfer, so `transferAndRemove` would be
blocked unless the SBT contract specifically exempts guard-initiated transfers from the soulbound
lock. So a guard-based recovery for a true SBT requires a contract-level carve-out: "transfers are
forbidden EXCEPT a guard-initiated recovery transfer." That is implementable but it weakens the
soulbound invariant and makes the guard a powerful trusted party (it can move the identity at will
while active).

**Relevance:** ERC-6147 is a viable recovery primitive, but for non-crypto pet owners the guard
must be a trustworthy custodial recovery service or the issuer/protocol. We compare it against a
burn-and-remint / signature recovery in the recommendation (§7). For DogTag we favor a
**protocol-mediated re-bind authorized by the user**, optionally layering a guard for users who
opt in.

Sources: https://eips.ethereum.org/EIPS/eip-6147 ,
https://eipsinsight.com/ercs/erc-6147

---

## 5. OpenZeppelin v5 AccessControl — granular role design

Two viable shapes for "break the single whitelist into per-action authorization":

### (A) Multiple narrow `bytes32` roles (RBAC)
```solidity
bytes32 public constant MINTER_ROLE  = keccak256("MINTER_ROLE");
bytes32 public constant UPDATER_ROLE = keccak256("UPDATER_ROLE");
bytes32 public constant REVOKER_ROLE = keccak256("REVOKER_ROLE");
bytes32 public constant STATUS_ROLE  = keccak256("STATUS_ROLE"); // authority that can set status
// _grantRole(...) in constructor; onlyRole(X) on each function.
```
- `DEFAULT_ADMIN_ROLE` administers all roles by default; `_setRoleAdmin` builds hierarchy.
- `grantRole`/`revokeRole` gated by each role's admin role.
- `AccessControlEnumerable` adds `getRoleMemberCount`/`getRoleMember`/`getRoleMembers` so the set of
  authorized issuers/authorities is enumerable on-chain (useful for audits / a public "accredited
  issuers" list).
- **Pros:** standard, audited, well-understood, least-privilege, off-the-shelf events
  (`RoleGranted`/`RoleRevoked`), enumerable. **Cons:** roles are global to the contract — a
  MINTER_ROLE holder can mint *any* DogTag; the role does not by itself encode per-token scoping.

### (B) Action-scoped registry: `isWhitelistedFor(bytes32 action, address)`
A custom mapping `mapping(bytes32 action => mapping(address => bool))` with admin-gated setters.
- **Pros:** one mental model ("action whitelists"); easy to add new actions without new role
  constants; can be combined with arbitrary metadata (accreditation expiry, jurisdiction).
- **Cons:** you are re-implementing what AccessControl already does (and must re-derive its safety
  properties, events, enumeration, admin hierarchy). More custom code = more audit surface.

### Recommendation for DogTag
Use **(A) OZ `AccessControlEnumerable` with narrow roles** for the *global* "who may act at all"
question, and add **per-token `issuerOf` + an authority-override rule** for the *per-token* "who may
mutate this specific token" question. RBAC answers "is this address an accredited minter/authority?";
`issuerOf` answers "did *this* address originate *this* token?". The mutation guard is the
conjunction:

```solidity
mapping(uint256 => address) public issuerOf;   // recorded at mint, immutable

modifier onlyIssuerOrAuthority(uint256 tokenId) {
    require(
        msg.sender == issuerOf[tokenId] ||
        hasRole(AUTHORITY_ROLE, msg.sender),
        "not issuer or authority"
    );
    _;
}
```
This gives: *the original issuer may always update/revoke its own tokens; a current authority (a
distinct accredited party / regulator) may also act on any token* — exactly the cross-vet decease
scenario. `AUTHORITY_ROLE` membership is mutable, so authority can change over time without
re-issuing tokens (impossible under ERC-5484's frozen BurnAuth).

`AccessManager` (v5 centralized permissioning) is overkill for a single-contract identity token;
per-contract `AccessControl` is the right altitude.

Sources: https://docs.openzeppelin.com/contracts/5.x/access-control ,
https://github.com/OpenZeppelin/openzeppelin-contracts/blob/master/contracts/access/AccessControl.sol

---

## 6. Lifecycle / status modeling for identity tokens — soft status vs burn

W3C **Bitstring Status List v1.0** (and the older StatusList2021 / VC Status List 2021) is the
canonical credential-status vocabulary. It defines distinct, named status *purposes*:
- **`revocation`** — cancels validity; **"This status is not reversible."** (permanent).
- **`suspension`** — temporarily prevents acceptance; **reversible**.
- **`refresh`** — updated credential available (non-invalidating).
- **`message`** — arbitrary status messages.

Crucially, status is recorded **about** the credential, not by destroying it: *"Status information
is about the verifiable credential itself and might not apply to any underlying or backing
credential."* The credential's claims and proofs **stay cryptographically intact and historically
auditable** even after revocation. A revocation registry is a separate, issuer-maintained structure
listing invalidated credential identifiers — it never deletes the credential.

### Burn vs. soft-status — critical analysis for an identity referenced by other credentials
DogTag's `tokenId` is the canonical pet identity that *other* on-chain/off-chain credentials
reference (vaccination records, ownership history, microchip attestations). If we **burn** the
token:
- The NFT ceases to exist; `ownerOf`/`tokenURI` revert; ERC-721 `Transfer(...,0x0,...)` is the only
  trace. Any credential pointing at that `tokenId` now dangles — verifiers cannot resolve the
  subject, breaking historical verifiability.
- Burn is irrecoverable and loses the *reason* (deceased vs. fraudulent vs. lost) and the audit
  trail of state transitions.

Therefore **prefer soft status over burn** for DogTag. Model an explicit status enum + events; keep
the token in existence so dependent credentials remain resolvable. Mirror the
revocation-vs-suspension distinction (irreversible vs. reversible). Reserve burn for one narrow
case: GDPR-style erasure / mistaken-mint cleanup, gated tightly.

```solidity
enum DogTagStatus {
    Active,             // 0 — normal
    Lost,               // 1 — reversible (owner/authority reports pet lost; can return to Active)
    TransferPending,    // 2 — ownership change in flight (reversible)
    Deceased,           // 3 — terminal, irreversible
    Revoked             // 4 — terminal, irreversible (issued in error / fraud)
}
```
Reversible transitions: `Active <-> Lost`, `Active <-> TransferPending`. Terminal/irreversible:
`Deceased`, `Revoked` (any state -> terminal, never back). Emit a status event on every change with
actor + reason for the audit trail.

Sources: https://www.w3.org/TR/vc-bitstring-status-list/ ,
https://www.w3.org/TR/2023/WD-vc-status-list-20230427/ ,
https://w3c.github.io/vc-bitstring-status-list/

---

## 7. Death / decease handling precedents

From DID/VC and KYC practice:
- The **three-role model** (issuer signs, holder stores, verifier checks) is standard; revocation
  authority typically rests with the **issuer or a designated authority**, and verifiers can
  *initiate* revocation requests (e.g., LinkDID lets a verifier submit a misuse document that causes
  the issuer to invalidate a credential; holders can also voluntarily request revocation with proof
  of ownership).
- **Audit trail is immutable and append-only**: every consent/lifecycle event (grant, modification,
  revocation) is timestamped with cryptographic proof of *who* acted under *which terms*. Records
  are never altered/deleted — superseded, not erased.
- **Revoked is irreversible by definition** (W3C, §6); suspension is the reversible analogue.

For a *deceased* event specifically there is no single canonical EIP, but the precedents converge
on: (a) a **trusted authority** (not necessarily the original issuer) performs the terminal
transition; (b) the action is **logged immutably with actor + timestamp + reason**; (c) the state is
**irreversible**; (d) the record is **retained**, not destroyed, so historical references resolve.

This maps cleanly onto DogTag: a pet's death may be reported by a *different* vet/authority than the
minter, so `Deceased` must be settable by `AUTHORITY_ROLE` (and the original issuer), be terminal,
emit an audited event, and keep the token alive.

Sources: (LinkDID) https://arxiv.org/pdf/2307.14679 ,
https://www.w3.org/TR/vc-bitstring-status-list/ ,
https://www.nadcab.com/blog/patient-consent-management-blockchain

---

## 8. RECOMMENDED FINAL DESIGN — DogTagSBT

### 8.1 Standards posture
- Inherit ERC-721 + **ERC-5192** (permanently locked; `locked()==true`, transfers revert).
- Borrow **`issuerOf(tokenId)`** and issuer/verifier separation from **ERC-5727** (vocabulary only).
- Use OZ v5 **`AccessControlEnumerable`** for global roles.
- Adopt **soft status** (W3C revocation/suspension semantics), not burn, as the lifecycle model.
- Do **not** adopt ERC-5484's frozen mint-time `BurnAuth` — replace with a mutable
  registry + `issuerOf` + authority-override (justification in §1 and §5). Keep the
  `Issued`-style event recording issuer + authorization at mint.

### 8.2 Roles (narrow, least-privilege)
```solidity
bytes32 public constant DEFAULT_ADMIN_ROLE; // OZ built-in — manages roles, emergency
bytes32 public constant ISSUER_ROLE   = keccak256("ISSUER_ROLE");   // accredited vets/protocol: may mint
bytes32 public constant UPDATER_ROLE  = keccak256("UPDATER_ROLE");  // may UPDATE profile (often == issuer)
bytes32 public constant AUTHORITY_ROLE = keccak256("AUTHORITY_ROLE"); // cross-issuer authority/regulator: revoke + status
bytes32 public constant RECOVERY_ROLE = keccak256("RECOVERY_ROLE");  // protocol recovery operator (re-bind)
```
Notes:
- `ISSUER_ROLE` answers "may mint at all." `issuerOf[tokenId]` records *which* issuer minted *which*
  token (the per-token scope RBAC alone can't express).
- `AUTHORITY_ROLE` is the cross-issuer override — required for the decease-by-other-vet scenario.
- Enumerable so the accredited-issuer/authority set is publicly auditable.

### 8.3 Issuer record + authority-override mutation rule
```solidity
mapping(uint256 => address)      public issuerOf;     // set at mint, immutable
mapping(uint256 => DogTagStatus) public statusOf;

modifier onlyIssuerOrAuthority(uint256 tokenId) {
    require(_exists(tokenId), "no token");
    require(msg.sender == issuerOf[tokenId] || hasRole(AUTHORITY_ROLE, msg.sender),
            "not issuer or authority");
    _;
}
```
Rule: **the original issuer may always update/revoke its own tokens; any current AUTHORITY may act
on any token.** Authority membership is mutable, so authority can evolve without re-issuing tokens.

### 8.4 Status model (soft, no burn)
See §6 enum. Transition matrix:
- `Active <-> Lost` — reversible. Settable by owner (self-report) OR issuer OR authority.
- `Active <-> TransferPending` — reversible. Set during owner-key re-bind / ownership change.
- `* -> Deceased` — **terminal, irreversible**. Settable by **AUTHORITY_ROLE or original issuer**
  (NOT owner; a deceased report needs an accredited party). This is the key cross-vet rule.
- `* -> Revoked` — **terminal, irreversible**. Issued-in-error / fraud. AUTHORITY or issuer.
- Token is **never burned** on death/revocation, so credentials referencing `tokenId` still
  resolve. Burn reserved for a separate, admin-only erasure path.

### 8.5 Lost-key recovery — RECOMMENDED: user-signature-authorized re-bind (with optional guard)
Pet owners are non-crypto users; pure self-recovery is unrealistic, and a pure guardian gives a
third party standing power over the identity. Recommended primary path = **ERC-5727-Recovery-style,
user-signature-authorized protocol re-bind**:
1. Owner proves intent off-chain by signing a recovery message (EIP-712) authorizing migration to a
   new address (signature produced once at onboarding and escrowed, or via the protocol's recovery
   flow with KYC). If the old key is fully lost, fall back to attested off-chain identity proof to
   the protocol + `RECOVERY_ROLE` execution, logged immutably.
2. `RECOVERY_ROLE` (or the verified signature) triggers `recover(tokenId, newOwner, sig)`:
   set `TransferPending`, perform a *recovery-only* internal re-bind that is exempt from the
   soulbound lock, update owner, restore `Active`, emit `Recovered`. The soulbound invariant holds
   for all paths except this explicitly gated recovery.
3. **Optional ERC-6147 guard layer** for users who opt in to social recovery: allow setting a guard
   (a multisig / social-recovery contract) and permit a guard-initiated `transferAndRemove`
   carve-out from the lock. This is opt-in because an active guard can move the identity at will.

Prefer signature/protocol re-bind over **burn-and-remint**: burn-and-remint changes `tokenId`,
orphaning every credential that referenced the old id — unacceptable for an identity token (§6).
Re-bind preserves `tokenId` and `issuerOf`, only changing the owner address.

### 8.6 Interface sketch
```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IDogTagSBT /* is IERC721, IERC5192 */ {
    enum DogTagStatus { Active, Lost, TransferPending, Deceased, Revoked }
    enum Authorization { IssuerOnly, Both }  // recorded at mint, ERC-5484-flavored, advisory

    // --- events (audit trail: actor + reason on every transition) ---
    event Issued(address indexed issuer, address indexed to, uint256 indexed tokenId, Authorization auth);
    event ProfileUpdated(uint256 indexed tokenId, address indexed by, string uri);
    event StatusChanged(uint256 indexed tokenId, DogTagStatus indexed from,
                        DogTagStatus indexed to, address by, string reason);
    event Revoked(uint256 indexed tokenId, address indexed by, string reason);
    event Recovered(uint256 indexed tokenId, address indexed oldOwner,
                    address indexed newOwner, address by);
    // plus ERC-5192 Locked(tokenId), OZ RoleGranted/RoleRevoked

    // --- views ---
    function issuerOf(uint256 tokenId) external view returns (address);
    function statusOf(uint256 tokenId) external view returns (DogTagStatus);
    function locked(uint256 tokenId) external view returns (bool); // ERC-5192, always true

    // --- lifecycle ---
    function mint(address to, string calldata uri, Authorization auth)
        external returns (uint256 tokenId);                              // onlyRole(ISSUER_ROLE)
    function updateProfile(uint256 tokenId, string calldata uri) external; // onlyIssuerOrAuthority + UPDATER_ROLE
    function setStatus(uint256 tokenId, DogTagStatus newStatus, string calldata reason) external; // gated per transition
    function markDeceased(uint256 tokenId, string calldata reason) external; // AUTHORITY_ROLE or issuer; terminal
    function revoke(uint256 tokenId, string calldata reason) external;       // AUTHORITY_ROLE or issuer; terminal

    // --- recovery (soulbound carve-out) ---
    function recover(uint256 tokenId, address newOwner, bytes calldata ownerSig) external; // RECOVERY_ROLE / verified sig
    // optional ERC-6147 opt-in:
    function changeGuard(uint256 tokenId, address newGuard, uint64 expires) external;
    function removeGuard(uint256 tokenId) external;
}
```

Modifier sketch for the per-transition gate (deceased = authority, not owner):
```solidity
function markDeceased(uint256 tokenId, string calldata reason)
    external onlyIssuerOrAuthority(tokenId)
{
    DogTagStatus prev = statusOf[tokenId];
    require(prev != DogTagStatus.Deceased && prev != DogTagStatus.Revoked, "terminal");
    statusOf[tokenId] = DogTagStatus.Deceased;          // terminal, irreversible
    emit StatusChanged(tokenId, prev, DogTagStatus.Deceased, msg.sender, reason);
}
```

### 8.7 Why this beats ERC-5484-only
- ERC-5484 freezes burn rights at mint; DogTag needs authority that changes over time and a
  decease performed by a non-minter authority — impossible with frozen `BurnAuth`.
- ERC-5484 burns; DogTag must retain the token so referencing credentials stay verifiable (W3C
  soft-status precedent).
- We still honor ERC-5484's good idea: emit an `Issued` event capturing issuer + authorization at
  mint for the audit trail; we just make the *enforcement* dynamic via roles + `issuerOf`.

---

## Sources
- ERC-5484 Consensual Soulbound Tokens — https://eips.ethereum.org/EIPS/eip-5484
- ERC-5192 Minimal Soulbound NFTs — https://eips.ethereum.org/EIPS/eip-5192
- ERC-5727 Semi-Fungible Soulbound Token — https://eips.ethereum.org/EIPS/eip-5727 ,
  https://github.com/ethereum/ercs/blob/master/ERCS/erc-5727.md
- ERC-6147 Guard of NFT/SBT — https://eips.ethereum.org/EIPS/eip-6147 ,
  https://eipsinsight.com/ercs/erc-6147
- OZ v5 AccessControl — https://docs.openzeppelin.com/contracts/5.x/access-control ,
  https://github.com/OpenZeppelin/openzeppelin-contracts/blob/master/contracts/access/AccessControl.sol
- W3C Bitstring Status List v1.0 — https://www.w3.org/TR/vc-bitstring-status-list/ ;
  VC Status List 2021 — https://www.w3.org/TR/2023/WD-vc-status-list-20230427/
- LinkDID (verifier/holder-initiated revocation, key recovery) — https://arxiv.org/pdf/2307.14679
- Patient consent / immutable audit trail — https://www.nadcab.com/blog/patient-consent-management-blockchain
