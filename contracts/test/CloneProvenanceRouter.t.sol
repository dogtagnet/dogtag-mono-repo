// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";
import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {CloneProvenanceRouter} from "../src/CloneProvenanceRouter.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagSBTConsent} from "../src/DogTagSBTConsent.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";
import {Groth16VerifierConsent} from "../src/Groth16VerifierConsent.sol";

interface IRouterRootView {
    function isRootAnchored(bytes32 root) external view returns (bool);
}

/// @notice The cross-generation write guard S-7's factory must carry, as a test double.
///
/// It is a faithful copy of `DogTagIssuerFactory` plus ONE line: `registerRoot` refuses a root any
/// earlier generation already holds. It exists as a double rather than as production code because the
/// generation-2 factory is a different slice (S-7) and depends on the provider core (S-6); what S-8
/// owns is the {CloneProvenanceRouter.isRootAnchored} hook the guard is built on, and the proof that
/// wiring it works.
///
/// It is deliberately NOT used by the revocation-bypass test. That test must run against an UNGUARDED
/// later generation - which is exactly today's real `DogTagIssuerFactory` - because the router's
/// ordering has to hold against a generation that is unguarded, buggy or hostile. A guarded factory
/// there would make the attack setup revert and the test would pass while proving nothing.
contract GuardedIssuerFactory {
    address public immutable implementation;
    address public immutable registry;
    IRouterRootView public immutable priorIndex;

    mapping(address => bool) public isClone;
    mapping(bytes32 => address) public rootIssuer;

    event RootRegistered(bytes32 indexed root, address indexed clone);

    constructor(address impl, address registry_, address priorIndex_) {
        implementation = impl;
        registry = registry_;
        priorIndex = IRouterRootView(priorIndex_);
    }

    function createIssuer(string calldata name, bytes32 recordType, address business)
        external
        returns (address clone)
    {
        clone = Clones.cloneDeterministic(implementation, keccak256(abi.encode(recordType, business)));
        isClone[clone] = true;
        DogTagIssuer(clone).initialize(name, recordType, registry, address(this));
    }

    function registerRoot(bytes32 root) external {
        require(isClone[msg.sender], "!clone");
        require(rootIssuer[root] == address(0), "root taken");
        // The whole delta from `DogTagIssuerFactory`. Safe to call while this factory is itself in the
        // router's list: the check above proves our own mapping is still empty for `root`, so the
        // router's answer reflects only the OTHER generations.
        require(!priorIndex.isRootAnchored(root), "root taken by a prior generation");
        rootIssuer[root] = msg.sender;
        emit RootRegistered(root, msg.sender);
    }
}

