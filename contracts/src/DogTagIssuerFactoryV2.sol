// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {DogTagIssuerV2} from "./DogTagIssuerV2.sol";

/// @dev The approval oracle. `isWhitelistedFor(bytes32,address)` is deliberately the ONLY function this
/// factory asks of it, because that signature exists today on the deployed `IssuerRegistry` AND is a
/// load-bearing interface requirement on the generation-2 authority core (`ProviderRegistry`, slice S-6,
/// plan §1): the core must expose it with exactly this shape so it can drop into the immutable slots
/// existing consumers hold. Binding to that one function is what lets this factory be deployed against
/// today's registry and later against the core with **no code change and no adapter**.
interface IProviderApproval {
    function isWhitelistedFor(bytes32 recordType, address signer) external view returns (bool);
}

/// @dev A prior generation's write-once root index — either the generation-1 `DogTagIssuerFactory`
/// itself, or (for a third generation) a `CloneProvenanceRouter` spanning every earlier generation.
/// This is the exact and only shape `VerificationRegistryConsent` consumes (`:34-36`), so the same
/// address serves both roles.
///
/// **Two requirements on whatever occupies this slot, and both are permanent because the reference is
/// immutable.** It MUST return `address(0)` for a root it has never seen, and it MUST NOT revert. This
/// factory treats the call as a gate on every `registerRoot`, so a reverting occupant would brick
/// issuance for the whole generation with no way to repoint. Stated here for slice S-8.
interface IRootIssuerIndex {
    function rootIssuer(bytes32 root) external view returns (address);
}

/// @dev The two getters this factory reads back off a clone it deployed. Never called on an address
/// that failed `isClone` — see `cloneAuthorization`.
interface IOwnedIssuer {
    function owner() external view returns (address);
    function recordType() external view returns (bytes32);
}

