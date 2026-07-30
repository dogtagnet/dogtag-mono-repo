// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {DogTagIssuerV2, IProviderAuthority} from "./DogTagIssuerV2.sol";

/// @dev The two reads this generation requires from the S-8 `CloneProvenanceRouter`.
/// `isRootAnchored` answers "does ANY generation already hold this root", spanning every generation the
/// router carries rather than one factory's own mapping. `isGeneration` is the membership assertion the
/// router publishes; this factory requires a true answer for itself before it may accept an anchor.
///
/// **The requirements on whatever occupies this slot are permanent because the reference is
/// immutable.** It MUST answer both selectors without reverting; it MUST answer `false` for a root
/// nothing has anchored; and it MUST report this factory as a generation before `registerRoot` accepts
/// anything. The constructor probes both selectors before storing the reference, while this factory
/// cannot yet have been appended because it has no runtime code. `registerRoot` enforces the membership
/// transition afterwards. An occupant that reverts, claims every root, or never records this factory
/// therefore fails loudly instead of producing anchors the protocol router cannot resolve.
///
/// This is deliberately NOT `rootIssuer(bytes32)`. That selector is the generation-LOCAL index — a
/// generation-1 factory answers only for roots its own clones anchored — so wiring it here would leave
/// every generation before the immediately preceding one unguarded the moment a third generation exists.
/// `isRootAnchored` is the query that spans them, which is why S-8 published it.
interface IPriorRootAnchors {
    function isRootAnchored(bytes32 root) external view returns (bool);
    function isGeneration(address factory) external view returns (bool);
}

/// @dev The owner-bearing getter surface authorization reads from every clone of this generation.
/// Neither getter is called on an address that failed `isClone` — see
/// {DogTagIssuerFactoryV2-_authorization}. The implementation's complete identity is established by
/// exact runtime-code hash at construction, not by probing this partial read surface.
interface IOwnedIssuer {
    function owner() external view returns (address);
    function recordType() external view returns (bytes32);
}

