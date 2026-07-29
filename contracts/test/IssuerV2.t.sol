// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test, Vm} from "forge-std/Test.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Errors} from "@openzeppelin/contracts/utils/Errors.sol";
import {Initializable} from "@openzeppelin/contracts/proxy/utils/Initializable.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {CloneProvenanceRouter} from "../src/CloneProvenanceRouter.sol";
import {DogTagIssuerV2, IProviderAuthority} from "../src/DogTagIssuerV2.sol";
import {DogTagIssuerFactoryV2, IPriorRootAnchors} from "../src/DogTagIssuerFactoryV2.sol";

/// @dev A stand-in for the S-6 `ProviderRegistry`, built to be faithful where fidelity is load-bearing.
///
/// The S-7 pair is built against the core's INTERFACE rather than against `dogtag-provreg-s6`'s branch,
/// so the suite needs a local authority. The danger in that is specific and worth naming: the core
/// documents `isRecognizedIssuer ⊇ canRevoke ⊇ canIssue` as a nested ladder, and several tests below
/// depend on the GAPS between the rungs — a superseded clone must stay revocable while refusing new
/// issuance. A mock exposing three independent booleans would let this suite assert behaviour the real
/// core forbids, and it would pass for the wrong reason.
///
/// So the three rungs here are DERIVED from one set of registrar facts, exactly as the core derives
/// them, and none of them is settable on its own. The containment is asserted directly by
/// `test_the_authority_ladder_is_nested_not_three_independent_switches`.
///
/// The owner term is read LIVE off the clone and compared against the registrar-confirmed owner, which
/// is what makes a two-step handover suspend issuance until the registrar reconfirms.
contract MockProviderAuthority is IProviderAuthority {
    struct Service {
        bytes20 providerId;
        bytes32 recordType;
        address confirmedOwner;
        bool cleared;
    }

    address public immutable admin;

    mapping(address => bool) public activeGeneration;
    mapping(bytes20 => address) public controller;
    mapping(bytes20 => mapping(address => bool)) public providerDelegate;
    mapping(bytes20 => bool) public providerCleared;
    mapping(bytes20 => mapping(bytes32 => bool)) public creationApproved;

    mapping(address => Service) internal _service;
    mapping(address => mapping(address => bool)) public issuanceGranted;
    mapping(bytes20 => mapping(bytes32 => address)) public currentService;

    error NotRegistrar();
    error UnknownService();

    constructor(address admin_) {
        admin = admin_;
    }

    modifier onlyRegistrar() {
        if (msg.sender != admin) revert NotRegistrar();
        _;
    }

    // --- registrar surface (mirrors the core's own write names) ---------------------------------

    function addFactoryGeneration(address factory) external onlyRegistrar {
        activeGeneration[factory] = true;
    }

    function deprecateFactoryGeneration(address factory) external onlyRegistrar {
        activeGeneration[factory] = false;
    }

    function registerProvider(bytes20 providerId, address controller_) external onlyRegistrar {
        controller[providerId] = controller_;
        providerCleared[providerId] = true;
    }

    function setProviderCleared(bytes20 providerId, bool cleared) external onlyRegistrar {
        providerCleared[providerId] = cleared;
    }

    function setProviderDelegate(bytes20 providerId, address delegate, bool allowed) external onlyRegistrar {
        providerDelegate[providerId][delegate] = allowed;
    }

    function setServiceCreationApproval(bytes20 providerId, bytes32 recordType, bool allowed)
        external
        onlyRegistrar
    {
        creationApproved[providerId][recordType] = allowed;
    }

    /// @dev Mirrors the core: the registrar supplies only the provider binding, and the record type and
    /// owner are RESOLVED off the clone.
    function attachService(address service, bytes20 providerId) external onlyRegistrar {
        _service[service] = Service({
            providerId: providerId,
            recordType: IIssuerReads(service).recordType(),
            confirmedOwner: IIssuerReads(service).owner(),
            cleared: true
        });
    }

    function setServiceCleared(address service, bool cleared) external onlyRegistrar {
        _requireService(service);
        _service[service].cleared = cleared;
    }

    /// @dev Re-resolves the live owner. Until this runs after a handover, live != confirmed.
    function confirmServiceOwner(address service) external onlyRegistrar {
        _requireService(service);
        _service[service].confirmedOwner = IIssuerReads(service).owner();
    }

    function setIssuanceCapability(address service, address signer, bool allowed) external onlyRegistrar {
        _requireService(service);
        issuanceGranted[service][signer] = allowed;
    }

    function repointService(address service) external onlyRegistrar {
        Service storage s = _requireService(service);
        currentService[s.providerId][s.recordType] = service;
    }

    // --- the ladder: derived, never set ---------------------------------------------------------

    function isRecognizedIssuer(address service, address signer) public view returns (bool) {
        return _service[service].providerId != bytes20(0) && issuanceGranted[service][signer];
    }

    function canRevoke(address service, address signer) public view override returns (bool) {
        return isRecognizedIssuer(service, signer) && _ownerConfirmed(service);
    }

    function canIssue(address service, address signer) public view override returns (bool) {
        if (!canRevoke(service, signer)) return false;
        Service storage s = _service[service];
        if (!providerCleared[s.providerId] || !s.cleared) return false;
        return currentService[s.providerId][s.recordType] == service;
    }

    function canCreateService(bytes20 providerId, bytes32 recordType, address caller)
        external
        view
        override
        returns (bool)
    {
        return activeGeneration[msg.sender] && creationApproved[providerId][recordType]
            && providerCleared[providerId]
            && (controller[providerId] == caller || providerDelegate[providerId][caller]);
    }

    /// @dev The core projects its one two-step authority key into both legacy role reads.
    function hasRole(bytes32 role, address account) external view override returns (bool) {
        return (role == bytes32(0) || role == keccak256("WHITELIST_ADMIN")) && account == admin;
    }

    function _ownerConfirmed(address service) internal view returns (bool) {
        Service storage s = _service[service];
        if (s.providerId == bytes20(0)) return false;
        return IIssuerReads(service).owner() == s.confirmedOwner;
    }

    function _requireService(address service) internal view returns (Service storage s) {
        s = _service[service];
        if (s.providerId == bytes20(0)) revert UnknownService();
    }
}

interface IIssuerReads {
    function owner() external view returns (address);
    function recordType() external view returns (bytes32);
}

/// @dev A hand-rolled contract that lies perfectly: it answers `owner()` and `recordType()` with exactly
/// the values a repoint would need. It is the whole point of the provenance check that this is not
/// enough — the factory never deployed it, so it can never be pointed at.
contract ImpostorIssuer {
    address public owner;
    bytes32 public recordType;

    constructor(address owner_, bytes32 recordType_) {
        owner = owner_;
        recordType = recordType_;
    }
}

/// @dev A non-clone whose getters revert loudly. If the factory ever called into it, the revert reason
/// would be this contract's, not the factory's — which is how `test_a_hostile_impostor_is_never_even_called`
/// proves the `isClone` storage read short-circuits before any external call.
contract HostileIssuer {
    error HostileGetterExecuted();

    function owner() external pure returns (address) {
        revert HostileGetterExecuted();
    }

    function recordType() external pure returns (bytes32) {
        revert HostileGetterExecuted();
    }
}

/// @dev Answers the prior-index selector and nothing else, always `false`. Indistinguishable from a real
/// router at construction — see `test_a_conforming_stub_prior_index_is_accepted_and_that_is_a_residual`.
contract StubAnchors is IPriorRootAnchors {
    function isRootAnchored(bytes32) external pure override returns (bool) {
        return false;
    }
}

/// @dev Claims every root, which would make every `registerRoot` revert. Refused at construction.
contract AllAnchored is IPriorRootAnchors {
    function isRootAnchored(bytes32) external pure override returns (bool) {
        return true;
    }
}

