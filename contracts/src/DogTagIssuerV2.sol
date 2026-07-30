// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Initializable} from "@openzeppelin/contracts/proxy/utils/Initializable.sol";
import {Ownable, Ownable2Step} from "@openzeppelin/contracts/access/Ownable2Step.sol";

/// @notice The protocol-global write-once root->clone index (impl §11.10(a), architecture §13.9).
/// Generation 2's index is `DogTagIssuerFactoryV2`; the shape is unchanged from generation 1.
interface IRootIndex {
    function registerRoot(bytes32 root) external;
}

/// @dev The generation-2 authority core — the S-6 `ProviderRegistry`. These four functions are the
/// WHOLE surface this generation requires of it, and every one is load-bearing: the factory asks
/// `canCreateService` before it clones, and a clone asks `canIssue`, `canRevoke` and `hasRole` on every
/// write. Both references are permanent (the factory's `registry` is `immutable`, and a clone pins its
/// own at `initialize` with no setter), so a core missing any one of the four cannot be repointed away
/// from later — a core without `hasRole` would leave `adminRevoke`, the compromised-signer mass-revoke
/// lever, reverting for every call forever.
///
/// **The three issuance-axis reads are a nested ladder, not three switches**, and generation 2 depends
/// on the gaps between them. `canRevoke` deliberately omits the live lifecycle terms `canIssue` folds
/// (standing, an active generation, the provider's current pointer), which is what preserves an
/// originator's ability to invalidate roots on a clone it has since superseded. Substituting one for the
/// other in either direction is a defect: upward it strands roots as unrevocable, downward it reopens a
/// retired clone for new issuance.
///
/// **This is deliberately NOT the legacy `isWhitelistedFor(bytes32,address)`.** That selector cannot
/// tell an issue call from a revoke call, so it cannot express the ladder at all; and on the core it
/// branches on `msg.sender`, answering the orthogonal VERIFY-key capability for any caller that is not
/// itself an attached service — so a factory asking it about a creation would be reading the wrong
/// mapping entirely.
interface IProviderAuthority {
    function canCreateService(bytes20 providerId, bytes32 recordType, address caller)
        external
        view
        returns (bool);
    function canIssue(address service, address signer) external view returns (bool);
    function canRevoke(address service, address signer) external view returns (bool);
    function hasRole(bytes32 role, address account) external view returns (bool);
}

