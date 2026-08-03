// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {ProtocolRegistry, MIN_PUBLISH_TIMELOCK_SECONDS} from "../src/ProtocolRegistry.sol";
import {Deploy} from "../script/Deploy.s.sol";
import {
    PublishProtocolVersionsPropose,
    PublishProtocolVersionsExecute
} from "../script/PublishProtocolVersions.s.sol";
import {ProtocolVersions} from "../script/ProtocolVersions.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {Groth16VerifierConsent} from "../src/Groth16VerifierConsent.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {LaunchStack} from "./LaunchStack.sol";

/// @notice The deploy and publish scripts.
///
/// Both halves run against a REAL stack, stood up by the REAL deploy script through {LaunchStack} — a
/// real core, a real factory, a real SBT, a real frozen verifier and a real verification registry —
/// rather than against doubles. The preflight's whole job is to compare a transcribed record against
/// what deployed contracts actually say, and a double that returned whatever the test told it to return
/// would agree with the test rather than with the chain.
///
/// What is deliberately NOT driven here is `Deploy.run()`: it reads the environment, broadcasts, and
/// WRITES `deployments/roax.json`, so running it in a test would mutate the committed address ledger.
/// Its two separable halves are covered instead — `validatePublishTimelock` directly, and the whole
/// deployment sequence through `deploy()`, which every suite built on {LaunchStack} exercises.
contract DeployTest is LaunchStack {
    Deploy internal guardHarness;

    /// A publish script instance used ONLY for its `public view` preflight, which needs no environment.
    PublishProtocolVersionsPropose internal preflightHarness;

    address internal GOV;

    function setUp() public {
        _deployLaunchStack();
        GOV = REGISTRAR;
        guardHarness = new Deploy();
        preflightHarness = new PublishProtocolVersionsPropose();

        // The publish scripts broadcast as forge's default sender, so that key needs the role. Granted
        // by the ADMIN rather than assumed, which is also the real shape: `PUBLISHER_ROLE` is separable
        // from `DEFAULT_ADMIN_ROLE` precisely so the key that publishes need not be the key that governs.
        // Read the role BEFORE pranking: `vm.prank` survives only to the next CALL, and
        // `PUBLISHER_ROLE()` would otherwise consume it.
        bytes32 publisherRole = protocolRegistry.PUBLISHER_ROLE();
        vm.prank(REGISTRAR);
        protocolRegistry.grantRole(publisherRole, DEFAULT_SENDER);
    }

    // --- the delay guard -------------------------------------------------------------------------

    function test_mainnet_guard_accepts_only_the_two_day_default() public view {
        guardHarness.validatePublishTimelock(2 days, false);
    }

    function test_mainnet_guard_refuses_any_other_delay() public {
        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        guardHarness.validatePublishTimelock(0, false);
        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        guardHarness.validatePublishTimelock(2 days - 1, false);
        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        guardHarness.validatePublishTimelock(2 days + 1, false);
    }

    /// @notice The deliberate divergence from generation 1's deploy script, which has a passing test named
    /// `test_explicit_testnet_opt_in_accepts_zero_timelock`. Here the testnet opt-in buys a SHORTER delay
    /// and never a zero one: the live registry's zero is the defect this generation exists to correct, so
    /// an opt-in that could still reach it would reproduce it on the one deployment that can fix it.
    function test_the_testnet_opt_in_cannot_reach_zero() public {
        vm.expectRevert(bytes("testnet publish timelock below the 1-hour floor"));
        guardHarness.validatePublishTimelock(0, true);
        vm.expectRevert(bytes("testnet publish timelock below the 1-hour floor"));
        guardHarness.validatePublishTimelock(1, true);
        vm.expectRevert(bytes("testnet publish timelock below the 1-hour floor"));
        guardHarness.validatePublishTimelock(MIN_PUBLISH_TIMELOCK_SECONDS - 1, true);

        // At and above the floor a testnet may go short.
        guardHarness.validatePublishTimelock(MIN_PUBLISH_TIMELOCK_SECONDS, true);
        guardHarness.validatePublishTimelock(6 hours, true);
    }

    /// @notice The script's guard and the contract's floor are not one guard written twice: the contract
    /// refuses a zero even when the script is bypassed entirely, which is the case that matters because
    /// the value is immutable.
    function test_the_contract_floor_holds_without_the_script() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                ProtocolRegistry.PublishTimelockBelowFloor.selector, 0, MIN_PUBLISH_TIMELOCK_SECONDS
            )
        );
        new ProtocolRegistry(GOV, GOV, 0);
    }

    // --- the deployment sequence -----------------------------------------------------------------

    /// @notice The deployed system is wired to itself, read back from the contracts rather than from the
    /// script's own variables. `deploy()` asserts all of this internally too; asserting it again here is
    /// what makes those internal `require`s a tested claim rather than an untested one.
    ///
    /// The `rootIndex` line is the one that matters most: it is immutable, so a deployment that got it
    /// wrong would answer `unknown root` for every credential ever issued through it, with no repair.
    function test_the_deploy_script_wires_the_system_to_itself() public view {
        assertEq(address(vr.rootIndex()), address(factory), "the registry resolves roots through the factory");
        assertEq(address(vr.providerRegistry()), address(core));
        assertEq(address(vr.sbt()), address(sbt));
        assertEq(address(vr.zkVerifier()), address(verifier));
        assertEq(factory.registry(), address(core));
        assertEq(factory.implementation(), address(implementation));
        assertEq(core.generationOfFactory(address(factory)), FACTORY_GENERATION);
        assertTrue(
            core.isResolverApproved(
                ProviderRegistry.ResolverKind.DIRECTORY, deployer.providerDirectory()
            )
        );
    }

    /// @notice The custodian is a sink that never signs, so pointing it at the authority would re-link
    /// every tag to a key that does. Refused before anything is deployed.
    function test_the_deploy_script_refuses_a_custodian_that_is_the_authority() public {
        Deploy fresh = new Deploy();
        vm.expectRevert(bytes("CUSTODIAN must not be ADMIN"));
        fresh.deploy(address(fresh), address(fresh), address(fresh), TEST_PUBLISH_TIMELOCK);
    }

    // --- the env-driven publish ------------------------------------------------------------------

    /// @dev Every value written here is CANONICAL — derived from the one `setUp` fixture, which forge
    /// snapshots, so every test function in this file writes byte-identical bytes. That is what makes
    /// writing the process-global environment safe here: `vm.setEnv` mutates the real process
    /// environment while forge runs test functions concurrently, so a test that wrote a DIVERGENT value
    /// would be observable by every other test in the file. The invariant is divergence, not abstinence.
    function _setPublishEnv() internal {
        vm.setEnv("PUBLISH_PROTOCOL_REGISTRY", vm.toString(address(protocolRegistry)));
        vm.setEnv("PUBLISH_FACTORY", vm.toString(address(factory)));
        vm.setEnv("PUBLISH_VERIFICATION_REGISTRY", vm.toString(address(vr)));
        vm.setEnv("PUBLISH_SBT", vm.toString(address(sbt)));
        vm.setEnv("PUBLISH_VERIFIER", vm.toString(address(verifier)));
        vm.setEnv("PUBLISH_PROVIDER_REGISTRY", vm.toString(address(core)));
        vm.setEnv("PUBLISH_ZKEY_SHA256", vm.toString(bytes32(uint256(0xA11CE))));
        vm.setEnv("PUBLISH_WITNESS_MOBILE_SHA256", vm.toString(bytes32(uint256(0xB0B))));
        vm.setEnv("PUBLISH_R1CS_SHA256", vm.toString(bytes32(uint256(0xCAFE))));
        vm.setEnv("PUBLISH_WASM_SHA256", vm.toString(bytes32(uint256(0xD06))));
        vm.setEnv("PUBLISH_ARTIFACTS_URL", "https://artifacts.dogtag.test/consent");
        vm.setEnv("PUBLISH_MIN_APP_VERSION", "1.6.0");
    }
    /// @notice The real scripts, end to end, against the real stack: deploy, preflight, propose, wait the
    /// configured delay, execute, and read every published member back.
    function test_env_driven_scripts_publish_generation_two_on_both_axes() public {
        ProtocolRegistry registry = protocolRegistry;
        assertEq(
            registry.PUBLISH_TIMELOCK(),
            MIN_PUBLISH_TIMELOCK_SECONDS,
            "even the shortest testnet deployment carries a real delay"
        );
        _setPublishEnv();

        new PublishProtocolVersionsPropose().run();

        // The delay is real in THIS deployment, not merely stored: executing now reverts. Asserted
        // against the registry directly rather than by running the execute script early, because a
        // reverted `run()` leaves the script's broadcast open and the later, legitimate run would then
        // fail for a cheatcode reason instead of a protocol one.
        bytes32 pendingId = ProtocolVersions.levelBId();
        assertGt(registry.discoverySetEta(pendingId), block.timestamp, "the window is still open");
        vm.prank(DEFAULT_SENDER);
        vm.expectRevert(bytes("timelock"));
        registry.executeDiscoverySet(pendingId);

        vm.warp(block.timestamp + registry.PUBLISH_TIMELOCK());
        new PublishProtocolVersionsExecute().run();

        bytes32 versionId = ProtocolVersions.levelBId();
        bytes32 artifactId = ProtocolVersions.levelBArtifactsId();
        assertEq(registry.discoverySetCount(), 1, "exactly one discovery set");
        assertEq(registry.artifactSetCount(), 1, "exactly one artifact set");
        assertEq(registry.discoverySetList(0), versionId);
        assertEq(registry.artifactSetList(0), artifactId);
        assertEq(registry.activeArtifactSetOf(versionId), artifactId, "both axes must be bound");

        ProtocolRegistry.DiscoverySet memory d = registry.getDiscoverySet(versionId);
        assertEq(d.factory, address(factory));
        assertEq(d.verificationRegistry, address(vr));
        assertEq(d.sbt, address(sbt));
        assertEq(d.verifier, address(verifier));
        assertEq(d.providerRegistry, address(core));
        assertEq(d.circuitId, keccak256("consent.circom/DogTagConsent(6)"));
        assertTrue(d.active);

        ProtocolRegistry.ArtifactSet memory a = registry.getActiveArtifactSet(versionId);
        assertEq(a.artifactSetId, artifactId);
        assertEq(a.zkeySha256, bytes32(uint256(0xA11CE)));
        assertEq(a.witnessMobileSha256, bytes32(uint256(0xB0B)));
        assertEq(a.witnessServerR1csSha256, bytes32(uint256(0xCAFE)));
        assertEq(a.witnessServerWasmSha256, bytes32(uint256(0xD06)));
        assertEq(a.artifactBaseUrl, "https://artifacts.dogtag.test/consent");
        assertEq(a.minAppVersion, "1.6.0", "the app floor is published as given, with no default");
        assertTrue(a.active);
    }

    /// @notice The canonical record, built straight from the fixture. Every negative case below mutates
    /// exactly one member of this and asks `preflight` about it.
    function _canonicalDiscoverySet() internal view returns (ProtocolRegistry.DiscoverySet memory) {
        return ProtocolVersions.levelBDiscovery(
            address(factory),
            address(vr),
            address(sbt),
            address(verifier),
            address(core)
        );
    }

    /// @notice The preflight is the point of the propose phase, so each of its five relations is broken in
    /// turn and must refuse. Without these the preflight could be deleted and the happy path above would
    /// still pass.
    ///
    /// Asserted through `preflight` DIRECTLY rather than by mutating `PUBLISH_*` and running the script.
    /// These five cases are about which relations the preflight enforces, not about env plumbing, and
    /// `preflight` is `public view` and takes the record by value precisely so they can be asked that way.
    /// Driving them through the environment was also unsound as a test: `vm.setEnv` writes the
    /// PROCESS-global environment while forge runs a suite's test functions concurrently, so a
    /// non-canonical value was visible to every other test in this file and the suite failed 8 runs out of
    /// 8 at default threads. Do not restore the env-driven form; the happy-path test above is where env
    /// plumbing belongs, and it covers all thirteen variables end to end.
    ///
    /// The invariant is DIVERGENCE, not abstinence. Tests that run the real scripts still write the
    /// environment, and that is safe because `setUp` is snapshotted, so every test function sees the same
    /// fixture addresses and every `_setPublishEnv` writes byte-identical values. A new test may run the
    /// scripts; it must not write a `PUBLISH_*` value that differs from the canonical set.
    function test_the_preflight_refuses_every_mis_transcribed_address() public {
        PublishProtocolVersionsPropose propose = preflightHarness;
        ProtocolRegistry.DiscoverySet memory d;

        d = _canonicalDiscoverySet();
        d.providerRegistry = address(sbt);
        vm.expectRevert(bytes("providerRegistry != verificationRegistry.providerRegistry()"));
        propose.preflight(d);

        d = _canonicalDiscoverySet();
        d.sbt = address(core);
        vm.expectRevert(bytes("sbt != verificationRegistry.sbt()"));
        propose.preflight(d);

        d = _canonicalDiscoverySet();
        d.verifier = address(sbt);
        vm.expectRevert(bytes("verifier != verificationRegistry.zkVerifier()"));
        propose.preflight(d);

        // The most consequential slip of all: a genuine, correctly-built factory that is simply not the
        // one this verification registry resolves roots through. Every clone it makes is a real clone
        // and not one of its roots would ever verify.
        DogTagIssuerFactory stranger = new DogTagIssuerFactory(address(implementation), address(core));
        d = _canonicalDiscoverySet();
        d.factory = address(stranger);
        vm.expectRevert(bytes("factory != verificationRegistry.rootIndex()"));
        propose.preflight(d);

        // The positive control: unmutated, the same call passes, so none of the above refused for a stray
        // reason such as an unrelated relation already being broken in the fixture.
        propose.preflight(_canonicalDiscoverySet());
    }

    /// @notice The one relation that can drift INSIDE the publish window, driven for real rather than
    /// asserted: `zkVerifier` is the verification registry's only mutable member and its own timelock is
    /// 2 days, the same length as the mainnet publish timelock, so a swap proposed shortly before a
    /// publish executes squarely inside that window. The other three relations are immutable slots, so
    /// this is the whole of what the execute-phase re-preflight buys.
    ///
    /// Every `vm.setEnv` here writes the canonical values, deliberately: `vm.setEnv` mutates the PROCESS
    /// environment, so a test that wrote a divergent one would race every other test in this file.
    function test_execute_refuses_a_verifier_that_moved_inside_the_publish_window() public {
        ProtocolRegistry registry = protocolRegistry;
        _setPublishEnv();
        // Both scripts are constructed UP FRONT: `vm.expectRevert` applies to the next call, and a
        // `new ...()` on the same statement would consume it, so the refusals below would be asserted
        // against a contract creation that never reverts.
        PublishProtocolVersionsPropose propose = new PublishProtocolVersionsPropose();
        PublishProtocolVersionsExecute execute = new PublishProtocolVersionsExecute();
        propose.run();

        // The registry's REAL swap, through its own timelock - not a storage poke.
        Groth16VerifierConsent replacement = new Groth16VerifierConsent();
        assertTrue(address(replacement) != address(verifier), "the swap must actually change the address");
        vm.prank(GOV);
        vr.proposeZkVerifier(address(replacement));
        vm.warp(block.timestamp + 2 days);
        vm.prank(GOV);
        vr.executeZkVerifier();

        // Asserted BEFORE the refusal: a setup that silently failed would leave the old verifier in
        // place, the preflight would pass, and the refusal below would then have to come from somewhere
        // else - so this test would be pinning nothing.
        assertEq(
            address(vr.zkVerifier()),
            address(replacement),
            "the verifier really moved on-chain"
        );
        bytes32 versionId = ProtocolVersions.levelBId();
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
    /// `PUBLISH_VERIFIER` - while the retired verifier stayed staged and got published as certified.
    ///
    /// The divergence is created by RE-STAGING a different record straight on the registry rather than by
    /// mutating `PUBLISH_VERIFIER`: `vm.setEnv` writes the process-global environment, so a divergent value
    /// would race every other test in this file. The environment stays canonical throughout, which also
    /// makes the refusal attributable - a preflight over that environment would pass.
    function test_the_execute_phase_refuses_a_record_re_staged_since_the_environment_was_read() public {
        ProtocolRegistry registry = protocolRegistry;
        _setPublishEnv();
        PublishProtocolVersionsExecute execute = new PublishProtocolVersionsExecute();
        new PublishProtocolVersionsPropose().run();

        // Same id, one different member - so this REPLACES what `executeDiscoverySet` would write.
        ProtocolRegistry.DiscoverySet memory reStaged = ProtocolVersions.levelBDiscovery(
            address(factory),
            address(vr),
            address(sbt),
            address(new Groth16VerifierConsent()),
            address(core)
        );
        vm.prank(DEFAULT_SENDER);
        registry.proposeDiscoverySet(reStaged);
        assertEq(
            registry.getPendingDiscoverySet(ProtocolVersions.levelBId()).verifier,
            reStaged.verifier,
            "the staged record really was replaced"
        );

        vm.warp(block.timestamp + registry.PUBLISH_TIMELOCK());
        vm.expectRevert(bytes("staged discovery set differs from this environment"));
        execute.run();
    }

    /// @notice The ARTIFACT-axis twin of the test above, so neither staged-versus-env check is vacuous.
    ///
    /// This axis has no preflight to be misled - pins are byte-integrity and cannot be checked on-chain -
    /// so what the check protects is the ONE combined verdict phase 2 prints. Without it an operator who
    /// edited a pin between the phases would read that verdict over a publication of the OLD staged pins,
    /// and only `minAppVersion` would have been visible in the closing log.
    ///
    /// The zkey pin is the member driven here because it is the one a swap would make unrecoverable: a
    /// zkey proving against a different VK is exactly what the pin exists to refuse.
    function test_the_execute_phase_refuses_artifact_pins_re_staged_since_the_environment_was_read() public {
        ProtocolRegistry registry = protocolRegistry;
        _setPublishEnv();
        PublishProtocolVersionsExecute execute = new PublishProtocolVersionsExecute();
        new PublishProtocolVersionsPropose().run();

        // Same artifactSetId, one different pin - so this REPLACES what `executeArtifactSet` would write.
        ProtocolRegistry.ArtifactSet memory reStaged = ProtocolVersions.levelBArtifacts(
            bytes32(uint256(0xDEADBEEF)),
            bytes32(uint256(0xB0B)),
            bytes32(uint256(0xCAFE)),
            bytes32(uint256(0xD06)),
            "https://artifacts.dogtag.test/consent",
            "1.6.0"
        );
        vm.prank(DEFAULT_SENDER);
        registry.proposeArtifactSet(reStaged);
        assertEq(
            registry.getPendingArtifactSet(ProtocolVersions.levelBArtifactsId()).zkeySha256,
            reStaged.zkeySha256,
            "the staged artifact set really was replaced"
        );

        vm.warp(block.timestamp + registry.PUBLISH_TIMELOCK());
        vm.expectRevert(bytes("staged zkeySha256 differs from this environment"));
        execute.run();
    }
}
