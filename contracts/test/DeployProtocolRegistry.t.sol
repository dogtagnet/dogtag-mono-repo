// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";
import {DeployProtocolRegistry} from "../script/DeployProtocolRegistry.s.sol";
import {
    PublishProtocolVersionsPropose,
    PublishProtocolVersionsExecute
} from "../script/PublishProtocolVersions.s.sol";
import {ProtocolVersions} from "../script/ProtocolVersions.sol";

contract DeployProtocolRegistryTest is Test {
    DeployProtocolRegistry internal deployer;

    function setUp() public {
        deployer = new DeployProtocolRegistry();
    }

    function test_mainnet_guard_accepts_only_two_day_default() public view {
        deployer.validatePublishTimelock(2 days, false);
    }

    function test_mainnet_guard_refuses_sub_two_day_timelock_without_testnet_opt_in() public {
        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        deployer.validatePublishTimelock(0, false);

        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        deployer.validatePublishTimelock(2 days - 1, false);

        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        deployer.validatePublishTimelock(2 days + 1, false);
    }

    function test_explicit_testnet_opt_in_accepts_zero_timelock() public view {
        deployer.validatePublishTimelock(0, true);
    }

    function test_env_driven_scripts_publish_one_version_on_both_axes() public {
        address factory = address(0xFACADE);
        address verificationRegistry = address(0xC0DE);
        address sbt = address(0x5B7);
        address verifier = address(0xBEEF);
        bytes32 zkeySha256 = bytes32(uint256(0xA11CE));
        bytes32 witnessMobileSha256 = bytes32(uint256(0xB0B));
        bytes32 r1csSha256 = bytes32(uint256(0xCAFE));
        bytes32 wasmSha256 = bytes32(uint256(0xD06));
        string memory artifactBaseUrl = "https://artifacts.dogtag.test/consent";

        // Exercise the real deploy script's explicit disposable-testnet path.
        vm.setEnv("ADMIN", vm.toString(DEFAULT_SENDER));
        vm.setEnv("PUBLISHER", vm.toString(DEFAULT_SENDER));
        vm.setEnv("PUBLISH_TIMELOCK_SECS", "0");
        vm.setEnv("TESTNET_DEPLOY", "true");
        deployer.run();

        ProtocolRegistry registry = ProtocolRegistry(deployer.protocolRegistry());
        assertEq(registry.PUBLISH_TIMELOCK(), 0, "testnet deploy must be immediately publishable");

        vm.setEnv("PROTOCOL_REGISTRY", vm.toString(address(registry)));
        vm.setEnv("ROOT_INDEX", vm.toString(factory));
        vm.setEnv("VERIFICATION_REGISTRY_CONSENT_ADDR", vm.toString(verificationRegistry));
        vm.setEnv("SBT_CONSENT_ADDR", vm.toString(sbt));
        vm.setEnv("CONSENT_VERIFIER", vm.toString(verifier));
        vm.setEnv("CONSENT_ZKEY_SHA256", vm.toString(zkeySha256));
        vm.setEnv("CONSENT_WITNESS_MOBILE_SHA256", vm.toString(witnessMobileSha256));
        vm.setEnv("CONSENT_R1CS_SHA256", vm.toString(r1csSha256));
        vm.setEnv("CONSENT_WASM_SHA256", vm.toString(wasmSha256));
        vm.setEnv("DOGTAG_ARTIFACTS_URL", artifactBaseUrl);

        new PublishProtocolVersionsPropose().run();
        new PublishProtocolVersionsExecute().run();

        bytes32 versionId = ProtocolVersions.levelBId();
        bytes32 artifactId = ProtocolVersions.levelBArtifactsId();
        assertEq(registry.contractSetCount(), 1, "exactly one contract set");
        assertEq(registry.artifactSetCount(), 1, "exactly one artifact set");
        assertEq(registry.contractSetList(0), versionId);
        assertEq(registry.artifactSetList(0), artifactId);
        assertEq(registry.activeArtifactSetOf(versionId), artifactId, "both axes must be bound");

        ProtocolRegistry.ContractSet memory contracts_ = registry.getContractSet(versionId);
        assertEq(contracts_.factory, factory);
        assertEq(contracts_.verificationRegistry, verificationRegistry);
        assertEq(contracts_.sbt, sbt);
        assertEq(contracts_.verifier, verifier);
        assertEq(contracts_.circuitId, keccak256("consent.circom/DogTagConsent(6)"));
        assertTrue(contracts_.active);

        ProtocolRegistry.ArtifactSet memory artifacts = registry.getActiveArtifactSet(versionId);
        assertEq(artifacts.artifactSetId, artifactId);
        assertEq(artifacts.zkeySha256, zkeySha256);
        assertEq(artifacts.witnessMobileSha256, witnessMobileSha256);
        assertEq(artifacts.witnessServerR1csSha256, r1csSha256);
        assertEq(artifacts.witnessServerWasmSha256, wasmSha256);
        assertEq(artifacts.artifactBaseUrl, artifactBaseUrl);
        assertEq(artifacts.minAppVersion, "1.4.0");
        assertTrue(artifacts.active);
    }
}
