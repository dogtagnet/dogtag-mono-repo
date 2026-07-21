// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {ProtocolRegistry, DEFAULT_PUBLISH_TIMELOCK_SECONDS} from "../src/ProtocolRegistry.sol";

/// @notice M7 P3: deploy the dogtag-governed `ProtocolRegistry` — the on-chain discovery TRUST ANCHOR
/// (§5.1, lock B). ADDITIVE: it references the existing trio/verifier addresses but deploys and changes
/// NOTHING else; the frozen circuit/VK/ceremony are untouched.
///
/// `admin` receives `DEFAULT_ADMIN_ROLE` (under the 2-day ACDAR timelock) and alone may grant/revoke
/// `PUBLISHER_ROLE`. `publisher` receives `PUBLISHER_ROLE` (may propose/execute/deprecate versions).
/// Both default to the Phase-2 governance authority `0x8E27E117…` (deployments/roax.json `_governance`),
/// NOT the former deployer EOA.
///
/// The publish delay defaults to the production-safe 2 days. A different `PUBLISH_TIMELOCK_SECS` is
/// accepted ONLY with the loud `TESTNET_DEPLOY=true` opt-in; ROAX uses that testnet path. This guard is
/// deliberately in the deploy script so a mainnet operator cannot accidentally inherit a zero delay.
///
/// After deploy, record the address in `deployments/roax.json` under `ProtocolRegistry`, then run
/// `PublishProtocolVersions.s.sol` (Propose phase, then Execute once the configured timelock elapses).
///
/// Broadcast with the governance signer:
///   forge script contracts/script/DeployProtocolRegistry.s.sol:DeployProtocolRegistry \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $GOV_KEY
contract DeployProtocolRegistry is Script {
    uint256 public constant MAINNET_PUBLISH_TIMELOCK = DEFAULT_PUBLISH_TIMELOCK_SECONDS;

    address public protocolRegistry; // NEW

    function run() external {
        address admin = vm.envOr("ADMIN", 0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2);
        address publisher = vm.envOr("PUBLISHER", admin);
        uint256 publishTimelock = vm.envOr("PUBLISH_TIMELOCK_SECS", MAINNET_PUBLISH_TIMELOCK);
        bool testnetDeploy = vm.envOr("TESTNET_DEPLOY", false);
        require(admin != address(0) && publisher != address(0), "zero admin/publisher");
        validatePublishTimelock(publishTimelock, testnetDeploy);

        vm.startBroadcast();
        protocolRegistry = address(new ProtocolRegistry(admin, publisher, publishTimelock));
        vm.stopBroadcast();

        console2.log("--- ProtocolRegistry (M7 P3, additive) ---");
        console2.log("ProtocolRegistry   ", protocolRegistry);
        console2.log("Admin (governance) ", admin);
        console2.log("Publisher          ", publisher);
        console2.log("Publish timelock   ", publishTimelock, "seconds");
        console2.log("Testnet deploy     ", testnetDeploy);
        console2.log("");
        console2.log("Record the address in deployments/roax.json under 'ProtocolRegistry', then run");
        console2.log("PublishProtocolVersions (Propose), and Execute once its ETA is reached.");
    }

    /// @notice LOUD production guard: without an explicit testnet opt-in, the delay must be exactly
    /// the governance-approved 2-day default. Keeping this public also makes the deploy invariant
    /// directly testable without broadcasting a deployment.
    function validatePublishTimelock(uint256 publishTimelock, bool testnetDeploy) public pure {
        if (!testnetDeploy) {
            require(publishTimelock == MAINNET_PUBLISH_TIMELOCK, "mainnet publish timelock must be 2 days");
        }
    }
}
