// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {DogTagIssuer, IProviderAuthority} from "./DogTagIssuer.sol";

/// @dev The owner-bearing getter surface authorization reads from every clone.
/// Neither getter is called on an address that failed `isClone` — see
/// {DogTagIssuerFactory-_authorization}. The implementation's complete identity is established by
/// exact runtime-code hash at construction, not by probing this partial read surface.
interface IOwnedIssuer {
    function owner() external view returns (address);
    function recordType() external view returns (bytes32);
}

/// @title DogTagIssuerFactory — self-service clone deployment, plus the clone-authorization predicate.
///
/// @notice Deploys a provider's own `DogTagIssuer` clones, holds the write-once root index every
/// verifier resolves a credential through, and publishes the predicate that decides whether a given
/// contract is a genuine clone under a given owner.
///
/// # 1. Creation is self-service, and the clone's owner is its creator
///
/// Creation is gated on the authority core's `canCreateService(providerId, recordType, msg.sender)` —
/// the captain's *"they can deploy their own clone contracts from the factory after being approved"*.
/// The protocol does not deploy on a provider's behalf; a provider deploys, and owns what it deployed.
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
/// issuance grant for the signer, a confirmed owner and the provider's current pointer. Conflating the
/// two — deploying and being allowed to anchor — would make approval to exist mean approval to issue.
///
/// There is exactly ONE creation path and it takes no `owner` argument: the owner is always
/// `msg.sender`. An "operator creates on behalf of a provider" variant was considered and rejected,
/// because a create-on-behalf-of path is how a clone's recorded business comes to be the operator who
/// pressed the button rather than the organisation it was for — an appearance of provider identity with
/// an operator behind it. With a single path, **`owner()` and
/// the salt binding agree at creation** — they can diverge later, and only through the two-step
/// handover, which is the one act that is supposed to move control.
///
/// **Any approved provider can add to the `isClone` set**, which is the whole point of self-service and
/// is worth stating as a consequence. The mandatory issuer-whitelist pillar is unaffected, because it keys the
/// whitelist question on `clone.recordType()` rather than on any claim — a provider approved only for
/// VACCINATION can create only a VACCINATION clone, whose `recordType()` is VACCINATION, so nothing it
/// produces can read as a TRAVEL credential. Pinned by
/// `test_a_widened_clone_set_cannot_forge_a_record_type`. The other thing a provider-writable clone set
/// could have carried is a provider-chosen `name()`, and that is closed at the source: clones have no
/// settable name — see `DogTagIssuer`.
///
/// Root squatting: any approved signer can burn a root by anchoring it first, since `registerRoot` is
/// strictly write-once. Salted Poseidon roots make it infeasible to GUESS a root, which is a
/// narrower claim than it looks — a root becomes public the moment an `issue` transaction is observable,
/// so an attacker watching the mempool can still front-run a specific pending anchor. What salting rules
/// out is blind, untargeted squatting.
///
/// # 2. The salt carries a nonce, so a provider has somewhere to move to
///
/// The salt is `keccak256(recordType, business, cloneNonce)` and `Clones.cloneDeterministic` reverts on
/// a repeated `(implementation, salt)` pair. Two of those three inputs are fixed for a given provider and
/// record type, so WITHOUT the nonce a provider would have exactly one possible clone address, forever,
/// and the captain's *"they can also change their smart contract address, to a VALID CLONED smart
/// contract"* would be unreachable — there would be no second address to move to. The nonce gives a
/// provider a fresh clone for key rotation or after a compromise while keeping what determinism buys:
/// the address is still exactly predictable before deployment (`predictIssuer`).
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
/// The forgery-proof repoint predicate is published HERE as a function rather than inlined, so that a
/// consumer can call it instead of re-deriving it. Two parallel implementations of one authorization
/// rule is how the vet and mobile verdict paths came to disagree in this codebase already. Both public
/// shapes below are thin wrappers over one internal `_authorization`, so the rule cannot be half-changed
/// and a candidate that failed provenance cannot be reached by either.
///
/// **Nothing composes it today, so this is not the only place the rule lives, and that is stated rather
/// than hidden.** {ServiceDomainResolver} deliberately does not call it — this predicate requires
/// `claimant == owner()` exactly, so it cannot admit an owner-appointed delegate and would leave the
/// core's `SERVICE_PERMISSION_RECORD` bit with no consumer. {ProviderRegistry} derives both halves
/// itself: provenance through its own fail-soft `isClone` staticcall (`_factoryRecognizes`) and control
/// through its own fail-soft `owner()` staticcall (`_readServiceOwner`). On the core's per-issuance
/// predicates that shape is deliberate rather than an omission, for two specific reasons: those reads
/// sit inside `canIssue`, which a clone asks on every `issue`, so a REVERTING predicate cannot serve
/// that call site; and `_isOwnerConfirmed` compares the live `owner()` against the registrar's recorded
/// `confirmedOwner`, so it needs the owner VALUE, which neither shape here returns. Both derivations
/// fail closed, so nothing is unguarded — but they are two derivations, and reconciling them is open.
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
///   * **Attachment** — that the clone belongs to *this provider* in the authority core — is the core's, not
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
/// `ProviderRegistry`'s registrar-confirmed service attachment is the authoritative record of which
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
/// which matters because a repointable factory reference would let one transaction redefine what counts
/// as a genuine clone — for every consumer that resolves provenance through it, at once. Here there is
/// nothing to repoint, in either direction, which is also why both dependencies are checked at
/// construction: a wrong one is remedied only by deploying a new factory.
contract DogTagIssuerFactory {
    address public immutable implementation;
    /// @notice The authority core — `ProviderRegistry`. See [`IProviderAuthority`] for the four
    /// functions this factory and its clones require of it, all four permanent.
    address public immutable registry;

    mapping(address => bool) public isClone; // deployed by this factory
    /// @notice R -> issuing clone, write-once. This is the protocol's root index: it is the address
    /// `VerificationRegistryConsent.rootIndex` names, and the mapping every verifier resolves a
    /// credential's issuing clone through. A root can only ever be written here by one of this
    /// factory's own clones (see {registerRoot}), and never overwritten.
    mapping(bytes32 => address) public rootIssuer;

    /// @notice owner => recordType => the clone that owner most recently designated as live. RAW: this
    /// is the CURRENT stored value, not a history — the mapping holds one address per slot and a repoint
    /// overwrites it, so the audit trail is {ActiveIssuerSet}/{ActiveIssuerCleared}, never this read. It
    /// is also not re-validated here; use [`resolveActiveIssuer`] unless you specifically want the raw
    /// designation.
    mapping(address => mapping(bytes32 => address)) public activeIssuer;

    /// @dev The shape the oversight indexer's decoder is written against. `name` is always the empty
    /// string — clones
    /// have no settable name (see `DogTagIssuer`) — so the field is carried to preserve the signature
    /// and topic0, not to convey a value.
    event IssuerCreated(address indexed clone, bytes32 indexed recordType, string name);
    /// @dev Emitted from the factory (not only as the clone's own
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
    /// @dev `impl` is code, but not the exact `DogTagIssuer` runtime this factory was compiled for.
    error ImplementationCodeMismatch(address impl);
    /// @dev The core did not ANSWER `selector`: it reverted, returned nothing, returned the wrong width,
    /// or returned a word that is not a canonical boolean. Distinct from
    /// {AuthorityAuthorizesUnconditionally}, which is a definite `true` - a broken authorization rule and
    /// a missing selector have different remedies, and naming one as the other sends an operator after
    /// the wrong dependency.
    error AuthorityDoesNotAnswer(address authority, bytes4 selector);
    /// @dev The core answered a canonical `true` to a zero-everything query on `selector`. It authorizes
    /// indiscriminately, so every creation and every anchor would pass.
    error AuthorityAuthorizesUnconditionally(address authority, bytes4 selector);

    /// @param impl the `DogTagIssuer` implementation clones delegate to.
    /// @param registry_ the authority core — see [`IProviderAuthority`].
    ///
    /// @dev Both dependencies are `immutable` and this contract has no admin, so a wrong one is
    /// remedied only by deploying a new factory — and that means a new verification registry too,
    /// since its `rootIndex` names this factory and is immutable as well. Each is therefore checked
    /// for code and then either exact identity (the implementation) or required ABI behaviour (the
    /// authority) before it is stored, so the ways a wrong address stays silent are closed here rather
    /// than discovered in production: an EOA (a staticcall to one SUCCEEDS, with empty returndata), a
    /// contract that does not answer a required selector, one that answers the wrong width, and an
    /// implementation-shaped impostor.
    ///
    /// The implementation check is exact: `impl.codehash` must equal the hash of
    /// `type(DogTagIssuer).runtimeCode` compiled into this factory. Getter-shaped code, including a
    /// fallback that returns one word for every selector, is not sufficient. This pins every clone to
    /// the precise implementation whose owner and record-type reads the factory relies on.
    ///
    /// The core is additionally required to answer `false` to zero-everything queries, which refuses
    /// one that authorizes indiscriminately.
    ///
    /// **A dependency that did not answer and one that answered a definite `true` are refused under
    /// SEPARATE errors**, each naming the dependency and the probed selector. Both are equally
    /// fail-closed; what differs is the remedy. A single error for both told an operator to go looking
    /// for a missing selector when the real cause was an authorization rule that authorizes everything,
    /// which is this codebase's could-not-check-rendered-as-a-neighbouring-state defect, inverted.
    constructor(address impl, address registry_) {
        if (impl == address(0) || registry_ == address(0)) {
            revert ZeroAddress();
        }

        _requireCode(impl);
        if (impl.codehash != keccak256(type(DogTagIssuer).runtimeCode)) {
            revert ImplementationCodeMismatch(impl);
        }

        _requireAuthorityAnswers(registry_);

        implementation = impl;
        registry = registry_;
    }

    function _requireCode(address dependency) private view {
        if (dependency.code.length == 0) revert NotAContract(dependency);
    }

    /// @dev All four functions of [`IProviderAuthority`], each required to answer `false` to a
    /// zero-everything query. A core missing any one of them cannot be repointed away from later.
    ///
    /// The probe order is `canCreateService`, `canIssue`, `canRevoke`, `hasRole`, and it is stable on
    /// purpose: the reported selector is the FIRST one that failed, so a contract implementing none of
    /// them -
    /// which answers `hasRole` and none of the three capability questions - is named for the capability
    /// selector it lacks rather than for a later one it happens to share.
    function _requireAuthorityAnswers(address authority_) private view {
        _requireCode(authority_);
        _requireAuthorityFalse(
            authority_,
            IProviderAuthority.canCreateService.selector,
            abi.encodeCall(IProviderAuthority.canCreateService, (bytes20(0), bytes32(0), address(0)))
        );
        _requireAuthorityFalse(
            authority_,
            IProviderAuthority.canIssue.selector,
            abi.encodeCall(IProviderAuthority.canIssue, (address(0), address(0)))
        );
        _requireAuthorityFalse(
            authority_,
            IProviderAuthority.canRevoke.selector,
            abi.encodeCall(IProviderAuthority.canRevoke, (address(0), address(0)))
        );
        _requireAuthorityFalse(
            authority_,
            IProviderAuthority.hasRole.selector,
            abi.encodeCall(IProviderAuthority.hasRole, (bytes32(0), address(0)))
        );
    }

    function _requireAuthorityFalse(address authority_, bytes4 selector, bytes memory call_) private view {
        Answer answer = _probe(authority_, call_);
        if (answer == Answer.True) revert AuthorityAuthorizesUnconditionally(authority_, selector);
        if (answer != Answer.False) revert AuthorityDoesNotAnswer(authority_, selector);
    }

    /// @dev What a one-word boolean probe established. `NoAnswer` covers every shape in which the target
    /// failed to state a boolean at all - a revert, no code path for the selector, the wrong returndata
    /// width, and a word that is neither 0 nor 1.
    enum Answer {
        NoAnswer,
        False,
        True
    }

    /// @dev Decoded as `uint256` rather than `bool` on purpose: a non-conforming word above 1 makes
    /// `bool` decoding panic, which would replace the named revert with an opaque one. Such a word is
    /// reported as `NoAnswer` rather than as `True`, so a malformed reply can never be read as a definite
    /// authorization - the caller refuses it either way, and only the diagnostic differs.
    function _probe(address target, bytes memory call_) private view returns (Answer) {
        (bool ok, bytes memory ret) = target.staticcall(call_);
        if (!ok || ret.length != 32) return Answer.NoAnswer;
        uint256 word = abi.decode(ret, (uint256));
        if (word == 0) return Answer.False;
        if (word == 1) return Answer.True;
        return Answer.NoAnswer;
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
        DogTagIssuer(clone).initialize(recordType, registry, address(this), msg.sender);
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
    /// caller does not choose the address this dispatches to, so a consumer (the core's attachment, the
    /// domain resolver's
    /// domain read) treats it as a cheap resolve and would be surprised by a revert where it expected an
    /// address.
    ///
    /// The only address reachable here is one already in `isClone`, which `createIssuer` only ever fills
    /// with `Clones.cloneDeterministic(implementation, ...)` — and `implementation` is `immutable` AND
    /// was required at construction to be byte-for-byte the `DogTagIssuer` runtime compiled into this
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
    // The write-once root index
    // ---------------------------------------------------------------------------------------------

    /// @notice Write-once registration of `root -> issuing clone` (§11.10(a), audit-11 V4-C1/M1).
    ///
    /// @dev Two guards, and both are load-bearing. `isClone[msg.sender]` is what makes {rootIssuer}
    /// meaningful as provenance at all: only a contract this factory deployed can ever name itself as a
    /// root's issuer, so an arbitrary contract cannot insert itself into the index and be resolved as a
    /// genuine issuing clone by every verifier downstream.
    ///
    /// `rootIssuer[root] == address(0)` is strictly write-once and is what makes a REVOCATION binding.
    /// The verification registry resolves a proof's clone through this mapping and then re-reads
    /// `isValid(root)` on it; if a root could be re-registered against a second clone, a credential
    /// revoked on the clone that issued it would resolve to the new one and verify again. Nothing here
    /// may ever be relaxed into an overwrite, not even by the clone that holds the entry.
    function registerRoot(bytes32 root) external {
        require(isClone[msg.sender], "!clone");
        require(rootIssuer[root] == address(0), "root taken"); // strictly write-once
        rootIssuer[root] = msg.sender;
        emit RootRegistered(root, msg.sender);
    }
}
