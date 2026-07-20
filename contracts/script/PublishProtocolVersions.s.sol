// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";
import {ProtocolVersions} from "./ProtocolVersions.sol";

/// @notice M7 P3: publish `dogtag-levela/1` + `dogtag-levelb/1` to the `ProtocolRegistry` (§5, §7.1 P8).
/// The record DATA is in `ProtocolVersions` — the single source of truth this and the registry test
/// share.
///
/// # Two axes (R-5), so each level publishes THREE things
///
/// A level is a [`ProtocolRegistry.ContractSet`] (trio + verifier + circuitId), a
/// [`ProtocolRegistry.ArtifactSet`] (pins + base URL + minAppVersion), and the BINDING between them.
/// All three writes are timelocked, but their timelocks run CONCURRENTLY: phase 1 proposes all six
/// records plus both bindings in one transaction batch, so this is still a TWO-phase rollout.
///
/// Publishing mirrors `proposeZkVerifier`/`executeZkVerifier`:
///   1. `PublishProtocolVersionsPropose` — stages both levels (sets + bindings). Starts the 2-day
///      `PUBLISH_TIMELOCK` on all of them at once.
///   2. `PublishProtocolVersionsExecute` — run AFTER the timelock elapses. Executes the sets FIRST,
///      then the bindings (`executeArtifactBinding` requires both sides to already be published).
/// Both are `PUBLISHER_ROLE`-gated (broadcast with the publisher key) and read the deployed registry
/// address from the `PROTOCOL_REGISTRY` env var.
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
}

/// @notice Phase 1 — propose both levels on both axes, plus their bindings (starts the 2-day timelock).
///   forge script .../PublishProtocolVersions.s.sol:PublishProtocolVersionsPropose \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
contract PublishProtocolVersionsPropose is PublishBase {
    function run() external {
        ProtocolRegistry reg = _registry();
        console2.log("--- Phase 1: PROPOSE contract sets + artifact sets + bindings (2-day timelock) ---");
        console2.log("ProtocolRegistry", address(reg));

        vm.startBroadcast();
        reg.proposeContractSet(ProtocolVersions.levelAContracts());
        reg.proposeContractSet(ProtocolVersions.levelBContracts());
        reg.proposeArtifactSet(ProtocolVersions.levelAArtifacts());
        reg.proposeArtifactSet(ProtocolVersions.levelBArtifacts());
        reg.proposeArtifactBinding(ProtocolVersions.levelAId(), ProtocolVersions.levelAArtifactsId());
        reg.proposeArtifactBinding(ProtocolVersions.levelBId(), ProtocolVersions.levelBArtifactsId());
        vm.stopBroadcast();

        console2.log("levela/1 contract-set ETA (unix)", reg.contractSetEta(ProtocolVersions.levelAId()));
        console2.log("levelb/1 contract-set ETA (unix)", reg.contractSetEta(ProtocolVersions.levelBId()));
        console2.log("levela artifacts ETA (unix)", reg.artifactSetEta(ProtocolVersions.levelAArtifactsId()));
        console2.log("levelb artifacts ETA (unix)", reg.artifactSetEta(ProtocolVersions.levelBArtifactsId()));
        console2.log("levela binding ETA (unix)", reg.bindingEta(ProtocolVersions.levelAId()));
        console2.log("levelb binding ETA (unix)", reg.bindingEta(ProtocolVersions.levelBId()));
        console2.log("Next: after the timelock, run PublishProtocolVersionsExecute.");
    }
}

/// @notice Phase 2 — execute both levels (only valid AFTER the timelock). Sets first, then bindings:
/// `executeArtifactBinding` requires both sides to be published already.
///   forge script .../PublishProtocolVersions.s.sol:PublishProtocolVersionsExecute \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
contract PublishProtocolVersionsExecute is PublishBase {
    function run() external {
        ProtocolRegistry reg = _registry();
        console2.log("--- Phase 2: EXECUTE sets, then bindings (after timelock) ---");
        console2.log("ProtocolRegistry", address(reg));

        vm.startBroadcast();
        reg.executeContractSet(ProtocolVersions.levelAId());
        reg.executeContractSet(ProtocolVersions.levelBId());
        reg.executeArtifactSet(ProtocolVersions.levelAArtifactsId());
        reg.executeArtifactSet(ProtocolVersions.levelBArtifactsId());
        reg.executeArtifactBinding(ProtocolVersions.levelAId());
        reg.executeArtifactBinding(ProtocolVersions.levelBId());
        vm.stopBroadcast();

        console2.log("Published contract sets:", reg.contractSetCount());
        console2.log("Published artifact sets:", reg.artifactSetCount());
        console2.log("levela/1 active", reg.getContractSet(ProtocolVersions.levelAId()).active);
        console2.log("levelb/1 active", reg.getContractSet(ProtocolVersions.levelBId()).active);
        console2.log(
            "levela/1 minAppVersion", reg.getActiveArtifactSet(ProtocolVersions.levelAId()).minAppVersion
        );
        console2.log(
            "levelb/1 minAppVersion", reg.getActiveArtifactSet(ProtocolVersions.levelBId()).minAppVersion
        );
    }
}