/// @dev Answers the selector with the wrong width — the shape a code-size check alone would miss.
contract WrongWidthAnchors {
    function isRootAnchored(bytes32) external pure returns (bool, bool) {
        return (false, false);
    }
}

/// @dev Has code, and answers no selector at all - deliberately empty.
contract AnswersNothing {}

/// @dev An implementation whose getters revert. A clone of it would be permanently unusable, and
/// `resolveActiveIssuer`'s no-revert guarantee would be false.
contract RevertingImplementation {
    error GetterReverted();

    function owner() external pure returns (address) {
        revert GetterReverted();
    }

    function pendingOwner() external pure returns (address) {
        revert GetterReverted();
    }

    function recordType() external pure returns (bytes32) {
        revert GetterReverted();
    }
}

/// @dev Authorizes indiscriminately, including the zero-everything query. Refused at construction.
contract PermissiveAuthority {
    fallback() external {
        assembly {
            mstore(0, 1)
            return(0, 32)
        }
    }
}

/// @notice Slice S-7 acceptance: the owner-bearing issuer clone and the self-service factory.
///
/// Four properties this suite exists to hold, each corresponding to something that is impossible today:
///
///   1. **A clone has a real, checkable, transferable owner** — generation-1 clones have none, so the
///      captain's "whitelisted people AND owner of contracts" cannot be enforced at all.
///   2. **Key rotation is two-step and actually works** — exercised as a real handover, proving the old
///      key can no longer act and the new one can. A rotation path that has never been run is a check
///      that never ran.
///   3. **A repoint resolves the address rather than accepting a claim about it** — a hand-rolled
///      contract cannot be pointed at even when it lies perfectly, and neither can another provider's
///      genuine clone.
///   4. **A wrong immutable dependency is refused at construction, not discovered in production** —
///      an EOA, a non-answering contract, and one that answers indiscriminately are each rejected.
///
/// Plus the property that must NOT regress: `rootIssuer` write-once, per contract and honest.
///
/// The topology in `setUp` is the real cutover ordering: router over generation 1 FIRST, then the
/// generation-2 factory pointed at that router, then generation 2 appended to it. Nothing about it is
/// circular, and `test_the_router_topology_is_router_first_then_factory_then_append` states so.
contract IssuerV2Test is Test {
    MockProviderAuthority authority;
    DogTagIssuerV2 impl;
    DogTagIssuerFactoryV2 factory;

    IssuerRegistry gen1Registry;
    DogTagIssuer gen1Impl;
    DogTagIssuerFactory gen1;
    DogTagIssuer gen1Clone;
    CloneProvenanceRouter router;

    address admin = address(0xA11CE);
    /// @dev An approved provider's controlling key, cleared for both record types.
    address providerA = address(0xA0A0);
    /// @dev A second approved provider — the misattribution target.
    address providerB = address(0xB0B0);
    /// @dev Cleared for TRAVEL only.
    address providerC = address(0xC0C0);
    /// @dev The rotation target: a controller key that is deliberately NOT an issuance signer. See
    /// `test_key_rotation_hands_control_from_the_issuance_signer_to_a_controller`.
    address controller = address(0xC717);
    address stranger = address(0x5721);
    /// @dev Where a fat-fingered handover goes. Nothing here ever accepts.
    address mistyped = address(0xDEAD);

    bytes20 constant PROVIDER_A = bytes20(uint160(0xAAA1));
    bytes20 constant PROVIDER_B = bytes20(uint160(0xBBB2));
    bytes20 constant PROVIDER_C = bytes20(uint160(0xCCC3));

    bytes32 constant VACCINATION = keccak256("VACCINATION");
    bytes32 constant TRAVEL = keccak256("TRAVEL_CLEARANCE");

    bytes32 constant ROOT_1 = bytes32(uint256(0x1111));
    bytes32 constant ROOT_2 = bytes32(uint256(0x2222));
    bytes32 constant ROOT_3 = bytes32(uint256(0x3333));

    function setUp() public {
        authority = new MockProviderAuthority(admin);
        impl = new DogTagIssuerV2();

        // A real generation-1 stack, so the cross-generation guard is exercised against real code.
        gen1Registry = new IssuerRegistry(admin);
        gen1Impl = new DogTagIssuer();
        gen1 = new DogTagIssuerFactory(address(gen1Impl), address(gen1Registry), admin);
        vm.startPrank(admin);
        gen1Registry.whitelistFor(VACCINATION, providerA);
        gen1Clone = DogTagIssuer(gen1.createIssuer("Gen1", VACCINATION, providerA));
        vm.stopPrank();

        // The S-8 router, deployed over generation 1 BEFORE generation 2 exists.
        address[] memory generations = new address[](1);
        generations[0] = address(gen1);
        router = new CloneProvenanceRouter(generations, admin);

        factory = new DogTagIssuerFactoryV2(address(impl), address(authority), address(router));

        // Generation 2 joins the router afterwards. Append-only is what makes this ordering legal.
        vm.prank(admin);
        router.appendGeneration(address(factory));

        vm.startPrank(admin);
        authority.addFactoryGeneration(address(factory));
        authority.registerProvider(PROVIDER_A, providerA);
        authority.registerProvider(PROVIDER_B, providerB);
        authority.registerProvider(PROVIDER_C, providerC);
        authority.setServiceCreationApproval(PROVIDER_A, VACCINATION, true);
        authority.setServiceCreationApproval(PROVIDER_A, TRAVEL, true);
        authority.setServiceCreationApproval(PROVIDER_B, VACCINATION, true);
        authority.setServiceCreationApproval(PROVIDER_C, TRAVEL, true);
        vm.stopPrank();
    }

    function _create(bytes20 providerId, address who, bytes32 rt, uint96 nonce)
        internal
        returns (DogTagIssuerV2)
    {
        vm.prank(who);
        return DogTagIssuerV2(factory.createIssuer(providerId, rt, nonce));
    }

    /// @dev Creation alone anchors nothing. This is the registrar work that follows it: attach the
    /// clone, grant the signer, and select it as the provider's current service.
    function _commission(DogTagIssuerV2 clone, bytes20 providerId, address signer) internal {
        vm.startPrank(admin);
        authority.attachService(address(clone), providerId);
        authority.setIssuanceCapability(address(clone), signer, true);
        authority.repointService(address(clone));
        vm.stopPrank();
    }

    function _live(bytes20 providerId, address who, bytes32 rt, uint96 nonce)
        internal
        returns (DogTagIssuerV2 clone)
    {
        clone = _create(providerId, who, rt, nonce);
        _commission(clone, providerId, who);
    }

    // =============================================================================================
    // Creation: self-service, gated per record type, owner is the creator
    // =============================================================================================

    /// @notice The generation-1 blocker: `createIssuer` was `onlyOwner`, so no provider could deploy
    /// anything. Here an approved provider deploys its own, and owns it.
    function test_an_approved_provider_deploys_its_own_clone_and_owns_it() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);

        assertTrue(factory.isClone(address(clone)), "not recorded as a clone");
        assertEq(clone.owner(), providerA, "creator is not the owner");
        assertEq(clone.recordType(), VACCINATION);
        assertEq(address(clone.registry()), address(authority));
        assertEq(address(clone.rootIndex()), address(factory));
        // No handover is in flight on a fresh clone.
        assertEq(clone.pendingOwner(), address(0));
    }

    function test_an_unapproved_caller_cannot_create() public {
        vm.prank(stranger);
        vm.expectRevert(DogTagIssuerFactoryV2.NotApproved.selector);
        factory.createIssuer(PROVIDER_A, VACCINATION, 0);
    }

    /// @notice Approval is per record type, which is what keeps the widened `isClone` set sound.
    function test_approval_is_per_record_type() public {
        vm.prank(providerC); // cleared for TRAVEL only
        vm.expectRevert(DogTagIssuerFactoryV2.NotApproved.selector);
        factory.createIssuer(PROVIDER_C, VACCINATION, 0);
    }

    /// @notice The provider is named, not claimed: a caller cannot create against a provider it does not
    /// act for, even one that is itself approved for the record type.
    function test_a_caller_cannot_create_against_another_providers_id() public {
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerFactoryV2.NotApproved.selector);
        factory.createIssuer(PROVIDER_B, VACCINATION, 0);
    }

    /// @notice The caller may be a delegate holding the provider's create permission, and the clone is
    /// then owned by the DELEGATE — the owner is always the caller, never the controller. Recorded as a
    /// test because it is the case where "owner is the creator" is surprising.
    function test_a_delegate_may_create_and_the_clone_is_owned_by_the_caller() public {
        vm.prank(admin);
        authority.setProviderDelegate(PROVIDER_A, stranger, true);

        DogTagIssuerV2 clone = _create(PROVIDER_A, stranger, VACCINATION, 9);
        assertEq(clone.owner(), stranger, "the clone is not owned by its creator");
        assertEq(factory.predictIssuer(VACCINATION, stranger, 9), address(clone));
    }

    /// @notice The core resolves this factory from `msg.sender`, so an unpinned or deprecated generation
    /// cannot create — a deployment precondition, not an incidental one.
    function test_the_core_must_pin_this_factory_as_an_active_generation() public {
        vm.prank(admin);
        authority.deprecateFactoryGeneration(address(factory));

        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerFactoryV2.NotApproved.selector);
        factory.createIssuer(PROVIDER_A, VACCINATION, 0);
    }

    /// @notice Creation is NOT issuance. The gate is `canCreateService`; anchoring additionally needs the
    /// registrar's attachment, grant, confirmed owner and current pointer.
    function test_a_freshly_created_clone_can_anchor_nothing() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);

        assertFalse(authority.canIssue(address(clone), providerA));
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.NotIssuanceCapable.selector);
        clone.issue(ROOT_1);

        _commission(clone, PROVIDER_A, providerA);
        vm.prank(providerA);
        clone.issue(ROOT_1);
        assertTrue(clone.isValid(ROOT_1));
    }

    /// @notice Self-service widens who may add to `isClone` — under generation 1 only the protocol owner
    /// could. The mandatory issuer-whitelist pillar keys its question on `clone.recordType()`, so a
    /// provider cleared for one record type cannot produce a contract that reads as another.
    function test_a_widened_clone_set_cannot_forge_a_record_type() public {
        DogTagIssuerV2 clone = _create(PROVIDER_C, providerC, TRAVEL, 0);

        assertTrue(factory.isClone(address(clone)), "the clone really did join the widened set");
        assertEq(clone.recordType(), TRAVEL, "the record type is resolved from the clone");
        // And there is no VACCINATION contract providerC could have made — the gate refused above.
        vm.prank(providerC);
        vm.expectRevert(DogTagIssuerFactoryV2.NotApproved.selector);
        factory.createIssuer(PROVIDER_C, VACCINATION, 1);
    }

    /// @notice A self-service generation must not let a provider choose the string a consumer might read
    /// as the issuer's authoritative identity. `name()` is empty on every clone, forever.
    function test_the_legacy_name_getter_is_permanently_empty() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);
        assertEq(clone.name(), "", "a generation-2 clone carries a name");

        // There is no argument by which a caller could supply one: creation takes none, and the clone is
        // already initialized, so the only setter-shaped path is closed too.
        vm.prank(providerA);
        vm.expectRevert(Initializable.InvalidInitialization.selector);
        clone.initialize(VACCINATION, address(authority), address(factory), providerA);
    }

    /// @notice `predictIssuer` is exact before deployment — the property the generation-1 salt bought and
    /// that the nonce must not cost.
    function test_predict_matches_the_deployment_exactly() public {
        address predicted = factory.predictIssuer(VACCINATION, providerA, 7);
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 7);
        assertEq(address(clone), predicted, "prediction diverged from deployment");
    }

    /// @notice THE reason the salt gained a nonce: without one a provider has exactly one possible clone
    /// address per record type and the repoint in requirement 1 has no target to move to.
    function test_the_nonce_gives_a_provider_a_second_address() public {
        DogTagIssuerV2 first = _create(PROVIDER_A, providerA, VACCINATION, 0);
        DogTagIssuerV2 second = _create(PROVIDER_A, providerA, VACCINATION, 1);

        assertTrue(address(first) != address(second), "nonce did not produce a distinct address");
        assertTrue(factory.isClone(address(first)) && factory.isClone(address(second)));
        assertEq(first.owner(), providerA);
        assertEq(second.owner(), providerA);
    }

    /// @notice Reusing a nonce still reverts (OZ `Clones` refuses a repeated salt). Recorded so the
    /// nonce's purpose is not mistaken for "creation became unbounded".
    function test_reusing_a_nonce_still_reverts() public {
        _create(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        vm.expectRevert(Errors.FailedDeployment.selector);
        factory.createIssuer(PROVIDER_A, VACCINATION, 0);
    }

    /// @notice There is exactly one creation path and it takes no owner argument, so at creation `owner()`
    /// and the salt binding agree — the generation-1 weakness where `business` defaulted to the operator's
    /// own signer is structurally unreachable. They can diverge afterwards, and only through the two-step
    /// handover; the salt records who created the clone, not who controls it now.
    function test_owner_and_the_salt_binding_agree_at_creation() public {
        DogTagIssuerV2 clone = _create(PROVIDER_B, providerB, VACCINATION, 3);
        assertEq(clone.owner(), providerB);
        assertEq(factory.predictIssuer(VACCINATION, providerB, 3), address(clone));
        // No other address predicts to it.
        assertTrue(factory.predictIssuer(VACCINATION, providerA, 3) != address(clone));

        // After a handover the salt still names the creator, and that is not a divergence to repair.
        vm.prank(providerB);
        clone.transferOwnership(controller);
        vm.prank(controller);
        clone.acceptOwnership();
        assertEq(clone.owner(), controller);
        assertEq(factory.predictIssuer(VACCINATION, providerB, 3), address(clone), "the salt moved");
    }

    // =============================================================================================
    // Key rotation — a real handover, run end to end
    // =============================================================================================

    /// @notice The rotation the captain asked for ("its okay keep it as one key for now. but allow key
    /// switch"), exercised as the migration it actually is.
    ///
    /// The assertions that matter: **nothing moves until the recipient accepts**, and after acceptance the
    /// **old key can no longer act** while the new one can — tested through `authorizeClone`, the predicate
    /// S-6 and S-9 will call, not only through this factory's own pointer.
    function test_key_rotation_hands_control_from_the_issuance_signer_to_a_controller() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);

        // Before: the signer holds control; the controller holds none.
        assertEq(factory.authorizeClone(address(clone), providerA), VACCINATION);
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.authorizeClone(address(clone), controller);

        // Step 1 of 2. This alone changes nothing.
        vm.prank(providerA);
        clone.transferOwnership(controller);

        assertEq(clone.owner(), providerA, "ownership moved before acceptance");
        assertEq(clone.pendingOwner(), controller);
        assertEq(factory.authorizeClone(address(clone), providerA), VACCINATION, "old key lost control early");
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.authorizeClone(address(clone), controller);
        // And the pending owner cannot act through the factory either.
        vm.prank(controller);
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.setActiveIssuer(address(clone));

        // Step 2 of 2.
        vm.prank(controller);
        clone.acceptOwnership();

        // After: the handover is complete and total.
        assertEq(clone.owner(), controller, "ownership did not move");
        assertEq(clone.pendingOwner(), address(0), "a pending owner survived acceptance");

        // The new key can act.
        assertEq(factory.authorizeClone(address(clone), controller), VACCINATION);
        vm.prank(controller);
        factory.setActiveIssuer(address(clone));
        assertEq(factory.resolveActiveIssuer(controller, VACCINATION), address(clone));

        // The old key cannot — not through the predicate, not through the pointer, not over the clone.
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.authorizeClone(address(clone), providerA);
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.setActiveIssuer(address(clone));
        vm.prank(providerA);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, providerA));
        clone.transferOwnership(stranger);
    }

    /// @notice Ownership is not a capability, yet the core folds the CONFIRMED owner into both issuance
    /// and revocation — so a completed handover suspends both until the registrar reconfirms. That is a
    /// real operational consequence of a rotation and the one most likely to surprise an operator.
    function test_a_handover_suspends_issuance_until_the_registrar_reconfirms() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        clone.issue(ROOT_1);

        vm.prank(providerA);
        clone.transferOwnership(controller);
        vm.prank(controller);
        clone.acceptOwnership();

        // The signer's grant is untouched, but live != confirmed, so the core answers "unresolved".
        assertTrue(authority.issuanceGranted(address(clone), providerA));
        assertFalse(authority.canIssue(address(clone), providerA));
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.NotIssuanceCapable.selector);
        clone.issue(ROOT_2);
        // And revocation is suspended with it — the confirmed-owner term is on both rungs.
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.NotRevocationCapable.selector);
        clone.revoke(ROOT_1);

        vm.prank(admin);
        authority.confirmServiceOwner(address(clone));

        vm.prank(providerA);
        clone.issue(ROOT_2);
        assertTrue(clone.isValid(ROOT_2));
    }

    /// @notice A one-step transfer to a mistyped address would strand the clone forever. Two-step means
    /// the mistake is recoverable: the pending owner never accepted, and `transferOwnership(address(0))`
    /// cancels. This is both the recovery proof and the proof that the cancel path cannot be used to
    /// orphan the clone.
    function test_a_handover_to_a_mistyped_address_is_recoverable() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);

        vm.prank(providerA);
        clone.transferOwnership(mistyped);
        assertEq(clone.owner(), providerA, "a mistyped handover took effect immediately");
        assertEq(clone.pendingOwner(), mistyped);

        // Cancel: allowed for exactly this, and it zeroes the PENDING owner, never the owner.
        vm.prank(providerA);
        clone.transferOwnership(address(0));
        assertEq(clone.pendingOwner(), address(0));
        assertEq(clone.owner(), providerA, "cancelling a handover orphaned the clone");

        // The mistyped address can no longer complete the transfer it never accepted.
        vm.prank(mistyped);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, mistyped));
        clone.acceptOwnership();

        // And the real handover still works afterwards.
        vm.prank(providerA);
        clone.transferOwnership(controller);
        vm.prank(controller);
        clone.acceptOwnership();
        assertEq(clone.owner(), controller);
    }

    /// @notice After `initialize`, `owner()` is non-zero forever. Both paths that could vacate it are
    /// closed in the contract rather than left to the fact that nobody can sign as the zero address.
    function test_owner_can_never_become_the_zero_address() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);

        // Path 1: renounce is disabled outright.
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.OwnerCannotBeZero.selector);
        clone.renounceOwnership();

        // Path 2: with no transfer pending, OZ's `pendingOwner() != msg.sender` comparison passes for the
        // zero address (0 == 0) and would hand ownership to it. The override refuses.
        assertEq(clone.pendingOwner(), address(0));
        vm.prank(address(0));
        vm.expectRevert(DogTagIssuerV2.OwnerCannotBeZero.selector);
        clone.acceptOwnership();

        assertEq(clone.owner(), providerA, "owner was vacated");
    }

    function test_only_the_owner_may_start_a_handover() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(stranger);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, stranger));
        clone.transferOwnership(stranger);
    }

    /// @notice Ownership is CONTROL, not an issuance capability. Withdrawing a signer's grant must still
    /// stop the next `issue` and touch nothing already anchored (plan §3.3), and it must not strip
    /// control — a withdrawn provider can still hand its clone over.
    function test_withdrawing_the_grant_stops_issuance_without_stripping_ownership() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);

        vm.prank(providerA);
        clone.issue(ROOT_1);
        assertTrue(clone.isValid(ROOT_1));

        vm.prank(admin);
        authority.setIssuanceCapability(address(clone), providerA, false);

        // New issuance stops.
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.NotIssuanceCapable.selector);
        clone.issue(ROOT_2);
        // What was already anchored is untouched.
        assertTrue(clone.isValid(ROOT_1), "withdrawing a grant invalidated an anchored root");

        // Control survives: the withdrawn provider still owns it and can still hand it over.
        assertEq(clone.owner(), providerA);
        vm.prank(providerA);
        clone.transferOwnership(controller);
        vm.prank(controller);
        clone.acceptOwnership();
        assertEq(clone.owner(), controller);
    }

    /// @notice The owner is not an issuance signer by virtue of owning. Stated as a test because merging
    /// the two would silently disarm the withdrawal lever above.
    function test_owning_a_clone_confers_no_issuance_right() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        clone.transferOwnership(controller);
        vm.prank(controller);
        clone.acceptOwnership();
        vm.prank(admin);
        authority.confirmServiceOwner(address(clone));

        vm.prank(controller); // owns it, holds no grant
        vm.expectRevert(DogTagIssuerV2.NotIssuanceCapable.selector);
        clone.issue(ROOT_1);
    }

    // =============================================================================================
    // Revocation authority
    // =============================================================================================

    /// @notice `revoke` keeps generation 1's split: the H-1 originator, or the protocol admin. A signer
    /// that holds a current grant but did not anchor the root is neither.
    function test_a_capable_signer_that_did_not_issue_a_root_cannot_revoke_it() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        clone.issue(ROOT_1);

        vm.prank(admin);
        authority.setIssuanceCapability(address(clone), stranger, true);
        assertTrue(authority.canRevoke(address(clone), stranger), "the capability really is held");

        vm.prank(stranger);
        vm.expectRevert(DogTagIssuerV2.NotOriginatorOrAdmin.selector);
        clone.revoke(ROOT_1);
    }

    /// @notice The protocol admin arm, which needs no issuance grant on the clone — and grants nothing
    /// `adminRevoke` did not already give it unconditionally.
    function test_the_protocol_admin_may_revoke_a_root_it_did_not_issue() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        clone.issue(ROOT_1);

        assertFalse(authority.canRevoke(address(clone), admin), "the admin holds no grant here");
        vm.prank(admin);
        clone.revoke(ROOT_1);
        assertFalse(clone.isValid(ROOT_1));
    }

    /// @notice An emergency sweep must not abort on one stale entry, so it reports what it passed over
    /// instead of reverting — and it still revokes everything it can.
    function test_admin_revoke_reports_every_skipped_root_and_still_revokes_the_rest() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.startPrank(providerA);
        clone.issue(ROOT_1);
        clone.issue(ROOT_2);
        clone.revoke(ROOT_2);
        vm.stopPrank();

        bytes32[] memory rs = new bytes32[](3);
        rs[0] = ROOT_3; // never issued
        rs[1] = ROOT_2; // already revoked
        rs[2] = ROOT_1; // the one that must still be swept

        vm.expectEmit(true, false, false, true, address(clone));
        emit DogTagIssuerV2.AdminRevokeSkipped(ROOT_3, DogTagIssuerV2.RevokeSkipReason.NotIssued);
        vm.expectEmit(true, false, false, true, address(clone));
        emit DogTagIssuerV2.AdminRevokeSkipped(ROOT_2, DogTagIssuerV2.RevokeSkipReason.AlreadyRevoked);
        vm.prank(admin);
        clone.adminRevoke(rs);

        assertFalse(clone.isValid(ROOT_1), "a skip aborted the sweep");
        assertFalse(clone.isIssued(ROOT_3), "a root was invented");
    }

    /// @notice The bulk paths carry the same gates as their singular forms. `bulkIssue` keeps an outer
    /// capability check; `bulkRevoke` deliberately has none, because `revoke`'s gate is a two-arm
    /// disjunction (the capability OR the protocol admin) and an outer single-arm modifier would refuse
    /// the admin. Each element is gated by the call it makes, so the authority is identical either way.
    function test_the_bulk_paths_carry_the_same_gates_as_their_singular_forms() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);

        bytes32[] memory two = new bytes32[](2);
        two[0] = ROOT_1;
        two[1] = ROOT_2;

        vm.prank(stranger); // holds no grant
        vm.expectRevert(DogTagIssuerV2.NotIssuanceCapable.selector);
        clone.bulkIssue(two);

        vm.prank(providerA);
        clone.bulkIssue(two);
        assertTrue(clone.isValid(ROOT_1) && clone.isValid(ROOT_2), "a capable signer could not bulk issue");

        vm.prank(stranger);
        vm.expectRevert(DogTagIssuerV2.NotRevocationCapable.selector);
        clone.bulkRevoke(two);

        vm.prank(providerA);
        clone.bulkRevoke(two);
        assertTrue(clone.isRevoked(ROOT_1) && clone.isRevoked(ROOT_2), "bulk revoke did not take");
    }

    /// @notice `bulkRevoke` reverts on the FIRST root it cannot revoke - the whole batch rolls back. That
    /// is the deliberate asymmetry with `adminRevoke`, which skips and reports instead, because an
    /// emergency sweep must not be abortable by one stale entry.
    function test_bulk_revoke_reverts_on_the_first_unrevocable_root() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        clone.issue(ROOT_1);

        bytes32[] memory rs = new bytes32[](2);
        rs[0] = ROOT_3; // never issued
        rs[1] = ROOT_1; // would otherwise be revoked

        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.BadRoot.selector);
        clone.bulkRevoke(rs);

        assertTrue(clone.isValid(ROOT_1), "a reverting batch still revoked something");
    }

    /// @notice The EMPTY batch is where the two bulk paths differ, and it is the only place either outer
    /// gate is observable at all - with a non-empty batch the per-element call refuses first, so the outer
    /// check on `bulkIssue` is otherwise redundant.
    ///
    /// `bulkIssue([])` refuses an incapable caller, because it keeps the outer modifier. `bulkRevoke([])`
    /// is a no-op for anyone, because it cannot carry one: `revoke`'s gate is a two-arm disjunction and a
    /// single-arm modifier would refuse the protocol admin. The asymmetry is a consequence of that, not a
    /// choice - and the no-op asserts NOTHING about the caller's authority, which is why the same caller
    /// is then refused a real revoke here.
    function test_an_empty_batch_is_where_the_two_bulk_gates_differ() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        clone.issue(ROOT_1);

        vm.prank(stranger);
        vm.expectRevert(DogTagIssuerV2.NotIssuanceCapable.selector);
        clone.bulkIssue(new bytes32[](0));

        vm.prank(stranger);
        clone.bulkRevoke(new bytes32[](0));

        assertTrue(clone.isValid(ROOT_1), "an empty batch touched a root");
        vm.prank(stranger);
        vm.expectRevert(DogTagIssuerV2.NotRevocationCapable.selector);
        clone.revoke(ROOT_1);
    }

    function test_only_the_protocol_admin_may_admin_revoke() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        clone.issue(ROOT_1);

        bytes32[] memory rs = new bytes32[](1);
        rs[0] = ROOT_1;
        vm.prank(providerA); // holds the issuance grant, is not the admin
        vm.expectRevert(DogTagIssuerV2.NotAdmin.selector);
        clone.adminRevoke(rs);
    }

    // =============================================================================================
    // The authority core's ladder — the property this suite's mock must not fake
    // =============================================================================================

    /// @notice The clone must ask the NARROW rung to anchor and the WIDE one to invalidate, and this is
    /// the test that holds it. A superseded clone is the state where the two answers differ: it still
    /// holds its grant and its confirmed owner, so `canRevoke` is true, while the provider's pointer has
    /// moved, so `canIssue` is false.
    ///
    /// Substituting one rung for the other is invisible to every other test here, because everywhere else
    /// the two agree. Upward (`issue` asking `canRevoke`) a retired clone silently reopens for new
    /// issuance; downward (`revoke` asking `canIssue`) a root anchored on a superseded clone becomes
    /// permanently unrevocable by the originator that anchored it.
    function test_a_superseded_clone_refuses_new_issuance_but_still_revokes() public {
        DogTagIssuerV2 oldClone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        oldClone.issue(ROOT_1);

        // Repointing the core's own pointer is what supersedes it.
        DogTagIssuerV2 newClone = _live(PROVIDER_A, providerA, VACCINATION, 1);
        assertTrue(authority.canRevoke(address(oldClone), providerA), "the wide rung must still hold");
        assertFalse(authority.canIssue(address(oldClone), providerA), "the narrow rung must have dropped");

        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.NotIssuanceCapable.selector);
        oldClone.issue(ROOT_2);

        vm.prank(providerA);
        oldClone.revoke(ROOT_1);
        assertFalse(oldClone.isValid(ROOT_1), "a superseded root became unrevocable");

        // The clone that IS current anchors normally.
        vm.prank(providerA);
        newClone.issue(ROOT_2);
        assertTrue(newClone.isValid(ROOT_2));
    }

    /// @notice The other live lifecycle term `canIssue` folds and `canRevoke` does not: a suspended
    /// provider anchors nothing new, and invalidation must not be blocked by a review state.
    function test_a_suspended_provider_anchors_nothing_but_can_still_revoke() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        clone.issue(ROOT_1);

        vm.prank(admin);
        authority.setProviderCleared(PROVIDER_A, false);

        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.NotIssuanceCapable.selector);
        clone.issue(ROOT_2);

        vm.prank(providerA);
        clone.revoke(ROOT_1);
        assertFalse(clone.isValid(ROOT_1), "a suspension stranded a root as unrevocable");
    }

    /// @notice `isRecognizedIssuer ⊇ canRevoke ⊇ canIssue`, asserted directly. Several tests here depend
    /// on the GAPS between the rungs, so a mock whose three answers were independently settable would let
    /// this suite assert states the real core forbids.
    function test_the_authority_ladder_is_nested_not_three_independent_switches() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);
        address c = address(clone);

        // Unattached: no rung answers.
        _assertLadder(c, providerA, false, false, false);

        vm.prank(admin);
        authority.attachService(c, PROVIDER_A);
        _assertLadder(c, providerA, false, false, false); // attached, no grant

        vm.prank(admin);
        authority.setIssuanceCapability(c, providerA, true);
        // Recognized and revocable; not yet issuable — no current pointer.
        _assertLadder(c, providerA, true, true, false);

        vm.prank(admin);
        authority.repointService(c);
        _assertLadder(c, providerA, true, true, true);

        // A live lifecycle term falls away: only the narrowest rung drops.
        vm.prank(admin);
        authority.setProviderCleared(PROVIDER_A, false);
        _assertLadder(c, providerA, true, true, false);
        vm.prank(admin);
        authority.setProviderCleared(PROVIDER_A, true);

        // The confirmed-owner term falls away: the two lower rungs drop, the widest holds.
        vm.prank(providerA);
        clone.transferOwnership(controller);
        vm.prank(controller);
        clone.acceptOwnership();
        _assertLadder(c, providerA, true, false, false);
    }

    function _assertLadder(address service, address signer, bool recognized, bool revocable, bool issuable)
        internal
        view
    {
        assertEq(authority.isRecognizedIssuer(service, signer), recognized, "isRecognizedIssuer");
        assertEq(authority.canRevoke(service, signer), revocable, "canRevoke");
        assertEq(authority.canIssue(service, signer), issuable, "canIssue");
        // Containment, restated as the implication it is.
        assertTrue(!issuable || revocable, "canIssue exceeded canRevoke");
        assertTrue(!revocable || recognized, "canRevoke exceeded isRecognizedIssuer");
    }

    // =============================================================================================
    // The repoint: resolved, never accepted
    // =============================================================================================

    /// @notice A hand-rolled contract that answers `owner()` and `recordType()` with exactly the right
    /// values is still refused, because provenance is read from the factory's OWN storage and that
    /// mapping is written only by `createIssuer`.
    function test_a_repoint_cannot_accept_an_address_the_factory_did_not_produce() public {
        ImpostorIssuer fake = new ImpostorIssuer(providerA, VACCINATION);

        // The lie is perfect on its face.
        assertEq(fake.owner(), providerA);
        assertEq(fake.recordType(), VACCINATION);
        // And it buys nothing.
        assertFalse(factory.isClone(address(fake)));
        (bool ok,) = factory.cloneAuthorization(address(fake), providerA);
        assertFalse(ok, "an impostor passed the predicate");

        vm.expectRevert(DogTagIssuerFactoryV2.NotAClone.selector);
        factory.authorizeClone(address(fake), providerA);

        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerFactoryV2.NotAClone.selector);
        factory.setActiveIssuer(address(fake));

        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(0));
    }

    /// @notice The provenance check is a read of the factory's own storage and it runs FIRST, so a
    /// non-clone never executes at all. If the order were reversed this reverts with the hostile
    /// contract's error instead of the factory's.
    function test_a_hostile_impostor_is_never_even_called() public {
        HostileIssuer hostile = new HostileIssuer();

        vm.expectRevert(DogTagIssuerFactoryV2.NotAClone.selector);
        factory.authorizeClone(address(hostile), providerA);

        (bool ok, bytes32 rt) = factory.cloneAuthorization(address(hostile), providerA);
        assertFalse(ok);
        assertEq(rt, bytes32(0));

        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerFactoryV2.NotAClone.selector);
        factory.setActiveIssuer(address(hostile));
    }

    /// @notice Provenance alone is not enough. Provider B's clone is entirely genuine; pointing A's
    /// listing at it would be misattribution rather than forgery, and the control check is what catches
    /// it.
    function test_a_repoint_cannot_take_another_providers_genuine_clone() public {
        DogTagIssuerV2 bClone = _create(PROVIDER_B, providerB, VACCINATION, 0);
        assertTrue(factory.isClone(address(bClone)), "the target really is a genuine clone");

        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.authorizeClone(address(bClone), providerA);

        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.setActiveIssuer(address(bClone));

        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(0));
    }

    /// @notice The storage key is READ off the clone, never supplied. `setActiveIssuer` takes exactly one
    /// argument — the target address — so there is no argument by which a caller could name a slot: a
    /// VACCINATION clone can only ever occupy the VACCINATION slot.
    function test_the_record_type_key_is_resolved_from_the_clone_not_supplied() public {
        DogTagIssuerV2 vacc = _create(PROVIDER_A, providerA, VACCINATION, 0);
        DogTagIssuerV2 travel = _create(PROVIDER_A, providerA, TRAVEL, 0);

        vm.prank(providerA);
        factory.setActiveIssuer(address(vacc));
        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(vacc));
        assertEq(
            factory.resolveActiveIssuer(providerA, TRAVEL), address(0), "a clone occupied a foreign slot"
        );

        vm.prank(providerA);
        factory.setActiveIssuer(address(travel));
        assertEq(factory.resolveActiveIssuer(providerA, TRAVEL), address(travel));
        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(vacc), "the other slot moved");

        // The predicate itself returns each clone's own key.
        assertEq(factory.authorizeClone(address(vacc), providerA), VACCINATION);
        assertEq(factory.authorizeClone(address(travel), providerA), TRAVEL);
    }

    /// @notice The self-service repoint the captain described: move to a second clone of your own. What
    /// the old clone already anchored stays anchored to the old clone — `rootIssuer[R]` is write-once, and
    /// retroactively re-attributing issued credentials to a contract that did not issue them is exactly
    /// the misattribution the control check exists to prevent.
    ///
    /// The old clone stays REVOCABLE after being superseded, and that is the ladder's gap doing its job:
    /// `canRevoke` omits the current-pointer term that `canIssue` folds, so no repoint can strand a root
    /// as unrevocable by the originator that anchored it.
    function test_a_provider_repoints_to_a_second_clone_of_its_own() public {
        DogTagIssuerV2 oldClone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.startPrank(providerA);
        factory.setActiveIssuer(address(oldClone));
        oldClone.issue(ROOT_1);
        vm.stopPrank();
        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(oldClone));

        DogTagIssuerV2 newClone = _live(PROVIDER_A, providerA, VACCINATION, 1);
        vm.prank(providerA);
        factory.setActiveIssuer(address(newClone));

        assertEq(
            factory.resolveActiveIssuer(providerA, VACCINATION), address(newClone), "repoint did not take"
        );
        // History is untouched and still revocable where it was issued, though the superseded clone can
        // no longer anchor anything new.
        assertEq(factory.rootIssuer(ROOT_1), address(oldClone), "an issued root followed the repoint");
        assertFalse(authority.canIssue(address(oldClone), providerA), "a superseded clone still issues");
        assertTrue(authority.canRevoke(address(oldClone), providerA), "a superseded root became unrevocable");
        assertTrue(oldClone.isValid(ROOT_1));
        vm.prank(providerA);
        oldClone.revoke(ROOT_1);
        assertFalse(oldClone.isValid(ROOT_1));
    }

    /// @notice A pointer left behind by a handover degrades to "nothing recorded" rather than to a claim
    /// that is no longer true. The raw mapping keeps the CURRENT designation; the resolving read refuses
    /// it. The trail of designations is in the events, not in this mapping.
    function test_a_stale_pointer_degrades_to_absent_rather_than_to_a_false_claim() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        factory.setActiveIssuer(address(clone));
        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(clone));

        vm.prank(providerA);
        clone.transferOwnership(controller);
        vm.prank(controller);
        clone.acceptOwnership();

        assertEq(
            factory.activeIssuer(providerA, VACCINATION), address(clone), "the raw designation was rewritten"
        );
        assertEq(
            factory.resolveActiveIssuer(providerA, VACCINATION), address(0), "a stale pointer still resolved"
        );
        // Nor is the pointer inherited: a repoint is an explicit act by the new owner.
        assertEq(factory.resolveActiveIssuer(controller, VACCINATION), address(0));
    }

    /// @notice The pointer is advisory: nothing routes through it, so a clone with no designation at all
    /// still anchors normally once the core clears it. Recorded because a reader could otherwise take
    /// `activeIssuer` for a gate.
    function test_the_pointer_is_advisory_and_gates_no_issuance() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        assertEq(factory.activeIssuer(providerA, VACCINATION), address(0), "a pointer was set implicitly");

        vm.prank(providerA);
        clone.issue(ROOT_1);
        assertTrue(clone.isValid(ROOT_1), "an unpointed clone could not anchor");
    }

    function test_a_provider_can_withdraw_its_own_pointer() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);
        vm.startPrank(providerA);
        factory.setActiveIssuer(address(clone));
        factory.clearActiveIssuer(VACCINATION);
        vm.stopPrank();

        assertEq(factory.activeIssuer(providerA, VACCINATION), address(0));
        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(0));
    }

    function test_the_zero_address_never_authorizes() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);
        (bool ok,) = factory.cloneAuthorization(address(clone), address(0));
        assertFalse(ok);
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.authorizeClone(address(clone), address(0));
    }

    // =============================================================================================
    // Construction: every immutable dependency is checked for code AND for ABI behaviour
    // =============================================================================================

    function test_a_zero_dependency_is_refused() public {
        vm.expectRevert(DogTagIssuerFactoryV2.ZeroAddress.selector);
        new DogTagIssuerFactoryV2(address(0), address(authority), address(router));
        vm.expectRevert(DogTagIssuerFactoryV2.ZeroAddress.selector);
        new DogTagIssuerFactoryV2(address(impl), address(0), address(router));
        // The cross-generation guard is MANDATORY: a zero here silently disabled it.
        vm.expectRevert(DogTagIssuerFactoryV2.ZeroAddress.selector);
        new DogTagIssuerFactoryV2(address(impl), address(authority), address(0));
    }

    /// @notice A staticcall to an EOA SUCCEEDS with empty returndata, so an EOA dependency is the shape
    /// that stays silent. Each of the three is refused for having no code at all.
    function test_an_eoa_dependency_is_refused() public {
        vm.expectRevert(abi.encodeWithSelector(DogTagIssuerFactoryV2.NotAContract.selector, stranger));
        new DogTagIssuerFactoryV2(stranger, address(authority), address(router));
        vm.expectRevert(abi.encodeWithSelector(DogTagIssuerFactoryV2.NotAContract.selector, stranger));
        new DogTagIssuerFactoryV2(address(impl), stranger, address(router));
        vm.expectRevert(abi.encodeWithSelector(DogTagIssuerFactoryV2.NotAContract.selector, stranger));
        new DogTagIssuerFactoryV2(address(impl), address(authority), stranger);
    }

    /// @notice An implementation whose `owner()`/`recordType()` revert would make `createIssuer` report
    /// success while producing a permanently unusable clone, and would falsify the no-revert guarantee
    /// `resolveActiveIssuer` rests on.
    function test_an_implementation_whose_getters_revert_is_refused() public {
        address bad = address(new RevertingImplementation());
        vm.expectRevert(
            abi.encodeWithSelector(DogTagIssuerFactoryV2.ImplementationDoesNotAnswer.selector, bad)
        );
        new DogTagIssuerFactoryV2(bad, address(authority), address(router));
    }

    /// @notice Code alone is not enough: a contract answering no selector is refused too.
    function test_an_implementation_that_answers_nothing_is_refused() public {
        address bad = address(new AnswersNothing());
        vm.expectRevert(
            abi.encodeWithSelector(DogTagIssuerFactoryV2.ImplementationDoesNotAnswer.selector, bad)
        );
        new DogTagIssuerFactoryV2(bad, address(authority), address(router));
    }

    /// @notice A core that authorizes indiscriminately — including the zero-everything query — would make
    /// every creation and every anchor pass. Refused before it is stored, because it cannot be repointed.
    function test_an_authority_that_authorizes_indiscriminately_is_refused() public {
        address bad = address(new PermissiveAuthority());
        vm.expectRevert(abi.encodeWithSelector(DogTagIssuerFactoryV2.AuthorityDoesNotAnswer.selector, bad));
        new DogTagIssuerFactoryV2(address(impl), bad, address(router));
    }

    function test_an_authority_that_answers_nothing_is_refused() public {
        address bad = address(new AnswersNothing());
        vm.expectRevert(abi.encodeWithSelector(DogTagIssuerFactoryV2.AuthorityDoesNotAnswer.selector, bad));
        new DogTagIssuerFactoryV2(address(impl), bad, address(router));
    }

    /// @notice Generation 2 must be bound to its OWN authority core, and that is a held property rather
    /// than only a documented precondition: generation 1's `IssuerRegistry` is a real, live contract that
    /// answers `isWhitelistedFor` and `hasRole` and none of the three capability questions, so it is
    /// refused in the slot outright. Sharing one core across generations would also break the router's
    /// cutover freeze, since one grant would authorize anchoring on both generations' clones.
    function test_a_generation_one_registry_cannot_be_the_authority_core() public {
        assertTrue(gen1Registry.isWhitelistedFor(VACCINATION, providerA), "the core really is live");
        vm.expectRevert(
            abi.encodeWithSelector(
                DogTagIssuerFactoryV2.AuthorityDoesNotAnswer.selector, address(gen1Registry)
            )
        );
        new DogTagIssuerFactoryV2(address(impl), address(gen1Registry), address(router));
    }

    /// @notice A prior index that does not answer the cross-generation query, or answers it with the
    /// wrong width, would revert every `registerRoot` — bricking issuance for the whole generation with no
    /// way to repoint.
    function test_a_prior_index_that_does_not_answer_is_refused() public {
        address bad = address(new AnswersNothing());
        vm.expectRevert(abi.encodeWithSelector(DogTagIssuerFactoryV2.PriorIndexDoesNotAnswer.selector, bad));
        new DogTagIssuerFactoryV2(address(impl), address(authority), bad);

        // A generation-1 factory is a real contract that answers `rootIssuer` — and NOT the
        // cross-generation query, which is why it can no longer occupy this slot directly.
        vm.expectRevert(
            abi.encodeWithSelector(DogTagIssuerFactoryV2.PriorIndexDoesNotAnswer.selector, address(gen1))
        );
        new DogTagIssuerFactoryV2(address(impl), address(authority), address(gen1));
    }

    function test_a_prior_index_answering_the_wrong_width_is_refused() public {
        address bad = address(new WrongWidthAnchors());
        vm.expectRevert(abi.encodeWithSelector(DogTagIssuerFactoryV2.PriorIndexDoesNotAnswer.selector, bad));
        new DogTagIssuerFactoryV2(address(impl), address(authority), bad);
    }

    /// @notice An occupant claiming every root would refuse every anchor — permanently, since the slot is
    /// immutable. Caught by requiring `false` for a root nothing has anchored.
    function test_a_prior_index_that_claims_every_root_is_refused() public {
        address bad = address(new AllAnchored());
        vm.expectRevert(abi.encodeWithSelector(DogTagIssuerFactoryV2.PriorIndexDoesNotAnswer.selector, bad));
        new DogTagIssuerFactoryV2(address(impl), address(authority), bad);
    }

    /// @notice The residual, pinned as a limitation rather than as a passing property: a contract that
    /// merely ANSWERS the query conformingly is accepted, and a stub that always says `false` is
    /// indistinguishable from a real router at construction. Nothing on chain can tell them apart, so
    /// wiring a real router over every earlier generation is a cutover precondition, not something this
    /// constructor can enforce.
    function test_a_conforming_stub_prior_index_is_accepted_and_that_is_a_residual() public {
        DogTagIssuerFactoryV2 stubbed =
            new DogTagIssuerFactoryV2(address(impl), address(authority), address(new StubAnchors()));

        vm.prank(admin);
        authority.addFactoryGeneration(address(stubbed));
        vm.prank(providerA);
        DogTagIssuerV2 clone = DogTagIssuerV2(stubbed.createIssuer(PROVIDER_A, VACCINATION, 0));
        _commission(clone, PROVIDER_A, providerA);

        // Generation 1 already holds ROOT_1 and has revoked it, and this factory's guard cannot see it.
        _anchorAndRevokeOnGeneration1(ROOT_1);
        assertFalse(gen1Clone.isValid(ROOT_1));
        vm.prank(providerA);
        clone.issue(ROOT_1);
        assertEq(stubbed.rootIssuer(ROOT_1), address(clone), "the stub is what let this through");
    }

    // =============================================================================================
    // The property that must not regress: write-once, per contract, honest
    // =============================================================================================

    function test_register_root_stays_clone_only_and_write_once() public {
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);

        // Only a clone may register.
        vm.prank(stranger);
        vm.expectRevert(bytes("!clone"));
        factory.registerRoot(ROOT_1);

        vm.prank(providerA);
        clone.issue(ROOT_1);
        assertEq(factory.rootIssuer(ROOT_1), address(clone));

        // Write-once in the factory: a second clone cannot claim the same root.
        DogTagIssuerV2 other = _live(PROVIDER_B, providerB, VACCINATION, 0);
        vm.prank(providerB);
        vm.expectRevert(bytes("root taken"));
        other.issue(ROOT_1);

        // Write-once in the clone: even its own issuer cannot re-issue.
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.BadRoot.selector);
        clone.issue(ROOT_1);
    }

    /// @notice The write-side half of the provenance router's resolution order (S-8). The guards are per
    /// contract, so a root anchored and then REVOKED on a generation-1 clone would otherwise be
    /// re-anchorable on a generation-2 clone — and under newest-first resolution the revoked credential
    /// would verify again. With the router wired as `priorIndex`, the duplicate never comes into
    /// existence.
    function test_a_revoked_prior_generation_root_cannot_be_re_anchored() public {
        _anchorAndRevokeOnGeneration1(ROOT_1);
        assertFalse(gen1Clone.isValid(ROOT_1));
        assertEq(gen1.rootIssuer(ROOT_1), address(gen1Clone));

        DogTagIssuerV2 gen2Clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        vm.expectRevert(bytes("root taken upstream"));
        gen2Clone.issue(ROOT_1);

        // The resurrection never happened, and the generation-2 clone is otherwise fully functional.
        assertEq(factory.rootIssuer(ROOT_1), address(0));
        vm.prank(providerA);
        gen2Clone.issue(ROOT_2);
        assertEq(factory.rootIssuer(ROOT_2), address(gen2Clone));
    }

    /// @notice The guard spans EVERY generation the router carries, not just the one immediately before —
    /// which is the whole reason it asks the router's cross-generation query rather than a single
    /// factory's generation-local `rootIssuer`.
    function test_the_upstream_guard_spans_every_generation_the_router_carries() public {
        // A second generation-2-shaped factory joins the router as generation 3.
        DogTagIssuerFactoryV2 gen3 =
            new DogTagIssuerFactoryV2(address(impl), address(authority), address(router));
        vm.prank(admin);
        router.appendGeneration(address(gen3));
        vm.prank(admin);
        authority.addFactoryGeneration(address(gen3));

        DogTagIssuerV2 gen2Clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        gen2Clone.issue(ROOT_1);

        vm.prank(providerA);
        DogTagIssuerV2 gen3Clone = DogTagIssuerV2(gen3.createIssuer(PROVIDER_A, VACCINATION, 0));
        _commission(gen3Clone, PROVIDER_A, providerA);

        vm.prank(providerA);
        vm.expectRevert(bytes("root taken upstream"));
        gen3Clone.issue(ROOT_1);

        // And a root only the OLDEST generation holds is refused there too.
        _anchorAndRevokeOnGeneration1(ROOT_2);
        vm.prank(providerA);
        vm.expectRevert(bytes("root taken upstream"));
        gen3Clone.issue(ROOT_2);
    }

    /// @notice The cutover ordering, stated as a test because the reverse reading — that pointing a
    /// factory at the router would be circular — is wrong and would misdirect whoever wires generation 3.
    /// The router is append-only precisely so it can be deployed FIRST.
    function test_the_router_topology_is_router_first_then_factory_then_append() public view {
        assertEq(router.generationAt(0), address(gen1), "generation 1 must resolve first");
        assertEq(router.generationAt(1), address(factory), "generation 2 was appended after the fact");
        assertEq(address(factory.priorIndex()), address(router), "the factory points at the router");
        // And the router recognises clones of both generations through one read.
        assertEq(router.generationCount(), 2);
    }

    // =============================================================================================
    // Event compatibility, asserted against generation 1's own signature strings
    // =============================================================================================

    /// @notice `topic0` is derived from the literal generation-1 signature string, not from this
    /// generation's own declaration — so a widened event would fail here even though it would still be
    /// self-consistent. A changed topic0 silently drops the log from the oversight indexer's feed.
    function test_the_legacy_creation_and_anchoring_event_topics_are_unchanged() public {
        vm.recordLogs();
        DogTagIssuerV2 clone = _live(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(providerA);
        clone.issue(ROOT_1);
        vm.prank(providerA);
        clone.revoke(ROOT_1);
        Vm.Log[] memory logs = vm.getRecordedLogs();

        bytes32 created = keccak256("IssuerCreated(address,bytes32,string)");
        bytes32 registered = keccak256("RootRegistered(bytes32,address)");
        bytes32 issued = keccak256("RootIssued(bytes32,address,uint256)");
        bytes32 revoked = keccak256("RootRevoked(bytes32,address,uint256)");

        assertTrue(_sawTopic(logs, created, address(factory)), "IssuerCreated topic0 moved");
        assertTrue(_sawTopic(logs, registered, address(factory)), "RootRegistered topic0 moved");
        assertTrue(_sawTopic(logs, issued, address(clone)), "RootIssued topic0 moved");
        assertTrue(_sawTopic(logs, revoked, address(clone)), "RootRevoked topic0 moved");

        // The creation log's indexed fields, and its deliberately empty name.
        for (uint256 i; i < logs.length; i++) {
            if (logs[i].topics[0] != created) continue;
            assertEq(logs[i].topics[1], bytes32(uint256(uint160(address(clone)))));
            assertEq(logs[i].topics[2], VACCINATION);
            assertEq(abi.decode(logs[i].data, (string)), "", "a name was emitted");
        }
    }

    /// @notice The owner arrives as an ADDITIVE event rather than by widening `IssuerCreated`, so one log
    /// filter on the factory captures the whole creation without moving an existing topic.
    function test_the_owner_and_provider_arrive_in_an_additive_event() public {
        address predicted = factory.predictIssuer(VACCINATION, providerA, 5);

        vm.expectEmit(true, true, true, true, address(factory));
        emit DogTagIssuerFactoryV2.IssuerOwnerRegistered(predicted, providerA, PROVIDER_A, 5);
        _create(PROVIDER_A, providerA, VACCINATION, 5);
    }

    // =============================================================================================
    // No admin surface
    // =============================================================================================

    /// @notice The factory holds no owner and no privileged function, so there is nothing about it to
    /// capture or repoint — the property `IssuerDomainRegistry` asks of a factory reference.
    ///
    /// @dev Both probes use WELL-FORMED calldata, and the mutating one is a real `call` rather than a
    /// `staticcall`. Both matter: malformed calldata makes a genuine function revert on decode, and a
    /// `staticcall` makes a genuine state-changing function revert on the write — so the earlier form of
    /// this test would have passed even if `transferOwnership` existed and worked. What a failing call
    /// establishes is that the SELECTOR is not reachable: no such function and no fallback. It would also
    /// pass for a function that exists and reverts, so it is not evidence that the source declares none;
    /// that part is held by review. Only the two selectors named here are covered.
    function test_the_factory_has_no_owner_or_transfer_ownership_selector() public {
        (bool answered,) = address(factory).staticcall(abi.encodeCall(Ownable.owner, ()));
        assertFalse(answered, "the factory answers owner()");

        (answered,) = address(factory).call(abi.encodeCall(Ownable.transferOwnership, (stranger)));
        assertFalse(answered, "the factory answers transferOwnership(address)");

        (answered,) = address(factory).call(abi.encodeCall(Ownable.renounceOwnership, ()));
        assertFalse(answered, "the factory answers renounceOwnership()");
    }

    /// @notice The implementation is locked, so it can never be initialized and adopted directly.
    function test_the_implementation_cannot_be_initialized() public {
        vm.expectRevert(Initializable.InvalidInitialization.selector);
        impl.initialize(VACCINATION, address(authority), address(factory), providerA);
    }

    function test_a_clone_cannot_be_re_initialized() public {
        DogTagIssuerV2 clone = _create(PROVIDER_A, providerA, VACCINATION, 0);
        vm.prank(stranger);
        vm.expectRevert(Initializable.InvalidInitialization.selector);
        clone.initialize(TRAVEL, address(authority), address(factory), stranger);
    }

    // =============================================================================================
    // Helpers
    // =============================================================================================

    /// @dev Anchor `root` on the real generation-1 clone and revoke it, so the credential must stay dead.
    function _anchorAndRevokeOnGeneration1(bytes32 root) internal {
        vm.startPrank(providerA);
        gen1Clone.issue(root);
        gen1Clone.revoke(root);
        vm.stopPrank();
    }

    function _sawTopic(Vm.Log[] memory logs, bytes32 topic0, address emitter) internal pure returns (bool) {
        for (uint256 i; i < logs.length; i++) {
            if (logs[i].emitter == emitter && logs[i].topics[0] == topic0) return true;
        }
        return false;
    }
}
