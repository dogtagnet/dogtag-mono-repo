// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";
import {ProtocolRegistryV2, MIN_PUBLISH_TIMELOCK_SECONDS_V2} from "../src/ProtocolRegistryV2.sol";
import {ProtocolVersions} from "../script/ProtocolVersions.sol";
import {ProtocolVersionsV2} from "../script/ProtocolVersionsV2.sol";

/// @notice The generation-2 discovery anchor. Three things here are NOT re-tests of generation 1 and are
/// what this suite exists for:
///
///   * `--- The timelock floor ---` — a zero-delay registry is unrepresentable, enforced by the
///     CONSTRUCTOR rather than by a deploy script, because the value is immutable and the mistake would
///     be unfixable.
///   * `--- The wider record cannot be misdecoded ---` — the renamed getter's selector, and the exact
///     10-word return arity, are pinned. Those two facts are what stop a generation-1 client reading
///     `providerRegistry` as `circuitId`.
///   * `--- factory and rootIndex are separate members ---` — generation 1's
///     `factory == verificationRegistry.rootIndex()` invariant is GONE, and collapsing the two back into
///     one member is the mistake that loses every historical root.
///
/// The rest mirrors generation 1's suite so a reviewer can diff behaviour rather than re-derive it.
contract ProtocolRegistryV2Test is Test {
    ProtocolRegistryV2 internal reg;

    address internal constant ADMIN = address(0xA11CE);
    address internal constant PUBLISHER = address(0xB0B);
    address internal constant OUTSIDER = address(0xBAD);

    address internal constant FACTORY_V2 = address(0xFACADE2);
    address internal constant VERIFICATION_REGISTRY = address(0xC0DE);
    address internal constant SBT = address(0x5B7);
    address internal constant VERIFIER = address(0xBEEF);
    address internal constant PROVIDER_REGISTRY = address(0x9309);
    address internal constant ROOT_INDEX = address(0x120127E4); // the CloneProvenanceRouter

    bytes32 internal constant ZKEY_SHA256 =
        0xf83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868;
    bytes32 internal constant GRAPH_SHA256 =
        0x2f74d26b2f0d47d0d1b3f2f8f0a1a5c3d5b7e9a1c3e5f70921436587a9cbedf7;
    bytes32 internal constant R1CS_SHA256 =
        0x828e2923a159b04f2de421d4b447f8c85356677f4f83a5af55b42eb2b4f9b6b7;
    bytes32 internal constant WASM_SHA256 =
        0x482debcff5a4325c008dd00e4476bba011d0a706da955e3129d114f996a913e6;

    string internal constant ARTIFACT_URL = "https://artifacts.dogtag.io/consent1";
    string internal constant MIN_APP = "1.6.0";

    bytes32 internal v2Id;
    bytes32 internal artId;

    function setUp() public {
        reg = new ProtocolRegistryV2(ADMIN, PUBLISHER, 2 days);
        v2Id = ProtocolVersionsV2.levelBV2Id();
        artId = ProtocolVersionsV2.levelBArtifactsId();
    }

    // --- helpers ---------------------------------------------------------------------------------

    function _discovery() internal pure returns (ProtocolRegistryV2.DiscoverySet memory) {
        return ProtocolVersionsV2.levelBV2Discovery(
            FACTORY_V2, VERIFICATION_REGISTRY, SBT, VERIFIER, PROVIDER_REGISTRY, ROOT_INDEX
        );
    }

    function _artifacts() internal pure returns (ProtocolRegistryV2.ArtifactSet memory) {
        return ProtocolVersionsV2.levelBArtifacts(
            ZKEY_SHA256, GRAPH_SHA256, R1CS_SHA256, WASM_SHA256, ARTIFACT_URL, MIN_APP
        );
    }

    function _publishDiscovery(ProtocolRegistryV2.DiscoverySet memory d) internal {
        vm.prank(PUBLISHER);
        reg.proposeDiscoverySet(d);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        reg.executeDiscoverySet(d.discoverySetId);
    }

    function _publishArtifacts(ProtocolRegistryV2.ArtifactSet memory a) internal {
        vm.prank(PUBLISHER);
        reg.proposeArtifactSet(a);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        reg.executeArtifactSet(a.artifactSetId);
    }

    function _bind(bytes32 discoverySetId, bytes32 artifactSetId) internal {
        vm.prank(PUBLISHER);
        reg.proposeArtifactBinding(discoverySetId, artifactSetId);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        reg.executeArtifactBinding(discoverySetId);
    }

    function _publishGenerationTwo() internal {
        _publishDiscovery(_discovery());
        _publishArtifacts(_artifacts());
        _bind(v2Id, artId);
    }

    // =============================================================================================
    // --- The timelock floor ---
    // =============================================================================================

    /// @notice The headline. The live generation-1 registry carries `PUBLISH_TIMELOCK == 0` and the value
    /// is `immutable`, so nothing short of a redeploy can fix it. Here a zero is refused by the
    /// CONSTRUCTOR, which is what makes the fix independent of whether anyone remembers to use the deploy
    /// script.
    function test_a_zero_publish_timelock_cannot_be_deployed_at_all() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                ProtocolRegistryV2.PublishTimelockBelowFloor.selector, 0, MIN_PUBLISH_TIMELOCK_SECONDS_V2
            )
        );
        new ProtocolRegistryV2(ADMIN, PUBLISHER, 0);

        // ...and generation 1 accepts exactly that value, which is the defect being closed. Asserting it
        // here keeps the contrast honest rather than asserted in prose.
        ProtocolRegistry generationOne = new ProtocolRegistry(ADMIN, PUBLISHER, 0);
        assertEq(generationOne.PUBLISH_TIMELOCK(), 0, "generation 1 permits the zero this refuses");
    }

    /// @notice The floor is a real boundary, not a `!= 0` check wearing a floor's name: one second below
    /// it reverts and the floor itself is accepted. A `> 0` guard would pass a one-second delay, which is
    /// a timelock that exists only in the getter.
    function test_the_floor_is_a_boundary_not_a_nonzero_check() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                ProtocolRegistryV2.PublishTimelockBelowFloor.selector, 1, MIN_PUBLISH_TIMELOCK_SECONDS_V2
            )
        );
        new ProtocolRegistryV2(ADMIN, PUBLISHER, 1);

        vm.expectRevert(
            abi.encodeWithSelector(
                ProtocolRegistryV2.PublishTimelockBelowFloor.selector,
                MIN_PUBLISH_TIMELOCK_SECONDS_V2 - 1,
                MIN_PUBLISH_TIMELOCK_SECONDS_V2
            )
        );
        new ProtocolRegistryV2(ADMIN, PUBLISHER, MIN_PUBLISH_TIMELOCK_SECONDS_V2 - 1);

        ProtocolRegistryV2 atFloor = new ProtocolRegistryV2(ADMIN, PUBLISHER, MIN_PUBLISH_TIMELOCK_SECONDS_V2);
        assertEq(atFloor.PUBLISH_TIMELOCK(), 1 hours, "the floor itself must be deployable");
        assertEq(atFloor.MIN_PUBLISH_TIMELOCK(), 1 hours);
        assertEq(atFloor.DEFAULT_PUBLISH_TIMELOCK(), 2 days, "the production default is unchanged");
    }

    /// @notice The delay actually delays: an execute before the ETA reverts on all three writes, and the
    /// same call succeeds after it. Without this, the floor would only prove a number was stored.
    function test_the_configured_delay_gates_every_write() public {
        ProtocolRegistryV2.DiscoverySet memory d = _discovery();
        ProtocolRegistryV2.ArtifactSet memory a = _artifacts();

        vm.startPrank(PUBLISHER);
        reg.proposeDiscoverySet(d);
        reg.proposeArtifactSet(a);
        reg.proposeArtifactBinding(v2Id, artId);

        vm.expectRevert(bytes("timelock"));
        reg.executeDiscoverySet(v2Id);
        vm.expectRevert(bytes("timelock"));
        reg.executeArtifactSet(artId);
        vm.expectRevert(bytes("timelock"));
        reg.executeArtifactBinding(v2Id);

        // One second before the ETA is still too early — the boundary is `>=`, not "roughly".
        vm.warp(reg.discoverySetEta(v2Id) - 1);
        vm.expectRevert(bytes("timelock"));
        reg.executeDiscoverySet(v2Id);

        vm.warp(reg.discoverySetEta(v2Id));
        reg.executeDiscoverySet(v2Id);
        reg.executeArtifactSet(artId);
        reg.executeArtifactBinding(v2Id);
        vm.stopPrank();

        assertTrue(reg.getDiscoverySet(v2Id).active);
        assertEq(reg.getActiveArtifactSet(v2Id).artifactSetId, artId);
    }

    // =============================================================================================
    // --- The wider record cannot be misdecoded ---
    // =============================================================================================

    /// @notice A generation-1 client pointed at this registry must REVERT, not misdecode.
    ///
    /// The renamed getter is the guard. `getContractSet(bytes32)` and `getDiscoverySet(bytes32)` are
    /// different selectors, so the old call finds no function and the fallback-less contract reverts.
    /// Had the name been kept, the call would have dispatched and the client would have decoded the
    /// first 8 of 10 words — reading `providerRegistry` where it expected `circuitId` and `publishedAt`
    /// where it expected `active`, which decodes as a plausible live record. That is the same
    /// identical-shape/different-semantics trap as the two `recordVerificationZK` arities sharing one
    /// selector.
    function test_a_generation_one_client_reverts_instead_of_misdecoding() public {
        _publishGenerationTwo();

        assertTrue(
            ProtocolRegistry.getContractSet.selector != ProtocolRegistryV2.getDiscoverySet.selector,
            "the renamed getter must not share generation 1's selector"
        );
        assertTrue(
            ProtocolRegistry.resolve.selector != ProtocolRegistryV2.resolveDiscovery.selector,
            "the renamed resolver must not share generation 1's selector"
        );

        // The literal generation-1 call, byte for byte, against this registry.
        (bool ok,) =
            address(reg).staticcall(abi.encodeWithSelector(ProtocolRegistry.getContractSet.selector, v2Id));
        assertFalse(ok, "a generation-1 getContractSet call must fail closed here");

        // The artifact axis is UNCHANGED, so its selectors are deliberately shared and a generation-1
        // artifact decoder stays correct. Asserting this keeps the rename scoped to the axis that moved.
        assertEq(
            ProtocolRegistry.getActiveArtifactSet.selector,
            ProtocolRegistryV2.getActiveArtifactSet.selector,
            "the unchanged axis must keep its selector"
        );
        (bool artifactOk, bytes memory artifactRet) = address(reg)
            .staticcall(abi.encodeWithSelector(ProtocolRegistry.getActiveArtifactSet.selector, v2Id));
        assertTrue(artifactOk, "a generation-1 artifact read must still work");
        assertGt(artifactRet.length, 0);
    }

    /// @notice The record's ARITY is pinned, which is what pins the field list itself.
    ///
    /// Ten words: eight from the members generation 1 had, plus `providerRegistry` and `rootIndex`. An
    /// eleventh member — say a protocol-wide `providerDirectory` or `serviceDomainResolver` — would fail
    /// here, and it should: the authority core allowlists many resolvers per kind and each
    /// provider/service selects its own, so a single protocol-wide resolver address would be a published
    /// falsehood. `providerRegistry` is how a consumer reaches the real resolution root.
    function test_the_discovery_record_is_exactly_ten_words() public {
        _publishGenerationTwo();

        (bool ok, bytes memory ret) =
            address(reg).staticcall(abi.encodeWithSelector(ProtocolRegistryV2.getDiscoverySet.selector, v2Id));
        assertTrue(ok);
        assertEq(ret.length, 32 * 10, "a static 10-member tuple decodes as 10 inline words");

        // Generation 1's is 8 — the arity difference is the thing a width-checking client discriminates on.
        ProtocolRegistry generationOne = new ProtocolRegistry(ADMIN, PUBLISHER, 2 days);
        vm.startPrank(PUBLISHER);
        generationOne.proposeContractSet(
            ProtocolVersions.levelBContracts(FACTORY_V2, VERIFICATION_REGISTRY, SBT, VERIFIER)
        );
        vm.warp(block.timestamp + 2 days);
        generationOne.executeContractSet(ProtocolVersions.levelBId());
        vm.stopPrank();
        (bool ok1, bytes memory ret1) = address(generationOne)
            .staticcall(
                abi.encodeWithSelector(ProtocolRegistry.getContractSet.selector, ProtocolVersions.levelBId())
            );
        assertTrue(ok1);
        assertEq(ret1.length, 32 * 8, "generation 1's record is 8 words");
    }

    // =============================================================================================
    // --- factory and rootIndex are separate members ---
    // =============================================================================================

    /// @notice Generation 1 documented `factory == verificationRegistry.rootIndex()`. In generation 2 the
    /// factory is where a clone comes from and the root index is the provenance router, so the record
    /// carries BOTH and they are different addresses. Collapsing them — publishing the factory in the
    /// root-index slot — resolves generation-2 roots only and loses every historical one, which is the
    /// exact failure the router was built to prevent.
    function test_factory_and_root_index_are_distinct_published_members() public {
        _publishGenerationTwo();
        ProtocolRegistryV2.DiscoverySet memory got = reg.getDiscoverySet(v2Id);

        assertEq(got.factory, FACTORY_V2);
        assertEq(got.rootIndex, ROOT_INDEX);
        assertTrue(got.factory != got.rootIndex, "the two slots are different contracts in generation 2");
        assertEq(got.providerRegistry, PROVIDER_REGISTRY);
        assertTrue(
            got.providerRegistry != got.verificationRegistry,
            "the authority core is not the verification registry"
        );
    }

    /// @notice Every member reads back exactly, including the two the auto-getter would let a reviewer
    /// overlook, and `publishedAt`/`active` are stamped by the contract rather than taken from calldata.
    function test_publish_reads_back_exactly() public {
        ProtocolRegistryV2.DiscoverySet memory want = _discovery();
        // Try to pre-activate and back-date: both must be ignored.
        want.active = true;
        want.publishedAt = 1;

        vm.warp(1_800_000_000);
        _publishDiscovery(want);

        ProtocolRegistryV2.DiscoverySet memory got = reg.getDiscoverySet(v2Id);
        assertEq(got.discoverySetId, v2Id);
        assertEq(got.factory, FACTORY_V2);
        assertEq(got.verificationRegistry, VERIFICATION_REGISTRY);
        assertEq(got.sbt, SBT);
        assertEq(got.verifier, VERIFIER);
        assertEq(got.providerRegistry, PROVIDER_REGISTRY);
        assertEq(got.rootIndex, ROOT_INDEX);
        assertEq(got.circuitId, keccak256("consent.circom/DogTagConsent(6)"));
        assertEq(got.publishedAt, uint64(block.timestamp), "publishedAt is stamped, not supplied");
        assertTrue(got.active);

        _publishArtifacts(_artifacts());
        ProtocolRegistryV2.ArtifactSet memory a = reg.getArtifactSet(artId);
        assertEq(a.artifactSetId, artId);
        assertEq(a.zkeySha256, ZKEY_SHA256);
        assertEq(a.witnessMobileSha256, GRAPH_SHA256);
        assertEq(a.witnessServerR1csSha256, R1CS_SHA256);
        assertEq(a.witnessServerWasmSha256, WASM_SHA256);
        assertEq(a.artifactBaseUrl, ARTIFACT_URL, "the string members the auto-getter omits");
        assertEq(a.minAppVersion, MIN_APP);
        assertTrue(a.active);
    }

    /// @notice Each address member is separately required, with its own reason. A single lumped
    /// "zero address" check would tell an operator nothing about WHICH transcription slipped.
    function test_every_component_address_is_required() public {
        vm.startPrank(PUBLISHER);

        ProtocolRegistryV2.DiscoverySet memory noProvider = _discovery();
        noProvider.providerRegistry = address(0);
        vm.expectRevert(bytes("zero providerRegistry"));
        reg.proposeDiscoverySet(noProvider);

        ProtocolRegistryV2.DiscoverySet memory noRootIndex = _discovery();
        noRootIndex.rootIndex = address(0);
        vm.expectRevert(bytes("zero rootIndex"));
        reg.proposeDiscoverySet(noRootIndex);

        ProtocolRegistryV2.DiscoverySet memory noFactory = _discovery();
        noFactory.factory = address(0);
        vm.expectRevert(bytes("zero trio/verifier"));
        reg.proposeDiscoverySet(noFactory);

        ProtocolRegistryV2.DiscoverySet memory noRegistry = _discovery();
        noRegistry.verificationRegistry = address(0);
        vm.expectRevert(bytes("zero trio/verifier"));
        reg.proposeDiscoverySet(noRegistry);

        ProtocolRegistryV2.DiscoverySet memory noSbt = _discovery();
        noSbt.sbt = address(0);
        vm.expectRevert(bytes("zero trio/verifier"));
        reg.proposeDiscoverySet(noSbt);

        ProtocolRegistryV2.DiscoverySet memory noVerifier = _discovery();
        noVerifier.verifier = address(0);
        vm.expectRevert(bytes("zero trio/verifier"));
        reg.proposeDiscoverySet(noVerifier);

        ProtocolRegistryV2.DiscoverySet memory noId = _discovery();
        noId.discoverySetId = bytes32(0);
        vm.expectRevert(bytes("discoverySetId=0"));
        reg.proposeDiscoverySet(noId);

        ProtocolRegistryV2.DiscoverySet memory noCircuit = _discovery();
        noCircuit.circuitId = bytes32(0);
        vm.expectRevert(bytes("circuitId=0"));
        reg.proposeDiscoverySet(noCircuit);

        ProtocolRegistryV2.ArtifactSet memory noZkey = _artifacts();
        noZkey.zkeySha256 = bytes32(0);
        vm.expectRevert(bytes("zkeySha256=0"));
        reg.proposeArtifactSet(noZkey);

        vm.stopPrank();
    }

    /// @notice An unpinned witness graph is still publishable: a deployment may publish its set before it
    /// has a graph identity to attest, and that window must be representable. Only the zkey is mandatory.
    function test_an_unpinned_graph_is_publishable() public {
        ProtocolRegistryV2.ArtifactSet memory a = _artifacts();
        a.witnessMobileSha256 = bytes32(0);
        _publishArtifacts(a);
        assertEq(reg.getArtifactSet(artId).witnessMobileSha256, bytes32(0));
    }

    // =============================================================================================
    // --- R-5: the two axes still rotate independently ---
    // =============================================================================================

    /// @notice An artifact rotation writes no discovery member — not even `publishedAt`, which is the
    /// on-chain provenance of the trio and would be silently rewritten by a republish.
    function test_an_artifact_rotation_leaves_the_discovery_set_untouched() public {
        _publishGenerationTwo();
        ProtocolRegistryV2.DiscoverySet memory before = reg.getDiscoverySet(v2Id);

        bytes32 rotatedId = keccak256("dogtag-levelb-artifacts/2");
        ProtocolRegistryV2.ArtifactSet memory rotated = _artifacts();
        rotated.artifactSetId = rotatedId;
        rotated.zkeySha256 = bytes32(uint256(0xC0FFEE));
        rotated.minAppVersion = "1.7.0";
        _publishArtifacts(rotated);
        _bind(v2Id, rotatedId);

        assertEq(reg.getActiveArtifactSet(v2Id).zkeySha256, bytes32(uint256(0xC0FFEE)));
        assertEq(reg.getActiveArtifactSet(v2Id).minAppVersion, "1.7.0");

        ProtocolRegistryV2.DiscoverySet memory after_ = reg.getDiscoverySet(v2Id);
        assertEq(after_.factory, before.factory);
        assertEq(after_.verificationRegistry, before.verificationRegistry);
        assertEq(after_.sbt, before.sbt);
        assertEq(after_.verifier, before.verifier);
        assertEq(after_.providerRegistry, before.providerRegistry);
        assertEq(after_.rootIndex, before.rootIndex);
        assertEq(after_.circuitId, before.circuitId);
        assertEq(after_.publishedAt, before.publishedAt, "the trio's provenance stamp must not move");
        assertEq(reg.discoverySetCount(), 1, "no new discovery set was created");
    }

    /// @notice ...and the converse: rotating the on-chain set writes no artifact member and leaves the
    /// binding in place, so no app is forced to re-fetch anything.
    function test_a_discovery_rotation_leaves_the_artifact_axis_untouched() public {
        _publishGenerationTwo();
        ProtocolRegistryV2.ArtifactSet memory before = reg.getArtifactSet(artId);

        ProtocolRegistryV2.DiscoverySet memory moved = _discovery();
        moved.verificationRegistry = address(0xDEAD01);
        moved.rootIndex = address(0xDEAD02);
        _publishDiscovery(moved);

        assertEq(reg.getDiscoverySet(v2Id).verificationRegistry, address(0xDEAD01));
        ProtocolRegistryV2.ArtifactSet memory after_ = reg.getArtifactSet(artId);
        assertEq(after_.zkeySha256, before.zkeySha256);
        assertEq(after_.minAppVersion, before.minAppVersion);
        assertEq(after_.publishedAt, before.publishedAt);
        assertEq(reg.activeArtifactSetOf(v2Id), artId, "the binding survives an on-chain rotation");
        assertEq(reg.artifactSetCount(), 1);
    }

    /// @notice The two axes cannot collide: the discovery key and the artifact key are different strings
    /// in different namespaces, so no id can serve as both.
    function test_the_two_axes_do_not_share_a_keyspace() public {
        _publishGenerationTwo();
        assertTrue(v2Id != artId);
        // A discovery id is unknown on the artifact axis and vice versa.
        vm.expectRevert(bytes("unknown artifact set"));
        reg.getArtifactSet(v2Id);
        vm.expectRevert(bytes("unknown discovery set"));
        reg.getDiscoverySet(artId);
        // ...and generation 2's discovery key is not generation 1's.
        assertTrue(v2Id != ProtocolVersions.levelBId(), "generation 2 publishes under a new key");
        // The artifact key IS generation 1's — the artifacts are unchanged, so the identity is too.
        assertEq(artId, ProtocolVersions.levelBArtifactsId(), "the frozen artifacts keep their identity");
    }

    // =============================================================================================
    // --- Deprecate: immediate, history-preserving, and cancels a stale proposal ---
    // =============================================================================================

    /// @notice Deprecate flips `active` without deleting the record or its list entry, and it cancels an
    /// in-flight proposal — which is what makes it an emergency halt rather than a bit a stale proposal
    /// whose timelock already elapsed can flip straight back.
    function test_deprecate_keeps_history_and_cancels_an_in_flight_proposal() public {
        _publishGenerationTwo();

        // Stage a swap and let its timelock elapse, then deprecate.
        ProtocolRegistryV2.DiscoverySet memory swap = _discovery();
        swap.verificationRegistry = address(0xBADBAD);
        vm.prank(PUBLISHER);
        reg.proposeDiscoverySet(swap);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());

        vm.prank(PUBLISHER);
        reg.deprecateDiscoverySet(v2Id);

        assertFalse(reg.getDiscoverySet(v2Id).active, "deprecated");
        assertEq(reg.getDiscoverySet(v2Id).verificationRegistry, VERIFICATION_REGISTRY, "record kept");
        assertEq(reg.discoverySetCount(), 1, "the list entry is untouched");
        assertEq(reg.discoverySetEta(v2Id), 0, "the stale proposal is cancelled");

        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("none pending"));
        reg.executeDiscoverySet(v2Id);

        // Same on the artifact axis, where a re-activation would restore compromised artifacts as live.
        vm.prank(PUBLISHER);
        reg.proposeArtifactSet(_artifacts());
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        reg.deprecateArtifactSet(artId);
        assertFalse(reg.getArtifactSet(artId).active);
        assertEq(reg.artifactSetEta(artId), 0);
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("none pending"));
        reg.executeArtifactSet(artId);
    }

    /// @notice Re-publishing after a deprecate costs a fresh propose plus the FULL timelock.
    function test_republish_after_deprecate_requires_a_fresh_timelock() public {
        _publishGenerationTwo();
        vm.prank(PUBLISHER);
        reg.deprecateDiscoverySet(v2Id);

        vm.prank(PUBLISHER);
        reg.proposeDiscoverySet(_discovery());
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("timelock"));
        reg.executeDiscoverySet(v2Id);

        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        reg.executeDiscoverySet(v2Id);
        assertTrue(reg.getDiscoverySet(v2Id).active);
        assertEq(reg.discoverySetCount(), 1, "a republish does not duplicate the list entry");
    }

    /// @notice A binding needs both sides published AND active at the moment of execute, so a set retired
    /// mid-window cannot be bound.
    function test_binding_requires_both_sides_published_and_active() public {
        // Unbound: artifact resolution fails closed rather than answering a zeroed record.
        _publishDiscovery(_discovery());
        vm.expectRevert(bytes("no artifact binding"));
        reg.getActiveArtifactSet(v2Id);
        vm.expectRevert(bytes("no artifact binding"));
        reg.resolveDiscovery(v2Id);

        // Unpublished artifact side.
        vm.prank(PUBLISHER);
        reg.proposeArtifactBinding(v2Id, artId);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("unknown artifact set"));
        reg.executeArtifactBinding(v2Id);

        // Published but deprecated artifact side.
        _publishArtifacts(_artifacts());
        vm.prank(PUBLISHER);
        reg.deprecateArtifactSet(artId);
        vm.prank(PUBLISHER);
        reg.proposeArtifactBinding(v2Id, artId);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("inactive artifact set"));
        reg.executeArtifactBinding(v2Id);

        // Deprecated discovery side.
        vm.startPrank(PUBLISHER);
        reg.proposeArtifactSet(_artifacts());
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        reg.executeArtifactSet(artId);
        reg.deprecateDiscoverySet(v2Id);
        reg.proposeArtifactBinding(v2Id, artId);
        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK());
        vm.expectRevert(bytes("inactive discovery set"));
        reg.executeArtifactBinding(v2Id);
        vm.stopPrank();
    }

    // =============================================================================================
    // --- Fail-closed reads and role gating ---
    // =============================================================================================

    /// @notice Every resolver reverts on an unknown id rather than answering a zeroed record that would
    /// read as an unpinned, inactive-but-present set.
    function test_unknown_ids_revert() public {
        bytes32 nope = keccak256("dogtag-nope/1");
        vm.expectRevert(bytes("unknown discovery set"));
        reg.getDiscoverySet(nope);
        vm.expectRevert(bytes("unknown artifact set"));
        reg.getArtifactSet(nope);
        vm.expectRevert(bytes("no artifact binding"));
        reg.getActiveArtifactSet(nope);
        vm.expectRevert(bytes("unknown discovery set"));
        reg.resolveDiscovery(nope);
        vm.expectRevert(bytes("none pending"));
        reg.getPendingDiscoverySet(nope);
        vm.expectRevert(bytes("none pending"));
        reg.getPendingArtifactSet(nope);
        vm.expectRevert(bytes("none pending"));
        reg.getPendingBinding(nope);

        // The deprecates are pranked as the publisher deliberately: unpranked they would revert on the
        // ROLE gate, and this test would then pass while asserting nothing about the unknown-id branch.
        vm.startPrank(PUBLISHER);
        vm.expectRevert(bytes("unknown discovery set"));
        reg.deprecateDiscoverySet(nope);
        vm.expectRevert(bytes("unknown artifact set"));
        reg.deprecateArtifactSet(nope);
        vm.stopPrank();
    }

    /// @notice `getPending*` exposes exactly what an execute would write, which is what turns the delay
    /// into a review period rather than a wait.
    function test_a_pending_proposal_is_fully_inspectable_during_the_window() public {
        ProtocolRegistryV2.DiscoverySet memory d = _discovery();
        d.rootIndex = address(0xFEED);
        vm.prank(PUBLISHER);
        reg.proposeDiscoverySet(d);

        ProtocolRegistryV2.DiscoverySet memory pending = reg.getPendingDiscoverySet(v2Id);
        assertEq(pending.rootIndex, address(0xFEED), "governance can see the exact staged rootIndex");
        assertEq(pending.providerRegistry, PROVIDER_REGISTRY);

        vm.prank(PUBLISHER);
        reg.proposeArtifactBinding(v2Id, artId);
        assertEq(reg.getPendingBinding(v2Id), artId);
    }

    /// @notice Re-proposing before execute overwrites the staged record and RESETS its timelock, so a
    /// second proposal cannot inherit the first one's elapsed window.
    function test_reproposing_resets_the_timelock() public {
        vm.prank(PUBLISHER);
        reg.proposeDiscoverySet(_discovery());
        uint256 firstEta = reg.discoverySetEta(v2Id);

        vm.warp(block.timestamp + reg.PUBLISH_TIMELOCK() - 1);
        ProtocolRegistryV2.DiscoverySet memory second = _discovery();
        second.rootIndex = address(0xFEED);
        vm.prank(PUBLISHER);
        reg.proposeDiscoverySet(second);

        assertGt(reg.discoverySetEta(v2Id), firstEta, "the window restarts");
        assertEq(reg.getPendingDiscoverySet(v2Id).rootIndex, address(0xFEED));
        vm.prank(PUBLISHER);
        vm.expectRevert(bytes("timelock"));
        reg.executeDiscoverySet(v2Id);
    }

    /// @notice Every write is `PUBLISHER_ROLE`-gated, including the un-timelocked deprecates, and the
    /// admin does not get them implicitly.
    function test_every_write_is_publisher_gated() public {
        bytes memory denied = abi.encodeWithSelector(
            IAccessControl.AccessControlUnauthorizedAccount.selector, OUTSIDER, reg.PUBLISHER_ROLE()
        );

        vm.startPrank(OUTSIDER);
        vm.expectRevert(denied);
        reg.proposeDiscoverySet(_discovery());
        vm.expectRevert(denied);
        reg.executeDiscoverySet(v2Id);
        vm.expectRevert(denied);
        reg.deprecateDiscoverySet(v2Id);
        vm.expectRevert(denied);
        reg.proposeArtifactSet(_artifacts());
        vm.expectRevert(denied);
        reg.executeArtifactSet(artId);
        vm.expectRevert(denied);
        reg.deprecateArtifactSet(artId);
        vm.expectRevert(denied);
        reg.proposeArtifactBinding(v2Id, artId);
        vm.expectRevert(denied);
        reg.executeArtifactBinding(v2Id);
        vm.stopPrank();

        // The admin holds DEFAULT_ADMIN_ROLE but not PUBLISHER_ROLE — the roles are separable, which is
        // the point of holding them apart.
        assertTrue(reg.hasRole(reg.DEFAULT_ADMIN_ROLE(), ADMIN));
        assertFalse(reg.hasRole(reg.PUBLISHER_ROLE(), ADMIN));
        assertTrue(reg.hasRole(reg.PUBLISHER_ROLE(), PUBLISHER));
        assertEq(reg.PUBLISHER_ROLE(), keccak256("PUBLISHER"), "NOT keccak256('PUBLISHER_ROLE')");
    }

    /// @notice `resolveDiscovery` answers both halves and agrees with the two single-axis getters.
    function test_resolve_returns_both_axes() public {
        _publishGenerationTwo();
        (ProtocolRegistryV2.DiscoverySet memory d, ProtocolRegistryV2.ArtifactSet memory a) =
            reg.resolveDiscovery(v2Id);
        assertEq(d.discoverySetId, reg.getDiscoverySet(v2Id).discoverySetId);
        assertEq(d.rootIndex, ROOT_INDEX);
        assertEq(a.artifactSetId, reg.getActiveArtifactSet(v2Id).artifactSetId);
        assertEq(a.minAppVersion, MIN_APP);
    }

    /// @notice The GOLDEN encoding both mobile decoders are pinned against, asserted from this end too.
    ///
    /// `apps/android/.../AnchorResolverTest.kt` and `apps/ios/DogTagTests/AnchorResolverTests.swift` each
    /// carry these exact bytes and run them through their own `decodeDiscoverySet`. Pinning the same
    /// literal here is what makes the pair a contract rather than two independent guesses: a change to the
    /// record's shape or member order fails HERE first, naming the two files to regenerate, instead of
    /// leaving two mobile decoders quietly reading the wrong words. Regenerate by re-encoding
    /// `getDiscoverySet` — never by hand-editing hex, which tests your idea of the ABI rather than the ABI.
    ///
    /// Neither mobile suite runs in CI, so this assertion is the only automated end of the pair.
    function test_the_golden_encoding_the_mobile_decoders_are_pinned_against() public {
        ProtocolRegistryV2.DiscoverySet memory d = ProtocolVersionsV2.levelBV2Discovery(
            0x1C9Ac2eB3f1A2D4B5C6d7E8f90A1B2C3D4e5F607, // factory (generation 2)
            0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87, // verificationRegistry
            0x96Cba4580D79bc9b8e51Fc1B3a044A29592AfFFc, // sbt (the SHARED, reused one)
            0x272be146C0aEd6401000E9Aa8241201F6f0fdF1a, // verifier (frozen ceremony VK)
            0x9309aB1c2d3E4F5061728394A5B6C7D8E9F00112, // providerRegistry (authority core)
            0x120127E4a5b6C7D8e9F001122334455667788990 // rootIndex (CloneProvenanceRouter)
        );
        vm.warp(1_800_000_000);
        _publishDiscovery(d);

        assertEq(
            abi.encode(reg.getDiscoverySet(v2Id)),
            hex"44a8d618d62ba7874b6df187bc27a0c147d0523c8cda11f7764d343d032ae0b5"
            hex"0000000000000000000000001c9ac2eb3f1a2d4b5c6d7e8f90a1b2c3d4e5f607"
            hex"000000000000000000000000b9b313c17fd8725bb50a7f41121ac4cf5f4fec87"
            hex"00000000000000000000000096cba4580d79bc9b8e51fc1b3a044a29592afffc"
            hex"000000000000000000000000272be146c0aed6401000e9aa8241201f6f0fdf1a"
            hex"0000000000000000000000009309ab1c2d3e4f5061728394a5b6c7d8e9f00112"
            hex"000000000000000000000000120127e4a5b6c7d8e9f001122334455667788990"
            hex"a708f8e240d9734e5f054f55fa891a37c31f536a5de28874439572018c9aa54f"
            hex"000000000000000000000000000000000000000000000000000000006b4c7500"
            hex"0000000000000000000000000000000000000000000000000000000000000001",
            "regenerate the two mobile golden vectors from this encoding"
        );
        // The two facts a reader would otherwise have to take on trust from the hex.
        assertEq(v2Id, keccak256("dogtag-levelb/2"));
        assertEq(reg.getDiscoverySet(v2Id).circuitId, keccak256("consent.circom/DogTagConsent(6)"));
    }

    /// @notice The generation-2 record does NOT claim the frozen circuit moved: the circuitId is
    /// byte-identical to generation 1's, because no part of this work touches the circuit.
    function test_the_frozen_circuit_identity_is_unchanged() public {
        _publishGenerationTwo();
        assertEq(
            reg.getDiscoverySet(v2Id).circuitId,
            ProtocolVersions.levelBContracts(FACTORY_V2, VERIFICATION_REGISTRY, SBT, VERIFIER).circuitId,
            "generation 2 proves the same circuit"
        );
        assertEq(ProtocolVersionsV2.consentCircuitId(), keccak256("consent.circom/DogTagConsent(6)"));
    }
}
