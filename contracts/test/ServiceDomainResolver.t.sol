// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {
    ICloneProvenance,
    ICoreRecordPermission,
    ServiceDomainResolver
} from "../src/ServiceDomainResolver.sol";

/// @dev A contract this repo's factory lineage never deployed. Nothing about it is malformed — it answers
/// `owner()` and `recordType()` perfectly well — which is the point: the only thing standing between it and
/// a published domain claim is provenance.
contract StrangerService {
    bytes32 public immutable recordType;
    address public owner;

    constructor(bytes32 recordType_, address owner_) {
        recordType = recordType_;
        owner = owner_;
    }
}

/// @dev Answers `isClone` true for ANY address, including the zero address. The stub shape the resolver's
/// constructor must refuse: a provenance oracle that vouches for everything vouches for nothing.
contract PermissiveFactoryStub {
    function isClone(address) external pure returns (bool) {
        return true;
    }

    function rootIssuer(bytes32) external pure returns (address) {
        return address(0);
    }
}

/// @dev Has code, and answers none of the reads the resolver depends on: a staticcall to any selector
/// reverts, so it is deliberately empty.
contract SilentDependency {}

/// @dev Answers every probed read, but reports a ZERO content-write permission. `canWriteService` returns
/// false for a zero permission by its own guard, so a resolver built on this would refuse every write with
/// an authorization diagnostic for what is really a wiring fault.
contract ZeroPermissionCoreStub {
    function isClone(address) external pure returns (bool) {
        return false;
    }

    function isResolverApproved(uint8, address) external pure returns (bool) {
        return false;
    }

    function canWriteService(address, address, uint32) external pure returns (bool) {
        return false;
    }

    function service(address) external pure returns (ProviderRegistry.Service memory s) {
        return s;
    }

    function effectiveService(address)
        external
        pure
        returns (ProviderRegistry.Standing, ProviderRegistry.Standing, bool, bool, bool)
    {
        return (ProviderRegistry.Standing.NONE, ProviderRegistry.Standing.NONE, false, false, false);
    }

    function SERVICE_PERMISSION_RECORD() external pure returns (uint32) {
        return 0;
    }
}

