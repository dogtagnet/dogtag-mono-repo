// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";
import {ProtocolVersions} from "./ProtocolVersions.sol";

/// @notice Publishes the single owner-hidden `dogtag-levelb/1` version to the `ProtocolRegistry`.
/// The record DATA is in `ProtocolVersions` — the single source of truth this and the registry test
/// share.
///
/// # Two axes, so the version publishes THREE things
///
/// The version is a [`ProtocolRegistry.ContractSet`] (trio + verifier + circuitId), a
/// [`ProtocolRegistry.ArtifactSet`] (pins + base URL + minAppVersion), and the BINDING between them.
/// All three writes are timelocked, but their timelocks run CONCURRENTLY: phase 1 proposes both sets
/// plus their binding in one transaction batch, so this is still a TWO-phase rollout.
///
/// Publishing mirrors `proposeZkVerifier`/`executeZkVerifier`:
///   1. `PublishProtocolVersionsPropose` — stages both sets plus the binding. Starts the registry's
///      immutable `PUBLISH_TIMELOCK` on all three at once.
///   2. `PublishProtocolVersionsExecute` — run AFTER the timelock elapses. Executes the sets FIRST,
///      then the bindings (`executeArtifactBinding` requires both sides to already be published).
/// Both are `PUBLISHER_ROLE`-gated (broadcast with the publisher key) and read the deployed registry
/// address from the `PROTOCOL_REGISTRY` env var.
///
/// The freshly deployed addresses are mandatory `ROOT_INDEX`, `VERIFICATION_REGISTRY_CONSENT_ADDR`,
/// `SBT_CONSENT_ADDR`, and `CONSENT_VERIFIER` env vars. Artifact publication is likewise explicit via
/// `CONSENT_ZKEY_SHA256`, `CONSENT_WITNESS_MOBILE_SHA256`, `CONSENT_R1CS_SHA256`,
/// `CONSENT_WASM_SHA256`, and `DOGTAG_ARTIFACTS_URL`; there are no stale-network fallbacks.
///
/// # Rotating artifacts later
///
/// A zkey rotation does NOT re-run this script. It is: `proposeArtifactSet(<new …-artifacts/2>)` +
/// `proposeArtifactBinding(levelBId(), levelBArtifactsV2Id())`, wait the timelock, then the two
/// executes. No `ContractSet` is written, so no trio address moves and nothing is redeployed.
abstract contract PublishBase is Script {
    function _registry() internal view returns (ProtocolRegistry) {
        return ProtocolRegistry(vm.envAddress("PROTOCOL_REGISTRY"));
    }

    function _contractSet() internal view returns (ProtocolRegistry.ContractSet memory) {
        return ProtocolVersions.levelBContracts(
            vm.envAddress("ROOT_INDEX"),
            vm.envAddress("VERIFICATION_REGISTRY_CONSENT_ADDR"),
            vm.envAddress("SBT_CONSENT_ADDR"),
            vm.envAddress("CONSENT_VERIFIER")
        );
    }

    function _artifactSet() internal view returns (ProtocolRegistry.ArtifactSet memory) {
        return ProtocolVersions.levelBArtifacts(
            vm.envBytes32("CONSENT_ZKEY_SHA256"),
            vm.envBytes32("CONSENT_WITNESS_MOBILE_SHA256"),
            vm.envBytes32("CONSENT_R1CS_SHA256"),
            vm.envBytes32("CONSENT_WASM_SHA256"),
            vm.envString("DOGTAG_ARTIFACTS_URL")
        );
    }
}

/// @notice Phase 1 — propose the version on both axes, plus its binding (starts the timelock).
///   forge script .../PublishProtocolVersions.s.sol:PublishProtocolVersionsPropose \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
contract PublishProtocolVersionsPropose is PublishBase {
    function run() external {
        ProtocolRegistry reg = _registry();
        ProtocolRegistry.ContractSet memory contractSet = _contractSet();
        ProtocolRegistry.ArtifactSet memory artifactSet = _artifactSet();
        console2.log("--- Phase 1: PROPOSE contract sets + artifact sets + bindings ---");
        console2.log("ProtocolRegistry", address(reg));
        console2.log("Publish timelock (seconds)", reg.PUBLISH_TIMELOCK());

        vm.startBroadcast();
        reg.proposeContractSet(contractSet);
        reg.proposeArtifactSet(artifactSet);
        reg.proposeArtifactBinding(contractSet.contractSetId, artifactSet.artifactSetId);
        vm.stopBroadcast();

        console2.log("dogtag-levelb/1 contract-set ETA (unix)", reg.contractSetEta(contractSet.contractSetId));
        console2.log("artifact-set ETA (unix)", reg.artifactSetEta(artifactSet.artifactSetId));
        console2.log("binding ETA (unix)", reg.bindingEta(contractSet.contractSetId));
        console2.log("Next: after the timelock, run PublishProtocolVersionsExecute.");
    }
}

/// @notice Phase 2 — execute the version (only valid AFTER the timelock). Sets first, then binding:
/// `executeArtifactBinding` requires both sides to be published already.
///   forge script .../PublishProtocolVersions.s.sol:PublishProtocolVersionsExecute \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
contract PublishProtocolVersionsExecute is PublishBase {
    function run() external {
        ProtocolRegistry reg = _registry();
        console2.log("--- Phase 2: EXECUTE sets, then bindings (after timelock) ---");
        console2.log("ProtocolRegistry", address(reg));

        vm.startBroadcast();
        reg.executeContractSet(ProtocolVersions.levelBId());
        reg.executeArtifactSet(ProtocolVersions.levelBArtifactsId());
        reg.executeArtifactBinding(ProtocolVersions.levelBId());
        vm.stopBroadcast();

        console2.log("Published contract sets:", reg.contractSetCount());
        console2.log("Published artifact sets:", reg.artifactSetCount());
        console2.log("dogtag-levelb/1 active", reg.getContractSet(ProtocolVersions.levelBId()).active);
        console2.log(
            "dogtag-levelb/1 minAppVersion",
            reg.getActiveArtifactSet(ProtocolVersions.levelBId()).minAppVersion
        );
    }
}
