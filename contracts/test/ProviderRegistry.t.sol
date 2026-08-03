// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {Ownable, Ownable2Step} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {IProviderRegistry, ProviderRegistry} from "../src/ProviderRegistry.sol";

contract ProviderRegistryServiceMock is Ownable2Step {
    bytes32 public immutable recordType;

    error RenounceDisabled();

    constructor(bytes32 recordType_, address initialOwner) Ownable(initialOwner) {
        recordType = recordType_;
    }

    function renounceOwnership() public pure override {
        revert RenounceDisabled();
    }
}

contract ProviderRegistryOwnerlessServiceMock {
    bytes32 public immutable recordType;

    constructor(bytes32 recordType_) {
        recordType = recordType_;
    }
}

contract ProviderRegistryFactoryMock {
    mapping(address => bool) public isClone;

    function canCreate(IProviderRegistry registry, bytes20 providerId, bytes32 recordType, address caller)
        external
        view
        returns (bool)
    {
        return registry.canCreateService(providerId, recordType, caller);
    }

    function create(bytes32 recordType, address serviceOwner)
        external
        returns (ProviderRegistryServiceMock created)
    {
        created = new ProviderRegistryServiceMock(recordType, serviceOwner);
        isClone[address(created)] = true;
    }

    /// @dev Test-only mutation used to prove the core re-checks provenance at write time instead of
    /// trusting the attachment forever. The real factory's mapping has no equivalent clearing path.
    function setRecognized(address candidate, bool recognized) external {
        isClone[candidate] = recognized;
    }
}

contract ProviderRegistryMalformedFactoryMock {
    function isClone(address) external pure returns (bool) {
        assembly ("memory-safe") {
            mstore(0, 2)
            return(0, 0x20)
        }
    }
}

contract ProviderRegistryMalformedOwnerMock {
    bytes32 public immutable recordType;

    constructor(bytes32 recordType_) {
        recordType = recordType_;
    }

    function owner() external pure returns (address) {
        assembly ("memory-safe") {
            mstore(0, shl(160, 1))
            return(0, 0x20)
        }
    }
}

contract ProviderRegistryZeroOwnerMock {
    function owner() external pure returns (address) {
        return address(0);
    }
}

contract ProviderRegistryResolverMock {}

