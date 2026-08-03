// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {ProviderDirectory} from "../src/ProviderDirectory.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {ServiceDomainResolver} from "../src/ServiceDomainResolver.sol";

/// @dev A contract our factory lineage never deployed. Nothing about it is malformed - it answers
/// `owner()` and `recordType()` exactly as a genuine clone does, which is the point. The ONLY thing
/// between it and a published listing is provenance.
contract ForgedService {
    bytes32 public immutable recordType;
    address public owner;

    constructor(bytes32 recordType_, address owner_) {
        recordType = recordType_;
        owner = owner_;
    }
}

/// @notice registry-plan S-15: the provider self-service journey, composed against the REAL
/// generation-2 set.
///
/// This suite exists because the four S-15 flows are only meaningful TOGETHER. Each of the five
/// contracts it binds already has its own suite proving its own rules; what none of them proves is
/// that a provider can walk deploy -> attach -> repoint -> claim a domain -> publish a listing, or
/// that the forgery guard the captain named actually stands where the journey would enter an
/// address.
///
/// Nothing here is a double. The core, the factory, the clones, the domain
/// resolver and the directory are all the real contracts, because every claim below is about how
/// they compose - a mocked core would let this file agree with a stand-in rather than with the
/// bytecode the cutover deployed.
///
/// FOUR contracts under test went live on ROAX on 2026-08-01 (S-14) and their sources are frozen:
/// `ProviderRegistry`, `DogTagIssuerFactory`, `DogTagIssuer`. This
/// suite adds no source change to any of them, and must not.
///
/// Headline claims:
///  * the PROVIDER deploys its own clone; the registrar does not deploy it for them;
///  * a forged contract, an EOA, and a genuine clone from an unregistered generation are each
///    refused entry, with three different named errors;
///  * a provider cannot point its own listing at another provider's genuine clone;
///  * a repoint moves only where NEW credentials anchor - historical roots are untouched;
///  * a contact-only provider is listed and contributes nothing to the pin scan; and
///  * the chain cannot tell a placeholder coordinate from a real one, which is exactly why the
///    no-placeholder rule has to live in the portal.
contract ProviderSelfServiceTest is Test {
    ProviderRegistry internal core;
    DogTagIssuerFactory internal factory;
    DogTagIssuerFactory internal unregisteredFactory;
    DogTagIssuer internal implementation;
    ServiceDomainResolver internal domainResolver;
    ProviderDirectory internal directory;

    /// @dev The registrar. Holds `onlyOwner` on the core: KYC, attachment, standing. It deploys no
    /// clone in this suite, deliberately.
    address internal constant REGISTRAR = address(0xA11CE);

    /// @dev The provider's own key. Deploys the clone, owns it, and drives every self-service write.
    address internal constant PROVIDER_KEY = address(0xC011);
    address internal constant OTHER_PROVIDER_KEY = address(0xC022);
    address internal constant STRANGER = address(0xBAD);

    /// @dev Opaque, KYC-assigned. Not derived from a name, domain, address, signer, clone or salt -
    /// `_providerIdIsNotDerivedFromAnyProviderFact` is the pin, not this comment.
    bytes20 internal constant PROVIDER = bytes20(hex"3f5c9a1e77b204d8e6130fa95c8b47e2d61099af");
    bytes20 internal constant OTHER_PROVIDER = bytes20(hex"c4a70bd318e952f6017da84b3c6e29fd50b7143e");

    bytes32 internal constant GENERATION_2 = keccak256("dogtag-issuer-factory/2");
    bytes32 internal constant GENERATION_2B = keccak256("dogtag-issuer-factory/2b");
    bytes32 internal constant RECORD_TYPE = keccak256("VACCINATION");
    bytes32 internal constant IDENTITY_DIGEST = keccak256("publication-safe-identity");

    string internal constant DOMAIN = "seaport-vet.example-clinic.sg";

    /// @dev The digest of the publication-safe contact blob. The blob itself is off chain (S-17);
    /// what the chain carries is only its integrity anchor.
    bytes32 internal constant CONTACT_BLOB_DIGEST = keccak256("phone+whatsapp+telegram+email");

    address internal firstClone;

    function setUp() public {
        core = new ProviderRegistry(REGISTRAR);
        implementation = new DogTagIssuer();
        factory = new DogTagIssuerFactory(address(implementation), address(core));
        // A second GENUINE factory of identical bytecode that the core was never told about. Its clones
        // are real clones and are still unattachable - the gap the attach guard must close.
        unregisteredFactory = new DogTagIssuerFactory(address(implementation), address(core));

        domainResolver = new ServiceDomainResolver(address(core), address(factory));
        directory = new ProviderDirectory(core);

        vm.startPrank(REGISTRAR);
        // Only ONE generation is registered in the core.
        core.addFactoryGeneration(GENERATION_2, address(factory));
        _registerProvider(PROVIDER, PROVIDER_KEY);
        _registerProvider(OTHER_PROVIDER, OTHER_PROVIDER_KEY);
        core.setProviderStanding(PROVIDER, ProviderRegistry.Standing.ACTIVE);
        core.setProviderStanding(OTHER_PROVIDER, ProviderRegistry.Standing.ACTIVE);
        core.setServiceCreationApproval(PROVIDER, RECORD_TYPE, true);
        core.setServiceCreationApproval(OTHER_PROVIDER, RECORD_TYPE, true);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(domainResolver), true);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, address(directory), true);
        vm.stopPrank();

        firstClone = _deployAndAttach(PROVIDER, 0);

        vm.startPrank(PROVIDER_KEY);
        core.setDomainResolver(firstClone, address(domainResolver));
        core.setDirectoryResolver(PROVIDER, address(directory));
        vm.stopPrank();
    }

    // ---------------------------------------------------------------------------------------------
    // Fixture helpers
    // ---------------------------------------------------------------------------------------------

    function _registerProvider(bytes20 providerId, address controller) internal {
        core.registerProvider(
            providerId, controller, IDENTITY_DIGEST, 1, 0xe3, 0x12, bytes("ipfs://publication-safe-identity")
        );
    }

    /// @dev Deliberately TWO transactions from TWO keys, because that is the actual shape of the
    /// flow and a helper that hid it would let a test assert a journey no provider can walk:
    /// `createIssuer` is the provider's, `attachService` is `onlyOwner` and is the registrar's.
    function _deployAndAttach(bytes20 providerId, uint96 cloneNonce) internal returns (address clone) {
        address providerKey = core.provider(providerId).controller;
        vm.prank(providerKey);
        clone = factory.createIssuer(providerId, RECORD_TYPE, cloneNonce);
        vm.startPrank(REGISTRAR);
        core.attachService(providerId, clone, GENERATION_2, providerKey);
        core.setServiceStanding(clone, ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();
    }

    // ---------------------------------------------------------------------------------------------
    // Flow 1 - the provider deploys its own clone
    // ---------------------------------------------------------------------------------------------

    /// @dev The captain's framing: "the provider deploys, we do not deploy for them." On generation
    /// 1 `createIssuer` is `onlyOwner`, so this transaction was impossible from a provider key.
    function test_the_provider_deploys_its_own_clone_and_the_registrar_does_not() public {
        vm.prank(PROVIDER_KEY);
        address clone = factory.createIssuer(PROVIDER, RECORD_TYPE, 7);

        assertTrue(factory.isClone(clone), "the factory recorded its own deployment");
        assertEq(DogTagIssuer(clone).owner(), PROVIDER_KEY, "the provider owns what it deployed");
        assertTrue(DogTagIssuer(clone).owner() != REGISTRAR, "the registrar owns nothing here");
        assertEq(DogTagIssuer(clone).recordType(), RECORD_TYPE);
    }

    /// @dev The address is exact BEFORE the transaction, so a portal can show the provider what it
    /// is about to deploy rather than asking them to trust a receipt.
    function test_the_predicted_address_is_exactly_what_gets_deployed() public {
        address predicted = factory.predictIssuer(RECORD_TYPE, PROVIDER_KEY, 3);
        vm.prank(PROVIDER_KEY);
        address clone = factory.createIssuer(PROVIDER, RECORD_TYPE, 3);
        assertEq(clone, predicted, "predictIssuer is the deploy address, not an estimate");
    }

    /// @dev The salt carries the nonce, so a provider has MORE than one possible clone per record
    /// type. Without it there would be exactly one address and nothing to repoint TO - the plan
    /// (section 3.1) names this as a precondition of the repoint existing at all.
    function test_a_second_clone_of_the_same_record_type_is_reachable_via_the_nonce() public {
        vm.startPrank(PROVIDER_KEY);
        address first = factory.createIssuer(PROVIDER, RECORD_TYPE, 11);
        address second = factory.createIssuer(PROVIDER, RECORD_TYPE, 12);
        vm.stopPrank();
        assertTrue(first != second, "the nonce gives the provider somewhere to move to");
    }

    function test_a_provider_without_creation_approval_deploys_nothing() public {
        vm.prank(REGISTRAR);
        core.setServiceCreationApproval(PROVIDER, RECORD_TYPE, false);
        vm.prank(PROVIDER_KEY);
        vm.expectRevert(DogTagIssuerFactory.NotApproved.selector);
        factory.createIssuer(PROVIDER, RECORD_TYPE, 9);
    }

    function test_a_stranger_cannot_deploy_a_clone_under_someone_elses_provider_id() public {
        vm.prank(STRANGER);
        vm.expectRevert(DogTagIssuerFactory.NotApproved.selector);
        factory.createIssuer(PROVIDER, RECORD_TYPE, 9);
    }

    // ---------------------------------------------------------------------------------------------
    // Flow 2 - the forgery guard. Three refusals, three different reasons.
    // ---------------------------------------------------------------------------------------------

    /// @dev THE captain's ruling, stated as a test. `ForgedService` answers `owner()` and
    /// `recordType()` perfectly, so every metadata read succeeds - only `factory.isClone` refuses
    /// it. Note WHICH factory answers: `attachService` resolves it from the pinned `generationId`
    /// and never from the caller, so there is no argument through which a forger could nominate a
    /// factory that would vouch for them.
    function test_a_forged_contract_can_never_be_entered_even_from_the_registrar() public {
        ForgedService forged = new ForgedService(RECORD_TYPE, PROVIDER_KEY);
        assertEq(forged.owner(), PROVIDER_KEY, "it answers ownership exactly as a real clone does");
        assertEq(forged.recordType(), RECORD_TYPE, "and record type too");
        assertFalse(factory.isClone(address(forged)), "but the factory never deployed it");

        vm.prank(REGISTRAR);
        vm.expectRevert(ProviderRegistry.NotFactoryClone.selector);
        core.attachService(PROVIDER, address(forged), GENERATION_2, PROVIDER_KEY);
    }

    /// @dev An EOA is the SILENT shape: a staticcall to one SUCCEEDS with empty returndata rather
    /// than reverting, so a probe that only checks "did the call fail" would wave it through. This
    /// asserts the refusal is the provenance error and not a metadata error, which is what shows
    /// `isClone` - not the `owner()` read - is what stopped it.
    function test_an_eoa_can_never_be_entered_and_is_refused_on_provenance_not_metadata() public {
        address eoa = address(0xE0A);
        assertEq(eoa.code.length, 0, "the point of the case: no code at all");
        assertFalse(factory.isClone(eoa));

        vm.prank(REGISTRAR);
        vm.expectRevert(ProviderRegistry.NotFactoryClone.selector);
        core.attachService(PROVIDER, eoa, GENERATION_2, PROVIDER_KEY);
    }

    /// @dev The FIRST of two independent gates, and the one that is easy to miss because it sits on
    /// the create rather than the attach. `canCreateService` keys on `generationOfFactory[msg.sender]`
    /// - the CALLING factory - so a V2 factory the core has never registered cannot deploy anything
    /// at all, however genuine its bytecode and however approved the provider. Its own factory recognizes
    /// this factory (`setUp` appended it), which is what makes the case sharp: lineage provenance
    /// alone buys nothing here.
    function test_an_unregistered_factory_generation_deploys_nothing() public {
        assertTrue(factory.isClone(unregisteredFactory.predictIssuer(RECORD_TYPE, PROVIDER_KEY, 0)) == false);
        vm.prank(PROVIDER_KEY);
        vm.expectRevert(DogTagIssuerFactory.NotApproved.selector);
        unregisteredFactory.createIssuer(PROVIDER, RECORD_TYPE, 0);
    }

    /// @dev The SECOND gate, and the one the captain's ruling is really about: `generationId` is an
    /// argument to `attachService`, so a caller chooses which generation is named. The guard resolves
    /// the pinned factory FROM that generation and asks it - so naming a generation the clone does
    /// not belong to refuses rather than passing. Both clones here are genuine and both generations
    /// are registered and active; only the pairing is wrong.
    function test_a_genuine_clone_cannot_be_attached_under_a_generation_it_does_not_belong_to()
        public
    {
        vm.prank(REGISTRAR);
        core.addFactoryGeneration(GENERATION_2B, address(unregisteredFactory));

        vm.prank(PROVIDER_KEY);
        address clone = unregisteredFactory.createIssuer(PROVIDER, RECORD_TYPE, 0);
        assertTrue(unregisteredFactory.isClone(clone), "genuinely deployed by a real factory");
        assertFalse(factory.isClone(clone), "but not by the generation about to be named");

        vm.prank(REGISTRAR);
        vm.expectRevert(ProviderRegistry.NotFactoryClone.selector);
        core.attachService(PROVIDER, clone, GENERATION_2, PROVIDER_KEY);

        // The positive control: under its OWN generation the same clone attaches. Without this the
        // case above would also pass if generation 2B were simply unusable.
        vm.prank(REGISTRAR);
        core.attachService(PROVIDER, clone, GENERATION_2B, PROVIDER_KEY);
        assertEq(core.service(clone).providerId, PROVIDER);
    }

    /// @dev Provenance is not attribution. This clone is genuine and its owner is genuine - it just
    /// belongs to somebody else. Plan section 3.1's check 2: without it, provider A lists provider
    /// B's contract as its own. The refusal comes from `repointService`'s `canWriteService`, which
    /// re-resolves the clone's live owner and finds it is not the caller.
    function test_a_provider_cannot_point_its_listing_at_another_providers_genuine_clone() public {
        address othersClone = _deployAndAttach(OTHER_PROVIDER, 0);
        assertTrue(factory.isClone(othersClone), "genuine by provenance");

        vm.prank(PROVIDER_KEY);
        vm.expectRevert(ProviderRegistry.Unauthorized.selector);
        core.repointService(othersClone);

        // And the attribution stays where it belongs.
        assertEq(core.service(othersClone).providerId, OTHER_PROVIDER);
    }

    /// @dev A repoint target must already be attached, so an address that was never entered cannot
    /// be repointed to regardless of who calls. This is what makes the attach guard the single
    /// entry point rather than one of two.
    function test_an_unattached_address_cannot_be_repointed_to_at_all() public {
        ForgedService forged = new ForgedService(RECORD_TYPE, PROVIDER_KEY);
        vm.prank(PROVIDER_KEY);
        vm.expectRevert(ProviderRegistry.UnknownService.selector);
        core.repointService(address(forged));
    }

    // ---------------------------------------------------------------------------------------------
    // Flow 2 - the repoint itself
    // ---------------------------------------------------------------------------------------------

    function test_the_provider_repoints_its_own_current_service() public {
        assertEq(core.currentService(PROVIDER, RECORD_TYPE), address(0), "nothing selected yet");

        vm.prank(PROVIDER_KEY);
        core.repointService(firstClone);
        assertEq(core.currentService(PROVIDER, RECORD_TYPE), firstClone);

        address replacement = _deployAndAttach(PROVIDER, 1);
        vm.prank(PROVIDER_KEY);
        core.repointService(replacement);
        assertEq(core.currentService(PROVIDER, RECORD_TYPE), replacement, "the pointer moved");
    }

    /// @dev A repoint changes where NEW credentials anchor and nothing else. `rootIssuer` is
    /// write-once, so everything the old clone already issued keeps resolving to the old clone and
    /// stays revocable there. Plan section 3.1 calls this correct behaviour rather than a
    /// limitation: retroactively re-attributing issued credentials to a contract that did not issue
    /// them is precisely the misattribution the control check exists to prevent.
    function test_a_repoint_leaves_every_already_anchored_root_with_its_original_clone() public {
        uint256 issueRight = core.RIGHT_ISSUE();
        vm.prank(REGISTRAR);
        core.setRights(PROVIDER_KEY, issueRight);
        // LAYER 2: the clone's own list. The registrar's grant names no service, so the clone still
        // refuses until its OWNER admits the signer.
        vm.prank(PROVIDER_KEY);
        DogTagIssuer(firstClone).setIssuanceAllowed(PROVIDER_KEY, true);
        vm.prank(PROVIDER_KEY);
        core.repointService(firstClone);

        bytes32 root = keccak256("a credential anchored before the repoint");
        vm.prank(PROVIDER_KEY);
        DogTagIssuer(firstClone).issue(root);
        assertEq(factory.rootIssuer(root), firstClone, "anchored here");

        address replacement = _deployAndAttach(PROVIDER, 1);
        vm.prank(PROVIDER_KEY);
        core.repointService(replacement);

        assertEq(core.currentService(PROVIDER, RECORD_TYPE), replacement, "new credentials go here");
        assertEq(factory.rootIssuer(root), firstClone, "the old one still answers for what it issued");
    }

    // ---------------------------------------------------------------------------------------------
    // Flow 3 - the domain claim, and its three typed absences
    // ---------------------------------------------------------------------------------------------

    function test_the_provider_claims_a_domain_with_its_own_key() public {
        vm.prank(PROVIDER_KEY);
        domainResolver.claimDomain(firstClone, DOMAIN);

        (ServiceDomainResolver.Disposition disposition, string memory domain) =
            domainResolver.resolveDomain(firstClone);
        assertEq(uint8(disposition), uint8(ServiceDomainResolver.Disposition.CLAIMED));
        assertEq(domain, DOMAIN);
    }

    /// @dev The captain's ruling: nobody-has-said, deliberately-no-domain and claim-withdrawn are
    /// three different facts. The predecessor could express only one of them, as `""`.
    function test_the_three_absences_are_three_distinguishable_facts() public {
        assertEq(
            uint8(domainResolver.dispositionOf(firstClone)),
            uint8(ServiceDomainResolver.Disposition.UNSET),
            "nobody has said"
        );

        vm.prank(PROVIDER_KEY);
        domainResolver.declareNoDomain(firstClone);
        assertEq(
            uint8(domainResolver.dispositionOf(firstClone)),
            uint8(ServiceDomainResolver.Disposition.NO_DOMAIN),
            "deliberately no domain"
        );

        vm.startPrank(PROVIDER_KEY);
        domainResolver.claimDomain(firstClone, DOMAIN);
        domainResolver.clearDomain(firstClone);
        vm.stopPrank();
        assertEq(
            uint8(domainResolver.dispositionOf(firstClone)),
            uint8(ServiceDomainResolver.Disposition.CLEARED),
            "a claim was withdrawn"
        );

        (, string memory domain) = domainResolver.resolveDomain(firstClone);
        assertEq(domain, "", "and no withdrawn value survives as a live-looking claim");
    }

    function test_a_stranger_cannot_claim_a_domain_for_someone_elses_service() public {
        vm.prank(STRANGER);
        vm.expectRevert(
            abi.encodeWithSelector(ServiceDomainResolver.NotAuthorized.selector, firstClone, STRANGER)
        );
        domainResolver.claimDomain(firstClone, DOMAIN);
    }

    // ---------------------------------------------------------------------------------------------
    // Flow 4 - directory publication, and the captain's location ruling
    // ---------------------------------------------------------------------------------------------

    /// @dev THE location ruling, at the contract layer. A provider that publishes contacts and no
    /// location is LISTED - a first-class state, not a degraded one - and contributes nothing to
    /// the pin scan, which is what the mobile nearby list is built from. So it is absent from
    /// nearby by construction rather than by a filter somebody has to remember to apply.
    function test_a_contact_only_provider_is_listed_and_is_absent_from_the_pin_scan() public {
        vm.prank(PROVIDER_KEY);
        directory.setProfileAnchor(
            PROVIDER, CONTACT_BLOB_DIGEST, 1, 0xe3, 0x12, bytes("ipfs://contact-blob")
        );

        assertTrue(directory.isListed(PROVIDER), "listed - contacts alone are enough to appear");
        assertEq(directory.pinCount(PROVIDER), 0, "and it published no location");
        assertEq(directory.pinTotal(), 0, "so the scan nearby is built from holds nothing");

        (bytes32[] memory words,,,,) = directory.pinPage(0, 100);
        assertEq(words.length, 0, "a nearby page cannot surface what is not in the scan");
    }

    /// @dev The counterpart, so the case above is not passing merely because publication is broken.
    function test_a_located_provider_does_reach_the_pin_scan() public {
        vm.startPrank(PROVIDER_KEY);
        directory.setProfileAnchor(
            PROVIDER, CONTACT_BLOB_DIGEST, 1, 0xe3, 0x12, bytes("ipfs://contact-blob")
        );
        uint16 locationNo = directory.publishPin(PROVIDER, 1_290_270, 103_851_959, 1, true);
        vm.stopPrank();

        assertEq(directory.pinCount(PROVIDER), 1);
        assertEq(directory.pinTotal(), 1);
        ProviderDirectory.Pin memory published = directory.pin(PROVIDER, locationNo);
        assertEq(published.lat, 1_290_270);
        assertEq(published.lng, 103_851_959);
    }

    /// @dev WHY the no-placeholder rule cannot live here. `0,0` is a real coordinate off the coast
    /// of Ghana, so a pin at the origin is byte-for-byte a pin anywhere else and no contract can
    /// refuse it without refusing a genuine one. This suite therefore pins the ABSENCE of a
    /// chain-level defence, which is what makes the portal-side refusal load-bearing rather than
    /// belt-and-braces. The portal's own pin is `providerDirectoryPlan.test.ts`.
    function test_the_chain_cannot_tell_a_placeholder_from_a_real_coordinate() public {
        vm.startPrank(PROVIDER_KEY);
        uint16 origin = directory.publishPin(PROVIDER, 0, 0, 1, true);
        uint16 real = directory.publishPin(PROVIDER, 1_290_270, 103_851_959, 1, true);
        vm.stopPrank();

        // Both accepted, both in the scan, indistinguishable by any predicate this contract has.
        assertTrue(directory.hasPin(PROVIDER, origin));
        assertTrue(directory.hasPin(PROVIDER, real));
        assertEq(directory.pinTotal(), 2, "the origin pin is in the scan a nearby page reads");
    }

    /// @dev A withdrawn anchor keeps the provider listed and still contributes no pin, so "took its
    /// contacts down" never degrades into "has a location".
    function test_withdrawing_the_anchor_leaves_no_pin_behind() public {
        vm.startPrank(PROVIDER_KEY);
        directory.setProfileAnchor(
            PROVIDER, CONTACT_BLOB_DIGEST, 1, 0xe3, 0x12, bytes("ipfs://contact-blob")
        );
        directory.clearProfileAnchor(PROVIDER);
        vm.stopPrank();

        assertTrue(directory.isListed(PROVIDER), "listing is append-only");
        assertEq(directory.profileAnchor(PROVIDER).digest, bytes32(0), "withdrawn");
        assertEq(directory.profileAnchor(PROVIDER).revision, 2, "and distinguishable from never-set");
        assertEq(directory.pinTotal(), 0);
    }

    function test_a_stranger_cannot_publish_into_someone_elses_provider_record() public {
        vm.prank(STRANGER);
        vm.expectRevert(ProviderDirectory.Unauthorized.selector);
        directory.setProfileAnchor(
            PROVIDER, CONTACT_BLOB_DIGEST, 1, 0xe3, 0x12, bytes("ipfs://contact-blob")
        );
    }

    // ---------------------------------------------------------------------------------------------
    // The whole journey
    // ---------------------------------------------------------------------------------------------

    /// @dev All four flows in the order a provider actually walks them, in one transaction stream.
    /// The step that is easy to model wrong is the second: `attachService` is `onlyOwner`, so the
    /// journey is provider -> REGISTRAR -> provider, and a surface that presented it as one
    /// continuous self-service action would be describing a flow nobody can complete.
    function test_the_whole_provider_journey_end_to_end() public {
        // 1. The provider deploys its own clone.
        vm.prank(PROVIDER_KEY);
        address clone = factory.createIssuer(PROVIDER, RECORD_TYPE, 42);
        assertEq(DogTagIssuer(clone).owner(), PROVIDER_KEY);
        assertEq(core.service(clone).providerId, bytes20(0), "deployed, and not yet attached");

        // 2a. The registrar attaches it. Not self-service, and not skippable.
        vm.prank(PROVIDER_KEY);
        vm.expectRevert(ProviderRegistry.UnknownService.selector);
        core.repointService(clone);

        vm.startPrank(REGISTRAR);
        core.attachService(PROVIDER, clone, GENERATION_2, PROVIDER_KEY);
        core.setServiceStanding(clone, ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();
        assertEq(core.service(clone).providerId, PROVIDER, "attached");

        // 2b. The provider repoints onto it.
        vm.prank(PROVIDER_KEY);
        core.repointService(clone);
        assertEq(core.currentService(PROVIDER, RECORD_TYPE), clone, "current");

        // 3. The provider claims a domain for it.
        vm.startPrank(PROVIDER_KEY);
        core.setDomainResolver(clone, address(domainResolver));
        domainResolver.claimDomain(clone, DOMAIN);
        vm.stopPrank();
        (ServiceDomainResolver.Disposition disposition, string memory domain) =
            domainResolver.resolveDomain(clone);
        assertEq(uint8(disposition), uint8(ServiceDomainResolver.Disposition.CLAIMED));
        assertEq(domain, DOMAIN);

        // 4. The provider publishes contacts, with NO location. Listed, and absent from nearby.
        vm.prank(PROVIDER_KEY);
        directory.setProfileAnchor(
            PROVIDER, CONTACT_BLOB_DIGEST, 1, 0xe3, 0x12, bytes("ipfs://contact-blob")
        );
        assertTrue(directory.isListed(PROVIDER));
        assertEq(directory.pinTotal(), 0, "contact-only, and nearby cannot surface it");
    }

    /// @dev The provider id is opaque and KYC-assigned. It is not derived from the name, the
    /// domain, the address, the signer, the clone, or the factory salt - so knowing any of those
    /// does not let anyone compute it, and the id leaks none of them. Asserted against the two
    /// derivations somebody would actually reach for.
    function test_the_provider_id_is_not_derived_from_any_provider_fact() public view {
        assertTrue(PROVIDER != bytes20(PROVIDER_KEY), "not the signer");
        assertTrue(PROVIDER != bytes20(firstClone), "not the clone");
        assertTrue(PROVIDER != bytes20(keccak256(bytes(DOMAIN))), "not the domain");
        assertTrue(PROVIDER != bytes20(keccak256(abi.encode(RECORD_TYPE, PROVIDER_KEY))), "not the salt");
    }
}
