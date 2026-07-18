// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";
import {ProtocolVersions} from "../script/ProtocolVersions.sol";

/// @notice M7 P3 — the discovery TRUST ANCHOR (§5.1). Covers: timelocked publish (propose→2-day→
/// execute), role-gating, deprecate-keeps-history, enumeration, exact read-back (incl. the string
/// members the auto-getter omits), and the frozen artifact pins against the committed artifacts.
contract ProtocolRegistryTest is Test {
    ProtocolRegistry internal reg;

    address internal constant ADMIN = address(0xA11CE);
    address internal constant PUBLISHER = address(0xB0B);
    address internal constant OUTSIDER = address(0xBAD);

    bytes32 internal aId;
    bytes32 internal bId;

    function setUp() public {
        reg = new ProtocolRegistry(ADMIN, PUBLISHER);
        aId = ProtocolVersions.levelAId();
        bId = ProtocolVersions.levelBId();
    }

    // --- helpers ---------------------------------------------------------------------------------

    function _publish(ProtocolRegistry.Version memory v) internal {
        vm.prank(PUBLISHER);
        reg.proposeVersion(v);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        reg.executeVersion(v.versionId);
    }

    function _assertEqVersion(ProtocolRegistry.Version memory got, ProtocolRegistry.Version memory want)
        internal
        pure
    {
        assertEq(got.versionId, want.versionId, "versionId");
        assertEq(got.factory, want.factory, "factory");
        assertEq(got.verificationRegistry, want.verificationRegistry, "verificationRegistry");
        assertEq(got.sbt, want.sbt, "sbt");
        assertEq(got.verifier, want.verifier, "verifier");
        assertEq(got.circuitId, want.circuitId, "circuitId");
        assertEq(got.zkeySha256, want.zkeySha256, "zkeySha256");
        assertEq(got.witnessMobileSha256, want.witnessMobileSha256, "witnessMobileSha256");
        assertEq(got.witnessServerR1csSha256, want.witnessServerR1csSha256, "witnessServerR1csSha256");
        assertEq(got.witnessServerWasmSha256, want.witnessServerWasmSha256, "witnessServerWasmSha256");
        // The string members are exactly what getVersion (not the auto-getter) must round-trip.
        assertEq(got.artifactBaseUrl, want.artifactBaseUrl, "artifactBaseUrl");
        assertEq(got.minAppVersion, want.minAppVersion, "minAppVersion");
    }

    // --- publish + exact read-back ---------------------------------------------------------------

    /// getVersion reads back EXACTLY what was published, including the string members, and the flow
    /// stamps publishedAt/active authoritatively at execute.
    function test_publish_reads_back_exactly() public {
        ProtocolRegistry.Version memory want = ProtocolVersions.levelB();
        _publish(want);

        ProtocolRegistry.Version memory got = reg.getVersion(bId);
        _assertEqVersion(got, want);
        assertTrue(got.active, "active set true at execute");
        assertEq(got.publishedAt, uint64(block.timestamp), "publishedAt stamped at execute");
    }

    /// executeVersion IGNORES calldata publishedAt/active — a publisher cannot back-date or
    /// pre-activate a version, and a re-proposed record cannot smuggle active=false through execute.
    function test_execute_ignores_calldata_publishedAt_and_active() public {
        ProtocolRegistry.Version memory v = ProtocolVersions.levelA();
        v.publishedAt = 12345; // lie
        v.active = false; // will be overridden to true

        vm.prank(PUBLISHER);
        reg.proposeVersion(v);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        reg.executeVersion(aId);

        ProtocolRegistry.Version memory got = reg.getVersion(aId);
        assertEq(got.publishedAt, uint64(block.timestamp), "publishedAt is block time, not calldata");
        assertTrue(got.active, "active forced true");
    }

    // --- timelock enforcement --------------------------------------------------------------------

    function test_execute_before_timelock_reverts() public {
        ProtocolRegistry.Version memory v = ProtocolVersions.levelB();
        vm.prank(PUBLISHER);
        reg.proposeVersion(v);

        // one second short of the ETA
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK() - 1);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("timelock"));
        reg.executeVersion(bId);

        // exactly at the ETA it succeeds
        vm.warp(block.timestamp + 1);
        vm.prank(PUBLISHER);
        reg.executeVersion(bId);
        assertTrue(reg.getVersion(bId).active);
    }

    function test_execute_without_proposal_reverts() public {
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("none pending"));
        reg.executeVersion(bId);
    }

    function test_reproposing_resets_the_timelock() public {
        ProtocolRegistry.Version memory v = ProtocolVersions.levelB();
        vm.prank(PUBLISHER);
        reg.proposeVersion(v);
        uint256 firstEta = reg.publishEta(bId);

        // Nearly at the first ETA, re-propose: the ETA must move forward by a fresh full timelock.
        vm.warp(firstEta - 1);
        vm.prank(PUBLISHER);
        reg.proposeVersion(v);
        assertEq(reg.publishEta(bId), block.timestamp + reg.PUBLISH_TIMELOCK(), "eta reset");

        // The old ETA is no longer sufficient.
        vm.warp(firstEta);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("timelock"));
        reg.executeVersion(bId);
    }

    // --- role-gating (propose / execute / deprecate) ---------------------------------------------

    /// Reads the role FIRST (a real call), then arms expectRevert — so the role read is not itself
    /// caught, and the immediately-following pranked call is the one that must revert.
    function _expectUnauthorized(address who) internal {
        bytes32 role = reg.PUBLISHER_ROLE();
        vm.expectRevert(
            abi.encodeWithSelector(IAccessControl.AccessControlUnauthorizedAccount.selector, who, role)
        );
    }

    function test_propose_requires_publisher_role() public {
        _expectUnauthorized(OUTSIDER);
        vm.prank(OUTSIDER);
        reg.proposeVersion(ProtocolVersions.levelB());
    }

    function test_execute_requires_publisher_role() public {
        vm.prank(PUBLISHER);
        reg.proposeVersion(ProtocolVersions.levelB());
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());

        _expectUnauthorized(OUTSIDER);
        vm.prank(OUTSIDER);
        reg.executeVersion(bId);
    }

    function test_deprecate_requires_publisher_role() public {
        _publish(ProtocolVersions.levelB());
        _expectUnauthorized(OUTSIDER);
        vm.prank(OUTSIDER);
        reg.deprecateVersion(bId);
    }

    /// The admin can hand PUBLISHER_ROLE to a fresh key, which can then publish — the governance lever.
    function test_admin_can_grant_publisher_role() public {
        address newPub = address(0xC0FFEE);
        bytes32 role = reg.PUBLISHER_ROLE();
        vm.prank(ADMIN);
        reg.grantRole(role, newPub);

        vm.prank(newPub);
        reg.proposeVersion(ProtocolVersions.levelB());
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(newPub);
        reg.executeVersion(bId);
        assertTrue(reg.getVersion(bId).active);
    }

    // --- deprecate keeps history -----------------------------------------------------------------

    function test_deprecate_flips_active_without_deleting() public {
        _publish(ProtocolVersions.levelB());
        assertTrue(reg.getVersion(bId).active);

        vm.prank(PUBLISHER);
        reg.deprecateVersion(bId);

        // Still readable, still enumerated, still fully pinned — only `active` changed.
        ProtocolRegistry.Version memory got = reg.getVersion(bId);
        assertFalse(got.active, "active flipped false");
        _assertEqVersion(got, ProtocolVersions.levelB()); // every other field intact (history pinned)
        assertEq(reg.versionCount(), 1, "deprecate does not remove from the list");
        assertEq(reg.versionList(0), bId, "list entry preserved");
    }

    function test_deprecate_unknown_reverts() public {
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("unknown version"));
        reg.deprecateVersion(keccak256("nope"));
    }

    // --- enumeration -----------------------------------------------------------------------------

    function test_enumeration_of_both_versions() public {
        _publish(ProtocolVersions.levelA());
        _publish(ProtocolVersions.levelB());

        assertEq(reg.versionCount(), 2, "two distinct versions");
        // Order is publish order.
        assertEq(reg.versionList(0), aId);
        assertEq(reg.versionList(1), bId);
        assertEq(reg.getVersion(aId).versionId, aId);
        assertEq(reg.getVersion(bId).versionId, bId);
    }

    /// A swap-republish (same id, new addresses) updates in place and MUST NOT duplicate the list
    /// entry — history is one row per version, updated, never appended twice.
    function test_swap_republish_does_not_duplicate_list_entry() public {
        _publish(ProtocolVersions.levelB());
        assertEq(reg.versionCount(), 1);

        // Contract-only bump (§5.5): same version id, new registry address, same VK/pins.
        ProtocolRegistry.Version memory v2 = ProtocolVersions.levelB();
        v2.verificationRegistry = address(0xDEAD);
        _publish(v2);

        assertEq(reg.versionCount(), 1, "swap-republish does not grow the list");
        assertEq(reg.getVersion(bId).verificationRegistry, address(0xDEAD), "updated in place");
    }

    // --- getVersion fail-closed ------------------------------------------------------------------

    function test_getVersion_unknown_reverts() public {
        vm.expectRevert(bytes("unknown version"));
        reg.getVersion(keccak256("dogtag-levelc/9"));
    }

    // --- proposeVersion input validation ---------------------------------------------------------

    function test_propose_rejects_zero_versionId() public {
        ProtocolRegistry.Version memory v = ProtocolVersions.levelB();
        v.versionId = bytes32(0);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("versionId=0"));
        reg.proposeVersion(v);
    }

    function test_propose_rejects_zero_trio_or_verifier() public {
        ProtocolRegistry.Version memory v = ProtocolVersions.levelB();
        v.verifier = address(0);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("zero trio/verifier"));
        reg.proposeVersion(v);
    }

    function test_propose_rejects_zero_zkey_pin() public {
        ProtocolRegistry.Version memory v = ProtocolVersions.levelB();
        v.zkeySha256 = bytes32(0);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("zkeySha256=0"));
        reg.proposeVersion(v);
    }

    /// The graph pin is DELIBERATELY allowed to be 0 (unpinned — the .graph is not committed), unlike
    /// the mandatory zkey pin. Publishing with witnessMobileSha256 == 0 must succeed.
    function test_propose_allows_unpinned_graph() public {
        ProtocolRegistry.Version memory v = ProtocolVersions.levelB();
        assertEq(v.witnessMobileSha256, bytes32(0), "fixture leaves the graph unpinned");
        _publish(v);
        assertEq(reg.getVersion(bId).witnessMobileSha256, bytes32(0));
    }

    // --- frozen pins verified against the committed artifacts -------------------------------------

    /// The `witnessServerWasmSha256` each version publishes IS the SHA-256 of the committed
    /// `circuits/build/*.wasm` — hashed here in-EVM (the ~4 MB wasm fits; the 11-64 MB r1cs/zkey do
    /// not — `vm.readFileBinary` MemoryOOGs — so those pins are file-verified by the Rust
    /// `dogtag-prover-rs` `*_descriptor_pins_match_the_real_artifacts` tests, which stream-hash them).
    function test_wasm_pins_match_committed_artifacts() public view {
        assertEq(
            sha256(vm.readFileBinary("../circuits/build/consent_js/consent.wasm")),
            ProtocolVersions.levelB().witnessServerWasmSha256,
            "levelb wasm pin != committed consent.wasm"
        );
        assertEq(
            sha256(vm.readFileBinary("../circuits/build/verification_js/verification.wasm")),
            ProtocolVersions.levelA().witnessServerWasmSha256,
            "levela wasm pin != committed verification.wasm"
        );
    }

    /// Guard the frozen zkey/r1cs pins as explicit non-zero constants (the values the Rust tests hash
    /// the committed files against). A pin silently zeroed or swapped in the fixture fails here.
    function test_frozen_zkey_and_r1cs_pins_are_the_expected_constants() public pure {
        ProtocolRegistry.Version memory a = ProtocolVersions.levelA();
        assertEq(a.zkeySha256, 0x9e3636b9c12b57b8662e34505a01e19bfc87a99189c994b0d87bc2e3dcdcd992);
        assertEq(a.witnessServerR1csSha256, 0x8890e52860b5b43535143682892bd6fa87ffefab3084b4eb2186b91c99be6fe8);

        ProtocolRegistry.Version memory b = ProtocolVersions.levelB();
        assertEq(b.zkeySha256, 0xf83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868);
        assertEq(b.witnessServerR1csSha256, 0x828e2923a159b04f2de421d4b447f8c85356677f4f83a5af55b42eb2b4f9b6b7);
    }
}
