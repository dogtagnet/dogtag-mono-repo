// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {DogTagSBTConsent} from "../src/DogTagSBTConsent.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";

/// @notice M5: deploy the Level-B custodial stack - the `DogTagSBTConsent` SBT, plus a REDEPLOYED
/// `VerificationRegistryConsent` wired to it.
///
/// ADDITIVE, like M3/M4 before it. Nothing live is touched: the Level-A `DogTagSBT` 0x1FB89865… and
/// `VerificationRegistry` 0x4E2f0996… keep serving today's apps until the M7 cutover.
///
/// ### ⚠ ALREADY EXECUTED ON ROAX - do NOT re-broadcast there without captain approval.
/// The canonical ROAX pair is DEPLOYED and bytecode/wiring-verified (2026-07-16): `DogTagSBTConsent`
/// 0x96Cba4580D79bc9b8e51Fc1B3a044A29592AfFFc (block 202601) + `VerificationRegistryConsent`
/// 0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87 (block 202602), custodian
/// 0x637A514628d06Af711e3C9A2636fdBe5AE0E5A10 - see deployments/roax.json `_m5_custodial_issuance`.
/// Re-running this on ROAX deploys a SECOND, unwired pair and would overwrite those canonical entries.
/// The script stays a valid template for a fresh chain/environment.
///
/// ### Why the M4 registry is redeployed rather than reused
/// `VerificationRegistryConsent.sbt` is `immutable` (set in the constructor), so the M4 instance
/// 0x53F988Ae… is permanently bound to the Level-A SBT and CANNOT be repointed at the custodial one. The
/// registry's CODE is unchanged - this deploys the same contract against a new SBT. That is cheap and safe
/// precisely because the M4 instance was never live: at the time of writing it carries exactly one log (its
/// deployment `RoleGranted`) and ZERO `Verified` events, so nothing consumes it and no state is orphaned.
/// Re-verify before broadcasting:
///   cast logs --rpc-url $ROAX_RPC --address 0x53F988Ae0124b96069d90CBC78E6245FeB01E125 --from-block 0 \
///     0xeb5f75f2727c9d0d91b1fe9120ed35d6867952fa5f5103488f44a7afe21f532b
/// If that returns anything, STOP: a consumer exists and the redeploy needs a migration plan.
///
/// ### CUSTODIAN is mandatory and has no default
/// It is the one address `ownerOf` ever returns on the new SBT, and it is IMMUTABLE. The unlinkability of
/// every Level-B tag rests on it being owner-independent, so it must be a deliberate choice, never a
/// silently-inherited default. It MUST NOT be an address associated with any pet owner, and SHOULD NOT be a
/// vet/issuer signer either (that would re-link tags to the issuing practice's key). A dedicated,
/// neutral protocol address is the intent. It never signs: tags are soulbound and the contract has no
/// transfer path, so the custodian is a neutral sink, not an actor.
///
/// Broadcast with the governance signer:
///   CUSTODIAN=0x… forge script contracts/script/DeployCustodialIssuance.s.sol:DeployCustodialIssuance \
///     --rpc-url https://devrpc.roax.net --broadcast --legacy --private-key $GOV_KEY
contract DeployCustodialIssuance is Script {
    address public sbtConsent; // NEW
    address public registryConsent; // NEW (redeploy of M4's code against the new SBT)

    function run() external {
        // No default: a wrong custodian silently breaks the property the whole milestone exists for.
        address custodian = vm.envAddress("CUSTODIAN");
        address issuerRegistry = vm.envOr("ISSUER_REGISTRY", 0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c);
        address zkVerifier = vm.envOr("CONSENT_VERIFIER", 0x272be146C0aEd6401000E9Aa8241201F6f0fdF1a);
        address rootIndex = vm.envOr("ROOT_INDEX", 0xd3179AbBfb0274D0a5F7017d76015A93C159511D);
        address admin = vm.envOr("ADMIN", 0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2);

        require(
            issuerRegistry != address(0) && zkVerifier != address(0) && rootIndex != address(0)
                && admin != address(0) && custodian != address(0),
            "zero component"
        );
        // Cheap guards against the two custodian mistakes that would silently defeat unlinkability.
        require(custodian != admin, "custodian must not be the admin");
        require(custodian.code.length == 0, "custodian must be an EOA-style neutral sink");

        vm.startBroadcast();
        sbtConsent = address(new DogTagSBTConsent(admin, custodian));
        registryConsent = address(
            new VerificationRegistryConsent(issuerRegistry, sbtConsent, zkVerifier, rootIndex, admin)
        );
        vm.stopBroadcast();

        console2.log("--- reused live components ---");
        console2.log("IssuerRegistry            ", issuerRegistry);
        console2.log("Groth16VerifierConsent    ", zkVerifier);
        console2.log("RootIndex (Factory)       ", rootIndex);
        console2.log("Admin (governance)        ", admin);
        console2.log("--- NEW (M5) ---");
        console2.log("Custodian (neutral, immutable)", custodian);
        console2.log("DogTagSBTConsent          ", sbtConsent);
        console2.log("VerificationRegistryConsent (redeploy vs new SBT)", registryConsent);
        console2.log("");
        console2.log("Record BOTH in contracts/deployments/roax.json; supersede the M4 registry entry");
        console2.log("0x53F988Ae... (bound to the Level-A SBT, never live, zero Verified events).");
        console2.log("");
        console2.log("NEXT: grant ISSUER_ROLE on the new SBT to the vet signer, or issuance reverts.");
        console2.log("REMEMBER: issuance must issue(R) into a DogTagIssuer clone as well as mint, or");
        console2.log("every verify reverts 'unknown root'. Minting alone yields an unusable tag.");
    }
}
