// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {ConsentKeyRegistry} from "../src/ConsentKeyRegistry.sol";
import {VerificationRegistry} from "../src/VerificationRegistry.sol";

/// @notice Redeploys a FRESH ConsentKeyRegistry (now with the gasless `bindConsentKeyFor` meta-tx bind)
/// and a FRESH VerificationRegistry wired to the EXISTING live addresses for everything else, with the
/// freshly-deployed ConsentKeyRegistry plugged in immediately (no timelock wait for THIS deploy — the VR
/// `consentKeys` is assigned in the constructor; the 2-day timelock only gates FUTURE rotations).
///
/// @dev Pass the existing live addresses via env (read them from deployments/roax.json). The VR
/// constructor order is (issuerRegistry, sbt, zkVerifier, consentKeys, rootIndex, poseidon6, admin),
/// where `rootIndex` is the DogTagIssuerFactory. Env:
///   ISSUER_REGISTRY  — existing IssuerRegistry
///   SBT              — existing DogTagSBT
///   ZK_VERIFIER      — existing Groth16Verifier (may be 0; the ECDSA path needs no verifier)
///   ROOT_INDEX       — existing DogTagIssuerFactory (the rootIssuer[] index)
///   POSEIDON6        — existing Poseidon6
///   ADMIN            — protocol multisig (DEFAULT_ADMIN_ROLE on the new VR)
/// The NEW ConsentKeyRegistry is deployed by this script and passed to the new VR automatically.
contract RedeployConsentKey is Script {
    address public consentKeys; // NEW
    address public verification; // NEW

    function run() external {
        address issuerRegistry = vm.envAddress("ISSUER_REGISTRY");
        address sbt = vm.envAddress("SBT");
        address zkVerifier = vm.envOr("ZK_VERIFIER", address(0));
        address rootIndex = vm.envAddress("ROOT_INDEX");
        address poseidon6 = vm.envAddress("POSEIDON6");
        address admin = vm.envOr("ADMIN", msg.sender);

        vm.startBroadcast();
        consentKeys = address(new ConsentKeyRegistry());
        verification = address(
            new VerificationRegistry(issuerRegistry, sbt, zkVerifier, consentKeys, rootIndex, poseidon6, admin)
        );
        vm.stopBroadcast();

        console2.log("--- inputs (existing live addresses) ---");
        console2.log("IssuerRegistry      ", issuerRegistry);
        console2.log("DogTagSBT           ", sbt);
        console2.log("ZkVerifier          ", zkVerifier);
        console2.log("RootIndex (Factory) ", rootIndex);
        console2.log("Poseidon6           ", poseidon6);
        console2.log("Admin               ", admin);
        console2.log("--- NEW deployments (update deployments/roax.json with these) ---");
        console2.log("ConsentKeyRegistry  ", consentKeys);
        console2.log("VerificationRegistry", verification);
    }
}
