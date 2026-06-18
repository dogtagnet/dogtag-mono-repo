// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagSBT} from "../src/DogTagSBT.sol";
import {ConsentKeyRegistry} from "../src/ConsentKeyRegistry.sol";
import {VerificationRegistry} from "../src/VerificationRegistry.sol";

/// @notice Deploys the full DogTag contract set to ROAX (chainId 135) and writes deployments/roax.json.
/// @dev Gate B prechecks must pass first: `cast chain-id --rpc-url $ROAX_RPC` == 135 and the BN254
/// pairing precompiles (0x06/0x07/0x08) present. Env: ADMIN (protocol multisig; default broadcaster),
/// ZK_VERIFIER (Groth16Verifier; default 0 — set later via the registry timelock after the phase-2
/// ceremony, since the normal ECDSA path needs no verifier), POSEIDON6_INITCODE (default
/// test/poseidon6.initcode — the circomlib-exact Poseidon(6) creation bytecode).
contract Deploy is Script {
    // storage (keeps run() off the stack — avoids stack-too-deep without via-ir)
    address public registry;
    address public issuerImpl;
    address public factory;
    address public sbt;
    address public consentKeys;
    address public poseidon6;
    address public verification;

    function run() external {
        address admin = vm.envOr("ADMIN", msg.sender);
        address zkVerifier = vm.envOr("ZK_VERIFIER", address(0));
        string memory initPath = vm.envOr("POSEIDON6_INITCODE", string("test/poseidon6.initcode"));
        bytes memory initcode = vm.parseBytes(vm.readFile(initPath));

        vm.startBroadcast();
        registry = address(new IssuerRegistry(admin));
        issuerImpl = address(new DogTagIssuer());
        factory = address(new DogTagIssuerFactory(issuerImpl, registry, admin));
        sbt = address(new DogTagSBT(admin));
        consentKeys = address(new ConsentKeyRegistry());
        poseidon6 = _deployPoseidon6(initcode);
        verification = address(
            new VerificationRegistry(registry, sbt, zkVerifier, consentKeys, factory, poseidon6, admin)
        );
        vm.stopBroadcast();

        _report(admin);
    }

    function _deployPoseidon6(bytes memory initcode) internal returns (address p6) {
        assembly {
            p6 := create(0, add(initcode, 0x20), mload(initcode))
        }
        require(p6 != address(0), "poseidon6 deploy failed");
    }

    function _report(address admin) internal {
        console2.log("IssuerRegistry      ", registry);
        console2.log("DogTagIssuer impl   ", issuerImpl);
        console2.log("DogTagIssuerFactory ", factory);
        console2.log("DogTagSBT           ", sbt);
        console2.log("ConsentKeyRegistry  ", consentKeys);
        console2.log("Poseidon6           ", poseidon6);
        console2.log("VerificationRegistry", verification);

        string memory obj = "deployments";
        vm.serializeUint(obj, "chainId", block.chainid);
        vm.serializeAddress(obj, "admin", admin);
        vm.serializeAddress(obj, "IssuerRegistry", registry);
        vm.serializeAddress(obj, "DogTagIssuerImpl", issuerImpl);
        vm.serializeAddress(obj, "DogTagIssuerFactory", factory);
        vm.serializeAddress(obj, "DogTagSBT", sbt);
        vm.serializeAddress(obj, "ConsentKeyRegistry", consentKeys);
        vm.serializeAddress(obj, "Poseidon6", poseidon6);
        string memory json = vm.serializeAddress(obj, "VerificationRegistry", verification);
        vm.writeJson(json, "deployments/roax.json");
    }
}
