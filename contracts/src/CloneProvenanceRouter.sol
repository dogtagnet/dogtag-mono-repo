// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Ownable2Step, Ownable} from "@openzeppelin/contracts/access/Ownable2Step.sol";

/// @dev The two reads a `DogTagIssuerFactory` generation answers, and the whole surface this router
/// needs from one. Both are plain public getters on the deployed factory
/// (`DogTagIssuerFactory.sol:18-19`), so a generation needs no new code to be routable.
interface IIssuerFactoryGeneration {
    function rootIssuer(bytes32 root) external view returns (address);
    function isClone(address candidate) external view returns (bool);
}

/// @title CloneProvenanceRouter - one stable root->clone index across factory generations.
///
/// @notice `VerificationRegistryConsent` resolves the issuing clone for every proof through
/// `rootIndex.rootIssuer(R)`, and its `rootIndex` is `immutable`
/// (`VerificationRegistryConsent.sol:88`, read at `:186`). A root can only ever be written into a
/// factory's index by a clone of that same factory (`DogTagIssuerFactory.registerRoot` requires
/// `isClone[msg.sender]`, `:51`). So a new verification registry pointed straight at a new factory
/// cannot see a single root any earlier clone anchored: every existing credential answers
/// `unknown root` and stops verifying, permanently, and it cannot be repaired afterwards because the
/// pointer cannot be rewritten and the new factory refuses any caller that is not one of its own
/// clones.
///
/// This contract occupies that immutable slot instead of a specific factory, and answers for every
/// generation it holds. It implements exactly the read shape the registry consumes
/// (`rootIssuer(bytes32) view returns (address)`, `VerificationRegistryConsent.sol:34-36`) plus
/// `isClone(address)`, which is the other factory read the apps and the web verifier make directly
/// (`packages/ui/src/wallet/contracts.ts`, `RoaxRpc.kt` `rootIssuer`/`isClone`). Pointing those
/// consumers at the router rather than at one factory is what makes a single app build work across
/// generations.
///
/// It deliberately does NOT implement `registerRoot`. Writes stay on each generation's own factory,
/// where the `isClone[msg.sender]` gate lives; the router is read-only over them.
///
/// # RESOLUTION ORDER IS OLDEST FIRST, AND THIS IS THE SECURITY PROPERTY
///
/// Newest-first is the natural way to write this loop and it is a revocation bypass.
///
/// The write-once guards are per-CONTRACT, not protocol-global. `DogTagIssuer.issue` checks
/// `issuedAt[r]` in its own storage (`DogTagIssuer.sol:53`) and `registerRoot` checks
/// `rootIssuer[root]` in its own factory's storage (`DogTagIssuerFactory.sol:52`). So a root anchored
/// and then REVOKED on a generation-1 clone can be re-anchored on a generation-2 clone by any signer
/// whitelisted for that clone's record type: both guards pass, because neither generation-2 contract
/// has ever seen that root. The tag binding does not stop it either, because the SBT is shared across
/// generations, so `R == profileRoot(dogTagId)` still holds.
///
/// Under newest-first the router would then return the fresh clone, `isValid(R)` would read true, and
/// the revoked credential would verify again - resurrected by a provider other than the one whose
/// credential it is.
///
/// Oldest-first closes it. A root resolves to the clone in the EARLIEST GENERATION that holds it, so
/// the revocation recorded there keeps answering. A root only a later generation holds is simply
/// absent from every earlier mapping and falls through to it.
///
/// Read that as earliest-GENERATION-wins, never as first-anchor-wins. The two coincide only for a
/// root whose first anchor is in the EARLIEST generation; the residual section below owns the
/// direction where they come apart.
///
/// This removes no capability that exists today: cross-generation re-anchoring becomes inert, which
/// is the same thing `issuedAt[r] != 0` and `rootIssuer[root] == address(0)` already enforce WITHIN a
/// generation. For a root whose first anchor is in the EARLIEST generation, oldest-first makes that
/// write-once behaviour protocol-global instead of per-contract. For a root first anchored in a later
/// generation it does not, and the residual section below says why and what closes it instead.
///
/// # WHAT OLDEST-FIRST DOES NOT CLOSE: THE MIRROR DIRECTION
///
/// Oldest-first closes resurrection by a LATER generation. It does NOT close the mirror.
///
/// A root first anchored on a later generation can be anchored AFTERWARDS on an EARLIER generation's
/// clone by any signer still whitelisted for that clone's record type on the earlier registry.
/// `DogTagIssuer.issue` gates only on that registry's `isWhitelistedFor` and on its own `issuedAt[r]`
/// (`DogTagIssuer.sol:52-53`), and `registerRoot` only on its own `rootIssuer[root]`
/// (`DogTagIssuerFactory.sol:51-52`); neither earlier contract has ever seen that root, so both let it
/// through. Oldest-first then resolves that earlier clone, and the LATER generation's revocation stops
/// being consulted - the same harm as the resurrection above, arriving from the other side.
///
/// The write guard below structurally cannot reach this. {isRootAnchored} can only ever be wired into
/// a NEW generation's `registerRoot`, and an already-deployed earlier factory is immutable, so the one
/// open direction is precisely the one defence in depth cannot cover. That is a real limit of this
/// design rather than an oversight, and it is written here so no reader infers a symmetry that does
/// not exist. It is not protected by obscurity either: every root is public in `RootIssued`
/// (`DogTagIssuer.sol:28`) and `RootRegistered` (`DogTagIssuerFactory.sol:22`).
///
/// It is closed OPERATIONALLY, and that is a PRECONDITION of deploying this router rather than a
/// nice-to-have. Freeze earlier-generation issuance at cutover by delisting every signer in the
/// earlier `IssuerRegistry` (registry-plan cutover step C-12): with no whitelisted signer left, the
/// `onlyWhitelisted` gate on `issue` refuses the mirror anchor at its source. Delisting is safe for
/// the revocations that must keep working, and that is why this remedy is available at all - `revoke`
/// is `onlyWhitelisted`, but `adminRevoke` is gated on the registry DEFAULT_ADMIN alone
/// (`DogTagIssuer.sol:84-85`), so earlier-generation revocations survive the freeze.
///
/// `test_a_root_first_anchored_later_can_still_be_claimed_by_an_earlier_generation` pins this
/// direction as a known, accepted limitation, so it is never mistaken for covered ground.
///
/// # WHY THIS DOES NOT REVERT WHEN TWO GENERATIONS ANSWER
///
/// Reverting on a conflict looks like the fail-closed choice and it is a denial of service: anyone
/// able to anchor in a later generation could kill an honest credential permanently by re-anchoring
/// its root in a clone they control, and the victim has no remedy because the router cannot be
/// repointed. Someone will propose the revert, so it is written down here: oldest-first is
/// DETERMINISTIC and cannot be perturbed by an attacker, and that is the property that matters. A
/// duplicate is not an error condition to be surfaced; it is a claim that arrived too late to count.
///
/// # WHY THE GENERATION LIST IS APPEND-ONLY RATHER THAN IMMUTABLE
///
/// Appending at the TAIL under oldest-first resolution is MONOTONE: a new last generation is consulted
/// only after every existing one has answered `address(0)`, so it can add an answer for a root that
/// previously resolved to nothing and can never change an answer that already exists. That is what
/// makes the mutation safe, and it is why the other three shapes are absent by construction: there is
/// no insert, no replace, no reorder and no REMOVAL. Removal is the same denial of service as the
/// revert above, aimed at a whole generation at once.
///
/// The alternative - a frozen list, with a new router for every new generation - forces a new
/// verification registry alongside it every time, because `rootIndex` is immutable. It also makes the
/// cross-generation write guard below unwireable: a router deployed after its newest factory cannot be
/// named in that factory's constructor. Append-only inverts that ordering (deploy the router first,
/// append each new factory as it is deployed) and is what turns "backwards compatibility" into a
/// standing property rather than a one-off migration.
///
/// The owner can therefore add a factory whose clones this router will vouch for. That is a real
/// authority and it is deliberately the same tier as the existing protocol admin, which can already
/// whitelist an arbitrary signer on any record type. It cannot reach BACKWARDS: no append changes what
/// any already-anchored root resolves to.
///
/// # THE CROSS-GENERATION WRITE GUARD (defence in depth, and NOT what oldest-first rests on)
///
/// A later generation's factory SHOULD call {isRootAnchored} from its `registerRoot` and refuse a root
/// an earlier generation already holds, so the duplicate never comes into existence at all.
///
/// Read the two together carefully, because conflating them is how this contract's guarantee gets
/// silently weakened. Oldest-first is LOAD-BEARING: it holds no matter how a later factory behaves,
/// including one that is unguarded, buggy or hostile. The write guard is DEFENCE IN DEPTH: it stops
/// the duplicate being created by a WELL-BEHAVED factory. A router whose safety depended on every
/// future factory being well-behaved would rest on exactly the assumption it exists to remove, so the
/// guard is never a substitute for the ordering.
///
/// # WHAT IT DELIBERATELY DOES NOT CARRY
///
/// No `predictIssuer`. The factory read is `predictIssuer(recordType, business)` and the answer is
/// generation-specific (it depends on that factory's `implementation` and its own address), so a
/// router-level version would have to pick a generation and would silently answer for the wrong one.
/// The only consumer today is `IssuerDomainRegistry`, which holds its own factory reference and reads
/// it directly, so nothing regresses; the resolver that supersedes it must take a generation
/// explicitly rather than expect this router to guess.
contract CloneProvenanceRouter is Ownable2Step {
    /// @notice Upper bound on the generation list.
    /// @dev {rootIssuer} loops with one external call per generation and is called from inside
    /// `recordVerificationZK`, a STATE-CHANGING transaction, so every generation is charged to the
    /// relayer submitting a verification. An unbounded append is therefore an admin denial of service
    /// on verification itself, which is why the cap is enforced rather than merely intended.
    uint256 public constant MAX_GENERATIONS = 8;

    /// @dev Oldest first. Index 0 is the earliest generation and is consulted first.
    address[] private _generations;

    /// @notice Whether `factory` is one of this router's generations. Distinct from {isClone}, which
    /// asks whether an address was DEPLOYED BY a generation; a factory is never one of its own clones.
    /// @dev Kept alongside the array so an append rejects a duplicate in O(1) rather than by scanning.
    mapping(address => bool) public isGeneration;

    event GenerationAppended(address indexed factory, uint256 indexed index);

    error EmptyGenerationList();
    error ZeroAddress();
    error DuplicateGeneration();
    error TooManyGenerations();
    error IndexOutOfRange();
    error RenounceDisabled();
    error GenerationDoesNotAnswer(address factory);

    /// @param generations_ the factory generations, OLDEST FIRST. At least one is required: a router
    /// holding none would answer `address(0)` for every root, which the verification registry reads as
    /// `unknown root` - it would brick verification for the whole protocol rather than fail visibly.
    /// @param admin_ the owner. Two-step handover only; see {transferOwnership}/{acceptOwnership}.
    constructor(address[] memory generations_, address admin_) Ownable(admin_) {
        if (generations_.length == 0) revert EmptyGenerationList();
        if (generations_.length > MAX_GENERATIONS) revert TooManyGenerations();
        for (uint256 i; i < generations_.length; i++) {
            _append(generations_[i]);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // The read surface the immutable `rootIndex` slot consumes
    // ---------------------------------------------------------------------------------------------

    /// @notice The clone that issued `root`, resolved OLDEST GENERATION FIRST.
    ///
    /// @dev Returns `address(0)` when no generation holds the root, and does NOT revert: an unknown
    /// root is a normal state (a root that was never anchored), and the caller already distinguishes
    /// it - `VerificationRegistryConsent` answers `unknown root`
    /// (`VerificationRegistryConsent.sol:187`). Reverting here would replace that specific diagnosis
    /// with an opaque failure.
    ///
    /// The loop MUST run from index 0 upwards. Reversing it is a revocation bypass; see the
    /// resolution-order section on the contract.
    function rootIssuer(bytes32 root) external view returns (address) {
        uint256 n = _generations.length;
        for (uint256 i; i < n; i++) {
            address clone = IIssuerFactoryGeneration(_generations[i]).rootIssuer(root);
            if (clone != address(0)) return clone;
        }
        return address(0);
    }

    /// @notice True iff `candidate` was deployed by ANY generation this router holds.
    /// @dev Order is irrelevant to a boolean union, unlike {rootIssuer}. This is link 1 of the
    /// issuer<->domain chain and the provenance predicate the apps read; routing it here is what lets
    /// one app build recognise clones of every generation.
    function isClone(address candidate) external view returns (bool) {
        uint256 n = _generations.length;
        for (uint256 i; i < n; i++) {
            if (IIssuerFactoryGeneration(_generations[i]).isClone(candidate)) return true;
        }
        return false;
    }

    /// @notice True iff any generation already holds `root`.
    ///
    /// @dev THE CROSS-GENERATION WRITE GUARD. A later generation's `registerRoot` should call this and
    /// refuse when it answers true, so a root anchored (and possibly revoked) in an earlier generation
    /// can never be re-anchored in a later one.
    ///
    /// Safe for a factory that is itself in the list to call: `registerRoot` checks before it writes,
    /// so its own mapping still answers `address(0)` for the root under consideration and the result
    /// reflects only the OTHER generations. All reads, so no reentrancy surface.
    ///
    /// This is defence in depth. {rootIssuer}'s ordering is what makes the router safe against a LATER
    /// generation that does NOT call this. It does not, and structurally cannot, cover the mirror
    /// direction; see the residual section on the contract.
    function isRootAnchored(bytes32 root) external view returns (bool) {
        uint256 n = _generations.length;
        for (uint256 i; i < n; i++) {
            if (IIssuerFactoryGeneration(_generations[i]).rootIssuer(root) != address(0)) return true;
        }
        return false;
    }

    // ---------------------------------------------------------------------------------------------
    // Enumeration
    // ---------------------------------------------------------------------------------------------

    /// @notice The generation list, oldest first.
    function generations() external view returns (address[] memory) {
        return _generations;
    }

    function generationCount() external view returns (uint256) {
        return _generations.length;
    }

    /// @notice The generation at `index`; index 0 is the oldest and is resolved first.
    function generationAt(uint256 index) external view returns (address) {
        if (index >= _generations.length) revert IndexOutOfRange();
        return _generations[index];
    }

    // ---------------------------------------------------------------------------------------------
    // Admin
    // ---------------------------------------------------------------------------------------------

    /// @notice Append a factory generation at the TAIL, so it is consulted LAST.
    ///
    /// @dev Tail-only is the whole safety argument: a new last generation is reached only once every
    /// existing one has answered `address(0)`, so this call cannot change what any already-anchored
    /// root resolves to. There is deliberately no counterpart that inserts, replaces, reorders or
    /// removes - each of those breaks that monotonicity, and removal in particular is a denial of
    /// service on every credential the removed generation anchored. A hostile generation is remedied
    /// by a new router, the same remedy this codebase already uses for every immutable binding.
    function appendGeneration(address factory) external onlyOwner {
        if (_generations.length >= MAX_GENERATIONS) revert TooManyGenerations();
        _append(factory);
    }

    function _append(address factory) internal {
        if (factory == address(0)) revert ZeroAddress();
        if (isGeneration[factory]) revert DuplicateGeneration();
        _requireAnswers(factory);
        isGeneration[factory] = true;
        _generations.push(factory);
        emit GenerationAppended(factory, _generations.length - 1);
    }

    /// @dev Refuse a generation that cannot answer the two reads this router will make for the life of
    /// the protocol.
    ///
    /// This is not defensive decoration. {rootIssuer} makes a high-level call to EVERY generation, so
    /// one entry that reverts or returns nothing makes the WHOLE router revert for every root - and
    /// because there is deliberately no removal, and because the verification registry's `rootIndex`
    /// is immutable, an appended EOA or wrong address would brick verification protocol-wide with no
    /// repair. The one mutable surface on this contract is therefore the one place a typo could
    /// reproduce exactly the unrecoverable failure the contract exists to prevent.
    ///
    /// A `staticcall` rather than a code-size check, because it catches all three shapes at once: an
    /// EOA (succeeds with empty returndata), a contract without this function (reverts or falls
    /// through a fallback), and one that answers with the wrong width. It cannot prove the address is
    /// an honest factory - nothing on chain can - it only proves the call will not brick the router.
    function _requireAnswers(address factory) internal view {
        (bool ok, bytes memory ret) =
            factory.staticcall(abi.encodeCall(IIssuerFactoryGeneration.rootIssuer, (bytes32(0))));
        if (!ok || ret.length != 32) revert GenerationDoesNotAnswer(factory);

        (ok, ret) = factory.staticcall(abi.encodeCall(IIssuerFactoryGeneration.isClone, (address(0))));
        if (!ok || ret.length != 32) revert GenerationDoesNotAnswer(factory);
    }

    /// @notice Disabled. Ownership can only be HANDED OVER, never dropped.
    ///
    /// @dev `Ownable2Step` gives the handover its two steps, which is what stops a mistyped
    /// `transferOwnership` stranding the role: an unaccepted transfer leaves the current owner fully in
    /// place. `renounceOwnership` is inherited from `Ownable` and is not two-step - it drops the role
    /// to `address(0)` in ONE transaction, with no acceptance and no way back, which is precisely the
    /// permanent stranding the two-step pattern was chosen to prevent. Overridden to revert so the one
    /// surviving one-way door is closed.
    ///
    /// Deliberately NOT `onlyOwner`: the function is disabled for everyone, so every caller gets the
    /// honest reason rather than the owner alone learning it and a non-owner being told they are not
    /// the owner of a capability that does not exist.
    function renounceOwnership() public pure override {
        revert RenounceDisabled();
    }
}