/// @notice S-8 `clone-provenance-router`.
///
/// The headline is {test_a_revoked_root_cannot_be_resurrected_by_a_later_generation} and its
/// end-to-end twin {test_resurrection_attempt_is_refused_by_the_real_registry}: they perform the
/// actual attack (anchor, revoke, re-anchor on a later generation) against the REAL production
/// factory and assert the credential still reads revoked. Both fail if the resolution loop is
/// reversed to newest-first.
contract CloneProvenanceRouterTest is Test {
    using stdJson for string;

    IssuerRegistry registry;
    DogTagIssuer impl;
    DogTagIssuerFactory factoryV1;
    DogTagIssuerFactory factoryV2; // a SECOND real, unguarded factory - the honest later generation
    CloneProvenanceRouter router;

    address admin = address(0xA11CE);
    address owner = address(0x0117E4);
    address newOwner = address(0x0117E5);
    address stranger = address(0xDEAD);
    address vetSigner = address(0xBEEF);
    /// @dev A signer legitimately whitelisted for VACCINATION on the later generation. It does not
    /// need to be hostile for the attack to work, which is the point: any such signer can re-anchor.
    address genTwoSigner = address(0xCAFE);

    bytes32 constant VACCINATION = keccak256("VACCINATION");
    bytes32 constant ROOT = keccak256("a-credential-root");
    bytes32 constant FRESH_ROOT = keccak256("a-root-no-generation-holds");

    function setUp() public {
        vm.startPrank(admin);
        registry = new IssuerRegistry(admin);
        impl = new DogTagIssuer();
        factoryV1 = new DogTagIssuerFactory(address(impl), address(registry), admin);
        factoryV2 = new DogTagIssuerFactory(address(impl), address(registry), admin);
        registry.whitelistFor(VACCINATION, vetSigner);
        registry.whitelistFor(VACCINATION, genTwoSigner);
        vm.stopPrank();

        address[] memory gens = new address[](2);
        gens[0] = address(factoryV1); // OLDEST first
        gens[1] = address(factoryV2);
        router = new CloneProvenanceRouter(gens, owner);
    }

    function _clone(DogTagIssuerFactory f, address business) internal returns (DogTagIssuer) {
        vm.prank(admin);
        return DogTagIssuer(f.createIssuer("Vacc", VACCINATION, business));
    }

    // =============================================================================================
    // THE HEADLINE: oldest-first is a revocation bypass when reversed
    // =============================================================================================

    /// @notice Anchor on generation 1, REVOKE it there, then re-anchor the same root on generation 2.
    ///
    /// Every step of that attack succeeds on chain, and the test asserts it does before asserting the
    /// router's answer - otherwise a setup that silently failed would make this pass vacuously. The
    /// write-once guards are per-CONTRACT (`DogTagIssuer.issue` reads its own `issuedAt`,
    /// `registerRoot` reads its own `rootIssuer`), so generation 2 has never seen this root and both
    /// let it through.
    ///
    /// Under newest-first the router would return the fresh generation-2 clone, whose `isValid` is
    /// true, and the revoked credential would verify again. Both assertions at the end flip under that
    /// mutation.
    function test_a_revoked_root_cannot_be_resurrected_by_a_later_generation() public {
        DogTagIssuer cloneV1 = _clone(factoryV1, vetSigner);
        vm.prank(vetSigner);
        cloneV1.issue(ROOT);
        vm.prank(vetSigner);
        cloneV1.revoke(ROOT);
        assertFalse(cloneV1.isValid(ROOT), "setup: the generation-1 credential must be revoked");

        // The re-anchor. Assert it really lands, or the attack is not being performed.
        DogTagIssuer cloneV2 = _clone(factoryV2, genTwoSigner);
        vm.prank(genTwoSigner);
        cloneV2.issue(ROOT);
        assertEq(factoryV2.rootIssuer(ROOT), address(cloneV2), "setup: the duplicate must exist");
        assertTrue(cloneV2.isValid(ROOT), "setup: the re-anchored copy must read valid on its own clone");

        // Oldest-first: the root stays bound to the clone that first anchored it, forever.
        assertEq(router.rootIssuer(ROOT), address(cloneV1), "router must resolve the ORIGINAL clone");
        assertFalse(
            DogTagIssuer(router.rootIssuer(ROOT)).isValid(ROOT),
            "a revoked credential must not be resurrected by a later generation"
        );
    }

    /// @notice The same attack driven end to end through the REAL verification registry and a REAL
    /// Groth16 proof, with the router occupying the immutable `rootIndex` slot.
    ///
    /// This is the form that matters operationally: `recordVerificationZK` must revert `cred !valid`.
    /// Under newest-first it would emit `Verified` for a revoked credential.
    function test_resurrection_attempt_is_refused_by_the_real_registry() public {
        RealProofFixture memory f = _wireRealRegistry();

        // Resolve BEFORE pranking: `vm.prank` survives only to the next CALL, and `router.rootIssuer`
        // is itself one - it would consume the prank and leave `revoke` sent by the test contract.
        DogTagIssuer anchored = DogTagIssuer(router.rootIssuer(f.root));
        vm.prank(vetSigner);
        anchored.revoke(f.root);

        DogTagIssuer cloneV2 = _clone(factoryV2, genTwoSigner);
        vm.prank(genTwoSigner);
        cloneV2.issue(f.root);
        assertTrue(cloneV2.isValid(f.root), "setup: the re-anchored copy must read valid on its own clone");

        vm.prank(f.relayer);
        vm.expectRevert("cred !valid");
        f.vr.recordVerificationZK(f.a, f.b, f.c, f.pub);
    }

    // =============================================================================================
    // Substitutable in the immutable slot, not merely same-named
    // =============================================================================================

    /// @notice A generation-1 credential verifies through the REAL registry when its immutable
    /// `rootIndex` is the router rather than the factory. That slot can never be rewritten, so
    /// "implements the same function" is not enough - it has to carry a real proof.
    function test_a_generation_one_credential_verifies_with_the_router_in_the_rootIndex_slot() public {
        RealProofFixture memory f = _wireRealRegistry();

        vm.prank(f.relayer);
        f.vr.recordVerificationZK(f.a, f.b, f.c, f.pub);
        assertTrue(f.vr.consumed(f.nullifier), "the verification must have been recorded");
    }

    /// @notice And a credential anchored ONLY in a later generation verifies through the same router,
    /// which is the other half of what the slot has to do: both generations answer.
    function test_a_generation_two_credential_verifies_through_the_same_router() public {
        RealProofFixture memory f = _wireRealRegistryAnchoredOn(factoryV2, genTwoSigner);

        assertEq(factoryV1.rootIssuer(f.root), address(0), "the older generation must not hold this root");
        vm.prank(f.relayer);
        f.vr.recordVerificationZK(f.a, f.b, f.c, f.pub);
        assertTrue(f.vr.consumed(f.nullifier), "the verification must have been recorded");
    }

    // =============================================================================================
    // Resolution and absence
    // =============================================================================================

    function test_a_root_no_generation_holds_resolves_to_zero_without_reverting() public view {
        assertEq(router.rootIssuer(FRESH_ROOT), address(0));
        assertFalse(router.isRootAnchored(FRESH_ROOT));
    }

    /// @notice A root only the later generation holds falls through the earlier one to it. This is the
    /// case that makes oldest-first a routing rule rather than a freeze on new issuance.
    function test_a_later_generation_root_falls_through_to_it() public {
        DogTagIssuer cloneV2 = _clone(factoryV2, genTwoSigner);
        vm.prank(genTwoSigner);
        cloneV2.issue(ROOT);

        assertEq(factoryV1.rootIssuer(ROOT), address(0), "the older generation must not hold it");
        assertEq(router.rootIssuer(ROOT), address(cloneV2));
        assertTrue(DogTagIssuer(router.rootIssuer(ROOT)).isValid(ROOT));
    }

    function test_isClone_is_the_union_over_generations() public {
        DogTagIssuer cloneV1 = _clone(factoryV1, vetSigner);
        DogTagIssuer cloneV2 = _clone(factoryV2, genTwoSigner);

        assertTrue(router.isClone(address(cloneV1)), "a generation-1 clone");
        assertTrue(router.isClone(address(cloneV2)), "a generation-2 clone");
        assertFalse(router.isClone(stranger), "a contract no generation deployed");
        assertFalse(router.isClone(address(factoryV1)), "a factory is not one of its own clones");
    }

    // =============================================================================================
    // The cross-generation write guard (defence in depth)
    // =============================================================================================

    /// @notice A well-behaved later generation refuses the duplicate outright, so it never exists.
    /// Note this is a SEPARATE property from the ordering: the headline test above proves the router
    /// is safe even when the later generation carries no such guard.
    function test_a_guarded_later_generation_refuses_a_root_an_earlier_one_holds() public {
        DogTagIssuer cloneV1 = _clone(factoryV1, vetSigner);
        vm.prank(vetSigner);
        cloneV1.issue(ROOT);
        vm.prank(vetSigner);
        cloneV1.revoke(ROOT);

        GuardedIssuerFactory guarded = _appendGuardedGeneration();
        DogTagIssuer cloneV3 = DogTagIssuer(guarded.createIssuer("Vacc", VACCINATION, genTwoSigner));

        vm.prank(genTwoSigner);
        vm.expectRevert("root taken by a prior generation");
        cloneV3.issue(ROOT);

        assertEq(guarded.rootIssuer(ROOT), address(0), "the duplicate must never come into existence");
    }

    /// @notice The guard must not become a freeze: a genuinely new root still anchors.
    function test_a_guarded_later_generation_still_anchors_a_new_root() public {
        GuardedIssuerFactory guarded = _appendGuardedGeneration();
        DogTagIssuer cloneV3 = DogTagIssuer(guarded.createIssuer("Vacc", VACCINATION, genTwoSigner));

        vm.prank(genTwoSigner);
        cloneV3.issue(FRESH_ROOT);

        assertEq(guarded.rootIssuer(FRESH_ROOT), address(cloneV3));
        assertEq(router.rootIssuer(FRESH_ROOT), address(cloneV3));
    }

    // =============================================================================================
    // Two-step handover - the captain's key-switch requirement
    // =============================================================================================

    /// @notice A real rotation: the old key can act, a pending transfer strands nothing, and after
    /// acceptance the old key cannot act and the new one can.
    ///
    /// The middle arm is the one that answers the stated failure mode ("a single-step transfer to a
    /// mistyped address strands the role forever"). With `Ownable2Step` a transfer to an address that
    /// never accepts leaves the CURRENT owner fully in place, so the mistake is recoverable by simply
    /// transferring again. A one-step `transferOwnership` would have moved the role at that point and
    /// nothing after it would be possible.
    function test_two_step_handover_rotates_the_key_and_a_pending_transfer_strands_nothing() public {
        DogTagIssuerFactory genThree = _newGeneration();
        DogTagIssuerFactory genFour = _newGeneration();

        // 1. The old key can act.
        vm.prank(owner);
        router.appendGeneration(address(genThree));
        assertEq(router.generationCount(), 3);

        // 2. A transfer to a mistyped address that never accepts. The role does NOT move.
        address mistyped = address(0xBADBAD);
        vm.prank(owner);
        router.transferOwnership(mistyped);
        assertEq(router.owner(), owner, "an unaccepted transfer must not move the role");
        assertEq(router.pendingOwner(), mistyped);

        vm.prank(owner);
        router.appendGeneration(address(genFour)); // still the owner - nothing is stranded
        assertEq(router.generationCount(), 4);

        // 3. Retarget and complete the handover.
        vm.prank(owner);
        router.transferOwnership(newOwner);
        assertEq(router.pendingOwner(), newOwner, "re-targeting must replace the pending owner");

        vm.prank(mistyped);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, mistyped));
        router.acceptOwnership();

        vm.prank(newOwner);
        router.acceptOwnership();
        assertEq(router.owner(), newOwner);
        assertEq(router.pendingOwner(), address(0));

        // 4. The old key can no longer act; the new one can.
        DogTagIssuerFactory genFive = _newGeneration();
        vm.prank(owner);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, owner));
        router.appendGeneration(address(genFive));

        vm.prank(newOwner);
        router.appendGeneration(address(genFive));
        assertEq(router.generationCount(), 5);
        assertEq(router.generationAt(4), address(genFive));
    }

    /// @notice `renounceOwnership` is inherited from `Ownable`, is NOT two-step, and would drop the
    /// role to zero in one transaction with no way back - the same permanent stranding the two-step
    /// pattern was chosen to prevent. It must be closed for everyone, owner included.
    function test_ownership_can_be_handed_over_but_never_dropped() public {
        vm.prank(owner);
        vm.expectRevert(CloneProvenanceRouter.RenounceDisabled.selector);
        router.renounceOwnership();

        vm.prank(stranger);
        vm.expectRevert(CloneProvenanceRouter.RenounceDisabled.selector);
        router.renounceOwnership();

        assertEq(router.owner(), owner, "the owner must survive both attempts");
    }

    // =============================================================================================
    // Append is tail-only and monotone
    // =============================================================================================

    /// @notice The safety argument for allowing appends at all: a new LAST generation is consulted
    /// only after every existing one has answered zero, so it cannot change an answer that already
    /// exists - even when it holds a competing copy of the same root.
    function test_appending_a_generation_cannot_change_an_existing_answer() public {
        DogTagIssuer cloneV1 = _clone(factoryV1, vetSigner);
        vm.prank(vetSigner);
        cloneV1.issue(ROOT);
        address before = router.rootIssuer(ROOT);

        DogTagIssuerFactory genThree = _newGeneration();
        vm.prank(owner);
        router.appendGeneration(address(genThree));

        vm.prank(admin);
        DogTagIssuer cloneV3 = DogTagIssuer(genThree.createIssuer("Vacc", VACCINATION, genTwoSigner));
        vm.prank(genTwoSigner);
        cloneV3.issue(ROOT); // a competing copy in the newest generation

        assertEq(router.rootIssuer(ROOT), before, "an append must not move an existing root");
        assertEq(router.rootIssuer(ROOT), address(cloneV1));
    }

    function test_append_is_owner_only() public {
        DogTagIssuerFactory genThree = _newGeneration();
        vm.prank(stranger);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, stranger));
        router.appendGeneration(address(genThree));
    }

    function test_append_rejects_zero_and_duplicates() public {
        vm.prank(owner);
        vm.expectRevert(CloneProvenanceRouter.ZeroAddress.selector);
        router.appendGeneration(address(0));

        vm.prank(owner);
        vm.expectRevert(CloneProvenanceRouter.DuplicateGeneration.selector);
        router.appendGeneration(address(factoryV1));
    }

    /// @notice An appended address that cannot answer would make {CloneProvenanceRouter.rootIssuer}
    /// revert for EVERY root, and there is no removal and no way to repoint the registry's `rootIndex`
    /// - so a typo here would brick verification protocol-wide, permanently. All three shapes must be
    /// refused before the append lands.
    function test_append_refuses_an_address_that_cannot_answer() public {
        address eoa = address(0xE0A);
        vm.prank(owner);
        vm.expectRevert(abi.encodeWithSelector(CloneProvenanceRouter.GenerationDoesNotAnswer.selector, eoa));
        router.appendGeneration(eoa);

        // A real, live contract of this protocol - just not a factory.
        address notAFactory = address(registry);
        vm.prank(owner);
        vm.expectRevert(
            abi.encodeWithSelector(CloneProvenanceRouter.GenerationDoesNotAnswer.selector, notAFactory)
        );
        router.appendGeneration(notAFactory);

        assertEq(router.generationCount(), 2, "no unanswerable generation may have landed");
        assertEq(router.rootIssuer(ROOT), address(0), "the router must still resolve");
    }

    function test_construction_refuses_an_address_that_cannot_answer() public {
        address[] memory gens = new address[](2);
        gens[0] = address(factoryV1);
        gens[1] = address(registry);
        vm.expectRevert(
            abi.encodeWithSelector(CloneProvenanceRouter.GenerationDoesNotAnswer.selector, address(registry))
        );
        new CloneProvenanceRouter(gens, owner);
    }

    /// @notice The cap is not cosmetic: {CloneProvenanceRouter.rootIssuer} loops with one external
    /// call per generation and runs inside `recordVerificationZK`, a state-changing transaction, so an
    /// unbounded list would let the owner price verification out of reach.
    function test_the_generation_list_is_bounded() public {
        uint256 max = router.MAX_GENERATIONS();
        for (uint256 i = router.generationCount(); i < max; i++) {
            // Deploy BEFORE pranking: `_newGeneration` pranks `admin` itself, and two pending pranks
            // is a harness error, not a contract one.
            address next = address(_newGeneration());
            vm.prank(owner);
            router.appendGeneration(next);
        }
        assertEq(router.generationCount(), max);

        address overflow = address(_newGeneration());
        vm.prank(owner);
        vm.expectRevert(CloneProvenanceRouter.TooManyGenerations.selector);
        router.appendGeneration(overflow);
    }

    /// @notice There is deliberately no remove/replace/insert/reorder. A removal lever is the same
    /// denial of service as reverting on a conflict, aimed at a whole generation: it would strip every
    /// credential that generation anchored of its issuer and leave the registry answering
    /// `unknown root`, with no repair because `rootIndex` cannot be repointed.
    /// @dev A selector is the FIRST four bytes of the signature hash. Do NOT route it through
    /// `address`/`uint160`: that keeps the LOW 160 bits and discards the high 96 where the selector
    /// lives, so `bytes4(bytes20(...))` would scan for hash bytes 12-15 and pass against any bytecode
    /// at all. That is an inert guard reading as a kept promise, in the one test standing between this
    /// contract and a removal lever - the exact defect class the router itself exists to prevent. The
    /// positive control below is what makes the negative result mean something.
    function test_no_mutation_other_than_append_exists() public view {
        string[7] memory forbidden = [
            "removeGeneration(address)",
            "setGeneration(uint256,address)",
            "insertGeneration(uint256,address)",
            "replaceGeneration(address,address)",
            "reorderGenerations(address[])",
            "setGenerations(address[])",
            "deprecateGeneration(address)"
        ];
        bytes memory code = address(router).code;

        assertTrue(
            _containsSelector(code, bytes4(keccak256(bytes("appendGeneration(address)")))),
            "positive control: the scanner cannot even find a selector that IS present"
        );

        for (uint256 i; i < forbidden.length; i++) {
            bytes4 sel = bytes4(keccak256(bytes(forbidden[i])));
            assertFalse(_containsSelector(code, sel), "a non-append mutation reached the bytecode");
        }
    }

    // =============================================================================================
    // Construction
    // =============================================================================================

    /// @notice A router holding no generation answers zero for every root, which the verification
    /// registry reads as `unknown root` - it would brick verification protocol-wide rather than fail
    /// visibly, so it is unrepresentable.
    function test_construction_requires_at_least_one_generation() public {
        address[] memory none = new address[](0);
        vm.expectRevert(CloneProvenanceRouter.EmptyGenerationList.selector);
        new CloneProvenanceRouter(none, owner);
    }

    function test_construction_rejects_zero_and_duplicate_generations() public {
        address[] memory withZero = new address[](2);
        withZero[0] = address(factoryV1);
        withZero[1] = address(0);
        vm.expectRevert(CloneProvenanceRouter.ZeroAddress.selector);
        new CloneProvenanceRouter(withZero, owner);

        address[] memory dup = new address[](2);
        dup[0] = address(factoryV1);
        dup[1] = address(factoryV1);
        vm.expectRevert(CloneProvenanceRouter.DuplicateGeneration.selector);
        new CloneProvenanceRouter(dup, owner);
    }

    function test_construction_preserves_the_declared_order() public view {
        address[] memory gens = router.generations();
        assertEq(gens.length, 2);
        assertEq(gens[0], address(factoryV1), "index 0 must be the OLDEST generation");
        assertEq(gens[1], address(factoryV2));
        assertEq(router.generationAt(0), address(factoryV1));
        assertTrue(router.isGeneration(address(factoryV1)));
        assertFalse(router.isGeneration(stranger));
    }

    function test_generationAt_rejects_an_out_of_range_index() public {
        vm.expectRevert(CloneProvenanceRouter.IndexOutOfRange.selector);
        router.generationAt(2);
    }

    // =============================================================================================
    // helpers
    // =============================================================================================

    function _newGeneration() internal returns (DogTagIssuerFactory) {
        vm.prank(admin);
        return new DogTagIssuerFactory(address(impl), address(registry), admin);
    }

    function _appendGuardedGeneration() internal returns (GuardedIssuerFactory guarded) {
        guarded = new GuardedIssuerFactory(address(impl), address(registry), address(router));
        vm.prank(owner);
        router.appendGeneration(address(guarded));
    }

    function _containsSelector(bytes memory code, bytes4 sel) internal pure returns (bool) {
        for (uint256 i; i + 4 <= code.length; i++) {
            if (code[i] == sel[0] && code[i + 1] == sel[1] && code[i + 2] == sel[2] && code[i + 3] == sel[3])
            {
                return true;
            }
        }
        return false;
    }

    struct RealProofFixture {
        VerificationRegistryConsent vr;
        uint256[2] a;
        uint256[2][2] b;
        uint256[2] c;
        uint256[7] pub;
        bytes32 root;
        bytes32 nullifier;
        address relayer;
    }

    function _wireRealRegistry() internal returns (RealProofFixture memory) {
        return _wireRealRegistryAnchoredOn(factoryV1, vetSigner);
    }

    /// @dev Stands up the full owner-hidden stack with the ROUTER in the immutable `rootIndex` slot,
    /// and anchors the fixture's root on `f`'s generation. Mirrors `ConsentRegistry.t.sol`'s wiring;
    /// the only difference that matters is the fourth constructor argument.
    function _wireRealRegistryAnchoredOn(DogTagIssuerFactory f, address signer)
        internal
        returns (RealProofFixture memory fx)
    {
        string memory j = vm.readFile("test/consent-fixture.json");
        uint256[] memory av = j.readUintArray(".a");
        uint256[] memory b0 = j.readUintArray(".b[0]");
        uint256[] memory b1 = j.readUintArray(".b[1]");
        uint256[] memory cv = j.readUintArray(".c");
        uint256[] memory pv = j.readUintArray(".pub");
        fx.a = [av[0], av[1]];
        fx.b = [[b0[0], b0[1]], [b1[0], b1[1]]];
        fx.c = [cv[0], cv[1]];
        for (uint256 i; i < 7; i++) {
            fx.pub[i] = pv[i];
        }
        fx.root = bytes32(fx.pub[4]);
        fx.nullifier = bytes32(fx.pub[3]);
        fx.relayer = address(uint160(fx.pub[2]));

        address custodian = address(0xC0FFEE);
        vm.startPrank(admin);
        DogTagSBTConsent sbt = new DogTagSBTConsent(admin, custodian);
        Groth16VerifierConsent verifier = new Groth16VerifierConsent();
        fx.vr = new VerificationRegistryConsent(
            address(registry), address(sbt), address(verifier), address(router), admin
        );
        vm.stopPrank();

        DogTagIssuer c = _clone(f, signer);
        vm.prank(signer);
        c.issue(fx.root);

        bytes32 issuerRole = sbt.ISSUER_ROLE();
        vm.prank(admin);
        sbt.grantRole(issuerRole, address(this));
        sbt.mintCustodial(fx.pub[0], fx.root);

        vm.prank(admin);
        registry.whitelistFor(keccak256(abi.encode("VERIFY:", bytes32(fx.pub[1]))), fx.relayer);
    }
}
