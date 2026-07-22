// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";
import {ProtocolVersions} from "../script/ProtocolVersions.sol";

/// @notice M7 P3 — the discovery TRUST ANCHOR (§5.1). Covers: timelocked publish (propose→2-day→
/// execute) on BOTH axes and on the binding, role-gating, deprecate-keeps-history, enumeration, exact
/// read-back (incl. the string members the auto-getter omits), and the frozen artifact pins against the
/// committed artifacts.
///
/// The load-bearing section is `--- R-5: the two axes rotate independently ---`: it is what proves the
/// on-chain contract set and the off-chain proving artifacts are no longer one coupled version.
contract ProtocolRegistryTest is Test {
    ProtocolRegistry internal reg;

    address internal constant ADMIN = address(0xA11CE);
    address internal constant PUBLISHER = address(0xB0B);
    address internal constant OUTSIDER = address(0xBAD);

    address internal constant FACTORY = address(0xFACADE);
    address internal constant VERIFICATION_REGISTRY = address(0xC0DE);
    address internal constant SBT = address(0x5B7);
    address internal constant VERIFIER = address(0xBEEF);

    bytes32 internal constant ZKEY_SHA256 =
        0xf83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868;
    bytes32 internal constant R1CS_SHA256 =
        0x828e2923a159b04f2de421d4b447f8c85356677f4f83a5af55b42eb2b4f9b6b7;
    bytes32 internal constant WASM_SHA256 =
        0x482debcff5a4325c008dd00e4476bba011d0a706da955e3129d114f996a913e6;

    bytes32 internal bId;
    bytes32 internal bArtId;

    function setUp() public {
        reg = new ProtocolRegistry(ADMIN, PUBLISHER, 2 days);
        bId = ProtocolVersions.levelBId();
        bArtId = ProtocolVersions.levelBArtifactsId();
    }

    // --- helpers ---------------------------------------------------------------------------------

    function _publishContracts(ProtocolRegistry.ContractSet memory c) internal {
        vm.prank(PUBLISHER);
        reg.proposeContractSet(c);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        reg.executeContractSet(c.contractSetId);
    }

    function _publishArtifacts(ProtocolRegistry.ArtifactSet memory a) internal {
        vm.prank(PUBLISHER);
        reg.proposeArtifactSet(a);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        reg.executeArtifactSet(a.artifactSetId);
    }

    function _bind(bytes32 contractSetId, bytes32 artifactSetId) internal {
        vm.prank(PUBLISHER);
        reg.proposeArtifactBinding(contractSetId, artifactSetId);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        reg.executeArtifactBinding(contractSetId);
    }

    function _contracts() internal pure returns (ProtocolRegistry.ContractSet memory) {
        return ProtocolVersions.levelBContracts(FACTORY, VERIFICATION_REGISTRY, SBT, VERIFIER);
    }

    function _artifacts() internal pure returns (ProtocolRegistry.ArtifactSet memory) {
        return ProtocolVersions.levelBArtifacts(
            ZKEY_SHA256, bytes32(0), R1CS_SHA256, WASM_SHA256, "https://artifacts.dogtag.io/consent1"
        );
    }

    /// Publish the one live version on both independent axes, then bind them.
    function _publishLevelB() internal {
        _publishContracts(_contracts());
        _publishArtifacts(_artifacts());
        _bind(bId, bArtId);
    }

    function _assertEqContracts(
        ProtocolRegistry.ContractSet memory got,
        ProtocolRegistry.ContractSet memory want
    ) internal pure {
        assertEq(got.contractSetId, want.contractSetId, "contractSetId");
        assertEq(got.factory, want.factory, "factory");
        assertEq(got.verificationRegistry, want.verificationRegistry, "verificationRegistry");
        assertEq(got.sbt, want.sbt, "sbt");
        assertEq(got.verifier, want.verifier, "verifier");
        assertEq(got.circuitId, want.circuitId, "circuitId");
    }

    function _assertEqArtifacts(
        ProtocolRegistry.ArtifactSet memory got,
        ProtocolRegistry.ArtifactSet memory want
    ) internal pure {
        assertEq(got.artifactSetId, want.artifactSetId, "artifactSetId");
        assertEq(got.zkeySha256, want.zkeySha256, "zkeySha256");
        assertEq(got.witnessMobileSha256, want.witnessMobileSha256, "witnessMobileSha256");
        assertEq(got.witnessServerR1csSha256, want.witnessServerR1csSha256, "witnessServerR1csSha256");
        assertEq(got.witnessServerWasmSha256, want.witnessServerWasmSha256, "witnessServerWasmSha256");
        // The string members are exactly what getArtifactSet (not the auto-getter) must round-trip.
        assertEq(got.artifactBaseUrl, want.artifactBaseUrl, "artifactBaseUrl");
        assertEq(got.minAppVersion, want.minAppVersion, "minAppVersion");
    }

    // =============================================================================================
    // R-5: the two axes rotate independently
    // =============================================================================================

    /// THE headline invariant, direction 1: rotating the PROVING ARTIFACTS (a new zkey, a new host, a
    /// raised minAppVersion) leaves the on-chain contract set byte-identical. No trio address moves, no
    /// verifier changes, nothing is redeployed — the only writes are the new artifact set plus the
    /// binding re-point.
    function test_artifact_rotation_leaves_the_contract_set_untouched() public {
        _publishLevelB();
        ProtocolRegistry.ContractSet memory before = reg.getContractSet(bId);

        // A genuine artifact-only rotation: brand new zkey + pins + host + app floor, same protocol.
        ProtocolRegistry.ArtifactSet memory v2 = _artifacts();
        v2.artifactSetId = keccak256("dogtag-levelb-artifacts/2");
        v2.zkeySha256 = keccak256("a-freshly-ceremonied-zkey");
        v2.witnessServerR1csSha256 = keccak256("new-r1cs");
        v2.witnessServerWasmSha256 = keccak256("new-wasm");
        v2.artifactBaseUrl = "https://artifacts.dogtag.io/levelb2";
        v2.minAppVersion = "2.0.0";
        _publishArtifacts(v2);
        _bind(bId, v2.artifactSetId);

        // The resolver now sees the NEW artifacts...
        ProtocolRegistry.ArtifactSet memory nowArt = reg.getActiveArtifactSet(bId);
        assertEq(nowArt.artifactSetId, v2.artifactSetId, "binding re-pointed");
        assertEq(nowArt.zkeySha256, v2.zkeySha256, "new zkey pin is live");
        assertEq(nowArt.minAppVersion, "2.0.0", "new app floor is live");

        // ...while the on-chain axis is EXACTLY what it was, publishedAt included: it was never written.
        ProtocolRegistry.ContractSet memory afterRotation = reg.getContractSet(bId);
        _assertEqContracts(afterRotation, before);
        assertEq(afterRotation.publishedAt, before.publishedAt, "contract set was not re-published");
        assertTrue(afterRotation.active, "contract set still active");
        assertEq(reg.contractSetCount(), 1, "no new contract set was created");

        // And the superseded artifact set is still pinned in history, readable by its own id.
        _assertEqArtifacts(reg.getArtifactSet(bArtId), _artifacts());
        assertEq(reg.artifactSetCount(), 2, "both artifact sets enumerated");
    }

    /// THE headline invariant, direction 2: rotating the ON-CHAIN trio addresses leaves the artifact set
    /// byte-identical and the binding intact. No app is forced to update, no zkey is re-fetched.
    ///
    /// Deliberately rotates the TRIO ADDRESSES (factory/registry/sbt) rather than the `verifier`: a
    /// verifier swap means a new VK, which in reality implies new proving artifacts too. Rotating the
    /// trio is the change that is genuinely artifact-neutral, so it is the honest test of "the reverse".
    function test_contract_rotation_leaves_the_artifact_set_untouched() public {
        _publishLevelB();
        ProtocolRegistry.ArtifactSet memory before = reg.getArtifactSet(bArtId);

        ProtocolRegistry.ContractSet memory c2 = _contracts();
        c2.factory = address(0xF00D);
        c2.verificationRegistry = address(0xDEAD);
        c2.sbt = address(0xBEEF);
        _publishContracts(c2);

        // The on-chain axis moved...
        ProtocolRegistry.ContractSet memory nowContracts = reg.getContractSet(bId);
        assertEq(nowContracts.verificationRegistry, address(0xDEAD), "trio rotated");
        assertEq(nowContracts.factory, address(0xF00D));
        assertEq(nowContracts.sbt, address(0xBEEF));

        // ...and the artifact axis did not: same record, same publishedAt, same binding, no new set.
        ProtocolRegistry.ArtifactSet memory afterRotation = reg.getArtifactSet(bArtId);
        _assertEqArtifacts(afterRotation, before);
        assertEq(afterRotation.publishedAt, before.publishedAt, "artifact set was not re-published");
        assertEq(reg.activeArtifactSetOf(bId), bArtId, "binding survived the contract rotation");
        assertEq(reg.artifactSetCount(), 1, "no new artifact set was created");
    }

    /// The two axes are separately keyed: a contract-set id is NOT an artifact-set id and vice versa.
    /// This is what makes "independent" structural rather than merely conventional.
    function test_the_two_axes_do_not_share_a_keyspace() public {
        _publishLevelB();

        vm.expectRevert(bytes("unknown artifact set"));
        reg.getArtifactSet(bId); // the contract-set id is meaningless on the artifact axis

        vm.expectRevert(bytes("unknown contract set"));
        reg.getContractSet(bArtId); // and vice versa
    }

    /// `resolve` is the one-call app path and must return BOTH halves, each from its own axis.
    function test_resolve_returns_both_axes() public {
        _publishLevelB();
        (ProtocolRegistry.ContractSet memory c, ProtocolRegistry.ArtifactSet memory a) = reg.resolve(bId);
        _assertEqContracts(c, _contracts());
        _assertEqArtifacts(a, _artifacts());
    }

    // --- binding: fail-closed + timelock ----------------------------------------------------------

    /// A contract set with no artifacts bound yet is a valid intermediate state during rollout, and
    /// every artifact-side resolver must FAIL CLOSED on it rather than read a zeroed record.
    function test_unbound_contract_set_fails_closed_on_artifact_resolution() public {
        _publishContracts(_contracts());

        vm.expectRevert(bytes("no artifact binding"));
        reg.getActiveArtifactSet(bId);

        vm.expectRevert(bytes("no artifact binding"));
        reg.resolve(bId);

        // ...but the on-chain axis resolves fine: the axes really are independent, including in failure.
        assertEq(reg.getContractSet(bId).contractSetId, bId);
    }

    /// The binding is the pointer an app follows to decide which bytes to fetch, so a one-transaction
    /// repoint must be impossible — it carries the same 2-day timelock as a publish.
    function test_binding_is_timelocked() public {
        _publishContracts(_contracts());
        _publishArtifacts(_artifacts());

        vm.prank(PUBLISHER);
        reg.proposeArtifactBinding(bId, bArtId);
        assertEq(reg.getPendingBinding(bId), bArtId, "pending target is inspectable during the window");

        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK() - 1);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("timelock"));
        reg.executeArtifactBinding(bId);

        vm.warp(block.timestamp + 1);
        vm.prank(PUBLISHER);
        reg.executeArtifactBinding(bId);
        assertEq(reg.activeArtifactSetOf(bId), bArtId);
    }

    /// The pointer can never be made to dangle: both sides must be published AT EXECUTE. Checking at
    /// execute (not propose) is what lets the first rollout run all three timelocks concurrently.
    function test_binding_execute_requires_both_sides_published() public {
        _publishContracts(_contracts());

        vm.prank(PUBLISHER);
        reg.proposeArtifactBinding(bId, bArtId); // artifact set not published yet — allowed to stage
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("unknown artifact set"));
        reg.executeArtifactBinding(bId);

        // Publish it, and the SAME pending binding now executes (the proposal was not consumed).
        _publishArtifacts(_artifacts());
        vm.prank(PUBLISHER);
        reg.executeArtifactBinding(bId);
        assertEq(reg.activeArtifactSetOf(bId), bArtId);
    }

    /// Deprecate is the EMERGENCY lever: a set retired because it is COMPROMISED must not still be
    /// reachable through a proposal staged before the retirement, or the window that exists to catch a
    /// malicious repoint would let the repoint through anyway.
    function test_binding_cannot_point_at_a_deprecated_artifact_set() public {
        _publishContracts(_contracts());
        _publishArtifacts(_artifacts());

        vm.prank(PUBLISHER);
        reg.proposeArtifactBinding(bId, bArtId); // staged while the set is still active
        vm.prank(PUBLISHER);
        reg.deprecateArtifactSet(bArtId);

        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("inactive artifact set"));
        reg.executeArtifactBinding(bId);

        // The pointer was never set — the contract set stays unbound and fails closed on resolution.
        assertEq(reg.activeArtifactSetOf(bId), bytes32(0), "no binding was written");
    }

    /// The symmetric case on the on-chain axis: each `active` bit is an independent kill switch, so
    /// retiring the contract set must refuse a new binding just as retiring the artifact set does.
    function test_binding_cannot_point_at_a_deprecated_contract_set() public {
        _publishContracts(_contracts());
        _publishArtifacts(_artifacts());

        vm.prank(PUBLISHER);
        reg.proposeArtifactBinding(bId, bArtId);
        vm.prank(PUBLISHER);
        reg.deprecateContractSet(bId);

        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("inactive contract set"));
        reg.executeArtifactBinding(bId);

        assertEq(reg.activeArtifactSetOf(bId), bytes32(0), "no binding was written");
    }

    function test_binding_requires_publisher_role() public {
        _publishLevelB();
        _expectUnauthorized(OUTSIDER);
        vm.prank(OUTSIDER);
        reg.proposeArtifactBinding(bId, bArtId);
    }

    function test_binding_rejects_zero_ids() public {
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("contractSetId=0"));
        reg.proposeArtifactBinding(bytes32(0), bArtId);

        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("artifactSetId=0"));
        reg.proposeArtifactBinding(bId, bytes32(0));
    }

    // --- publish + exact read-back ---------------------------------------------------------------

    /// The getters read back EXACTLY what was published, including the string members, and each flow
    /// stamps publishedAt/active authoritatively at execute.
    function test_publish_reads_back_exactly() public {
        // Publish each axis separately (not via _publishLevelB) so `block.timestamp` at the moment of
        // each execute is known and `publishedAt` can be asserted exactly.
        _publishContracts(_contracts());
        uint64 contractsPublishedAt = uint64(block.timestamp);

        ProtocolRegistry.ContractSet memory gotC = reg.getContractSet(bId);
        _assertEqContracts(gotC, _contracts());
        assertTrue(gotC.active, "contract set active at execute");
        assertEq(gotC.publishedAt, contractsPublishedAt, "contract publishedAt stamped at execute");

        _publishArtifacts(_artifacts());

        ProtocolRegistry.ArtifactSet memory gotA = reg.getArtifactSet(bArtId);
        _assertEqArtifacts(gotA, _artifacts());
        assertTrue(gotA.active, "artifact set active at execute");
        assertEq(gotA.publishedAt, uint64(block.timestamp), "artifact publishedAt stamped at execute");
        // The two axes were published at DIFFERENT times — further evidence they are separate writes.
        assertTrue(gotA.publishedAt > gotC.publishedAt, "independent publish timestamps");
    }

    /// The executes IGNORE calldata publishedAt/active on BOTH axes — a publisher cannot back-date or
    /// pre-activate, and a re-proposed record cannot smuggle active=false through execute.
    function test_execute_ignores_calldata_publishedAt_and_active() public {
        ProtocolRegistry.ContractSet memory c = _contracts();
        c.publishedAt = 12345; // lie
        c.active = false; // will be overridden to true
        _publishContracts(c);

        ProtocolRegistry.ContractSet memory gotC = reg.getContractSet(bId);
        assertEq(gotC.publishedAt, uint64(block.timestamp), "publishedAt is block time, not calldata");
        assertTrue(gotC.active, "active forced true");

        ProtocolRegistry.ArtifactSet memory a = _artifacts();
        a.publishedAt = 12345;
        a.active = false;
        _publishArtifacts(a);

        ProtocolRegistry.ArtifactSet memory gotA = reg.getArtifactSet(bArtId);
        assertEq(gotA.publishedAt, uint64(block.timestamp), "publishedAt is block time, not calldata");
        assertTrue(gotA.active, "active forced true");
    }

    // --- timelock enforcement --------------------------------------------------------------------

    function test_default_publish_timelock_enforces_two_days_end_to_end() public {
        assertEq(reg.PUBLISH_TIMELOCK(), reg.DEFAULT_PUBLISH_TIMELOCK(), "safe default selected");
        assertEq(reg.PUBLISH_TIMELOCK(), 2 days, "production delay");

        uint256 proposedAt = block.timestamp;
        vm.startPrank(PUBLISHER);
        reg.proposeContractSet(_contracts());
        reg.proposeArtifactSet(_artifacts());
        reg.proposeArtifactBinding(bId, bArtId);
        vm.stopPrank();

        vm.warp(proposedAt + 2 days - 1);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("timelock"));
        reg.executeContractSet(bId);

        vm.warp(proposedAt + 2 days);
        vm.startPrank(PUBLISHER);
        reg.executeContractSet(bId);
        reg.executeArtifactSet(bArtId);
        reg.executeArtifactBinding(bId);
        vm.stopPrank();

        assertTrue(reg.getContractSet(bId).active, "contract set published");
        assertTrue(reg.getActiveArtifactSet(bId).active, "artifact set bound and published");
    }

    function test_zero_publish_timelock_executes_immediately_end_to_end() public {
        ProtocolRegistry fastRegistry = new ProtocolRegistry(ADMIN, PUBLISHER, 0);
        vm.warp(100);

        vm.startPrank(PUBLISHER);
        fastRegistry.proposeContractSet(_contracts());
        fastRegistry.proposeArtifactSet(_artifacts());
        fastRegistry.proposeArtifactBinding(bId, bArtId);
        assertEq(fastRegistry.contractSetEta(bId), block.timestamp, "contract ETA is immediate");
        assertEq(fastRegistry.artifactSetEta(bArtId), block.timestamp, "artifact ETA is immediate");
        assertEq(fastRegistry.bindingEta(bId), block.timestamp, "binding ETA is immediate");

        fastRegistry.executeContractSet(bId);
        fastRegistry.executeArtifactSet(bArtId);
        fastRegistry.executeArtifactBinding(bId);
        vm.stopPrank();

        assertTrue(fastRegistry.getContractSet(bId).active, "contract set published immediately");
        assertTrue(fastRegistry.getActiveArtifactSet(bId).active, "binding published immediately");
    }

    function test_execute_contract_set_before_timelock_reverts() public {
        vm.prank(PUBLISHER);
        reg.proposeContractSet(_contracts());

        // one second short of the ETA
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK() - 1);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("timelock"));
        reg.executeContractSet(bId);

        // exactly at the ETA it succeeds
        vm.warp(block.timestamp + 1);
        vm.prank(PUBLISHER);
        reg.executeContractSet(bId);
        assertTrue(reg.getContractSet(bId).active);
    }

    function test_execute_artifact_set_before_timelock_reverts() public {
        vm.prank(PUBLISHER);
        reg.proposeArtifactSet(_artifacts());

        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK() - 1);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("timelock"));
        reg.executeArtifactSet(bArtId);

        vm.warp(block.timestamp + 1);
        vm.prank(PUBLISHER);
        reg.executeArtifactSet(bArtId);
        assertTrue(reg.getArtifactSet(bArtId).active);
    }

    function test_execute_without_proposal_reverts() public {
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("none pending"));
        reg.executeContractSet(bId);

        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("none pending"));
        reg.executeArtifactSet(bArtId);

        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("none pending"));
        reg.executeArtifactBinding(bId);
    }

    function test_reproposing_resets_the_timelock() public {
        vm.prank(PUBLISHER);
        reg.proposeContractSet(_contracts());
        uint256 firstEta = reg.contractSetEta(bId);

        // Nearly at the first ETA, re-propose: the ETA must move forward by a fresh full timelock.
        vm.warp(firstEta - 1);
        vm.prank(PUBLISHER);
        reg.proposeContractSet(_contracts());
        assertEq(reg.contractSetEta(bId), block.timestamp + reg.PUBLISH_TIMELOCK(), "eta reset");

        // The old ETA is no longer sufficient.
        vm.warp(firstEta);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("timelock"));
        reg.executeContractSet(bId);
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
        reg.proposeContractSet(_contracts());
    }

    function test_propose_artifact_set_requires_publisher_role() public {
        _expectUnauthorized(OUTSIDER);
        vm.prank(OUTSIDER);
        reg.proposeArtifactSet(_artifacts());
    }

    function test_execute_requires_publisher_role() public {
        vm.prank(PUBLISHER);
        reg.proposeContractSet(_contracts());
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());

        _expectUnauthorized(OUTSIDER);
        vm.prank(OUTSIDER);
        reg.executeContractSet(bId);
    }

    function test_deprecate_requires_publisher_role() public {
        _publishLevelB();

        _expectUnauthorized(OUTSIDER);
        vm.prank(OUTSIDER);
        reg.deprecateContractSet(bId);
    }

    function test_deprecate_artifact_set_requires_publisher_role() public {
        _publishLevelB();

        _expectUnauthorized(OUTSIDER);
        vm.prank(OUTSIDER);
        reg.deprecateArtifactSet(bArtId);
    }

    /// The admin can hand PUBLISHER_ROLE to a fresh key, which can then publish — the governance lever.
    function test_admin_can_grant_publisher_role() public {
        address newPub = address(0xC0FFEE);
        bytes32 role = reg.PUBLISHER_ROLE();
        vm.prank(ADMIN);
        reg.grantRole(role, newPub);

        vm.prank(newPub);
        reg.proposeContractSet(_contracts());
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(newPub);
        reg.executeContractSet(bId);
        assertTrue(reg.getContractSet(bId).active);
    }

    // --- deprecate keeps history -----------------------------------------------------------------

    function test_deprecate_contract_set_flips_active_without_deleting() public {
        _publishLevelB();
        assertTrue(reg.getContractSet(bId).active);

        vm.prank(PUBLISHER);
        reg.deprecateContractSet(bId);

        // Still readable, still enumerated, still fully pinned — only `active` changed.
        ProtocolRegistry.ContractSet memory got = reg.getContractSet(bId);
        assertFalse(got.active, "active flipped false");
        _assertEqContracts(got, _contracts()); // history pinned
        assertEq(reg.contractSetCount(), 1, "deprecate does not remove from the list");
        assertEq(reg.contractSetList(0), bId, "list entry preserved");

        // Deprecating the on-chain axis does not touch the artifact axis or the binding.
        assertTrue(reg.getArtifactSet(bArtId).active, "artifact set untouched by a contract deprecate");
        assertEq(reg.activeArtifactSetOf(bId), bArtId, "binding untouched");
    }

    function test_deprecate_artifact_set_flips_active_without_deleting() public {
        _publishLevelB();

        vm.prank(PUBLISHER);
        reg.deprecateArtifactSet(bArtId);

        ProtocolRegistry.ArtifactSet memory got = reg.getArtifactSet(bArtId);
        assertFalse(got.active, "active flipped false");
        _assertEqArtifacts(got, _artifacts()); // history pinned
        assertEq(reg.artifactSetCount(), 1);
        assertEq(reg.artifactSetList(0), bArtId);

        // And the on-chain axis is unaffected.
        assertTrue(reg.getContractSet(bId).active, "contract set untouched by an artifact deprecate");
    }

    // Deprecate is the EMERGENCY lever, so it must also neutralize anything already in flight: a swap
    // proposed before the compromise was found, whose timelock has since elapsed, must not be executable
    // afterwards to flip the set straight back to active with no fresh review window.
    function test_deprecate_contract_set_cancels_an_in_flight_proposal() public {
        _publishLevelB();

        // A swap for the SAME id is staged and its timelock fully elapses...
        vm.prank(PUBLISHER);
        reg.proposeContractSet(_contracts());
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());

        // ...then the set is found compromised and retired.
        vm.prank(PUBLISHER);
        reg.deprecateContractSet(bId);

        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("none pending"));
        reg.executeContractSet(bId);

        assertFalse(reg.getContractSet(bId).active, "the retired set stays retired");
        assertEq(reg.contractSetEta(bId), 0, "the stale proposal's ETA is cleared");
        // History is untouched — cancelling a PENDING proposal is not deleting the published record.
        assertEq(reg.contractSetCount(), 1);
        _assertEqContracts(reg.getContractSet(bId), _contracts());
    }

    // The same halt on the artifact axis, where re-activation bites harder: deprecate leaves
    // `activeArtifactSetOf` still pointing at the retired set, so flipping `active` back would instantly
    // restore compromised artifacts as the live ones for every contract set bound to it.
    function test_deprecate_artifact_set_cancels_an_in_flight_proposal() public {
        _publishLevelB();

        vm.prank(PUBLISHER);
        reg.proposeArtifactSet(_artifacts());
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());

        vm.prank(PUBLISHER);
        reg.deprecateArtifactSet(bArtId);

        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("none pending"));
        reg.executeArtifactSet(bArtId);

        assertFalse(reg.getArtifactSet(bArtId).active, "the retired artifact set stays retired");
        assertEq(reg.artifactSetEta(bArtId), 0, "the stale proposal's ETA is cleared");
        assertEq(reg.activeArtifactSetOf(bId), bArtId, "the binding still points at it, hence the halt");
        assertEq(reg.artifactSetCount(), 1);
        _assertEqArtifacts(reg.getArtifactSet(bArtId), _artifacts());
    }

    // Re-publishing after a deprecate is still possible — it just costs a FRESH propose and the full
    // timelock, which is the review window the cancellation exists to force.
    function test_republish_after_deprecate_requires_a_fresh_timelock() public {
        _publishLevelB();

        vm.prank(PUBLISHER);
        reg.deprecateContractSet(bId);

        vm.prank(PUBLISHER);
        reg.proposeContractSet(_contracts());

        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("timelock"));
        reg.executeContractSet(bId);

        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        reg.executeContractSet(bId);
        assertTrue(reg.getContractSet(bId).active, "a fresh, fully-timelocked publish re-activates");
    }

    function test_deprecate_unknown_reverts() public {
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("unknown contract set"));
        reg.deprecateContractSet(keccak256("nope"));

        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("unknown artifact set"));
        reg.deprecateArtifactSet(keccak256("nope"));
    }

    // --- enumeration -----------------------------------------------------------------------------

    function test_single_surviving_version_keeps_the_compatibility_key() public view {
        assertEq(bId, keccak256(bytes("dogtag-levelb/1")));
        assertEq(bArtId, keccak256(bytes("dogtag-levelb-artifacts/1")));
    }

    function test_enumeration_has_one_version_on_both_axes() public {
        _publishLevelB();

        assertEq(reg.contractSetCount(), 1, "one contract set");
        assertEq(reg.artifactSetCount(), 1, "one artifact set");
        assertEq(reg.contractSetList(0), bId);
        assertEq(reg.artifactSetList(0), bArtId);
        assertEq(reg.getContractSet(bId).contractSetId, bId);
        assertEq(reg.getArtifactSet(bArtId).artifactSetId, bArtId);
        assertEq(reg.activeArtifactSetOf(bId), bArtId, "both axes are bound");
    }

    /// A swap-republish (same id, new addresses) updates in place and MUST NOT duplicate the list
    /// entry — history is one row per set, updated, never appended twice.
    function test_swap_republish_does_not_duplicate_list_entry() public {
        _publishContracts(_contracts());
        assertEq(reg.contractSetCount(), 1);

        // Contract-only bump (§5.5): same set id, new registry address, same VK.
        ProtocolRegistry.ContractSet memory c2 = _contracts();
        c2.verificationRegistry = address(0xDEAD);
        _publishContracts(c2);

        assertEq(reg.contractSetCount(), 1, "swap-republish does not grow the list");
        assertEq(reg.getContractSet(bId).verificationRegistry, address(0xDEAD), "updated in place");
    }

    // --- getters fail-closed ---------------------------------------------------------------------

    function test_unknown_ids_revert() public {
        vm.expectRevert(bytes("unknown contract set"));
        reg.getContractSet(keccak256("dogtag-levelc/9"));

        vm.expectRevert(bytes("unknown artifact set"));
        reg.getArtifactSet(keccak256("dogtag-levelc-artifacts/9"));

        vm.expectRevert(bytes("unknown contract set"));
        reg.resolve(keccak256("dogtag-levelc/9"));
    }

    function test_pending_getters_revert_when_nothing_pending() public {
        vm.expectRevert(bytes("none pending"));
        reg.getPendingContractSet(bId);

        vm.expectRevert(bytes("none pending"));
        reg.getPendingArtifactSet(bArtId);

        vm.expectRevert(bytes("none pending"));
        reg.getPendingBinding(bId);
    }

    // --- propose input validation ----------------------------------------------------------------

    function test_propose_rejects_zero_contractSetId() public {
        ProtocolRegistry.ContractSet memory c = _contracts();
        c.contractSetId = bytes32(0);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("contractSetId=0"));
        reg.proposeContractSet(c);
    }

    function test_propose_rejects_zero_artifactSetId() public {
        ProtocolRegistry.ArtifactSet memory a = _artifacts();
        a.artifactSetId = bytes32(0);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("artifactSetId=0"));
        reg.proposeArtifactSet(a);
    }

    function test_propose_rejects_zero_trio_or_verifier() public {
        ProtocolRegistry.ContractSet memory c = _contracts();
        c.verifier = address(0);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("zero trio/verifier"));
        reg.proposeContractSet(c);
    }

    function test_propose_rejects_zero_circuitId() public {
        ProtocolRegistry.ContractSet memory c = _contracts();
        c.circuitId = bytes32(0);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("circuitId=0"));
        reg.proposeContractSet(c);
    }

    function test_propose_rejects_zero_zkey_pin() public {
        ProtocolRegistry.ArtifactSet memory a = _artifacts();
        a.zkeySha256 = bytes32(0);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("zkeySha256=0"));
        reg.proposeArtifactSet(a);
    }

    /// The graph pin is DELIBERATELY allowed to be 0 (unpinned — the .graph is not committed), unlike
    /// the mandatory zkey pin. Publishing with witnessMobileSha256 == 0 must succeed.
    function test_propose_allows_unpinned_graph() public {
        ProtocolRegistry.ArtifactSet memory a = _artifacts();
        assertEq(a.witnessMobileSha256, bytes32(0), "fixture leaves the graph unpinned");
        _publishArtifacts(a);
        assertEq(reg.getArtifactSet(bArtId).witnessMobileSha256, bytes32(0));
    }

    // --- frozen pins verified against the committed artifacts -------------------------------------

    /// The `witnessServerWasmSha256` each artifact set publishes IS the SHA-256 of the committed
    /// `circuits/build/*.wasm` — hashed here in-EVM (the ~4 MB wasm fits; the 11-64 MB r1cs/zkey do
    /// not — `vm.readFileBinary` MemoryOOGs — so those pins are file-verified by the Rust
    /// `dogtag-prover-rs` `*_descriptor_pins_match_the_real_artifacts` tests, which stream-hash them).
    function test_wasm_pins_match_committed_artifacts() public view {
        assertEq(
            sha256(vm.readFileBinary("../circuits/build/consent_js/consent.wasm")),
            _artifacts().witnessServerWasmSha256,
            "wasm pin != committed consent.wasm"
        );
    }

    /// Guard the frozen zkey/r1cs pins as explicit non-zero constants (the values the Rust tests hash
    /// the committed files against). A pin silently zeroed or swapped in the fixture fails here.
    function test_frozen_zkey_and_r1cs_pins_are_the_expected_constants() public pure {
        ProtocolRegistry.ArtifactSet memory current = _artifacts();
        assertEq(current.zkeySha256, ZKEY_SHA256);
        assertEq(current.witnessServerR1csSha256, R1CS_SHA256);
    }
}
