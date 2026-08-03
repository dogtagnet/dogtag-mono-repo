// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Initializable} from "@openzeppelin/contracts/proxy/utils/Initializable.sol";
import {Ownable, Ownable2Step} from "@openzeppelin/contracts/access/Ownable2Step.sol";

/// @notice The protocol-global write-once root->clone index (impl §11.10(a), architecture §13.9).
/// It is `DogTagIssuerFactory`, which is also the address the verification registry resolves every
/// anchored root through.
interface IRootIndex {
    function registerRoot(bytes32 root) external;
}

/// @dev The authority core — `ProviderRegistry`. These four functions are the
/// WHOLE surface this contract requires of it, and every one is load-bearing: the factory asks
/// `canCreateService` before it clones, and a clone asks `canIssue`, `canRevoke` and `hasRole` on every
/// write. Both references are permanent (the factory's `registry` is `immutable`, and a clone pins its
/// own at `initialize` with no setter), so a core missing any one of the four cannot be repointed away
/// from later — a core without `hasRole` would leave `adminRevoke`, the compromised-signer mass-revoke
/// lever, reverting for every call forever.
///
/// **The two issuance-axis reads are a nested ladder, not two switches**, and this contract depends on
/// the gap between them. `canRevoke` deliberately omits the live lifecycle terms `canIssue` folds
/// (standing, an active factory generation, the provider's current pointer), which is what preserves an
/// originator's ability to invalidate roots on a clone it has since replaced. Substituting one for the
/// other in either direction is a defect: upward it strands roots as unrevocable, downward it reopens a
/// replaced clone for new issuance.
///
/// **A single `isWhitelistedFor(recordType, signer)`-shaped read cannot serve here**, and the reason is
/// worth keeping: such a selector cannot tell an issue call from a revoke call, so it cannot express the
/// ladder at all. The questions are genuinely different, so the reads are too.
interface IProviderAuthority {
    function canCreateService(bytes20 providerId, bytes32 recordType, address caller)
        external
        view
        returns (bool);
    function canIssue(address service, address signer) external view returns (bool);
    function canRevoke(address service, address signer) external view returns (bool);
    function hasRole(bytes32 role, address account) external view returns (bool);
}