/// @title DogTagIssuerV2 — per-record-type anchoring contract, now with a real owner.
///
/// @notice Generation-2 successor to `DogTagIssuer`. Anchoring SEMANTICS are unchanged —
/// `issue`/`revoke`/`isValid` mean what they meant, and `RootIssued`/`RootRevoked` are byte-identical,
/// so an existing verifier, indexer decoder or explorer reads a generation-2 clone exactly as it reads a
/// generation-1 clone. What changes is **who is asked**: the authority oracle is the S-6 authority core
/// rather than generation 1's `IssuerRegistry`, and every clone has an owner.
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
/// # Ownership is CONTROL, and confers no capability of its own
///
/// Owning a clone grants no right to issue and no right to revoke. Both are decided by the authority
/// core, for a property this codebase depends on (plan §3.3): **withdrawing a signer's grant must stop
/// the next `issue` and touch nothing already anchored.** If ownership carried an issuance right,
/// withdrawal would no longer stop issuance and that lever would be silently dead.
///
/// So a provider whose grant is withdrawn still owns its clone, may still transfer it and may still
/// repoint its listing — it simply cannot anchor anything new. Pinned by
/// `test_withdrawing_the_grant_stops_issuance_without_stripping_ownership` and
/// `test_owning_a_clone_confers_no_issuance_right`.
///
/// **The converse is not true, and it is the more surprising half:** ownership is not a capability, yet
/// the core folds the CONFIRMED owner into both `canIssue` and `canRevoke`. So a completed two-step
/// handover suspends both until the registrar confirms the new owner on the core — the live owner and the
/// confirmed owner disagree in between, and the core reads that disagreement as unresolved rather than as
/// authorization. That is a real operational consequence of a handover, not an incidental one, and it is
/// pinned by `test_a_handover_suspends_issuance_until_the_registrar_reconfirms`.
///
/// `revoke` keeps generation 1's authority split exactly — the H-1 originator, or the core's protocol
/// admin — with the ordinary arm now asking `canRevoke` rather than the legacy whitelist selector.
/// Extending revocation to the clone owner would let an owner revoke credentials it did not issue, which
/// is a distinct governance decision and is deliberately not taken here.
///
/// # The legacy `name()` getter is permanently EMPTY, and that is the point
///
/// Generation 1's `name` was written by the factory's `onlyOwner` `createIssuer` at KYC time, which is
/// the *only* reason a consumer could read it as an authoritative issuer identity. Creation is
/// self-service here, so a caller-supplied name would be a provider-chosen string arriving with genuine
/// factory provenance — a fabricated authority beside a green check, which is precisely the attack the
/// on-chain name read exists to defeat.
///
/// So no caller can set it: `initialize` takes no name, nothing in this contract ever writes the slot,
/// and `name()` answers `""` for the life of every generation-2 clone. A consumer reading it must report
/// authoritative identity **unavailable** rather than falling back to the document's own claim.
/// Registrar-controlled identity for this generation comes from the authority core instead — its
/// publication-safe identity anchor, reached through the core's directory resolver. Reconciling the
/// existing readers that still label the on-chain name authoritative is a later slice, not this one.
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
    /// @dev The core's protocol-admin role, in the `AccessControl` shape the core projects for exactly
    /// this read. Generation 1 spelled the same value as a bare `0x00` literal at each call site.
    bytes32 public constant DEFAULT_ADMIN_ROLE = bytes32(0);

    IProviderAuthority public registry;
    IRootIndex public rootIndex; // the factory (write-once rootIssuer index)
    bytes32 public recordType;
    /// @notice Retained for wire compatibility with generation 1's getter and **permanently empty** —
    /// no path in this contract writes it. See the contract doc: a self-service generation cannot offer
    /// a caller-chosen name as an authoritative identity.
    string public name;

    mapping(bytes32 => uint256) public issuedAt; // 0 = not issued
    mapping(bytes32 => uint256) public revokedAt; // 0 = not revoked
    mapping(bytes32 => address) public issuedBy; // H-1 originator

    /// @dev Byte-identical to generation 1's, so every existing decoder reads a generation-2 clone.
    event RootIssued(bytes32 indexed root, address indexed by, uint256 ts);
    event RootRevoked(bytes32 indexed root, address indexed by, uint256 ts);

    /// @dev Why `adminRevoke` passed over a requested root. An emergency sweep must not abort on one
    /// stale entry, so the alternative to reporting is silence — a caller told the whole batch succeeded
    /// when part of it did nothing.
    enum RevokeSkipReason {
        NotIssued,
        AlreadyRevoked
    }

    /// @dev Additive, and emitted only by `adminRevoke`. `revoke`/`bulkRevoke` revert on the same two
    /// conditions instead, which is the right answer for a targeted call.
    event AdminRevokeSkipped(bytes32 indexed root, RevokeSkipReason reason);

    /// @dev The caller holds no current grant to anchor on this clone (`canIssue`).
    error NotIssuanceCapable();
    /// @dev The caller holds no current grant to invalidate on this clone (`canRevoke`).
    error NotRevocationCapable();
    error BadRoot();
    error NotOriginatorOrAdmin();
    error NotAdmin();
    /// @dev Raised by the two paths that would leave the clone ownerless. See the contract doc.
    error OwnerCannotBeZero();

    /// @dev The implementation is locked at construction; clones initialize. `Ownable` rejects a zero
    /// initial owner, so the implementation is owned by its deployer — a value that never matters,
    /// because `_disableInitializers()` makes the implementation permanently uninitializable and it
    /// holds no roots. The factory nonetheless requires its exact runtime-code identity at construction;
    /// see `DogTagIssuerFactoryV2`'s dependency checks.
    constructor() Ownable(msg.sender) {
        _disableInitializers(); // C-1: lock the implementation (clones initialize)
    }

    modifier onlyIssuanceCapable() {
        if (!registry.canIssue(address(this), msg.sender)) revert NotIssuanceCapable();
        _;
    }

    /// @param owner_ the clone's controller. Set once, here, and thereafter only reachable through the
    /// two-step handover. The factory passes the creating caller, so there is no path by which a freshly
    /// created clone is owned by anyone but its creator.
    /// @dev Takes no name, deliberately — see the contract doc.
    function initialize(bytes32 rt, address reg, address index, address owner_) external initializer {
        require(reg != address(0) && index != address(0), "zero");
        if (owner_ == address(0)) revert OwnerCannotBeZero();
        recordType = rt;
        registry = IProviderAuthority(reg);
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
    // Anchoring — generation 1's semantics, asked of the generation-2 authority core
    // ---------------------------------------------------------------------------------------------

    function issue(bytes32 r) public onlyIssuanceCapable {
        if (r == bytes32(0) || issuedAt[r] != 0) revert BadRoot();
        issuedAt[r] = block.timestamp;
        issuedBy[r] = msg.sender;
        rootIndex.registerRoot(r); // write-once rootIssuer[r] = this clone (§11.10(a))
        emit RootIssued(r, msg.sender, block.timestamp);
    }

    /// @dev Authority is checked BEFORE the root's state, matching generation 1: a caller with no
    /// standing learns that first, whichever root it names.
    ///
    /// The admin arm is evaluated once and short-circuits the capability read, because the protocol
    /// admin is not required to hold an issuance grant on this clone — and `adminRevoke` already gives
    /// it this exact power unconditionally, so routing it through here grants nothing new. Generation 1
    /// reached the same arm only for an admin that also happened to be whitelisted, which was an
    /// accident of its `onlyWhitelisted` modifier rather than its documented intent.
    function revoke(bytes32 r) public {
        bool asAdmin = registry.hasRole(DEFAULT_ADMIN_ROLE, msg.sender);
        if (!asAdmin && !registry.canRevoke(address(this), msg.sender)) revert NotRevocationCapable();
        if (issuedAt[r] == 0 || revokedAt[r] != 0) revert BadRoot();
        if (!asAdmin && msg.sender != issuedBy[r]) revert NotOriginatorOrAdmin();
        revokedAt[r] = block.timestamp;
        emit RootRevoked(r, msg.sender, block.timestamp);
    }

    function bulkIssue(bytes32[] calldata rs) external onlyIssuanceCapable {
        for (uint256 i; i < rs.length; i++) {
            issue(rs[i]);
        }
    }

    /// @dev Reverts on the first root it cannot revoke. That is the difference from `adminRevoke`, which
    /// skips and reports: a targeted batch naming a root that is already revoked is a caller mistake
    /// worth surfacing, while an emergency sweep over a compromised signer's whole history must not be
    /// abortable by one stale entry in it.
    function bulkRevoke(bytes32[] calldata rs) external {
        for (uint256 i; i < rs.length; i++) {
            revoke(rs[i]);
        }
    }

    /// @notice Admin mass-revoke for a compromised signer (withdrawal is forward-only — §13.3).
    /// Bypasses originator binding; gated by the core's protocol admin only.
    ///
    /// @dev Every root the sweep passes over emits `AdminRevokeSkipped` with the reason. The loop still
    /// does not revert, so no single stale entry can abort the sweep — the reporting is additive and
    /// weakens nothing. Silence here was the defect: a caller submitting a signer's full history got one
    /// successful transaction whether it revoked everything or nothing, and the two are the difference
    /// between a contained compromise and an uncontained one.
    function adminRevoke(bytes32[] calldata rs) external {
        if (!registry.hasRole(DEFAULT_ADMIN_ROLE, msg.sender)) revert NotAdmin();
        for (uint256 i; i < rs.length; i++) {
            bytes32 r = rs[i];
            if (issuedAt[r] == 0) {
                emit AdminRevokeSkipped(r, RevokeSkipReason.NotIssued);
                continue;
            }
            if (revokedAt[r] != 0) {
                emit AdminRevokeSkipped(r, RevokeSkipReason.AlreadyRevoked);
                continue;
            }
            revokedAt[r] = block.timestamp;
            emit RootRevoked(r, msg.sender, block.timestamp);
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
