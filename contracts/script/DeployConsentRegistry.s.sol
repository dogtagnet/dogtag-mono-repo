// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";

/// @notice M4: deploy the Level-B owner-blind `VerificationRegistryConsent`, wired to the EXISTING live
/// ROAX components and to the M3 consent verifier `Groth16VerifierConsent` (0x272be146...).
///
/// This is an ADDITIVE deployment, not a swap. The Level-A `VerificationRegistry` 0x4E2f0996... stays
/// live and keeps serving the ECDSA + Level-A ZK flow: today's apps still produce Level-A proofs
/// (subject/keyHash) and Level-B tags do not exist yet (custodial issuance is M5). Repointing the live
/// consumers at this registry is M7's cutover, once the apps prove against the new VK — see
/// deployments/roax.json `_m4_consent_registry.m7_cutover` for the exhaustive list.
///
/// @dev Constructor = (issuerRegistry, sbt, zkVerifier, rootIndex, admin) — four components, not
/// Level-A's six: there is no ConsentKeyRegistry (D2: the consent key lives in the tree) and no
/// Poseidon6 (the nullifier is a public signal, never derived on-chain). `rootIndex` is the
/// DogTagIssuerFactory. Defaults are the live ROAX values (deployments/roax.json); override via env.
/// ADMIN defaults to the governance authority 0x8E27E117... (Phase-2), NOT the stripped deployer EOA.
///
/// Broadcast with the governance signer:
///   forge script contracts/script/DeployConsentRegistry.s.sol:DeployConsentRegistry \
///     --rpc-url https://devrpc.roax.net --broadcast --legacy --private-key $GOV_KEY
contract DeployConsentRegistry is Script {
    address public registryConsent; // NEW

    function run() external {
        address issuerRegistry = vm.envOr("ISSUER_REGISTRY", 0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c);
        address sbt = vm.envOr("SBT", 0x1FB8986573Ac36d532cF7d5a5352202B094D4233);
        // The M3 consent verifier — NOT the Level-A Groth16Verifier 0xEEFCf..., which is keyed to the
        // frozen verification.circom VK and would reject every DogTagConsent proof.
        address zkVerifier = vm.envOr("CONSENT_VERIFIER", 0x272be146C0aEd6401000E9Aa8241201F6f0fdF1a);
        address rootIndex = vm.envOr("ROOT_INDEX", 0xd3179AbBfb0274D0a5F7017d76015A93C159511D);
        address admin = vm.envOr("ADMIN", 0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2);

        require(
            issuerRegistry != address(0) && sbt != address(0) && zkVerifier != address(0)
                && rootIndex != address(0) && admin != address(0),
            "zero component"
        );

        vm.startBroadcast();
        registryConsent =
            address(new VerificationRegistryConsent(issuerRegistry, sbt, zkVerifier, rootIndex, admin));
        vm.stopBroadcast();

        console2.log("--- reused live components ---");
        console2.log("IssuerRegistry          ", issuerRegistry);
        console2.log("DogTagSBT               ", sbt);
        console2.log("Groth16VerifierConsent  ", zkVerifier);
        console2.log("RootIndex (Factory)     ", rootIndex);
        console2.log("Admin (governance)      ", admin);
        console2.log("--- NEW (M4) ---");
        console2.log("VerificationRegistryConsent", registryConsent);
        console2.log("");
        console2.log("Record in contracts/deployments/roax.json as VerificationRegistryConsent.");
        console2.log("Do NOT repoint the live consumers yet: the apps still prove Level-A (that is M7),");
        console2.log("and Level-B custodial tags do not exist until M5. See _m4_consent_registry.");
    }
}
