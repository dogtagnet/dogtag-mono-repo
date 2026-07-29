// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Errors} from "@openzeppelin/contracts/utils/Errors.sol";
import {Initializable} from "@openzeppelin/contracts/proxy/utils/Initializable.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagIssuerV2} from "../src/DogTagIssuerV2.sol";
import {DogTagIssuerFactoryV2} from "../src/DogTagIssuerFactoryV2.sol";

/// @dev A hand-rolled contract that lies perfectly: it answers `owner()` and `recordType()` with exactly
/// the values a repoint would need. It is the whole point of the provenance check that this is not
/// enough — the factory never deployed it, so it can never be pointed at.
contract ImpostorIssuer {
    address public owner;
    bytes32 public recordType;

    constructor(address owner_, bytes32 recordType_) {
        owner = owner_;
        recordType = recordType_;
    }
}

/// @dev A non-clone whose getters revert loudly. If the factory ever called into it, the revert reason
/// would be this contract's, not the factory's — which is how `test_a_hostile_impostor_is_never_even_called`
/// proves the `isClone` storage read short-circuits before any external call.
contract HostileIssuer {
    error HostileGetterExecuted();

    function owner() external pure returns (address) {
        revert HostileGetterExecuted();
    }

    function recordType() external pure returns (bytes32) {
        revert HostileGetterExecuted();
    }
}

