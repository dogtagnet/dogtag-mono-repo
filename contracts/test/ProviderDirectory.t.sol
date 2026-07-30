// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {ProviderDirectory} from "../src/ProviderDirectory.sol";

/// @dev Exposes the one internal arm whose only other route is 65,535 real publications.
contract ProviderDirectoryHarness is ProviderDirectory {
    constructor(ProviderRegistry core_) ProviderDirectory(core_) {}

    function issueLocationNumber(uint16 current) external pure returns (uint16) {
        return _issueLocationNumber(current);
    }
}

contract ProviderDirectoryTest is Test {
    ProviderRegistry internal core;
    ProviderDirectoryHarness internal directory;

    address internal constant REGISTRAR = address(0xA11CE);
    address internal constant PINNED_CONTROLLER = address(0xB0B);
    address internal constant CONTACT_ONLY_CONTROLLER = address(0xCA11);
    address internal constant THIRD_CONTROLLER = address(0xD00D);
    address internal constant DELEGATE = address(0xDE1E);
    address internal constant STRANGER = address(0xBAD);

    bytes20 internal constant PINNED = bytes20(uint160(0x1111));
    bytes20 internal constant CONTACT_ONLY = bytes20(uint160(0x2222));
    bytes20 internal constant THIRD = bytes20(uint160(0x3333));

    uint8 internal constant KIND_VET = 1;
    uint8 internal constant KIND_GROOMER = 2;

    bytes32 internal constant ANCHOR_DIGEST = keccak256("contact blob v1");

    // Singapore and Sydney, degrees x 1e6. Sydney is deliberately southern and Singapore northern so
    // a sign error in either coordinate shows up as a wrong value rather than as a wrong magnitude.
    int32 internal constant SG_LAT = 1_352_100;
    int32 internal constant SG_LNG = 103_819_800;
    int32 internal constant SYD_LAT = -33_868_800;
    int32 internal constant SYD_LNG = 151_209_300;

    function setUp() public {
        core = new ProviderRegistry(REGISTRAR);
        directory = new ProviderDirectoryHarness(core);

        vm.startPrank(REGISTRAR);
        _register(PINNED, PINNED_CONTROLLER);
        _register(CONTACT_ONLY, CONTACT_ONLY_CONTROLLER);
        _register(THIRD, THIRD_CONTROLLER);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, address(directory), true);
        vm.stopPrank();

        _select(PINNED, PINNED_CONTROLLER);
        _select(CONTACT_ONLY, CONTACT_ONLY_CONTROLLER);
        _select(THIRD, THIRD_CONTROLLER);
    }

    function _register(bytes20 providerId, address controller) internal {
        core.registerProvider(providerId, controller, keccak256(abi.encode(providerId)), 1, 1, 1, "");
        core.setProviderStanding(providerId, ProviderRegistry.Standing.ACTIVE);
    }

    function _select(bytes20 providerId, address controller) internal {
        vm.prank(controller);
        core.setDirectoryResolver(providerId, address(directory));
    }

    function _publishAnchor(bytes20 providerId, address controller, bytes32 digest) internal {
        vm.prank(controller);
        directory.setProfileAnchor(providerId, digest, 1, 0x55, 0x12, hex"1220aabb");
    }

    // =============================================================================================
    // The headline: enumeration is not the pin scan
    // =============================================================================================

    /// A provider that publishes only contacts must be LISTED, and must not put a pin on any map.
    /// Deriving the listing from the pin set — which is how both source designs built it — makes this
    /// provider invisible, turning "the provider is listed" into the much weaker "we will not show a
    /// fake pin".
    function test_a_contact_only_provider_is_enumerated_and_contributes_nothing_to_the_pin_scan() public {
        vm.prank(PINNED_CONTROLLER);
        directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        uint256 pinTotalBefore = directory.pinTotal();

        _publishAnchor(CONTACT_ONLY, CONTACT_ONLY_CONTROLLER, ANCHOR_DIGEST);

        // Enumerated.
        assertTrue(directory.isListed(CONTACT_ONLY), "contact-only provider is not listed");
        assertEq(directory.listingTotal(), 2, "listing did not grow");

        // And present in the paged enumeration, with its anchor and no pins.
        (bytes32[] memory words,,,,) = directory.listingPage(0, 10);
        (bytes20 id,, uint16 pins,, bool anchorPresent,, bytes32 digest) =
            directory.unpackListing(words[2], words[3]);
        assertEq(id, CONTACT_ONLY, "second listing record is not the contact-only provider");
        assertEq(pins, 0, "contact-only provider reports a pin");
        assertTrue(anchorPresent, "contact-only provider reports no anchor");
        assertEq(digest, ANCHOR_DIGEST, "anchor digest not carried");

        // Contributing nothing to the pin scan.
        assertEq(directory.pinTotal(), pinTotalBefore, "the pin scan grew for a location-less provider");
        (bytes32[] memory pinWords,, uint256 total,,) = directory.pinPage(0, 10);
        assertEq(total, 1, "pin scan total moved");
        assertEq(pinWords.length, 1, "pin scan returned a record for a location-less provider");
        assertEq(
            directory.unpackPin(pinWords[0]).providerId, PINNED, "the only pin is not the pinned provider's"
        );
    }

    /// The inverse half: a provider with a pin and no anchor is also fully first class, so neither
    /// content kind is a precondition for the other.
    function test_a_pin_only_provider_is_enumerated_with_no_anchor() public {
        vm.prank(PINNED_CONTROLLER);
        directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);

        (bytes32[] memory words,,,,) = directory.listingPage(0, 10);
        (bytes20 id, uint64 revision, uint16 pins,, bool anchorPresent,, bytes32 digest) =
            directory.unpackListing(words[0], words[1]);
        assertEq(id, PINNED);
        assertEq(pins, 1, "pin not counted on the listing");
        assertFalse(anchorPresent, "claims an anchor that was never published");
        assertEq(digest, bytes32(0), "non-zero digest with no anchor");
        assertEq(revision, 0, "revision advanced without an anchor write");
    }

    /// Withdrawing every published thing does not unlist a provider. Enumeration is append-only, so a
    /// cursor a consumer is part way through stays valid, and the record tells the truth by reporting
    /// no anchor and no pins rather than by vanishing.
    function test_withdrawing_all_content_leaves_the_provider_listed_and_honest() public {
        _publishAnchor(CONTACT_ONLY, CONTACT_ONLY_CONTROLLER, ANCHOR_DIGEST);
        vm.startPrank(CONTACT_ONLY_CONTROLLER);
        uint16 locationNo = directory.publishPin(CONTACT_ONLY, SYD_LAT, SYD_LNG, KIND_GROOMER, true);
        directory.removePin(CONTACT_ONLY, locationNo);
        directory.clearProfileAnchor(CONTACT_ONLY);
        vm.stopPrank();

        assertTrue(directory.isListed(CONTACT_ONLY), "provider unlisted by withdrawing content");
        assertEq(directory.listingTotal(), 1);

        (bytes32[] memory words,,,,) = directory.listingPage(0, 10);
        (, uint64 revision, uint16 pins,, bool anchorPresent,, bytes32 digest) =
            directory.unpackListing(words[0], words[1]);
        assertEq(pins, 0);
        assertFalse(anchorPresent);
        assertEq(digest, bytes32(0));
        // A withdrawn anchor is distinguishable from one never published: absent at revision 0 has
        // never been set, absent at a higher revision was taken down.
        assertEq(revision, 2, "revision did not record the publish and the withdrawal");
    }

    function test_publishing_twice_lists_the_provider_once() public {
        vm.startPrank(PINNED_CONTROLLER);
        directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        directory.publishPin(PINNED, SYD_LAT, SYD_LNG, KIND_VET, true);
        vm.stopPrank();
        _publishAnchor(PINNED, PINNED_CONTROLLER, ANCHOR_DIGEST);

        assertEq(directory.listingTotal(), 1, "one provider listed more than once");
        assertEq(directory.pinTotal(), 2);
        assertEq(directory.pinCount(PINNED), 2);
    }

    // =============================================================================================
    // The packed pin word
    // =============================================================================================

    function test_the_pin_word_round_trips_every_field_including_negative_coordinates() public view {
        ProviderDirectory.Pin memory original = ProviderDirectory.Pin({
            providerId: PINNED,
            lat: SYD_LAT,
            lng: SYD_LNG,
            locationNo: 40_000,
            kind: KIND_GROOMER,
            // active (bit 0) plus MATCHED_LICENSING_REGISTER (value 1) in the provenance bits 1-2.
            flags: 0x01 | (uint8(ProviderDirectory.AddressProvenance.MATCHED_LICENSING_REGISTER) << 1)
        });

        ProviderDirectory.Pin memory back = directory.unpackPin(directory.packPin(original));

        assertEq(back.providerId, original.providerId, "providerId");
        assertEq(back.lat, original.lat, "lat");
        assertEq(back.lng, original.lng, "lng");
        assertEq(back.locationNo, original.locationNo, "locationNo");
        assertEq(back.kind, original.kind, "kind");
        assertEq(back.flags, original.flags, "flags");
        assertTrue(directory.pinIsActive(directory.packPin(original)));
        assertEq(
            uint8(directory.pinProvenance(directory.packPin(original))),
            uint8(ProviderDirectory.AddressProvenance.MATCHED_LICENSING_REGISTER)
        );
    }

    /// Negative coordinates are the classic packing bug: a two's-complement `int32` read back through
    /// the wrong width silently becomes a large positive number, which is a plausible-looking
    /// coordinate on the other side of the planet rather than an error.
    function test_the_coordinate_extremes_survive_the_round_trip_in_both_signs() public view {
        int32[4] memory lats = [int32(90_000_000), int32(-90_000_000), int32(0), int32(-1)];
        int32[4] memory lngs = [int32(180_000_000), int32(-180_000_000), int32(0), int32(-1)];

        for (uint256 i = 0; i < 4; i++) {
            ProviderDirectory.Pin memory back = directory.unpackPin(
                directory.packPin(
                    ProviderDirectory.Pin({
                        providerId: PINNED,
                        lat: lats[i],
                        lng: lngs[i],
                        locationNo: uint16(i),
                        kind: KIND_VET,
                        flags: 0
                    })
                )
            );
            assertEq(back.lat, lats[i], "lat did not survive");
            assertEq(back.lng, lngs[i], "lng did not survive");
        }
    }

    /// The single most load-bearing structural fact: one pin is one storage slot, so a scan is one
    /// cold SLOAD per record. If the record ever spills to two slots it halves what a single
    /// `eth_call` can return. 20 + 4 + 4 + 2 + 1 + 1 = 32.
    function test_the_widest_representable_pin_fills_the_word_with_no_spare_bits() public view {
        // Provenance 3 is not a value, so the widest VALID flags byte is active + POSTAL_CONFIRMED.
        uint8 widestFlags = 0x01 | (uint8(ProviderDirectory.AddressProvenance.POSTAL_CONFIRMED) << 1);
        bytes32 word = directory.packPin(
            ProviderDirectory.Pin({
                providerId: bytes20(type(uint160).max),
                lat: -1,
                lng: -1,
                locationNo: type(uint16).max,
                kind: type(uint8).max,
                flags: widestFlags
            })
        );
        // Every byte outside the flags byte is saturated, which is only possible if the fields tile
        // the word exactly rather than leaving a gap the codec silently ignores.
        assertEq(
            word,
            bytes32((type(uint256).max << 8) | uint256(widestFlags)),
            "the packed fields do not tile the 32-byte word"
        );
    }

    function test_a_coordinate_outside_the_earth_is_refused() public {
        vm.expectRevert(
            abi.encodeWithSelector(ProviderDirectory.LatitudeOutOfRange.selector, int32(90_000_001))
        );
        directory.requireCoordinates(90_000_001, 0);

        vm.expectRevert(
            abi.encodeWithSelector(ProviderDirectory.LongitudeOutOfRange.selector, int32(-180_000_001))
        );
        directory.requireCoordinates(0, -180_000_001);

        vm.prank(PINNED_CONTROLLER);
        vm.expectRevert(
            abi.encodeWithSelector(ProviderDirectory.LatitudeOutOfRange.selector, int32(-90_000_001))
        );
        directory.publishPin(PINNED, -90_000_001, 0, KIND_VET, true);
    }

    /// `0,0` is a real place off the coast of Ghana and must stay publishable. Absence of a location
    /// is represented by having no pin, never by a zero pair — the exact confusion that made a blank
    /// admin location render as a pin in the Gulf of Guinea.
    function test_zero_zero_is_a_real_coordinate_and_absence_is_the_lack_of_a_pin() public {
        vm.prank(PINNED_CONTROLLER);
        uint16 locationNo = directory.publishPin(PINNED, 0, 0, KIND_VET, true);

        assertTrue(directory.hasPin(PINNED, locationNo), "a 0,0 pin was not stored");
        assertEq(directory.pinTotal(), 1, "a 0,0 pin is missing from the scan");
        ProviderDirectory.Pin memory stored = directory.pin(PINNED, locationNo);
        assertEq(stored.lat, 0);
        assertEq(stored.lng, 0);

        // The provider next door published nothing, and that is what absence looks like.
        assertFalse(directory.hasPin(CONTACT_ONLY, 0), "a provider with no pin appears to have one");
    }

    function test_a_reserved_flag_bit_is_refused() public view {
        // Reserved bits are refused so they stay available for a future field rather than being
        // occupied by whatever a caller happened to pass.
        ProviderDirectory.Pin memory value =
            ProviderDirectory.Pin({providerId: PINNED, lat: 0, lng: 0, locationNo: 0, kind: 0, flags: 0x08});
        (bool ok, bytes memory reason) =
            address(directory).staticcall(abi.encodeCall(ProviderDirectory.packPin, (value)));
        assertFalse(ok, "a reserved flag bit was accepted");
        assertEq(reason, abi.encodeWithSelector(ProviderDirectory.ReservedFlagBitsSet.selector, uint8(0x08)));
    }

    function test_an_unknown_provenance_is_refused() public view {
        ProviderDirectory.Pin memory value = ProviderDirectory.Pin({
            providerId: PINNED,
            lat: 0,
            lng: 0,
            locationNo: 0,
            kind: 0,
            flags: 0x06 // provenance bits set to 3, which is not a value
        });
        (bool ok, bytes memory reason) =
            address(directory).staticcall(abi.encodeCall(ProviderDirectory.packPin, (value)));
        assertFalse(ok, "provenance 3 was accepted");
        assertEq(reason, abi.encodeWithSelector(ProviderDirectory.UnknownProvenance.selector, uint8(3)));
    }

    /// The public decoders are the documented consumer surface, and a consumer may reach them with a
    /// word this contract never produced — one read back from an untrusted mirror. Such a word must be
    /// refused by name, not by the enum cast's opaque panic, which says nothing about which field of
    /// the word was wrong.
    function test_a_decoder_refuses_a_word_no_page_could_have_produced_by_name() public view {
        // Only the flags byte matters here: provenance bits 1-2 set to 3, which is not a value.
        (bool ok, bytes memory reason) = address(directory).staticcall(
            abi.encodeCall(ProviderDirectory.pinProvenance, (bytes32(uint256(0x06))))
        );
        assertFalse(ok, "provenance 3 decoded as a value");
        assertEq(reason, abi.encodeWithSelector(ProviderDirectory.UnknownProvenance.selector, uint8(3)));

        // Standing sits at byte [30] of word 0; 5 is one past RETIRED, the core's last member.
        (ok, reason) = address(directory).staticcall(
            abi.encodeCall(ProviderDirectory.unpackListing, (bytes32(uint256(5) << 8), bytes32(0)))
        );
        assertFalse(ok, "a standing the core does not define decoded as a value");
        assertEq(reason, abi.encodeWithSelector(ProviderDirectory.UnknownStanding.selector, uint8(5)));
    }

    /// Kind is an opaque caller-selected code with no on-chain allowlist, mirroring the directory
    /// service's stated kind policy. Zero means NOT STATED and is never inferred into a real kind.
    function test_kind_is_opaque_and_zero_means_not_stated() public {
        vm.prank(PINNED_CONTROLLER);
        uint16 locationNo = directory.publishPin(PINNED, SG_LAT, SG_LNG, 0, true);
        assertEq(directory.pin(PINNED, locationNo).kind, 0, "a zero kind was replaced with a guess");

        vm.prank(PINNED_CONTROLLER);
        uint16 exotic = directory.publishPin(PINNED, SG_LAT, SG_LNG, 200, true);
        assertEq(directory.pin(PINNED, exotic).kind, 200, "an unrecognised kind was rejected or altered");
    }

    // =============================================================================================
    // Stable (providerId, locationNo)
    // =============================================================================================

    function test_a_location_number_is_never_reissued_after_removal() public {
        vm.startPrank(PINNED_CONTROLLER);
        uint16 first = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        directory.removePin(PINNED, first);
        uint16 second = directory.publishPin(PINNED, SYD_LAT, SYD_LNG, KIND_GROOMER, true);
        vm.stopPrank();

        assertEq(first, 0);
        assertEq(second, 1, "a withdrawn location number was reissued to a different place");
        assertFalse(directory.hasPin(PINNED, first), "the removed location is still resolvable");
    }

    function test_location_numbers_are_per_provider() public {
        vm.prank(PINNED_CONTROLLER);
        uint16 a = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        vm.prank(CONTACT_ONLY_CONTROLLER);
        uint16 b = directory.publishPin(CONTACT_ONLY, SYD_LAT, SYD_LNG, KIND_GROOMER, true);

        assertEq(a, 0);
        assertEq(b, 0, "location numbers are shared across providers");
        assertEq(directory.pin(PINNED, 0).providerId, PINNED);
        assertEq(directory.pin(CONTACT_ONLY, 0).providerId, CONTACT_ONLY);
    }

    function test_an_update_keeps_the_location_number() public {
        vm.startPrank(PINNED_CONTROLLER);
        uint16 locationNo = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        directory.updatePin(PINNED, locationNo, SYD_LAT, SYD_LNG, KIND_GROOMER, false);
        vm.stopPrank();

        ProviderDirectory.Pin memory stored = directory.pin(PINNED, locationNo);
        assertEq(stored.locationNo, locationNo, "the location number moved under an update");
        assertEq(stored.lat, SYD_LAT);
        assertEq(stored.kind, KIND_GROOMER);
        assertFalse(directory.pinIsActive(directory.pinWord(PINNED, locationNo)));
        assertEq(directory.nextLocationNumber(PINNED), 1, "an update consumed a location number");
    }

    function test_exhausting_the_location_numbers_reverts_rather_than_wrapping() public {
        assertEq(directory.issueLocationNumber(type(uint16).max - 1), type(uint16).max - 1);
        vm.expectRevert(ProviderDirectory.LocationNumbersExhausted.selector);
        directory.issueLocationNumber(type(uint16).max);
    }

    function test_an_unknown_location_is_refused_rather_than_answered() public {
        vm.expectRevert(ProviderDirectory.UnknownLocation.selector);
        directory.pinWord(PINNED, 7);

        vm.prank(PINNED_CONTROLLER);
        vm.expectRevert(ProviderDirectory.UnknownLocation.selector);
        directory.updatePin(PINNED, 7, SG_LAT, SG_LNG, KIND_VET, true);
    }

    // =============================================================================================
    // The scan stays consistent under swap-and-pop
    // =============================================================================================

    /// Removal compacts the scan by moving the tail record into the hole. The moved word carries its
    /// own `(providerId, locationNo)`, so its index is repaired from the word itself — this asserts
    /// that repair rather than the happy path, because a stale index silently resolves one provider's
    /// location number to a different provider's place.
    function test_removing_a_pin_repairs_the_moved_records_index() public {
        vm.startPrank(PINNED_CONTROLLER);
        uint16 a = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        uint16 b = directory.publishPin(PINNED, SYD_LAT, SYD_LNG, KIND_GROOMER, true);
        vm.stopPrank();
        vm.prank(CONTACT_ONLY_CONTROLLER);
        uint16 c = directory.publishPin(CONTACT_ONLY, 0, 0, KIND_VET, true);

        // Remove the middle record so the last one has to move into its place.
        vm.prank(PINNED_CONTROLLER);
        directory.removePin(PINNED, b);

        assertEq(directory.pinTotal(), 2);
        _assertScanIsSelfConsistent();

        // The moved record still resolves to itself, by its own identity.
        ProviderDirectory.Pin memory moved = directory.pin(CONTACT_ONLY, c);
        assertEq(moved.providerId, CONTACT_ONLY, "the moved pin resolves to the wrong provider");
        assertEq(moved.locationNo, c);
        assertTrue(directory.hasPin(PINNED, a), "an untouched pin was lost");
    }

    function test_removing_the_last_scan_record_needs_no_move() public {
        vm.startPrank(PINNED_CONTROLLER);
        uint16 a = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        uint16 b = directory.publishPin(PINNED, SYD_LAT, SYD_LNG, KIND_GROOMER, true);
        directory.removePin(PINNED, b);
        vm.stopPrank();

        assertEq(directory.pinTotal(), 1);
        _assertScanIsSelfConsistent();
        assertTrue(directory.hasPin(PINNED, a));
    }

    function test_the_scan_stays_consistent_across_a_mixed_sequence() public {
        vm.startPrank(PINNED_CONTROLLER);
        uint16 p0 = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        uint16 p1 = directory.publishPin(PINNED, SYD_LAT, SYD_LNG, KIND_VET, false);
        vm.stopPrank();
        vm.startPrank(CONTACT_ONLY_CONTROLLER);
        uint16 c0 = directory.publishPin(CONTACT_ONLY, 0, 0, KIND_GROOMER, true);
        vm.stopPrank();
        vm.prank(THIRD_CONTROLLER);
        uint16 t0 = directory.publishPin(THIRD, -1, 1, KIND_VET, true);

        vm.prank(PINNED_CONTROLLER);
        directory.removePin(PINNED, p0);
        _assertScanIsSelfConsistent();

        vm.prank(CONTACT_ONLY_CONTROLLER);
        directory.updatePin(CONTACT_ONLY, c0, SG_LAT, SG_LNG, KIND_GROOMER, true);
        _assertScanIsSelfConsistent();

        vm.prank(THIRD_CONTROLLER);
        directory.removePin(THIRD, t0);
        _assertScanIsSelfConsistent();

        assertEq(directory.pinTotal(), 2);
        assertTrue(directory.hasPin(PINNED, p1));
        assertTrue(directory.hasPin(CONTACT_ONLY, c0));
        assertEq(directory.pinCount(PINNED), 1);
        assertEq(directory.pinCount(THIRD), 0);
    }

    /// Every scan record must be reachable by its own identity, and that lookup must land back on the
    /// same word. Both halves are needed: checking only that the count matches would pass with every
    /// index pointing at record zero.
    function _assertScanIsSelfConsistent() internal view {
        (bytes32[] memory words,, uint256 total,,) = directory.pinPage(0, 1000);
        assertEq(words.length, total, "page length disagrees with the reported total");
        for (uint256 i = 0; i < words.length; i++) {
            ProviderDirectory.Pin memory decoded = directory.unpackPin(words[i]);
            assertTrue(
                directory.hasPin(decoded.providerId, decoded.locationNo),
                "a scanned pin is not reachable by its own identity"
            );
            assertEq(
                directory.pinWord(decoded.providerId, decoded.locationNo),
                words[i],
                "a pin's index points at a different record"
            );
        }
    }

    // =============================================================================================
    // Address provenance: a provider may never vouch for its own address
    // =============================================================================================

    function test_a_provider_publishes_only_a_self_declared_address() public {
        vm.prank(PINNED_CONTROLLER);
        uint16 locationNo = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        assertEq(
            uint8(directory.pinProvenance(directory.pinWord(PINNED, locationNo))),
            uint8(ProviderDirectory.AddressProvenance.SELF_DECLARED),
            "a freshly published address is not self-declared"
        );
    }

    function test_only_the_registrar_can_raise_the_address_provenance() public {
        vm.prank(PINNED_CONTROLLER);
        uint16 locationNo = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);

        vm.prank(PINNED_CONTROLLER);
        vm.expectRevert(ProviderDirectory.Unauthorized.selector);
        directory.setPinAddressProvenance(
            PINNED, locationNo, SG_LAT, SG_LNG, ProviderDirectory.AddressProvenance.MATCHED_LICENSING_REGISTER
        );

        vm.prank(REGISTRAR);
        directory.setPinAddressProvenance(
            PINNED, locationNo, SG_LAT, SG_LNG, ProviderDirectory.AddressProvenance.MATCHED_LICENSING_REGISTER
        );
        assertEq(
            uint8(directory.pinProvenance(directory.pinWord(PINNED, locationNo))),
            uint8(ProviderDirectory.AddressProvenance.MATCHED_LICENSING_REGISTER)
        );
    }

    /// The registrar checked ONE address. Carrying its confirmation across to a different one would
    /// attribute a check that was never made — the same defect class as a stale verdict rendered as a
    /// fresh one, in the one place the system admits it cannot verify anything itself.
    function test_moving_a_pin_resets_the_address_provenance() public {
        vm.prank(PINNED_CONTROLLER);
        uint16 locationNo = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        vm.prank(REGISTRAR);
        directory.setPinAddressProvenance(
            PINNED, locationNo, SG_LAT, SG_LNG, ProviderDirectory.AddressProvenance.POSTAL_CONFIRMED
        );

        vm.prank(PINNED_CONTROLLER);
        directory.updatePin(PINNED, locationNo, SYD_LAT, SYD_LNG, KIND_VET, true);

        assertEq(
            uint8(directory.pinProvenance(directory.pinWord(PINNED, locationNo))),
            uint8(ProviderDirectory.AddressProvenance.SELF_DECLARED),
            "a confirmation for one address survived a move to another"
        );
    }

    /// The converse must hold too, or a provider correcting a typo in its own opening hours would
    /// silently drop a postal confirmation of an address that did not change.
    function test_changing_only_the_kind_or_the_active_flag_preserves_the_provenance() public {
        vm.prank(PINNED_CONTROLLER);
        uint16 locationNo = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        vm.prank(REGISTRAR);
        directory.setPinAddressProvenance(
            PINNED, locationNo, SG_LAT, SG_LNG, ProviderDirectory.AddressProvenance.POSTAL_CONFIRMED
        );

        vm.prank(PINNED_CONTROLLER);
        directory.updatePin(PINNED, locationNo, SG_LAT, SG_LNG, KIND_GROOMER, false);

        assertEq(
            uint8(directory.pinProvenance(directory.pinWord(PINNED, locationNo))),
            uint8(ProviderDirectory.AddressProvenance.POSTAL_CONFIRMED),
            "a confirmation was dropped by a change that did not touch the address"
        );
    }

    /// The expected coordinates are a transaction guard, not a selector: a provider that moves the pin
    /// first must not be able to capture a confirmation intended for the old address.
    function test_a_confirmation_cannot_land_on_coordinates_the_registrar_did_not_check() public {
        vm.startPrank(PINNED_CONTROLLER);
        uint16 locationNo = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        directory.updatePin(PINNED, locationNo, SYD_LAT, SYD_LNG, KIND_VET, true);
        vm.stopPrank();

        vm.prank(REGISTRAR);
        vm.expectRevert(
            abi.encodeWithSelector(ProviderDirectory.UnexpectedCoordinates.selector, SYD_LAT, SYD_LNG)
        );
        directory.setPinAddressProvenance(
            PINNED, locationNo, SG_LAT, SG_LNG, ProviderDirectory.AddressProvenance.POSTAL_CONFIRMED
        );
    }

    /// Retraction is why this one registrar write does not require ACTIVE standing: a confirmation
    /// must remain withdrawable after a provider is suspended.
    function test_a_confirmation_can_be_retracted_after_the_provider_is_suspended() public {
        vm.prank(PINNED_CONTROLLER);
        uint16 locationNo = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        vm.startPrank(REGISTRAR);
        directory.setPinAddressProvenance(
            PINNED, locationNo, SG_LAT, SG_LNG, ProviderDirectory.AddressProvenance.POSTAL_CONFIRMED
        );
        core.setProviderStanding(PINNED, ProviderRegistry.Standing.SUSPENDED);
        directory.setPinAddressProvenance(
            PINNED, locationNo, SG_LAT, SG_LNG, ProviderDirectory.AddressProvenance.SELF_DECLARED
        );
        vm.stopPrank();

        assertEq(
            uint8(directory.pinProvenance(directory.pinWord(PINNED, locationNo))),
            uint8(ProviderDirectory.AddressProvenance.SELF_DECLARED)
        );
    }

    function test_a_provenance_that_changes_nothing_is_refused() public {
        vm.prank(PINNED_CONTROLLER);
        uint16 locationNo = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        vm.prank(REGISTRAR);
        vm.expectRevert(ProviderDirectory.NoChange.selector);
        directory.setPinAddressProvenance(
            PINNED, locationNo, SG_LAT, SG_LNG, ProviderDirectory.AddressProvenance.SELF_DECLARED
        );
    }

    // =============================================================================================
    // Resolver liveness: all three checks, and none substitutes for another
    // =============================================================================================

    /// The core never clears a stored selector when a resolver is deapproved, so reading the selector
    /// alone would defeat the registry authority's fleet-wide kill switch. This is the case that
    /// catches dropping the approval half.
    function test_a_deapproved_resolver_refuses_every_write_even_while_still_selected() public {
        vm.prank(PINNED_CONTROLLER);
        directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);

        vm.prank(REGISTRAR);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, address(directory), false);

        // Still selected — the core kept the pointer.
        assertEq(core.provider(PINNED).directoryResolver, address(directory));
        assertFalse(directory.resolverApproved(), "resolver still reports itself approved");
        assertFalse(directory.isLiveFor(PINNED), "a deapproved resolver reports itself live");

        vm.prank(PINNED_CONTROLLER);
        vm.expectRevert(ProviderDirectory.ResolverNotApproved.selector);
        directory.publishPin(PINNED, SYD_LAT, SYD_LNG, KIND_VET, true);

        vm.prank(PINNED_CONTROLLER);
        vm.expectRevert(ProviderDirectory.ResolverNotApproved.selector);
        directory.setProfileAnchor(PINNED, ANCHOR_DIGEST, 1, 0x55, 0x12, hex"1220aabb");

        vm.prank(REGISTRAR);
        vm.expectRevert(ProviderDirectory.ResolverNotApproved.selector);
        directory.setPinAddressProvenance(
            PINNED, 0, SG_LAT, SG_LNG, ProviderDirectory.AddressProvenance.POSTAL_CONFIRMED
        );
    }

    /// And the selection half: still fleet-approved, but no longer this provider's choice.
    function test_a_deselected_resolver_refuses_writes_while_still_approved() public {
        vm.prank(PINNED_CONTROLLER);
        core.setDirectoryResolver(PINNED, address(0));

        assertTrue(directory.resolverApproved(), "the fleet approval was withdrawn by a deselection");
        assertFalse(directory.isLiveFor(PINNED));

        vm.prank(PINNED_CONTROLLER);
        vm.expectRevert(ProviderDirectory.ResolverNotSelected.selector);
        directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
    }

    /// The third check is the core's own authority predicate, which neither of the other two implies.
    function test_a_stranger_cannot_write_a_providers_records() public {
        vm.prank(STRANGER);
        vm.expectRevert(ProviderDirectory.Unauthorized.selector);
        directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);

        vm.prank(STRANGER);
        vm.expectRevert(ProviderDirectory.Unauthorized.selector);
        directory.setProfileAnchor(PINNED, ANCHOR_DIGEST, 1, 0x55, 0x12, hex"1220aabb");
    }

    /// Choosing the resolver and writing into it are separate permissions in the core, so a delegate
    /// trusted with one must not thereby hold the other.
    function test_a_delegate_holding_only_the_resolver_permission_cannot_publish() public {
        // Hoisted: a getter call in the argument list would consume the prank that follows.
        uint32 resolverPermission = core.PROVIDER_PERMISSION_DIRECTORY_RESOLVER();
        uint32 recordPermission = core.PROVIDER_PERMISSION_RECORD();

        vm.prank(PINNED_CONTROLLER);
        core.setProviderDelegate(PINNED, DELEGATE, resolverPermission, 0);

        vm.prank(DELEGATE);
        vm.expectRevert(ProviderDirectory.Unauthorized.selector);
        directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);

        vm.prank(PINNED_CONTROLLER);
        core.setProviderDelegate(PINNED, DELEGATE, recordPermission, 0);

        vm.prank(DELEGATE);
        uint16 locationNo = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        assertTrue(directory.hasPin(PINNED, locationNo), "the record permission did not admit a publish");
    }

    function test_an_unregistered_provider_cannot_be_written() public {
        bytes20 unknown = bytes20(uint160(0x9999));
        vm.prank(PINNED_CONTROLLER);
        vm.expectRevert(ProviderDirectory.UnknownProvider.selector);
        directory.publishPin(unknown, SG_LAT, SG_LNG, KIND_VET, true);
    }

    /// Losing ACTIVE standing FREEZES a provider's content rather than deleting it, and there is
    /// deliberately no registrar override that could edit it for them.
    function test_a_suspended_provider_can_no_longer_edit_or_withdraw_its_content() public {
        vm.prank(PINNED_CONTROLLER);
        uint16 locationNo = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        _publishAnchor(PINNED, PINNED_CONTROLLER, ANCHOR_DIGEST);

        vm.prank(REGISTRAR);
        core.setProviderStanding(PINNED, ProviderRegistry.Standing.SUSPENDED);

        vm.startPrank(PINNED_CONTROLLER);
        vm.expectRevert(ProviderDirectory.Unauthorized.selector);
        directory.updatePin(PINNED, locationNo, SYD_LAT, SYD_LNG, KIND_VET, true);
        vm.expectRevert(ProviderDirectory.Unauthorized.selector);
        directory.removePin(PINNED, locationNo);
        vm.expectRevert(ProviderDirectory.Unauthorized.selector);
        directory.clearProfileAnchor(PINNED);
        vm.stopPrank();

        // Frozen, not gone: the pin is still in the scan and the consumer's standing join is what
        // stops it being presented.
        assertEq(directory.pinTotal(), 1, "a suspended provider's pin was dropped from the scan");
        assertTrue(directory.hasPin(PINNED, locationNo));
    }

    // =============================================================================================
    // The listing page carries the standing the pin scan deliberately omits
    // =============================================================================================

    /// The pin scan answers where a location was published, not whether the provider should be shown.
    /// Folding standing into it would cost a staticcall per pin or a mirrored copy that drifts, so the
    /// listing page carries it — read live from the core, never stored here.
    function test_the_listing_page_reports_standing_live_while_the_pin_scan_omits_it() public {
        vm.prank(PINNED_CONTROLLER);
        directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);

        (bytes32[] memory words,,,,) = directory.listingPage(0, 10);
        (,,, ProviderRegistry.Standing standing,,,) = directory.unpackListing(words[0], words[1]);
        assertEq(uint8(standing), uint8(ProviderRegistry.Standing.ACTIVE));

        vm.prank(REGISTRAR);
        core.setProviderStanding(PINNED, ProviderRegistry.Standing.SUSPENDED);

        (words,,,,) = directory.listingPage(0, 10);
        (,,, standing,,,) = directory.unpackListing(words[0], words[1]);
        assertEq(
            uint8(standing),
            uint8(ProviderRegistry.Standing.SUSPENDED),
            "the listing served a stale standing instead of the core's current one"
        );

        // The pin is still in the scan, unqualified. That is the join the consumer owes.
        (bytes32[] memory pinWords,, uint256 total,,) = directory.pinPage(0, 10);
        assertEq(total, 1);
        assertEq(directory.unpackPin(pinWords[0]).providerId, PINNED);
    }

    /// The two levers are reported SEPARATELY: the provider owns the selection and the registry
    /// authority owns the approval. Pre-combining them would leave a consumer unable to tell a
    /// provider that moved to another resolver from one the authority pulled — the same mistake as
    /// and-ing the discovery anchor's two active bits before handing them to the validator.
    function test_the_selection_and_the_approval_are_reported_as_separate_levers() public {
        vm.prank(PINNED_CONTROLLER);
        directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);

        (,,, bool pinApproved,) = directory.pinPage(0, 10);
        (bytes32[] memory words,,, bool listApproved,) = directory.listingPage(0, 10);
        (,,,,, bool selected,) = directory.unpackListing(words[0], words[1]);
        assertTrue(pinApproved);
        assertTrue(listApproved);
        assertTrue(selected);

        vm.prank(REGISTRAR);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, address(directory), false);

        bytes32[] memory pinWords;
        (pinWords,,, pinApproved,) = directory.pinPage(0, 10);
        (words,,, listApproved,) = directory.listingPage(0, 10);
        (,,,,, selected,) = directory.unpackListing(words[0], words[1]);
        assertFalse(pinApproved, "a deapproved resolver reported its page as approved");
        assertFalse(listApproved, "a deapproved resolver reported its listing page as approved");
        // Still the provider's selection: only the authority's lever moved, and the record says so.
        assertTrue(selected, "the approval lever silently cleared the provider's own selection");
        assertFalse(directory.isLiveFor(PINNED), "isLiveFor must require both levers");

        // The records themselves are still readable — deapproval withdraws the claim, not the history.
        assertEq(pinWords.length, 1, "deapproval erased the published records");

        // And the other lever on its own: approved again, but the provider has moved away.
        vm.prank(REGISTRAR);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, address(directory), true);
        vm.prank(PINNED_CONTROLLER);
        core.setDirectoryResolver(PINNED, address(0));

        (words,,, listApproved,) = directory.listingPage(0, 10);
        (,,,,, selected,) = directory.unpackListing(words[0], words[1]);
        assertTrue(listApproved, "the selection lever silently cleared the fleet approval");
        assertFalse(selected, "a provider that moved away still reads as the selection");
        assertFalse(directory.isLiveFor(PINNED));
    }

    /// An inactive pin stays in the page carrying its own flag, so the consumer filters it
    /// deliberately rather than the read deciding on their behalf. A page that quietly dropped it
    /// would report a provider as having no location when it has one it has marked closed — and the
    /// same "helpfully exclude it" instinct applied to standing is what the join above exists to
    /// prevent.
    function test_an_inactive_pin_is_returned_by_the_page_with_its_flag_intact() public {
        vm.startPrank(PINNED_CONTROLLER);
        uint16 openLocation = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        uint16 closedLocation = directory.publishPin(PINNED, SYD_LAT, SYD_LNG, KIND_VET, false);
        vm.stopPrank();

        (bytes32[] memory words,, uint256 total,,) = directory.pinPage(0, 10);
        assertEq(total, 2, "the reported total dropped an inactive pin");
        assertEq(words.length, 2, "the page dropped an inactive pin");

        bool sawClosed;
        for (uint256 i = 0; i < words.length; i++) {
            ProviderDirectory.Pin memory decoded = directory.unpackPin(words[i]);
            if (decoded.providerId == PINNED && decoded.locationNo == closedLocation) {
                sawClosed = true;
                assertFalse(directory.pinIsActive(words[i]), "the inactive flag was lost in the page");
                assertEq(decoded.lat, SYD_LAT, "the inactive pin's coordinates were blanked");
                assertEq(decoded.lng, SYD_LNG);
            }
        }
        assertTrue(sawClosed, "the inactive pin is missing from the page");
        assertTrue(directory.pinIsActive(directory.pinWord(PINNED, openLocation)));
        assertEq(directory.pinCount(PINNED), 2, "an inactive pin was not counted on the listing");
    }

    function test_a_page_carries_the_block_it_was_read_at() public {
        vm.prank(PINNED_CONTROLLER);
        directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        vm.roll(block.number + 5);

        (,,,, uint64 pinBlock) = directory.pinPage(0, 10);
        (,,,, uint64 listBlock) = directory.listingPage(0, 10);
        assertEq(pinBlock, uint64(block.number));
        assertEq(listBlock, uint64(block.number));
    }

    // =============================================================================================
    // Paging
    // =============================================================================================

    function test_pages_walk_every_record_exactly_once() public {
        vm.startPrank(PINNED_CONTROLLER);
        for (uint256 i = 0; i < 5; i++) {
            directory.publishPin(PINNED, SG_LAT + int32(uint32(i)), SG_LNG, KIND_VET, true);
        }
        vm.stopPrank();

        uint256 cursor;
        uint256 seen;
        bool[5] memory found;
        while (true) {
            (bytes32[] memory words, uint256 next, uint256 total,,) = directory.pinPage(cursor, 2);
            assertEq(total, 5);
            for (uint256 i = 0; i < words.length; i++) {
                uint16 locationNo = directory.unpackPin(words[i]).locationNo;
                assertFalse(found[locationNo], "a record appeared in two pages");
                found[locationNo] = true;
                seen++;
            }
            if (next == cursor) break;
            cursor = next;
            if (cursor == total) break;
        }
        assertEq(seen, 5, "paging did not walk every record");
    }

    /// A pin removed BETWEEN two unpinned pages can move a record past a cursor that has already gone
    /// by, so a consumer paging across blocks silently misses it. Pinned as a deliberate limitation
    /// rather than a solved problem: the remedy is to page at one block, which is what `atBlock` exists
    /// to let a consumer show it did.
    function test_paging_across_a_removal_can_skip_a_record() public {
        vm.startPrank(PINNED_CONTROLLER);
        uint16 a = directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);
        uint16 b = directory.publishPin(PINNED, SG_LAT + 1, SG_LNG, KIND_VET, true);
        uint16 c = directory.publishPin(PINNED, SG_LAT + 2, SG_LNG, KIND_VET, true);
        uint16 d = directory.publishPin(PINNED, SG_LAT + 3, SG_LNG, KIND_VET, true);
        vm.stopPrank();

        (bytes32[] memory page0, uint256 next,,,) = directory.pinPage(0, 2);
        assertEq(directory.unpackPin(page0[0]).locationNo, a);
        assertEq(directory.unpackPin(page0[1]).locationNo, b);

        // Removing the record at index 0 swaps the tail into its place.
        vm.prank(PINNED_CONTROLLER);
        directory.removePin(PINNED, a);

        (bytes32[] memory page1,,,,) = directory.pinPage(next, 2);
        assertEq(page1.length, 1, "the tail did not shrink as expected");
        assertEq(directory.unpackPin(page1[0]).locationNo, c);

        // `d` was never returned by either page, though it exists and was never removed.
        assertTrue(directory.hasPin(PINNED, d), "the skipped pin should still exist");
        assertEq(directory.unpackPin(directory.pinWord(PINNED, d)).locationNo, d);
        // And it is reachable again the moment the whole scan is read at one block.
        (bytes32[] memory whole,, uint256 total,,) = directory.pinPage(0, 10);
        assertEq(total, 3);
        bool sawD;
        for (uint256 i = 0; i < whole.length; i++) {
            if (directory.unpackPin(whole[i]).locationNo == d) sawD = true;
        }
        assertTrue(sawD, "a single-block read must see every record");
    }

    function test_a_cursor_at_the_end_returns_an_empty_page_rather_than_reverting() public {
        vm.prank(PINNED_CONTROLLER);
        directory.publishPin(PINNED, SG_LAT, SG_LNG, KIND_VET, true);

        (bytes32[] memory words, uint256 next, uint256 total,,) = directory.pinPage(1, 10);
        assertEq(words.length, 0);
        assertEq(next, 1);
        assertEq(total, 1);
    }

    function test_a_bad_page_request_is_refused() public {
        // Hoisted: a getter call in the argument list would satisfy the expectRevert by itself.
        uint256 overPinCap = directory.MAX_PIN_PAGE_SIZE() + 1;
        uint256 overListingCap = directory.MAX_LISTING_PAGE_SIZE() + 1;

        vm.expectRevert(ProviderDirectory.BadPage.selector);
        directory.pinPage(0, 0);

        vm.expectRevert(ProviderDirectory.BadPage.selector);
        directory.pinPage(0, overPinCap);

        vm.expectRevert(ProviderDirectory.BadPage.selector);
        directory.pinPage(1, 10);

        vm.expectRevert(ProviderDirectory.BadPage.selector);
        directory.listingPage(0, overListingCap);
    }

    /// The pin page's cap is deliberately larger than the listing page's because the work is not
    /// alike: a pin record is a bare storage read while a listing record makes a call into the core.
    function test_the_two_page_caps_are_deliberately_different() public view {
        assertEq(directory.MAX_LISTING_PAGE_SIZE(), core.MAX_PAGE_SIZE());
        assertTrue(
            directory.MAX_PIN_PAGE_SIZE() > directory.MAX_LISTING_PAGE_SIZE(),
            "the pin scan lost its larger page budget"
        );
    }

    function test_a_listing_page_is_two_words_per_record() public {
        _publishAnchor(PINNED, PINNED_CONTROLLER, ANCHOR_DIGEST);
        _publishAnchor(CONTACT_ONLY, CONTACT_ONLY_CONTROLLER, keccak256("second"));

        (bytes32[] memory words, uint256 next, uint256 total,,) = directory.listingPage(0, 10);
        assertEq(total, 2);
        assertEq(next, 2);
        assertEq(words.length, 2 * directory.LISTING_RECORD_WORDS());

        (bytes20 first,,,,,,) = directory.unpackListing(words[0], words[1]);
        (bytes20 second,,,,,, bytes32 secondDigest) = directory.unpackListing(words[2], words[3]);
        assertEq(first, PINNED);
        assertEq(second, CONTACT_ONLY);
        assertEq(secondDigest, keccak256("second"));
    }

    // =============================================================================================
    // The profile / contact anchor
    // =============================================================================================

    function test_the_anchor_round_trips_and_records_who_wrote_it() public {
        vm.roll(1234);
        _publishAnchor(PINNED, PINNED_CONTROLLER, ANCHOR_DIGEST);

        ProviderDirectory.ProfileAnchor memory anchor = directory.profileAnchor(PINNED);
        assertEq(anchor.digest, ANCHOR_DIGEST);
        assertEq(anchor.schema, 1);
        assertEq(anchor.codec, 0x55);
        assertEq(anchor.hashAlgorithm, 0x12);
        assertEq(anchor.contenthash, hex"1220aabb");
        assertEq(anchor.revision, 1);
        assertEq(anchor.updatedAtBlock, 1234);
        assertEq(anchor.setBy, PINNED_CONTROLLER);
    }

    function test_a_malformed_anchor_is_refused() public {
        // Hoisted: a getter call in the argument list would satisfy the expectRevert by itself.
        bytes memory tooLong = new bytes(directory.MAX_CONTENTHASH_LENGTH() + 1);

        vm.startPrank(PINNED_CONTROLLER);
        vm.expectRevert(ProviderDirectory.BadProfileAnchor.selector);
        directory.setProfileAnchor(PINNED, bytes32(0), 1, 0x55, 0x12, "");

        vm.expectRevert(ProviderDirectory.BadProfileAnchor.selector);
        directory.setProfileAnchor(PINNED, ANCHOR_DIGEST, 0, 0x55, 0x12, "");

        vm.expectRevert(ProviderDirectory.BadProfileAnchor.selector);
        directory.setProfileAnchor(PINNED, ANCHOR_DIGEST, 1, 0x55, 0, "");

        vm.expectRevert(ProviderDirectory.ContenthashTooLong.selector);
        directory.setProfileAnchor(PINNED, ANCHOR_DIGEST, 1, 0x55, 0x12, tooLong);
        vm.stopPrank();

        assertFalse(directory.isListed(PINNED), "a refused anchor still listed the provider");
    }

    function test_clearing_an_anchor_that_was_never_published_is_refused() public {
        vm.prank(PINNED_CONTROLLER);
        vm.expectRevert(ProviderDirectory.NoProfileAnchor.selector);
        directory.clearProfileAnchor(PINNED);
    }

    function test_republishing_advances_the_revision_and_keeps_one_listing() public {
        _publishAnchor(PINNED, PINNED_CONTROLLER, ANCHOR_DIGEST);
        _publishAnchor(PINNED, PINNED_CONTROLLER, keccak256("v2"));

        ProviderDirectory.ProfileAnchor memory anchor = directory.profileAnchor(PINNED);
        assertEq(anchor.digest, keccak256("v2"));
        assertEq(anchor.revision, 2);
        assertEq(directory.listingTotal(), 1);
    }

    /// This anchor is provider-written content and carries no registrar attestation. The core's
    /// `PublicIdentityAnchor` is the registrar-written one; conflating them would let a provider's own
    /// blob inherit the registrar's word.
    function test_the_directory_anchor_is_independent_of_the_cores_identity_anchor() public {
        _publishAnchor(PINNED, PINNED_CONTROLLER, ANCHOR_DIGEST);

        assertEq(directory.profileAnchor(PINNED).digest, ANCHOR_DIGEST);
        assertEq(
            core.publicIdentityAnchor(PINNED).digest,
            keccak256(abi.encode(PINNED)),
            "the provider's own blob overwrote the registrar's identity anchor"
        );
    }

    // =============================================================================================
    // Construction
    // =============================================================================================

    function test_the_core_address_is_required() public {
        vm.expectRevert(ProviderDirectory.ZeroAddress.selector);
        new ProviderDirectory(ProviderRegistry(address(0)));
    }

    /// The permission bit is snapshotted from the core's own constant rather than restated as a
    /// literal, so the two cannot drift.
    function test_the_record_permission_is_taken_from_the_core() public view {
        assertEq(directory.providerRecordPermission(), core.PROVIDER_PERMISSION_RECORD());
        assertEq(address(directory.core()), address(core));
    }

    function test_the_coordinate_scale_leaves_real_headroom_in_int32() public view {
        assertEq(directory.COORDINATE_SCALE(), 1_000_000);
        assertEq(directory.LATITUDE_LIMIT(), 90_000_000);
        assertEq(directory.LONGITUDE_LIMIT(), 180_000_000);
        // 1e7 scaling would leave only 16.2% headroom; 1e6 leaves 91.6%, which is why the scale is a
        // decision rather than a default.
        assertTrue(
            uint32(type(int32).max) > uint32(directory.LONGITUDE_LIMIT()) * 11 / 10,
            "the longitude limit is within 10% of the int32 ceiling"
        );
    }
}

