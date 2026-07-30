// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {
    ProtocolRegistryV2,
    DEFAULT_PUBLISH_TIMELOCK_SECONDS_V2,
    MIN_PUBLISH_TIMELOCK_SECONDS_V2
} from "../src/ProtocolRegistryV2.sol";

/// @notice Deploy the generation-2 discovery registry. ADDITIVE: it references the generation-2
/// component addresses but deploys and changes nothing else, and it publishes nothing — run
/// `PublishProtocolVersionsV2.s.sol` afterwards.
///
/// `admin` receives `DEFAULT_ADMIN_ROLE` (under the 2-day ACDAR handover) and alone may grant/revoke
/// `PUBLISHER_ROLE`. `publisher` receives `PUBLISHER_ROLE`, and defaults to `admin` only because a
/// single-holder deployment is a legitimate starting state that the ACDAR handover can then split.
///
/// `GEN2_ADMIN` is MANDATORY here, with no default — the deliberate divergence from generation 1's
/// script, which defaults its `ADMIN` to the current governance EOA. That EOA has no contract code, and
/// it holds `DEFAULT_ADMIN_ROLE` on the issuer registry, the verification registry, the SBT and the
/// generation-1 discovery registry, plus `WHITELIST_ADMIN` and factory ownership. Inheriting it by
/// default would quietly reproduce that concentration on the one deployment whose whole purpose is to
/// harden the pointer. Naming the holder is the decision this deployment exists to make, so the script
/// refuses to make it silently.
///
/// Every variable is namespaced `GEN2_` — `GEN2_ADMIN`, `GEN2_PUBLISHER`, `GEN2_PUBLISH_TIMELOCK_SECS`,
/// `GEN2_TESTNET_DEPLOY` — for the reasons set out on `PublishProtocolVersionsV2.s.sol`. The one that
/// matters most here: generation 1's ROAX deployment sets `PUBLISH_TIMELOCK_SECS=0` and
/// `TESTNET_DEPLOY=true`, and an environment carrying those must not be able to reach this script's
/// timelock decision at all.
///
/// # The delay this script exists to get right
///
/// The generation-1 registry is live with `PUBLISH_TIMELOCK == 0` and the value is `immutable`, so its
/// publisher key can repoint the whole declared protocol set in a single block and no redeploy-free fix
/// exists. This deployment is the only opportunity to correct that, which is why the delay is treated as
/// a decision to state rather than a constant to inherit.
///
/// Two guards, at different layers, and neither is redundant:
///   * The CONTRACT refuses anything below `MIN_PUBLISH_TIMELOCK` (1 hour). That is what makes a
///     zero-delay registry impossible even for someone deploying the contract directly, without this
///     script.
///   * This SCRIPT additionally requires exactly the 2-day production default unless
///     `GEN2_TESTNET_DEPLOY=true` is stated aloud. A testnet may go shorter — but not below the
///     contract's floor, and never to zero.
/// Deliberately no `GEN2_PUBLISH_TIMELOCK_SECS` default that a mainnet operator could inherit by
/// accident: the env var is optional only because the default IS the production value.
///
/// Broadcast with the governance signer:
///   forge script contracts/script/DeployProtocolRegistryV2.s.sol:DeployProtocolRegistryV2 \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $GOV_KEY
contract DeployProtocolRegistryV2 is Script {
    /// @notice The delay a non-testnet deployment must use, exactly. Single-sourced from the contract.
    uint256 public constant MAINNET_PUBLISH_TIMELOCK = DEFAULT_PUBLISH_TIMELOCK_SECONDS_V2;

    /// @notice The floor the CONTRACT enforces. Mirrored here so this script's own guard can explain a
    /// rejection in terms of the same number the constructor would revert on.
    uint256 public constant MIN_PUBLISH_TIMELOCK = MIN_PUBLISH_TIMELOCK_SECONDS_V2;

    address public protocolRegistryV2; // NEW

    function run() external {
        address admin = vm.envAddress("GEN2_ADMIN");
        address publisher = vm.envOr("GEN2_PUBLISHER", admin);
        uint256 publishTimelock = vm.envOr("GEN2_PUBLISH_TIMELOCK_SECS", MAINNET_PUBLISH_TIMELOCK);
        bool testnetDeploy = vm.envOr("GEN2_TESTNET_DEPLOY", false);
        require(admin != address(0) && publisher != address(0), "zero admin/publisher");
        validatePublishTimelock(publishTimelock, testnetDeploy);

        vm.startBroadcast();
        protocolRegistryV2 = address(new ProtocolRegistryV2(admin, publisher, publishTimelock));
        vm.stopBroadcast();

        console2.log("--- ProtocolRegistryV2 (generation-2 discovery anchor) ---");
        console2.log("ProtocolRegistryV2 ", protocolRegistryV2);
        console2.log("Admin (governance) ", admin);
        console2.log("Publisher          ", publisher);
        console2.log("Publish timelock   ", publishTimelock, "seconds");
        console2.log("Contract floor     ", MIN_PUBLISH_TIMELOCK, "seconds");
        console2.log("Testnet deploy     ", testnetDeploy);
        console2.log("");
        console2.log("Record the address in deployments/roax.json under 'ProtocolRegistryV2', then run");
        console2.log("PublishProtocolVersionsV2 (Propose), and Execute once its ETA is reached.");
    }

    /// @notice LOUD production guard: without an explicit testnet opt-in the delay must be exactly the
    /// governance-approved 2-day default; with one it may be shorter but never below the contract's floor.
    ///
    /// The floor is re-checked here even though the constructor enforces it, so an operator learns from
    /// the script — which knows whether they claimed a testnet — rather than from a constructor revert
    /// during a broadcast. Public and pure so the invariant is testable without deploying anything.
    function validatePublishTimelock(uint256 publishTimelock, bool testnetDeploy) public pure {
        if (!testnetDeploy) {
            require(publishTimelock == MAINNET_PUBLISH_TIMELOCK, "mainnet publish timelock must be 2 days");
            return;
        }
        require(publishTimelock >= MIN_PUBLISH_TIMELOCK, "testnet publish timelock below the 1-hour floor");
    }
}
