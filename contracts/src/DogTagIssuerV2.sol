// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Initializable} from "@openzeppelin/contracts/proxy/utils/Initializable.sol";
import {Ownable, Ownable2Step} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {IssuerRegistry} from "./IssuerRegistry.sol";

/// @notice The protocol-global write-once root->clone index (impl §11.10(a), architecture §13.9).
/// Generation 2's index is `DogTagIssuerFactoryV2`; the shape is unchanged from generation 1.
interface IRootIndex {
    function registerRoot(bytes32 root) external;
}

/// @title DogTagIssuerV2 — per-record-type anchoring contract, now with a real owner.
///
/// @notice Generation-2 successor to `DogTagIssuer`. Everything about anchoring is unchanged:
/// `issue`/`revoke`/`isValid` have identical semantics and identical event signatures, so an existing
/// verifier, indexer decoder or explorer reads a generation-2 clone exactly as it reads a generation-1
/// clone. The single addition is **ownership**, and it exists because generation-1 clones have none.
///
/// # Why an owner at all
///
/// `DogTagIssuer` is `Initializable` only. It has no owner, no admin and no controller — every write is
/// gated on `IssuerRegistry.isWhitelistedFor(recordType, msg.sender)` and nothing else. So the question
/// "who controls this contract?" has no on-chain answer, and the captain's requirement that only
/// *"whitelisted people, AND owner of contracts"* may publish a DNS claim is not merely unimplemented —
/// it is **unimplementable**, because there is no owner to check. `IssuerDomainRegistry` had to
/// substitute a proxy for it (`_isSpawningBusiness`, recomputing the clone's deterministic address from
/// the salt), and that proxy authorizes whoever happened to be passed as `business` at creation — which
/// in every clone deployed to date is the operator's own signer, not the organisation.
///
/// This contract replaces that proxy with the real thing: a checkable, transferable `owner()`.
///
/// # Ownership is CONTROL, not an issuance capability — the two must not merge
///
/// The owner may not issue, may not revoke, and gains no privilege over roots. Issuance stays gated
/// solely on the registry whitelist, for a property this codebase depends on (plan §3.3): **delisting a
/// signer must stop the next `issue` and touch nothing already anchored.** If ownership carried an
/// issuance right, delisting would no longer stop issuance and the delist lever would be silently dead.
///
/// So a delisted provider still owns its clone, may still transfer it and may still repoint its listing —
/// it simply cannot anchor anything new. That separation is deliberate and is pinned by
/// `test_delisting_stops_issuance_without_stripping_ownership`.
///
/// `revoke` likewise keeps generation 1's authority exactly (the H-1 originator, or the registry's
/// protocol admin). Extending revocation to the clone owner would let an owner revoke credentials it did
/// not issue, which is a distinct governance decision and is deliberately not taken here.
///
/// # The handover is TWO-STEP, and `owner()` can never become zero
///
/// A single-step transfer to a mistyped address strands the role forever: the clone would keep issuing
/// but could never again claim a domain, be repointed, or be handed to its real controller. So transfer
/// is OZ `Ownable2Step` — `transferOwnership` records a pending owner, `acceptOwnership` completes it,
/// and `transferOwnership(address(0))` cancels a pending transfer. Nothing changes until the recipient
/// proves it can transact.
///
/// Two paths could otherwise zero the owner, and both are closed here rather than left to convention:
///
///   * `renounceOwnership` is **disabled**. An ownerless clone is precisely the generation-1 state this
///     contract exists to end, and OZ's default would let one transaction re-enter it irreversibly.
///   * `acceptOwnership` refuses `msg.sender == address(0)`. OZ's implementation compares
///     `pendingOwner() != msg.sender`; with no transfer pending both sides are the zero address, so the
///     comparison passes and ownership transfers to zero. That is unreachable in practice (nobody can
///     sign as the zero address) but it is unreachable by EVM accident rather than by contract, and the
///     invariant is worth stating in code where a test can hold it.
///
/// Net: after `initialize`, `owner()` is non-zero forever. Pinned by
/// `test_owner_can_never_become_the_zero_address`.
contract DogTagIssuerV2 is Initializable, Ownable2Step {
    IssuerRegistry public registry;
    IRootIndex public rootIndex; // the factory (write-once rootIssuer index)
    bytes32 public recordType;
    string public name;

    mapping(bytes32 => uint256) public issuedAt; // 0 = not issued
    mapping(bytes32 => uint256) public revokedAt; // 0 = not revoked
    mapping(bytes32 => address) public issuedBy; // H-1 originator

    /// @dev Byte-identical to generation 1's, so every existing decoder reads a generation-2 clone.
    event RootIssued(bytes32 indexed root, address indexed by, uint256 ts);
    event RootRevoked(bytes32 indexed root, address indexed by, uint256 ts);

    error NotWhitelisted();
    error BadRoot();
    error NotOriginatorOrAdmin();
    /// @dev Raised by the two paths that would leave the clone ownerless. See the contract doc.
    error OwnerCannotBeZero();

    /// @dev The implementation is locked at construction; clones initialize. `Ownable` rejects a zero
    /// initial owner, so the implementation is owned by its deployer — a value that never matters,
    /// because `_disableInitializers()` makes the implementation permanently uninitializable and it
    /// holds no roots.
    constructor() Ownable(msg.sender) {
        _disableInitializers(); // C-1: lock the implementation (clones initialize)
    }

    modifier onlyWhitelisted() {
        if (!registry.isWhitelistedFor(recordType, msg.sender)) revert NotWhitelisted();
        _;
    }

    /// @param owner_ the clone's controller. Set once, here, and thereafter only reachable through the
    /// two-step handover. The factory passes the creating provider, so there is no path by which a
    /// freshly created clone is owned by anyone but its creator.
    function initialize(string calldata n, bytes32 rt, address reg, address index, address owner_)
        external
        initializer
    {
        require(reg != address(0) && index != address(0), "zero");
        if (owner_ == address(0)) revert OwnerCannotBeZero();
        name = n;
        recordType = rt;
        registry = IssuerRegistry(reg);
        rootIndex = IRootIndex(index);
        _transferOwnership(owner_);
    }

    // ---------------------------------------------------------------------------------------------
    // Ownership — two-step, and terminal in the sense that it can never be vacated
    // ---------------------------------------------------------------------------------------------

    /// @notice Disabled. A clone with no owner cannot claim a domain, be repointed, or be handed over —
    /// which is the generation-1 defect this contract exists to fix, so it must not be re-enterable.
    function renounceOwnership() public pure override {
        revert OwnerCannotBeZero();
    }

    /// @notice Complete a pending handover. Refuses the zero address, closing OZ's
    /// `pendingOwner() == msg.sender == address(0)` coincidence (see the contract doc).
    function acceptOwnership() public override {
        if (msg.sender == address(0)) revert OwnerCannotBeZero();
        super.acceptOwnership();
    }

    // ---------------------------------------------------------------------------------------------
    // Anchoring — unchanged from generation 1
    // ---------------------------------------------------------------------------------------------

    function issue(bytes32 r) public onlyWhitelisted {
        if (r == bytes32(0) || issuedAt[r] != 0) revert BadRoot();
        issuedAt[r] = block.timestamp;
        issuedBy[r] = msg.sender;
        rootIndex.registerRoot(r); // write-once rootIssuer[r] = this clone (§11.10(a))
        emit RootIssued(r, msg.sender, block.timestamp);
    }

    function revoke(bytes32 r) public onlyWhitelisted {
        if (issuedAt[r] == 0 || revokedAt[r] != 0) revert BadRoot();
        // H-1: only the original issuer OR the protocol admin (registry DEFAULT_ADMIN) may revoke.
        // Deliberately NOT the owner — see the contract doc.
        if (msg.sender != issuedBy[r] && !IAccessControl(address(registry)).hasRole(0x00, msg.sender)) {
            revert NotOriginatorOrAdmin();
        }
        revokedAt[r] = block.timestamp;
        emit RootRevoked(r, msg.sender, block.timestamp);
    }

    function bulkIssue(bytes32[] calldata rs) external onlyWhitelisted {
        for (uint256 i; i < rs.length; i++) {
            issue(rs[i]);
        }
    }

    function bulkRevoke(bytes32[] calldata rs) external onlyWhitelisted {
        for (uint256 i; i < rs.length; i++) {
            revoke(rs[i]);
        }
    }

    /// @notice Admin mass-revoke for a compromised signer (delisting is forward-only — §13.3).
    /// Bypasses originator binding; gated by the registry's protocol admin only.
    function adminRevoke(bytes32[] calldata rs) external {
        require(IAccessControl(address(registry)).hasRole(0x00, msg.sender), "!admin");
        for (uint256 i; i < rs.length; i++) {
            bytes32 r = rs[i];
            if (issuedAt[r] != 0 && revokedAt[r] == 0) {
                revokedAt[r] = block.timestamp;
                emit RootRevoked(r, msg.sender, block.timestamp);
            }
        }
    }

    function isIssued(bytes32 r) external view returns (bool) {
        return issuedAt[r] != 0;
    }

    function isRevoked(bytes32 r) external view returns (bool) {
        return revokedAt[r] != 0;
    }

    function isValid(bytes32 r) external view returns (bool) {
        return issuedAt[r] != 0 && revokedAt[r] == 0;
    }
}
