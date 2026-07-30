// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {ProtocolRegistryV2, MIN_PUBLISH_TIMELOCK_SECONDS_V2} from "../src/ProtocolRegistryV2.sol";
import {DeployProtocolRegistryV2} from "../script/DeployProtocolRegistryV2.s.sol";
import {
    PublishProtocolVersionsV2Propose,
    PublishProtocolVersionsV2Execute
} from "../script/PublishProtocolVersionsV2.s.sol";
import {ProtocolVersionsV2} from "../script/ProtocolVersionsV2.sol";
import {CloneProvenanceRouter} from "../src/CloneProvenanceRouter.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagSBTConsent} from "../src/DogTagSBTConsent.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";
import {Groth16VerifierConsent} from "../src/Groth16VerifierConsent.sol";

/// @notice The generation-2 deploy + publish scripts.
///
/// The publish half runs against a REAL stack — a real `CloneProvenanceRouter` over real factories, a
/// real `DogTagSBTConsent`, a real `Groth16VerifierConsent` and a real `VerificationRegistryConsent` —
/// rather than against doubles. The preflight's whole job is to compare a transcribed record against
/// what deployed contracts actually say, and a double that returns whatever the test told it to return
/// would agree with the test rather than with the chain.
contract DeployProtocolRegistryV2Test is Test {
    DeployProtocolRegistryV2 internal deployer;

    IssuerRegistry internal providerCore; // stands in the immutable `issuerRegistry` slot
    DogTagIssuer internal impl;
    DogTagIssuerFactory internal factoryV1;
    DogTagIssuerFactory internal factoryV2;
    CloneProvenanceRouter internal router;
    DogTagSBTConsent internal sbt;
    Groth16VerifierConsent internal verifier;
    VerificationRegistryConsent internal verificationRegistry;

    address internal constant GOV = address(0xA11CE);
    address internal constant CUSTODIAN = address(0xC0570D1A);

    function setUp() public {
        deployer = new DeployProtocolRegistryV2();

        // A real generation-1 + generation-2 factory pair behind a real router, oldest first.
        //
        // `IssuerRegistry` stands in for the S-6 `ProviderRegistry` here on purpose: what this test
        // exercises is the PREFLIGHT's address comparison against the verification registry's immutable
        // slot, and the two are interchangeable for that read because S-6 keeps
        // `isWhitelistedFor(bytes32,address)` at generation 1's exact shape — which is precisely what
        // lets the core drop into that slot at C-5. Deploying the real core here would test the core, not
        // the preflight.
        providerCore = new IssuerRegistry(GOV);
        impl = new DogTagIssuer();
        factoryV1 = new DogTagIssuerFactory(address(impl), address(providerCore), GOV);
        factoryV2 = new DogTagIssuerFactory(address(impl), address(providerCore), GOV);
        address[] memory gens = new address[](2);
        gens[0] = address(factoryV1); // OLDEST first — the router's whole safety property
        gens[1] = address(factoryV2);
        router = new CloneProvenanceRouter(gens, GOV);

        sbt = new DogTagSBTConsent(GOV, CUSTODIAN);
        verifier = new Groth16VerifierConsent();
        verificationRegistry = new VerificationRegistryConsent(
            address(providerCore), address(sbt), address(verifier), address(router), GOV
        );
    }

    // --- the delay guard -------------------------------------------------------------------------

    function test_mainnet_guard_accepts_only_the_two_day_default() public view {
        deployer.validatePublishTimelock(2 days, false);
    }

    function test_mainnet_guard_refuses_any_other_delay() public {
        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        deployer.validatePublishTimelock(0, false);
        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        deployer.validatePublishTimelock(2 days - 1, false);
        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        deployer.validatePublishTimelock(2 days + 1, false);
    }

    /// @notice The deliberate divergence from generation 1's deploy script, which has a passing test named
    /// `test_explicit_testnet_opt_in_accepts_zero_timelock`. Here the testnet opt-in buys a SHORTER delay
    /// and never a zero one: the live registry's zero is the defect this generation exists to correct, so
    /// an opt-in that could still reach it would reproduce it on the one deployment that can fix it.
    function test_the_testnet_opt_in_cannot_reach_zero() public {
        vm.expectRevert(bytes("testnet publish timelock below the 1-hour floor"));
        deployer.validatePublishTimelock(0, true);
        vm.expectRevert(bytes("testnet publish timelock below the 1-hour floor"));
        deployer.validatePublishTimelock(1, true);
        vm.expectRevert(bytes("testnet publish timelock below the 1-hour floor"));
        deployer.validatePublishTimelock(MIN_PUBLISH_TIMELOCK_SECONDS_V2 - 1, true);

        // At and above the floor a testnet may go short.
        deployer.validatePublishTimelock(MIN_PUBLISH_TIMELOCK_SECONDS_V2, true);
        deployer.validatePublishTimelock(6 hours, true);
    }

    /// @notice The script's guard and the contract's floor are not one guard written twice: the contract
    /// refuses a zero even when the script is bypassed entirely, which is the case that matters because
    /// the value is immutable.
    function test_the_contract_floor_holds_without_the_script() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                ProtocolRegistryV2.PublishTimelockBelowFloor.selector, 0, MIN_PUBLISH_TIMELOCK_SECONDS_V2
            )
        );
        new ProtocolRegistryV2(GOV, GOV, 0);
    }

    // --- the env-driven publish ------------------------------------------------------------------

    function _setPublishEnv(ProtocolRegistryV2 registry) internal {
        vm.setEnv("GEN2_PROTOCOL_REGISTRY", vm.toString(address(registry)));
        vm.setEnv("GEN2_FACTORY", vm.toString(address(factoryV2)));
        vm.setEnv("GEN2_VERIFICATION_REGISTRY", vm.toString(address(verificationRegistry)));
        vm.setEnv("GEN2_SBT", vm.toString(address(sbt)));
        vm.setEnv("GEN2_VERIFIER", vm.toString(address(verifier)));
        vm.setEnv("GEN2_PROVIDER_REGISTRY", vm.toString(address(providerCore)));
        vm.setEnv("GEN2_ROOT_INDEX", vm.toString(address(router)));
        vm.setEnv("GEN2_ZKEY_SHA256", vm.toString(bytes32(uint256(0xA11CE))));
        vm.setEnv("GEN2_WITNESS_MOBILE_SHA256", vm.toString(bytes32(uint256(0xB0B))));
        vm.setEnv("GEN2_R1CS_SHA256", vm.toString(bytes32(uint256(0xCAFE))));
        vm.setEnv("GEN2_WASM_SHA256", vm.toString(bytes32(uint256(0xD06))));
        vm.setEnv("GEN2_ARTIFACTS_URL", "https://artifacts.dogtag.test/consent");
        vm.setEnv("GEN2_MIN_APP_VERSION", "1.6.0");
    }

    function _deployRegistry() internal returns (ProtocolRegistryV2) {
        vm.setEnv("GEN2_ADMIN", vm.toString(DEFAULT_SENDER));
        vm.setEnv("GEN2_PUBLISHER", vm.toString(DEFAULT_SENDER));
        vm.setEnv("GEN2_PUBLISH_TIMELOCK_SECS", vm.toString(MIN_PUBLISH_TIMELOCK_SECONDS_V2));
        vm.setEnv("GEN2_TESTNET_DEPLOY", "true");
        deployer.run();
        return ProtocolRegistryV2(deployer.protocolRegistryV2());
    }

    /// @notice The real scripts, end to end, against the real stack: deploy, preflight, propose, wait the
    /// configured delay, execute, and read every published member back.
    function test_env_driven_scripts_publish_generation_two_on_both_axes() public {
        ProtocolRegistryV2 registry = _deployRegistry();
        assertEq(
            registry.PUBLISH_TIMELOCK(),
            MIN_PUBLISH_TIMELOCK_SECONDS_V2,
            "even the shortest testnet deployment carries a real delay"
        );
        _setPublishEnv(registry);

        new PublishProtocolVersionsV2Propose().run();

        // The delay is real in THIS deployment, not merely stored: executing now reverts. Asserted
        // against the registry directly rather than by running the execute script early, because a
        // reverted `run()` leaves the script's broadcast open and the later, legitimate run would then
        // fail for a cheatcode reason instead of a protocol one.
        bytes32 pendingId = ProtocolVersionsV2.levelBV2Id();
        assertGt(registry.discoverySetEta(pendingId), block.timestamp, "the window is still open");
        vm.prank(DEFAULT_SENDER);
        vm.expectRevert(bytes("timelock"));
        registry.executeDiscoverySet(pendingId);

        vm.warp(block.timestamp + registry.PUBLISH_TIMELOCK());
        new PublishProtocolVersionsV2Execute().run();

        bytes32 versionId = ProtocolVersionsV2.levelBV2Id();
        bytes32 artifactId = ProtocolVersionsV2.levelBArtifactsId();
        assertEq(registry.discoverySetCount(), 1, "exactly one discovery set");
        assertEq(registry.artifactSetCount(), 1, "exactly one artifact set");
        assertEq(registry.discoverySetList(0), versionId);
        assertEq(registry.artifactSetList(0), artifactId);
        assertEq(registry.activeArtifactSetOf(versionId), artifactId, "both axes must be bound");

        ProtocolRegistryV2.DiscoverySet memory d = registry.getDiscoverySet(versionId);
        assertEq(d.factory, address(factoryV2));
        assertEq(d.verificationRegistry, address(verificationRegistry));
        assertEq(d.sbt, address(sbt));
        assertEq(d.verifier, address(verifier));
        assertEq(d.providerRegistry, address(providerCore));
        assertEq(d.rootIndex, address(router));
        assertEq(d.circuitId, keccak256("consent.circom/DogTagConsent(6)"));
        assertTrue(d.active);

        ProtocolRegistryV2.ArtifactSet memory a = registry.getActiveArtifactSet(versionId);
        assertEq(a.artifactSetId, artifactId);
        assertEq(a.zkeySha256, bytes32(uint256(0xA11CE)));
        assertEq(a.witnessMobileSha256, bytes32(uint256(0xB0B)));
        assertEq(a.witnessServerR1csSha256, bytes32(uint256(0xCAFE)));
        assertEq(a.witnessServerWasmSha256, bytes32(uint256(0xD06)));
        assertEq(a.artifactBaseUrl, "https://artifacts.dogtag.test/consent");
        assertEq(a.minAppVersion, "1.6.0", "the app floor is published as given, with no default");
        assertTrue(a.active);
    }

    /// @notice The preflight is the point of the propose phase, so each of its five relations is broken in
    /// turn and must refuse. Without these the preflight could be deleted and the happy path above would
    /// still pass.
    function test_the_preflight_refuses_every_mis_transcribed_address() public {
        ProtocolRegistryV2 registry = _deployRegistry();
        _setPublishEnv(registry);
        PublishProtocolVersionsV2Propose propose = new PublishProtocolVersionsV2Propose();

        // The most consequential slip: the factory published where the root index belongs. It is exactly
        // the generation-1 invariant (`factory == rootIndex`) applied to a generation where it is false,
        // and a consumer following it resolves generation-2 roots only and loses all 19 historical ones.
        vm.setEnv("GEN2_ROOT_INDEX", vm.toString(address(factoryV2)));
        vm.expectRevert(bytes("rootIndex != verificationRegistry.rootIndex()"));
        propose.run();
        vm.setEnv("GEN2_ROOT_INDEX", vm.toString(address(router)));

        vm.setEnv("GEN2_PROVIDER_REGISTRY", vm.toString(address(sbt)));
        vm.expectRevert(bytes("providerRegistry != verificationRegistry.issuerRegistry()"));
        propose.run();
        vm.setEnv("GEN2_PROVIDER_REGISTRY", vm.toString(address(providerCore)));

        vm.setEnv("GEN2_SBT", vm.toString(address(providerCore)));
        vm.expectRevert(bytes("sbt != verificationRegistry.sbt()"));
        propose.run();
        vm.setEnv("GEN2_SBT", vm.toString(address(sbt)));

        vm.setEnv("GEN2_VERIFIER", vm.toString(address(sbt)));
        vm.expectRevert(bytes("verifier != verificationRegistry.zkVerifier()"));
        propose.run();
        vm.setEnv("GEN2_VERIFIER", vm.toString(address(verifier)));

        // A factory the router has never been told about: its clones' roots would resolve nowhere.
        DogTagIssuerFactory stranger = new DogTagIssuerFactory(address(impl), address(providerCore), GOV);
        vm.setEnv("GEN2_FACTORY", vm.toString(address(stranger)));
        vm.expectRevert(bytes("factory is not a generation of rootIndex"));
        propose.run();
        vm.setEnv("GEN2_FACTORY", vm.toString(address(factoryV2)));

        // With everything restored the same call succeeds, so none of the above passed for a stray reason.
        propose.run();
        assertGt(registry.discoverySetEta(ProtocolVersionsV2.levelBV2Id()), 0);
    }

    /// @notice The one relation that can drift INSIDE the publish window, driven for real rather than
    /// asserted: `zkVerifier` is the verification registry's only mutable member and its own timelock is
    /// 2 days, the same length as the mainnet publish timelock, so a swap proposed shortly before a
    /// publish executes squarely inside that window. The other four relations are immutable slots or the
    /// router's append-only membership, so this is the whole of what the execute-phase re-preflight buys.
    ///
    /// Every `vm.setEnv` here writes the canonical values, deliberately: `vm.setEnv` mutates the PROCESS
    /// environment, so a test that wrote a divergent one would race every other test in this file.
    function test_execute_refuses_a_verifier_that_moved_inside_the_publish_window() public {
        ProtocolRegistryV2 registry = _deployRegistry();
        _setPublishEnv(registry);
        // Both scripts are constructed UP FRONT: `vm.expectRevert` applies to the next call, and a
        // `new ...()` on the same statement would consume it, so the refusals below would be asserted
        // against a contract creation that never reverts.
        PublishProtocolVersionsV2Propose propose = new PublishProtocolVersionsV2Propose();
        PublishProtocolVersionsV2Execute execute = new PublishProtocolVersionsV2Execute();
        propose.run();

        // The registry's REAL swap, through its own timelock - not a storage poke.
        Groth16VerifierConsent replacement = new Groth16VerifierConsent();
        assertTrue(address(replacement) != address(verifier), "the swap must actually change the address");
        vm.prank(GOV);
        verificationRegistry.proposeZkVerifier(address(replacement));
        vm.warp(block.timestamp + 2 days);
        vm.prank(GOV);
        verificationRegistry.executeZkVerifier();

        // Asserted BEFORE the refusal: a setup that silently failed would leave the old verifier in
        // place, the preflight would pass, and the refusal below would then have to come from somewhere
        // else - so this test would be pinning nothing.
        assertEq(
            address(verificationRegistry.zkVerifier()),
            address(replacement),
            "the verifier really moved on-chain"
        );
        bytes32 versionId = ProtocolVersionsV2.levelBV2Id();
        assertLe(
            registry.discoverySetEta(versionId),
            block.timestamp,
            "the publish window has elapsed, so a refusal cannot be the timelock's"
        );

        vm.expectRevert(bytes("verifier != verificationRegistry.zkVerifier()"));
        execute.run();

        // The remedy is a fresh propose, so the propose phase must refuse this record too until the
        // operator names the new verifier. This also pins the re-preflight's PLACEMENT: it runs before
        // `vm.startBroadcast()`, so the refusal above left no broadcast open - were it placed after,
        // this call would fail with a cheatcode error instead of the protocol revert asserted here.
        vm.expectRevert(bytes("verifier != verificationRegistry.zkVerifier()"));
        propose.run();
    }

    /// @notice The other half of the execute-phase re-check, on its own because it is a different claim:
    /// `executeDiscoverySet` writes the STAGED bytes and never reads the environment, so preflighting the
    /// environment alone would pass for an operator who reacted to a mid-window verifier swap by editing
    /// `GEN2_VERIFIER` - while the retired verifier stayed staged and got published as certified.
    ///
    /// The divergence is created by RE-STAGING a different record straight on the registry rather than by
    /// mutating `GEN2_VERIFIER`: `vm.setEnv` writes the process-global environment, so a divergent value
    /// would race every other test in this file. The environment stays canonical throughout, which also
    /// makes the refusal attributable - a preflight over that environment would pass.
    function test_the_execute_phase_refuses_a_record_re_staged_since_the_environment_was_read() public {
        ProtocolRegistryV2 registry = _deployRegistry();
        _setPublishEnv(registry);
        PublishProtocolVersionsV2Execute execute = new PublishProtocolVersionsV2Execute();
        new PublishProtocolVersionsV2Propose().run();

        // Same id, one different member - so this REPLACES what `executeDiscoverySet` would write.
        ProtocolRegistryV2.DiscoverySet memory reStaged = ProtocolVersionsV2.levelBV2Discovery(
            address(factoryV2),
            address(verificationRegistry),
            address(sbt),
            address(new Groth16VerifierConsent()),
            address(providerCore),
            address(router)
        );
        vm.prank(DEFAULT_SENDER);
        registry.proposeDiscoverySet(reStaged);
        assertEq(
            registry.getPendingDiscoverySet(ProtocolVersionsV2.levelBV2Id()).verifier,
            reStaged.verifier,
            "the staged record really was replaced"
        );

        vm.warp(block.timestamp + registry.PUBLISH_TIMELOCK());
        vm.expectRevert(bytes("staged discovery set differs from this environment"));
        execute.run();
    }

    /// @notice The preflight accepts generation 1's factory too, because the router holds both generations
    /// and either is a legitimate `factory` member — the check is membership, not recency.
    function test_the_preflight_accepts_any_generation_the_router_holds() public {
        ProtocolRegistryV2 registry = _deployRegistry();
        _setPublishEnv(registry);
        vm.setEnv("GEN2_FACTORY", vm.toString(address(factoryV1)));
        new PublishProtocolVersionsV2Propose().run();
        assertEq(registry.getPendingDiscoverySet(ProtocolVersionsV2.levelBV2Id()).factory, address(factoryV1));
    }
}