/// @title DogTagIssuerFactoryV2 — self-service clone deployment, plus the clone-authorization predicate.
///
/// @notice Generation-2 successor to `DogTagIssuerFactory`. Three changes, each answering a specific
/// requirement, and nothing else moves — `isClone`, `rootIssuer` and `registerRoot` keep generation 1's
/// semantics exactly.
///
/// # 1. Creation is self-service, and the clone's owner is its creator
///
/// `DogTagIssuerFactory.createIssuer` is `onlyOwner` (`:36`), so no provider can deploy anything: every
/// clone in existence was minted by the protocol multisig. Here creation is gated instead on the caller
/// being an approved provider **for the record type being created** — the captain's *"they can deploy
/// their own clone contracts from the factory after being approved"*.
///
/// There is exactly ONE creation path and it takes no `owner` argument: the owner is always
/// `msg.sender`. An "operator creates on behalf of a provider" variant was considered and rejected,
/// because it is the shape that produced the generation-1 weakness — `business` defaulting to the
/// operator's own signer (`stacks/admin/api/src/routes.rs::resolve_business`), so every deployed clone's
/// "spawning business" is the operator rather than the organisation. With a single path, **`owner()` and
/// the salt binding can never disagree**, and an operator wanting a protocol-owned clone simply creates
/// one itself. Migration is unaffected: the governance signer is already whitelisted for the live record
/// types.
///
/// **This widens the `isClone` set**: under generation 1 only the protocol owner could add to it, and now
/// any whitelisted signer can. The mandatory issuer-whitelist pillar is unaffected, because it keys the
/// whitelist question on `clone.recordType()` rather than on any claim — a provider cleared only for
/// VACCINATION can create only a VACCINATION clone, whose `recordType()` is VACCINATION, so nothing it
/// produces can read as a TRAVEL credential. Pinned by
/// `test_a_widened_clone_set_cannot_forge_a_record_type`. Root squatting is unchanged in kind: any
/// whitelisted signer could already burn a root through `"root taken"` in generation 1, and it stays
/// infeasible against salted Poseidon roots because the attacker must first know one.
///
/// # 2. The salt carries a nonce, so a provider has somewhere to move to
///
/// Generation 1 salts a clone `keccak256(recordType, business)` and `Clones.cloneDeterministic` reverts
/// on a repeated `(implementation, salt)` pair, so a provider has **exactly one possible clone address
/// per record type** and a second `createIssuer` for the same pair simply reverts. The captain's *"they
/// can also change their smart contract address, to a VALID CLONED smart contract"* is then unreachable —
/// there is no second address to change to. Adding `cloneNonce` gives a provider a fresh clone for key
/// rotation or after a compromise, while keeping everything the old salt bought: the address is still
/// deterministic and still exactly predictable before deployment (`predictIssuer`).
///
/// # 3. `authorizeClone` — the resolving predicate, published once
///
/// The forgery-proof repoint predicate is defined HERE and nowhere else, so consumers call it rather
/// than re-deriving it. Two contracts need it (`ProviderRegistry`'s service attachment, S-6; and
/// `ServiceDomainResolver`'s domain write, S-9), and two parallel implementations of one authorization
/// rule is how the vet and mobile verdict paths came to disagree in this codebase already.
///
/// The rule, and why each half is load-bearing (plan §3.1):
///
///   * **Provenance** — `isClone[candidate]`, read from *this factory's own storage*. That mapping is
///     written in exactly one place, `createIssuer`, so an address is in it if and only if this factory
///     deployed it. This is what stops a hand-rolled contract, and it is checked FIRST, which means the
///     external getters below are only ever called on code this factory itself deployed. A hostile
///     address cannot execute at all, not even to revert. Pinned by
///     `test_a_hostile_impostor_is_never_even_called`.
///   * **Control** — `IOwnedIssuer(candidate).owner() == claimant`, read live from the named contract.
///     Without it, provider A could repoint its listing at provider B's genuine clone: not contract
///     forgery, but misattribution, and *"there's no way to do any false contract inputs"* is true of
///     provenance and silent about attribution.
///   * **Attachment** — that the clone belongs to *this provider* in the authority core — is S-6's, not
///     this factory's. This predicate answers provenance and control; the core composes it with identity.
///
/// **The record type is RETURNED, not accepted.** `authorizeClone` reads `recordType()` off the clone and
/// hands it back, so a caller keying anything by record type gets a chain-resolved key in the same call
/// that authorized the address. This is the codebase's standing rule applied to a write path: *an address
/// may be an argument to a write, checked against the registry's own factory reference; an address may
/// never be an argument to a read that decides trust.* The caller of `setActiveIssuer` supplies one
/// value — the target address — and both the authorization and the storage key are resolved from chain
/// state. There is no argument by which a caller could name a slot it did not earn.
///
/// # The active-issuer pointer, and how it relates to S-6
///
/// `activeIssuer` is the **owner-keyed, self-service** pointer: which of a provider's own clones is
/// currently its live one for a record type. `ProviderRegistry`'s providerId-keyed, registrar-confirmed
/// service attachment (S-6) is the **authoritative** record of which contracts belong to which
/// organisation; these are complementary, not competing, and they are keyed differently on purpose (an
/// owner address is a key, an organisation is not).
///
/// The pointer is re-validated on READ. `resolveActiveIssuer` returns the stored address only if it still
/// passes the same predicate, and `address(0)` otherwise — so a pointer left stale by an ownership
/// handover degrades to "no active issuer recorded" rather than to a claim that is no longer true. A
/// stale pointer is a could-not-establish, and this codebase does not render those as established.
///
/// # What a repoint does NOT do
///
/// It changes only where **new** credentials are anchored. `rootIssuer[R]` is write-once, so everything
/// the old clone already issued keeps resolving to the old clone and stays revocable there. That is the
/// correct behaviour rather than a limitation: retroactively re-attributing issued credentials to a
/// contract that did not issue them is exactly the misattribution the control check exists to prevent.
///
/// # No admin surface
///
/// This factory has no owner and no privileged function. Nothing about it can be repointed or captured,
/// which is the property `IssuerDomainRegistry`'s doc asks of a factory reference: *"a repointable
/// factory reference would let one transaction redefine what counts as a genuine clone."* Here there is
/// nothing to repoint, in either direction.
contract DogTagIssuerFactoryV2 {
    address public immutable implementation;
    /// @notice The approval oracle: today's `IssuerRegistry`, tomorrow's `ProviderRegistry`.
    address public immutable registry;
    /// @notice A prior generation's root index, or `address(0)` for the first generation. See
    /// [`IRootIssuerIndex`] for the two permanent requirements on whatever sits here.
    IRootIssuerIndex public immutable priorIndex;

    mapping(address => bool) public isClone; // deployed by this factory
    mapping(bytes32 => address) public rootIssuer; // R -> issuing clone (write-once)

    /// @notice owner => recordType => the clone that owner most recently designated as live. RAW: it is
    /// not re-validated on this read. Use [`resolveActiveIssuer`] unless you are auditing history.
    mapping(address => mapping(bytes32 => address)) public activeIssuer;

    /// @dev Byte-identical to generation 1's, so the oversight indexer's existing decoder reads a
    /// generation-2 creation with no change. The owner is carried in the additive event below rather
    /// than by widening this one.
    event IssuerCreated(address indexed clone, bytes32 indexed recordType, string name);
    /// @dev The generation-2 addition. Emitted from the factory (not only as the clone's own
    /// `OwnershipTransferred`) so one log filter on the factory address captures the whole creation.
    event IssuerOwnerRegistered(address indexed clone, address indexed owner, uint96 cloneNonce);
    event RootRegistered(bytes32 indexed root, address indexed clone);
    event ActiveIssuerSet(address indexed owner, bytes32 indexed recordType, address indexed clone);
    event ActiveIssuerCleared(address indexed owner, bytes32 indexed recordType, address previousClone);

    /// @dev The caller is not an approved provider for the record type it tried to create.
    error NotApproved();
    /// @dev The named address was not deployed by this factory.
    error NotAClone();
    /// @dev The named address is a genuine clone, but the claimant does not own it.
    error NotCloneOwner();
    error ZeroAddress();

    /// @param impl the `DogTagIssuerV2` implementation clones delegate to.
    /// @param registry_ the approval oracle — see [`IProviderApproval`].
    /// @param priorIndex_ a prior generation's root index, or `address(0)` for the first generation.
    constructor(address impl, address registry_, address priorIndex_) {
        if (impl == address(0) || registry_ == address(0)) revert ZeroAddress();
        implementation = impl;
        registry = registry_;
        priorIndex = IRootIssuerIndex(priorIndex_);
    }

    function _salt(bytes32 recordType, address business, uint96 cloneNonce) internal pure returns (bytes32) {
        return keccak256(abi.encode(recordType, business, cloneNonce));
    }

    // ---------------------------------------------------------------------------------------------
    // Creation
    // ---------------------------------------------------------------------------------------------

    /// @notice Deploy a clone owned by the caller. Requires the caller to be approved for `recordType`.
    /// @param cloneNonce lets one provider hold several clones for a record type; see the contract doc.
    function createIssuer(string calldata name, bytes32 recordType, uint96 cloneNonce)
        external
        returns (address clone)
    {
        if (!IProviderApproval(registry).isWhitelistedFor(recordType, msg.sender)) {
            revert NotApproved();
        }
        clone = Clones.cloneDeterministic(implementation, _salt(recordType, msg.sender, cloneNonce));
        isClone[clone] = true;
        DogTagIssuerV2(clone).initialize(name, recordType, registry, address(this), msg.sender);
        emit IssuerCreated(clone, recordType, name);
        emit IssuerOwnerRegistered(clone, msg.sender, cloneNonce);
    }

    /// @notice The exact address `createIssuer(_, recordType, cloneNonce)` would produce for `business`.
    function predictIssuer(bytes32 recordType, address business, uint96 cloneNonce)
        external
        view
        returns (address)
    {
        return Clones.predictDeterministicAddress(
            implementation, _salt(recordType, business, cloneNonce), address(this)
        );
    }

    // ---------------------------------------------------------------------------------------------
    // The authorization predicate — one implementation, two shapes
    // ---------------------------------------------------------------------------------------------

    /// @notice Non-reverting form, for consoles rendering "can this key act on this contract?".
    /// @return ok whether `claimant` may act for `candidate`.
    /// @return recordType the clone's own record type, resolved from chain state; zero when `!ok`.
    function cloneAuthorization(address candidate, address claimant)
        public
        view
        returns (bool ok, bytes32 recordType)
    {
        // Own storage first. Everything below only ever executes against code this factory deployed.
        if (!isClone[candidate]) return (false, bytes32(0));
        if (claimant == address(0)) return (false, bytes32(0));
        if (IOwnedIssuer(candidate).owner() != claimant) return (false, bytes32(0));
        return (true, IOwnedIssuer(candidate).recordType());
    }

    /// @notice Fail-closed form — the one a write path calls. Reverts unless `claimant` may act for
    /// `candidate`, and returns the clone's chain-resolved record type so the caller never supplies it.
    function authorizeClone(address candidate, address claimant) public view returns (bytes32 recordType) {
        if (!isClone[candidate]) revert NotAClone();
        if (claimant == address(0) || IOwnedIssuer(candidate).owner() != claimant) revert NotCloneOwner();
        return IOwnedIssuer(candidate).recordType();
    }

    // ---------------------------------------------------------------------------------------------
    // The self-service pointer
    // ---------------------------------------------------------------------------------------------

    /// @notice Designate `clone` as the caller's live issuer for that clone's record type. The caller
    /// supplies ONLY the address: provenance, control and the storage key are all resolved.
    function setActiveIssuer(address clone) external {
        bytes32 recordType = authorizeClone(clone, msg.sender);
        activeIssuer[msg.sender][recordType] = clone;
        emit ActiveIssuerSet(msg.sender, recordType, clone);
    }

    /// @notice Withdraw the caller's own pointer. Needs no authorization beyond owning the slot: the
    /// mapping is keyed by `msg.sender`, so a caller can only ever clear its own.
    function clearActiveIssuer(bytes32 recordType) external {
        address previous = activeIssuer[msg.sender][recordType];
        delete activeIssuer[msg.sender][recordType];
        emit ActiveIssuerCleared(msg.sender, recordType, previous);
    }

    /// @notice `provider`'s live clone for `recordType`, or `address(0)` if none is recorded **or the
    /// recorded one no longer passes the predicate** (see the contract doc on stale pointers).
    ///
    /// @dev **This function cannot revert, and that is structural rather than incidental** — worth
    /// stating because unlike `authorizeClone` the caller does not choose the address this dispatches
    /// to, so a consumer (S-6's attachment, S-9's domain read) treats it as a cheap resolve and would be
    /// surprised by a revert where it expected an address.
    ///
    /// The only address reachable here is one already in `isClone`, which `createIssuer` only ever fills
    /// with `Clones.cloneDeterministic(implementation, ...)` — and `implementation` is `immutable`. So
    /// every callee is a `DogTagIssuerV2`, and `owner()`/`recordType()` there are plain public getters
    /// over storage: no external call, no loop, no revert path. A hostile or absent callee is
    /// unreachable, not merely unlikely.
    ///
    /// Deliberately NOT wrapped in `try/catch`. A catch arm here could never execute, so it would be an
    /// untestable branch standing in for a case that cannot arise — and this codebase treats an
    /// unexercised guard as worse than none. If a future implementation ever gives those getters a
    /// revert path, this guarantee moves with it and the choice has to be made again on the evidence.
    function resolveActiveIssuer(address provider, bytes32 recordType) external view returns (address) {
        address clone = activeIssuer[provider][recordType];
        if (clone == address(0)) return address(0);
        (bool ok, bytes32 rt) = cloneAuthorization(clone, provider);
        if (!ok || rt != recordType) return address(0);
        return clone;
    }

    // ---------------------------------------------------------------------------------------------
    // The write-once root index — generation 1's semantics, plus one cross-generation guard
    // ---------------------------------------------------------------------------------------------

    /// @notice Write-once registration of `root -> issuing clone` (§11.10(a), audit-11 V4-C1/M1).
    ///
    /// @dev The `priorIndex` check is the write-side half of the provenance router's resolution order
    /// (slice S-8, plan §1). The write-once guards are **per contract**: `DogTagIssuer.issue` checks
    /// `issuedAt[r]` in its own storage and `registerRoot` checks `rootIssuer[root]` in its own factory's
    /// storage. So a root anchored and then REVOKED on a generation-1 clone could be re-anchored on a
    /// generation-2 clone by any signer whitelisted for that clone's record type — both guards pass,
    /// because neither contract has ever seen it — and the tag binding does not stop it either, since the
    /// SBT is shared and `R == profileRoot(dogTagId)` still holds. Under newest-first resolution the
    /// revoked credential would then verify again.
    ///
    /// The router closes that on the read side by resolving oldest-first. This closes it on the write
    /// side, so the duplicate never comes into existence at all. Both, deliberately: this guard is only
    /// as good as the deployed `priorIndex`, and the router's ordering must remain correct on its own.
    /// This does NOT loosen either write-once guard — it only ever refuses more.
    function registerRoot(bytes32 root) external {
        require(isClone[msg.sender], "!clone");
        require(rootIssuer[root] == address(0), "root taken"); // strictly write-once
        if (address(priorIndex) != address(0)) {
            require(priorIndex.rootIssuer(root) == address(0), "root taken upstream");
        }
        rootIssuer[root] = msg.sender;
        emit RootRegistered(root, msg.sender);
    }
}
