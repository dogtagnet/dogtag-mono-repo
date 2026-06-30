// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {GovernanceMigration} from "./GovernanceMigration.sol";
import {
    IAccessControlDefaultAdminRules
} from "@openzeppelin/contracts/access/extensions/IAccessControlDefaultAdminRules.sol";

/// @notice Reads the live address book from `deployments/roax.json` and the target multisig from the
/// `MULTISIG` env var. Shared by both migration phases.
/// @dev DESTRUCTIVE / IRREVERSIBLE on a live network — do NOT run against ROAX without the captain's
/// explicit approval and the chosen multisig. The phases exist precisely so the hand-off is reviewable
/// and time-locked; see docs/architecture.md §13.1 (H-3) and docs/GOVERNANCE_MIGRATION.md.
abstract contract MigrateGovernanceBase is Script {
    function _targets() internal view returns (GovernanceMigration.Targets memory t, address multisig) {
        string memory json = vm.readFile("deployments/roax.json");
        t = GovernanceMigration.Targets({
            issuerRegistry: vm.parseJsonAddress(json, ".IssuerRegistry"),
            verificationRegistry: vm.parseJsonAddress(json, ".VerificationRegistry"),
            sbt: vm.parseJsonAddress(json, ".DogTagSBT"),
            factory: vm.parseJsonAddress(json, ".DogTagIssuerFactory")
        });
        multisig = vm.envAddress("MULTISIG");
    }

    function _report(GovernanceMigration.Targets memory t, address multisig) internal view {
        console2.log("MULTISIG (target admin)", multisig);
        console2.log("IssuerRegistry         ", t.issuerRegistry);
        console2.log("VerificationRegistry   ", t.verificationRegistry);
        console2.log(
            "DogTagSBT              ",
            t.sbt,
            GovernanceMigration.supportsTwoStep(t.sbt) ? "(two-step)" : "(legacy/atomic)"
        );
        console2.log("DogTagIssuerFactory    ", t.factory);
    }
}

/// @notice Phase 1: broadcast by the CURRENT deployer EOA admin (default `--sender`/`--private-key`).
/// Starts the two-step admin transfer everywhere and pre-grants the multisig WHITELIST_ADMIN.
/// After this, the timelocks (3 days IssuerRegistry/SBT, 2 days VerificationRegistry) must elapse before
/// the multisig can run Phase 2. Re-running before acceptance simply re-arms the same pending transfer.
contract MigrateGovernanceBegin is MigrateGovernanceBase {
    function run() external {
        (GovernanceMigration.Targets memory t, address multisig) = _targets();
        console2.log("--- Phase 1: BEGIN admin hand-off (deployer EOA) ---");
        _report(t, multisig);

        vm.startBroadcast();
        GovernanceMigration.begin(t, multisig);
        vm.stopBroadcast();

        (, uint48 etaIR) = IAccessControlDefaultAdminRules(t.issuerRegistry).pendingDefaultAdmin();
        (, uint48 etaVR) = IAccessControlDefaultAdminRules(t.verificationRegistry).pendingDefaultAdmin();
        console2.log("IssuerRegistry accept ETA      (unix)", etaIR);
        console2.log("VerificationRegistry accept ETA(unix)", etaVR);
        console2.log("Next: after the timelock, the multisig runs Phase 2 (MigrateGovernanceAccept).");
    }
}

/// @notice Phase 2: executed BY (or through) the multisig AFTER the timelocks elapse. Accepts admin /
/// ownership everywhere and strips the EOA's residual roles.
/// @dev When the multisig is a Safe (a contract, not an EOA key), do NOT broadcast this script: instead
/// submit each `accept`/`revoke` call from the Safe (the begin-phase log + GovernanceMigration.accept list
/// are the call set). Broadcasting works only when MULTISIG is a key you control (anvil / a 1-of-1 / an
/// EOA-threshold scheme) — that path is exercised by the forge test.
contract MigrateGovernanceAccept is MigrateGovernanceBase {
    function run() external {
        (GovernanceMigration.Targets memory t, address multisig) = _targets();
        address oldAdmin = vm.envAddress("OLD_ADMIN"); // deployer EOA being decommissioned
        console2.log("--- Phase 2: ACCEPT admin hand-off (multisig) ---");
        _report(t, multisig);
        console2.log("OLD_ADMIN (decommissioned)", oldAdmin);

        vm.startBroadcast();
        GovernanceMigration.accept(t, oldAdmin);
        vm.stopBroadcast();

        console2.log("Hand-off complete. Verify: defaultAdmin()==MULTISIG and OLD_ADMIN has no roles.");
    }
}