/// @title DogTagIssuerFactoryV2 — self-service clone deployment, plus the clone-authorization predicate.
///
/// @notice Generation-2 successor to `DogTagIssuerFactory`. `isClone`, `rootIssuer` and `registerRoot`
/// keep generation 1's semantics; what moves is who may create, what a clone's owner is, and which
/// oracle is asked.
///
/// # 1. Creation is self-service, and the clone's owner is its creator
///
/// `DogTagIssuerFactory.createIssuer` is `onlyOwner` (`:36`), so no provider can deploy anything: every
/// clone in existence was minted by the protocol multisig. Here creation is gated instead on the
/// authority core's `canCreateService(providerId, recordType, msg.sender)` — the captain's *"they can
/// deploy their own clone contracts from the factory after being approved"*.
///
/// The `providerId` is the core's opaque registrar-assigned identity. It is an **argument to a write,
/// checked against the factory's own immutable core reference** — never a read that decides trust — and
/// the core resolves this factory from `msg.sender`, so a caller cannot name a generation either. That
/// gate folds four registrar facts the factory could not check itself: this factory is a pinned, active
/// generation on the core; the provider is cleared; the provider is approved for this record type; and
/// the caller may act for the provider (its controller, or a delegate holding the create permission).
///
/// **Creation is not issuance, and the gate is deliberately not the issuance capability.** A freshly
/// created clone can anchor nothing: `canIssue` additionally requires a registrar attachment, an
/// issuance grant for the signer, a confirmed owner and the provider's current pointer. Generation 1
/// conflated the two by gating creation on `isWhitelistedFor`, which on the core is not a creation
/// question at all — for a caller that is not itself an attached service it answers the orthogonal
/// VERIFY-key capability.
///
/// There is exactly ONE creation path and it takes no `owner` argument: the owner is always
/// `msg.sender`. An "operator creates on behalf of a provider" variant was considered and rejected,
/// because it is the shape that produced the generation-1 weakness — `business` defaulting to the
/// operator's own signer (`stacks/admin/api/src/routes.rs::resolve_business`), so every deployed clone's
/// "spawning business" is the operator rather than the organisation. With a single path, **`owner()` and
/// the salt binding agree at creation** — they can diverge later, and only through the two-step
/// handover, which is the one act that is supposed to move control.
///
/// **This widens the `isClone` set**: under generation 1 only the protocol owner could add to it, and now
/// any approved provider can. The mandatory issuer-whitelist pillar is unaffected, because it keys the
/// whitelist question on `clone.recordType()` rather than on any claim — a provider approved only for
/// VACCINATION can create only a VACCINATION clone, whose `recordType()` is VACCINATION, so nothing it
/// produces can read as a TRAVEL credential. Pinned by
/// `test_a_widened_clone_set_cannot_forge_a_record_type`. The other thing the widened set could have
/// carried is a provider-chosen `name()`, and that is closed at the source: this generation's clones have
/// no settable name — see `DogTagIssuerV2`.
///
/// Root squatting is unchanged in kind: any approved signer could already burn a root through
/// `"root taken"` in generation 1. Salted Poseidon roots make it infeasible to GUESS a root, which is a
/// narrower claim than it looks — a root becomes public the moment an `issue` transaction is observable,
/// so an attacker watching the mempool can still front-run a specific pending anchor. What salting rules
/// out is blind, untargeted squatting.
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
/// `providerId` is deliberately NOT in the salt. The factory stores no provider binding and the core
/// resolves a service's provider from its own registrar attachment, so a salted `providerId` would be
/// unresolvable by any consumer — an appearance of binding with nothing behind it. The only thing it
/// would buy is address separation for two providers sharing one caller address, which bumping the nonce
/// already buys. The intended provider is recorded in {IssuerOwnerRegistered} instead, where it is
/// legible as the creation's stated intent and not mistakable for a resolvable fact.
///
/// # 3. `authorizeClone` — the resolving predicate, published once
///
/// The forgery-proof repoint predicate is defined HERE and nowhere else, so consumers call it rather
/// than re-deriving it. Two contracts need it (`ProviderRegistry`'s service attachment, S-6; and
/// `ServiceDomainResolver`'s domain write, S-9), and two parallel implementations of one authorization
/// rule is how the vet and mobile verdict paths came to disagree in this codebase already. Both public
/// shapes below are thin wrappers over one internal `_authorization`, so the rule cannot be half-changed
/// and a candidate that failed provenance cannot be reached by either.
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
/// # The active-issuer pointer is ADVISORY
///
/// `activeIssuer` is the **owner-keyed, self-service** record of which of a provider's own clones its
/// controlling key currently designates as live. It is advisory and nothing routes through it: an `issue`
/// call reaches whichever clone the caller called, and whether it succeeds is decided entirely by the
/// core's `canIssue` — which folds the core's OWN providerId-keyed pointer, not this one. So this pointer
/// cannot authorize anything, cannot redirect anything, and a stale one cannot cause a wrong anchor.
/// `ProviderRegistry`'s registrar-confirmed service attachment (S-6) is the authoritative record of which
/// contracts belong to which organisation; the two are complementary and keyed differently on purpose (an
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
/// nothing to repoint, in either direction — which is also why all three dependencies are checked at
/// construction: a wrong one is remedied only by deploying a new factory.
contract DogTagIssuerFactoryV2 {
    address public immutable implementation;
    /// @notice The authority core — the S-6 `ProviderRegistry`. See [`IProviderAuthority`] for the four
    /// functions this generation requires of it, all four permanent.
    address public immutable registry;
    /// @notice The cross-generation root guard. Mandatory and non-zero; see [`IPriorRootAnchors`].
    IPriorRootAnchors public immutable priorIndex;

    mapping(address => bool) public isClone; // deployed by this factory
    /// @notice R -> issuing clone, write-once, and **generation-local**: this mapping answers only for
    /// roots anchored by THIS factory's clones. Resolution across generations is the S-8 router's job.
    mapping(bytes32 => address) public rootIssuer;

    /// @notice owner => recordType => the clone that owner most recently designated as live. RAW: this
    /// is the CURRENT stored value, not a history — the mapping holds one address per slot and a repoint
    /// overwrites it, so the audit trail is {ActiveIssuerSet}/{ActiveIssuerCleared}, never this read. It
    /// is also not re-validated here; use [`resolveActiveIssuer`] unless you specifically want the raw
    /// designation.
    mapping(address => mapping(bytes32 => address)) public activeIssuer;

    /// @dev Byte-identical to generation 1's, so the oversight indexer's existing decoder reads a
    /// generation-2 creation with no change. `name` is always the empty string — this generation's clones
    /// have no settable name (see `DogTagIssuerV2`) — so the field is carried to preserve the signature
    /// and topic0, not to convey a value.
    event IssuerCreated(address indexed clone, bytes32 indexed recordType, string name);
    /// @dev The generation-2 addition. Emitted from the factory (not only as the clone's own
    /// `OwnershipTransferred`) so one log filter on the factory address captures the whole creation.
    /// `providerId` is the creation's stated intent, checked against the core at the time of the call; the
    /// authoritative provider binding is the core's own attachment, which this factory does not hold.
    event IssuerOwnerRegistered(
        address indexed clone, address indexed owner, bytes20 indexed providerId, uint96 cloneNonce
    );
    event RootRegistered(bytes32 indexed root, address indexed clone);
    event ActiveIssuerSet(address indexed owner, bytes32 indexed recordType, address indexed clone);
    event ActiveIssuerCleared(address indexed owner, bytes32 indexed recordType, address previousClone);

    /// @dev The core refused the creation for this provider, record type and caller.
    error NotApproved();
    /// @dev The named address was not deployed by this factory.
    error NotAClone();
    /// @dev The named address is a genuine clone, but the claimant does not own it.
    error NotCloneOwner();
    error ZeroAddress();
    /// @dev A constructor dependency has no code at all.
    error NotAContract(address dependency);
    /// @dev `impl` is code, but not the exact `DogTagIssuerV2` runtime this factory was compiled for.
    error ImplementationCodeMismatch(address impl);
    error AuthorityDoesNotAnswer(address authority);
    error PriorIndexDoesNotAnswer(address priorIndex);
    /// @dev This factory has not yet been appended to its immutable provenance router.
    error FactoryNotRegisteredInPriorIndex(address factory);

    /// @param impl the `DogTagIssuerV2` implementation clones delegate to.
    /// @param registry_ the authority core — see [`IProviderAuthority`].
    /// @param priorIndex_ the cross-generation root guard — see [`IPriorRootAnchors`]. Mandatory.
    ///
    /// @dev Every dependency is `immutable` and this contract has no admin, so a wrong one is remedied
    /// only by deploying a new factory — and for `priorIndex` that means a new verification registry too,
    /// since its `rootIndex` is immutable as well. Each is therefore checked for code and then either
    /// exact identity (the implementation) or required ABI behaviour (the authority and prior index)
    /// before it is stored, so the ways a wrong address stays silent are closed here rather than
    /// discovered in production: an EOA (a staticcall to one SUCCEEDS, with empty returndata), a contract
    /// that does not answer a required selector, one that answers the wrong width, and an
    /// implementation-shaped impostor.
    ///
    /// The implementation check is exact: `impl.codehash` must equal the hash of
    /// `type(DogTagIssuerV2).runtimeCode` compiled into this factory. Getter-shaped code, including a
    /// fallback that returns one word for every selector, is not sufficient. This pins every clone to
    /// the precise implementation whose owner and record-type reads the factory relies on.
    ///
    /// The core and the prior index are additionally required to answer `false` to zero-everything
    /// queries. For the core that refuses one which authorizes indiscriminately. For the prior index it
    /// refuses one which claims every root, and also proves that `isGeneration` has the expected ABI
    /// before this not-yet-deployed factory can be appended. `registerRoot` later requires that answer
    /// to have become true, enforcing router-first -> factory -> append when the intended router occupies
    /// the slot.
    constructor(address impl, address registry_, address priorIndex_) {
        if (impl == address(0) || registry_ == address(0) || priorIndex_ == address(0)) {
            revert ZeroAddress();
        }

        _requireCode(impl);
        if (impl.codehash != keccak256(type(DogTagIssuerV2).runtimeCode)) {
            revert ImplementationCodeMismatch(impl);
        }

        _requireAuthorityAnswers(registry_);

        _requireCode(priorIndex_);
        if (
            !_answersFalse(priorIndex_, abi.encodeCall(IPriorRootAnchors.isRootAnchored, (bytes32(0))))
                || !_answersFalse(
                    priorIndex_, abi.encodeCall(IPriorRootAnchors.isGeneration, (address(this)))
                )
        ) {
            revert PriorIndexDoesNotAnswer(priorIndex_);
        }

        implementation = impl;
        registry = registry_;
        priorIndex = IPriorRootAnchors(priorIndex_);
    }

    function _requireCode(address dependency) private view {
        if (dependency.code.length == 0) revert NotAContract(dependency);
    }

    /// @dev All four functions of [`IProviderAuthority`], each required to answer `false` to a
    /// zero-everything query. A core missing any one of them cannot be repointed away from later.
    function _requireAuthorityAnswers(address authority_) private view {
        _requireCode(authority_);
        bytes memory create =
            abi.encodeCall(IProviderAuthority.canCreateService, (bytes20(0), bytes32(0), address(0)));
        bytes memory issue_ = abi.encodeCall(IProviderAuthority.canIssue, (address(0), address(0)));
        bytes memory revoke_ = abi.encodeCall(IProviderAuthority.canRevoke, (address(0), address(0)));
        bytes memory role = abi.encodeCall(IProviderAuthority.hasRole, (bytes32(0), address(0)));
        if (
            !_answersFalse(authority_, create) || !_answersFalse(authority_, issue_)
                || !_answersFalse(authority_, revoke_) || !_answersFalse(authority_, role)
        ) {
            revert AuthorityDoesNotAnswer(authority_);
        }
    }

    /// @dev True when the target answers one word and that word is zero. Decoded as `uint256` rather
    /// than `bool` on purpose: a non-conforming word above 1 makes `bool` decoding panic, which would
    /// replace the named revert with an opaque one.
    function _answersFalse(address target, bytes memory call_) private view returns (bool) {
        (bool ok, bytes memory ret) = target.staticcall(call_);
        if (!ok || ret.length != 32) return false;
        return abi.decode(ret, (uint256)) == 0;
    }

    function _salt(bytes32 recordType, address business, uint96 cloneNonce) internal pure returns (bytes32) {
        return keccak256(abi.encode(recordType, business, cloneNonce));
    }

    // ---------------------------------------------------------------------------------------------
    // Creation
    // ---------------------------------------------------------------------------------------------

    /// @notice Deploy a clone owned by the caller. The core must clear the caller to create a service of
    /// `recordType` for `providerId`, which includes having pinned THIS factory as an active generation.
    /// @param cloneNonce lets one provider hold several clones for a record type; see the contract doc.
    function createIssuer(bytes20 providerId, bytes32 recordType, uint96 cloneNonce)
        external
        returns (address clone)
    {
        if (!IProviderAuthority(registry).canCreateService(providerId, recordType, msg.sender)) {
            revert NotApproved();
        }
        clone = Clones.cloneDeterministic(implementation, _salt(recordType, msg.sender, cloneNonce));
        isClone[clone] = true;
        DogTagIssuerV2(clone).initialize(recordType, registry, address(this), msg.sender);
        emit IssuerCreated(clone, recordType, "");
        emit IssuerOwnerRegistered(clone, msg.sender, providerId, cloneNonce);
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

    /// @dev THE rule. Both public shapes below delegate here, so provenance, control and the resolved
    /// key can never be applied in one order by a read and another by a write.
    ///
    /// `known` is reported separately from `ok` only so `authorizeClone` can name the two failures
    /// apart — they have different remedies, and collapsing them would tell a provider whose clone is
    /// genuine that its address is a forgery.
    ///
    /// **Ordering is load-bearing.** `isClone` is own-storage and short-circuits, so nothing below
    /// executes against an address this factory did not deploy: an unrecognised candidate is never
    /// called, not even to revert.
    function _authorization(address candidate, address claimant)
        private
        view
        returns (bool known, bool ok, bytes32 recordType)
    {
        if (!isClone[candidate]) return (false, false, bytes32(0));
        if (claimant == address(0)) return (true, false, bytes32(0));
        if (IOwnedIssuer(candidate).owner() != claimant) return (true, false, bytes32(0));
        return (true, true, IOwnedIssuer(candidate).recordType());
    }

    /// @notice Non-reverting form, for consoles rendering "can this key act on this contract?".
    /// @return ok whether `claimant` may act for `candidate`.
    /// @return recordType the clone's own record type, resolved from chain state; zero when `!ok`.
    function cloneAuthorization(address candidate, address claimant)
        public
        view
        returns (bool ok, bytes32 recordType)
    {
        (, ok, recordType) = _authorization(candidate, claimant);
    }

    /// @notice Fail-closed form — the one a write path calls. Reverts unless `claimant` may act for
    /// `candidate`, and returns the clone's chain-resolved record type so the caller never supplies it.
    function authorizeClone(address candidate, address claimant) public view returns (bytes32 recordType) {
        (bool known, bool ok, bytes32 rt) = _authorization(candidate, claimant);
        if (!known) revert NotAClone();
        if (!ok) revert NotCloneOwner();
        return rt;
    }

    // ---------------------------------------------------------------------------------------------
    // The self-service pointer
    // ---------------------------------------------------------------------------------------------

    /// @notice Designate `clone` as the caller's live issuer for that clone's record type. The caller
    /// supplies ONLY the address: provenance, control and the storage key are all resolved. Advisory —
    /// see the contract doc; it grants no capability and redirects no call.
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
    /// @dev **This function does not revert**, which is worth stating because unlike `authorizeClone` the
    /// caller does not choose the address this dispatches to, so a consumer (S-6's attachment, S-9's
    /// domain read) treats it as a cheap resolve and would be surprised by a revert where it expected an
    /// address.
    ///
    /// The only address reachable here is one already in `isClone`, which `createIssuer` only ever fills
    /// with `Clones.cloneDeterministic(implementation, ...)` — and `implementation` is `immutable` AND
    /// was required at construction to be byte-for-byte the `DogTagIssuerV2` runtime compiled into this
    /// factory. So every callee delegates to the exact code whose `owner()` and `recordType()` getters
    /// this read uses.
    ///
    /// Deliberately NOT wrapped in `try/catch`. Given the above a catch arm would stand in for a case the
    /// exact-code construction check already refuses, and this codebase treats an unexercised guard as
    /// worse than none. A future implementation is a different runtime hash and therefore requires a new
    /// factory whose behaviour is reviewed as a new generation.
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
    /// (slice S-8, plan §1). The write-once guards are **per contract**: `DogTagIssuerV2.issue` checks
    /// `issuedAt[r]` in its own storage and `registerRoot` checks `rootIssuer[root]` in its own factory's
    /// storage. So a root anchored and then REVOKED on a generation-1 clone could be re-anchored on a
    /// generation-2 clone by any signer able to issue there — both guards pass, because neither contract
    /// has ever seen it — and the tag binding does not stop it either, since the SBT is shared and
    /// `R == profileRoot(dogTagId)` still holds. Under newest-first resolution the revoked credential
    /// would then verify again.
    ///
    /// The router closes that on the read side by resolving oldest-first. This closes it on the write
    /// side, so the duplicate never comes into existence at all. Both, deliberately: the router's
    /// ordering must remain correct on its own, because it is what holds against a LATER generation that
    /// does not call this. This does NOT loosen either write-once guard — it only ever refuses more.
    ///
    /// The membership check is a separate load-bearing precondition. The router must be deployed first
    /// so its address can be immutable here; this factory must then be deployed; and the router can only
    /// append it after its runtime code exists. Until that append, anchoring would create a root in this
    /// factory that the protocol-wide resolver does not yet carry. Refusing it with a named error turns
    /// the required router -> factory -> append sequence into contract behaviour rather than an operator
    /// promise when the intended router occupies `priorIndex`.
    ///
    /// Exact extent: this establishes membership as REPORTED by the immutable `priorIndex`; it cannot
    /// authenticate that occupant or prove its generation list is complete. Wiring the real router over
    /// every earlier generation remains a cutover precondition.
    function registerRoot(bytes32 root) external {
        if (!priorIndex.isGeneration(address(this))) {
            revert FactoryNotRegisteredInPriorIndex(address(this));
        }
        require(isClone[msg.sender], "!clone");
        require(rootIssuer[root] == address(0), "root taken"); // strictly write-once
        require(!priorIndex.isRootAnchored(root), "root taken upstream");
        rootIssuer[root] = msg.sender;
        emit RootRegistered(root, msg.sender);
    }
}