/// @notice S-6: provider identity and authority core.
///
/// Headline invariants:
///  * metadata authorization is KYC standing AND owner/delegate authority, with operational signers
///    deliberately separate;
///  * provider, service-owner and registry authority rotations are exercised as real handovers, and
///    old keys lose actual write power;
///  * a service's factory, record type and owner are resolved on chain, never accepted as a supplied
///    claim; and
///  * exact IssuerRegistry compatibility selectors preserve service scoping and the orthogonal
///    verifier-relayer axis.
contract ProviderRegistryTest is Test {
    ProviderRegistry internal registry;
    ProviderRegistryFactoryMock internal factoryA;
    ProviderRegistryFactoryMock internal factoryB;
    ProviderRegistryServiceMock internal serviceA;

    address internal constant AUTHORITY = address(0xA11CE);
    address internal constant NEXT_AUTHORITY = address(0xA11CE2);
    address internal constant CONTROLLER = address(0xC011);
    address internal constant NEXT_CONTROLLER = address(0xC012);
    address internal constant SERVICE_OWNER = address(0x5E01);
    address internal constant NEXT_SERVICE_OWNER = address(0x5E02);
    address internal constant ISSUANCE_SIGNER = address(0x1550);
    address internal constant OTHER_SIGNER = address(0x1551);
    address internal constant DELEGATE = address(0xDE1E);
    address internal constant STRANGER = address(0xBAD);

    // The rights layout, mirrored so a test never calls a getter mid-`vm.prank` (the prank would be
    // consumed by that call). `test_the_bit_layout_is_pinned_where_a_reorder_reddens` holds these
    // against the contract's own constants, which is what makes the never-reorder rule enforceable.
    uint256 internal constant R_ISSUE = 1 << 0;
    uint256 internal constant R_PROTOCOL_ADMIN = 1 << 1;
    uint256 internal constant R_SERVICE_ATTACHED = 1 << 2;
    uint256 internal constant R_SERVICE_OWNER_CONFIRMED = 1 << 3;
    uint256 internal constant R_SERVICE_STANDING = 1 << 4;
    uint256 internal constant R_SERVICE_CURRENT = 1 << 5;
    address internal constant RELAYER = address(0xBEEF);

    bytes20 internal providerA = hex"9d7c0e341a85b69f2c08d1ee4f6a7398bc52d401";
    bytes20 internal providerB = hex"42a16f8c73d9e205b84c0a6f9137de52c80b4a19";
    bytes20 internal providerC = hex"f10b62c4938e57ad20c6417b9e035ac864d2ef70";

    bytes32 internal constant GENERATION_A = keccak256("dogtag/factory/a");
    bytes32 internal constant GENERATION_B = keccak256("dogtag/factory/b");
    bytes32 internal constant RECORD_TYPE = keccak256("VACCINATION");
    bytes32 internal constant OTHER_RECORD_TYPE = keccak256("TRAVEL_CLEARANCE");
    bytes32 internal constant PURPOSE = keccak256("BOARDING");
    bytes32 internal constant IDENTITY_DIGEST = keccak256("publication-safe-identity-a");
    uint32 internal constant PROVIDER_DIRECTORY_PERMISSION = 1 << 1;
    uint32 internal constant PROVIDER_CREATE_SERVICE_PERMISSION = 1 << 2;
    uint32 internal constant SERVICE_RECORD_PERMISSION = 1 << 0;
    uint32 internal constant SERVICE_DOMAIN_PERMISSION = 1 << 1;
    uint32 internal constant SERVICE_REPOINT_PERMISSION = 1 << 2;

    function setUp() public {
        registry = new ProviderRegistry(AUTHORITY);
        factoryA = new ProviderRegistryFactoryMock();
        factoryB = new ProviderRegistryFactoryMock();

        vm.startPrank(AUTHORITY);
        registry.addFactoryGeneration(GENERATION_A, address(factoryA));
        registry.addFactoryGeneration(GENERATION_B, address(factoryB));
        _register(providerA, CONTROLLER, IDENTITY_DIGEST);
        registry.setProviderStanding(providerA, ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();

        serviceA = factoryA.create(RECORD_TYPE, SERVICE_OWNER);
        vm.startPrank(AUTHORITY);
        registry.attachService(providerA, address(serviceA), GENERATION_A, SERVICE_OWNER);
        registry.setServiceStanding(address(serviceA), ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();
        vm.prank(SERVICE_OWNER);
        registry.repointService(address(serviceA));
    }

    function _register(bytes20 providerId, address controller, bytes32 digest) internal {
        registry.registerProvider(
            providerId, controller, digest, 1, 0xe3, 0x12, bytes("ipfs://publication-safe-provider-identity")
        );
    }

    function _createAndAttach(
        ProviderRegistryFactoryMock factory,
        bytes32 generation,
        bytes20 providerId,
        bytes32 recordType,
        address serviceOwner
    ) internal returns (ProviderRegistryServiceMock created) {
        created = factory.create(recordType, serviceOwner);
        vm.startPrank(AUTHORITY);
        registry.attachService(providerId, address(created), generation, serviceOwner);
        registry.setServiceStanding(address(created), ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();
    }

    function _approveResolver(ProviderRegistry.ResolverKind kind, address resolver) internal {
        vm.prank(AUTHORITY);
        registry.setResolverApproved(kind, resolver, true);
    }

    // ---------------------------------------------------------------------------------------------
    // The captain's two independent predicates
    // ---------------------------------------------------------------------------------------------

    function test_service_write_requires_active_kyc_and_owner_authority_as_independent_predicates() public {
        vm.prank(AUTHORITY);
        registry.setRights(ISSUANCE_SIGNER, R_ISSUE);

        // Predicate 1 + predicate 2: a cleared provider and the confirmed live owner.
        assertTrue(registry.canWriteService(address(serviceA), SERVICE_OWNER, SERVICE_DOMAIN_PERMISSION));
        // The owner is deliberately NOT the issuance signer.
        assertFalse(registry.canIssue(address(serviceA), SERVICE_OWNER));

        // A hot operational signer has issuance authority, not metadata authority.
        assertTrue(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        assertFalse(registry.canWriteService(address(serviceA), ISSUANCE_SIGNER, SERVICE_DOMAIN_PERMISSION));

        // Owner authority alone cannot bypass suspended KYC standing.
        vm.prank(AUTHORITY);
        registry.setProviderStanding(providerA, ProviderRegistry.Standing.SUSPENDED);
        assertFalse(registry.canWriteService(address(serviceA), SERVICE_OWNER, SERVICE_DOMAIN_PERMISSION));

        vm.prank(AUTHORITY);
        registry.setProviderStanding(providerA, ProviderRegistry.Standing.ACTIVE);
        vm.prank(SERVICE_OWNER);
        registry.setServiceDelegate(
            address(serviceA), DELEGATE, SERVICE_DOMAIN_PERMISSION, uint64(block.timestamp + 1 days)
        );
        assertTrue(registry.canWriteService(address(serviceA), DELEGATE, SERVICE_DOMAIN_PERMISSION));
        assertFalse(registry.canWriteService(address(serviceA), DELEGATE, SERVICE_REPOINT_PERMISSION));

        vm.warp(block.timestamp + 1 days + 1);
        assertFalse(registry.canWriteService(address(serviceA), DELEGATE, SERVICE_DOMAIN_PERMISSION));
    }

    // ---------------------------------------------------------------------------------------------
    // Real key rotations: controller, clone owner, and the one registry authority
    // ---------------------------------------------------------------------------------------------

    function test_controller_rotation_request_accept_confirm_revokes_old_key_and_enables_new_key() public {
        ProviderRegistryResolverMock resolver = new ProviderRegistryResolverMock();
        _approveResolver(ProviderRegistry.ResolverKind.DIRECTORY, address(resolver));

        vm.prank(CONTROLLER);
        registry.setProviderDelegate(
            providerA, DELEGATE, PROVIDER_DIRECTORY_PERMISSION, uint64(block.timestamp + 7 days)
        );

        vm.prank(CONTROLLER);
        registry.requestControllerTransfer(providerA, NEXT_CONTROLLER);

        vm.prank(STRANGER);
        vm.expectRevert(ProviderRegistry.NotPendingController.selector);
        registry.acceptControllerTransfer(providerA);

        vm.prank(NEXT_CONTROLLER);
        vm.expectRevert(ProviderRegistry.Unauthorized.selector);
        registry.setDirectoryResolver(providerA, address(resolver));

        vm.prank(NEXT_CONTROLLER);
        registry.acceptControllerTransfer(providerA);
        uint64 reviewedRequestNonce = registry.provider(providerA).controllerRequestNonce;

        // Acceptance proves key control but does not move KYC identity authority.
        vm.prank(NEXT_CONTROLLER);
        vm.expectRevert(ProviderRegistry.Unauthorized.selector);
        registry.setDirectoryResolver(providerA, address(resolver));

        // A replacement between registrar review and mining cannot silently change the accepted key.
        vm.prank(CONTROLLER);
        registry.requestControllerTransfer(providerA, STRANGER);
        vm.prank(STRANGER);
        registry.acceptControllerTransfer(providerA);
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.UnexpectedControllerTransfer.selector);
        registry.confirmControllerTransfer(providerA, NEXT_CONTROLLER, reviewedRequestNonce);

        vm.prank(CONTROLLER);
        registry.requestControllerTransfer(providerA, NEXT_CONTROLLER);
        vm.prank(NEXT_CONTROLLER);
        registry.acceptControllerTransfer(providerA);
        ProviderRegistry.Provider memory accepted = registry.provider(providerA);
        vm.prank(AUTHORITY);
        registry.confirmControllerTransfer(providerA, NEXT_CONTROLLER, accepted.controllerRequestNonce);

        ProviderRegistry.Provider memory p = registry.provider(providerA);
        assertEq(p.controller, NEXT_CONTROLLER);
        assertEq(p.pendingController, address(0));
        assertFalse(p.pendingControllerAccepted);
        assertFalse(p.pendingControllerRequestedByRegistrar);
        assertEq(p.controllerEpoch, 2);

        vm.prank(CONTROLLER);
        vm.expectRevert(ProviderRegistry.Unauthorized.selector);
        registry.setDirectoryResolver(providerA, address(resolver));

        vm.prank(DELEGATE);
        vm.expectRevert(ProviderRegistry.Unauthorized.selector);
        registry.setDirectoryResolver(providerA, address(resolver));

        vm.prank(NEXT_CONTROLLER);
        registry.setDirectoryResolver(providerA, address(resolver));
        assertEq(registry.provider(providerA).directoryResolver, address(resolver));
    }

    function test_real_clone_owner_rotation_quarantines_then_old_loses_power() public {
        ProviderRegistryResolverMock resolverOne = new ProviderRegistryResolverMock();
        ProviderRegistryResolverMock resolverTwo = new ProviderRegistryResolverMock();
        _approveResolver(ProviderRegistry.ResolverKind.DOMAIN, address(resolverOne));
        _approveResolver(ProviderRegistry.ResolverKind.DOMAIN, address(resolverTwo));

        vm.prank(SERVICE_OWNER);
        registry.setServiceDelegate(
            address(serviceA), DELEGATE, SERVICE_DOMAIN_PERMISSION, uint64(block.timestamp + 7 days)
        );
        vm.prank(SERVICE_OWNER);
        registry.setDomainResolver(address(serviceA), address(resolverOne));

        // This is the real OZ request/accept path on an owner-bearing clone.
        vm.prank(SERVICE_OWNER);
        serviceA.transferOwnership(NEXT_SERVICE_OWNER);
        assertEq(serviceA.owner(), SERVICE_OWNER);
        assertEq(serviceA.pendingOwner(), NEXT_SERVICE_OWNER);

        vm.prank(NEXT_SERVICE_OWNER);
        serviceA.acceptOwnership();
        assertEq(serviceA.owner(), NEXT_SERVICE_OWNER);

        // Live owner != KYC-confirmed owner: both keys and the old epoch delegate are quarantined.
        assertFalse(registry.canWriteService(address(serviceA), SERVICE_OWNER, SERVICE_DOMAIN_PERMISSION));
        assertFalse(
            registry.canWriteService(address(serviceA), NEXT_SERVICE_OWNER, SERVICE_DOMAIN_PERMISSION)
        );
        assertFalse(registry.canWriteService(address(serviceA), DELEGATE, SERVICE_DOMAIN_PERMISSION));

        vm.prank(NEXT_SERVICE_OWNER);
        vm.expectRevert(ProviderRegistry.Unauthorized.selector);
        registry.setDomainResolver(address(serviceA), address(resolverTwo));

        // A second accepted clone rotation cannot make the registrar confirm a different key than
        // the one it reviewed. The expected owner is a guard; owner() remains authoritative.
        vm.prank(NEXT_SERVICE_OWNER);
        serviceA.transferOwnership(STRANGER);
        vm.prank(STRANGER);
        serviceA.acceptOwnership();
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.UnexpectedServiceOwner.selector);
        registry.confirmServiceOwner(address(serviceA), NEXT_SERVICE_OWNER, 1);

        vm.prank(STRANGER);
        serviceA.transferOwnership(NEXT_SERVICE_OWNER);
        vm.prank(NEXT_SERVICE_OWNER);
        serviceA.acceptOwnership();
        vm.prank(AUTHORITY);
        registry.confirmServiceOwner(address(serviceA), NEXT_SERVICE_OWNER, 1);
        ProviderRegistry.Service memory s = registry.service(address(serviceA));
        assertEq(s.confirmedOwner, NEXT_SERVICE_OWNER);
        assertEq(s.ownerEpoch, 2);

        vm.prank(SERVICE_OWNER);
        vm.expectRevert(ProviderRegistry.Unauthorized.selector);
        registry.setDomainResolver(address(serviceA), address(resolverTwo));
        vm.prank(DELEGATE);
        vm.expectRevert(ProviderRegistry.Unauthorized.selector);
        registry.setDomainResolver(address(serviceA), address(resolverTwo));

        vm.prank(NEXT_SERVICE_OWNER);
        registry.setDomainResolver(address(serviceA), address(resolverTwo));
        assertEq(registry.service(address(serviceA)).domainResolver, address(resolverTwo));

        vm.prank(NEXT_SERVICE_OWNER);
        vm.expectRevert(ProviderRegistryServiceMock.RenounceDisabled.selector);
        serviceA.renounceOwnership();
        assertEq(serviceA.owner(), NEXT_SERVICE_OWNER);

        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.NoChange.selector);
        registry.confirmServiceOwner(address(serviceA), NEXT_SERVICE_OWNER, 2);
    }

    function test_registrar_recovery_request_is_locked_but_can_be_cancelled_without_liveness_loss() public {
        vm.prank(AUTHORITY);
        registry.requestControllerTransfer(providerA, NEXT_CONTROLLER);
        ProviderRegistry.Provider memory recovery = registry.provider(providerA);
        assertTrue(recovery.pendingControllerRequestedByRegistrar);

        vm.prank(CONTROLLER);
        vm.expectRevert(ProviderRegistry.Unauthorized.selector);
        registry.requestControllerTransfer(providerA, STRANGER);

        vm.prank(AUTHORITY);
        registry.cancelControllerTransfer(providerA);
        ProviderRegistry.Provider memory cancelled = registry.provider(providerA);
        assertEq(cancelled.pendingController, address(0));
        assertFalse(cancelled.pendingControllerAccepted);
        assertFalse(cancelled.pendingControllerRequestedByRegistrar);
        assertEq(cancelled.controllerRequestNonce, recovery.controllerRequestNonce + 1);

        // Cancelling the registrar recovery restores the controller's ordinary rotation path.
        vm.prank(CONTROLLER);
        registry.requestControllerTransfer(providerA, STRANGER);
        vm.prank(AUTHORITY);
        registry.cancelControllerTransfer(providerA);

        vm.prank(AUTHORITY);
        registry.requestControllerTransfer(providerA, NEXT_CONTROLLER);
        ProviderRegistry.Provider memory replacementRecovery = registry.provider(providerA);
        vm.prank(NEXT_CONTROLLER);
        registry.acceptControllerTransfer(providerA);
        vm.prank(AUTHORITY);
        registry.confirmControllerTransfer(
            providerA, NEXT_CONTROLLER, replacementRecovery.controllerRequestNonce
        );
        assertEq(registry.provider(providerA).controller, NEXT_CONTROLLER);
    }

    function test_real_admin_key_rotation_old_key_loses_authority_and_new_key_can_act() public {
        assertTrue(registry.hasRole(registry.DEFAULT_ADMIN_ROLE(), AUTHORITY));
        assertTrue(registry.hasRole(registry.WHITELIST_ADMIN(), AUTHORITY));

        vm.prank(AUTHORITY);
        registry.transferOwnership(NEXT_AUTHORITY);
        assertEq(registry.pendingOwner(), NEXT_AUTHORITY);

        vm.prank(NEXT_AUTHORITY);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, NEXT_AUTHORITY));
        _register(providerB, NEXT_CONTROLLER, keccak256("identity-b"));

        vm.prank(NEXT_AUTHORITY);
        registry.acceptOwnership();

        assertEq(registry.owner(), NEXT_AUTHORITY);
        assertFalse(registry.hasRole(registry.DEFAULT_ADMIN_ROLE(), AUTHORITY));
        assertFalse(registry.hasRole(registry.WHITELIST_ADMIN(), AUTHORITY));
        assertTrue(registry.hasRole(registry.DEFAULT_ADMIN_ROLE(), NEXT_AUTHORITY));
        assertTrue(registry.hasRole(registry.WHITELIST_ADMIN(), NEXT_AUTHORITY));

        vm.prank(AUTHORITY);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, AUTHORITY));
        _register(providerB, NEXT_CONTROLLER, keccak256("identity-b"));

        vm.prank(NEXT_AUTHORITY);
        _register(providerB, NEXT_CONTROLLER, keccak256("identity-b"));
        assertEq(registry.provider(providerB).controller, NEXT_CONTROLLER);
    }

    function test_registry_authority_cannot_be_renounced_in_one_step() public {
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.RenounceDisabled.selector);
        registry.renounceOwnership();
        assertEq(registry.owner(), AUTHORITY);
    }

    // ---------------------------------------------------------------------------------------------
    // Provenance, immutable factory generations, and genuine-clone repointing
    // ---------------------------------------------------------------------------------------------

    function test_attach_resolves_registered_factory_and_clone_metadata_not_supplied_claims() public {
        ProviderRegistryServiceMock fromFactoryB = factoryB.create(OTHER_RECORD_TYPE, NEXT_SERVICE_OWNER);

        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.NotFactoryClone.selector);
        registry.attachService(providerA, address(fromFactoryB), GENERATION_A, NEXT_SERVICE_OWNER);

        // KYC reviewed NEXT_SERVICE_OWNER, but the clone completed another handover before the
        // attachment transaction. The expected owner is only a race guard; owner() is resolved.
        vm.prank(NEXT_SERVICE_OWNER);
        fromFactoryB.transferOwnership(STRANGER);
        vm.prank(STRANGER);
        fromFactoryB.acceptOwnership();
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.UnexpectedServiceOwner.selector);
        registry.attachService(providerA, address(fromFactoryB), GENERATION_B, NEXT_SERVICE_OWNER);

        vm.prank(STRANGER);
        fromFactoryB.transferOwnership(NEXT_SERVICE_OWNER);
        vm.prank(NEXT_SERVICE_OWNER);
        fromFactoryB.acceptOwnership();
        vm.prank(AUTHORITY);
        registry.attachService(providerA, address(fromFactoryB), GENERATION_B, NEXT_SERVICE_OWNER);

        ProviderRegistry.Service memory attached = registry.service(address(fromFactoryB));
        assertEq(bytes32(attached.providerId), bytes32(providerA));
        assertEq(attached.factoryGeneration, GENERATION_B);
        assertEq(attached.recordType, OTHER_RECORD_TYPE);
        assertEq(attached.confirmedOwner, NEXT_SERVICE_OWNER);

        // A look-alike implementing owner()/recordType() is still not a factory clone.
        ProviderRegistryServiceMock forged = new ProviderRegistryServiceMock(RECORD_TYPE, SERVICE_OWNER);
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.NotFactoryClone.selector);
        registry.attachService(providerA, address(forged), GENERATION_A, SERVICE_OWNER);

        // An owner-less legacy-shaped clone cannot be called an owner-confirmed service.
        ProviderRegistryOwnerlessServiceMock ownerless = new ProviderRegistryOwnerlessServiceMock(RECORD_TYPE);
        factoryA.setRecognized(address(ownerless), true);
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.InvalidServiceMetadata.selector);
        registry.attachService(providerA, address(ownerless), GENERATION_A, SERVICE_OWNER);

        // Malformed ABI words fail closed instead of bubbling abi.decode panics.
        ProviderRegistryMalformedOwnerMock malformedOwner =
            new ProviderRegistryMalformedOwnerMock(RECORD_TYPE);
        factoryA.setRecognized(address(malformedOwner), true);
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.InvalidServiceMetadata.selector);
        registry.attachService(providerA, address(malformedOwner), GENERATION_A, SERVICE_OWNER);

        ProviderRegistryMalformedFactoryMock malformedFactory = new ProviderRegistryMalformedFactoryMock();
        bytes32 malformedGeneration = keccak256("dogtag/factory/malformed");
        vm.prank(AUTHORITY);
        registry.addFactoryGeneration(malformedGeneration, address(malformedFactory));
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.NotFactoryClone.selector);
        registry.attachService(providerA, address(forged), malformedGeneration, SERVICE_OWNER);
    }

    function test_factory_generation_address_is_write_once_and_deprecation_is_terminal() public {
        ProviderRegistryFactoryMock replacement = new ProviderRegistryFactoryMock();

        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.AlreadyRegistered.selector);
        registry.addFactoryGeneration(GENERATION_A, address(replacement));

        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.DuplicateFactory.selector);
        registry.addFactoryGeneration(keccak256("alias-generation"), address(factoryA));

        vm.prank(AUTHORITY);
        registry.deprecateFactoryGeneration(GENERATION_A);
        ProviderRegistry.FactoryGeneration memory generation = registry.factoryGeneration(GENERATION_A);
        assertEq(generation.factory, address(factoryA));
        assertFalse(generation.active);
        assertTrue(generation.deprecatedAtBlock != 0);

        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.FactoryGenerationInactive.selector);
        registry.deprecateFactoryGeneration(GENERATION_A);

        ProviderRegistryServiceMock later = factoryA.create(RECORD_TYPE, SERVICE_OWNER);
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.FactoryGenerationInactive.selector);
        registry.attachService(providerA, address(later), GENERATION_A, SERVICE_OWNER);

        // History remains readable, while current authority is disabled.
        assertEq(registry.service(address(serviceA)).factoryGeneration, GENERATION_A);
        assertFalse(registry.canWriteService(address(serviceA), SERVICE_OWNER, SERVICE_RECORD_PERMISSION));
    }

    function test_repoint_accepts_only_attached_genuine_clone_and_rechecks_provenance() public {
        ProviderRegistryServiceMock replacement =
            _createAndAttach(factoryA, GENERATION_A, providerA, RECORD_TYPE, SERVICE_OWNER);

        assertEq(registry.currentService(providerA, RECORD_TYPE), address(serviceA));
        // ONE grant, on the signer's address, reaching both services. `canIssue` still separates them,
        // on the provider's current pointer rather than on the grant.
        vm.prank(AUTHORITY);
        registry.setRights(ISSUANCE_SIGNER, R_ISSUE);
        assertTrue(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        assertFalse(registry.canIssue(address(replacement), ISSUANCE_SIGNER));

        vm.prank(STRANGER);
        vm.expectRevert(ProviderRegistry.Unauthorized.selector);
        registry.repointService(address(replacement));

        vm.prank(SERVICE_OWNER);
        registry.repointService(address(replacement));
        assertEq(registry.currentService(providerA, RECORD_TYPE), address(replacement));
        assertFalse(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        assertTrue(registry.canIssue(address(replacement), ISSUANCE_SIGNER));
        // Historical roots stay revocable at the old service through S-7's distinct revoke gate.
        assertTrue(registry.canRevoke(address(serviceA), ISSUANCE_SIGNER));
        vm.prank(address(serviceA));
        assertFalse(registry.isWhitelistedFor(RECORD_TYPE, ISSUANCE_SIGNER));

        ProviderRegistryServiceMock forged = new ProviderRegistryServiceMock(RECORD_TYPE, SERVICE_OWNER);
        vm.prank(SERVICE_OWNER);
        vm.expectRevert(ProviderRegistry.UnknownService.selector);
        registry.repointService(address(forged));

        // Proves repoint does not merely trust a provenance flag copied at attachment.
        factoryA.setRecognized(address(serviceA), false);
        vm.prank(SERVICE_OWNER);
        vm.expectRevert(ProviderRegistry.Unauthorized.selector);
        registry.repointService(address(serviceA));
        // Live factory recognition is deliberately absent from the revoke rung: it is an external
        // call whose answer a broken or hostile factory could withhold, and revocation is the safety
        // direction. Attachment already proved provenance and recorded it in state.
        assertTrue(registry.canRevoke(address(serviceA), ISSUANCE_SIGNER));
    }

    // ---------------------------------------------------------------------------------------------
    // Address-keyed issue rights and the orthogonal VERIFY axis
    // ---------------------------------------------------------------------------------------------

    /// @dev THE WIDENING, pinned as intended behaviour rather than left to be rediscovered as a bug.
    ///
    /// The grant was `_issuanceCapabilities[service][signer]`; it is now one bit on the signer's
    /// address. So a signer approved alongside one service reaches EVERY service in effective standing,
    /// including another provider's. A later reader who finds this surprising must take it to the
    /// captain rather than "fixing" it by reintroducing a service key into the lookup — that key is
    /// exactly what cannot exist, because at approval time the applicant's clone does not exist yet.
    function test_the_issue_right_is_on_the_address_and_reaches_every_service_in_standing() public {
        // A SECOND provider, so this is cross-tenant and not merely cross-service.
        vm.startPrank(AUTHORITY);
        _register(providerB, NEXT_CONTROLLER, keccak256("identity-b"));
        registry.setProviderStanding(providerB, ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();
        ProviderRegistryServiceMock serviceB =
            _createAndAttach(factoryA, GENERATION_A, providerB, RECORD_TYPE, SERVICE_OWNER);
        vm.prank(SERVICE_OWNER);
        registry.repointService(address(serviceB));

        vm.prank(AUTHORITY);
        registry.setRights(ISSUANCE_SIGNER, R_ISSUE);

        // One grant, both providers' services.
        assertTrue(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        assertTrue(registry.canIssue(address(serviceB), ISSUANCE_SIGNER));
        assertEq(registry.issueRightHolders(), 1);

        // A service in standing but NOT its provider's current selection is still refused: no
        // lifecycle term was dropped, they simply no longer say anything about which provider the
        // signer belongs to.
        ProviderRegistryServiceMock unselected =
            _createAndAttach(factoryA, GENERATION_A, providerB, OTHER_RECORD_TYPE, SERVICE_OWNER);
        assertFalse(registry.canIssue(address(unselected), ISSUANCE_SIGNER));

        // An address the registrar never granted is refused everywhere, and being an attached service
        // is not itself an issue right.
        assertFalse(registry.canIssue(address(serviceA), OTHER_SIGNER));
        assertFalse(registry.canIssue(address(serviceA), address(serviceB)));

        // The exact legacy selector is still clone-aware through msg.sender, and still answers the
        // orthogonal VERIFY axis for a caller that is not an attached service.
        vm.prank(address(serviceA));
        assertTrue(registry.isWhitelistedFor(RECORD_TYPE, ISSUANCE_SIGNER));
        vm.prank(address(serviceA));
        assertFalse(registry.isWhitelistedFor(OTHER_RECORD_TYPE, ISSUANCE_SIGNER));
        assertFalse(registry.isWhitelistedFor(RECORD_TYPE, ISSUANCE_SIGNER));
        assertTrue(registry.isRecognizedIssuer(address(serviceA), ISSUANCE_SIGNER));

        // The operational key still has no metadata-edit authority.
        assertFalse(registry.canWriteService(address(serviceA), ISSUANCE_SIGNER, SERVICE_RECORD_PERMISSION));

        vm.prank(AUTHORITY);
        registry.setRights(ISSUANCE_SIGNER, 0);
        assertFalse(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        assertFalse(registry.canIssue(address(serviceB), ISSUANCE_SIGNER));
        assertEq(registry.issueRightHolders(), 0);
    }

    /// @dev The layout is a WIRE FORMAT: off-chain readers fold `RightsSet` masks and every one of them
    /// hard-codes these positions. A bit silently reused or reordered grants the wrong right with no
    /// revert anywhere, so the positions are asserted literally here — a reorder reddens this and
    /// nothing else would.
    function test_the_bit_layout_is_pinned_where_a_reorder_reddens() public view {
        assertEq(registry.RIGHT_ISSUE(), R_ISSUE);
        assertEq(registry.RIGHT_PROTOCOL_ADMIN(), R_PROTOCOL_ADMIN);
        assertEq(registry.RIGHT_SERVICE_ATTACHED(), R_SERVICE_ATTACHED);
        assertEq(registry.RIGHT_SERVICE_OWNER_CONFIRMED(), R_SERVICE_OWNER_CONFIRMED);
        assertEq(registry.RIGHT_SERVICE_STANDING(), R_SERVICE_STANDING);
        assertEq(registry.RIGHT_SERVICE_CURRENT(), R_SERVICE_CURRENT);
        // Only the grant is settable; every other bit reports state this core already holds.
        assertEq(registry.SETTABLE_RIGHTS(), R_ISSUE);
    }

    /// @dev A bit is not a general-purpose "authorized" flag: setting one grants exactly its own right
    /// and nothing adjacent. Without this, a layout where two bits happened to be checked by the same
    /// predicate would look correct.
    function test_a_wrong_bit_grants_no_right() public {
        // Every derived bit is refused by the writer, so no caller can fabricate one.
        vm.startPrank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.UnsettableRights.selector);
        registry.setRights(ISSUANCE_SIGNER, R_PROTOCOL_ADMIN);
        vm.expectRevert(ProviderRegistry.UnsettableRights.selector);
        registry.setRights(ISSUANCE_SIGNER, R_SERVICE_ATTACHED);
        vm.expectRevert(ProviderRegistry.UnsettableRights.selector);
        registry.setRights(ISSUANCE_SIGNER, R_SERVICE_OWNER_CONFIRMED);
        vm.expectRevert(ProviderRegistry.UnsettableRights.selector);
        registry.setRights(ISSUANCE_SIGNER, R_SERVICE_STANDING);
        vm.expectRevert(ProviderRegistry.UnsettableRights.selector);
        registry.setRights(ISSUANCE_SIGNER, R_SERVICE_CURRENT);
        // Including one smuggled in beside a legitimate grant.
        vm.expectRevert(ProviderRegistry.UnsettableRights.selector);
        registry.setRights(ISSUANCE_SIGNER, R_ISSUE | R_PROTOCOL_ADMIN);
        // And an unallocated position, so a future bit cannot be pre-set against its own meaning.
        vm.expectRevert(ProviderRegistry.UnsettableRights.selector);
        registry.setRights(ISSUANCE_SIGNER, 1 << 6);
        vm.stopPrank();

        assertEq(registry.rightsOf(ISSUANCE_SIGNER), 0);
        assertFalse(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        assertFalse(registry.hasRole(registry.DEFAULT_ADMIN_ROLE(), ISSUANCE_SIGNER));
    }

    /// @dev One function, one address argument, and the answer differs for an EOA and a contract only
    /// because the core KNOWS different things about them — never because it sniffed `code.length`,
    /// which is unreliable (a contract in its own constructor reports zero) and would split a clinic
    /// contract from an individual practitioner's wallet, which this product must not do.
    function test_rights_of_answers_for_an_eoa_and_a_contract_through_the_same_lookup() public {
        vm.prank(AUTHORITY);
        registry.setRights(ISSUANCE_SIGNER, R_ISSUE);

        // A plain wallet: the granted bit, and no service bits.
        assertEq(registry.rightsOf(ISSUANCE_SIGNER), R_ISSUE);
        // The registrar key: derived, never written.
        assertEq(registry.rightsOf(AUTHORITY), R_PROTOCOL_ADMIN);
        // A stranger: nothing at all.
        assertEq(registry.rightsOf(STRANGER), 0);

        // An attached service CONTRACT reports its live lifecycle through the same call.
        uint256 serviceBits = registry.rightsOf(address(serviceA));
        assertEq(
            serviceBits,
            R_SERVICE_ATTACHED | R_SERVICE_OWNER_CONFIRMED
                | R_SERVICE_STANDING | R_SERVICE_CURRENT
        );

        // Those bits track state rather than restating it: a suspension clears standing, and the
        // pointer bit survives it, so they are genuinely independent members.
        vm.prank(AUTHORITY);
        registry.setProviderStanding(providerA, ProviderRegistry.Standing.SUSPENDED);
        uint256 suspended = registry.rightsOf(address(serviceA));
        assertEq(suspended & R_SERVICE_STANDING, 0);
        assertTrue(suspended & R_SERVICE_CURRENT != 0);
        assertTrue(suspended & R_SERVICE_ATTACHED != 0);

        // A contract the registrar never attached is not a service, and says so by omission.
        ProviderRegistryServiceMock unattached = factoryA.create(RECORD_TYPE, SERVICE_OWNER);
        assertEq(registry.rightsOf(address(unattached)), 0);
    }

    function test_can_issue_folds_provider_service_factory_and_owner_confirmation() public {
        ProviderRegistryZeroOwnerMock zeroOwner = new ProviderRegistryZeroOwnerMock();
        (,,, bool unregisteredOwnerConfirmed,) = registry.effectiveService(address(zeroOwner));
        assertFalse(unregisteredOwnerConfirmed);

        vm.prank(AUTHORITY);
        registry.setRights(ISSUANCE_SIGNER, R_ISSUE);
        assertTrue(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        {
            (
                ProviderRegistry.Standing providerStanding,
                ProviderRegistry.Standing serviceStanding,
                bool factoryActive,
                bool ownerConfirmed,
                bool hasActiveIssuer
            ) = registry.effectiveService(address(serviceA));
            assertEq(uint8(providerStanding), uint8(ProviderRegistry.Standing.ACTIVE));
            assertEq(uint8(serviceStanding), uint8(ProviderRegistry.Standing.ACTIVE));
            assertTrue(factoryActive);
            assertTrue(ownerConfirmed);
            assertTrue(hasActiveIssuer);
        }

        vm.prank(AUTHORITY);
        registry.setServiceStanding(address(serviceA), ProviderRegistry.Standing.SUSPENDED);
        assertFalse(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        (,,,, bool activeWhileSuspended) = registry.effectiveService(address(serviceA));
        assertFalse(activeWhileSuspended);
        vm.prank(AUTHORITY);
        registry.setServiceStanding(address(serviceA), ProviderRegistry.Standing.ACTIVE);

        vm.prank(SERVICE_OWNER);
        serviceA.transferOwnership(NEXT_SERVICE_OWNER);
        vm.prank(NEXT_SERVICE_OWNER);
        serviceA.acceptOwnership();
        assertFalse(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        (,,, bool ownerConfirmedBeforeKyc,) = registry.effectiveService(address(serviceA));
        assertFalse(ownerConfirmedBeforeKyc);

        vm.prank(AUTHORITY);
        registry.confirmServiceOwner(address(serviceA), NEXT_SERVICE_OWNER, 1);
        assertTrue(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));

        vm.prank(AUTHORITY);
        registry.deprecateFactoryGeneration(GENERATION_A);
        assertFalse(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        (,, bool factoryActiveAfterDeprecation,, bool activeAfterDeprecation) =
            registry.effectiveService(address(serviceA));
        assertFalse(factoryActiveAfterDeprecation);
        assertFalse(activeAfterDeprecation);
    }

    /// @dev The mandatory issuer-whitelist verification pillar reads a definite `false` as an
    /// authenticity failure and REFUSES the credential, so the historical issuer-status question must
    /// never be answered by a current-eligibility predicate. This pins the three-rung ladder:
    /// `isRecognizedIssuer` (what a verifier asks) is strictly wider than `canRevoke`, which is
    /// strictly wider than `canIssue` (the pre-issue gate).
    function test_historical_issuer_status_outlives_every_lifecycle_event_that_stops_new_issuance()
        public
    {
        ProviderRegistryServiceMock replacement =
            _createAndAttach(factoryA, GENERATION_A, providerA, RECORD_TYPE, SERVICE_OWNER);
        vm.prank(AUTHORITY);
        registry.setRights(ISSUANCE_SIGNER, R_ISSUE);

        assertTrue(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        assertTrue(registry.isRecognizedIssuer(address(serviceA), ISSUANCE_SIGNER));
        // Never granted, and never attached: both are genuine negatives on the historical axis too.
        assertFalse(registry.isRecognizedIssuer(address(serviceA), OTHER_SIGNER));
        ProviderRegistryServiceMock unattached = factoryA.create(RECORD_TYPE, SERVICE_OWNER);
        assertFalse(registry.isRecognizedIssuer(address(unattached), ISSUANCE_SIGNER));

        // (1) A repoint supersedes the old clone for new issuance only.
        vm.prank(SERVICE_OWNER);
        registry.repointService(address(replacement));
        assertFalse(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        assertTrue(registry.canRevoke(address(serviceA), ISSUANCE_SIGNER));
        assertTrue(registry.isRecognizedIssuer(address(serviceA), ISSUANCE_SIGNER));

        // (2) Retiring the superseded service is terminal, so it must not reach either wider rung.
        vm.prank(AUTHORITY);
        registry.setServiceStanding(address(serviceA), ProviderRegistry.Standing.RETIRED);
        assertFalse(registry.canIssue(address(serviceA), ISSUANCE_SIGNER));
        assertTrue(registry.canRevoke(address(serviceA), ISSUANCE_SIGNER));
        assertTrue(registry.isRecognizedIssuer(address(serviceA), ISSUANCE_SIGNER));

        // (3) An unrelated KYC suspension stops new issuance fleet-wide for the provider.
        vm.prank(AUTHORITY);
        registry.setProviderStanding(providerA, ProviderRegistry.Standing.SUSPENDED);
        assertFalse(registry.canIssue(address(replacement), ISSUANCE_SIGNER));
        assertTrue(registry.canRevoke(address(replacement), ISSUANCE_SIGNER));
        assertTrue(registry.isRecognizedIssuer(address(replacement), ISSUANCE_SIGNER));

        // (4) Generation deprecation is irreversible, so folding it into revocation would strand
        // every root the generation ever anchored as unrevocable by its own originator.
        vm.prank(AUTHORITY);
        registry.deprecateFactoryGeneration(GENERATION_A);
        assertFalse(registry.canIssue(address(replacement), ISSUANCE_SIGNER));
        assertTrue(registry.canRevoke(address(serviceA), ISSUANCE_SIGNER));
        assertTrue(registry.canRevoke(address(replacement), ISSUANCE_SIGNER));
        assertTrue(registry.isRecognizedIssuer(address(serviceA), ISSUANCE_SIGNER));

        // (5) An unconfirmed clone-owner handover quarantines operations, but a verifier must not
        // read a pending handover as "this credential was never genuinely issued".
        vm.prank(SERVICE_OWNER);
        serviceA.transferOwnership(NEXT_SERVICE_OWNER);
        vm.prank(NEXT_SERVICE_OWNER);
        serviceA.acceptOwnership();
        assertFalse(registry.canRevoke(address(serviceA), ISSUANCE_SIGNER));
        assertTrue(registry.isRecognizedIssuer(address(serviceA), ISSUANCE_SIGNER));
        // The registrar can clear that quarantine even on a retired, deprecated-generation service.
        vm.prank(AUTHORITY);
        registry.confirmServiceOwner(address(serviceA), NEXT_SERVICE_OWNER, 1);
        assertTrue(registry.canRevoke(address(serviceA), ISSUANCE_SIGNER));

        // Only an explicit registrar revocation of the forward-only grant flips the historical answer.
        vm.prank(AUTHORITY);
        registry.setRights(ISSUANCE_SIGNER, 0);
        assertFalse(registry.isRecognizedIssuer(address(serviceA), ISSUANCE_SIGNER));
        assertFalse(registry.canRevoke(address(serviceA), ISSUANCE_SIGNER));
    }

    // ---------------------------------------------------------------------------------------------
    // Correcting the one attachment fact chain provenance cannot verify
    // ---------------------------------------------------------------------------------------------

    function test_registrar_corrects_a_misbound_provider_without_publishing_under_the_new_one() public {
        vm.startPrank(AUTHORITY);
        _register(providerB, NEXT_CONTROLLER, keccak256("identity-b"));
        registry.setProviderStanding(providerB, ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();

        // The registrar typo'd the provider: this clone belongs to provider B. A third service is
        // attached after it so the correction has to move a NON-tail entry out of the enumeration.
        ProviderRegistryServiceMock misbound =
            _createAndAttach(factoryB, GENERATION_B, providerA, OTHER_RECORD_TYPE, SERVICE_OWNER);
        ProviderRegistryServiceMock trailing =
            _createAndAttach(factoryB, GENERATION_B, providerA, RECORD_TYPE, SERVICE_OWNER);
        vm.prank(SERVICE_OWNER);
        registry.repointService(address(misbound));
        assertEq(registry.currentService(providerA, OTHER_RECORD_TYPE), address(misbound));
        assertEq(registry.providerServiceCount(providerA), 3);
        assertEq(registry.providerServiceCount(providerB), 0);

        ProviderRegistryServiceMock unattached = factoryB.create(RECORD_TYPE, SERVICE_OWNER);

        vm.prank(STRANGER);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, STRANGER));
        registry.reassignServiceProvider(address(misbound), providerA, providerB);

        vm.startPrank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.UnexpectedServiceProvider.selector);
        registry.reassignServiceProvider(address(misbound), providerB, providerB);
        vm.expectRevert(ProviderRegistry.NoChange.selector);
        registry.reassignServiceProvider(address(misbound), providerA, providerA);
        vm.expectRevert(ProviderRegistry.UnknownProvider.selector);
        registry.reassignServiceProvider(address(misbound), providerA, providerC);
        vm.expectRevert(ProviderRegistry.UnknownService.selector);
        registry.reassignServiceProvider(address(unattached), providerA, providerB);

        registry.reassignServiceProvider(address(misbound), providerA, providerB);
        vm.stopPrank();

        // The binding moves; every chain-resolved fact is untouched.
        ProviderRegistry.Service memory corrected = registry.service(address(misbound));
        assertEq(bytes32(corrected.providerId), bytes32(providerB));
        assertEq(corrected.factoryGeneration, GENERATION_B);
        assertEq(corrected.recordType, OTHER_RECORD_TYPE);
        assertEq(corrected.confirmedOwner, SERVICE_OWNER);
        assertEq(uint8(corrected.standing), uint8(ProviderRegistry.Standing.ACTIVE));

        // The mistaken provider's stale pointer is cleared rather than carried across, and the
        // corrected provider is NOT published to: repointing stays the clone owner's decision.
        assertEq(registry.currentService(providerA, OTHER_RECORD_TYPE), address(0));
        assertEq(registry.currentService(providerB, OTHER_RECORD_TYPE), address(0));
        assertEq(registry.currentService(providerA, RECORD_TYPE), address(serviceA));

        vm.prank(AUTHORITY);
        registry.setRights(ISSUANCE_SIGNER, R_ISSUE);
        assertFalse(registry.canIssue(address(misbound), ISSUANCE_SIGNER));
        vm.prank(SERVICE_OWNER);
        registry.repointService(address(misbound));
        assertTrue(registry.canIssue(address(misbound), ISSUANCE_SIGNER));

        // Enumeration moves with the binding, on both sides, and the entry displaced to backfill the
        // gap stays correctable itself — a stale removal index would silently orphan it.
        assertEq(registry.providerServiceCount(providerA), 2);
        assertEq(registry.providerServiceCount(providerB), 1);
        {
            (address[] memory providerAServices,) = registry.providerServicePage(providerA, 0, 10);
            assertEq(providerAServices.length, 2);
            assertEq(providerAServices[0], address(serviceA));
            assertEq(providerAServices[1], address(trailing));
            (address[] memory providerBServices,) = registry.providerServicePage(providerB, 0, 10);
            assertEq(providerBServices.length, 1);
            assertEq(providerBServices[0], address(misbound));
        }

        vm.prank(AUTHORITY);
        registry.reassignServiceProvider(address(trailing), providerA, providerB);
        assertEq(registry.providerServiceCount(providerA), 1);
        assertEq(registry.providerServiceCount(providerB), 2);
        {
            (address[] memory providerAServices,) = registry.providerServicePage(providerA, 0, 10);
            assertEq(providerAServices.length, 1);
            assertEq(providerAServices[0], address(serviceA));
            (address[] memory providerBServices,) = registry.providerServicePage(providerB, 0, 10);
            assertEq(providerBServices.length, 2);
            assertEq(providerBServices[0], address(misbound));
            assertEq(providerBServices[1], address(trailing));
        }

        // The append-only global audit list is unaffected by a binding correction.
        assertEq(registry.serviceCount(), 3);
    }

    function test_verifier_capability_preserves_exact_orthogonal_key_shape() public {
        bytes32 expectedKey = keccak256(abi.encode("VERIFY:", PURPOSE));
        assertEq(registry.verificationKey(PURPOSE), expectedKey);

        vm.prank(AUTHORITY);
        registry.setVerifierCapability(PURPOSE, RELAYER, true);

        assertTrue(registry.canVerify(PURPOSE, RELAYER));
        assertTrue(registry.isWhitelistedFor(expectedKey, RELAYER));
        assertFalse(registry.canVerify(OTHER_RECORD_TYPE, RELAYER));

        // An attached issuer caller never receives a verifier grant as an issuance grant.
        vm.prank(address(serviceA));
        assertFalse(registry.isWhitelistedFor(expectedKey, RELAYER));

        vm.prank(AUTHORITY);
        registry.setVerifierCapability(PURPOSE, RELAYER, false);
        assertFalse(registry.isWhitelistedFor(expectedKey, RELAYER));
    }

    function test_legacy_compatibility_selectors_and_roles_are_exact() public view {
        assertLt(address(registry).code.length, 24_576);
        assertEq(
            IProviderRegistry.isWhitelistedFor.selector,
            bytes4(keccak256("isWhitelistedFor(bytes32,address)"))
        );
        assertEq(IProviderRegistry.hasRole.selector, bytes4(keccak256("hasRole(bytes32,address)")));
        assertEq(IProviderRegistry.isWhitelistedFor.selector, bytes4(0x779c3985));
        assertEq(IProviderRegistry.hasRole.selector, bytes4(0x91d14854));

        assertTrue(registry.hasRole(bytes32(0), AUTHORITY));
        assertTrue(registry.hasRole(keccak256("WHITELIST_ADMIN"), AUTHORITY));
        assertFalse(registry.hasRole(keccak256("SOME_OTHER_ROLE"), AUTHORITY));
        assertFalse(registry.hasRole(bytes32(0), STRANGER));
    }

    function test_service_creation_gate_uses_provider_clearance_and_controller_scope() public {
        ProviderRegistryFactoryMock unregisteredFactory = new ProviderRegistryFactoryMock();
        assertFalse(factoryA.canCreate(registry, providerA, RECORD_TYPE, CONTROLLER));
        assertFalse(unregisteredFactory.canCreate(registry, providerA, RECORD_TYPE, CONTROLLER));

        vm.prank(AUTHORITY);
        registry.setServiceCreationApproval(providerA, RECORD_TYPE, true);
        assertTrue(factoryA.canCreate(registry, providerA, RECORD_TYPE, CONTROLLER));
        assertTrue(factoryB.canCreate(registry, providerA, RECORD_TYPE, CONTROLLER));
        assertFalse(factoryA.canCreate(registry, providerA, RECORD_TYPE, ISSUANCE_SIGNER));
        // Direct callers do not get to impersonate an approved factory.
        assertFalse(registry.canCreateService(providerA, RECORD_TYPE, CONTROLLER));

        vm.prank(CONTROLLER);
        registry.setProviderDelegate(
            providerA, DELEGATE, PROVIDER_CREATE_SERVICE_PERMISSION, uint64(block.timestamp + 1 days)
        );
        assertTrue(factoryA.canCreate(registry, providerA, RECORD_TYPE, DELEGATE));

        vm.prank(AUTHORITY);
        registry.deprecateFactoryGeneration(GENERATION_A);
        assertFalse(factoryA.canCreate(registry, providerA, RECORD_TYPE, CONTROLLER));
        assertTrue(factoryB.canCreate(registry, providerA, RECORD_TYPE, CONTROLLER));

        vm.prank(AUTHORITY);
        registry.setProviderStanding(providerA, ProviderRegistry.Standing.SUSPENDED);
        assertFalse(factoryB.canCreate(registry, providerA, RECORD_TYPE, CONTROLLER));
        assertFalse(factoryB.canCreate(registry, providerA, RECORD_TYPE, DELEGATE));
    }

    // ---------------------------------------------------------------------------------------------
    // Typed resolver allowlist
    // ---------------------------------------------------------------------------------------------

    function test_resolver_allowlist_is_typed_and_preserves_history_on_deprecation() public {
        ProviderRegistryResolverMock resolver = new ProviderRegistryResolverMock();
        address resolverAddress = address(resolver);

        vm.prank(STRANGER);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, STRANGER));
        registry.setResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, resolverAddress, true);

        _approveResolver(ProviderRegistry.ResolverKind.DIRECTORY, resolverAddress);
        assertTrue(registry.isResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, resolverAddress));
        assertFalse(registry.isResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, resolverAddress));

        vm.prank(CONTROLLER);
        registry.setDirectoryResolver(providerA, resolverAddress);
        assertEq(registry.provider(providerA).directoryResolver, resolverAddress);

        vm.prank(SERVICE_OWNER);
        vm.expectRevert(ProviderRegistry.ResolverNotApproved.selector);
        registry.setDomainResolver(address(serviceA), resolverAddress);

        _approveResolver(ProviderRegistry.ResolverKind.DOMAIN, resolverAddress);
        vm.prank(SERVICE_OWNER);
        registry.setDomainResolver(address(serviceA), resolverAddress);
        assertEq(registry.service(address(serviceA)).domainResolver, resolverAddress);

        vm.prank(AUTHORITY);
        registry.setResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, resolverAddress, false);
        assertFalse(registry.isResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, resolverAddress));
        // The raw historical pointer remains readable.
        assertEq(registry.provider(providerA).directoryResolver, resolverAddress);

        vm.prank(CONTROLLER);
        registry.setDirectoryResolver(providerA, address(0));
        assertEq(registry.provider(providerA).directoryResolver, address(0));

        // A deprecated resolver cannot be selected again.
        vm.prank(CONTROLLER);
        vm.expectRevert(ProviderRegistry.ResolverNotApproved.selector);
        registry.setDirectoryResolver(providerA, resolverAddress);

        // It is listed once despite approval on two typed axes and later deprecation.
        assertEq(registry.resolverCount(ProviderRegistry.ResolverKind.DIRECTORY), 1);
        assertEq(registry.resolverCount(ProviderRegistry.ResolverKind.DOMAIN), 1);
    }

    function test_service_and_provider_standing_retirement_is_terminal() public {
        vm.prank(AUTHORITY);
        registry.setProviderStanding(providerA, ProviderRegistry.Standing.RETIRED);
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.RetiredStanding.selector);
        registry.setProviderStanding(providerA, ProviderRegistry.Standing.ACTIVE);

        // Use another provider/service because provider A is now retired.
        vm.startPrank(AUTHORITY);
        _register(providerB, NEXT_CONTROLLER, keccak256("identity-b"));
        registry.setProviderStanding(providerB, ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();
        ProviderRegistryServiceMock another =
            _createAndAttach(factoryB, GENERATION_B, providerB, OTHER_RECORD_TYPE, NEXT_SERVICE_OWNER);

        vm.prank(AUTHORITY);
        registry.setServiceStanding(address(another), ProviderRegistry.Standing.RETIRED);
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.RetiredStanding.selector);
        registry.setServiceStanding(address(another), ProviderRegistry.Standing.ACTIVE);
    }

    // ---------------------------------------------------------------------------------------------
    // Opaque identity, public anchor and bounded enumeration
    // ---------------------------------------------------------------------------------------------

    function test_provider_id_is_opaque_stable_and_not_collapsed_by_shared_controller() public {
        vm.prank(AUTHORITY);
        _register(providerB, CONTROLLER, keccak256("identity-b"));

        assertEq(registry.provider(providerA).controller, CONTROLLER);
        assertEq(registry.provider(providerB).controller, CONTROLLER);
        assertTrue(providerA != providerB);

        ProviderRegistryServiceMock second =
            _createAndAttach(factoryB, GENERATION_B, providerA, OTHER_RECORD_TYPE, SERVICE_OWNER);
        assertEq(bytes32(registry.service(address(serviceA)).providerId), bytes32(providerA));
        assertEq(bytes32(registry.service(address(second)).providerId), bytes32(providerA));
        assertEq(registry.providerServiceCount(providerA), 2);

        vm.prank(CONTROLLER);
        registry.requestControllerTransfer(providerA, NEXT_CONTROLLER);
        vm.prank(NEXT_CONTROLLER);
        registry.acceptControllerTransfer(providerA);
        ProviderRegistry.Provider memory accepted = registry.provider(providerA);
        vm.prank(AUTHORITY);
        registry.confirmControllerTransfer(providerA, NEXT_CONTROLLER, accepted.controllerRequestNonce);

        assertEq(bytes32(registry.service(address(serviceA)).providerId), bytes32(providerA));
        assertEq(bytes32(registry.service(address(second)).providerId), bytes32(providerA));
    }

    function test_public_identity_anchor_is_registrar_controlled_versioned_and_bounded() public {
        ProviderRegistry.PublicIdentityAnchor memory initial = registry.publicIdentityAnchor(providerA);
        assertEq(initial.digest, IDENTITY_DIGEST);
        assertEq(initial.schema, 1);
        assertEq(initial.codec, 0xe3);
        assertEq(initial.hashAlgorithm, 0x12);
        assertEq(initial.contenthash, bytes("ipfs://publication-safe-provider-identity"));
        assertEq(initial.revision, 1);
        assertEq(initial.updatedAtBlock, uint64(block.number));

        bytes32 nextDigest = keccak256("publication-safe-identity-a-v2");
        vm.prank(CONTROLLER);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, CONTROLLER));
        registry.setPublicIdentityAnchor(providerA, nextDigest, 2, 0xe3, 0x12, bytes("ipfs://public-v2"));

        vm.roll(block.number + 3);
        vm.prank(AUTHORITY);
        registry.setPublicIdentityAnchor(providerA, nextDigest, 2, 0xe3, 0x12, bytes("ipfs://public-v2"));
        ProviderRegistry.PublicIdentityAnchor memory updated = registry.publicIdentityAnchor(providerA);
        assertEq(updated.digest, nextDigest);
        assertEq(updated.revision, 2);
        assertEq(updated.updatedAtBlock, uint64(block.number));

        // The bounded contenthash route is optional; discovery mirrors can derive a route from
        // schema + digest without forcing an on-chain pointer.
        bytes32 mirrorOnlyDigest = keccak256("publication-safe-identity-a-v3");
        vm.prank(AUTHORITY);
        registry.setPublicIdentityAnchor(providerA, mirrorOnlyDigest, 3, 0xe3, 0x12, bytes(""));
        ProviderRegistry.PublicIdentityAnchor memory mirrorOnly = registry.publicIdentityAnchor(providerA);
        assertEq(mirrorOnly.digest, mirrorOnlyDigest);
        assertEq(mirrorOnly.contenthash.length, 0);
        assertEq(mirrorOnly.revision, 3);

        bytes memory tooLong = new bytes(registry.MAX_CONTENTHASH_LENGTH() + 1);
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.ContenthashTooLong.selector);
        registry.setPublicIdentityAnchor(providerA, nextDigest, 2, 0xe3, 0x12, tooLong);
    }

    function test_rejects_zero_and_duplicate_provider_ids() public {
        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.ZeroProviderId.selector);
        _register(bytes20(0), CONTROLLER, keccak256("zero"));

        vm.prank(AUTHORITY);
        vm.expectRevert(ProviderRegistry.AlreadyRegistered.selector);
        _register(providerA, NEXT_CONTROLLER, keccak256("duplicate"));
    }

    function test_bounded_enumeration_pages_every_core_axis_without_unbounded_get_all() public {
        vm.startPrank(AUTHORITY);
        _register(providerB, NEXT_CONTROLLER, keccak256("identity-b"));
        _register(providerC, STRANGER, keccak256("identity-c"));
        registry.setProviderStanding(providerB, ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();
        address second = address(
            _createAndAttach(factoryB, GENERATION_B, providerB, OTHER_RECORD_TYPE, NEXT_SERVICE_OWNER)
        );
        ProviderRegistryResolverMock resolverOne = new ProviderRegistryResolverMock();
        ProviderRegistryResolverMock resolverTwo = new ProviderRegistryResolverMock();
        _approveResolver(ProviderRegistry.ResolverKind.DIRECTORY, address(resolverOne));
        _approveResolver(ProviderRegistry.ResolverKind.DIRECTORY, address(resolverTwo));

        {
            (bytes20[] memory firstProviders, uint256 providerCursor) = registry.providerPage(0, 2);
            assertEq(firstProviders.length, 2);
            assertEq(bytes32(firstProviders[0]), bytes32(providerA));
            assertEq(bytes32(firstProviders[1]), bytes32(providerB));
            assertEq(providerCursor, 2);
            (bytes20[] memory finalProviders, uint256 finalProviderCursor) =
                registry.providerPage(providerCursor, 2);
            assertEq(finalProviders.length, 1);
            assertEq(bytes32(finalProviders[0]), bytes32(providerC));
            assertEq(finalProviderCursor, 3);
            (bytes20[] memory emptyProviders, uint256 endCursor) = registry.providerPage(3, 1);
            assertEq(emptyProviders.length, 0);
            assertEq(endCursor, 3);
        }

        {
            (bytes32[] memory generations, uint256 generationCursor) = registry.factoryGenerationPage(0, 100);
            assertEq(generations.length, 2);
            assertEq(generations[0], GENERATION_A);
            assertEq(generations[1], GENERATION_B);
            assertEq(generationCursor, 2);
        }

        {
            (address[] memory services, uint256 serviceCursor) = registry.servicePage(0, 100);
            assertEq(services.length, 2);
            assertEq(services[0], address(serviceA));
            assertEq(services[1], second);
            assertEq(serviceCursor, 2);
        }

        {
            (address[] memory providerBServices, uint256 providerBServiceCursor) =
                registry.providerServicePage(providerB, 0, 1);
            assertEq(providerBServices.length, 1);
            assertEq(providerBServices[0], second);
            assertEq(providerBServiceCursor, 1);
        }

        {
            (address[] memory resolvers, uint256 resolverCursor) =
                registry.resolverPage(ProviderRegistry.ResolverKind.DIRECTORY, 0, 1);
            assertEq(resolvers.length, 1);
            assertEq(resolvers[0], address(resolverOne));
            assertEq(resolverCursor, 1);
        }

        vm.expectRevert(ProviderRegistry.BadPage.selector);
        registry.providerPage(0, 0);
        vm.expectRevert(ProviderRegistry.BadPage.selector);
        registry.providerPage(0, 101);
        vm.expectRevert(ProviderRegistry.BadPage.selector);
        registry.providerPage(4, 1);

        // State changes do not duplicate append-only audit enumeration.
        vm.prank(AUTHORITY);
        registry.setProviderStanding(providerB, ProviderRegistry.Standing.SUSPENDED);
        assertEq(registry.providerCount(), 3);
        assertEq(registry.factoryGenerationCount(), 2);
        assertEq(registry.serviceCount(), 2);
    }

    // ---------------------------------------------------------------------------------------------
    // The emitted audit trail
    //
    // Every authority fact this core stores is also announced, and an off-chain oversight consumer
    // reads ONLY the announcement: it decodes by event signature and indexed topic, never by
    // re-reading storage. So an event that carries the wrong field, indexes the wrong argument, or
    // reports the post-state where the transition is the fact is not a cosmetic slip - it is
    // silently undecodable, or decodably wrong, at exactly the consumer that cannot ask again.
    // These assert, by full signature and topic layout, every announcement carrying a term no getter
    // answers: both ends of a provider AND a service standing change, each handover rung's own nonce,
    // and the epoch a provider or service delegation is scoped to. The announcements not pinned here
    // (`ControllerTransferCancelled`, `ServiceCreationApprovalSet`, `DirectoryResolverSet`,
    // `DomainResolverSet`) are the ones whose fact a caller can still read back off this core.
    // ---------------------------------------------------------------------------------------------

    event ProviderRegistered(bytes20 indexed providerId, address indexed controller);
    event ProviderStandingChanged(
        bytes20 indexed providerId, ProviderRegistry.Standing oldStanding, ProviderRegistry.Standing newStanding
    );
    event PublicIdentityAnchorSet(
        bytes20 indexed providerId,
        bytes32 indexed oldDigest,
        bytes32 indexed newDigest,
        uint32 schema,
        uint16 codec,
        uint8 hashAlgorithm,
        bytes contenthash,
        uint64 revision,
        uint64 atBlock
    );
    event ControllerTransferRequested(
        bytes20 indexed providerId,
        address indexed currentController,
        address indexed pendingController,
        address requestedBy,
        uint64 requestNonce
    );
    event ControllerTransferAccepted(
        bytes20 indexed providerId, address indexed pendingController, uint64 requestNonce
    );
    event ControllerTransferConfirmed(
        bytes20 indexed providerId,
        address indexed oldController,
        address indexed newController,
        uint64 controllerEpoch
    );
    event ProviderDelegateSet(
        bytes20 indexed providerId,
        address indexed delegate,
        uint32 permissions,
        uint64 validUntil,
        uint64 controllerEpoch
    );
    event FactoryGenerationAdded(bytes32 indexed generationId, address indexed factory, uint64 atBlock);
    event FactoryGenerationDeprecated(bytes32 indexed generationId, address indexed factory, uint64 atBlock);
    event ServiceAttached(
        address indexed service,
        bytes20 indexed providerId,
        bytes32 indexed factoryGeneration,
        bytes32 recordType,
        address confirmedOwner
    );
    event ServiceProviderReassigned(
        address indexed service,
        bytes20 indexed oldProviderId,
        bytes20 indexed newProviderId,
        address reassignedBy
    );
    event ServiceStandingChanged(
        address indexed service,
        ProviderRegistry.Standing oldStanding,
        ProviderRegistry.Standing newStanding
    );
    event ServiceOwnerConfirmed(
        address indexed service, address indexed oldOwner, address indexed newOwner, uint64 ownerEpoch
    );
    event ServiceDelegateSet(
        address indexed service,
        address indexed delegate,
        uint32 permissions,
        uint64 validUntil,
        uint64 ownerEpoch
    );
    event CurrentServiceChanged(
        bytes20 indexed providerId,
        bytes32 indexed recordType,
        address indexed oldService,
        address newService,
        address setBy
    );
    event RightsSet(address indexed account, uint256 rights);
    event VerifierCapabilitySet(
        bytes32 indexed purpose, bytes32 indexed compatibilityKey, address indexed relayer, bool allowed
    );
    event ResolverApprovalSet(
        ProviderRegistry.ResolverKind indexed kind, address indexed resolver, bool approved
    );

    function test_identity_and_standing_transitions_are_announced_with_full_audit_detail() public {
        bytes20 provider = providerC;
        bytes32 firstDigest = keccak256("publication-safe-identity-c1");
        bytes memory contenthash = bytes("ipfs://publication-safe-provider-identity");

        // Registration announces the opaque id and its first controller, and the identity anchor
        // arrives as its own event so a publication consumer never has to infer revision 1.
        vm.expectEmit(true, true, true, true, address(registry));
        emit ProviderRegistered(provider, CONTROLLER);
        vm.expectEmit(true, true, true, true, address(registry));
        emit PublicIdentityAnchorSet(
            provider, bytes32(0), firstDigest, 1, 0xe3, 0x12, contenthash, 1, uint64(block.number)
        );
        vm.prank(AUTHORITY);
        registry.registerProvider(provider, CONTROLLER, firstDigest, 1, 0xe3, 0x12, contenthash);

        // A re-anchor carries BOTH digests and the bumped revision: superseding a published identity
        // statement is the fact, and an event that reported only the new digest would leave a
        // consumer unable to tell a first publication from a silent replacement.
        bytes32 secondDigest = keccak256("publication-safe-identity-c2");
        vm.expectEmit(true, true, true, true, address(registry));
        emit PublicIdentityAnchorSet(
            provider, firstDigest, secondDigest, 2, 0xe3, 0x12, contenthash, 2, uint64(block.number)
        );
        vm.prank(AUTHORITY);
        registry.setPublicIdentityAnchor(provider, secondDigest, 2, 0xe3, 0x12, contenthash);

        // Standing is a TRANSITION, so both ends ride the event. A consumer that joined the feed
        // late cannot reconstruct the previous standing from anywhere else.
        vm.expectEmit(true, true, true, true, address(registry));
        emit ProviderStandingChanged(
            provider, ProviderRegistry.Standing.PENDING, ProviderRegistry.Standing.ACTIVE
        );
        vm.prank(AUTHORITY);
        registry.setProviderStanding(provider, ProviderRegistry.Standing.ACTIVE);

        vm.expectEmit(true, true, true, true, address(registry));
        emit ProviderStandingChanged(
            provider, ProviderRegistry.Standing.ACTIVE, ProviderRegistry.Standing.SUSPENDED
        );
        vm.prank(AUTHORITY);
        registry.setProviderStanding(provider, ProviderRegistry.Standing.SUSPENDED);

        // The service axis carries both ends for the same reason, and its event is the ONLY record of
        // the transition: `Service.standing` holds the post-state alone, so a consumer reading storage
        // cannot tell a service suspended from ACTIVE for review from one that never cleared review.
        vm.expectEmit(true, true, true, true, address(registry));
        emit ServiceStandingChanged(
            address(serviceA), ProviderRegistry.Standing.ACTIVE, ProviderRegistry.Standing.SUSPENDED
        );
        vm.prank(AUTHORITY);
        registry.setServiceStanding(address(serviceA), ProviderRegistry.Standing.SUSPENDED);
    }

    function test_every_safe_handover_step_is_separately_announced_with_its_nonce_and_epoch() public {
        uint64 validUntil = uint64(block.timestamp + 7 days);

        // A delegation announces the epoch it is SCOPED to, which is the term a rotation invalidates.
        // Without it the feed shows a grant with no way to tell, at any later block, whether it is
        // still live: the consumer joins this epoch against the rotation events below.
        vm.expectEmit(true, true, true, true, address(registry));
        emit ProviderDelegateSet(providerA, DELEGATE, PROVIDER_DIRECTORY_PERMISSION, validUntil, 1);
        vm.prank(CONTROLLER);
        registry.setProviderDelegate(providerA, DELEGATE, PROVIDER_DIRECTORY_PERMISSION, validUntil);

        // Each rung of the controller handover is its own event. Collapsing them would make a
        // requested-but-unaccepted transfer indistinguishable from a completed one - the exact
        // ambiguity the request/accept/confirm split exists to remove.
        vm.expectEmit(true, true, true, true, address(registry));
        emit ControllerTransferRequested(providerA, CONTROLLER, NEXT_CONTROLLER, CONTROLLER, 1);
        vm.prank(CONTROLLER);
        registry.requestControllerTransfer(providerA, NEXT_CONTROLLER);

        vm.expectEmit(true, true, true, true, address(registry));
        emit ControllerTransferAccepted(providerA, NEXT_CONTROLLER, 1);
        vm.prank(NEXT_CONTROLLER);
        registry.acceptControllerTransfer(providerA);

        // Only confirmation moves authority, and it is the only step carrying the new epoch - the
        // term every delegation is scoped to.
        vm.expectEmit(true, true, true, true, address(registry));
        emit ControllerTransferConfirmed(providerA, CONTROLLER, NEXT_CONTROLLER, 2);
        vm.prank(AUTHORITY);
        registry.confirmControllerTransfer(providerA, NEXT_CONTROLLER, 1);

        // The epoch on the confirmation is what orphans every grant announced under the previous one,
        // so a re-grant of the SAME delegate and permissions is distinguishable on the feed only by
        // this field. The two announcements are otherwise byte-identical.
        assertFalse(registry.canWriteProvider(providerA, DELEGATE, PROVIDER_DIRECTORY_PERMISSION));
        vm.expectEmit(true, true, true, true, address(registry));
        emit ProviderDelegateSet(providerA, DELEGATE, PROVIDER_DIRECTORY_PERMISSION, validUntil, 2);
        vm.prank(NEXT_CONTROLLER);
        registry.setProviderDelegate(providerA, DELEGATE, PROVIDER_DIRECTORY_PERMISSION, validUntil);
        assertTrue(registry.canWriteProvider(providerA, DELEGATE, PROVIDER_DIRECTORY_PERMISSION));

        // The service-delegate axis is scoped to the owner epoch instead, and announces it for the
        // same reason.
        vm.expectEmit(true, true, true, true, address(registry));
        emit ServiceDelegateSet(address(serviceA), DELEGATE, SERVICE_DOMAIN_PERMISSION, validUntil, 1);
        vm.prank(SERVICE_OWNER);
        registry.setServiceDelegate(address(serviceA), DELEGATE, SERVICE_DOMAIN_PERMISSION, validUntil);

        // The clone-owner axis announces its own handover with the bumped owner epoch, which is the
        // term that un-quarantines service writes and `canRevoke`.
        vm.prank(SERVICE_OWNER);
        serviceA.transferOwnership(NEXT_SERVICE_OWNER);
        vm.prank(NEXT_SERVICE_OWNER);
        serviceA.acceptOwnership();
        vm.expectEmit(true, true, true, true, address(registry));
        emit ServiceOwnerConfirmed(address(serviceA), SERVICE_OWNER, NEXT_SERVICE_OWNER, 2);
        vm.prank(AUTHORITY);
        registry.confirmServiceOwner(address(serviceA), NEXT_SERVICE_OWNER, 1);

        assertFalse(registry.canWriteService(address(serviceA), DELEGATE, SERVICE_DOMAIN_PERMISSION));
        vm.expectEmit(true, true, true, true, address(registry));
        emit ServiceDelegateSet(address(serviceA), DELEGATE, SERVICE_DOMAIN_PERMISSION, validUntil, 2);
        vm.prank(NEXT_SERVICE_OWNER);
        registry.setServiceDelegate(address(serviceA), DELEGATE, SERVICE_DOMAIN_PERMISSION, validUntil);
        assertTrue(registry.canWriteService(address(serviceA), DELEGATE, SERVICE_DOMAIN_PERMISSION));

        // The registry authority's own two-step rotation announces through OZ's own event, so the
        // one key this generation trusts is auditable on the same feed.
        vm.prank(AUTHORITY);
        registry.transferOwnership(NEXT_AUTHORITY);
        vm.expectEmit(true, true, true, true, address(registry));
        emit Ownable.OwnershipTransferred(AUTHORITY, NEXT_AUTHORITY);
        vm.prank(NEXT_AUTHORITY);
        registry.acceptOwnership();
    }

    function test_service_provenance_capability_and_resolver_writes_are_announced() public {
        ProviderRegistryFactoryMock factoryC = new ProviderRegistryFactoryMock();
        bytes32 generationC = keccak256("dogtag/factory/c");

        vm.expectEmit(true, true, true, true, address(registry));
        emit FactoryGenerationAdded(generationC, address(factoryC), uint64(block.number));
        vm.prank(AUTHORITY);
        registry.addFactoryGeneration(generationC, address(factoryC));

        ProviderRegistryServiceMock service = factoryC.create(OTHER_RECORD_TYPE, SERVICE_OWNER);

        // Attachment announces the provenance triple - generation, record type and owner. Note this
        // assertion pins the triple's SHAPE and topic layout, not the owner field's provenance:
        // `attachService` reverts unless the resolved `owner()` equals `expectedOwner`, so the two
        // are provably equal at the emit site and no event assertion could tell them apart. That the
        // owner is READ FROM THE CLONE rather than taken from the registrar's claim is established
        // by the revert arm in `test_attach_resolves_registered_factory_and_clone_metadata_not_supplied_claims`.
        vm.expectEmit(true, true, true, true, address(registry));
        emit ServiceAttached(
            address(service), providerA, generationC, OTHER_RECORD_TYPE, SERVICE_OWNER
        );
        vm.prank(AUTHORITY);
        registry.attachService(providerA, address(service), generationC, SERVICE_OWNER);

        vm.prank(AUTHORITY);
        registry.setServiceStanding(address(service), ProviderRegistry.Standing.ACTIVE);

        // The grant is announced on the ADDRESS and carries the address's whole settable mask, so the
        // at-issuance pillar can fold the sequence with no prior state and no per-bit event.
        vm.expectEmit(true, true, true, true, address(registry));
        emit RightsSet(ISSUANCE_SIGNER, R_ISSUE);
        vm.prank(AUTHORITY);
        registry.setRights(ISSUANCE_SIGNER, R_ISSUE);

        // The verifier axis announces the purpose AND the derived legacy compatibility key, so an
        // existing `isWhitelistedFor` consumer can follow the grant without recomputing it.
        vm.expectEmit(true, true, true, true, address(registry));
        emit VerifierCapabilitySet(PURPOSE, registry.verificationKey(PURPOSE), RELAYER, true);
        vm.prank(AUTHORITY);
        registry.setVerifierCapability(PURPOSE, RELAYER, true);

        // Publication is the clone owner's decision, and the pointer move carries both ends plus
        // the caller that made it.
        vm.expectEmit(true, true, true, true, address(registry));
        emit CurrentServiceChanged(
            providerA, OTHER_RECORD_TYPE, address(0), address(service), SERVICE_OWNER
        );
        vm.prank(SERVICE_OWNER);
        registry.repointService(address(service));

        // Resolver approval is announced per typed kind, keeping domain claims and directory
        // listings separable on the feed itself.
        ProviderRegistryResolverMock resolver = new ProviderRegistryResolverMock();
        vm.expectEmit(true, true, true, true, address(registry));
        emit ResolverApprovalSet(ProviderRegistry.ResolverKind.DOMAIN, address(resolver), true);
        vm.prank(AUTHORITY);
        registry.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(resolver), true);

        vm.expectEmit(true, true, true, true, address(registry));
        emit FactoryGenerationDeprecated(generationC, address(factoryC), uint64(block.number));
        vm.prank(AUTHORITY);
        registry.deprecateFactoryGeneration(generationC);
    }

    function test_a_provider_binding_correction_announces_the_clearing_and_the_move() public {
        vm.startPrank(AUTHORITY);
        _register(providerB, NEXT_CONTROLLER, keccak256("identity-b"));
        registry.setProviderStanding(providerB, ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();

        // `serviceA` is the mistaken provider's published pointer, so the correction must announce
        // TWO facts, in order: the mistaken provider stops publishing it, then the binding moves.
        // A correction that announced only the move would leave a directory consumer still listing
        // the service under a provider that no longer owns it.
        vm.expectEmit(true, true, true, true, address(registry));
        emit CurrentServiceChanged(
            providerA, RECORD_TYPE, address(serviceA), address(0), AUTHORITY
        );
        vm.expectEmit(true, true, true, true, address(registry));
        emit ServiceProviderReassigned(address(serviceA), providerA, providerB, AUTHORITY);
        vm.prank(AUTHORITY);
        registry.reassignServiceProvider(address(serviceA), providerA, providerB);

        // No publication is announced under the corrected provider: that stays the clone owner's
        // own `repointService` decision, so the feed can never show a correction as a publication.
        assertEq(registry.currentService(providerB, RECORD_TYPE), address(0));
    }
}
