// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Test} from "forge-std/Test.sol";
import {CutoverSequence} from "../script/CutoverSequence.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {DogTagIssuerV2} from "../src/DogTagIssuerV2.sol";
import {DogTagIssuerFactoryV2} from "../src/DogTagIssuerFactoryV2.sol";
import {CloneProvenanceRouter} from "../src/CloneProvenanceRouter.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";

interface ISBT {
    function profileRoot(uint256 id) external view returns (bytes32);
    function mintCustodial(uint256 id, bytes32 root) external;
    function grantRole(bytes32 role, address account) external;
    function hasRole(bytes32 role, address account) external view returns (bool);
}

interface IIssuerRegistryV1 {
    function whitelistFor(bytes32 recordType, address signer) external;
    function delistFor(bytes32 recordType, address signer) external;
    function isWhitelistedFor(bytes32 recordType, address signer) external view returns (bool);
}

interface IFactoryV1 {
    function rootIssuer(bytes32 root) external view returns (address);
    function isClone(address candidate) external view returns (bool);
}

/// @title CutoverRehearsal — the whole registry cutover, executed against a fork of the live chain.
///
/// @notice REHEARSAL ONLY, and it deploys nothing live. It runs the exact sequence in
/// `script/CutoverSequence.sol` — the same code the transaction list is captured from — against
/// `anvil --fork-url` or a direct fork of ROAX, and then asserts the six properties each of which
/// corresponds to a way the cutover fails SILENTLY.
///
/// A fork rather than a testnet deploy because a fork answers what the DEPLOYED BYTECODE PERMITS,
/// not what the source suggests. Every generation-1 address here is the real one, every generation-1
/// contract is the real deployed instance, and the Groth16 proof is verified by the real deployed
/// ceremony verifier.
///
/// # THE THREE CORRECTIONS THIS REHEARSAL FOUND
///
/// Each is an EXECUTED assertion in this file rather than an argument, because the plan's version
/// looks reasonable on paper and only the chain settles it:
///
///   `test_correction_1_...` — the plan's C-3-before-C-4 order cannot be executed at all.
///   `test_correction_2_...` — the plan's `[factoryV2, factoryV1]` array resolves NEWEST first,
///                             which is the revocation bypass.
///   `test_correction_3_...` — C-2's "attach the existing clones" is unexecutable against the
///                             merged core, because generation-1 clones have no `owner()`.
///
/// # WHY THE PROOF FIXTURE IS NOT MOCK DATA
///
/// `test/consent-fixture.json` is a REAL Groth16 proof from `circuits/consent.circom` against the
/// frozen ceremony key, and it is verified here by the REAL deployed `Groth16VerifierConsent` at the
/// address the live registry names. What the rehearsal synthesises is only the things a rehearsal
/// must synthesise — a provider id, a controller and a signer — and every one of those is labelled
/// as a rehearsal input below, because they are precisely the KYC facts C-2 is blocked on.
contract CutoverRehearsalTest is Test {
    // ---------------------------------------------------------------------------------------------
    // Fork
    // ---------------------------------------------------------------------------------------------

    /// @dev Pinned so the rehearsal is reproducible. `scripts/derive-cutover-inventory.sh` derives
    /// the historical-root fixture at this same block; the two must move together.
    uint256 internal constant PIN_BLOCK = 304_000;

    // ---------------------------------------------------------------------------------------------
    // Generation 1 — every address read from the ledger, never transcribed
    // ---------------------------------------------------------------------------------------------

    CutoverSequence.Generation1 internal gen1;

    // ---------------------------------------------------------------------------------------------
    // What the cutover deploys
    // ---------------------------------------------------------------------------------------------

    CutoverSequence.Deployment internal d;

    // ---------------------------------------------------------------------------------------------
    // The real consent proof
    // ---------------------------------------------------------------------------------------------

    uint256[2] internal pA;
    uint256[2][2] internal pB;
    uint256[2] internal pC;
    uint256[7] internal pub;
    uint256 internal fixtureDogTagId;
    bytes32 internal fixtureRoot;
    bytes32 internal fixturePurpose;
    address internal fixtureRelayer;

    // ---------------------------------------------------------------------------------------------
    // REHEARSAL INPUTS — synthesised, and blocked on KYC in the live cutover (plan §4)
    // ---------------------------------------------------------------------------------------------

    /// @dev Plan §4 item 1: an opaque KYC-assigned provider id. There is no such id today.
    bytes20 internal constant REHEARSAL_PROVIDER_ID = bytes20(uint160(0xA11CE));
    /// @dev Plan §4 item 2: the provider's controller key. Not the issuance signer.
    address internal constant REHEARSAL_CONTROLLER = address(uint160(0xC0417401));
    /// @dev Plan §4 item 4: which provider a signer belongs to is not derivable from the chain.
    address internal constant REHEARSAL_GEN2_SIGNER = address(uint160(0x5169E12));
    address internal constant REHEARSAL_GEN1_SIGNER = address(uint160(0x5169E11));

    bytes32 internal constant REHEARSAL_RECORD_TYPE = keccak256("VACCINATION");
    bytes32 internal constant SBT_ISSUER_ROLE = keccak256("ISSUER");

    /// @dev Plan §4 item 6: the registrar-controlled public identity statement — legal name,
    /// jurisdiction, licence references. `registerProvider` REFUSES a zero digest, schema or hash
    /// algorithm (`BadIdentityAnchor`), so a provider cannot be registered without one at all. That
    /// is a second, independent thing C-2 is blocked on, and it is not visible from the plan text.
    bytes32 internal constant REHEARSAL_IDENTITY_DIGEST = keccak256("REHEARSAL ONLY - not a real KYC record");
    uint32 internal constant REHEARSAL_IDENTITY_SCHEMA = 1;
    uint16 internal constant REHEARSAL_IDENTITY_CODEC = 0;
    uint8 internal constant REHEARSAL_IDENTITY_HASH_ALG = 0x12; // multihash sha2-256

    /// @dev One place, so every caller registers the provider the same way.
    function _registerRehearsalProvider() internal {
        d.core.registerProvider(
            REHEARSAL_PROVIDER_ID,
            REHEARSAL_CONTROLLER,
            REHEARSAL_IDENTITY_DIGEST,
            REHEARSAL_IDENTITY_SCHEMA,
            REHEARSAL_IDENTITY_CODEC,
            REHEARSAL_IDENTITY_HASH_ALG,
            ""
        );
    }

    function setUp() public {
        // No default. A missing endpoint must fail LOUDLY rather than let the suite self-skip: a
        // rehearsal that reports green in exactly the case it did not run is worthless.
        vm.createSelectFork(vm.envString("ROAX_FORK_RPC"), PIN_BLOCK);
        assertEq(block.chainid, 135, "fork is not ROAX");

        string memory ledger = vm.readFile("deployments/roax.json");
        gen1.factory = vm.parseJsonAddress(ledger, ".DogTagIssuerFactory");
        gen1.issuerRegistry = vm.parseJsonAddress(ledger, ".IssuerRegistry");
        gen1.sbt = vm.parseJsonAddress(ledger, ".DogTagSBTConsent");
        gen1.verifier = vm.parseJsonAddress(ledger, ".Groth16VerifierConsent");
        gen1.governance = vm.parseJsonAddress(ledger, ".admin");

        _loadConsentFixture();

        // The whole on-chain cutover, signed by governance — the same code the transaction list is
        // captured from.
        vm.startPrank(gen1.governance);
        d = CutoverSequence.runOnchainSequence(gen1, 1 hours);
        vm.stopPrank();

        // The tag the credential belongs to. Minting is gated on the SBT's OWN role rather than on
        // any registry (plan C-10b), which is why the SBT is reused rather than moved and why this
        // step is easy to forget: a generation-2 clone whose signer can `issue(R)` still cannot mint.
        vm.startPrank(gen1.governance);
        ISBT(gen1.sbt).grantRole(SBT_ISSUER_ROLE, gen1.governance);
        ISBT(gen1.sbt).mintCustodial(fixtureDogTagId, fixtureRoot);
        vm.stopPrank();
        assertEq(ISBT(gen1.sbt).profileRoot(fixtureDogTagId), fixtureRoot, "tag not bound");
    }

    function _loadConsentFixture() internal {
        string memory j = vm.readFile("test/consent-fixture.json");
        bytes32[] memory a = vm.parseJsonBytes32Array(j, ".a");
        bytes32[] memory b0 = vm.parseJsonBytes32Array(j, ".b[0]");
        bytes32[] memory b1 = vm.parseJsonBytes32Array(j, ".b[1]");
        bytes32[] memory c = vm.parseJsonBytes32Array(j, ".c");
        bytes32[] memory p = vm.parseJsonBytes32Array(j, ".pub");
        pA = [uint256(a[0]), uint256(a[1])];
        pB = [[uint256(b0[0]), uint256(b0[1])], [uint256(b1[0]), uint256(b1[1])]];
        pC = [uint256(c[0]), uint256(c[1])];
        for (uint256 i; i < 7; i++) {
            pub[i] = uint256(p[i]);
        }
        fixtureDogTagId = pub[0];
        fixturePurpose = bytes32(pub[1]);
        fixtureRelayer = address(uint160(pub[2]));
        fixtureRoot = bytes32(pub[4]);
    }

    // =============================================================================================
    // Helpers — anchoring the same root in each generation
    // =============================================================================================

    /// @dev A REAL deployed generation-1 clone, resolved from the chain rather than transcribed: the
    /// clone that anchored the first historical root. A literal address here would be a copy that
    /// goes stale silently, which this repo has a standing rule against.
    function _aLiveGeneration1Clone() internal view returns (address clone) {
        string memory f = vm.readFile("rehearsal/fixtures/historical-roots.json");
        bytes32[] memory roots = vm.parseJsonBytes32Array(f, ".roots");
        clone = IFactoryV1(gen1.factory).rootIssuer(roots[0]);
        require(clone != address(0), "no live generation-1 clone resolved");
    }

    /// @dev Anchors `fixtureRoot` in that real clone, through the real generation-1 whitelist.
    /// The record type comes from the CLONE, because `onlyWhitelisted` asks about the clone's own
    /// record type and assuming a label here would whitelist the wrong key.
    function _anchorInGeneration1() internal returns (address clone) {
        clone = _aLiveGeneration1Clone();
        assertTrue(IFactoryV1(gen1.factory).isClone(clone), "not a generation-1 clone");

        bytes32 cloneRecordType = DogTagIssuer(clone).recordType();
        vm.prank(gen1.governance);
        IIssuerRegistryV1(gen1.issuerRegistry).whitelistFor(cloneRecordType, REHEARSAL_GEN1_SIGNER);

        vm.prank(REHEARSAL_GEN1_SIGNER);
        DogTagIssuer(clone).issue(fixtureRoot);
    }

    /// @dev Stands up a generation-2 provider and clone through the real merged core, then anchors
    /// `fixtureRoot` in it. Every registrar call here is C-2/C-11 work.
    function _anchorInGeneration2() internal returns (address clone) {
        clone = _createGeneration2Clone();

        // C-11: the issuance grant. This is the first moment a credential can exist that a
        // generation-1-only client cannot verify, which is why every client step precedes it.
        vm.prank(gen1.governance);
        d.core.setIssuanceCapability(clone, REHEARSAL_GEN2_SIGNER, true);
        assertTrue(d.core.canIssue(clone, REHEARSAL_GEN2_SIGNER), "generation-2 signer cannot issue");

        vm.prank(REHEARSAL_GEN2_SIGNER);
        DogTagIssuerV2(clone).issue(fixtureRoot);
    }

    function _createGeneration2Clone() internal returns (address clone) {
        vm.startPrank(gen1.governance);
        _registerRehearsalProvider();
        d.core.setProviderStanding(REHEARSAL_PROVIDER_ID, ProviderRegistry.Standing.ACTIVE);
        d.core.setServiceCreationApproval(REHEARSAL_PROVIDER_ID, REHEARSAL_RECORD_TYPE, true);
        vm.stopPrank();

        // Self-service: the CONTROLLER deploys its own clone and owns it.
        vm.prank(REHEARSAL_CONTROLLER);
        clone = d.factoryV2.createIssuer(REHEARSAL_PROVIDER_ID, REHEARSAL_RECORD_TYPE, 0);

        vm.startPrank(gen1.governance);
        d.core.attachService(
            REHEARSAL_PROVIDER_ID, clone, CutoverSequence.GEN2_ID, REHEARSAL_CONTROLLER
        );
        d.core.setServiceStanding(clone, ProviderRegistry.Standing.ACTIVE);
        vm.stopPrank();

        vm.prank(REHEARSAL_CONTROLLER);
        d.core.repointService(clone);
    }

    /// @dev C-6 for the fixture's relayer, on the new core.
    function _migrateRelayer() internal {
        vm.prank(gen1.governance);
        CutoverSequence.c6_migrateVerifierCapability(d.core, fixturePurpose, fixtureRelayer);
    }

    /// @dev The generation-1 equivalent, so a generation-1 submission gets past `!verify-wl` and
    /// reaches the root resolution the assertion is actually about.
    function _whitelistRelayerOnGeneration1() internal {
        bytes32 verifyKey = keccak256(abi.encode("VERIFY:", fixturePurpose));
        vm.prank(gen1.governance);
        IIssuerRegistryV1(gen1.issuerRegistry).whitelistFor(verifyKey, fixtureRelayer);
    }

    // =============================================================================================
    // ASSERTION 1 — all historical roots resolve through the router to their original clone
    // =============================================================================================

    /// Silent failure it guards: the generation-2 registry's `rootIndex` is immutable, so a router
    /// that misses a historical root strands that credential PERMANENTLY and nothing errors at
    /// deploy time — the credential simply answers `unknown root` forever after.
    function test_1_every_historical_root_resolves_through_the_router() public view {
        string memory f = vm.readFile("rehearsal/fixtures/historical-roots.json");
        bytes32[] memory roots = vm.parseJsonBytes32Array(f, ".roots");
        uint256 declared = vm.parseJsonUint(f, ".rootCount");
        uint256 pinned = vm.parseJsonUint(f, ".pinnedBlock");

        // The fixture must describe THIS fork, or it is describing a different chain state.
        assertEq(pinned, PIN_BLOCK, "root fixture pinned to a different block than the fork");
        assertEq(roots.length, declared, "root fixture count disagrees with its own list");
        assertGt(roots.length, 0, "no historical roots to check");

        for (uint256 i; i < roots.length; i++) {
            // Compared against what the generation-1 factory ITSELF answers, not against a
            // transcribed clone address. That makes the assertion self-checking against the chain
            // rather than against this file.
            address expected = IFactoryV1(gen1.factory).rootIssuer(roots[i]);
            assertTrue(expected != address(0), "historical root missing from the generation-1 index");
            assertEq(
                d.router.rootIssuer(roots[i]), expected, "router lost a historical root's issuing clone"
            );
        }
    }

    // =============================================================================================
    // ASSERTION 2 — a generation-1 credential verifies on the generation-2 registry
    // =============================================================================================

    /// Silent failure it guards: this is the whole reason the router exists. Without it every one of
    /// the historical credentials stops verifying the moment clients repoint, unrecoverably.
    function test_2_a_generation_1_credential_verifies_on_the_generation_2_registry() public {
        address clone = _anchorInGeneration1();
        _migrateRelayer();

        vm.prank(fixtureRelayer);
        d.registryV2.recordVerificationZK(pA, pB, pC, pub);

        assertEq(d.router.rootIssuer(fixtureRoot), clone, "resolved the wrong clone");
        assertTrue(d.registryV2.consumed(bytes32(pub[3])), "nullifier not consumed");
    }

    // =============================================================================================
    // ASSERTION 3 — a generation-2 credential verifies on the generation-2 registry
    // =============================================================================================

    function test_3_a_generation_2_credential_verifies_on_the_generation_2_registry() public {
        address clone = _anchorInGeneration2();
        _migrateRelayer();

        vm.prank(fixtureRelayer);
        d.registryV2.recordVerificationZK(pA, pB, pC, pub);

        assertEq(d.router.rootIssuer(fixtureRoot), clone, "resolved the wrong clone");
        assertTrue(d.registryV2.consumed(bytes32(pub[3])), "nullifier not consumed");
    }

    // =============================================================================================
    // ASSERTION 4 — a generation-2 credential does NOT verify on the generation-1 registry
    // =============================================================================================

    /// This is WHY client repointing must precede generation-2 issuance. If it silently passed, the
    /// ordering constraint would look optional.
    ///
    /// The exact revert string is load-bearing. `!verify-wl` fires at
    /// `VerificationRegistryConsent.sol:153`, BEFORE root resolution at `:187`, so an unwhitelisted
    /// relayer reverts for a reason that has nothing to do with root resolution and a bare
    /// `vm.expectRevert()` would go green while proving nothing. The relayer is therefore whitelisted
    /// on generation 1 FIRST, so the submission reaches the check this assertion is about.
    function test_4_a_generation_2_credential_does_not_verify_on_the_generation_1_registry() public {
        _anchorInGeneration2();
        _whitelistRelayerOnGeneration1();

        VerificationRegistryConsent gen1Registry =
            VerificationRegistryConsent(_gen1RegistryAddress());

        // Precondition, so the assertion cannot pass for the relayer reason: the relayer IS
        // authorised on generation 1, and the shared SBT means the tag binding also holds there.
        bytes32 verifyKey = keccak256(abi.encode("VERIFY:", fixturePurpose));
        assertTrue(
            IIssuerRegistryV1(gen1.issuerRegistry).isWhitelistedFor(verifyKey, fixtureRelayer),
            "precondition: relayer must be authorised on generation 1"
        );
        assertEq(
            ISBT(gen1.sbt).profileRoot(fixtureDogTagId), fixtureRoot, "precondition: tag binding holds"
        );

        vm.prank(fixtureRelayer);
        vm.expectRevert("unknown root");
        gen1Registry.recordVerificationZK(pA, pB, pC, pub);
    }

    // =============================================================================================
    // ASSERTION 5 — an unmigrated relayer fails `!verify-wl`
    // =============================================================================================

    /// This is WHY capability migration precedes client repointing.
    ///
    /// Paired with the migrated case on the SAME credential and the SAME registry in one test, or it
    /// would be vacuous: a revert proves nothing unless the only difference from a pass is the thing
    /// under test.
    function test_5_an_unmigrated_relayer_fails_verify_wl_and_a_migrated_one_passes() public {
        _anchorInGeneration2();

        assertFalse(
            d.core.canVerify(fixturePurpose, fixtureRelayer), "precondition: relayer not yet migrated"
        );

        vm.prank(fixtureRelayer);
        vm.expectRevert("!verify-wl");
        d.registryV2.recordVerificationZK(pA, pB, pC, pub);

        // The ONLY change: C-6 runs.
        _migrateRelayer();

        vm.prank(fixtureRelayer);
        d.registryV2.recordVerificationZK(pA, pB, pC, pub);
        assertTrue(d.registryV2.consumed(bytes32(pub[3])), "migrated relayer still could not verify");
    }

    // =============================================================================================
    // ASSERTION 6 — a revoked generation-1 root cannot be resurrected in generation 2
    // =============================================================================================

    /// Half one: DEFENCE IN DEPTH. The generation-2 factory asks the router whether any earlier
    /// generation already holds the root and refuses, so the duplicate never comes into existence.
    function test_6a_the_generation_2_factory_refuses_a_root_generation_1_already_holds() public {
        address gen1Clone = _anchorInGeneration1();
        vm.prank(REHEARSAL_GEN1_SIGNER);
        DogTagIssuer(gen1Clone).revoke(fixtureRoot);
        assertFalse(DogTagIssuer(gen1Clone).isValid(fixtureRoot), "precondition: revoked");

        address gen2Clone = _createGeneration2Clone();
        vm.prank(gen1.governance);
        d.core.setIssuanceCapability(gen2Clone, REHEARSAL_GEN2_SIGNER, true);

        // `registerRoot` asks the router `isRootAnchored` and refuses. Generation 1's own factory
        // would have answered "root taken" only for a root IT held; this is the cross-generation
        // question, which is why it names a different failure.
        vm.prank(REHEARSAL_GEN2_SIGNER);
        vm.expectRevert("root taken upstream");
        DogTagIssuerV2(gen2Clone).issue(fixtureRoot);
    }

    /// Half two: LOAD-BEARING. Oldest-first holds even against a later generation that is unguarded,
    /// buggy or hostile — which is the case the write guard above cannot cover, and the reason the
    /// ordering rather than the guard is what the property rests on.
    ///
    /// The later generation here is a REAL, unmodified generation-1 `DogTagIssuerFactory` against an
    /// `IssuerRegistry` this test admins. A guarded factory would make the attack setup revert and
    /// the test would pass while proving nothing.
    function test_6b_a_revoked_root_survives_re_anchoring_by_an_unguarded_later_generation() public {
        address gen1Clone = _anchorInGeneration1();
        vm.prank(REHEARSAL_GEN1_SIGNER);
        DogTagIssuer(gen1Clone).revoke(fixtureRoot);

        // A later, unguarded generation entirely under an attacker's control.
        (address hostileClone, address hostileFactory) = _deployUnguardedLaterGeneration();

        // It really does anchor the revoked root — the duplicate exists.
        assertEq(
            IFactoryV1(hostileFactory).rootIssuer(fixtureRoot),
            hostileClone,
            "the later generation did not actually anchor the root"
        );

        // Oldest-first: the router still names the ORIGINAL clone, where the root is revoked.
        assertEq(
            d.router.rootIssuer(fixtureRoot), gen1Clone, "router did not resolve oldest generation first"
        );

        _migrateRelayer();
        vm.prank(fixtureRelayer);
        vm.expectRevert("cred !valid");
        d.registryV2.recordVerificationZK(pA, pB, pC, pub);
    }

    function _deployUnguardedLaterGeneration() internal returns (address clone, address factory) {
        address attacker = address(uint160(0xBADBAD));

        vm.startPrank(attacker);
        IssuerRegistry reg = new IssuerRegistry(attacker);
        DogTagIssuer impl = new DogTagIssuer();
        DogTagIssuerFactory fac = new DogTagIssuerFactory(address(impl), address(reg), attacker);
        clone = fac.createIssuer("hostile", REHEARSAL_RECORD_TYPE, attacker);
        reg.whitelistFor(REHEARSAL_RECORD_TYPE, attacker);
        DogTagIssuer(clone).issue(fixtureRoot);
        vm.stopPrank();

        // Appended at the TAIL, which is the only mutation the router has.
        vm.prank(gen1.governance);
        d.router.appendGeneration(address(fac));

        factory = address(fac);
    }

    // =============================================================================================
    // ASSERTION 7 (C-12) — the generation-1 freeze stops new issuance and breaks NO credential
    // =============================================================================================

    /// Silent failure it guards: C-12 exists because dual-generation resolution leaves the OLD
    /// registry's `WHITELIST_ADMIN` a live trust surface for the NEW verifier — a compromised old
    /// admin could whitelist a signer, issue through an old clone, and have that root resolve through
    /// the router. Freezing collapses that surface. The plan asserts the freeze is safe "because
    /// `isValid` never consults the registry", and that is the claim under test here: if it were
    /// wrong, the freeze would silently invalidate all 19 historical credentials, which is the exact
    /// outcome the router was built to prevent.
    ///
    /// Both halves are needed. A test that only showed issuance stopping would pass while the freeze
    /// destroyed every existing credential; a test that only showed verification surviving would pass
    /// while the freeze failed to close the surface it exists to close.
    function test_7_the_generation_1_freeze_stops_new_issuance_and_preserves_verification() public {
        address gen1Clone = _anchorInGeneration1();
        bytes32 cloneRecordType = DogTagIssuer(gen1Clone).recordType();

        // Precondition, or "issuance is refused" would be vacuous: the signer CAN issue right now.
        assertTrue(
            IIssuerRegistryV1(gen1.issuerRegistry).isWhitelistedFor(
                cloneRecordType, REHEARSAL_GEN1_SIGNER
            ),
            "precondition: the generation-1 signer must be whitelisted before the freeze"
        );

        // C-12 itself. This is the only call site of `delistFor`, and the freeze is one transaction
        // per (recordType, signer) — the set being exactly the KYC reconciliation, which is why the
        // transaction list does not enumerate it.
        vm.prank(gen1.governance);
        IIssuerRegistryV1(gen1.issuerRegistry).delistFor(cloneRecordType, REHEARSAL_GEN1_SIGNER);
        assertFalse(
            IIssuerRegistryV1(gen1.issuerRegistry).isWhitelistedFor(
                cloneRecordType, REHEARSAL_GEN1_SIGNER
            ),
            "the freeze did not take effect"
        );

        // (1) New generation-1 issuance is refused. A DIFFERENT root, so the refusal cannot be
        //     `BadRoot` from the already-anchored one — `onlyWhitelisted` is checked first, but a
        //     reused root would make the assertion pass under either rule.
        bytes32 freshRoot = keccak256("a root never anchored anywhere");
        assertEq(DogTagIssuer(gen1Clone).issuedAt(freshRoot), 0, "precondition: fresh root");
        vm.prank(REHEARSAL_GEN1_SIGNER);
        vm.expectRevert(abi.encodeWithSignature("NotWhitelisted()"));
        DogTagIssuer(gen1Clone).issue(freshRoot);

        // (2) Every historical root still resolves to its original clone. Delisting is forward-only.
        string memory f = vm.readFile("rehearsal/fixtures/historical-roots.json");
        bytes32[] memory roots = vm.parseJsonBytes32Array(f, ".roots");
        for (uint256 i; i < roots.length; i++) {
            address expected = IFactoryV1(gen1.factory).rootIssuer(roots[i]);
            assertTrue(expected != address(0), "historical root vanished from the generation-1 index");
            assertEq(d.router.rootIssuer(roots[i]), expected, "the freeze moved a historical root");
        }

        // (3) And a credential anchored by the NOW-DELISTED signer still verifies end to end on the
        //     generation-2 registry. This is the half that makes the freeze safe rather than merely
        //     effective, and only a real verification establishes it.
        _migrateRelayer();
        vm.prank(fixtureRelayer);
        d.registryV2.recordVerificationZK(pA, pB, pC, pub);
        assertTrue(
            d.registryV2.consumed(bytes32(pub[3])),
            "the freeze broke verification of a credential its own signer had already anchored"
        );
    }

    // =============================================================================================
    // CORRECTION 1 — the plan's C-3-before-C-4 order cannot be executed
    // =============================================================================================

    /// The plan (`dogtag-regplan-p3` §2) deploys the generation-2 factory at C-3 and the router at
    /// C-4. `DogTagIssuerFactoryV2`'s `priorIndex` is immutable, MANDATORY and non-zero, and is
    /// probed in the constructor — so at C-3-time there is nothing for it to bind to. This is not a
    /// mis-ordering of two independent steps; the earlier step cannot execute.
    ///
    /// It also proves the step the plan does not contain at all: until the router has been APPENDED
    /// to, a perfectly well-formed generation-2 clone cannot anchor anything.
    function test_correction_1_the_generation_2_factory_cannot_be_deployed_before_the_router() public {
        ProviderRegistry core = new ProviderRegistry(gen1.governance);
        DogTagIssuerV2 impl = new DogTagIssuerV2();

        // C-3 as the plan orders it: the router does not exist, so the only thing to pass is nothing.
        vm.expectRevert(abi.encodeWithSignature("ZeroAddress()"));
        new DogTagIssuerFactoryV2(address(impl), address(core), address(0));

        // Nor can the gap be papered over with a placeholder: the dependency is behaviour-probed,
        // and an EOA staticcall SUCCEEDS with empty returndata — the silent shape the probe rejects.
        vm.expectRevert(abi.encodeWithSignature("NotAContract(address)", address(0xDEAD)));
        new DogTagIssuerFactoryV2(address(impl), address(core), address(0xDEAD));

        // C-4b, the missing step. A factory deployed correctly but never appended is not a partial
        // success: every clone it makes is inert, and it says so only when someone tries to issue.
        address[] memory gens = new address[](1);
        gens[0] = gen1.factory;
        CloneProvenanceRouter router = new CloneProvenanceRouter(gens, gen1.governance);
        DogTagIssuerFactoryV2 orphan =
            new DogTagIssuerFactoryV2(address(impl), address(core), address(router));
        assertFalse(router.isGeneration(address(orphan)), "precondition: not yet appended");

        vm.expectRevert(
            abi.encodeWithSignature("FactoryNotRegisteredInPriorIndex(address)", address(orphan))
        );
        orphan.registerRoot(bytes32(uint256(1)));

        // And after C-4b it is a different failure — the ordinary "you are not one of my clones",
        // which is what shows the generation check really was the thing blocking it.
        vm.prank(gen1.governance);
        router.appendGeneration(address(orphan));
        vm.expectRevert("!clone");
        orphan.registerRoot(bytes32(uint256(1)));
    }

    // =============================================================================================
    // CORRECTION 2 — the plan's generation array resolves NEWEST first
    // =============================================================================================

    /// The plan specifies the router "over `[factoryV2, factoryV1]`". The constructor stores the
    /// array in order and {rootIssuer} iterates from index 0, so that literal array makes generation
    /// 2 answer first. This test builds exactly that router and shows the revoked credential
    /// resurrected — the assertion RED, executed rather than described.
    function test_correction_2_the_plans_generation_array_resurrects_a_revoked_credential() public {
        address gen1Clone = _anchorInGeneration1();
        vm.prank(REHEARSAL_GEN1_SIGNER);
        DogTagIssuer(gen1Clone).revoke(fixtureRoot);

        (address hostileClone, address hostileFactory) = _deployUnguardedLaterGeneration();

        // The plan's array, literally: the later generation at index 0.
        address[] memory planOrder = new address[](2);
        planOrder[0] = hostileFactory; // "factoryV2"
        planOrder[1] = gen1.factory; // "factoryV1"
        CloneProvenanceRouter wrongWayRound = new CloneProvenanceRouter(planOrder, gen1.governance);

        // The correct router names the original clone; the plan's names the attacker's.
        assertEq(d.router.rootIssuer(fixtureRoot), gen1Clone, "correct router");
        assertEq(wrongWayRound.rootIssuer(fixtureRoot), hostileClone, "plan's router");

        // And a registry bound to it verifies a REVOKED credential. Nothing reverts; nothing warns.
        VerificationRegistryConsent resurrecting = new VerificationRegistryConsent(
            address(d.core), gen1.sbt, gen1.verifier, address(wrongWayRound), gen1.governance
        );
        _migrateRelayer();

        assertFalse(DogTagIssuer(gen1Clone).isValid(fixtureRoot), "the credential IS revoked");
        vm.prank(fixtureRelayer);
        resurrecting.recordVerificationZK(pA, pB, pC, pub); // succeeds — that is the bug
        assertTrue(
            resurrecting.consumed(bytes32(pub[3])),
            "newest-first resurrected a revoked credential, silently"
        );
    }

    // =============================================================================================
    // CORRECTION 3 — C-2 cannot attach the existing generation-1 clones
    // =============================================================================================

    /// The plan's C-2 says "register providers, attach the existing clones". `attachService` reads
    /// `owner()` off the service and refuses a zero or unreadable answer; generation-1 `DogTagIssuer`
    /// has no owner at all — its stored identity is `registry`/`rootIndex`/`recordType`/`name`.
    /// So the five live clones cannot be attached, and C-2 must be restated.
    function test_correction_3_c2_cannot_attach_an_ownerless_generation_1_clone() public {
        bytes32 gen1Id = keccak256("dogtag-issuer-factory/1");
        address liveClone = _aLiveGeneration1Clone();
        assertTrue(IFactoryV1(gen1.factory).isClone(liveClone), "precondition: a real live clone");

        vm.startPrank(gen1.governance);
        d.core.addFactoryGeneration(gen1Id, gen1.factory);
        _registerRehearsalProvider();

        vm.expectRevert(abi.encodeWithSignature("InvalidServiceMetadata()"));
        d.core.attachService(REHEARSAL_PROVIDER_ID, liveClone, gen1Id, REHEARSAL_CONTROLLER);
        vm.stopPrank();

        // The cause, stated positively: there is no `owner()` to read.
        (bool ok,) = liveClone.staticcall(abi.encodeWithSignature("owner()"));
        assertFalse(ok, "a generation-1 clone answered owner() - the finding would be stale");
    }

    // =============================================================================================
    // The sequence's own invariants
    // =============================================================================================

    /// The router occupies the registry's immutable `rootIndex` slot, and generation 1's addresses
    /// are inputs that must not move. If any of these drifts the whole cutover is wrong at C-5.
    function test_the_generation_2_registry_is_bound_to_the_router_and_reuses_sbt_and_verifier()
        public
        view
    {
        assertEq(address(d.registryV2.rootIndex()), address(d.router), "rootIndex is not the router");
        assertEq(address(d.registryV2.sbt()), gen1.sbt, "SBT was not reused");
        assertEq(address(d.registryV2.zkVerifier()), gen1.verifier, "verifier was not reused");
        assertEq(address(d.registryV2.issuerRegistry()), address(d.core), "core is not the authority");

        // Generation 1's own registry is untouched and still names the generation-1 factory.
        VerificationRegistryConsent g1 = VerificationRegistryConsent(_gen1RegistryAddress());
        assertEq(address(g1.rootIndex()), gen1.factory, "generation 1's index moved");
    }

    /// The timelock the generation-1 discovery registry could never have, checked on the live
    /// instance rather than asserted: `PUBLISH_TIMELOCK` is immutable and this deployment is the only
    /// opportunity to fix it.
    function test_the_generation_2_discovery_registry_has_a_non_zero_timelock() public view {
        assertGt(d.discoveryV2.PUBLISH_TIMELOCK(), 0, "generation 2 published with a zero timelock");
        assertGe(
            d.discoveryV2.PUBLISH_TIMELOCK(),
            d.discoveryV2.MIN_PUBLISH_TIMELOCK(),
            "below the constructor floor"
        );
    }

    function _gen1RegistryAddress() internal view returns (address) {
        string memory ledger = vm.readFile("deployments/roax.json");
        return vm.parseJsonAddress(ledger, ".VerificationRegistryConsent");
    }
}
