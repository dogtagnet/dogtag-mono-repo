// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";

/// @notice M7 P3: deploy the dogtag-governed `ProtocolRegistry` — the on-chain discovery TRUST ANCHOR
/// (§5.1, lock B). ADDITIVE: it references the existing trio/verifier addresses but deploys and changes
/// NOTHING else; the frozen circuit/VK/ceremony are untouched.
///
/// `admin` receives `DEFAULT_ADMIN_ROLE` (under the 2-day ACDAR timelock) and alone may grant/revoke
/// `PUBLISHER_ROLE`. `publisher` receives `PUBLISHER_ROLE` (may propose/execute/deprecate versions).
/// Both default to the Phase-2 governance authority `0x8E27E117…` (deployments/roax.json `_governance`),
/// NOT the former deployer EOA.
///
/// After deploy, record the address in `deployments/roax.json` under `ProtocolRegistry`, then run
/// `PublishProtocolVersions.s.sol` (Propose phase, then Execute after the 2-day timelock).
///
/// Broadcast with the governance signer:
///   forge script contracts/script/DeployProtocolRegistry.s.sol:DeployProtocolRegistry \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $GOV_KEY
contract DeployProtocolRegistry is Script {
    address public protocolRegistry; // NEW

    function run() external {
        address admin = vm.envOr("ADMIN", 0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2);
        address publisher = vm.envOr("PUBLISHER", admin);
        require(admin != address(0) && publisher != address(0), "zero admin/publisher");

        vm.startBroadcast();
        protocolRegistry = address(new ProtocolRegistry(admin, publisher));
        vm.stopBroadcast();

        console2.log("--- ProtocolRegistry (M7 P3, additive) ---");
        console2.log("ProtocolRegistry   ", protocolRegistry);
        console2.log("Admin (governance) ", admin);
        console2.log("Publisher          ", publisher);
        console2.log("");
        console2.log("Record the address in deployments/roax.json under 'ProtocolRegistry', then run");
        console2.log("PublishProtocolVersions (Propose), and Execute after the 2-day timelock.");
    }
}
