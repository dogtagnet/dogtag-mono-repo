// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {DogTagSBTConsent} from "../src/DogTagSBTConsent.sol";
import {Groth16VerifierConsent} from "../src/Groth16VerifierConsent.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";

/// @notice Deploys the complete owner-hidden issuance and verification stack from fresh bytecode.
/// @dev `ISSUER_REGISTRY`, `ROOT_INDEX`, `ADMIN`, and `CUSTODIAN` are mandatory so a fresh deployment
/// cannot silently retain an address from another network. The frozen ceremony verifier is deployed by
/// this script and wired directly into the new registry.
///
/// `CUSTODIAN` is mandatory and has no default.
/// It is the one address `ownerOf` ever returns on the new SBT, and it is IMMUTABLE. The unlinkability of
/// every tag rests on it being owner-independent, so it must be a deliberate choice, never a
/// silently-inherited default. It MUST NOT be an address associated with any pet owner, and SHOULD NOT be a
/// vet/issuer signer either (that would re-link tags to the issuing practice's key). A dedicated,
/// neutral protocol address is the intent. It never signs: tags are soulbound and the contract has no
/// transfer path, so the custodian is a neutral sink, not an actor.
///
/// Broadcast with the governance signer:
///   ISSUER_REGISTRY=0x… ROOT_INDEX=0x… ADMIN=0x… CUSTODIAN=0x… \
///     forge script contracts/script/DeployCustodialIssuance.s.sol:DeployCustodialIssuance \
///       --rpc-url $ROAX_RPC --broadcast --legacy --private-key $GOV_KEY
contract DeployCustodialIssuance is Script {
    string internal constant DEPLOYMENT_PATH = "deployments/roax.json";

    address public verifierConsent;
    address public sbtConsent;
    address public registryConsent;

    function run() external {
        // No default: a wrong custodian silently breaks the property the whole milestone exists for.
        address custodian = vm.envAddress("CUSTODIAN");
        address issuerRegistry = vm.envAddress("ISSUER_REGISTRY");
        address rootIndex = vm.envAddress("ROOT_INDEX");
        address admin = vm.envAddress("ADMIN");

        require(
            issuerRegistry != address(0) && rootIndex != address(0) && admin != address(0)
                && custodian != address(0),
            "zero component"
        );
        // Cheap guards against the two custodian mistakes that would silently defeat unlinkability.
        require(custodian != admin, "custodian must not be the admin");
        require(custodian.code.length == 0, "custodian must be an EOA-style neutral sink");

        vm.startBroadcast();
        verifierConsent = address(new Groth16VerifierConsent());
        sbtConsent = address(new DogTagSBTConsent(admin, custodian));
        registryConsent = address(
            new VerificationRegistryConsent(issuerRegistry, sbtConsent, verifierConsent, rootIndex, admin)
        );
        vm.stopBroadcast();

        console2.log("--- shared deployment inputs ---");
        console2.log("IssuerRegistry            ", issuerRegistry);
        console2.log("RootIndex (Factory)       ", rootIndex);
        console2.log("Admin (governance)        ", admin);
        console2.log("--- owner-hidden stack ---");
        console2.log("Custodian (neutral, immutable)", custodian);
        console2.log("Groth16VerifierConsent    ", verifierConsent);
        console2.log("DogTagSBTConsent          ", sbtConsent);
        console2.log("VerificationRegistryConsent", registryConsent);

        // Repoint the canonical live stack while retaining `_legacy` addresses and the frozen
        // ceremony/deployment metadata in the same ledger.
        _writeAddress(".Groth16VerifierConsent", verifierConsent);
        _writeAddress(".DogTagSBTConsent", sbtConsent);
        _writeAddress(".VerificationRegistryConsent", registryConsent);

        console2.log("");
        console2.log("NEXT: grant ISSUER_ROLE on the new SBT to the vet signer, or issuance reverts.");
        console2.log("REMEMBER: issuance must issue(R) into a DogTagIssuer clone as well as mint, or");
        console2.log("every verify reverts 'unknown root'. Minting alone yields an unusable tag.");
    }

    function _writeAddress(string memory key, address value) internal {
        vm.writeJson(string.concat("\"", vm.toString(value), "\""), DEPLOYMENT_PATH, key);
    }
}