/// @title DogTagIssuer — per-record-type anchoring contract, owned by the provider that deployed it.
///
/// @notice One clone per (provider, record type). It anchors a credential's Merkle root, revokes it, and
/// answers `isValid` — and it asks the {ProviderRegistry} core, on every write, whether the caller may
/// do that.
///
/// # Why an owner at all
///
/// An anchoring clone gated ONLY on a signer whitelist has no on-chain answer to "who controls this
/// contract?", and the captain's requirement that only *"whitelisted people, AND owner of contracts"*
/// may publish a DNS claim is then not merely unimplemented — it is **unimplementable**, because there
/// is no owner to ask about.
///
/// The tempting substitute is a proxy: recompute the clone's deterministic address from its creation
/// salt and treat whoever is in the `business` slot as its controller. That proxy authorizes whoever
/// happened to be PASSED as `business` at creation, which in practice is the operator who pressed the
/// button rather than the organisation — so it answers a different question while looking like it
/// answers this one.
///
/// So every clone carries a real, checkable, transferable `owner()` instead.
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
/// **The creation seed does not weaken that, and the distinction is the whole design.** `initialize`
/// writes ONE ordinary entry into this clone's own list - the creator's - so a provider can use the
/// contract it just deployed. That is a list entry, not a rule: the owner is removable from it like any
/// other address and stays `owner()` afterwards, a handover seeds nobody, and the entry grants nothing
/// on its own because `issue` still asks the authority first. `owner() == msg.sender` appears nowhere in
/// the issuance path, and putting it there is what would actually break this section. Pinned by
/// `test_the_seeded_creator_is_an_ordinary_list_entry_that_removal_costs_no_ownership`,
/// `test_a_handover_does_not_seed_the_new_owner` and
/// `test_the_seeded_creator_still_anchors_nothing_without_the_authority_bit`.
///
/// **The converse is not true, and it is the more surprising half:** ownership is not a capability, yet
/// the core folds the CONFIRMED owner into both `canIssue` and `canRevoke`. So a completed two-step
/// handover suspends both until the registrar confirms the new owner on the core — the live owner and the
/// confirmed owner disagree in between, and the core reads that disagreement as unresolved rather than as
/// authorization. That is a real operational consequence of a handover, not an incidental one, and it is
/// pinned by `test_a_handover_suspends_issuance_until_the_registrar_reconfirms`.
///
/// `revoke` has two arms — the H-1 originator, or the core's protocol admin — with the ordinary arm
/// asking `canRevoke`.
/// Extending revocation to the clone owner would let an owner revoke credentials it did not issue, which
/// is a distinct governance decision and is deliberately not taken here.
///
/// # `name()` is permanently EMPTY, and that is the point
///
/// A clone's `name` could only be authoritative if a registrar wrote it at KYC time, which is
/// the *only* reason a consumer could read it as an authoritative issuer identity. Creation is
/// self-service here, so a caller-supplied name would be a provider-chosen string arriving with genuine
/// factory provenance — a fabricated authority beside a green check, which is precisely the attack the
/// on-chain name read exists to defeat.
///
/// So no caller can set it: `initialize` takes no name, nothing in this contract ever writes the slot,
/// and `name()` answers `""` for the life of every clone. A consumer reading it must report
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
///   * `renounceOwnership` is **disabled**. An ownerless clone is precisely the state this
///     contract exists to end, and OZ's default would let one transaction re-enter it irreversibly.
///   * `acceptOwnership` refuses `msg.sender == address(0)`. OZ's implementation compares
///     `pendingOwner() != msg.sender`; with no transfer pending both sides are the zero address, so the
///     comparison passes and ownership transfers to zero. That is unreachable in practice (nobody can
///     sign as the zero address) but it is unreachable by EVM accident rather than by contract, and the
///     invariant is worth stating in code where a test can hold it.
///
/// Net: after `initialize`, `owner()` is non-zero forever. Pinned by
/// `test_owner_can_never_become_the_zero_address`.
contract DogTagIssuer is Initializable, Ownable2Step {
    /// @dev The core's protocol-admin role, in the `AccessControl` shape the core projects for exactly
    /// this read, rather than spelling a bare `0x00` literal at each call site.
    bytes32 public constant DEFAULT_ADMIN_ROLE = bytes32(0);

    IProviderAuthority public registry;
    IRootIndex public rootIndex; // the factory (write-once rootIssuer index)
    bytes32 public recordType;
    /// @notice Part of the clone's read surface and **permanently empty** —
    /// no path in this contract writes it. See the contract doc: a self-service generation cannot offer
    /// a caller-chosen name as an authoritative identity.
    string public name;

    mapping(bytes32 => uint256) public issuedAt; // 0 = not issued
    mapping(bytes32 => uint256) public revokedAt; // 0 = not revoked
    mapping(bytes32 => address) public issuedBy; // H-1 originator

    /// @notice THIS CLONE'S OWN list of addresses that may anchor through it — the second of the two
    /// layers `issue` requires, and the one that makes the authority's grant safe to keep scope-free.
    ///
    /// The authority answers `rightsOf(address)`: one address in, a bitmask out, with no service in the
    /// question. That is deliberate and must stay that way — a registrar approves an applicant before
    /// that applicant has a clone, so there is nothing to key a grant against. The consequence, on its
    /// own, is that a signer approved for one provider could anchor on ANY clone in effective standing.
    ///
    /// So the scoping lives HERE, in the contract that already knows which provider it belongs to,
    /// rather than being pushed back into the lookup. Both layers must hold and either one refuses.
    ///
    /// **It is SEEDED with the creator at `initialize` and empty otherwise.** A provider that deploys
    /// its own clone must be able to use it; before the seed it could not, and there was no product
    /// path to fix that - no portal writes this list. The seed is a value, never a rule: read
    /// `initialize` for what it does and does not imply.
    mapping(address => bool) public issuanceAllowed;

    /// @dev The shape every off-chain decoder — the oversight indexer, the web verifier, both mobile
    /// clients — is written against. A widened event is a changed `topic0`, which drops the log from
    /// every one of them silently rather than loudly.
    event RootIssued(bytes32 indexed root, address indexed by, uint256 ts);
    event RootRevoked(bytes32 indexed root, address indexed by, uint256 ts);
    /// @dev Who changed this clone's own list, so an operator can reconstruct it without a getter
    /// call per candidate. `setBy` distinguishes an owner's enrolment from an admin's emergency
    /// removal, which have different meanings and different remedies.
    event IssuanceAllowedSet(address indexed signer, bool allowed, address indexed setBy);

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
    /// @dev The authority's grant holds, but THIS clone has not admitted the caller. Kept apart from
    /// {NotIssuanceCapable} because the two have different remedies and different owners: one is the
    /// registrar's grant, the other is this provider's own list.
    error NotLocallyAllowed();
    /// @dev Neither the clone owner nor the protocol admin.
    error NotOwnerOrAdmin();
    /// @dev The list already reads that way. Its own error rather than {BadRoot}, which is about roots.
    error NoChange();
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
    /// see `DogTagIssuerFactory`'s dependency checks.
    constructor() Ownable(msg.sender) {
        _disableInitializers(); // lock the implementation itself; only clones initialize
    }

    /// @dev TWO LAYERS, ANDed, and either one failing refuses.
    ///
    /// (1) the authority's scope-free grant — `canIssue` folds `rightsOf(signer) & RIGHT_ISSUE` with
    ///     every lifecycle term this clone's own service record carries; and
    /// (2) THIS clone's own list, which the authority knows nothing about.
    ///
    /// Layer 1 alone is what leaves a signer approved for one provider able to anchor on another's
    /// clone. Layer 2 alone would let a provider admit anyone it liked, with no KYC anywhere. Neither
    /// is redundant, and the order is deliberate: the authority answers first so a caller with no
    /// standing at all learns THAT, rather than being told it is merely not on a list.
    ///
    /// **There is no third arm, and adding one is the mistake to guard against.** `owner()` is not
    /// consulted here and must never be: the creation seed makes the creator usable by putting it in
    /// the LIST, which stays removable, whereas an `|| msg.sender == owner()` here would make removal
    /// unenforceable and silently disarm the withdrawal lever this contract exists to keep working.
    modifier onlyIssuanceCapable() {
        if (!registry.canIssue(address(this), msg.sender)) revert NotIssuanceCapable();
        if (!issuanceAllowed[msg.sender]) revert NotLocallyAllowed();
        _;
    }

    /// @param owner_ the clone's controller. Set once, here, and thereafter only reachable through the
    /// two-step handover. The factory passes the creating caller, so there is no path by which a freshly
    /// created clone is owned by anyone but its creator.
    /// @dev Takes no name, deliberately — see the contract doc.
    ///
    /// # The creation seed
    ///
    /// `owner_` is written onto {issuanceAllowed} here, so a provider can anchor through the contract it
    /// just deployed. Without it a self-service deployment lands in a dead end: the clone refuses its own
    /// creator with {NotLocallyAllowed}, and no surface in this product writes that list - the provider
    /// portal has no such affordance - so the contract a provider just paid to deploy is unusable by the
    /// only party that could fix it.
    ///
    /// **It is an ordinary list entry and NOT a rule about ownership.** Three consequences, each of which
    /// a rule would get wrong, and each pinned by its own test:
    ///
    ///   * it is REMOVABLE. `setIssuanceAllowed(owner(), false)` stops the next anchor and leaves
    ///     `owner()` exactly where it was - control and capability stay separate, which is the property
    ///     the contract doc argues at length and which an `owner() == msg.sender` check in
    ///     {onlyIssuanceCapable} would destroy;
    ///   * it grants NOTHING on its own. `issue` asks the authority first, so a creator the registrar
    ///     never granted is refused however this list reads; and
    ///   * it happens ONCE, at creation. A handover seeds nobody - re-seeding on transfer would make
    ///     ownership imply the right on an ongoing basis, and would silently re-admit an address the
    ///     previous owner had deliberately removed.
    ///
    /// The write is emitted through the same {IssuanceAllowedSet} event as every other, deliberately: an
    /// operator reconstructing the list from one log filter would otherwise miss its single most likely
    /// entry, and a second event topic would leave every existing decoder silently wrong. `setBy` is the
    /// initializing caller - the factory - which is both what actually wrote it and a third value that
    /// tells a creation seed apart from an owner's later enrolment.
    ///
    /// `owner_` is already refused as the zero address above, so the seed can never admit it - the same
    /// address {setIssuanceAllowed} rejects outright.
    function initialize(bytes32 rt, address reg, address index, address owner_) external initializer {
        require(reg != address(0) && index != address(0), "zero");
        if (owner_ == address(0)) revert OwnerCannotBeZero();
        recordType = rt;
        registry = IProviderAuthority(reg);
        rootIndex = IRootIndex(index);
        _transferOwnership(owner_);
        issuanceAllowed[owner_] = true;
        emit IssuanceAllowedSet(owner_, true, msg.sender);
    }

    // ---------------------------------------------------------------------------------------------
    // Ownership — two-step, and terminal in the sense that it can never be vacated
    // ---------------------------------------------------------------------------------------------

    /// @notice Disabled. A clone with no owner cannot claim a domain, be repointed, or be handed over —
    /// which is the state this contract exists to make unreachable, so it must not be re-enterable.
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
    // This clone's own issuance list — the second layer
    // ---------------------------------------------------------------------------------------------

    /// @notice Admit or remove an address from THIS clone's issuance list.
    ///
    /// # Who may write it, and why the two directions differ
    ///
    /// **Admitting is the OWNER's alone.** This is a new authority surface, so it is gated rather than
    /// left open, and the gate is the clone's own `owner()` — the provider that deployed it and the only
    /// party that knows which keys are its own staff. The protocol admin is deliberately EXCLUDED from
    /// this direction: a registrar that could enrol an address onto any clone could enrol its own key,
    /// and since it also writes the authority bit it would hold both layers at once — which is exactly
    /// the cross-provider issuance this list exists to prevent, reintroduced through the registrar.
    ///
    /// **Removing is the owner OR the protocol admin.** Removal only ever narrows, it is the incident
    /// -response direction for a compromised staff key, and it cannot manufacture an issuance right for
    /// anybody. This is the same asymmetry the core already applies on the revocation axis: the safety
    /// direction gets the wider authority.
    ///
    /// # This is not an issuance right
    ///
    /// Being on this list grants nothing by itself — `issue` still asks the authority first, so an
    /// address the registrar never granted is refused however this list reads. Owning the clone is
    /// likewise still not a capability: an owner who admits itself and holds no bit cannot anchor.
    /// Pinned by `test_owning_a_clone_confers_no_issuance_right`.
    ///
    /// The creator arrives already on the list - see {initialize} - so the entry this function most
    /// often writes ABOUT the creator is a removal. That removal is an ordinary one and is refused by
    /// nothing here: the seed carries no special protection, because a seed a provider could not undo
    /// would be exactly the ownership-implies-capability rule the seed was designed not to be.
    ///
    /// # It gates ISSUE only, never REVOKE
    ///
    /// `revoke` does not consult this list, and that is deliberate rather than an omission. Removing a
    /// signer must stop the next anchor and strand nothing already anchored — the same forward-only rule
    /// `delistFor` follows and the reason `canRevoke` is the wider rung. A removed originator can still
    /// invalidate what it issued.
    function setIssuanceAllowed(address signer, bool allowed) external {
        // The zero address can never sign, so admitting it could only ever be a mistake.
        if (signer == address(0)) revert NotLocallyAllowed();
        bool asOwner = msg.sender == owner();
        // Only a REMOVAL may come from the protocol admin; admitting is the owner's alone.
        bool asAdmin = !allowed && registry.hasRole(DEFAULT_ADMIN_ROLE, msg.sender);
        if (!asOwner && !asAdmin) revert NotOwnerOrAdmin();
        if (issuanceAllowed[signer] == allowed) revert NoChange();
        issuanceAllowed[signer] = allowed;
        emit IssuanceAllowedSet(signer, allowed, msg.sender);
    }

    // ---------------------------------------------------------------------------------------------
    // Anchoring
    // ---------------------------------------------------------------------------------------------

    function issue(bytes32 r) public onlyIssuanceCapable {
        if (r == bytes32(0) || issuedAt[r] != 0) revert BadRoot();
        issuedAt[r] = block.timestamp;
        issuedBy[r] = msg.sender;
        rootIndex.registerRoot(r); // write-once rootIssuer[r] = this clone (§11.10(a))
        emit RootIssued(r, msg.sender, block.timestamp);
    }

    /// @dev Authority is checked BEFORE the root's state: a caller with no
    /// standing learns that first, whichever root it names.
    ///
    /// The admin arm is evaluated once and short-circuits the capability read, because the protocol
    /// admin is not required to hold an issuance grant on this clone — and `adminRevoke` already gives
    /// it this exact power unconditionally, so routing it through here grants nothing new.
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
