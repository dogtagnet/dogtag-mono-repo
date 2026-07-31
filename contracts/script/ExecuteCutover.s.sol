// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {CutoverSequence} from "./CutoverSequence.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {DogTagIssuerV2} from "../src/DogTagIssuerV2.sol";
import {DogTagIssuerFactoryV2} from "../src/DogTagIssuerFactoryV2.sol";
import {CloneProvenanceRouter} from "../src/CloneProvenanceRouter.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";
import {ProtocolRegistryV2} from "../src/ProtocolRegistryV2.sol";
import {MIN_PUBLISH_TIMELOCK_SECONDS_V2, DEFAULT_PUBLISH_TIMELOCK_SECONDS_V2} from "../src/ProtocolRegistryV2.sol";

/// @title ExecuteCutover - the LIVE counterpart of `RehearseCutover.s.sol` (registry plan S-14).
///
/// @notice `RehearseCutover` **cannot** perform the live cutover, and that is deliberate rather than an
/// oversight: it requires `block.number == pinnedBlock`, which a live chain can never satisfy because
/// its head is already far past that block and only ever advances. That guard is what carries the
/// rehearsal's safety, so it must not be relaxed - a live driver is a separate artefact instead.
///
/// # WHAT IS SHARED, AND WHY THAT IS THE WHOLE POINT
///
/// Every step below calls the SAME `CutoverSequence.cN_*` function, with the same arguments, in the
/// same order as the rehearsal and the fork test. That library is the single definition the rehearsal
/// asserted and `make rehearse-cutover-mutations` proved able to fail. A live driver that re-derived
/// the constructor arguments - in Solidity or by hand-encoding `cast send --create` - would be a
/// SECOND definition, free to drift from the asserted one by a single argument with both looking
/// correct. `CutoverSequence`'s own header says so; this file exists to obey it.
///
/// # WHY IT IS PHASED RATHER THAN ONE RUN
///
/// C-4b (`appendGeneration`) is IRREVERSIBLE: the router has no removal, no replace and no reorder, so
/// a wrong generation list means a new router, therefore a new verification registry (its `rootIndex`
/// is immutable), therefore every downstream step again. `docs/CUTOVER_REHEARSAL.md` §4 requires the
/// router's ordering to be READ OFF THE CHAIN before proceeding, and a monolithic run cannot stop to
/// do that. The phases are:
///
///   * `CUTOVER_PHASE=1` - C-1, C-3a, C-4, C-3b. Every one is "rollback = abandon the address".
///   * `CUTOVER_PHASE=2` - C-4b, C-2. GOVERNANCE, and the irreversible half.
///   * `CUTOVER_PHASE=3` - C-5, C-8. Deployments again; nothing reads them until C-9.
///
/// Splitting the DRIVER changes no argument of the SEQUENCE, so the shared-definition property above
/// survives intact.
///
/// # THE ASSERTIONS HERE ARE PRECONDITIONS, NOT EVIDENCE
///
/// A `require` inside a forge script runs in the script's own EVM. In phase 2 that EVM has forked live
/// state, so the preconditions guarding the irreversible append really do read the deployed router and
/// factory - which is exactly where they are worth the most, because a failed simulation broadcasts
/// nothing. But a `require` placed AFTER a `new X()` in phase 1 or 3 reads the script's local copy, not
/// the chain, so it can only catch a wrong ARGUMENT and can never establish a landed result.
///
/// Landed state is therefore verified out of band, against the chain, between phases -
/// `docs/CUTOVER_REHEARSAL.md` §6 makes the same distinction for C-4b, which it checks by EFFECT
/// rather than by receipt precisely because it is the step whose omission is both fatal and silent.
contract ExecuteCutover is Script {
    /// @dev The live generation-1 set, read at run time exactly as `RehearseCutover._generation1()`
    /// does. Same file, same keys, no transcribed address - this repo has a standing rule that a
    /// transcribed address is a copy that goes stale silently.
    function _generation1() internal view returns (CutoverSequence.Generation1 memory gen1) {
        string memory ledger = vm.readFile("deployments/roax.json");
        gen1.factory = vm.parseJsonAddress(ledger, ".DogTagIssuerFactory");
        gen1.issuerRegistry = vm.parseJsonAddress(ledger, ".IssuerRegistry");
        gen1.sbt = vm.parseJsonAddress(ledger, ".DogTagSBTConsent");
        gen1.verifier = vm.parseJsonAddress(ledger, ".Groth16VerifierConsent");
        gen1.governance = vm.parseJsonAddress(ledger, ".admin");
    }

    function run() external {
        require(block.chainid == 135, "ExecuteCutover expects the ROAX chain id (135)");

        // The inverse of the rehearsal's guard. `RehearseCutover` requires the pinned block; this
        // driver refuses it, so the two can never be pointed at each other's endpoint. A fork taken at
        // the CURRENT head is deliberately still allowed - that is how the simulate-versus-broadcast
        // behaviour is measured before anything is sent live.
        uint256 pinnedBlock =
            vm.parseJsonUint(vm.readFile("rehearsal/fixtures/historical-roots.json"), ".pinnedBlock");
        require(
            block.number != pinnedBlock,
            "REFUSING: head is the pinned REHEARSAL block, so this is the rehearsal fork. Use "
            "RehearseCutover there; this driver is for a live endpoint or a fork at current head."
        );

        // Mandatory and defaulted to nothing: a stray invocation with no environment reverts here
        // rather than sending the first transaction of an irreversible sequence.
        uint256 phase = vm.envUint("CUTOVER_PHASE");
        CutoverSequence.Generation1 memory gen1 = _generation1();

        console2.log("chain                       ", block.chainid);
        console2.log("head                        ", block.number);
        console2.log("generation-1 factory        ", gen1.factory);
        console2.log("generation-1 issuerRegistry ", gen1.issuerRegistry);
        console2.log("shared SBT      (REUSED)    ", gen1.sbt);
        console2.log("shared verifier (REUSED)    ", gen1.verifier);
        console2.log("governance                  ", gen1.governance);
        console2.log("phase                       ", phase);
        console2.log("");

        if (phase == 1) {
            _phase1(gen1);
        } else if (phase == 2) {
            _phase2(gen1);
        } else if (phase == 3) {
            _phase3(gen1);
        } else {
            revert("CUTOVER_PHASE must be 1, 2 or 3");
        }
    }

    // =============================================================================================
    // Phase 1 - C-1, C-3a, C-4, C-3b. Nothing here is irreversible.
    // =============================================================================================

    function _phase1(CutoverSequence.Generation1 memory gen1) internal {
        vm.startBroadcast();

        ProviderRegistry core = CutoverSequence.c1_deployProviderRegistry(gen1.governance); // C-1
        DogTagIssuerV2 issuerImpl = CutoverSequence.c3a_deployIssuerImpl(); // C-3a
        CloneProvenanceRouter router = CutoverSequence.c4_deployRouter(gen1.factory, gen1.governance); // C-4

        // Ordering is a PRECONDITION of continuing, not a post-hoc read. Newest-first is the
        // revocation bypass S-8 exists to prevent and it fails silently: nothing reverts, a revoked
        // credential simply starts verifying again. Asserted before the factory is even deployed, so a
        // wrong list cannot reach C-4b.
        require(router.generationCount() == 1, "C-4: router must hold generation 1 ALONE");
        require(router.generationAt(0) == gen1.factory, "C-4: index 0 must be the generation-1 factory");
        require(router.owner() == gen1.governance, "C-4: router owner must be governance");

        DogTagIssuerFactoryV2 factoryV2 = CutoverSequence.c3b_deployFactoryV2(issuerImpl, core, router); // C-3b

        require(address(factoryV2.priorIndex()) == address(router), "C-3b: factory must be bound to THIS router");
        require(factoryV2.registry() == address(core), "C-3b: factory must be bound to THIS core");
        require(factoryV2.implementation() == address(issuerImpl), "C-3b: factory must name THIS implementation");
        require(!router.isGeneration(address(factoryV2)), "C-3b: router must not yet name the factory");

        vm.stopBroadcast();

        console2.log("C-1  ProviderRegistry       ", address(core));
        console2.log("C-3a DogTagIssuerV2 impl    ", address(issuerImpl));
        console2.log("C-4  CloneProvenanceRouter  ", address(router));
        console2.log("C-3b DogTagIssuerFactoryV2  ", address(factoryV2));
        console2.log("");
        console2.log("NEXT: verify on chain, then phase 2 with");
        console2.log("  CUTOVER_ROUTER=    ", address(router));
        console2.log("  CUTOVER_FACTORY_V2=", address(factoryV2));
        console2.log("  CUTOVER_CORE=      ", address(core));
    }

    // =============================================================================================
    // Phase 2 - C-4b and C-2. GOVERNANCE. C-4b is the irreversible step.
    // =============================================================================================

    function _phase2(CutoverSequence.Generation1 memory gen1) internal {
        CloneProvenanceRouter router = CloneProvenanceRouter(vm.envAddress("CUTOVER_ROUTER"));
        DogTagIssuerFactoryV2 factoryV2 = DogTagIssuerFactoryV2(vm.envAddress("CUTOVER_FACTORY_V2"));
        ProviderRegistry core = ProviderRegistry(vm.envAddress("CUTOVER_CORE"));

        // A second, explicit statement of intent. Phase 2 cannot be undone by any transaction, so it
        // must not be reachable by re-running the command that performed phase 1 with one variable
        // changed.
        require(
            vm.envOr("CUTOVER_CONFIRM_IRREVERSIBLE", false),
            "REFUSING: phase 2 appends a generation to the router, which has no removal, replace or "
            "reorder. Set CUTOVER_CONFIRM_IRREVERSIBLE=true to state that aloud."
        );

        // These read the REAL deployed contracts, so unlike the post-`new` requires elsewhere in this
        // file they are genuine evidence about chain state - and a failure here broadcasts nothing.
        require(address(router).code.length > 0, "C-4b: router address holds no code");
        require(address(factoryV2).code.length > 0, "C-4b: factory address holds no code");
        require(address(core).code.length > 0, "C-4b: core address holds no code");
        require(router.owner() == gen1.governance, "C-4b: router owner is not governance");
        require(router.generationCount() == 1, "C-4b: router must still hold exactly generation 1");
        require(router.generationAt(0) == gen1.factory, "C-4b: index 0 is not the generation-1 factory");
        require(!router.isGeneration(address(factoryV2)), "C-4b: factory is already a generation");
        // Binds the append to the factory that was actually built over THIS router. Without it a
        // mistyped CUTOVER_FACTORY_V2 naming some other deployed factory would append permanently.
        require(address(factoryV2.priorIndex()) == address(router), "C-4b: factory is not bound to this router");
        require(factoryV2.registry() == address(core), "C-4b: factory is not bound to this core");
        require(core.owner() == gen1.governance, "C-2: core owner is not governance");

        vm.startBroadcast();
        CutoverSequence.c4b_appendGeneration(router, factoryV2); // C-4b  <- IRREVERSIBLE
        CutoverSequence.c2_recordFactoryGeneration(core, factoryV2); // C-2 (chain half)
        vm.stopBroadcast();

        require(router.generationCount() == 2, "C-4b: router must hold exactly two generations");
        require(router.generationAt(0) == gen1.factory, "C-4b: generation 1 must remain at index 0");
        require(router.generationAt(1) == address(factoryV2), "C-4b: generation 2 must be at the TAIL");

        console2.log("C-4b appended generation 2  ", address(factoryV2));
        console2.log("C-2  recorded generation id (keccak 'dogtag-issuer-factory/2')");
        console2.log("     router generations[0]  ", router.generationAt(0));
        console2.log("     router generations[1]  ", router.generationAt(1));
    }

    // =============================================================================================
    // Phase 3 - C-5 and C-8. Deployments; nothing reads either until C-9.
    // =============================================================================================

    function _phase3(CutoverSequence.Generation1 memory gen1) internal {
        CloneProvenanceRouter router = CloneProvenanceRouter(vm.envAddress("CUTOVER_ROUTER"));
        ProviderRegistry core = ProviderRegistry(vm.envAddress("CUTOVER_CORE"));

        // Stated aloud, never defaulted. The value is `immutable` on the deployed registry, so a wrong
        // one is remedied only by redeploying - cheap today (no client reads the address until C-9),
        // expensive the moment one does. `DeployProtocolRegistryV2.s.sol` encodes the same policy:
        // mainnet takes exactly the 2-day default, and anything shorter requires the testnet opt-in and
        // still cannot go below the contract's own 1-hour floor.
        uint256 publishTimelock = vm.envUint("CUTOVER_PUBLISH_TIMELOCK_SECS");
        bool testnetDeploy = vm.envOr("CUTOVER_TESTNET_DEPLOY", false);
        if (!testnetDeploy) {
            require(
                publishTimelock == DEFAULT_PUBLISH_TIMELOCK_SECONDS_V2,
                "publish timelock must be the 2-day default unless CUTOVER_TESTNET_DEPLOY=true"
            );
        }
        require(publishTimelock >= MIN_PUBLISH_TIMELOCK_SECONDS_V2, "publish timelock below the 1-hour floor");

        // The router must already be the complete generation list, because C-5 binds it immutably.
        require(router.generationCount() == 2, "C-5: router does not yet hold both generations");
        require(router.generationAt(0) == gen1.factory, "C-5: generation 1 must be at index 0");

        vm.startBroadcast();
        VerificationRegistryConsent registryV2 = CutoverSequence.c5_deployRegistryV2(core, router, gen1); // C-5
        ProtocolRegistryV2 discoveryV2 = CutoverSequence.c8_deployDiscoveryV2(gen1.governance, publishTimelock); // C-8
        vm.stopBroadcast();

        require(address(registryV2.rootIndex()) == address(router), "C-5: rootIndex must be the ROUTER");
        require(address(registryV2.sbt()) == gen1.sbt, "C-5: SBT must be REUSED");
        require(address(registryV2.zkVerifier()) == gen1.verifier, "C-5: verifier must be REUSED");
        require(address(registryV2.issuerRegistry()) == address(core), "C-5: authority must be the new core");
        require(discoveryV2.PUBLISH_TIMELOCK() == publishTimelock, "C-8: publish timelock mismatch");

        console2.log("C-5  VerificationRegistryV2 ", address(registryV2));
        console2.log("C-8  ProtocolRegistryV2     ", address(discoveryV2));
        console2.log("     publish timelock (secs)", publishTimelock);
    }
}
