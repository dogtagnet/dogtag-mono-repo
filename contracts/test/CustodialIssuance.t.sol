// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test, Vm} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {DogTagSBTConsent} from "../src/DogTagSBTConsent.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";
import {LaunchStack} from "./LaunchStack.sol";

/// @notice M5 acceptance: custodial issuance produces owner-unlinkable tags that M4 actually verifies.
///
/// This suite exercises the production owner-hidden pairing: `VerificationRegistryConsent` pointing at
/// the custodial `DogTagSBTConsent`. `ConsentRegistry.t.sol` independently keeps the full real-proof
/// registry guard coverage against the same pairing.
///
/// The load-bearing property, per spec §"Issuance": **the owner's wallet never appears on-chain - not as
/// `to`, not as `msg.sender`.** `test_owner_wallet_absent_from_issuance_state_and_calldata` is the guard:
/// it sweeps every storage slot the issuance writes AND scans the raw calldata bytes, so a regression that
/// reintroduced the owner anywhere in issuance fails here rather than silently shipping.
///
/// Fixture (a,b,c,pub + the hidden `_ownerAddress`) from `node circuits/scripts/gen-consent-fixture.mjs`.
contract CustodialIssuanceTest is LaunchStack {
    using stdJson for string;

    DogTagIssuer vacc;

    address admin; // == REGISTRAR, bound in setUp once the stack exists
    address vetSigner = address(0xBEEF);
    /// @dev The neutral custodian: the ONLY address `ownerOf` can ever return on this contract.
    address constant custodian = CUSTODIAN;

    bytes20 constant VET_PROVIDER = bytes20(uint160(0x5E7));
    address constant VET_PROVIDER_KEY = address(0x5E7C0);

    bytes32 constant VACCINATION = keccak256("VACCINATION");

    uint256[2] a;
    uint256[2][2] b;
    uint256[2] c;
    uint256[7] pub;

    uint256 dogTagId;
    bytes32 purpose;
    address relayer;
    bytes32 nullifier;
    bytes32 root;
    uint256 deadline;
    /// @dev The REAL owner behind the fixture's tree - the address that must be absent everywhere on-chain.
    address ownerAddress;

    function setUp() public {
        string memory j = vm.readFile("test/consent-fixture.json");
        uint256[] memory av = j.readUintArray(".a");
        uint256[] memory b0 = j.readUintArray(".b[0]");
        uint256[] memory b1 = j.readUintArray(".b[1]");
        uint256[] memory cv = j.readUintArray(".c");
        uint256[] memory pv = j.readUintArray(".pub");
        a = [av[0], av[1]];
        b = [[b0[0], b0[1]], [b1[0], b1[1]]];
        c = [cv[0], cv[1]];
        for (uint256 i; i < 7; i++) {
            pub[i] = pv[i];
        }

        dogTagId = pub[0];
        purpose = bytes32(pub[1]);
        relayer = address(uint160(pub[2]));
        nullifier = bytes32(pub[3]);
        root = bytes32(pub[4]);
        deadline = pub[6];
        ownerAddress = j.readAddress("._ownerAddress");

        _deployLaunchStack();
        admin = REGISTRAR;

        vacc = DogTagIssuer(_onboardIssuingClone(VET_PROVIDER, VET_PROVIDER_KEY, VACCINATION, vetSigner));

        // Read the role BEFORE pranking: vm.prank only survives to the next CALL, and `sbt.ISSUER_ROLE()`
        // would otherwise consume it.
        bytes32 issuerRole = sbt.ISSUER_ROLE();
        vm.prank(admin);
        sbt.grantRole(issuerRole, vetSigner);

        _approveRelayer(purpose, relayer);
    }

    /// @dev The full M5 issuance: `issue(R)` into the clone, then mint custodially. BOTH writes are
    /// required - see `test_root_must_also_be_issued_into_a_clone`.
    function _issueCustodial() internal {
        vm.prank(vetSigner);
        vacc.issue(root);
        vm.prank(vetSigner);
        sbt.mintCustodial(dogTagId, root);
    }

    /// @dev Byte-level scan: does `hay` contain the 20 raw bytes of `needle` at ANY offset? Stronger than a
    /// word-compare - it catches the owner appearing packed, offset, or inside a nested struct.
    function _contains(bytes memory hay, address needle) internal pure returns (bool) {
        bytes20 n = bytes20(needle);
        if (hay.length < 20) return false;
        for (uint256 i; i + 20 <= hay.length; i++) {
            bool hit = true;
            for (uint256 k; k < 20; k++) {
                if (hay[i + k] != n[k]) {
                    hit = false;
                    break;
                }
            }
            if (hit) return true;
        }
        return false;
    }

    // ---- D1: the tag is custodial, and the owner is nowhere on-chain ----

    /// @notice POSITIVE CONTROL for the owner-absence guard. `_contains` returning false is the whole
    /// basis of `test_owner_wallet_absent_from_issuance_state_and_calldata`; if the scanner were broken it
    /// would return false for EVERYTHING and that test would pass vacuously forever. Prove it detects the
    /// owner when the owner really is present, in each shape issuance could leak one.
    function test_owner_absence_scanner_actually_detects_the_owner() public view {
        assertTrue(_contains(abi.encode(ownerAddress), ownerAddress), "must detect a word-aligned owner");
        assertTrue(
            _contains(abi.encodePacked(bytes4(0x12345678), ownerAddress), ownerAddress),
            "must detect an unaligned owner (e.g. behind a selector)"
        );
        assertTrue(
            _contains(
                abi.encodeWithSignature("mint(address,uint256,bytes32)", ownerAddress, dogTagId, root),
                ownerAddress
            ),
            "must detect the owner in recipient-bearing mint calldata"
        );
        assertFalse(_contains(abi.encode(custodian), ownerAddress), "must not false-positive on custodian");
    }

    function test_mint_goes_to_the_custodian_never_the_owner() public {
        _issueCustodial();
        assertEq(sbt.ownerOf(dogTagId), custodian, "tags are held by the neutral custodian");
        assertTrue(sbt.ownerOf(dogTagId) != ownerAddress, "the owner must never hold the tag");
        assertEq(sbt.profileRoot(dogTagId), root, "profileRoot(dogTagId) == R");
        assertEq(uint8(sbt.status(dogTagId)), uint8(DogTagSBTConsent.Status.Active));
    }

    // ---------------------------------------------------------------------------------------------
    // Address-keyed rights, checked where they are actually enforced: at the clone
    // ---------------------------------------------------------------------------------------------

    /// @dev An address the registrar granted nothing is refused by the REAL clone against the REAL core.
    /// `ProviderRegistry.t.sol` asserts the predicate; this asserts that the predicate is what stands
    /// between a stranger and an anchored root, which is the property a user of this system relies on.
    function test_an_address_without_the_issue_bit_is_refused_at_the_clone() public {
        address stranger = address(0xDEAD10CC);
        assertEq(core.rightsOf(stranger), 0, "granted nothing");

        vm.prank(stranger);
        vm.expectRevert(DogTagIssuer.NotIssuanceCapable.selector);
        vacc.issue(keccak256("a root a stranger must not be able to anchor"));

        // Owning the clone is still not an issuance right, and holding a DIFFERENT provider's clone
        // address is not one either — only the bit is.
        vm.prank(VET_PROVIDER_KEY);
        vm.expectRevert(DogTagIssuer.NotIssuanceCapable.selector);
        vacc.issue(keccak256("a root the clone owner must not be able to anchor"));
    }

    /// @dev THE CROSS-PROVIDER HOLE, CLOSED — and this test exists to prove the closure rather than
    /// to assert it.
    ///
    /// The authority's grant is scope-free by design: `rightsOf` takes an address and returns bits,
    /// with no service in the question, because a registrar approves an applicant before that
    /// applicant has a clone. On its own that lets a signer approved for one provider anchor on any
    /// other provider's clone — and, worse, the at-issuance pillar would then fold that signer's own
    /// `RightsSet` history and answer AUTHORIZED, producing a clean verdict on government's
    /// UNAUTHENTICATED `POST /v1/verify` for a credential attributed to a provider that never issued
    /// it.
    ///
    /// The second layer closes it, and this walks the SAME path that previously produced that clean
    /// verdict — up to the step where it now stops. The whole chain is asserted, not just the revert:
    /// nothing is anchored, so the factory's write-once index never names the root, so there is no
    /// `RootIssued` for the pillar to sequence a grant against, so no AUTHORIZED can be reached for it
    /// by any reader. A closure asserted only at the revert would leave that unstated.
    function test_a_signer_approved_for_one_provider_cannot_anchor_on_another_providers_clone()
        public
    {
        bytes20 GOV_PROVIDER = bytes20(uint160(0x60F));
        address GOV_PROVIDER_KEY = address(0x60FC0);
        address govSigner = address(0x60F516);
        DogTagIssuer travel = DogTagIssuer(
            _onboardIssuingClone(GOV_PROVIDER, GOV_PROVIDER_KEY, keccak256("TRAVEL_CLEARANCE"), govSigner)
        );

        // LAYER 1 HOLDS for the vet's signer, on the government's clone: the grant names no service,
        // so the authority itself raises no objection. That is the whole hazard, asserted rather than
        // assumed - without it the refusal below could be coming from the authority and this test
        // would prove nothing about the clone's list.
        assertTrue(core.rightsOf(vetSigner) & core.RIGHT_ISSUE() != 0, "the bit is held");
        assertTrue(core.canIssue(address(travel), vetSigner), "the AUTHORITY permits it");

        // LAYER 2 REFUSES, and it is the only thing standing there.
        assertFalse(travel.issuanceAllowed(vetSigner), "not on this clone's list");
        bytes32 crossTenant = keccak256("the vet's signer, on the government's clone");
        vm.prank(vetSigner);
        vm.expectRevert(DogTagIssuer.NotLocallyAllowed.selector);
        travel.issue(crossTenant);

        // NOTHING WAS ANCHORED, so the pillar's inputs do not exist: `rootIssuer` never names the
        // root, the clone reports it neither issued nor valid, and no `RootIssued` was emitted for a
        // grant to be sequenced against. This is the half that makes it a closure of the VERIFY path
        // and not merely of the write.
        assertEq(factory.rootIssuer(crossTenant), address(0), "the factory has no record of it");
        assertFalse(travel.isIssued(crossTenant));
        assertFalse(travel.isValid(crossTenant));
        assertEq(travel.issuedBy(crossTenant), address(0));

        // THE CONTROL. The same signer, on the clone it IS admitted to, still anchors — so the
        // refusal above is the cross-provider list check and not a broken fixture or a signer that
        // could never issue anywhere.
        bytes32 own = keccak256("the vet's signer, on the vet's own clone");
        vm.prank(vetSigner);
        vacc.issue(own);
        assertTrue(vacc.isValid(own), "its own clone still anchors");
        assertEq(factory.rootIssuer(own), address(vacc));
    }

    /// @dev The list is what closes it, and a provider may still choose to admit an outside signer.
    ///
    /// Admitting is the OWNER's decision, so this is the government provider's own transaction. It is
    /// the in-test counterpart of the mutation that removes the check: with the list written, the very
    /// same call that reverted above succeeds, which is what shows the refusal came from the list
    /// rather than from anything else in the chain.
    function test_the_clones_own_list_is_what_admits_a_signer_and_only_its_owner_writes_it() public {
        bytes20 GOV_PROVIDER = bytes20(uint160(0x60F2));
        address GOV_PROVIDER_KEY = address(0x60FC02);
        address govSigner = address(0x60F5162);
        DogTagIssuer travel = DogTagIssuer(
            _onboardIssuingClone(GOV_PROVIDER, GOV_PROVIDER_KEY, keccak256("EU_HEALTH_CERT"), govSigner)
        );

        // Neither a stranger nor the REGISTRAR may admit. The registrar is excluded deliberately: it
        // also writes the authority bit, so a registrar that could admit would hold both layers at
        // once and the cross-provider issuance would be back, reached through the registrar.
        vm.prank(vetSigner);
        vm.expectRevert(DogTagIssuer.NotOwnerOrAdmin.selector);
        travel.setIssuanceAllowed(vetSigner, true);
        vm.prank(admin);
        vm.expectRevert(DogTagIssuer.NotOwnerOrAdmin.selector);
        travel.setIssuanceAllowed(vetSigner, true);

        // The owner may, and then the previously-refused anchor lands.
        vm.prank(GOV_PROVIDER_KEY);
        travel.setIssuanceAllowed(vetSigner, true);
        bytes32 root = keccak256("admitted by the government provider itself");
        vm.prank(vetSigner);
        travel.issue(root);
        assertTrue(travel.isValid(root));

        // REMOVAL is the safety direction, so the protocol admin may do it even though it may not
        // admit — and it stops the NEXT anchor without stranding what was already issued.
        vm.prank(admin);
        travel.setIssuanceAllowed(vetSigner, false);
        vm.prank(vetSigner);
        vm.expectRevert(DogTagIssuer.NotLocallyAllowed.selector);
        travel.issue(keccak256("after removal"));
        // Forward-only: the originator can still invalidate what it anchored.
        vm.prank(vetSigner);
        travel.revoke(root);
        assertTrue(travel.isRevoked(root), "removal must not strand a root as unrevocable");
    }

    /// @dev THE CAPTAIN'S ASK, walked end to end against the REAL core: a provider can use the contract
    /// it just deployed.
    ///
    /// Every step here is a real transaction by the party that owns that step, and there is deliberately
    /// NO `setIssuanceAllowed` anywhere in this test - that is the whole assertion. Before the creation
    /// seed this journey ended in `NotLocallyAllowed` at the last line, and no product surface writes the
    /// list, so the provider had no way through.
    ///
    /// It deliberately does not use {_onboardIssuingClone}: that helper writes layer 2 for a separate
    /// signer, which would leave it unclear whether the seed or the helper made the anchor land.
    function test_a_provider_anchors_through_the_clone_it_just_deployed_without_admitting_itself()
        public
    {
        bytes20 SOLO_PROVIDER = bytes20(uint160(0x5010));
        address SOLO_KEY = address(0x5010C0);
        bytes32 SOLO_TYPE = keccak256("BOARDING");

        // The registrar's half of onboarding.
        vm.startPrank(admin);
        core.registerProvider(
            SOLO_PROVIDER, SOLO_KEY, IDENTITY_DIGEST, 1, 0xe3, 0x12, bytes("ipfs://identity")
        );
        core.setProviderStanding(SOLO_PROVIDER, ProviderRegistry.Standing.ACTIVE);
        core.setServiceCreationApproval(SOLO_PROVIDER, SOLO_TYPE, true);
        vm.stopPrank();

        // The provider deploys its own contract, and is on its list without having written anything.
        vm.prank(SOLO_KEY);
        DogTagIssuer solo = DogTagIssuer(factory.createIssuer(SOLO_PROVIDER, SOLO_TYPE, 0));
        assertEq(solo.owner(), SOLO_KEY, "the provider owns what it deployed");
        assertTrue(solo.issuanceAllowed(SOLO_KEY), "the deployer is not on the list it just created");

        // The registrar attaches it and grants the bit - the KYC half, which the seed does not touch.
        vm.startPrank(admin);
        core.attachService(SOLO_PROVIDER, address(solo), FACTORY_GENERATION, SOLO_KEY);
        core.setServiceStanding(address(solo), ProviderRegistry.Standing.ACTIVE);
        core.setRights(SOLO_KEY, core.RIGHT_ISSUE());
        vm.stopPrank();

        // The provider selects it, then anchors through it. No local admit was ever needed.
        vm.startPrank(SOLO_KEY);
        core.repointService(address(solo));
        bytes32 own = keccak256("anchored through the contract the provider just deployed");
        solo.issue(own);
        vm.stopPrank();

        assertTrue(solo.isValid(own), "a provider could not use the contract it deployed");
        assertEq(factory.rootIssuer(own), address(solo), "the factory recorded it against this clone");
        assertEq(solo.issuedBy(own), SOLO_KEY);
    }

    /// @dev The seed is not a grant. Asserted against the REAL core rather than a mock, because the claim
    /// is about `rightsOf` - a freshly deployed clone admits its creator locally and still anchors
    /// nothing, so a reader cannot conclude the seed made the registrar's approval optional.
    ///
    /// This is the counterpart of {test_the_clone_list_alone_grants_nothing_without_the_authority_bit}:
    /// that one has the owner admit a third party, this one is the entry nobody wrote.
    function test_the_seeded_creator_still_anchors_nothing_without_the_authority_bit() public {
        bytes20 UNGRANTED = bytes20(uint160(0x0067));
        address UNGRANTED_KEY = address(0x0067C0);
        bytes32 UNGRANTED_TYPE = keccak256("GROOMING");

        vm.startPrank(admin);
        core.registerProvider(
            UNGRANTED, UNGRANTED_KEY, IDENTITY_DIGEST, 1, 0xe3, 0x12, bytes("ipfs://identity")
        );
        core.setProviderStanding(UNGRANTED, ProviderRegistry.Standing.ACTIVE);
        core.setServiceCreationApproval(UNGRANTED, UNGRANTED_TYPE, true);
        vm.stopPrank();

        vm.prank(UNGRANTED_KEY);
        DogTagIssuer clone = DogTagIssuer(factory.createIssuer(UNGRANTED, UNGRANTED_TYPE, 0));

        // LAYER 2 holds - the seed wrote it. LAYER 1 does not: the registrar granted no bit.
        assertTrue(clone.issuanceAllowed(UNGRANTED_KEY), "the creator is seeded");
        assertEq(core.rightsOf(UNGRANTED_KEY) & core.RIGHT_ISSUE(), 0, "granted nothing");

        bytes32 root = keccak256("seeded locally, approved nowhere");
        vm.prank(UNGRANTED_KEY);
        vm.expectRevert(DogTagIssuer.NotIssuanceCapable.selector);
        clone.issue(root);

        assertEq(factory.rootIssuer(root), address(0), "the factory has no record of it");
        assertFalse(clone.isIssued(root));
    }

    /// @dev Neither layer is redundant, stated as its own case so a later reader cannot conclude the
    /// authority bit became decorative once the clone kept a list.
    function test_the_clone_list_alone_grants_nothing_without_the_authority_bit() public {
        address unapproved = address(0xB17);
        // The clone's owner admits it - layer 2 holds.
        vm.prank(VET_PROVIDER_KEY);
        vacc.setIssuanceAllowed(unapproved, true);
        assertTrue(vacc.issuanceAllowed(unapproved));

        // The registrar never granted it, so layer 1 refuses and the anchor does not happen.
        assertEq(core.rightsOf(unapproved) & core.RIGHT_ISSUE(), 0);
        vm.prank(unapproved);
        vm.expectRevert(DogTagIssuer.NotIssuanceCapable.selector);
        vacc.issue(keccak256("admitted locally, approved nowhere"));
    }

    /// @notice M5 app-side: the root the OWNER'S APP builds locally is exactly what the contract seals as
    /// `profileRoot`. Spec §"Issuance" steps 2-3: *the owner's app builds the tree locally, computes `R`;
    /// the issuer sets `profileRoot(dogTagId) = R`*.
    ///
    /// `device-profile-root.json` is emitted by the REAL device builder
    /// (`dogtag-standard-rs profile_tree::build_profile_tree`, run over a fixed demo wallet seed via
    /// `cargo run -p dogtag-standard-rs --bin gen-device-profile-root`), NOT hand-written here. The Rust
    /// gate keeps it honest from the other side: `profile_tree_parity.rs` fails if the committed file
    /// drifts from a fresh build, and separately proves that the builder's primitives reproduce an `R`
    /// the M2 circuit proved and this suite verifies on-chain.
    ///
    /// These are two separate legs: the primitives reproduce the circuit-verified fixture root, while
    /// `build_profile_tree` produces the distinct demo root this test stores as `profileRoot`. The demo root
    /// is not itself circuit-proven; that end-to-end prover path belongs to M7. The assertion is genuinely
    /// device-derived rather than a re-assertion of `setUp`'s because the device root DIFFERS from the
    /// fixture's. The two fixtures also now carry DIFFERENT dogTagIds: `consent-fixture.json` binds the
    /// canonical field (`field_of_value(424242)` = 19282...896), while `device-profile-root.json` is still
    /// keyed by the raw handle 424242 (regenerating it under the canonical field is P6, out of scope here).
    /// So this test mints its own `deviceTagId` and does not collide with `setUp`'s fixture id; if a future
    /// change unified the two ids AND both got minted in one test, `mintCustodial`'s write-once guard would
    /// force `device-profile-root.json` to be regenerated under a distinct id.
    function test_device_built_root_is_what_the_contract_stores_as_profileRoot() public {
        string memory j = vm.readFile("test/device-profile-root.json");
        bytes32 deviceRoot = bytes32(j.readUint(".R"));
        uint256 deviceTagId = vm.parseUint(j.readString(".dogTagId"));
        address deviceOwner = j.readAddress("._ownerAddress");

        assertTrue(deviceRoot != bytes32(0), "device root must be non-zero");

        vm.prank(vetSigner);
        vacc.issue(deviceRoot);
        vm.prank(vetSigner);
        sbt.mintCustodial(deviceTagId, deviceRoot);

        assertEq(sbt.profileRoot(deviceTagId), deviceRoot, "profileRoot == the app-built R");
        // The device-side move must not reintroduce the linkability M5 removes.
        assertEq(sbt.ownerOf(deviceTagId), custodian, "device-built tag is still custodial");
        assertTrue(sbt.ownerOf(deviceTagId) != deviceOwner, "the owner must never hold the tag");
    }

    /// @notice THE load-bearing test (spec §"Issuance" step 4: *the owner's wallet never appears on-chain -
    /// not as `to`, not as `msg.sender`*). Three independent checks, because each catches a different
    /// regression:
    ///   1. CALLDATA - a byte scan of the exact calldata both issuance txs carry.
    ///   2. STATE - every storage slot the issuance WRITES, on the SBT and on the issuing clone.
    ///   3. LOGS + `msg.sender` - the owner is not an emitter, a topic, or event data.
    function test_owner_wallet_absent_from_issuance_state_and_calldata() public {
        bytes memory mintCalldata = abi.encodeCall(DogTagSBTConsent.mintCustodial, (dogTagId, root));
        bytes memory issueCalldata = abi.encodeCall(DogTagIssuer.issue, (root));
        assertFalse(_contains(mintCalldata, ownerAddress), "owner leaked into mint calldata");
        assertFalse(_contains(issueCalldata, ownerAddress), "owner leaked into issue calldata");

        vm.recordLogs();
        vm.record();
        _issueCustodial();

        // 2. STATE: sweep every slot written on both contracts touched by issuance.
        _assertOwnerAbsentFromWrites(address(sbt), "SBT");
        _assertOwnerAbsentFromWrites(address(vacc), "issuer clone");

        // The owner is not the minter either: `msg.sender` throughout is the vet signer.
        assertEq(sbt.issuerOf(dogTagId), vetSigner, "issuerOf is the vet, not the owner");
        assertTrue(sbt.issuerOf(dogTagId) != ownerAddress, "owner leaked into issuerOf");
        assertEq(vacc.issuedBy(root), vetSigner, "issuedBy is the vet, not the owner");

        // 3. LOGS: no emitter, topic or data word is the owner.
        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertGt(logs.length, 0, "issuance must emit");
        bytes32 ownerWord = bytes32(uint256(uint160(ownerAddress)));
        for (uint256 i; i < logs.length; i++) {
            assertTrue(logs[i].emitter != ownerAddress, "owner emitted a log");
            for (uint256 t; t < logs[i].topics.length; t++) {
                assertTrue(logs[i].topics[t] != ownerWord, "owner leaked into a log topic");
            }
            assertFalse(_contains(logs[i].data, ownerAddress), "owner leaked into log data");
        }
    }

    /// @dev Reads back every slot `target` wrote during the recorded window and asserts none holds the
    /// owner. Scoped to WRITES because that is precisely "the on-chain state issuance created". Uses the
    /// same byte scan as the calldata leg rather than a word compare, so an owner packed alongside another
    /// field in one slot (e.g. `address` + `uint96`) cannot slip through.
    function _assertOwnerAbsentFromWrites(address target, string memory label) internal {
        (, bytes32[] memory writes) = vm.accesses(target);
        for (uint256 i; i < writes.length; i++) {
            assertFalse(
                _contains(abi.encode(vm.load(target, writes[i])), ownerAddress),
                string.concat("owner leaked into ", label, " storage")
            );
        }
    }

    /// @notice The owner's wallet is not even expressible in the mint: `mintCustodial` takes no `to`. A
    /// future change re-adding one would fail to compile against this test rather than quietly ship.
    function test_mint_has_no_recipient_parameter() public view {
        // Selector is (uint256,bytes32) - no recipient address.
        assertEq(
            DogTagSBTConsent.mintCustodial.selector,
            bytes4(keccak256("mintCustodial(uint256,bytes32)")),
            "mintCustodial must never take a recipient"
        );
        assertEq(sbt.custodian(), custodian, "custodian is fixed at construction");
    }

    // ---- write-once profileRoot: the setProfileRoot hijack, structurally closed ----

    /// @notice The M4 hijack: with `ownerOf`/`keyOf` identity checks gone, `R == profileRoot` is the SOLE
    /// tag<->owner binding, so anyone able to repoint `profileRoot` could forge a `Verified` for a pet they
    /// do not own. This SBT has NO setter at all - not a re-gated one. The call must not even dispatch.
    function test_profileRoot_has_no_setter_at_all() public {
        _issueCustodial();
        (bool ok,) = address(sbt)
            .call(
                abi.encodeWithSignature(
                    "setProfileRoot(uint256,bytes32)", dogTagId, keccak256("attacker root")
                )
            );
        assertFalse(ok, "setProfileRoot must not exist on the owner-hidden SBT");
        assertEq(sbt.profileRoot(dogTagId), root, "root is write-once");
    }

    /// @notice AUTHORITY_ROLE was one of the mutable-root hijack's two vectors. Here it still governs
    /// status/lifecycle but has NO power over the root - there is nothing to overwrite.
    function test_authority_role_cannot_touch_the_root() public {
        _issueCustodial();
        bytes32 authorityRole = sbt.AUTHORITY_ROLE();
        vm.prank(admin);
        sbt.grantRole(authorityRole, admin);

        (bool ok,) = address(sbt)
            .call(
                abi.encodeWithSignature(
                    "setProfileRoot(uint256,bytes32)", dogTagId, keccak256("attacker root")
                )
            );
        assertFalse(ok, "an AUTHORITY holder must have no root-write path");
        assertEq(sbt.profileRoot(dogTagId), root, "victim tag still points at its real root");

        // ...and the tag still verifies, i.e. the hijack did not merely fail - it never happened.
        vm.prank(relayer);
        vr.recordVerificationZK(a, b, c, pub);
    }

    /// @notice A tag is never created rootless - there is no later call that could seal it.
    function test_mintCustodial_rejects_a_zero_root() public {
        vm.prank(vetSigner);
        vm.expectRevert(DogTagSBTConsent.BadRoot.selector);
        sbt.mintCustodial(dogTagId, bytes32(0));
    }

    function test_mintCustodial_is_issuer_gated() public {
        vm.prank(address(0xBAD));
        vm.expectRevert();
        sbt.mintCustodial(dogTagId, root);
    }

    function test_duplicate_dogTagId_reverts() public {
        _issueCustodial();
        vm.prank(vetSigner);
        vm.expectRevert();
        sbt.mintCustodial(dogTagId, root);
    }

    /// @notice The seal must survive a BURN, not just a duplicate mint. ERC-721 `_burn` leaves no tombstone
    /// (`_owners[id]` goes back to zero) and `_mint` only rejects a re-mint when the previous owner was
    /// non-zero, so without an explicit `profileRoot` guard an ISSUER holder could re-mint a burned id and
    /// overwrite its sealed root - reopening the very hijack this SBT exists to close, and un-erasing a
    /// GDPR-erased tag. `test_duplicate_dogTagId_reverts` never burns first, so it does not cover this.
    function test_burned_dogTagId_can_never_be_reminted() public {
        _issueCustodial();
        vm.prank(admin);
        sbt.burn(dogTagId);

        bytes32 attackerRoot = keccak256("attacker root");
        vm.prank(vetSigner);
        vm.expectRevert(DogTagSBTConsent.BadRoot.selector);
        sbt.mintCustodial(dogTagId, attackerRoot);

        assertEq(sbt.profileRoot(dogTagId), root, "the sealed root survives a burn/re-mint attempt");
    }

    // ---- the flow must actually produce tags M4 verifies ----

    /// @notice M5's whole purpose: a custodially-issued tag verifies end-to-end against the M4 registry and
    /// emits an owner-blind `Verified`. Uses a REAL Groth16 proof against the production M3 VK.
    function test_custodial_issuance_verifies_end_to_end() public {
        _issueCustodial();

        vm.expectEmit(true, true, true, true);
        emit VerificationRegistryConsent.Verified(
            dogTagId, relayer, purpose, nullifier, deadline, block.timestamp
        );
        vm.prank(relayer);
        vr.recordVerificationZK(a, b, c, pub);
        assertTrue(vr.consumed(nullifier), "nullifier consumed");
    }

    /// @notice Guards the M4 handoff note: issuance MUST `issue(R)` into a clone as well as set
    /// `profileRoot`, or the registry's revocation lookup (`rootIssuer[R]` -> `isValid`) reverts on EVERY
    /// verify. Minting alone yields a tag that can never be used - the exact trap the spec's step list
    /// (which names only `profileRoot`) invites.
    function test_root_must_also_be_issued_into_a_clone() public {
        vm.prank(vetSigner);
        sbt.mintCustodial(dogTagId, root); // profileRoot set, but issue(R) skipped

        vm.prank(relayer);
        vm.expectRevert("unknown root");
        vr.recordVerificationZK(a, b, c, pub);
    }

    /// @notice Revocation still works: revoke the root and the tag stops verifying.
    function test_revoked_root_stops_verifying() public {
        _issueCustodial();
        vm.prank(vetSigner);
        vacc.revoke(root);

        vm.prank(relayer);
        vm.expectRevert("cred !valid");
        vr.recordVerificationZK(a, b, c, pub);
    }

    // ---- soulbound, absolutely (D3: no rebind - recovery is a fresh issuance) ----

    function test_tag_is_soulbound_with_no_recovery_bypass() public {
        _issueCustodial();
        vm.prank(custodian);
        vm.expectRevert(DogTagSBTConsent.Soulbound.selector);
        sbt.transferFrom(custodian, address(0xD00D), dogTagId);
    }

    /// @notice D3/M8: the retired `recover(...)` named the new owner ON-CHAIN. Recovery is a fresh
    /// custodial issuance, so the function must not exist here.
    function test_no_recover_surface() public {
        _issueCustodial();
        (bool ok,) = address(sbt)
            .call(
                abi.encodeWithSignature(
                    "recover(uint256,address,uint256,uint256,bytes,bytes)",
                    dogTagId,
                    address(0xD00D),
                    uint256(0),
                    block.timestamp + 1,
                    bytes(""),
                    bytes("")
                )
            );
        assertFalse(ok, "recover must not exist on the owner-hidden SBT");
    }

    /// @notice Erasure still fails closed (the registry's `ownerOf` existence gate), and `profileRoot`
    /// deliberately survives the burn - which is why that gate is load-bearing rather than redundant.
    function test_burned_tag_cannot_verify() public {
        _issueCustodial();
        vm.prank(admin);
        sbt.burn(dogTagId);

        assertEq(sbt.profileRoot(dogTagId), root, "burn does not clear profileRoot");
        vm.prank(relayer);
        vm.expectRevert(); // ERC721NonexistentToken
        vr.recordVerificationZK(a, b, c, pub);
    }
}