/// @notice S-9: the service-domain resolver, keyed by service contract.
///
/// The fixture binds the REAL authority core and REAL clones from the REAL self-service factory. No
/// authority, provenance or ownership fact in these tests is supplied by a double, because the claims
/// under test are precisely about how those contracts compose — a mocked core would let the resolver
/// agree with a stand-in rather than with the thing it will be deployed against.
///
/// Headline claims:
///  * "nobody has said", "there is deliberately no domain" and "a claim was withdrawn" are three
///    distinguishable dispositions, and the stored string cannot express a claim that is not current;
///  * a write needs cleared standing AND confirmed owner-or-delegate authority, with the registrar and
///    every issuance signer deliberately excluded;
///  * a write needs the provenance the resolver's own FACTORY carries, which the core's
///    generation-pinned check does not imply; and
///  * the canonical-domain grammar is preserved exactly.
contract ServiceDomainResolverTest is Test {
    ProviderRegistry internal core;
    DogTagIssuerFactory internal factory;
    /// @dev A second REAL factory, registered as a generation on the core but NOT the one the resolver
    /// was constructed over. Its clones are genuine to the core and off-lineage to the resolver — the
    /// exact disagreement the provenance term exists to catch.
    DogTagIssuerFactory internal offLineageFactory;
    DogTagIssuer internal implementation;
    ServiceDomainResolver internal resolver;

    address internal service;

    address internal constant AUTHORITY = address(0xA11CE);
    address internal constant CONTROLLER = address(0xC011);
    address internal constant NEXT_OWNER = address(0xC012);
    address internal constant DELEGATE = address(0xDE1E);
    address internal constant ISSUANCE_SIGNER = address(0x1550);
    address internal constant STRANGER = address(0xBAD);

    bytes20 internal constant PROVIDER = bytes20(hex"9d7c0e341a85b69f2c08d1ee4f6a7398bc52d401");
    bytes20 internal constant OTHER_PROVIDER = bytes20(hex"42a16f8c73d9e205b84c0a6f9137de52c80b4a19");
    bytes32 internal constant GENERATION_2 = keccak256("dogtag/factory/generation-2");
    bytes32 internal constant GENERATION_2B = keccak256("dogtag/factory/generation-2b");
    bytes32 internal constant RECORD_TYPE = keccak256("VACCINATION");
    bytes32 internal constant IDENTITY_DIGEST = keccak256("publication-safe-identity");

    uint32 internal constant SERVICE_RECORD_PERMISSION = 1 << 0;
    uint32 internal constant SERVICE_DOMAIN_RESOLVER_PERMISSION = 1 << 1;

    string internal constant DOMAIN = "moh.gov.sg";
    string internal constant OTHER_DOMAIN = "vet.example-clinic.sg";

    function setUp() public {
        core = new ProviderRegistry(AUTHORITY);
        implementation = new DogTagIssuer();
        factory = new DogTagIssuerFactory(address(implementation), address(core));
        offLineageFactory = new DogTagIssuerFactory(address(implementation), address(core));

        vm.startPrank(AUTHORITY);
        core.addFactoryGeneration(GENERATION_2, address(factory));
        core.addFactoryGeneration(GENERATION_2B, address(offLineageFactory));
        _registerProvider(PROVIDER, CONTROLLER);
        core.setProviderStanding(PROVIDER, ProviderRegistry.Standing.ACTIVE);
        core.setServiceCreationApproval(PROVIDER, RECORD_TYPE, true);
        vm.stopPrank();

        service = _createAndAttach(factory, GENERATION_2, PROVIDER, 0);

        resolver = new ServiceDomainResolver(address(core), address(factory));
        vm.prank(AUTHORITY);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(resolver), true);
        vm.prank(CONTROLLER);
        core.setDomainResolver(service, address(resolver));
    }

    // ---------------------------------------------------------------------------------------------
    // Fixture helpers
    // ---------------------------------------------------------------------------------------------

    function _registerProvider(bytes20 providerId, address controller) internal {
        core.registerProvider(
            providerId, controller, IDENTITY_DIGEST, 1, 0xe3, 0x12, bytes("ipfs://publication-safe-identity")
        );
    }

    function _createAndAttach(
        DogTagIssuerFactory from,
        bytes32 generationId,
        bytes20 providerId,
        uint96 cloneNonce
    ) internal returns (address created) {
        address controller = core.provider(providerId).controller;
        vm.prank(controller);
        created = from.createIssuer(providerId, RECORD_TYPE, cloneNonce);
        vm.startPrank(AUTHORITY);
        core.attachService(providerId, created, generationId, controller);
        core.setServiceStanding(created, ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();
    }

    function _claim(address who, address target, string memory domain) internal {
        vm.prank(who);
        resolver.claimDomain(target, domain);
    }

    // ---------------------------------------------------------------------------------------------
    // Typed disposition: three absences, three names
    // ---------------------------------------------------------------------------------------------

    function test_an_unwritten_service_reads_unset_rather_than_an_empty_claim() public view {
        ServiceDomainResolver.DomainRecord memory r = resolver.record(service);
        assertEq(uint8(r.disposition), uint8(ServiceDomainResolver.Disposition.UNSET));
        assertEq(r.domain, "");
        assertEq(r.revision, 0);
        assertEq(r.updatedAt, 0);
        assertEq(r.updatedAtBlock, 0);
        assertEq(r.setBy, address(0));
        // An address nobody ever attached reads the same way, and does not revert.
        assertEq(uint8(resolver.dispositionOf(STRANGER)), uint8(ServiceDomainResolver.Disposition.UNSET));
    }

    /// @dev THE headline claim. The predecessor could say "no domain" exactly once, as `""`, so these
    /// three states were one state.
    function test_the_three_absences_are_three_distinct_dispositions() public {
        assertEq(uint8(resolver.dispositionOf(service)), uint8(ServiceDomainResolver.Disposition.UNSET));

        vm.prank(CONTROLLER);
        resolver.declareNoDomain(service);
        assertEq(uint8(resolver.dispositionOf(service)), uint8(ServiceDomainResolver.Disposition.NO_DOMAIN));

        _claim(CONTROLLER, service, DOMAIN);
        assertEq(uint8(resolver.dispositionOf(service)), uint8(ServiceDomainResolver.Disposition.CLAIMED));

        vm.prank(CONTROLLER);
        resolver.clearDomain(service);
        assertEq(uint8(resolver.dispositionOf(service)), uint8(ServiceDomainResolver.Disposition.CLEARED));

        // All four are different values, so no pair can be collapsed by a reader.
        assertTrue(uint8(ServiceDomainResolver.Disposition.UNSET) == 0);
        assertTrue(
            uint8(ServiceDomainResolver.Disposition.NO_DOMAIN)
                != uint8(ServiceDomainResolver.Disposition.CLEARED)
        );
    }

    function test_the_domain_string_is_non_empty_exactly_when_the_disposition_is_claimed() public {
        // UNSET
        assertEq(resolver.record(service).domain, "");

        // NO_DOMAIN
        vm.prank(CONTROLLER);
        resolver.declareNoDomain(service);
        assertEq(resolver.record(service).domain, "");

        // CLAIMED
        _claim(CONTROLLER, service, DOMAIN);
        assertEq(resolver.record(service).domain, DOMAIN);

        // CLEARED
        vm.prank(CONTROLLER);
        resolver.clearDomain(service);
        assertEq(resolver.record(service).domain, "");

        // And a claim can never be the empty string in the first place.
        vm.prank(CONTROLLER);
        vm.expectRevert(ServiceDomainResolver.BadDomain.selector);
        resolver.claimDomain(service, "");
    }

    /// @dev The withdrawn value is deliberately not retained in state. A `priorDomain` field would be read
    /// as a live claim by exactly the careless reader the typed disposition protects.
    function test_a_withdrawn_claim_cannot_be_read_back_as_a_live_domain() public {
        _claim(CONTROLLER, service, DOMAIN);

        vm.expectEmit(true, true, false, true, address(resolver));
        emit ServiceDomainResolver.DomainClaimWithdrawn(service, DOMAIN, CONTROLLER, 2, uint64(block.number));
        vm.prank(CONTROLLER);
        resolver.clearDomain(service);

        (ServiceDomainResolver.Disposition disposition, string memory domain) =
            resolver.resolveDomain(service);
        assertEq(uint8(disposition), uint8(ServiceDomainResolver.Disposition.CLEARED));
        assertEq(domain, "");
    }

    function test_unset_is_decidable_from_the_revision_and_the_timestamp_alike() public {
        ServiceDomainResolver.DomainRecord memory before = resolver.record(service);
        assertEq(before.revision, 0);
        assertEq(before.updatedAt, 0);

        // Every write stamps, including the two that store no domain, so "written" is never mistakable
        // for "never touched".
        vm.prank(CONTROLLER);
        resolver.declareNoDomain(service);
        ServiceDomainResolver.DomainRecord memory after_ = resolver.record(service);
        assertEq(after_.revision, 1);
        assertEq(after_.updatedAt, uint64(block.timestamp));
        assertEq(after_.updatedAtBlock, uint64(block.number));
        assertEq(after_.setBy, CONTROLLER);
    }

    function test_a_withdrawal_that_never_happened_cannot_be_recorded() public {
        // From UNSET there is no claim to withdraw.
        vm.prank(CONTROLLER);
        vm.expectRevert(
            abi.encodeWithSelector(
                ServiceDomainResolver.NothingToWithdraw.selector,
                service,
                ServiceDomainResolver.Disposition.UNSET
            )
        );
        resolver.clearDomain(service);

        // Nor from a deliberate declaration of no domain: nothing was ever claimed.
        vm.prank(CONTROLLER);
        resolver.declareNoDomain(service);
        vm.prank(CONTROLLER);
        vm.expectRevert(
            abi.encodeWithSelector(
                ServiceDomainResolver.NothingToWithdraw.selector,
                service,
                ServiceDomainResolver.Disposition.NO_DOMAIN
            )
        );
        resolver.clearDomain(service);

        // Nor twice: the second call would restamp a withdrawal that already happened.
        _claim(CONTROLLER, service, DOMAIN);
        vm.prank(CONTROLLER);
        resolver.clearDomain(service);
        vm.prank(CONTROLLER);
        vm.expectRevert(
            abi.encodeWithSelector(
                ServiceDomainResolver.NothingToWithdraw.selector,
                service,
                ServiceDomainResolver.Disposition.CLEARED
            )
        );
        resolver.clearDomain(service);
    }

    /// @dev Re-affirmation is a real write: the off-chain DNS re-check job reads `updatedAtBlock`, so an
    /// issuer restating an unchanged claim must be able to move it.
    function test_re_affirming_the_same_domain_re_anchors_the_block_rather_than_being_a_no_op() public {
        _claim(CONTROLLER, service, DOMAIN);
        ServiceDomainResolver.DomainRecord memory first = resolver.record(service);

        vm.roll(block.number + 10);
        vm.warp(block.timestamp + 3600);
        _claim(CONTROLLER, service, DOMAIN);
        ServiceDomainResolver.DomainRecord memory second = resolver.record(service);

        assertEq(second.domain, first.domain);
        assertEq(second.revision, first.revision + 1);
        assertEq(second.updatedAtBlock, first.updatedAtBlock + 10);
        assertGt(second.updatedAt, first.updatedAt);
    }

    function test_declaring_no_domain_twice_is_refused() public {
        vm.prank(CONTROLLER);
        resolver.declareNoDomain(service);
        vm.prank(CONTROLLER);
        vm.expectRevert(
            abi.encodeWithSelector(ServiceDomainResolver.AlreadyDeclaredNoDomain.selector, service)
        );
        resolver.declareNoDomain(service);
    }

    function test_a_claim_can_be_replaced_by_a_deliberate_no_domain_declaration() public {
        _claim(CONTROLLER, service, DOMAIN);
        vm.prank(CONTROLLER);
        resolver.declareNoDomain(service);

        ServiceDomainResolver.DomainRecord memory r = resolver.record(service);
        assertEq(uint8(r.disposition), uint8(ServiceDomainResolver.Disposition.NO_DOMAIN));
        assertEq(r.domain, "");
        assertEq(r.revision, 2);
    }

    function test_a_replaced_claim_reads_back_as_the_new_domain_only() public {
        _claim(CONTROLLER, service, DOMAIN);
        _claim(CONTROLLER, service, OTHER_DOMAIN);
        assertEq(resolver.record(service).domain, OTHER_DOMAIN);
    }

    // ---------------------------------------------------------------------------------------------
    // The captain's AND
    // ---------------------------------------------------------------------------------------------

    function test_a_cleared_provider_and_the_confirmed_owner_may_publish() public {
        assertTrue(resolver.canWriteDomain(service, CONTROLLER));

        vm.expectEmit(true, true, false, true, address(resolver));
        emit ServiceDomainResolver.DomainClaimed(
            service, DOMAIN, ServiceDomainResolver.Disposition.UNSET, CONTROLLER, 1, uint64(block.number)
        );
        _claim(CONTROLLER, service, DOMAIN);
        assertEq(resolver.record(service).domain, DOMAIN);
    }

    /// @dev The first half of the AND. Owner authority is real and is still not enough.
    function test_owner_authority_alone_cannot_publish_under_suspended_kyc_standing() public {
        vm.prank(AUTHORITY);
        core.setProviderStanding(PROVIDER, ProviderRegistry.Standing.SUSPENDED);

        assertFalse(resolver.canWriteDomain(service, CONTROLLER));
        vm.prank(CONTROLLER);
        vm.expectRevert(
            abi.encodeWithSelector(ServiceDomainResolver.NotAuthorized.selector, service, CONTROLLER)
        );
        resolver.claimDomain(service, DOMAIN);

        // And a service retired on its own axis is refused with the provider still cleared.
        vm.startPrank(AUTHORITY);
        core.setProviderStanding(PROVIDER, ProviderRegistry.Standing.ACTIVE);
        core.setServiceStanding(service, ProviderRegistry.Standing.RETIRED);
        vm.stopPrank();
        assertFalse(resolver.canWriteDomain(service, CONTROLLER));
    }

    /// @dev The second half. Cleared standing is a property of the service, not a licence for any caller.
    function test_cleared_standing_alone_does_not_let_a_stranger_publish() public {
        assertTrue(resolver.canWriteDomain(service, CONTROLLER));
        assertFalse(resolver.canWriteDomain(service, STRANGER));

        vm.prank(STRANGER);
        vm.expectRevert(
            abi.encodeWithSelector(ServiceDomainResolver.NotAuthorized.selector, service, STRANGER)
        );
        resolver.claimDomain(service, DOMAIN);
    }

    /// @dev A hot issuance key is exactly the key most likely to be exposed, and publishing an identity is
    /// not its job.
    function test_an_issuance_signer_has_no_content_write_authority() public {
        uint256 issueRight = core.RIGHT_ISSUE();
        vm.prank(AUTHORITY);
        core.setRights(ISSUANCE_SIGNER, issueRight);
        assertTrue(core.canRevoke(service, ISSUANCE_SIGNER));

        assertFalse(resolver.canWriteDomain(service, ISSUANCE_SIGNER));
        vm.prank(ISSUANCE_SIGNER);
        vm.expectRevert(
            abi.encodeWithSelector(ServiceDomainResolver.NotAuthorized.selector, service, ISSUANCE_SIGNER)
        );
        resolver.claimDomain(service, DOMAIN);
    }

    /// @dev `IssuerDomainRegistry`'s tier 1 was exactly this bypass. The core withholds it deliberately, so
    /// this resolver has no registrar write path; a registrar that must stop a claim deapproves the
    /// resolver instead.
    function test_the_registrar_has_no_content_write_bypass() public {
        assertTrue(core.hasRole(core.WHITELIST_ADMIN(), AUTHORITY));
        assertFalse(resolver.canWriteDomain(service, AUTHORITY));

        vm.prank(AUTHORITY);
        vm.expectRevert(
            abi.encodeWithSelector(ServiceDomainResolver.NotAuthorized.selector, service, AUTHORITY)
        );
        resolver.claimDomain(service, DOMAIN);
    }

    /// @dev The delegate path is why `authorizeClone` is not composed as the control term: it requires
    /// `claimant == owner()` exactly, so under it this test could not pass at all.
    function test_an_owner_scoped_delegate_may_publish_and_an_owner_rotation_revokes_it() public {
        vm.prank(CONTROLLER);
        core.setServiceDelegate(service, DELEGATE, SERVICE_RECORD_PERMISSION, 0);
        assertTrue(resolver.canWriteDomain(service, DELEGATE));
        _claim(DELEGATE, service, DOMAIN);
        assertEq(resolver.record(service).setBy, DELEGATE);

        // A completed clone-owner handover bumps the core's owner epoch, and the delegation was scoped to
        // the previous one.
        vm.prank(CONTROLLER);
        DogTagIssuer(service).transferOwnership(NEXT_OWNER);
        vm.prank(NEXT_OWNER);
        DogTagIssuer(service).acceptOwnership();
        vm.prank(AUTHORITY);
        core.confirmServiceOwner(service, NEXT_OWNER, 1);

        assertFalse(resolver.canWriteDomain(service, DELEGATE));
        assertFalse(resolver.canWriteDomain(service, CONTROLLER));
        assertTrue(resolver.canWriteDomain(service, NEXT_OWNER));
    }

    /// @dev Publishing content and choosing which resolver holds it are different powers. Reusing one bit
    /// for both would let an operations key move the record to a resolver of its own choosing.
    function test_the_write_uses_the_content_bit_and_not_the_resolver_selection_bit() public {
        vm.prank(CONTROLLER);
        core.setServiceDelegate(service, DELEGATE, SERVICE_DOMAIN_RESOLVER_PERMISSION, 0);
        assertTrue(core.canWriteService(service, DELEGATE, SERVICE_DOMAIN_RESOLVER_PERMISSION));
        assertFalse(resolver.canWriteDomain(service, DELEGATE));

        vm.prank(CONTROLLER);
        core.setServiceDelegate(service, DELEGATE, SERVICE_RECORD_PERMISSION, 0);
        assertTrue(resolver.canWriteDomain(service, DELEGATE));
    }

    /// @dev A clone-owner handover the registrar has not confirmed quarantines the write on both keys,
    /// which is the core's rule and must not be softened here.
    function test_an_unconfirmed_owner_handover_quarantines_the_write() public {
        vm.prank(CONTROLLER);
        DogTagIssuer(service).transferOwnership(NEXT_OWNER);
        vm.prank(NEXT_OWNER);
        DogTagIssuer(service).acceptOwnership();

        assertFalse(resolver.canWriteDomain(service, CONTROLLER));
        assertFalse(resolver.canWriteDomain(service, NEXT_OWNER));
    }

    /// @dev THE discriminating case for the fourth standing term, and the only one that distinguishes
    /// what it means from what `canWriteDomain` means. Every other case that reads it has it `false`, so
    /// each of those passes under either semantics; this one is `true` while no key can write.
    ///
    /// A quarantine is not a freeze. The registrar clears an unconfirmed handover with
    /// `confirmServiceOwner`, so the claim is still live and still maintainable, and rendering it as
    /// frozen history would be as wrong as rendering a retired service's claim as current - the same
    /// collapse from the opposite side. Folding the core's `ownerConfirmed` into
    /// `_serviceStandingIsEffective` reddens this test and nothing else.
    function test_a_quarantined_service_is_not_a_frozen_one() public {
        _claim(CONTROLLER, service, DOMAIN);

        vm.prank(CONTROLLER);
        DogTagIssuer(service).transferOwnership(NEXT_OWNER);
        vm.prank(NEXT_OWNER);
        DogTagIssuer(service).acceptOwnership();

        (,,,,, bool standing) = resolver.claimStanding(service);
        assertTrue(standing);
        assertFalse(resolver.canWriteDomain(service, CONTROLLER));
        assertFalse(resolver.canWriteDomain(service, NEXT_OWNER));

        // And confirming the handover restores the write, which a frozen service can never do.
        vm.prank(AUTHORITY);
        core.confirmServiceOwner(service, NEXT_OWNER, 1);
        assertTrue(resolver.canWriteDomain(service, NEXT_OWNER));
    }

    // ---------------------------------------------------------------------------------------------
    // Provenance against the resolver's own factory lineage
    // ---------------------------------------------------------------------------------------------

    /// @dev THE test for the lineage link, and the one that goes red if the check is removed.
    ///
    /// The service is genuinely a clone, genuinely attached, genuinely owned and genuinely cleared — the
    /// core says so — but it came from a DIFFERENT factory than the one the verification registry's
    /// immutable `rootIndex` resolves roots through, so its roots would answer `unknown root` at every
    /// verification. A domain claim it could publish would be a verified identity beside credentials
    /// that cannot verify.
    function test_a_service_the_verification_lineage_does_not_vouch_for_cannot_claim_a_domain() public {
        address offLineage = _createAndAttach(offLineageFactory, GENERATION_2B, PROVIDER, 0);
        vm.prank(CONTROLLER);
        core.setDomainResolver(offLineage, address(resolver));

        // Everything the CORE is asked holds, and the clone is genuine to the factory that made it.
        assertTrue(core.canWriteService(offLineage, CONTROLLER, SERVICE_RECORD_PERMISSION));
        assertTrue(offLineageFactory.isClone(offLineage));
        // The resolver's OWN lineage does not vouch for it, so the resolver refuses.
        assertFalse(factory.isClone(offLineage));
        assertFalse(resolver.canWriteDomain(offLineage, CONTROLLER));

        vm.prank(CONTROLLER);
        vm.expectRevert(
            abi.encodeWithSelector(ServiceDomainResolver.NotRecognizedByLineage.selector, offLineage)
        );
        resolver.claimDomain(offLineage, DOMAIN);
    }

    /// @dev Shows the refusal above is about the lineage and nothing else: the identical service, the
    /// identical caller and the identical domain succeed against a resolver whose lineage IS the factory
    /// that made it. Without this the refusal above could be passing for any unrelated reason.
    function test_the_same_claim_succeeds_against_a_resolver_on_that_services_own_lineage() public {
        address offLineage = _createAndAttach(offLineageFactory, GENERATION_2B, PROVIDER, 0);

        ServiceDomainResolver sibling =
            new ServiceDomainResolver(address(core), address(offLineageFactory));
        vm.prank(AUTHORITY);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(sibling), true);
        vm.prank(CONTROLLER);
        core.setDomainResolver(offLineage, address(sibling));

        assertTrue(sibling.canWriteDomain(offLineage, CONTROLLER));
        vm.prank(CONTROLLER);
        sibling.claimDomain(offLineage, DOMAIN);
        assertEq(sibling.record(offLineage).domain, DOMAIN);
    }

    /// @dev A stranger's contract must be told its address is not of this lineage, never that it merely
    /// lacks a permission — and a genuine provider must never be told the reverse. Two remedies, two
    /// diagnoses.
    function test_a_stranger_contract_is_refused_by_lineage_and_not_by_authorization() public {
        StrangerService stranger = new StrangerService(RECORD_TYPE, CONTROLLER);
        assertFalse(factory.isClone(address(stranger)));

        vm.prank(CONTROLLER);
        vm.expectRevert(
            abi.encodeWithSelector(ServiceDomainResolver.NotRecognizedByLineage.selector, address(stranger))
        );
        resolver.claimDomain(address(stranger), DOMAIN);
    }

    function test_the_zero_address_is_refused_before_any_dependency_is_asked() public {
        vm.prank(CONTROLLER);
        vm.expectRevert(ServiceDomainResolver.ZeroAddress.selector);
        resolver.claimDomain(address(0), DOMAIN);
    }

    // ---------------------------------------------------------------------------------------------
    // Resolver selection and the fleet-wide approval lever
    // ---------------------------------------------------------------------------------------------

    function test_a_resolver_the_service_never_selected_cannot_write_its_record() public {
        ServiceDomainResolver other = new ServiceDomainResolver(address(core), address(factory));
        vm.prank(AUTHORITY);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(other), true);

        assertFalse(other.canWriteDomain(service, CONTROLLER));
        vm.prank(CONTROLLER);
        vm.expectRevert(
            abi.encodeWithSelector(
                ServiceDomainResolver.ResolverNotSelected.selector, service, address(other)
            )
        );
        other.claimDomain(service, DOMAIN);
    }

    /// @dev The lever exists because the core never clears a stored selector — not on deapproval, and not
    /// on the generation deprecation that freezes the selection write permanently. Reading the selector
    /// alone would defeat it.
    function test_fleet_wide_deapproval_stops_writes_although_the_core_still_names_this_resolver() public {
        _claim(CONTROLLER, service, DOMAIN);

        vm.prank(AUTHORITY);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(resolver), false);

        // The core's stored selector is untouched, which is exactly why the approval term is required.
        assertEq(core.service(service).domainResolver, address(resolver));
        assertFalse(core.isResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(resolver)));

        assertFalse(resolver.canWriteDomain(service, CONTROLLER));
        vm.prank(CONTROLLER);
        vm.expectRevert(
            abi.encodeWithSelector(ServiceDomainResolver.ResolverNotApproved.selector, address(resolver))
        );
        resolver.claimDomain(service, OTHER_DOMAIN);
    }

    /// @dev A stored record outlives the permission that wrote it, so the read side re-checks too. The
    /// CONTENT is deliberately unchanged: deapproval says the resolver is no longer authoritative, not
    /// that the issuer withdrew anything.
    function test_a_record_written_before_deapproval_stops_being_authoritative() public {
        _claim(CONTROLLER, service, DOMAIN);
        assertTrue(resolver.isAuthoritativeFor(service));

        vm.prank(AUTHORITY);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(resolver), false);

        assertFalse(resolver.isAuthoritativeFor(service));
        (ServiceDomainResolver.Disposition disposition, string memory domain) =
            resolver.resolveDomain(service);
        assertEq(uint8(disposition), uint8(ServiceDomainResolver.Disposition.CLAIMED));
        assertEq(domain, DOMAIN);
    }

    // ---------------------------------------------------------------------------------------------
    // Reads that cannot collapse
    // ---------------------------------------------------------------------------------------------

    function test_claim_standing_reports_the_three_terms_separately_rather_than_one_verdict() public {
        _claim(CONTROLLER, service, DOMAIN);

        (
            ServiceDomainResolver.Disposition disposition,
            string memory domain,
            bool lineage,
            bool approved,
            bool selected,
            bool standing
        ) = resolver.claimStanding(service);
        assertEq(uint8(disposition), uint8(ServiceDomainResolver.Disposition.CLAIMED));
        assertEq(domain, DOMAIN);
        assertTrue(lineage);
        assertTrue(approved);
        assertTrue(selected);
        assertTrue(standing);

        // Deapproval fails exactly one term, and the other three still report what they observed.
        vm.prank(AUTHORITY);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(resolver), false);
        (,, lineage, approved, selected, standing) = resolver.claimStanding(service);
        assertTrue(lineage);
        assertFalse(approved);
        assertTrue(selected);
        assertTrue(standing);

        // Deselection fails a different one, so a consumer can tell which remedy applies.
        vm.prank(AUTHORITY);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(resolver), true);
        vm.prank(CONTROLLER);
        core.setDomainResolver(service, address(0));
        (,, lineage, approved, selected, standing) = resolver.claimStanding(service);
        assertTrue(lineage);
        assertTrue(approved);
        assertFalse(selected);
        assertTrue(standing);

        // And the core's own standing axis fails on its own, leaving this resolver's three terms intact.
        // Re-selection is the OWNER's structural decision, so the registrar cannot make it here.
        vm.prank(CONTROLLER);
        core.setDomainResolver(service, address(resolver));
        vm.prank(AUTHORITY);
        core.setProviderStanding(PROVIDER, ProviderRegistry.Standing.SUSPENDED);
        (,, lineage, approved, selected, standing) = resolver.claimStanding(service);
        assertTrue(lineage);
        assertTrue(approved);
        assertTrue(selected);
        assertFalse(standing);
    }

    /// @dev A `RETIRED` service standing cannot be left, so this is terminal: the write is refused for the
    /// owner, for a delegate and for the registrar alike, forever, and no per-record withdrawal exists.
    ///
    /// The honest reading of the record is BOTH halves. This resolver's own three terms are untouched -
    /// the record really is the last thing it accepted from this service - so the claim still reads
    /// `CLAIMED` and `isAuthoritativeFor` stays true. What says the claim is frozen history rather than a
    /// current one is the separate standing term, which is why it exists and why it is not folded in.
    function test_a_retired_service_freezes_its_claim_while_the_resolver_terms_stay_true() public {
        _claim(CONTROLLER, service, DOMAIN);
        _grantRecordDelegate();

        vm.prank(AUTHORITY);
        core.setServiceStanding(service, ProviderRegistry.Standing.RETIRED);

        // This resolver's three terms all still hold and the verdict with them; the fourth is the only
        // one that says the claim can no longer be maintained.
        _assertVerdictExcludesServiceStanding(service);
        (ServiceDomainResolver.Disposition disposition, string memory domain) =
            resolver.resolveDomain(service);
        assertEq(uint8(disposition), uint8(ServiceDomainResolver.Disposition.CLAIMED));
        assertEq(domain, DOMAIN);

        _assertEveryWriteIsFrozen();
    }

    /// @dev A deprecated factory generation has no reactivation path either, and reaches the same frozen
    /// state by a DIFFERENT cause - the two are asserted separately rather than assumed to share a code
    /// path, because the core reaches them through different fields.
    function test_a_deprecated_factory_generation_freezes_its_claim_the_same_way() public {
        _claim(CONTROLLER, service, DOMAIN);
        _grantRecordDelegate();

        vm.prank(AUTHORITY);
        core.deprecateFactoryGeneration(GENERATION_2);

        // The provider and the service are both still ACTIVE; only the generation moved.
        assertEq(uint8(core.provider(PROVIDER).standing), uint8(ProviderRegistry.Standing.ACTIVE));
        assertEq(uint8(core.service(service).standing), uint8(ProviderRegistry.Standing.ACTIVE));

        _assertVerdictExcludesServiceStanding(service);
        assertEq(resolver.record(service).domain, DOMAIN);

        _assertEveryWriteIsFrozen();
    }

    /// @dev Granted BEFORE a freeze, because the core refuses to grant a content permission on a service
    /// whose standing is no longer effective. Without it the delegate arm below would assert nothing.
    function _grantRecordDelegate() internal {
        vm.prank(CONTROLLER);
        core.setServiceDelegate(service, DELEGATE, SERVICE_RECORD_PERMISSION, 0);
        assertTrue(resolver.canWriteDomain(service, DELEGATE));
    }

    /// @dev No key reaches a frozen record: not the confirmed owner, not an owner-scoped delegate, and
    /// deliberately not the registrar, which has no content-write bypass here by design.
    function _assertEveryWriteIsFrozen() internal {
        address[3] memory keys = [CONTROLLER, DELEGATE, AUTHORITY];
        for (uint256 i; i < keys.length; i++) {
            assertFalse(resolver.canWriteDomain(service, keys[i]));

            vm.prank(keys[i]);
            vm.expectRevert(
                abi.encodeWithSelector(ServiceDomainResolver.NotAuthorized.selector, service, keys[i])
            );
            resolver.clearDomain(service);

            vm.prank(keys[i]);
            vm.expectRevert(
                abi.encodeWithSelector(ServiceDomainResolver.NotAuthorized.selector, service, keys[i])
            );
            resolver.declareNoDomain(service);

            vm.prank(keys[i]);
            vm.expectRevert(
                abi.encodeWithSelector(ServiceDomainResolver.NotAuthorized.selector, service, keys[i])
            );
            resolver.claimDomain(service, OTHER_DOMAIN);
        }
    }

    /// @dev The read side needs all three terms too, and deselection is the one that is easy to leave out
    /// of it: the record content and the fleet-wide approval both still hold, so only the selector says
    /// this resolver is no longer the service's. A stored record that stayed authoritative here would let a
    /// superseded resolver keep answering for a service that has moved to another one.
    function test_a_record_stops_being_authoritative_when_the_service_selects_another_resolver() public {
        _claim(CONTROLLER, service, DOMAIN);
        assertTrue(resolver.isAuthoritativeFor(service));

        ServiceDomainResolver other = new ServiceDomainResolver(address(core), address(factory));
        vm.prank(AUTHORITY);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(other), true);
        vm.prank(CONTROLLER);
        core.setDomainResolver(service, address(other));

        assertFalse(resolver.isAuthoritativeFor(service));
        // Still approved fleet-wide, and the record itself is untouched — only the selection moved.
        assertTrue(core.isResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(resolver)));
        assertEq(resolver.record(service).domain, DOMAIN);
        _assertStandingAgrees(service);
    }

    function test_is_authoritative_for_and_claim_standing_cannot_disagree() public {
        _claim(CONTROLLER, service, DOMAIN);
        _assertStandingAgrees(service);

        vm.prank(AUTHORITY);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(resolver), false);
        _assertStandingAgrees(service);

        // Deselected, with the approval restored, so the two remaining terms disagree with each other.
        vm.prank(AUTHORITY);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(resolver), true);
        vm.prank(CONTROLLER);
        core.setDomainResolver(service, address(0));
        assertFalse(resolver.isAuthoritativeFor(service));
        _assertStandingAgrees(service);

        // Also for an address the lineage never vouched for.
        _assertStandingAgrees(STRANGER);
    }

    /// @dev The verdict is exactly the three RESOLVER terms. The fourth is asserted to be outside it, not
    /// merely absent from the fold: it is the core's own standing axis and answers a different question,
    /// so a future edit that AND-ed it in would be caught here rather than silently making a frozen
    /// service's record read as though this resolver had stopped being the effective one.
    function _assertStandingAgrees(address target) internal view {
        (,, bool lineage, bool approved, bool selected,) = resolver.claimStanding(target);
        assertEq(resolver.isAuthoritativeFor(target), lineage && approved && selected);
    }

    function _assertVerdictExcludesServiceStanding(address target) internal view {
        (,, bool lineage, bool approved, bool selected, bool standing) = resolver.claimStanding(target);
        assertTrue(lineage && approved && selected);
        assertFalse(standing);
        assertTrue(resolver.isAuthoritativeFor(target));
    }

    // ---------------------------------------------------------------------------------------------
    // The canonical-domain grammar, preserved
    // ---------------------------------------------------------------------------------------------

    function test_the_canonical_domain_grammar_accepts_what_a_resolver_can_query() public {
        string[5] memory accepted =
            ["moh.gov.sg", "a.b", "vet-clinic.example.co.uk", "x1.2y.zz", "sub.sub2.sub3.example.com"];
        for (uint256 i; i < accepted.length; i++) {
            _claim(CONTROLLER, service, accepted[i]);
            assertEq(resolver.record(service).domain, accepted[i]);
        }
    }

    /// @dev Uppercase is REJECTED rather than folded, so the stored value is unambiguously the canonical
    /// query name. An unusable claim is unrepresentable rather than a silent lookup failure that would
    /// surface as "could not check" forever.
    function test_the_canonical_domain_grammar_rejects_anything_a_resolver_could_not_query() public {
        string[13] memory rejected = [
            "", // empty
            "MOH.gov.sg", // uppercase
            "localhost", // single label
            ".moh.gov.sg", // leading dot
            "moh.gov.sg.", // trailing dot
            "moh..gov", // empty label
            "-moh.gov.sg", // label starts with '-'
            "moh-.gov.sg", // label ends with '-'
            "moh.gov.sg-", // final label ends with '-'
            "moh_gov.sg", // non-LDH
            "moh gov.sg", // space
            "moh.gov.s\xc3\xa9", // non-ascii byte (an unnormalised IDN, not a query name)
            "moh.gov.sg/path" // not a name
        ];
        for (uint256 i; i < rejected.length; i++) {
            vm.prank(CONTROLLER);
            vm.expectRevert(ServiceDomainResolver.BadDomain.selector);
            resolver.claimDomain(service, rejected[i]);
        }
    }

    function test_the_canonical_domain_grammar_enforces_the_label_and_total_length_caps() public {
        string memory label63 = _repeat("a", 63);
        _claim(CONTROLLER, service, string.concat(label63, ".sg"));

        vm.prank(CONTROLLER);
        vm.expectRevert(ServiceDomainResolver.BadDomain.selector);
        resolver.claimDomain(service, string.concat(_repeat("a", 64), ".sg"));

        // 253 total is the cap and is accepted; 254 is not.
        string memory atCap = _dottedName(253);
        assertEq(bytes(atCap).length, 253);
        _claim(CONTROLLER, service, atCap);

        string memory overCap = _dottedName(254);
        assertEq(bytes(overCap).length, 254);
        vm.prank(CONTROLLER);
        vm.expectRevert(ServiceDomainResolver.BadDomain.selector);
        resolver.claimDomain(service, overCap);

        assertEq(resolver.MAX_DOMAIN_LENGTH(), 253);
    }

    function _repeat(string memory unit, uint256 times) internal pure returns (string memory out) {
        for (uint256 i; i < times; i++) {
            out = string.concat(out, unit);
        }
    }

    /// @dev A name of exactly `total` bytes built from legal 63-byte labels.
    function _dottedName(uint256 total) internal pure returns (string memory out) {
        while (bytes(out).length + 64 <= total) {
            out = string.concat(out, _repeat("a", 63), ".");
        }
        out = string.concat(out, _repeat("a", total - bytes(out).length));
    }

    // ---------------------------------------------------------------------------------------------
    // Construction: the dependencies are behaviour-probed, and the two failures stay apart
    // ---------------------------------------------------------------------------------------------

    function test_construction_refuses_a_zero_dependency() public {
        vm.expectRevert(ServiceDomainResolver.ZeroAddress.selector);
        new ServiceDomainResolver(address(0), address(factory));
        vm.expectRevert(ServiceDomainResolver.ZeroAddress.selector);
        new ServiceDomainResolver(address(core), address(0));
    }

    /// @dev An EOA staticcall SUCCEEDS with empty returndata, which is the silent shape a code-size check
    /// alone would miss — so both checks exist and this asserts the code one fires first.
    function test_construction_refuses_a_dependency_with_no_code() public {
        vm.expectRevert(abi.encodeWithSelector(ServiceDomainResolver.DependencyHasNoCode.selector, STRANGER));
        new ServiceDomainResolver(STRANGER, address(factory));
    }

    function test_construction_refuses_a_dependency_that_does_not_answer() public {
        SilentDependency silent = new SilentDependency();
        vm.expectRevert(
            abi.encodeWithSelector(
                ServiceDomainResolver.DependencyDoesNotAnswer.selector,
                address(silent),
                ICloneProvenance.isClone.selector
            )
        );
        new ServiceDomainResolver(address(core), address(silent));

        vm.expectRevert(
            abi.encodeWithSelector(
                ServiceDomainResolver.DependencyDoesNotAnswer.selector,
                address(silent),
                ProviderRegistry.isResolverApproved.selector
            )
        );
        new ServiceDomainResolver(address(silent), address(factory));
    }

    /// @dev A definite `true` from a stub is a DIFFERENT diagnosis from a non-answer, and collapsing them
    /// would send an operator hunting a missing selector when the real fault is a predicate that
    /// recognizes any address handed to it.
    function test_construction_refuses_a_factory_that_recognizes_every_address() public {
        PermissiveFactoryStub permissive = new PermissiveFactoryStub();
        vm.expectRevert(
            abi.encodeWithSelector(
                ServiceDomainResolver.FactoryRecognizesEveryAddress.selector, address(permissive)
            )
        );
        new ServiceDomainResolver(address(core), address(permissive));
    }

    function test_construction_refuses_a_core_whose_content_permission_is_zero() public {
        ZeroPermissionCoreStub zero = new ZeroPermissionCoreStub();
        vm.expectRevert(
            abi.encodeWithSelector(ServiceDomainResolver.CorePermissionIsZero.selector, address(zero))
        );
        new ServiceDomainResolver(address(zero), address(factory));
    }

    /// @dev The permission mask is resolved from the core rather than restated, so this pins both the
    /// interface signature and the resolved value against the REAL core.
    function test_the_content_permission_is_resolved_from_the_real_core() public view {
        assertEq(resolver.recordPermission(), core.SERVICE_PERMISSION_RECORD());
        assertEq(
            resolver.recordPermission(), ICoreRecordPermission(address(core)).SERVICE_PERMISSION_RECORD()
        );
        assertTrue(resolver.recordPermission() != 0);
        // And it is the content bit, not the resolver-selection bit.
        assertTrue(resolver.recordPermission() != core.SERVICE_PERMISSION_DOMAIN_RESOLVER());
    }

    function test_the_dependencies_are_immutable_and_are_the_ones_supplied() public view {
        assertEq(address(resolver.core()), address(core));
        assertEq(address(resolver.factory()), address(factory));
    }

    // ---------------------------------------------------------------------------------------------
    // Enumeration
    // ---------------------------------------------------------------------------------------------

    /// @dev A withdrawn claim stays listed so the off-chain re-check job can still observe that a domain
    /// was withdrawn, rather than the row simply vanishing.
    function test_a_withdrawn_claim_stays_listed_for_the_recheck_job() public {
        assertEq(resolver.recordedServiceCount(), 0);
        _claim(CONTROLLER, service, DOMAIN);
        assertEq(resolver.recordedServiceCount(), 1);

        vm.prank(CONTROLLER);
        resolver.clearDomain(service);
        assertEq(resolver.recordedServiceCount(), 1);
        assertEq(resolver.recordedServicePage(0, 10)[0], service);

        // A second write for the same service does not list it twice.
        _claim(CONTROLLER, service, DOMAIN);
        assertEq(resolver.recordedServiceCount(), 1);
    }

    function test_a_no_domain_declaration_is_listed_too() public {
        vm.prank(CONTROLLER);
        resolver.declareNoDomain(service);
        assertEq(resolver.recordedServiceCount(), 1);
        assertEq(resolver.recordedServicePage(0, 1)[0], service);
    }

    function test_enumeration_is_bounded_and_rejects_a_bad_page() public {
        vm.startPrank(AUTHORITY);
        _registerProvider(OTHER_PROVIDER, CONTROLLER);
        core.setProviderStanding(OTHER_PROVIDER, ProviderRegistry.Standing.ACTIVE);
        core.setServiceCreationApproval(OTHER_PROVIDER, RECORD_TYPE, true);
        vm.stopPrank();
        address second = _createAndAttach(factory, GENERATION_2, OTHER_PROVIDER, 1);
        vm.prank(CONTROLLER);
        core.setDomainResolver(second, address(resolver));

        _claim(CONTROLLER, service, DOMAIN);
        _claim(CONTROLLER, second, OTHER_DOMAIN);
        assertEq(resolver.recordedServiceCount(), 2);

        address[] memory firstPage = resolver.recordedServicePage(0, 1);
        assertEq(firstPage.length, 1);
        assertEq(firstPage[0], service);
        address[] memory secondPage = resolver.recordedServicePage(1, 1);
        assertEq(secondPage[0], second);
        // A cursor at the end is a legal empty page, not an error.
        assertEq(resolver.recordedServicePage(2, 1).length, 0);

        // Resolved BEFORE arming the revert expectation: an `expectRevert` applies to the next call, and a
        // getter read inside the argument list would be that call.
        uint256 overCap = resolver.MAX_PAGE_SIZE() + 1;

        vm.expectRevert(ServiceDomainResolver.BadPage.selector);
        resolver.recordedServicePage(0, 0);
        vm.expectRevert(ServiceDomainResolver.BadPage.selector);
        resolver.recordedServicePage(0, overCap);
        vm.expectRevert(ServiceDomainResolver.BadPage.selector);
        resolver.recordedServicePage(3, 1);
    }

    // ---------------------------------------------------------------------------------------------
    // What the resolver deliberately does not carry
    // ---------------------------------------------------------------------------------------------

    /// @dev A generation-2 clone's `name()` is permanently empty by construction, so a resolver that
    /// published a name would be publishing a provider-chosen string beside a green check. Registrar
    /// identity comes from the core's anchor instead.
    function test_no_identity_or_description_surface_exists_here() public view {
        // The identity anchor lives on the core, keyed by provider, and is registrar-written.
        ProviderRegistry.PublicIdentityAnchor memory anchor = core.publicIdentityAnchor(PROVIDER);
        assertEq(anchor.digest, IDENTITY_DIGEST);

        // And the resolver's whole record is a disposition plus a domain plus provenance stamps.
        ServiceDomainResolver.DomainRecord memory r = resolver.record(service);
        assertEq(uint8(r.disposition), uint8(ServiceDomainResolver.Disposition.UNSET));
        assertEq(r.domain, "");
    }
}
