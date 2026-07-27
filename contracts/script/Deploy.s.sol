// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {IssuerDomainRegistry} from "../src/IssuerDomainRegistry.sol";

/// @notice Deploys the shared DogTag contract base and writes deployments/roax.json.
/// @dev The owner-hidden issuance and verification stack is deployed separately by
/// `DeployCustodialIssuance.s.sol`. Env: ADMIN (protocol multisig; default broadcaster).
contract Deploy is Script {
    string internal constant DEPLOYMENT_PATH = "deployments/roax.json";

    // storage (keeps run() off the stack — avoids stack-too-deep without via-ir)
    address public registry;
    address public issuerImpl;
    address public factory;
    address public domainRegistry;

    function run() external {
        address admin = vm.envOr("ADMIN", msg.sender);

        vm.startBroadcast();
        registry = address(new IssuerRegistry(admin));
        issuerImpl = address(new DogTagIssuer());
        factory = address(new DogTagIssuerFactory(issuerImpl, registry, admin));
        // Purely ADDITIVE: reads the factory (clone provenance) and the registry (WHITELIST_ADMIN) and
        // writes to neither, so it can equally be deployed standalone against an EXISTING factory +
        // registry with no redeploy of either. See IssuerDomainRegistry's contract doc for why a field
        // on the clone or a mapping on IssuerRegistry would have been a protocol-wide v2 instead.
        domainRegistry = address(new IssuerDomainRegistry(factory, registry));
        vm.stopBroadcast();

        _report(admin);
    }

    function _report(address admin) internal {
        console2.log("IssuerRegistry      ", registry);
        console2.log("DogTagIssuer impl   ", issuerImpl);
        console2.log("DogTagIssuerFactory ", factory);
        console2.log("IssuerDomainRegistry", domainRegistry);

        // Replace only the freshly deployed shared addresses. The deployment ledger also contains
        // frozen ceremony metadata and explicitly retained historical addresses, so overwriting the
        // complete document here would destroy information that remains authoritative for old reads.
        vm.writeJson(vm.toString(block.chainid), DEPLOYMENT_PATH, ".chainId");
        _writeAddress(".admin", admin);
        _writeAddress(".IssuerRegistry", registry);
        _writeAddress(".DogTagIssuerImpl", issuerImpl);
        _writeAddress(".DogTagIssuerFactory", factory);
        _writeAddress(".IssuerDomainRegistry", domainRegistry);
    }

    function _writeAddress(string memory key, address value) internal {
        vm.writeJson(string.concat("\"", vm.toString(value), "\""), DEPLOYMENT_PATH, key);
    }
}
