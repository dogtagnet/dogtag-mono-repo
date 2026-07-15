// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {ERC721} from "@openzeppelin/contracts/token/ERC721/ERC721.sol";
import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {AccessControlEnumerable} from "@openzeppelin/contracts/access/extensions/AccessControlEnumerable.sol";
import {
    AccessControlDefaultAdminRules
} from "@openzeppelin/contracts/access/extensions/AccessControlDefaultAdminRules.sol";
import {IERC5192} from "./IERC5192.sol";

/// @title DogTagSBTConsent - the Level-B owner-unlinkable pet identity (M5 custodial issuance).
/// @notice Every tag is minted to ONE neutral, immutable `custodian`. The owner's wallet NEVER appears
/// on-chain - not as `to`, not as `msg.sender`, not in storage, not in an event. Ownership is proven in-ZK
/// against `profileRoot` (the per-tag M1-engine tree root), never by an `ownerOf` storage read.
///
/// Spec: `dogtag-zkverify-z2/level-b-spec.md` §"Issuance (M5, custodial - absorbs #39)".
///
/// @dev DELIBERATELY SEPARATE from the Level-A `DogTagSBT` (live on ROAX at 0x1FB89865…, still serving the
/// Level-A ECDSA + ZK flow until M7 cuts the apps over). Level-A is FROZEN, not edited - the same pattern
/// M2 used for `verification.circom`, M3 for `Groth16Verifier` and M4 for `VerificationRegistry`. A fresh
/// contract (rather than an edit) is what lets `profileRoot` be write-once here without a migration that
/// would orphan live Level-A state. Differences from Level-A, all forced by the owner-blind model:
///
///   - **Structural custodial mint (D1).** `mintCustodial(id, root)` takes NO `to`: it always mints to the
///     immutable `custodian`. Level-A's `mint(to, id, root)` let the caller pass ANY address - under D1
///     that is exactly the mistake to make impossible, so the parameter is gone rather than merely
///     documented. There is no code path on this contract that mints anywhere else.
///   - **Write-once `profileRoot` (closes the M4 `setProfileRoot` hijack).** `profileRoot[id]` is set once,
///     at mint, and can never change. Level-A's `setProfileRoot` is REMOVED, not just re-gated. See the
///     hijack note below - this is the whole reason a fresh SBT is deployed.
///   - **No `recover` / `Recovered` (D3, spec §"Recovery (M6)").** Recovery is a fresh custodial issuance:
///     new tree, new `R`, new tag. Level-A's signature-authorized rebind (and its `RECOVERY_ROLE`,
///     `recoverNonce`, EIP-712 typehashes, and `_inRecovery` soulbound bypass) are all gone - a rebind
///     names the new owner ON-CHAIN, which is precisely what Level-B removes. Dropping the bypass also
///     makes the soulbound lock ABSOLUTE: `_update` now permits only mint and burn, with no exception.
///
/// ### Why `profileRoot` is write-once
/// Under Level-B, `R == profileRoot(dogTagId)` is the SOLE tag↔owner binding - M4 correctly deleted the
/// `ownerOf`/`keyOf` identity checks. That makes a MUTABLE `profileRoot` a full forgery vector: anyone able
/// to overwrite it could fold a tree whose three reserved owner leaves they control, `issue(R2)` it through
/// a whitelisted clone, repoint a victim tag at `R2`, and record a `Verified` for a pet they do not own.
/// Level-A survived a mutable root only because it ALSO required `ownerOf == subject` and
/// `keyOf(subject) == keyHash`, both owner-controlled; Level-B has neither by design.
///
/// Re-gating was considered and rejected. A `status == Active` gate does not fire (a hijacker targets an
/// Active tag). Restricting to `issuerOf[id]` still lets a compromised issuer silently repoint a REAL
/// owner's tag it once issued. Only write-once removes the vector rather than narrowing it.
///
/// The cost is deliberate and bounded: a tag's tree is FROZEN at issuance, so ANY change to it - a rotated
/// key, a corrected attribute, a new weight - is a fresh custodial issuance under a new `dogTagId`, exactly
/// as D3 already mandates for recovery. `R` changing means old proofs stop verifying, which is the intended
/// Level-B semantics, not a regression. `mintCustodial` therefore REQUIRES a non-zero root: a tag is never
/// created in a rootless state that some later call would have to seal.
contract DogTagSBTConsent is ERC721, AccessControlEnumerable, AccessControlDefaultAdminRules, IERC5192 {
    enum Status {
        Active,
        Lost,
        TransferPending,
        Deceased,
        Revoked
    }

    /// @dev Two-step admin handover timelock (matches Level-A / `IssuerRegistry`; §13.1 H-3).
    uint48 public constant ADMIN_TRANSFER_DELAY = 3 days;

    bytes32 public constant ISSUER_ROLE = keccak256("ISSUER");
    /// @notice Cross-issuer status/lifecycle override. NOTE: unlike Level-A, this role has NO power over
    /// `profileRoot` - the root is write-once, so there is nothing for an authority to overwrite. That is
    /// the structural close of the hijack (an AUTHORITY holder was one of its two vectors).
    bytes32 public constant AUTHORITY_ROLE = keccak256("AUTHORITY");

    /// @notice The single neutral holder of every Level-B tag. IMMUTABLE by design: it is the one address
    /// `ownerOf` can ever return, so it carries no owner information and nothing can repoint it.
    /// @dev The custodian never signs and never acts: tags are soulbound (`locked() == true`) and this
    /// contract has no transfer path, so custody confers no authority and a custodian key compromise grants
    /// an attacker nothing. It is a neutral sink, not an actor - which is why it needs no rotation path.
    address public immutable custodian;

    mapping(uint256 => address) public issuerOf; // immutable, set at mint
    /// @notice Write-once: set at mint, never mutable. There is deliberately no setter. See the contract
    /// notice - a mutable root is a forgery vector once the `ownerOf` identity check is gone.
    mapping(uint256 => bytes32) public profileRoot;
    mapping(uint256 => Status) public status;

    /// @notice Owner-blind (spec §"Owner-blind events": `Issued` is unchanged - it never named the owner).
    event Issued(uint256 indexed dogTagId, address indexed issuer);
    event StatusChanged(uint256 indexed dogTagId, Status from, Status to, address by, string reason);
    event Burned(uint256 indexed dogTagId);

    error Soulbound();
    error NotIssuerOrAuthority();
    error Terminal();
    error BadRoot();

    /// @param admin The protocol multisig; receives `DEFAULT_ADMIN_ROLE` under the two-step timelock.
    /// @param custodian_ The neutral custodian every tag is minted to. MUST NOT be an address associated
    /// with any pet owner - it is the one address `ownerOf` ever returns, and the unlinkability of every
    /// tag on this contract rests on it being owner-independent.
    constructor(address admin, address custodian_)
        ERC721("DogTag", "DTAG")
        AccessControlDefaultAdminRules(ADMIN_TRANSFER_DELAY, admin)
    {
        require(custodian_ != address(0), "zero custodian");
        custodian = custodian_;
    }

    modifier issuerOrAuthority(uint256 id) {
        if (msg.sender != issuerOf[id] && !hasRole(AUTHORITY_ROLE, msg.sender)) {
            revert NotIssuerOrAuthority();
        }
        _;
    }

    /// @notice Mint a soulbound Level-B tag to the neutral custodian and bind its tree root - the whole of
    /// spec §"Issuance" steps 1 and 3, atomically.
    /// @dev There is NO `to` parameter: `custodian` is the only possible recipient, so an issuer CANNOT
    /// mint to an owner's wallet even by mistake. The owner's wallet is not an argument to this function,
    /// so it appears in neither the calldata nor the resulting state.
    ///
    /// Setting the root AT mint satisfies the spec's "issuer sets `profileRoot(dogTagId) = R`" (step 3)
    /// while keeping it write-once: the owner's app builds the tree and supplies `R` BEFORE issuance
    /// (register-first), so there is never a window in which the tag exists without its root.
    ///
    /// CALLER OBLIGATION - the root must ALSO be `issue(R)`d into a `DogTagIssuer` clone. This contract
    /// deliberately does not do it: `DogTagIssuer.issue` is `onlyWhitelisted` on the caller, and the two
    /// writes are gated by different authorities. `VerificationRegistryConsent` resolves the issuing clone
    /// FROM the root (`rootIssuer[R]` → `isValid(R)`) to keep `revoke(R)` working, so a tag minted here
    /// whose root was never issued reverts `unknown root` on EVERY verify. Issue the root, then mint.
    ///
    /// @param id The dogTagId - a non-personal identifier, allocated off-chain, NEVER a microchip hash.
    /// @param root The per-tag M1-engine tree root `R`. Non-zero and permanent: to change it, issue a new tag.
    function mintCustodial(uint256 id, bytes32 root) external onlyRole(ISSUER_ROLE) {
        if (root == bytes32(0)) revert BadRoot(); // never create a tag in a rootless state
        _safeMint(custodian, id); // reverts on a duplicate id
        issuerOf[id] = msg.sender;
        status[id] = Status.Active;
        profileRoot[id] = root; // write-once: no setter exists
        emit Locked(id);
        emit Issued(id, msg.sender);
    }

    /// @notice Status transitions; Deceased/Revoked are terminal. No owner can ever call this - under D1
    /// there is no owner on this contract to call it.
    function setStatus(uint256 id, Status s, string calldata reason) external issuerOrAuthority(id) {
        Status f = status[id];
        if (f == Status.Deceased || f == Status.Revoked) revert Terminal();
        status[id] = s;
        emit StatusChanged(id, f, s, msg.sender, reason);
    }

    /// @notice GDPR-erasure ONLY (admin). Level-B stores no owner link to erase, but burning still fails
    /// the tag closed at verify: `VerificationRegistryConsent` calls `ownerOf` purely as an existence gate,
    /// and OZ's `ownerOf` reverts on a burned token. Note `profileRoot[id]` deliberately SURVIVES the burn
    /// (that mapping is never cleared), which is exactly why that existence gate is load-bearing.
    function burn(uint256 id) external onlyRole(DEFAULT_ADMIN_ROLE) {
        _burn(id);
        emit Burned(id);
    }

    function locked(uint256) external pure returns (bool) {
        return true;
    }

    /// @dev Soulbound, ABSOLUTELY: mint (from == 0) and burn (to == 0) only. Level-A carried an
    /// `_inRecovery` bypass for its rebind path; D3 removes rebind, so the bypass is gone with it and no
    /// holder→holder transfer is expressible on this contract.
    function _update(address to, uint256 id, address auth) internal override returns (address) {
        address from = _ownerOf(id);
        if (from != address(0) && to != address(0)) revert Soulbound();
        return super._update(to, id, auth);
    }

    function supportsInterface(bytes4 i)
        public
        view
        override(ERC721, AccessControlEnumerable, AccessControlDefaultAdminRules)
        returns (bool)
    {
        return i == 0xb45a3c0e || super.supportsInterface(i); // ERC-5192
    }

    // ---- multi-base override plumbing: route role bookkeeping through BOTH the enumerable set and the
    // default-admin rules. `super` resolves to AccessControlDefaultAdminRules first (it enforces the
    // two-step DEFAULT_ADMIN handover), which then chains into AccessControlEnumerable's set bookkeeping. ----
    function grantRole(bytes32 role, address account)
        public
        override(AccessControl, IAccessControl, AccessControlDefaultAdminRules)
    {
        super.grantRole(role, account);
    }

    function revokeRole(bytes32 role, address account)
        public
        override(AccessControl, IAccessControl, AccessControlDefaultAdminRules)
    {
        super.revokeRole(role, account);
    }

    function renounceRole(bytes32 role, address account)
        public
        override(AccessControl, IAccessControl, AccessControlDefaultAdminRules)
    {
        super.renounceRole(role, account);
    }

    function _setRoleAdmin(bytes32 role, bytes32 adminRole)
        internal
        override(AccessControl, AccessControlDefaultAdminRules)
    {
        super._setRoleAdmin(role, adminRole);
    }

    function _grantRole(bytes32 role, address account)
        internal
        override(AccessControlEnumerable, AccessControlDefaultAdminRules)
        returns (bool)
    {
        return super._grantRole(role, account);
    }

    function _revokeRole(bytes32 role, address account)
        internal
        override(AccessControlEnumerable, AccessControlDefaultAdminRules)
        returns (bool)
    {
        return super._revokeRole(role, account);
    }
}