/// @dev The page-cost measurements live in their OWN contract because the seeding has to happen in
/// `setUp`, a separate transaction from the measured read. Publishing and reading in one test body
/// leaves every storage slot WARM, which understates a real `eth_call` by roughly a factor of three —
/// the design's whole cap argument is about the COLD `SLOAD` per record, so measuring the warm number
/// would be measuring the wrong thing and quoting it would be worse than quoting nothing.
contract ProviderDirectoryPageCostTest is Test {
    ProviderRegistry internal core;
    ProviderDirectory internal directory;

    address internal constant REGISTRAR = address(0xA11CE);
    address internal constant CONTROLLER = address(0xB0B);
    bytes20 internal constant PINNED = bytes20(uint160(0x1111));

    int32 internal constant SG_LAT = 1_352_100;
    int32 internal constant SG_LNG = 103_819_800;
    uint8 internal constant KIND_VET = 1;

    uint256 internal pinCap;
    uint256 internal listingCap;

    function setUp() public {
        core = new ProviderRegistry(REGISTRAR);
        directory = new ProviderDirectory(core);
        pinCap = directory.MAX_PIN_PAGE_SIZE();
        listingCap = directory.MAX_LISTING_PAGE_SIZE();

        vm.prank(REGISTRAR);
        core.setResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, address(directory), true);

        // One provider carrying a full pin page.
        _register(PINNED, CONTROLLER);
        vm.startPrank(CONTROLLER);
        for (uint256 i = 0; i < pinCap; i++) {
            directory.publishPin(PINNED, SG_LAT + int32(uint32(i)), SG_LNG, KIND_VET, true);
        }
        vm.stopPrank();

        // Enough distinct providers to fill a listing page, each with one pin so it is listed.
        for (uint256 i = 0; i < listingCap; i++) {
            bytes20 providerId = bytes20(uint160(0x100000 + i));
            address controller = address(uint160(0x200000 + i));
            _register(providerId, controller);
            vm.prank(controller);
            directory.publishPin(providerId, SG_LAT, SG_LNG, KIND_VET, true);
        }
    }

    function _register(bytes20 providerId, address controller) internal {
        vm.startPrank(REGISTRAR);
        core.registerProvider(providerId, controller, keccak256(abi.encode(providerId)), 1, 1, 1, "");
        core.setProviderStanding(providerId, ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();
        vm.prank(controller);
        core.setDirectoryResolver(providerId, address(directory));
    }

    /// The documented cap must be a measured fact, not an estimate carried over from a design note.
    function test_a_full_pin_page_at_the_documented_cap_is_readable() public {
        uint256 before = gasleft();
        (bytes32[] memory words, uint256 next, uint256 total,,) = directory.pinPage(0, pinCap);
        uint256 used = before - gasleft();

        assertEq(words.length, pinCap, "the page did not return a full cap of records");
        assertEq(next, pinCap);
        assertGe(total, pinCap);
        // Decode both ends, so a page returning the right LENGTH of garbage fails too.
        assertEq(directory.unpackPin(words[0]).locationNo, 0);
        assertEq(directory.unpackPin(words[0]).lat, SG_LAT);
        assertEq(directory.unpackPin(words[pinCap - 1]).locationNo, uint16(pinCap - 1));
        assertEq(directory.unpackPin(words[pinCap - 1]).lat, SG_LAT + int32(uint32(pinCap - 1)));

        emit log_named_uint("gas: full pin page at MAX_PIN_PAGE_SIZE (cold)", used);
        // A node's `eth_call` gas cap is ~50M (Geth's documented default). The bound is deliberately
        // well inside it so the cap keeps its headroom for ABI encoding and memory expansion.
        assertLt(used, 10_000_000, "a full pin page outgrew a node's call gas budget");
    }

    /// The listing cap is the expensive read — one staticcall into the core per record — which is what
    /// justifies its cap being the smaller of the two rather than an oversight.
    function test_a_full_listing_page_at_the_documented_cap_is_readable() public {
        uint256 before = gasleft();
        (bytes32[] memory words,, uint256 total,,) = directory.listingPage(0, listingCap);
        uint256 used = before - gasleft();

        assertGe(total, listingCap, "the listing did not hold a full cap of records");
        assertEq(words.length, listingCap * directory.LISTING_RECORD_WORDS());
        (,,, ProviderRegistry.Standing standing,, bool selected,) =
            directory.unpackListing(words[0], words[1]);
        assertEq(uint8(standing), uint8(ProviderRegistry.Standing.ACTIVE));
        assertTrue(selected);

        emit log_named_uint("gas: full listing page at MAX_LISTING_PAGE_SIZE (cold)", used);
        assertLt(used, 10_000_000, "a full listing page outgrew a node's call gas budget");
    }

    /// Per-record cost is what actually decides the caps, so state it as a measured ratio rather than
    /// leaving two absolute numbers whose relationship a reader has to infer.
    function test_a_listing_record_costs_more_than_a_pin_record() public {
        uint256 before = gasleft();
        directory.pinPage(0, pinCap);
        uint256 pinPageGas = before - gasleft();

        before = gasleft();
        directory.listingPage(0, listingCap);
        uint256 listingPageGas = before - gasleft();

        uint256 perPin = pinPageGas / pinCap;
        uint256 perListing = listingPageGas / listingCap;
        emit log_named_uint("gas per pin record", perPin);
        emit log_named_uint("gas per listing record", perListing);
        assertGt(perListing, perPin, "a listing record is supposed to be the costlier read");
    }
}
