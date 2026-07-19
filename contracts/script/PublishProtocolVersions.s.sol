// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";
import {ProtocolVersions} from "./ProtocolVersions.sol";

/// @notice M7 P3: publish `dogtag-levela/1` + `dogtag-levelb/1` to the `ProtocolRegistry` (§5, §7.1 P8).
/// The version DATA (trio addresses + frozen artifact pins) is in `ProtocolVersions` — the single
/// source of truth this and the registry test share.
///
/// Publishing is TIMELOCKED (mirrors `proposeZkVerifier`/`executeZkVerifier`), so it is TWO phases:
///   1. `PublishProtocolVersionsPropose` — stages both versions. Starts the 2-day `PUBLISH_TIMELOCK`.
///   2. `PublishProtocolVersionsExecute` — run AFTER the timelock elapses; writes both versions live.
/// Both are `PUBLISHER_ROLE`-gated (broadcast with the publisher key) and read the deployed registry
/// address from the `PROTOCOL_REGISTRY` env var.
abstract contract PublishBase is Script {
    function _registry() internal view returns (ProtocolRegistry) {
        return ProtocolRegistry(vm.envAddress("PROTOCOL_REGISTRY"));
    }
}

/// @notice Phase 1 — propose both current versions (starts the 2-day timelock).
///   forge script .../PublishProtocolVersions.s.sol:PublishProtocolVersionsPropose \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
contract PublishProtocolVersionsPropose is PublishBase {
    function run() external {
        ProtocolRegistry reg = _registry();
        console2.log("--- Phase 1: PROPOSE versions (starts 2-day timelock) ---");
        console2.log("ProtocolRegistry", address(reg));

        vm.startBroadcast();
        reg.proposeVersion(ProtocolVersions.levelA());
        reg.proposeVersion(ProtocolVersions.levelB());
        vm.stopBroadcast();

        console2.log("levela/1 execute ETA (unix)", reg.publishEta(ProtocolVersions.levelAId()));
        console2.log("levelb/1 execute ETA (unix)", reg.publishEta(ProtocolVersions.levelBId()));
        console2.log("Next: after the timelock, run PublishProtocolVersionsExecute.");
    }
}

/// @notice Phase 2 — execute both versions (only valid AFTER the timelock).
///   forge script .../PublishProtocolVersions.s.sol:PublishProtocolVersionsExecute \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
contract PublishProtocolVersionsExecute is PublishBase {
    function run() external {
        ProtocolRegistry reg = _registry();
        console2.log("--- Phase 2: EXECUTE versions (after timelock) ---");
        console2.log("ProtocolRegistry", address(reg));

        vm.startBroadcast();
        reg.executeVersion(ProtocolVersions.levelAId());
        reg.executeVersion(ProtocolVersions.levelBId());
        vm.stopBroadcast();

        console2.log("Published versions:", reg.versionCount());
        console2.log("levela/1 active", reg.getVersion(ProtocolVersions.levelAId()).active);
        console2.log("levelb/1 active", reg.getVersion(ProtocolVersions.levelBId()).active);
    }
}
