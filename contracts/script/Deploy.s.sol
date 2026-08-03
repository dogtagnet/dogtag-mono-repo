// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagSBTConsent} from "../src/DogTagSBTConsent.sol";
import {Groth16VerifierConsent} from "../src/Groth16VerifierConsent.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";
import {ProviderDirectory} from "../src/ProviderDirectory.sol";
import {ServiceDomainResolver} from "../src/ServiceDomainResolver.sol";
import {
    ProtocolRegistry,
    DEFAULT_PUBLISH_TIMELOCK_SECONDS,
    MIN_PUBLISH_TIMELOCK_SECONDS
} from "../src/ProtocolRegistry.sol";

/// @title Deploy — stands the whole DogTag system up from nothing, in one transaction sequence.
///
/// @notice Nine contracts and the registrar wiring that makes them a working deployment rather than
/// nine unrelated addresses. There is exactly one deploy script because there is exactly one system;
/// a second one that stood up "part of" it is how a stack comes to exist that no test covers.
///
/// # Ordering is a dependency graph, not a preference
///
/// Every dependency in this system is `immutable` and every contract that has one behaviour-probes it
/// in its constructor, so a wrong order does not produce a subtly-wrong deployment — it reverts. The
/// order below is the only one that satisfies all of them:
///
///   1. `ProviderRegistry` — the authority core. Depends on nothing.
///   2. `DogTagIssuer` — the clone implementation. Depends on nothing (clones delegate to it).
///   3. `DogTagIssuerFactory` — pins (2) by exact runtime-code hash and behaviour-probes (1).
///   4. `DogTagSBTConsent` — the tag. Depends on nothing but its admin and its neutral custodian.
///   5. `Groth16VerifierConsent` — the frozen ceremony VK. Depends on nothing.
///   6. `VerificationRegistryConsent` — over (1), (4), (5) and (3). Its `rootIndex` is the FACTORY:
///      `rootIssuer[R]` is where a credential's issuing clone is resolved from, so this is the one
///      pointer that decides whether an anchored root can ever be verified. It is immutable.
///   7. `ProviderDirectory` and `ServiceDomainResolver` — the typed resolvers, over (1); the domain
///      resolver additionally over (3), because a domain claim is only honoured for a service whose
///      lineage is the same lineage that resolves its roots.
///   8. `ProtocolRegistry` — the discovery anchor. Deliberately depends on NO other address here: it
///      publishes them as data, and publishing is a separate, timelocked, two-phase operation
///      (`PublishProtocolVersions.s.sol`). This script deploys it and publishes nothing.
///
/// # The registrar wiring is part of standing the system up
///
/// A deployment that stops at nine addresses is inert: the core would recognise no factory, so no
/// provider could deploy a clone, and no resolver would be approved, so no provider could publish a
/// domain or a listing. Those three calls are `onlyOwner` on the core and the deployer holds that key
/// at this moment, so they belong here. What is deliberately NOT here is anything provider-specific —
/// registering a provider, clearing it for a record type, attaching its clone — because those are KYC
/// decisions about a particular business, not part of the protocol's own shape.
///
/// # What this script must never be used for
///
/// It deploys a COMPLETE, INDEPENDENT set. It cannot repair, extend or partially replace a running
/// deployment: `VerificationRegistryConsent.rootIndex`, `.sbt`, `.providerRegistry` and `.zkVerifier`
/// are all immutable, as is `DogTagIssuerFactory.registry`, so a fresh factory beside an existing
/// registry resolves `rootIssuer[R] == 0` for every root ever anchored and strands every credential.
/// Deploying is all-or-nothing, and every client repoints together.
///
/// # Env
///
///   ADMIN                  (required) the protocol authority. Owns the core; admins the SBT, the
///                          verification registry and the discovery registry. Named rather than
///                          defaulted to the broadcaster, because who holds it is the decision this
///                          deployment makes and a silent default is how it gets made by accident.
///                          It MUST be the broadcasting key: the registrar wiring below is `onlyOwner`
///                          on a core this script has just given to ADMIN, so a broadcaster that is
///                          not ADMIN reverts on `addFactoryGeneration` with the deployment half done.
///   CUSTODIAN              (required) the SBT's immutable neutral custodian. Every tag is minted here.
///                          MUST NOT be an owner's wallet or an issuer signer — it is a sink that never
///                          signs, and pointing it at a real actor re-links tags to that actor.
///   PUBLISHER              (optional, default ADMIN) holds `PUBLISHER_ROLE` on the discovery registry.
///   PUBLISH_TIMELOCK_SECS  (optional, default 2 days) the discovery registry's immutable delay.
///   TESTNET_DEPLOY         (optional, default false) the loud opt-in for a shorter delay.
///
/// Broadcast:
///   forge script script/Deploy.s.sol:Deploy --rpc-url $ROAX_RPC --broadcast --legacy --slow \
///     --private-key $DEPLOYER_KEY
contract Deploy is Script {
    string internal constant DEPLOYMENT_PATH = "deployments/roax.json";

    /// @notice The core's id for the factory generation this deployment registers. The core supports
    /// several by design (`addFactoryGeneration`), so a deployment must name the one it is adding.
    bytes32 public constant FACTORY_GENERATION = keccak256("dogtag-issuer-factory/1");

    /// @notice The delay a non-testnet deployment must use, exactly. Single-sourced from the contract.
    uint256 public constant MAINNET_PUBLISH_TIMELOCK = DEFAULT_PUBLISH_TIMELOCK_SECONDS;

    /// @notice The floor the CONTRACT enforces. Mirrored so this script can explain a rejection in
    /// terms of the same number the constructor would revert on.
    uint256 public constant MIN_PUBLISH_TIMELOCK = MIN_PUBLISH_TIMELOCK_SECONDS;

    // Storage rather than locals: nine addresses plus the env values overflow the stack in `run()`,
    // and `via-ir` is not enabled for this profile.
    address public providerRegistry;
    address public issuerImpl;
    address public factory;
    address public sbt;
    address public verifier;
    address public verificationRegistry;
    address public providerDirectory;
    address public serviceDomainResolver;
    address public protocolRegistry;

    function run() external {
        address admin = vm.envAddress("ADMIN");
        address custodian = vm.envAddress("CUSTODIAN");
        address publisher = vm.envOr("PUBLISHER", admin);
        uint256 publishTimelock = vm.envOr("PUBLISH_TIMELOCK_SECS", MAINNET_PUBLISH_TIMELOCK);
        bool testnetDeploy = vm.envOr("TESTNET_DEPLOY", false);

        validatePublishTimelock(publishTimelock, testnetDeploy);

        vm.startBroadcast();
        deploy(admin, custodian, publisher, publishTimelock);
        vm.stopBroadcast();

        _report(admin, custodian, publisher, publishTimelock, testnetDeploy);
        _writeLedger(admin, custodian);
    }

    /// @notice The deployment sequence itself — nine contracts, the registrar wiring, and the read-back.
    ///
    /// @dev Separated from {run} so it can be driven directly by a test: no environment, no broadcast
    /// and no ledger write, so a test exercises the ORDER and the wiring without the script's I/O.
    /// {run} calls it inside its broadcast, where forge attributes every sub-call to the broadcasting
    /// key — which is why ADMIN must be that key.
    function deploy(address admin, address custodian, address publisher, uint256 publishTimelock)
        public
    {
        require(admin != address(0), "zero ADMIN");
        require(custodian != address(0), "zero CUSTODIAN");
        require(publisher != address(0), "zero PUBLISHER");
        // The custodian must not be the authority: `ownerOf` would then return a key that signs, which
        // is exactly the linkage the custodial mint exists to remove.
        require(custodian != admin, "CUSTODIAN must not be ADMIN");

        providerRegistry = address(new ProviderRegistry(admin));
        issuerImpl = address(new DogTagIssuer());
        factory = address(new DogTagIssuerFactory(issuerImpl, providerRegistry));
        sbt = address(new DogTagSBTConsent(admin, custodian));
        verifier = address(new Groth16VerifierConsent());
        verificationRegistry = address(
            new VerificationRegistryConsent(providerRegistry, sbt, verifier, factory, admin)
        );
        providerDirectory = address(new ProviderDirectory(ProviderRegistry(providerRegistry)));
        serviceDomainResolver = address(new ServiceDomainResolver(providerRegistry, factory));
        protocolRegistry = address(new ProtocolRegistry(admin, publisher, publishTimelock));

        // The wiring that makes the above a system. `onlyOwner` on the core; the broadcaster must be
        // `admin` for these to land, which is asserted rather than assumed below.
        ProviderRegistry(providerRegistry).addFactoryGeneration(FACTORY_GENERATION, factory);
        ProviderRegistry(providerRegistry).setResolverApproved(
            ProviderRegistry.ResolverKind.DIRECTORY, providerDirectory, true
        );
        ProviderRegistry(providerRegistry).setResolverApproved(
            ProviderRegistry.ResolverKind.DOMAIN, serviceDomainResolver, true
        );

        _assertWiring();
    }

    /// @notice Re-read every immutable this deployment set, from the deployed contracts themselves.
    ///
    /// @dev Not ceremony. Each of these is unrepairable after the fact, and each has a failure mode
    /// that is silent rather than loud: a verification registry pointed at the wrong root index answers
    /// `unknown root` for every credential ever issued, and a factory pointed at the wrong core refuses
    /// every provider while looking like an authorization problem. Reading them back turns a
    /// constructor-argument mistake into a failed deploy instead of a deployment nobody can use.
    function _assertWiring() internal view {
        VerificationRegistryConsent vr = VerificationRegistryConsent(verificationRegistry);
        require(address(vr.rootIndex()) == factory, "vr.rootIndex != factory");
        require(address(vr.providerRegistry()) == providerRegistry, "vr.providerRegistry != core");
        require(address(vr.sbt()) == sbt, "vr.sbt != sbt");
        require(address(vr.zkVerifier()) == verifier, "vr.zkVerifier != verifier");

        DogTagIssuerFactory f = DogTagIssuerFactory(factory);
        require(f.registry() == providerRegistry, "factory.registry != core");
        require(f.implementation() == issuerImpl, "factory.implementation != impl");

        ProviderRegistry core = ProviderRegistry(providerRegistry);
        require(core.generationOfFactory(factory) == FACTORY_GENERATION, "core does not carry this factory");
        require(
            core.isResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, providerDirectory),
            "directory resolver not approved"
        );
        require(
            core.isResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, serviceDomainResolver),
            "domain resolver not approved"
        );

        require(address(ServiceDomainResolver(serviceDomainResolver).factory()) == factory, "resolver lineage");
        require(address(ProviderDirectory(providerDirectory).core()) == providerRegistry, "directory core");
    }

    /// @notice LOUD production guard, and THE ONLY place the production/testnet distinction is drawn.
    ///
    /// Without an explicit testnet opt-in the delay must be exactly the governance-approved 2-day
    /// default. With one it may be anything, INCLUDING ZERO — a development chain deploys, publishes,
    /// tests and redeploys in one sitting, and a floor there buys nothing while costing every iteration.
    ///
    /// `ProtocolRegistry` enforces no floor of its own any more. That guard used to sit on the contract
    /// because `PUBLISH_TIMELOCK` is immutable and a wrong value could only be fixed by replacing the
    /// registry — which meant repointing every client, including two compile-time mobile bundles. A
    /// mobile rebuild-and-reinstall now accompanies every full redeploy as standing process, so that
    /// replacement is routine and a script guard is proportionate to it. See the contract's "The
    /// timelock is a deploy-time choice" note; this is a relocation, not a removal.
    ///
    /// Production is expressed HERE, by the default plus a stated-aloud opt-in — never by inspecting
    /// `block.chainid`. ROAX 135 is itself a live chain, so a `require(block.chainid == 135)` guard
    /// passed on exactly the deployment it claimed to refuse; a condition that cannot fail on the case
    /// it names reads as protection while providing none.
    ///
    /// Public and pure so the invariant is testable without deploying anything.
    function validatePublishTimelock(uint256 publishTimelock, bool testnetDeploy) public pure {
        if (!testnetDeploy) {
            require(publishTimelock == MAINNET_PUBLISH_TIMELOCK, "mainnet publish timelock must be 2 days");
            return;
        }
    }

    function _report(
        address admin,
        address custodian,
        address publisher,
        uint256 publishTimelock,
        bool testnetDeploy
    ) internal view {
        console2.log("--- DogTag deployment, chain", block.chainid, "---");
        console2.log("ProviderRegistry           ", providerRegistry);
        console2.log("DogTagIssuer (impl)        ", issuerImpl);
        console2.log("DogTagIssuerFactory        ", factory);
        console2.log("DogTagSBTConsent           ", sbt);
        console2.log("Groth16VerifierConsent     ", verifier);
        console2.log("VerificationRegistryConsent", verificationRegistry);
        console2.log("ProviderDirectory          ", providerDirectory);
        console2.log("ServiceDomainResolver      ", serviceDomainResolver);
        console2.log("ProtocolRegistry           ", protocolRegistry);
        console2.log("");
        console2.log("Admin (authority)  ", admin);
        console2.log("SBT custodian      ", custodian);
        console2.log("Publisher          ", publisher);
        console2.log("Publish timelock   ", publishTimelock, "seconds");
        console2.log("Testnet deploy     ", testnetDeploy);
        console2.log("");
        console2.log("Next: PublishProtocolVersions (Propose), then Execute once the ETA is reached.");
    }

    /// @dev Writes each address into its own key rather than replacing the document, so the ledger's
    /// prose — what was verified, and how — is not destroyed by a deploy.
    function _writeLedger(address admin, address custodian) internal {
        vm.writeJson(vm.toString(block.chainid), DEPLOYMENT_PATH, ".chainId");
        _writeAddress(".admin", admin);
        _writeAddress(".custodian", custodian);
        _writeAddress(".ProviderRegistry", providerRegistry);
        _writeAddress(".DogTagIssuerImpl", issuerImpl);
        _writeAddress(".DogTagIssuerFactory", factory);
        _writeAddress(".DogTagSBTConsent", sbt);
        _writeAddress(".Groth16VerifierConsent", verifier);
        _writeAddress(".VerificationRegistryConsent", verificationRegistry);
        _writeAddress(".ProviderDirectory", providerDirectory);
        _writeAddress(".ServiceDomainResolver", serviceDomainResolver);
        _writeAddress(".ProtocolRegistry", protocolRegistry);
    }

    function _writeAddress(string memory key, address value) internal {
        vm.writeJson(string.concat("\"", vm.toString(value), "\""), DEPLOYMENT_PATH, key);
    }
}
