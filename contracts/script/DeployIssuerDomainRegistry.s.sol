// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {IssuerDomainRegistry} from "../src/IssuerDomainRegistry.sol";

/// @dev The one read this script preflights. Declared locally so the script depends on no more of the
/// live contracts' surface than it actually asks about.
interface IFactoryPreflight {
    function registry() external view returns (address);
}

/// @notice Deploy `IssuerDomainRegistry` STANDALONE against an ALREADY-DEPLOYED factory + registry.
///
/// # Why this script exists at all - do NOT reach for `Deploy.s.sol`
///
/// `Deploy.s.sol` also constructs an `IssuerDomainRegistry`, which makes it look like the right tool.
/// It is not: it constructs a FRESH `IssuerRegistry`, a FRESH `DogTagIssuer` implementation and a FRESH
/// `DogTagIssuerFactory` first, then overwrites those keys in `deployments/roax.json`. On a live network
/// that is unrecoverable rather than merely untidy - `VerificationRegistryConsent.rootIndex` is
/// `immutable`, so a new factory leaves the live verification registry resolving `rootIssuer[R]` through
/// the OLD index. Every root issued by a clone of the new factory resolves to `address(0)`, which is the
/// anchor the mandatory issuer-whitelist pillar and every owner-hidden consent verification rest on.
///
/// `IssuerDomainRegistry` is purely ADDITIVE - it READS the factory (clone provenance via
/// `isClone`/`predictIssuer`) and the registry (`WHITELIST_ADMIN` membership) and writes to NEITHER - so
/// deploying it alone against the existing pair is the whole job. `Deploy.s.sol:29-32` says exactly this.
///
/// # Env (all MANDATORY - no defaults, no fallbacks)
///
/// A default here would be a stale-network address waiting to happen: a wrong-but-plausible factory
/// still deploys, and the failure only surfaces later as bindings nothing can authorize. `vm.envAddress`
/// reverts on an unset var, so a missing value stops the deploy instead of guessing.
///
///   * `FACTORY`         - the live `DogTagIssuerFactory`
///   * `ISSUER_REGISTRY` - the live `IssuerRegistry` whose `WHITELIST_ADMIN` gates tiers 1 and 3
///
/// # The coherence preflight is load-bearing, not decoration
///
/// The two addresses are not independent: a `DogTagIssuer` clone gates its own writes on the registry it
/// was initialized with, which is `factory.registry()`. Binding this contract to a DIFFERENT registry
/// would make tier 1 (`WHITELIST_ADMIN`) authorize against a registry that governs none of the clones
/// tier 2 recognises - the same class of split-brain this repo has already hit with clones bound to a
/// superseded registry, where every gate fails closed long after the misconfiguration. So the script
/// asserts `factory.registry() == ISSUER_REGISTRY` and refuses to broadcast otherwise.
///
/// Broadcast with the deployer key (this needs NO role - it only deploys):
///   FACTORY=0x… ISSUER_REGISTRY=0x… \
///   forge script script/DeployIssuerDomainRegistry.s.sol:DeployIssuerDomainRegistry \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $DEPLOYER_PRIVATE_KEY
contract DeployIssuerDomainRegistry is Script {
    string internal constant DEPLOYMENT_PATH = "deployments/roax.json";

    address public issuerDomainRegistry; // NEW - the only thing this script deploys

    function run() external {
        address factory = vm.envAddress("FACTORY");
        address issuerRegistry = vm.envAddress("ISSUER_REGISTRY");
        require(factory != address(0) && issuerRegistry != address(0), "zero factory/registry");

        // Liveness + coherence, BEFORE broadcasting anything. A stale ledger entry is the documented
        // failure mode here, and both halves of it are cheap to rule out from the chain itself.
        require(factory.code.length > 0, "FACTORY has no code on this chain");
        require(issuerRegistry.code.length > 0, "ISSUER_REGISTRY has no code on this chain");
        address boundRegistry = IFactoryPreflight(factory).registry();
        require(boundRegistry == issuerRegistry, "factory.registry() != ISSUER_REGISTRY");

        vm.startBroadcast();
        issuerDomainRegistry = address(new IssuerDomainRegistry(factory, issuerRegistry));
        vm.stopBroadcast();

        _report(factory, issuerRegistry);
    }

    function _report(address factory, address issuerRegistry) internal {
        console2.log("--- IssuerDomainRegistry (additive; no redeploy of factory/registry) ---");
        console2.log("IssuerDomainRegistry", issuerDomainRegistry);
        console2.log("factory             ", factory);
        console2.log("issuerRegistry      ", issuerRegistry);

        // Replace ONLY the one freshly deployed key. The ledger also carries frozen ceremony metadata
        // and deliberately retained historical addresses, so rewriting the whole document would destroy
        // information that stays authoritative for old reads.
        _writeAddress(".IssuerDomainRegistry", issuerDomainRegistry);

        console2.log("");
        console2.log("Wrote .IssuerDomainRegistry to", DEPLOYMENT_PATH);
        console2.log("Next: wire VITE_ISSUER_DOMAIN_REGISTRY_ADDR in every consumer (it has NO fallback,");
        console2.log("so an unset value reports 'unavailable' rather than failing loudly).");
    }

    function _writeAddress(string memory key, address value) internal {
        vm.writeJson(string.concat("\"", vm.toString(value), "\""), DEPLOYMENT_PATH, key);
    }
}
