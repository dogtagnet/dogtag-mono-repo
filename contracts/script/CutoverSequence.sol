// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {DogTagIssuerV2} from "../src/DogTagIssuerV2.sol";
import {DogTagIssuerFactoryV2} from "../src/DogTagIssuerFactoryV2.sol";
import {CloneProvenanceRouter} from "../src/CloneProvenanceRouter.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";
import {ProtocolRegistryV2} from "../src/ProtocolRegistryV2.sol";

/// @title CutoverSequence — the ordered on-chain cutover, as executable code.
///
/// @notice ONE definition of the sequence, called by BOTH the broadcast script that produces the
/// transaction list (`RehearseCutover.s.sol`) and the fork test that asserts the sequence is correct
/// (`rehearsal/CutoverRehearsal.t.sol`).
///
/// That sharing is load-bearing rather than tidy. A rehearsal whose assertions run a sequence
/// assembled separately from the one the transaction list was captured from proves nothing about
/// the transaction list: the two could drift by a single argument and both would stay green. Every
/// step below is `internal`, so it inlines into whichever caller — no library linking, and no
/// deployed copy that could itself go stale.
///
/// # THE ORDER IS THE POINT, AND IT IS NOT THE PLAN'S ORDER
///
/// The registry plan (`dogtag-regplan-p3` §2) sequences C-3 (deploy the generation-2 factory) before
/// C-4 (deploy the provenance router), and specifies the router's generation list as
/// `[factoryV2, factoryV1]`. Both are wrong against the contracts as merged, and each fails in its
/// own way:
///
/// 1. `DogTagIssuerFactoryV2`'s constructor takes `priorIndex` as an `immutable` and PROBES it,
///    reverting `PriorIndexPrematurelyClaimsThisFactory` if the router already lists this factory
///    and `PriorIndexDoesNotAnswer` if nothing is there to ask. So the router must exist FIRST, and
///    must not yet name the factory being deployed. C-3-before-C-4 does not merely mis-order two
///    independent steps — it cannot be executed at all.
///
/// 2. `CloneProvenanceRouter`'s constructor stores `generations_` in the given order and
///    {rootIssuer} iterates from index 0 upward, returning the first non-zero answer. Index 0 is
///    therefore consulted FIRST. Passing `[factoryV2, factoryV1]` makes generation 2 the first
///    answer — newest-first — which is exactly the revocation bypass S-8 exists to prevent, and it
///    fails SILENTLY: nothing reverts, a revoked credential simply starts verifying again.
///
/// 3. Appending the generation-2 factory to the router is a step the plan does not contain at all.
///    Until it runs, `DogTagIssuerFactoryV2.registerRoot` reverts `FactoryNotRegisteredInPriorIndex`
///    and generation-2 issuance is dead — loudly, but only once someone tries to issue.
///
/// The corrected order below is what the rehearsal executes and what the transaction list records.
///
/// # WHO SIGNS
///
/// Every function here is either a plain deployment (needs NO authority — a constructor grants the
/// caller nothing) or a call gated on an existing admin role (needs governance). The split is
/// annotated per step and is what makes the transaction list approvable one step at a time rather
/// than as one block. The caller establishes the signing context: `vm.startBroadcast()` in the
/// script, `vm.startPrank(gov)` in the test. Nothing in this library touches a cheatcode, so the
/// same bytes execute either way.
library CutoverSequence {
    /// @notice Everything the cutover deploys. Generation-1 addresses are inputs, never outputs:
    /// not one of them moves.
    struct Deployment {
        ProviderRegistry core;
        DogTagIssuerV2 issuerImpl;
        CloneProvenanceRouter router;
        DogTagIssuerFactoryV2 factoryV2;
        VerificationRegistryConsent registryV2;
        ProtocolRegistryV2 discoveryV2;
    }

    /// @notice The live generation-1 set, read from `contracts/deployments/roax.json`.
    /// @dev `sbt` and `verifier` are REUSED by the generation-2 registry and must not be redeployed:
    /// `profileRoot` is per-contract and write-once, so a fresh SBT has no root for any existing tag
    /// and every one of them fails the `R == profileRoot(dogTagId)` binding; and the verifier is the
    /// frozen ceremony VK, which none of this work changes.
    struct Generation1 {
        address factory;
        address issuerRegistry;
        address sbt;
        address verifier;
        address governance;
    }

    /// @dev The generation id under which the generation-2 factory is recorded in the core. An
    /// opaque key, not an address, so the core's generation set can outlive any one factory.
    bytes32 internal constant GEN2_ID = keccak256("dogtag-issuer-factory/2");

    // =============================================================================================
    // C-1 — deploy the provider authority core
    // =============================================================================================

    /// @notice C-1. Deploy `ProviderRegistry`, empty, holding authority over nothing.
    /// @dev SIGNER: any. A deployment grants the deployer nothing; `authority` is the constructor
    /// argument and becomes `owner()`.
    /// IRREVERSIBLE: no. Nothing reads this contract until C-11, and a core with no grants
    /// authorizes nobody. Rolling back is abandoning the address.
    function c1_deployProviderRegistry(address governance) internal returns (ProviderRegistry) {
        return new ProviderRegistry(governance);
    }

    // =============================================================================================
    // C-3a — deploy the generation-2 issuer implementation
    // =============================================================================================

    /// @notice C-3a. Deploy the `DogTagIssuerV2` implementation the factory will clone.
    /// @dev SIGNER: any. The implementation is identified by CODE HASH in the factory's constructor
    /// (`impl.codehash == keccak256(type(DogTagIssuerV2).runtimeCode)`), so who deployed it is
    /// irrelevant and an ABI-shaped impostor is refused.
    /// IRREVERSIBLE: no. It is inert until a factory names it.
    function c3a_deployIssuerImpl() internal returns (DogTagIssuerV2) {
        return new DogTagIssuerV2();
    }

    // =============================================================================================
    // C-4 — deploy the provenance router. MUST precede C-3b and C-5.
    // =============================================================================================

    /// @notice C-4. Deploy `CloneProvenanceRouter` holding generation 1 ALONE, oldest first.
    ///
    /// @dev The generation-2 factory is NOT passed here and cannot be: it does not exist yet, and its
    /// own constructor requires this router to report it ABSENT. It is appended in C-4b.
    ///
    /// SIGNER: any. `governance` becomes `owner()` via the constructor.
    ///
    /// IRREVERSIBLE: not by itself — but this is the LAST MOMENT the decision can be made, because
    /// C-5's `rootIndex` is `immutable`. Rolling back after C-5 means redeploying the verification
    /// registry, which repeats every step after it.
    function c4_deployRouter(address factoryV1, address governance)
        internal
        returns (CloneProvenanceRouter)
    {
        address[] memory generations = new address[](1);
        generations[0] = factoryV1; // index 0 is consulted FIRST — oldest first.
        return new CloneProvenanceRouter(generations, governance);
    }

    // =============================================================================================
    // C-3b — deploy the generation-2 factory (AFTER the router exists)
    // =============================================================================================

    /// @notice C-3b. Deploy `DogTagIssuerFactoryV2` bound to the core and the router.
    /// @dev SIGNER: any. The factory has no admin at all; every dependency is `immutable` and is
    /// behaviour-probed in the constructor.
    /// IRREVERSIBLE: no, until a clone anchors a root through it. Rolling back before C-4b is free.
    function c3b_deployFactoryV2(DogTagIssuerV2 impl, ProviderRegistry core, CloneProvenanceRouter router)
        internal
        returns (DogTagIssuerFactoryV2)
    {
        return new DogTagIssuerFactoryV2(address(impl), address(core), address(router));
    }

    // =============================================================================================
    // C-4b — append generation 2 to the router. NOT IN THE PLAN.
    // =============================================================================================

    /// @notice C-4b. Append the generation-2 factory to the router, at the TAIL.
    ///
    /// @dev Appending at the tail under oldest-first resolution is MONOTONE: the new generation is
    /// consulted only after every existing one has answered zero, so it can add an answer for a root
    /// that previously resolved to nothing and can never change one that already exists.
    ///
    /// SIGNER: GOVERNANCE (`onlyOwner` on the router).
    ///
    /// IRREVERSIBLE: YES. There is no removal, no replace and no reorder — deliberately, because
    /// removal is a denial of service aimed at a whole generation. Rolling this back requires a new
    /// router, and therefore a new verification registry.
    function c4b_appendGeneration(CloneProvenanceRouter router, DogTagIssuerFactoryV2 factoryV2) internal {
        router.appendGeneration(address(factoryV2));
    }

    // =============================================================================================
    // C-2 — record the generation-2 factory in the core
    // =============================================================================================

    /// @notice C-2 (chain half). Record the generation-2 factory as a recognised generation.
    ///
    /// @dev The rest of C-2 — registering providers, attaching clones, mirroring issuance grants —
    /// is BLOCKED ON KYC INPUTS that do not exist (plan §4 items 1-9) and, for the five existing
    /// generation-1 clones, is not executable against this core at all: `attachService` reads
    /// `owner()` off the service and generation-1 `DogTagIssuer` has none. See
    /// `test_c2_cannot_attach_an_ownerless_generation_1_clone`.
    ///
    /// SIGNER: GOVERNANCE (`onlyOwner`).
    ///
    /// IRREVERSIBLE: partially. A generation can be added and deprecated but never repointed or
    /// removed, and `deprecateFactoryGeneration` is TERMINAL — there is no reactivation path.
    function c2_recordFactoryGeneration(ProviderRegistry core, DogTagIssuerFactoryV2 factoryV2) internal {
        core.addFactoryGeneration(GEN2_ID, address(factoryV2));
    }

    // =============================================================================================
    // C-5 — deploy the generation-2 verification registry
    // =============================================================================================

    /// @notice C-5. Deploy `VerificationRegistryConsent` V2 over the new core and the router, reusing
    /// the EXISTING SBT and the EXISTING verifier.
    ///
    /// @dev SIGNER: any. `governance` becomes `DEFAULT_ADMIN_ROLE` via
    /// `AccessControlDefaultAdminRules`.
    ///
    /// IRREVERSIBLE: the BINDINGS are. `issuerRegistry`, `sbt` and `rootIndex` are all `immutable`,
    /// so a wrong argument here is remedied only by redeploying and repeating every client step.
    /// Deploying it changes nothing on its own: no client reads it until C-9.
    function c5_deployRegistryV2(
        ProviderRegistry core,
        CloneProvenanceRouter router,
        Generation1 memory gen1
    ) internal returns (VerificationRegistryConsent) {
        return new VerificationRegistryConsent(
            address(core), // the new authority core
            gen1.sbt, // REUSED — write-once profileRoot per contract
            gen1.verifier, // REUSED — frozen ceremony VK
            address(router), // the provenance router, NOT a factory
            gen1.governance
        );
    }

    // =============================================================================================
    // C-6 — migrate a VERIFY: relayer capability into the new core
    // =============================================================================================

    /// @notice C-6. Grant one relayer its `VERIFY:<purpose>` capability in the new core.
    ///
    /// @dev PRECEDES C-9. `restrictToWhitelistedRelayers` defaults true, and the gate at
    /// `VerificationRegistryConsent.sol:151-155` fires BEFORE root resolution, so a client repointed
    /// at the V2 registry ahead of its relayer reverts `!verify-wl` on every verification — an error
    /// that names a whitelist rather than a migration, so it reads as a permissions bug.
    ///
    /// This is a SEPARATE KEYSPACE from the issuance record types. `purpose` is the raw purpose
    /// signal; the core hashes it into the `VERIFY:` key itself, byte-identically to the registry's
    /// own `_verifyKey`.
    ///
    /// SIGNER: GOVERNANCE (`onlyOwner`).
    ///
    /// IRREVERSIBLE: no. `setVerifierCapability(..., false)` withdraws it in one transaction.
    function c6_migrateVerifierCapability(ProviderRegistry core, bytes32 purpose, address relayer)
        internal
    {
        core.setVerifierCapability(purpose, relayer, true);
    }

    // =============================================================================================
    // C-8 — deploy the generation-2 discovery registry, with a real timelock
    // =============================================================================================

    /// @notice C-8. Deploy `ProtocolRegistryV2` with a NON-ZERO publish timelock.
    ///
    /// @dev The live generation-1 `ProtocolRegistry` carries `PUBLISH_TIMELOCK == 0` and the value is
    /// `immutable`, so its publisher key can repoint the entire declared protocol set in one block.
    /// This deployment is the only opportunity to fix that. The contract enforces a floor
    /// (`MIN_PUBLISH_TIMELOCK`, 1 hour) in its CONSTRUCTOR, so zero is unrepresentable — but the
    /// floor is a floor, not a target: mainnet uses `DEFAULT_PUBLISH_TIMELOCK` (2 days).
    ///
    /// SIGNER: any. `admin` and `publisher` are constructor arguments.
    ///
    /// IRREVERSIBLE: the TIMELOCK is. It is `immutable`; a wrong value is remedied only by
    /// redeploying and repointing every client that carries the discovery address — including two
    /// compile-time mobile bundles.
    function c8_deployDiscoveryV2(address governance, uint256 publishTimelock)
        internal
        returns (ProtocolRegistryV2)
    {
        return new ProtocolRegistryV2(governance, governance, publishTimelock);
    }

    // =============================================================================================
    // The whole on-chain sequence, in order
    // =============================================================================================

    /// @notice Runs C-1 through C-8's on-chain steps in the corrected order.
    /// @dev The caller must already be in a broadcast or prank context signing as `gen1.governance`,
    /// because C-4b, C-2 and C-6 are all `onlyOwner`.
    function runOnchainSequence(Generation1 memory gen1, uint256 publishTimelock)
        internal
        returns (Deployment memory d)
    {
        d.core = c1_deployProviderRegistry(gen1.governance); // C-1
        d.issuerImpl = c3a_deployIssuerImpl(); // C-3a
        d.router = c4_deployRouter(gen1.factory, gen1.governance); // C-4  <- BEFORE the factory
        d.factoryV2 = c3b_deployFactoryV2(d.issuerImpl, d.core, d.router); // C-3b
        c4b_appendGeneration(d.router, d.factoryV2); // C-4b <- not in the plan
        c2_recordFactoryGeneration(d.core, d.factoryV2); // C-2 (chain half)
        d.registryV2 = c5_deployRegistryV2(d.core, d.router, gen1); // C-5
        d.discoveryV2 = c8_deployDiscoveryV2(gen1.governance, publishTimelock); // C-8
    }
}