/// @notice Slice S-7 acceptance: the owner-bearing issuer clone and the self-service factory.
///
/// Three properties this suite exists to hold, each corresponding to something that is impossible today:
///
///   1. **A clone has a real, checkable, transferable owner** — generation-1 clones have none, so the
///      captain's "whitelisted people AND owner of contracts" cannot be enforced at all.
///   2. **Key rotation is two-step and actually works** — exercised as a real handover, proving the old
///      key can no longer act and the new one can. A rotation path that has never been run is a check
///      that never ran.
///   3. **A repoint resolves the address rather than accepting a claim about it** — a hand-rolled
///      contract cannot be pointed at even when it lies perfectly, and neither can another provider's
///      genuine clone.
///
/// Plus the property that must NOT regress: `rootIssuer` write-once, per contract and honest.
contract IssuerV2Test is Test {
    IssuerRegistry registry;
    DogTagIssuerV2 impl;
    DogTagIssuerFactoryV2 factory;

    address admin = address(0xA11CE);
    /// @dev An approved issuance signer, cleared for both record types.
    address providerA = address(0xA0A0);
    /// @dev A second approved provider — the misattribution target.
    address providerB = address(0xB0B0);
    /// @dev Cleared for TRAVEL only.
    address providerC = address(0xC0C0);
    /// @dev The rotation target: a controller key that is deliberately NOT an issuance signer. See
    /// `test_key_rotation_hands_control_from_the_issuance_signer_to_a_controller`.
    address controller = address(0xC717);
    address stranger = address(0x5721);
    /// @dev Where a fat-fingered handover goes. Nothing here ever accepts.
    address mistyped = address(0xDEAD);

    bytes32 constant VACCINATION = keccak256("VACCINATION");
    bytes32 constant TRAVEL = keccak256("TRAVEL_CLEARANCE");

    bytes32 constant ROOT_1 = bytes32(uint256(0x1111));
    bytes32 constant ROOT_2 = bytes32(uint256(0x2222));

    function setUp() public {
        registry = new IssuerRegistry(admin);
        impl = new DogTagIssuerV2();
        // priorIndex = 0: this is the first generation under test unless a case builds its own.
        factory = new DogTagIssuerFactoryV2(address(impl), address(registry), address(0));

        vm.startPrank(admin);
        registry.whitelistFor(VACCINATION, providerA);
        registry.whitelistFor(TRAVEL, providerA);
        registry.whitelistFor(VACCINATION, providerB);
        registry.whitelistFor(TRAVEL, providerC);
        vm.stopPrank();
    }

    function _create(address who, bytes32 rt, uint96 nonce) internal returns (DogTagIssuerV2) {
        vm.prank(who);
        return DogTagIssuerV2(factory.createIssuer("Clone", rt, nonce));
    }

    // =============================================================================================
    // Creation: self-service, gated per record type, owner is the creator
    // =============================================================================================

    /// @notice The generation-1 blocker: `createIssuer` was `onlyOwner`, so no provider could deploy
    /// anything. Here an approved provider deploys its own, and owns it.
    function test_an_approved_provider_deploys_its_own_clone_and_owns_it() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);

        assertTrue(factory.isClone(address(clone)), "not recorded as a clone");
        assertEq(clone.owner(), providerA, "creator is not the owner");
        assertEq(clone.recordType(), VACCINATION);
        assertEq(address(clone.registry()), address(registry));
        assertEq(address(clone.rootIndex()), address(factory));
        // No handover is in flight on a fresh clone.
        assertEq(clone.pendingOwner(), address(0));
    }

    function test_an_unapproved_caller_cannot_create() public {
        vm.prank(stranger);
        vm.expectRevert(DogTagIssuerFactoryV2.NotApproved.selector);
        factory.createIssuer("Clone", VACCINATION, 0);
    }

    /// @notice Approval is per record type, which is what keeps the widened `isClone` set sound.
    function test_approval_is_per_record_type() public {
        vm.prank(providerC); // cleared for TRAVEL only
        vm.expectRevert(DogTagIssuerFactoryV2.NotApproved.selector);
        factory.createIssuer("Clone", VACCINATION, 0);
    }

    /// @notice Self-service widens who may add to `isClone` — under generation 1 only the protocol owner
    /// could. The mandatory issuer-whitelist pillar keys its question on `clone.recordType()`, so a
    /// provider cleared for one record type cannot produce a contract that reads as another.
    function test_a_widened_clone_set_cannot_forge_a_record_type() public {
        vm.prank(providerC);
        DogTagIssuerV2 clone = DogTagIssuerV2(factory.createIssuer("Travel", TRAVEL, 0));

        // The pillar asks `isWhitelistedFor(clone.recordType(), issuedBy[R])`. That key is TRAVEL,
        // resolved from the clone, and providerC is genuinely cleared for it.
        assertEq(clone.recordType(), TRAVEL);
        assertTrue(registry.isWhitelistedFor(clone.recordType(), providerC));
        // And there is no VACCINATION contract providerC could have made — the gate refused above.
        assertFalse(registry.isWhitelistedFor(VACCINATION, providerC));
    }

    /// @notice `predictIssuer` is exact before deployment — the property the generation-1 salt bought and
    /// that the nonce must not cost.
    function test_predict_matches_the_deployment_exactly() public {
        address predicted = factory.predictIssuer(VACCINATION, providerA, 7);
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 7);
        assertEq(address(clone), predicted, "prediction diverged from deployment");
    }

    /// @notice THE reason the salt gained a nonce: without one a provider has exactly one possible clone
    /// address per record type and the repoint in requirement 1 has no target to move to.
    function test_the_nonce_gives_a_provider_a_second_address() public {
        DogTagIssuerV2 first = _create(providerA, VACCINATION, 0);
        DogTagIssuerV2 second = _create(providerA, VACCINATION, 1);

        assertTrue(address(first) != address(second), "nonce did not produce a distinct address");
        assertTrue(factory.isClone(address(first)) && factory.isClone(address(second)));
        assertEq(first.owner(), providerA);
        assertEq(second.owner(), providerA);
    }

    /// @notice Reusing a nonce still reverts (OZ `Clones` refuses a repeated salt). Recorded so the
    /// nonce's purpose is not mistaken for "creation became unbounded".
    function test_reusing_a_nonce_still_reverts() public {
        _create(providerA, VACCINATION, 0);
        vm.prank(providerA);
        vm.expectRevert(Errors.FailedDeployment.selector);
        factory.createIssuer("Clone", VACCINATION, 0);
    }

    /// @notice There is exactly one creation path and it takes no owner argument, so `owner()` and the
    /// salt binding can never disagree — the generation-1 weakness where `business` defaulted to the
    /// operator's own signer is structurally unreachable.
    function test_owner_and_the_salt_binding_can_never_disagree() public {
        DogTagIssuerV2 clone = _create(providerB, VACCINATION, 3);
        assertEq(clone.owner(), providerB);
        assertEq(factory.predictIssuer(VACCINATION, providerB, 3), address(clone));
        // No other address predicts to it.
        assertTrue(factory.predictIssuer(VACCINATION, providerA, 3) != address(clone));
    }

    // =============================================================================================
    // Key rotation — a real handover, run end to end
    // =============================================================================================

    /// @notice The rotation the captain asked for ("its okay keep it as one key for now. but allow key
    /// switch"), exercised as the migration it actually is.
    ///
    /// Creation is gated on `isWhitelistedFor`, which is the *issuance-signing* capability — and the plan
    /// is explicit that a provider's controller is "**not** the issuance signer" (§4 item 2). So a freshly
    /// created clone is owned by a signing key, and the correct next step is to hand it to the
    /// organisation's controller. That conflation is transient and self-correcting precisely because this
    /// handover exists; the controller here is deliberately whitelisted for nothing.
    ///
    /// The assertions that matter: **nothing moves until the recipient accepts**, and after acceptance the
    /// **old key can no longer act** while the new one can — tested through `authorizeClone`, the predicate
    /// S-6 and S-9 will call, not only through this factory's own pointer.
    function test_key_rotation_hands_control_from_the_issuance_signer_to_a_controller() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);

        // Before: the signer holds control; the controller holds none.
        assertEq(factory.authorizeClone(address(clone), providerA), VACCINATION);
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.authorizeClone(address(clone), controller);

        // Step 1 of 2. This alone changes nothing.
        vm.prank(providerA);
        clone.transferOwnership(controller);

        assertEq(clone.owner(), providerA, "ownership moved before acceptance");
        assertEq(clone.pendingOwner(), controller);
        assertEq(factory.authorizeClone(address(clone), providerA), VACCINATION, "old key lost control early");
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.authorizeClone(address(clone), controller);
        // And the pending owner cannot act through the factory either.
        vm.prank(controller);
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.setActiveIssuer(address(clone));

        // Step 2 of 2.
        vm.prank(controller);
        clone.acceptOwnership();

        // After: the handover is complete and total.
        assertEq(clone.owner(), controller, "ownership did not move");
        assertEq(clone.pendingOwner(), address(0), "a pending owner survived acceptance");

        // The new key can act.
        assertEq(factory.authorizeClone(address(clone), controller), VACCINATION);
        vm.prank(controller);
        factory.setActiveIssuer(address(clone));
        assertEq(factory.resolveActiveIssuer(controller, VACCINATION), address(clone));

        // The old key cannot — not through the predicate, not through the pointer, not over the clone.
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.authorizeClone(address(clone), providerA);
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.setActiveIssuer(address(clone));
        vm.prank(providerA);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, providerA));
        clone.transferOwnership(stranger);
    }

    /// @notice A one-step transfer to a mistyped address would strand the clone forever. Two-step means
    /// the mistake is recoverable: the pending owner never accepted, and `transferOwnership(address(0))`
    /// cancels. This is both the recovery proof and the proof that the cancel path cannot be used to
    /// orphan the clone.
    function test_a_handover_to_a_mistyped_address_is_recoverable() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);

        vm.prank(providerA);
        clone.transferOwnership(mistyped);
        assertEq(clone.owner(), providerA, "a mistyped handover took effect immediately");
        assertEq(clone.pendingOwner(), mistyped);

        // Cancel: allowed for exactly this, and it zeroes the PENDING owner, never the owner.
        vm.prank(providerA);
        clone.transferOwnership(address(0));
        assertEq(clone.pendingOwner(), address(0));
        assertEq(clone.owner(), providerA, "cancelling a handover orphaned the clone");

        // The mistyped address can no longer complete the transfer it never accepted.
        vm.prank(mistyped);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, mistyped));
        clone.acceptOwnership();

        // And the real handover still works afterwards.
        vm.prank(providerA);
        clone.transferOwnership(controller);
        vm.prank(controller);
        clone.acceptOwnership();
        assertEq(clone.owner(), controller);
    }

    /// @notice After `initialize`, `owner()` is non-zero forever. Both paths that could vacate it are
    /// closed in the contract rather than left to the fact that nobody can sign as the zero address.
    function test_owner_can_never_become_the_zero_address() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);

        // Path 1: renounce is disabled outright.
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.OwnerCannotBeZero.selector);
        clone.renounceOwnership();

        // Path 2: with no transfer pending, OZ's `pendingOwner() != msg.sender` comparison passes for the
        // zero address (0 == 0) and would hand ownership to it. The override refuses.
        assertEq(clone.pendingOwner(), address(0));
        vm.prank(address(0));
        vm.expectRevert(DogTagIssuerV2.OwnerCannotBeZero.selector);
        clone.acceptOwnership();

        assertEq(clone.owner(), providerA, "owner was vacated");
    }

    function test_only_the_owner_may_start_a_handover() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);
        vm.prank(stranger);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, stranger));
        clone.transferOwnership(stranger);
    }

    /// @notice Ownership is CONTROL, not an issuance capability. Delisting must still stop the next
    /// `issue` and touch nothing already anchored (plan §3.3), and it must not strip control — a
    /// delisted provider can still hand its clone over.
    function test_delisting_stops_issuance_without_stripping_ownership() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);

        vm.prank(providerA);
        clone.issue(ROOT_1);
        assertTrue(clone.isValid(ROOT_1));

        vm.prank(admin);
        registry.delistFor(VACCINATION, providerA);

        // New issuance stops.
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.NotWhitelisted.selector);
        clone.issue(ROOT_2);
        // What was already anchored is untouched.
        assertTrue(clone.isValid(ROOT_1), "delisting invalidated an anchored root");

        // Control survives: the delisted provider still owns it and can still hand it over.
        assertEq(clone.owner(), providerA);
        vm.prank(providerA);
        clone.transferOwnership(controller);
        vm.prank(controller);
        clone.acceptOwnership();
        assertEq(clone.owner(), controller);
    }

    /// @notice The owner is not an issuance signer by virtue of owning. Stated as a test because merging
    /// the two would silently disarm the delist lever above.
    function test_owning_a_clone_confers_no_issuance_right() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);
        vm.prank(providerA);
        clone.transferOwnership(controller);
        vm.prank(controller);
        clone.acceptOwnership();

        vm.prank(controller); // owns it, whitelisted for nothing
        vm.expectRevert(DogTagIssuerV2.NotWhitelisted.selector);
        clone.issue(ROOT_1);
    }

    // =============================================================================================
    // The repoint: resolved, never accepted
    // =============================================================================================

    /// @notice A hand-rolled contract that answers `owner()` and `recordType()` with exactly the right
    /// values is still refused, because provenance is read from the factory's OWN storage and that
    /// mapping is written only by `createIssuer`.
    function test_a_repoint_cannot_accept_an_address_the_factory_did_not_produce() public {
        ImpostorIssuer fake = new ImpostorIssuer(providerA, VACCINATION);

        // The lie is perfect on its face.
        assertEq(fake.owner(), providerA);
        assertEq(fake.recordType(), VACCINATION);
        // And it buys nothing.
        assertFalse(factory.isClone(address(fake)));
        (bool ok,) = factory.cloneAuthorization(address(fake), providerA);
        assertFalse(ok, "an impostor passed the predicate");

        vm.expectRevert(DogTagIssuerFactoryV2.NotAClone.selector);
        factory.authorizeClone(address(fake), providerA);

        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerFactoryV2.NotAClone.selector);
        factory.setActiveIssuer(address(fake));

        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(0));
    }

    /// @notice The provenance check is a read of the factory's own storage and it runs FIRST, so a
    /// non-clone never executes at all. If the order were reversed this reverts with the hostile
    /// contract's error instead of the factory's.
    function test_a_hostile_impostor_is_never_even_called() public {
        HostileIssuer hostile = new HostileIssuer();

        vm.expectRevert(DogTagIssuerFactoryV2.NotAClone.selector);
        factory.authorizeClone(address(hostile), providerA);

        (bool ok, bytes32 rt) = factory.cloneAuthorization(address(hostile), providerA);
        assertFalse(ok);
        assertEq(rt, bytes32(0));

        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerFactoryV2.NotAClone.selector);
        factory.setActiveIssuer(address(hostile));
    }

    /// @notice Provenance alone is not enough. Provider B's clone is entirely genuine; pointing A's
    /// listing at it would be misattribution rather than forgery, and the control check is what catches
    /// it.
    function test_a_repoint_cannot_take_another_providers_genuine_clone() public {
        DogTagIssuerV2 bClone = _create(providerB, VACCINATION, 0);
        assertTrue(factory.isClone(address(bClone)), "the target really is a genuine clone");

        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.authorizeClone(address(bClone), providerA);

        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.setActiveIssuer(address(bClone));

        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(0));
    }

    /// @notice The storage key is READ off the clone, never supplied. `setActiveIssuer` takes exactly one
    /// argument — the target address — so there is no argument by which a caller could name a slot: a
    /// VACCINATION clone can only ever occupy the VACCINATION slot.
    function test_the_record_type_key_is_resolved_from_the_clone_not_supplied() public {
        DogTagIssuerV2 vacc = _create(providerA, VACCINATION, 0);
        DogTagIssuerV2 travel = _create(providerA, TRAVEL, 0);

        vm.prank(providerA);
        factory.setActiveIssuer(address(vacc));
        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(vacc));
        assertEq(
            factory.resolveActiveIssuer(providerA, TRAVEL), address(0), "a clone occupied a foreign slot"
        );

        vm.prank(providerA);
        factory.setActiveIssuer(address(travel));
        assertEq(factory.resolveActiveIssuer(providerA, TRAVEL), address(travel));
        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(vacc), "the other slot moved");

        // The predicate itself returns each clone's own key.
        assertEq(factory.authorizeClone(address(vacc), providerA), VACCINATION);
        assertEq(factory.authorizeClone(address(travel), providerA), TRAVEL);
    }

    /// @notice The self-service repoint the captain described: move to a second clone of your own. What
    /// the old clone already anchored stays anchored to the old clone — `rootIssuer[R]` is write-once, and
    /// retroactively re-attributing issued credentials to a contract that did not issue them is exactly
    /// the misattribution the control check exists to prevent.
    function test_a_provider_repoints_to_a_second_clone_of_its_own() public {
        DogTagIssuerV2 oldClone = _create(providerA, VACCINATION, 0);
        vm.startPrank(providerA);
        factory.setActiveIssuer(address(oldClone));
        oldClone.issue(ROOT_1);
        vm.stopPrank();
        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(oldClone));

        DogTagIssuerV2 newClone = _create(providerA, VACCINATION, 1);
        vm.prank(providerA);
        factory.setActiveIssuer(address(newClone));

        assertEq(
            factory.resolveActiveIssuer(providerA, VACCINATION), address(newClone), "repoint did not take"
        );
        // History is untouched and still revocable where it was issued.
        assertEq(factory.rootIssuer(ROOT_1), address(oldClone), "an issued root followed the repoint");
        assertTrue(oldClone.isValid(ROOT_1));
        vm.prank(providerA);
        oldClone.revoke(ROOT_1);
        assertFalse(oldClone.isValid(ROOT_1));
    }

    /// @notice A pointer left behind by a handover degrades to "nothing recorded" rather than to a claim
    /// that is no longer true. The raw mapping keeps the history; the resolving read refuses it.
    function test_a_stale_pointer_degrades_to_absent_rather_than_to_a_false_claim() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);
        vm.prank(providerA);
        factory.setActiveIssuer(address(clone));
        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(clone));

        vm.prank(providerA);
        clone.transferOwnership(controller);
        vm.prank(controller);
        clone.acceptOwnership();

        assertEq(factory.activeIssuer(providerA, VACCINATION), address(clone), "raw history was rewritten");
        assertEq(
            factory.resolveActiveIssuer(providerA, VACCINATION), address(0), "a stale pointer still resolved"
        );
        // Nor is the pointer inherited: a repoint is an explicit act by the new owner.
        assertEq(factory.resolveActiveIssuer(controller, VACCINATION), address(0));
    }

    function test_a_provider_can_withdraw_its_own_pointer() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);
        vm.startPrank(providerA);
        factory.setActiveIssuer(address(clone));
        factory.clearActiveIssuer(VACCINATION);
        vm.stopPrank();

        assertEq(factory.activeIssuer(providerA, VACCINATION), address(0));
        assertEq(factory.resolveActiveIssuer(providerA, VACCINATION), address(0));
    }

    function test_the_zero_address_never_authorizes() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);
        (bool ok,) = factory.cloneAuthorization(address(clone), address(0));
        assertFalse(ok);
        vm.expectRevert(DogTagIssuerFactoryV2.NotCloneOwner.selector);
        factory.authorizeClone(address(clone), address(0));
    }

    // =============================================================================================
    // The property that must not regress: write-once, per contract, honest
    // =============================================================================================

    function test_register_root_stays_clone_only_and_write_once() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);

        // Only a clone may register.
        vm.prank(stranger);
        vm.expectRevert(bytes("!clone"));
        factory.registerRoot(ROOT_1);

        vm.prank(providerA);
        clone.issue(ROOT_1);
        assertEq(factory.rootIssuer(ROOT_1), address(clone));

        // Write-once in the factory: a second clone cannot claim the same root.
        DogTagIssuerV2 other = _create(providerB, VACCINATION, 0);
        vm.prank(providerB);
        vm.expectRevert(bytes("root taken"));
        other.issue(ROOT_1);

        // Write-once in the clone: even its own issuer cannot re-issue.
        vm.prank(providerA);
        vm.expectRevert(DogTagIssuerV2.BadRoot.selector);
        clone.issue(ROOT_1);
    }

    /// @notice The write-side half of the provenance router's resolution order (S-8). The guards are per
    /// contract, so a root anchored and then REVOKED on a generation-1 clone would otherwise be
    /// re-anchorable on a generation-2 clone — and under newest-first resolution the revoked credential
    /// would verify again. With `priorIndex` wired, the duplicate never comes into existence.
    function test_a_revoked_prior_generation_root_cannot_be_re_anchored() public {
        // A real generation-1 stack.
        DogTagIssuer gen1Impl = new DogTagIssuer();
        DogTagIssuerFactory gen1 = new DogTagIssuerFactory(address(gen1Impl), address(registry), admin);
        vm.prank(admin);
        DogTagIssuer gen1Clone = DogTagIssuer(gen1.createIssuer("Gen1", VACCINATION, providerA));

        vm.startPrank(providerA);
        gen1Clone.issue(ROOT_1);
        gen1Clone.revoke(ROOT_1); // revoked — the credential must stay dead
        vm.stopPrank();
        assertFalse(gen1Clone.isValid(ROOT_1));
        assertEq(gen1.rootIssuer(ROOT_1), address(gen1Clone));

        // Generation 2, wired to the generation-1 index.
        DogTagIssuerFactoryV2 gen2 =
            new DogTagIssuerFactoryV2(address(impl), address(registry), address(gen1));
        vm.prank(providerA);
        DogTagIssuerV2 gen2Clone = DogTagIssuerV2(gen2.createIssuer("Gen2", VACCINATION, 0));

        vm.prank(providerA);
        vm.expectRevert(bytes("root taken upstream"));
        gen2Clone.issue(ROOT_1);

        // The resurrection never happened, and the generation-2 clone is otherwise fully functional.
        assertEq(gen2.rootIssuer(ROOT_1), address(0));
        vm.prank(providerA);
        gen2Clone.issue(ROOT_2);
        assertEq(gen2.rootIssuer(ROOT_2), address(gen2Clone));
    }

    /// @notice The upstream guard is what refuses the duplicate — not something incidental. With
    /// `priorIndex` unset (the first generation) the same root registers normally.
    function test_an_unset_prior_index_disables_the_upstream_check() public {
        DogTagIssuer gen1Impl = new DogTagIssuer();
        DogTagIssuerFactory gen1 = new DogTagIssuerFactory(address(gen1Impl), address(registry), admin);
        vm.prank(admin);
        DogTagIssuer gen1Clone = DogTagIssuer(gen1.createIssuer("Gen1", VACCINATION, providerA));
        vm.prank(providerA);
        gen1Clone.issue(ROOT_1);

        // `factory` was constructed with priorIndex == 0 and knows nothing of generation 1.
        assertEq(address(factory.priorIndex()), address(0));
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);
        vm.prank(providerA);
        clone.issue(ROOT_1);
        assertEq(factory.rootIssuer(ROOT_1), address(clone));
    }

    /// @notice The factory holds no owner and no privileged function, so there is nothing about it to
    /// capture or repoint — the property `IssuerDomainRegistry` asks of a factory reference.
    ///
    /// @dev Read this assertion precisely. A failing `staticcall` proves the selector is **not
    /// reachable** — no such function and no fallback — which is the property that matters here. It
    /// would ALSO pass if such a function existed and reverted, so it is not evidence that the source
    /// declares none; that part is held by review of the source, which declares no owner and no
    /// privileged function at all. Stated rather than left implicit, because "the call failed" and "the
    /// function is absent" are different facts and only the first is what a staticcall establishes.
    function test_the_factory_has_no_admin_surface() public view {
        (bool found,) = address(factory).staticcall(abi.encodeWithSignature("owner()"));
        assertFalse(found, "the factory answers owner()");
        (found,) = address(factory).staticcall(abi.encodeWithSignature("transferOwnership(address)"));
        assertFalse(found, "the factory answers transferOwnership()");
    }

    /// @notice The implementation is locked, so it can never be initialized and adopted directly.
    function test_the_implementation_cannot_be_initialized() public {
        vm.expectRevert(Initializable.InvalidInitialization.selector);
        impl.initialize("x", VACCINATION, address(registry), address(factory), providerA);
    }

    function test_a_clone_cannot_be_re_initialized() public {
        DogTagIssuerV2 clone = _create(providerA, VACCINATION, 0);
        vm.prank(stranger);
        vm.expectRevert(Initializable.InvalidInitialization.selector);
        clone.initialize("hijack", TRAVEL, address(registry), address(factory), stranger);
    }
}
