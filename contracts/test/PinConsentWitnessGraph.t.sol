// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";
import {PinConsentWitnessGraph} from "../script/PinConsentWitnessGraph.s.sol";
import {ProtocolVersions} from "../script/ProtocolVersions.sol";

/// @notice Pins every revert arm of `PinConsentWitnessGraph.requireOnlyTheGraphPinMoves`.
///
/// That guard is the only thing keeping the script inside its authorization: re-publishing an artifact
/// set restates ALL of its fields, so a stale env var would otherwise rewrite a pin or the base URL
/// under cover of "pinning the graph", and a rotation would masquerade as a first pin.
///
/// # The arms fire IN ORDER, which is what these tests must respect
///
/// published -> active -> not-already-pinned -> non-empty next -> the five "would change" arms. A test
/// for a later arm that leaves an earlier one unsatisfied reverts for the wrong reason and proves
/// nothing, so each case below publishes a set that satisfies every PRIOR arm and moves exactly one
/// thing. The exact revert string is asserted for the same reason - a bare `expectRevert` would pass on
/// whichever arm happened to fire.
contract PinConsentWitnessGraphTest is Test {
    ProtocolRegistry internal reg;
    PinConsentWitnessGraph internal script;

    address internal admin;
    address internal publisher;
    bytes32 internal id;

    bytes32 internal constant ZKEY = bytes32(uint256(0xA11CE));
    bytes32 internal constant R1CS = bytes32(uint256(0xCAFE));
    bytes32 internal constant WASM = bytes32(uint256(0xD06));
    bytes32 internal constant GRAPH = bytes32(uint256(0x94A94));
    string internal constant BASE_URL = "https://artifacts.dogtag.test/levelb1";

    function setUp() public {
        admin = makeAddr("admin");
        publisher = makeAddr("publisher");
        reg = new ProtocolRegistry(admin, publisher, 0);
        script = new PinConsentWitnessGraph();
        id = ProtocolVersions.levelBArtifactsId();
    }

    // --- helpers ----------------------------------------------------------------------------------

    /// The set as it stands on chain before a pin: every field final EXCEPT the graph.
    function _publishedUnpinned() internal returns (ProtocolRegistry.ArtifactSet memory) {
        ProtocolRegistry.ArtifactSet memory a =
            ProtocolVersions.levelBArtifacts(ZKEY, bytes32(0), R1CS, WASM, BASE_URL);
        _publish(a);
        return a;
    }

    /// What the script would build from a correctly-configured environment.
    function _next() internal pure returns (ProtocolRegistry.ArtifactSet memory) {
        return ProtocolVersions.levelBArtifacts(ZKEY, GRAPH, R1CS, WASM, BASE_URL);
    }

    function _publish(ProtocolRegistry.ArtifactSet memory a) internal {
        vm.startPrank(publisher);
        reg.proposeArtifactSet(a);
        reg.executeArtifactSet(a.artifactSetId);
        vm.stopPrank();
    }

    // --- the guard admits exactly one operation ---------------------------------------------------

    function test_guard_accepts_pinning_an_otherwise_identical_set() public {
        _publishedUnpinned();
        script.requireOnlyTheGraphPinMoves(reg, id, _next());
    }

    // --- arm 1: the set must already be published --------------------------------------------------

    /// Nothing published, so the auto-getter answers a zeroed record and the script's OWN refusal
    /// fires. Reading `getArtifactSet` first would surface the registry's "unknown artifact set"
    /// instead, which reads as a chain fault rather than an operator one.
    function test_guard_refuses_an_unpublished_set() public {
        vm.expectRevert(bytes("artifact set is not published - this script only re-publishes"));
        script.requireOnlyTheGraphPinMoves(reg, id, _next());
    }

    // --- arm 2: the set must be active -------------------------------------------------------------

    /// Deprecation is the emergency lever. Re-publishing flips `active` back to true, so pinning a
    /// deprecated set would silently un-retire it - a separate decision with a separate authorization.
    function test_guard_refuses_a_deprecated_set() public {
        _publishedUnpinned();
        vm.prank(publisher);
        reg.deprecateArtifactSet(id);

        vm.expectRevert(bytes("artifact set is deprecated - re-activating is a separate decision"));
        script.requireOnlyTheGraphPinMoves(reg, id, _next());
    }

    // --- arm 3: the graph must currently be UNPINNED (no rotation) ---------------------------------

    /// The property that makes this script structurally incapable of a rotation. Moving an EXISTING
    /// pin changes which bytes are authoritative; that needs the full runbook, not this two-tx script.
    function test_guard_refuses_a_rotation_when_the_graph_is_already_pinned() public {
        _publish(ProtocolVersions.levelBArtifacts(ZKEY, GRAPH, R1CS, WASM, BASE_URL));

        ProtocolRegistry.ArtifactSet memory next = _next();
        next.witnessMobileSha256 = bytes32(uint256(0xBEEF));

        vm.expectRevert(bytes("graph already pinned - a change here is a ROTATION"));
        script.requireOnlyTheGraphPinMoves(reg, id, next);
    }

    // --- arm 4: the incoming pin must be non-empty -------------------------------------------------

    /// An unset `CONSENT_WITNESS_MOBILE_SHA256` reads as `bytes32(0)`, which would re-publish the set
    /// unchanged while reporting success - the pin "done" and still absent.
    function test_guard_refuses_an_empty_graph_pin() public {
        _publishedUnpinned();

        ProtocolRegistry.ArtifactSet memory next = _next();
        next.witnessMobileSha256 = bytes32(0);

        vm.expectRevert(bytes("refusing to publish an empty graph pin"));
        script.requireOnlyTheGraphPinMoves(reg, id, next);
    }

    // --- arms 5-9: every other field is a byte-for-byte restatement --------------------------------

    function test_guard_refuses_a_changed_zkey_pin() public {
        _publishedUnpinned();

        ProtocolRegistry.ArtifactSet memory next = _next();
        next.zkeySha256 = bytes32(uint256(0xBADC0DE));

        vm.expectRevert(bytes("zkey pin would change"));
        script.requireOnlyTheGraphPinMoves(reg, id, next);
    }

    function test_guard_refuses_a_changed_r1cs_pin() public {
        _publishedUnpinned();

        ProtocolRegistry.ArtifactSet memory next = _next();
        next.witnessServerR1csSha256 = bytes32(uint256(0xBADC0DE));

        vm.expectRevert(bytes("r1cs pin would change"));
        script.requireOnlyTheGraphPinMoves(reg, id, next);
    }

    function test_guard_refuses_a_changed_wasm_pin() public {
        _publishedUnpinned();

        ProtocolRegistry.ArtifactSet memory next = _next();
        next.witnessServerWasmSha256 = bytes32(uint256(0xBADC0DE));

        vm.expectRevert(bytes("wasm pin would change"));
        script.requireOnlyTheGraphPinMoves(reg, id, next);
    }

    function test_guard_refuses_a_changed_artifact_base_url() public {
        _publishedUnpinned();

        ProtocolRegistry.ArtifactSet memory next =
            ProtocolVersions.levelBArtifacts(ZKEY, GRAPH, R1CS, WASM, "https://elsewhere.example/levelb1");

        vm.expectRevert(bytes("artifactBaseUrl would change"));
        script.requireOnlyTheGraphPinMoves(reg, id, next);
    }

    /// `ProtocolVersions.levelBArtifacts` hardcodes `minAppVersion`, so the DIVERGENCE has to be
    /// staged on the published side: a set published at an older floor, pinned against the helper's.
    function test_guard_refuses_a_changed_min_app_version() public {
        ProtocolRegistry.ArtifactSet memory published =
            ProtocolVersions.levelBArtifacts(ZKEY, bytes32(0), R1CS, WASM, BASE_URL);
        published.minAppVersion = "1.3.0";
        _publish(published);

        ProtocolRegistry.ArtifactSet memory next = _next();
        assertTrue(
            keccak256(bytes(next.minAppVersion)) != keccak256(bytes(published.minAppVersion)),
            "fixture must actually diverge or this test cannot fail"
        );

        vm.expectRevert(bytes("minAppVersion would change"));
        script.requireOnlyTheGraphPinMoves(reg, id, next);
    }

    // --- the two-phase re-run, under a REAL timelock ----------------------------------------------

    /// A non-zero timelock makes this a two-run operation, and the operator finishes it by re-running
    /// the same command. `proposeArtifactSet` recomputes `artifactSetEta = block.timestamp +
    /// PUBLISH_TIMELOCK`, so a second run that re-proposed would silently restart the whole delay and
    /// never execute - the pin would stay perpetually one run away. The second run must EXECUTE.
    function test_rerun_after_the_eta_executes_instead_of_restarting_the_delay() public {
        (PinConsentWitnessGraph timelocked, ProtocolRegistry timelockedReg) = _timelockedFixture();

        timelocked.run(); // run 1: proposes only
        uint256 eta = timelockedReg.artifactSetEta(id);
        assertGt(eta, block.timestamp, "run 1 must stage a proposal, not publish");
        assertEq(timelockedReg.getArtifactSet(id).witnessMobileSha256, bytes32(0), "not pinned yet");

        vm.warp(eta);
        timelocked.run(); // run 2: executes the ripened proposal

        assertEq(timelockedReg.artifactSetEta(id), 0, "the proposal must be consumed, not re-staged");
        assertEq(timelockedReg.getArtifactSet(id).witnessMobileSha256, GRAPH, "run 2 must publish the pin");
        assertTrue(timelockedReg.getArtifactSet(id).active, "the set must remain active");
    }

    /// Re-running INSIDE the window must abort without broadcasting. Re-proposing there is what pushes
    /// the ETA further out on every attempt; refusing is what makes the wait finite.
    function test_rerun_inside_the_window_refuses_rather_than_re_proposing() public {
        (PinConsentWitnessGraph timelocked, ProtocolRegistry timelockedReg) = _timelockedFixture();

        timelocked.run();
        uint256 eta = timelockedReg.artifactSetEta(id);

        vm.warp(eta - 1);
        vm.expectRevert(bytes("proposal pending - re-run after the printed ETA to execute"));
        timelocked.run();

        assertEq(timelockedReg.artifactSetEta(id), eta, "the staged ETA must not move");
    }

    /// A registry with a REAL delay, published unpinned, with the script's env wired to pin it.
    ///
    /// `PUBLISHER_ROLE` goes to `DEFAULT_SENDER`, not to this test contract: `run()` publishes inside
    /// `vm.startBroadcast()`, so the calls arrive from forge's broadcast sender however the test was
    /// invoked. Granting the role to `address(this)` compiles and then fails as `AccessControl…`,
    /// which reads as a role bug rather than the harness detail it is.
    function _timelockedFixture() internal returns (PinConsentWitnessGraph, ProtocolRegistry) {
        ProtocolRegistry timelockedReg = new ProtocolRegistry(admin, DEFAULT_SENDER, 2 days);
        ProtocolRegistry.ArtifactSet memory a =
            ProtocolVersions.levelBArtifacts(ZKEY, bytes32(0), R1CS, WASM, BASE_URL);
        vm.startPrank(DEFAULT_SENDER);
        timelockedReg.proposeArtifactSet(a);
        vm.warp(block.timestamp + 2 days);
        timelockedReg.executeArtifactSet(id);
        vm.stopPrank();

        vm.setEnv("PROTOCOL_REGISTRY", vm.toString(address(timelockedReg)));
        vm.setEnv("CONSENT_ZKEY_SHA256", vm.toString(ZKEY));
        vm.setEnv("CONSENT_WITNESS_MOBILE_SHA256", vm.toString(GRAPH));
        vm.setEnv("CONSENT_R1CS_SHA256", vm.toString(R1CS));
        vm.setEnv("CONSENT_WASM_SHA256", vm.toString(WASM));
        vm.setEnv("DOGTAG_ARTIFACTS_URL", BASE_URL);

        return (new PinConsentWitnessGraph(), timelockedReg);
    }
}
