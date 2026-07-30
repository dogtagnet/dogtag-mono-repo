// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {CloneProvenanceRouter} from "./CloneProvenanceRouter.sol";
import {ProviderRegistry} from "./ProviderRegistry.sol";

/// @dev The authority core's auto-generated getter for its content-write permission bit. Declared as an
/// interface only because Solidity does not expose another contract's `public constant` as an addressable
/// function — the VALUE is still resolved from the core at deployment rather than restated here, so this
/// mirrors a signature and never a fact. `ServiceDomainResolverTest` pins the signature against the REAL
/// core, so a rename cannot survive as a silent non-answer.
interface ICoreRecordPermission {
    function SERVICE_PERMISSION_RECORD() external view returns (uint32);
}

/// @title ServiceDomainResolver — the domain claim a service contract publishes, and its typed absence.
///
/// @notice Supersedes `IssuerDomainRegistry`. Same product fact — "this service contract asserts that its
/// domain is D, and the zone at D asserts this contract back" — with the authority question moved to the
/// S-6 authority core, the provenance question moved to the S-8 cross-generation router, and the three
/// different meanings of "no domain" given three different names.
///
/// # Absence is THREE facts, not an empty string
///
/// `IssuerDomainRegistry` had exactly one way to say "no domain": an empty `domain` string, reached both
/// by a clone nobody had ever written and by a clone whose claim had been withdrawn. Those are different
/// facts with different remedies, and a third — "this provider has deliberately published no domain" —
/// could not be said at all, so a provider with no website was indistinguishable from one whose record
/// had never been filled in.
///
///   * `UNSET` — nobody has written anything for this service. The day-one state.
///   * `NO_DOMAIN` — a writer deliberately stated that this service publishes no domain.
///   * `CLAIMED` — a domain is claimed right now, and {DomainRecord-domain} carries it.
///   * `CLEARED` — a claim existed and was withdrawn.
///
/// `UNSET` is the zero value, so an unwritten record reads as "nobody has said" without a write, and
/// {Disposition} can never be absent from an answer the way an empty string could be mistaken for one.
///
/// Two invariants make the string incapable of lying regardless of which branch a careless reader takes,
/// both pinned by tests:
///
///   * `domain` is non-empty **if and only if** `disposition == CLAIMED`. `NO_DOMAIN` and `CLEARED` zero
///     it, and {_requireValidDomain} makes an empty claim unrepresentable. A withdrawn domain therefore
///     cannot be read back as a live one — its value survives in {DomainClaimWithdrawn} instead, where it
///     is legible as history and not mistakable for a current claim.
///   * `disposition == UNSET` **if and only if** `revision == 0` and `updatedAt == 0`. So "never written"
///     is decidable from any one of the three fields, and a written record always carries a real stamp.
///
/// {clearDomain} additionally refuses to run unless a claim actually exists, because `CLEARED` asserts
/// that one was withdrawn. Allowing it from `UNSET` would let the typed disposition state a withdrawal
/// that never happened, which is the same class of false claim the type exists to remove.
///
/// # Who may write: the captain's AND, composed and not re-derived
///
/// One call decides authorization: `core.canWriteService(service, caller, SERVICE_PERMISSION_RECORD)`.
/// That is the authority core's own published two-predicate rule — the attached provider and service are
/// currently cleared, **AND** the caller is the confirmed live clone owner or an owner-epoch-scoped
/// delegate. Composing it rather than re-deriving standing and ownership here is deliberate: two parallel
/// implementations of one authorization rule is how the vet and mobile verdict paths came to disagree in
/// this codebase already.
///
/// The three tiers `IssuerDomainRegistry` carried are all gone, and each for its own reason. Its tier 2
/// recomputed the creation salt because generation-1 clones have no owner to ask — generation-2 clones do,
/// so the stand-in is unnecessary. Its tier 3 `domainAdmin` existed only to rescue clones whose salted
/// `business` was the operator's own signer; owner-epoch-scoped delegation in the core replaces it with a
/// revocable grant that a clone-owner rotation invalidates automatically. And its tier 1 was a
/// `WHITELIST_ADMIN` write bypass: the core deliberately gives an issuance signer, a provider controller
/// and the registrar **no** ordinary content-write bypass, so this contract has no registrar write path at
/// all. A registrar that needs a claim gone reaches for the fleet-wide lever below, not for the record.
///
/// **`authorizeClone` is deliberately NOT composed, and this is a considered deviation** from the handoff
/// note in `docs/ISSUER_V2_OWNERSHIP.md` §8, which names this contract as an intended consumer. Two
/// reasons, either sufficient. It requires `claimant == owner()` exactly, so composing it as the control
/// term would make every owner-appointed delegate unable to publish a domain and would leave the core's
/// `SERVICE_PERMISSION_RECORD` bit with no consumer — a functional loss, not a tightening. And it lives on
/// a generation-specific factory, so reaching it means either pinning this resolver to one generation or
/// hopping through the core's generation record to find the right factory. What that note is actually
/// protecting is that the rule lives in ONE place; composing the core's `canWriteService` satisfies that,
/// and reconciling the core's own derivation with `authorizeClone` is recorded there as S-6's obligation.
///
/// # Provenance: the router, and why the core's own check does not cover it
///
/// Every write also requires `router.isClone(service)` — the S-8 {CloneProvenanceRouter}, the union across
/// factory generations. Without it a stranger's hand-rolled contract could claim a domain, publish a
/// matching TXT record, and present as verified with nothing behind it.
///
/// This is **not** redundant with the core's own provenance check, and the difference is the whole point.
/// `canWriteService` proves clone-hood against the factory pinned to the generation the service was
/// attached under; the router proves it against the generation list that the verification registry's
/// immutable `rootIndex` actually resolves roots through. Those two lists are administered by two separate
/// owner-only calls on two separate contracts (`ProviderRegistry.addFactoryGeneration` and
/// `CloneProvenanceRouter.appendGeneration`), so they can genuinely disagree. A service attached under a
/// core generation whose factory was never appended to the router is one whose credentials answer
/// `unknown root` at every verification — and without this check it could still publish a domain, so a
/// verified-looking identity would sit beside credentials that cannot verify at all. Requiring both makes
/// that unrepresentable: a domain claim is honoured only for a service whose lineage is the same lineage
/// that resolves its roots.
///
/// # The resolver must still be selected AND still be approved
///
/// The core stores a service's selected domain resolver but never clears it — not on resolver deapproval,
/// and not on the generation deprecation that freezes the selection write permanently. The stored selector
/// is therefore a historical record of the last selection the core accepted, and a consumer computing an
/// EFFECTIVE resolver must require both halves. So must this contract, on every write:
///
///   * `core.service(service).domainResolver == address(this)` — this service selected *us*, so a second
///     approved resolver cannot write records for a service that never chose it; and
///   * `core.isResolverApproved(DOMAIN, address(this))` — the registry authority's fleet-wide
///     `setResolverApproved(DOMAIN, resolver, false)` lever still permits us. That lever is the only way
///     to disable a resolver for services whose selection write is already frozen, so reading the
///     selector alone would defeat it.
///
/// Both are re-read on the read side too, by {isAuthoritativeFor} and {claimStanding}, because a stored
/// record outlives the permission that wrote it.
///
/// # A frozen service, and why standing is a FOURTH term rather than a fourth conjunct
///
/// Two core lifecycle states are terminal. A `RETIRED` service standing cannot be left, and a deprecated
/// factory generation has no reactivation path. Either one makes the core's `canWriteService` false
/// forever, so every write here reverts {NotAuthorized} permanently - for the owner, for every delegate,
/// and for the registrar, which has no bypass by design. A `CLAIMED` record on such a service is frozen
/// as history.
///
/// None of the three terms above notices: the router's generation list is append-only, fleet approval is
/// unrelated, and the core never clears a stored selector. So {claimStanding} reports the core's own
/// standing as a SEPARATE fourth term, sourced from `core.effectiveService` rather than re-derived here.
/// It is deliberately NOT folded into {isAuthoritativeFor}, which answers whether THIS RESOLVER's record
/// is the effective one for the service - and a retired service's record genuinely is still the last
/// thing this resolver accepted from it. Two questions with different remedies must not share one bool.
///
/// # What this contract deliberately does not hold
///
/// **No name, no legal identity.** A generation-2 clone's `name()` is permanently empty by construction,
/// and a provider-chosen string beside a green check is a fabricated authority. Registrar-controlled
/// identity comes from the core's `publicIdentityAnchor`; never add an identity field here.
///
/// **No description.** The human-readable text is the DNS record's own value, written by whoever controls
/// the zone. Keeping it off-chain is what makes the binding bidirectional rather than two copies of one
/// party's claim.
///
/// **No DNS state.** A stored record is a CLAIM. The zone's half is observed off-chain, cannot be
/// observed historically at all, and must never be cached here as if the chain had witnessed it —
/// see `docs/ISSUER_DOMAIN_BINDING.md`.
///
/// # The record convention is unchanged
///
/// The zone half is still `<service-address-lowercase>._dogtag.<domain> IN TXT "<description>"`. This
/// contract is keyed by the service contract exactly as its predecessor was keyed by the clone, so the
/// off-chain resolvers in `dogtag-dns-rs`, Swift and Kotlin need no grammar change. Their **read** does
/// change: there is deliberately no getter whose primary return is a bare domain string, so a consumer
/// migrating here must branch on {Disposition} rather than on `""`.
contract ServiceDomainResolver {
    /// @notice Why a service has, or has not, a domain. `UNSET` is the zero value; see the contract doc.
    enum Disposition {
        UNSET,
        NO_DOMAIN,
        CLAIMED,
        CLEARED
    }

    /// @notice A service's published domain disposition.
    ///
    /// @dev Both a timestamp and a block number are stored and they answer different questions. The
    /// timestamp is what a human reads; the block is what makes the claim REPRODUCIBLE. Because DNS
    /// changes over time and a service may be superseded, "this contract claims moh.gov.sg" is only
    /// meaningful with a block anchor: with `updatedAtBlock` a verifier can ask what a service claimed at
    /// block N via a pinned `eth_call`, rather than only what it claims now.
    ///
    /// `revision` counts writes, so a re-affirmation of the same domain — which legitimately re-anchors
    /// the block for a daily DNS re-check — is distinguishable from no write at all.
    struct DomainRecord {
        Disposition disposition;
        address setBy;
        uint64 updatedAt;
        uint64 updatedAtBlock;
        uint64 revision;
        string domain;
    }

    /// @notice Longest DNS name this contract will store, per RFC 1035.
    uint256 public constant MAX_DOMAIN_LENGTH = 253;

    /// @notice Upper bound on {recordedServicePage}, mirroring the authority core's own page cap.
    uint256 public constant MAX_PAGE_SIZE = 100;

    /// @dev Words `ProviderRegistry.effectiveService` answers with: two standings and three flags, each
    /// ABI-padded to a word. Used only by the constructor probe, which asserts the width rather than the
    /// values.
    uint256 private constant EFFECTIVE_SERVICE_RETURN_WORDS = 5;

    /// @notice The S-6 authority core. Answers authorization, resolver selection and resolver approval.
    /// @dev Immutable. The core holds the standing, ownership and delegation facts a domain write depends
    /// on; a settable reference would let one transaction redefine who may publish an identity.
    ProviderRegistry public immutable core;

    /// @notice The S-8 cross-generation provenance router — the same lineage the verification registry's
    /// immutable `rootIndex` resolves roots through.
    /// @dev Immutable for the same reason. Its generation list is append-only, so the lineage can grow
    /// without this reference moving.
    CloneProvenanceRouter public immutable router;

    /// @notice The core's content-write permission bit, RESOLVED FROM THE CORE at deployment.
    ///
    /// @dev Read from the chain rather than restated as a local constant, which is this codebase's
    /// standing rule for any value another contract owns — a duplicated bit is a second source of truth
    /// that drifts silently, and a permission mask that has drifted refuses every legitimate write while
    /// looking like an authorization problem.
    ///
    /// `SERVICE_PERMISSION_RECORD` is the content-write bit, deliberately NOT
    /// `SERVICE_PERMISSION_DOMAIN_RESOLVER`. Those are different powers: choosing which resolver holds a
    /// service's domain is the owner's structural decision, while publishing the domain through the
    /// chosen resolver is content. An operations key trusted to keep a domain claim current is not
    /// thereby trusted to move the record to a different resolver, and reusing one bit for both would
    /// grant exactly that.
    uint32 public immutable recordPermission;

    mapping(address => DomainRecord) private _records;

    /// @dev Every service that has ever had a record written, for enumeration by the off-chain DNS
    /// re-check job. A withdrawn claim stays listed — its record becomes `CLEARED` rather than being
    /// removed — so the job can still observe the withdrawal.
    address[] private _recordedServices;
    mapping(address => bool) private _listed;

    event DomainClaimed(
        address indexed service,
        string domain,
        Disposition previous,
        address indexed setBy,
        uint64 revision,
        uint64 atBlock
    );
    event NoDomainDeclared(
        address indexed service, Disposition previous, address indexed setBy, uint64 revision, uint64 atBlock
    );
    /// @dev Carries the domain that was withdrawn. State zeroes it so it cannot be read back as current;
    /// the history it would otherwise lose lives here.
    event DomainClaimWithdrawn(
        address indexed service,
        string withdrawnDomain,
        address indexed setBy,
        uint64 revision,
        uint64 atBlock
    );

    error ZeroAddress();
    error DependencyHasNoCode(address dependency);
    error DependencyDoesNotAnswer(address dependency, bytes4 selector);
    error RouterRecognizesEveryAddress(address router);
    error CorePermissionIsZero(address core);
    /// @dev The router's generation list does not vouch for this address. Deliberately distinct from
    /// {NotAuthorized}: a provider whose contract is genuine must never be told its address is a forgery,
    /// and a stranger's contract must never be told it merely lacks a permission.
    error NotRecognizedByLineage(address service);
    error ResolverNotApproved(address resolver);
    error ResolverNotSelected(address service, address resolver);
    error NotAuthorized(address service, address caller);
    error BadDomain();
    error NothingToWithdraw(address service, Disposition disposition);
    error AlreadyDeclaredNoDomain(address service);
    error BadPage();

    /// @param core_ the S-6 `ProviderRegistry`.
    /// @param router_ the S-8 `CloneProvenanceRouter`.
    /// @dev Both are behaviour-probed rather than merely code-checked, because an EOA staticcall SUCCEEDS
    /// with empty returndata — the silent shape a code-size check alone would miss. The probes are
    /// TRI-state on purpose and their diagnostics stay split: {DependencyDoesNotAnswer} means nothing was
    /// stated (a revert, no such selector, or a word of the wrong width), while
    /// {RouterRecognizesEveryAddress} is a definite `true` from a stub that authorizes everything.
    /// Collapsing them would send an operator hunting a missing selector when the real cause is a
    /// predicate that recognizes any address handed to it.
    constructor(address core_, address router_) {
        if (core_ == address(0) || router_ == address(0)) revert ZeroAddress();
        _requireCode(core_);
        _requireCode(router_);

        // A router that vouches for the zero address vouches for anything, which would make the
        // provenance term above pass for a hand-rolled contract.
        bool vouchesForNothing = _probeBool(
            router_,
            abi.encodeCall(CloneProvenanceRouter.isClone, (address(0))),
            CloneProvenanceRouter.isClone.selector
        );
        if (vouchesForNothing) revert RouterRecognizesEveryAddress(router_);

        // Every core read this contract depends on must answer with the expected width, so a core that
        // cannot serve one is refused here rather than at the first call that needs it. `canWriteService`
        // with a zero permission is `false` by the core's own guard, so this probes the ABI without
        // asserting anything about a real service.
        _probeBool(
            core_,
            abi.encodeCall(
                ProviderRegistry.isResolverApproved, (ProviderRegistry.ResolverKind.DOMAIN, address(0))
            ),
            ProviderRegistry.isResolverApproved.selector
        );
        _probeBool(
            core_,
            abi.encodeCall(ProviderRegistry.canWriteService, (address(0), address(0), uint32(0))),
            ProviderRegistry.canWriteService.selector
        );
        _probeWords(
            core_,
            abi.encodeCall(ProviderRegistry.service, (address(0))),
            ProviderRegistry.service.selector,
            1
        );
        _probeWords(
            core_,
            abi.encodeCall(ProviderRegistry.effectiveService, (address(0))),
            ProviderRegistry.effectiveService.selector,
            EFFECTIVE_SERVICE_RETURN_WORDS
        );

        core = ProviderRegistry(core_);
        router = CloneProvenanceRouter(router_);
        recordPermission = _readRecordPermission(core_);
    }

    // ---------------------------------------------------------------------------------------------
    // Writes
    // ---------------------------------------------------------------------------------------------

    /// @notice Publish or re-affirm `service`'s domain claim.
    ///
    /// @dev `domain` must already be canonical lowercase — this contract rejects rather than silently
    /// normalizes, so what a resolver reads is exactly what the writer intended. Re-claiming the SAME
    /// domain is permitted and is not a no-op: it re-anchors `updatedAtBlock` and bumps `revision`, which
    /// is how an issuer re-affirms a claim for the off-chain re-check job.
    function claimDomain(address service, string calldata domain) external {
        _requireWritable(service);
        _requireValidDomain(domain);

        DomainRecord storage r = _records[service];
        Disposition previous = r.disposition;
        r.disposition = Disposition.CLAIMED;
        r.domain = domain;
        uint64 revision = _stamp(r);
        _list(service);
        emit DomainClaimed(service, domain, previous, msg.sender, revision, uint64(block.number));
    }

    /// @notice State that `service` deliberately publishes no domain.
    ///
    /// @dev This is a positive assertion, not an absence, which is why it is a write and not simply
    /// leaving the record `UNSET`. Reachable from `CLAIMED` directly: an issuer that has dropped its
    /// domain entirely is making the stronger statement, and `NO_DOMAIN` says more than `CLEARED` does.
    /// The withdrawn value is still emitted by {DomainClaimWithdrawn}'s sibling {DomainClaimed} history,
    /// so nothing is lost to an indexer.
    function declareNoDomain(address service) external {
        _requireWritable(service);

        DomainRecord storage r = _records[service];
        Disposition previous = r.disposition;
        if (previous == Disposition.NO_DOMAIN) revert AlreadyDeclaredNoDomain(service);
        r.disposition = Disposition.NO_DOMAIN;
        delete r.domain;
        uint64 revision = _stamp(r);
        _list(service);
        emit NoDomainDeclared(service, previous, msg.sender, revision, uint64(block.number));
    }

    /// @notice Withdraw `service`'s domain claim.
    ///
    /// @dev Refuses unless a claim actually exists, because `CLEARED` asserts that one was withdrawn.
    /// From `UNSET` there is nothing to withdraw and recording a withdrawal would be a false statement;
    /// from `NO_DOMAIN` there is no claim either; from `CLEARED` it already happened. A caller who wants
    /// "there is no domain here" without a prior claim wants {declareNoDomain}.
    function clearDomain(address service) external {
        _requireWritable(service);

        DomainRecord storage r = _records[service];
        if (r.disposition != Disposition.CLAIMED) revert NothingToWithdraw(service, r.disposition);
        string memory withdrawn = r.domain;
        r.disposition = Disposition.CLEARED;
        delete r.domain;
        uint64 revision = _stamp(r);
        emit DomainClaimWithdrawn(service, withdrawn, msg.sender, revision, uint64(block.number));
    }

    // ---------------------------------------------------------------------------------------------
    // The write predicate
    // ---------------------------------------------------------------------------------------------

    /// @notice True iff `who` may write `service`'s record right now — every term a write requires.
    ///
    /// @dev Exposed so a console can render "can this key self-serve?" without simulating a write.
    /// COMPOSED from {isAuthoritativeFor} rather than re-listing its three terms, so the standing half of
    /// this answer has exactly one derivation and a term added there cannot leave this over-permissive.
    /// The core's own standing is deliberately not AND-ed in beside it: `canWriteService` already folds
    /// it, and counting it twice would state one fact as two terms.
    ///
    /// A `false` here means the write would be refused, which is accurate whichever term failed. Note the
    /// core's `canWriteService` itself fails closed on an unreadable dependency, so `false` from that term
    /// folds "not permitted" together with "a dependency could not be read" — for a permission gate those
    /// have the same consequence.
    ///
    /// {claimStanding} is the surface that keeps the display-facing terms apart, and it is what a console
    /// renders beside this. Even so, `canWriteService` is ONE bool, so a bare refusal still cannot
    /// distinguish "your key lacks authority" from "your provider is suspended" from "your service is
    /// retired". {claimStanding}'s `serviceStandingEffective` is what separates the last of those out.
    function canWriteDomain(address service, address who) public view returns (bool) {
        return isAuthoritativeFor(service) && core.canWriteService(service, who, recordPermission);
    }

    /// @dev Ordered so the diagnostics are useful: lineage first (the strongest and least recoverable
    /// statement), then the two resolver-standing terms an operator fixes in the core, then the caller's
    /// own authority. Each has its own error; see {NotRecognizedByLineage} for why the first two classes
    /// must not be collapsed.
    function _requireWritable(address service) internal view {
        if (service == address(0)) revert ZeroAddress();
        if (!_lineageRecognizes(service)) revert NotRecognizedByLineage(service);
        if (!_resolverApproved()) revert ResolverNotApproved(address(this));
        if (!_resolverSelected(service)) revert ResolverNotSelected(service, address(this));
        if (!core.canWriteService(service, msg.sender, recordPermission)) {
            revert NotAuthorized(service, msg.sender);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Reads
    // ---------------------------------------------------------------------------------------------

    /// @notice `service`'s record. Does NOT revert on a service nobody has written: `UNSET` is a normal
    /// day-one state, not an error.
    function record(address service) external view returns (DomainRecord memory) {
        return _records[service];
    }

    /// @notice The cheap discriminator, for a caller that only needs to know which of the four facts holds.
    function dispositionOf(address service) external view returns (Disposition) {
        return _records[service].disposition;
    }

    /// @notice The claim, with its disposition first.
    ///
    /// @dev Deliberately a tuple rather than a bare `string`. `IssuerDomainRegistry.domainOf` returned
    /// `""` for three different facts, and a caller could consume it without ever learning which — the
    /// overload this contract exists to remove. A caller that wants only the string must write
    /// `(, string memory d) = resolveDomain(s)`, which discards the disposition visibly.
    function resolveDomain(address service) external view returns (Disposition, string memory) {
        DomainRecord storage r = _records[service];
        return (r.disposition, r.domain);
    }

    /// @notice Whether this resolver's record for `service` is the currently effective one.
    ///
    /// @dev THE machine answer, and the single derivation of the AND, so consumers cannot drift into
    /// three slightly different versions of it. It says nothing about the record's CONTENT: a `true` here
    /// beside a `NO_DOMAIN` disposition means "this resolver authoritatively reports no domain".
    ///
    /// It also says nothing about whether the SERVICE is still standing. A retired service's record is
    /// genuinely still the last thing this resolver accepted from it, so this stays `true` - see the
    /// frozen-service section of the contract doc, and read `serviceStandingEffective` beside it.
    ///
    /// Render {claimStanding}'s terms to a human, not this. Collapsing separate facts into one verdict is
    /// what makes a display unable to say which of them failed.
    function isAuthoritativeFor(address service) public view returns (bool) {
        return _lineageRecognizes(service) && _resolverApproved() && _resolverSelected(service);
    }

    /// @notice The record, the three standing terms behind {isAuthoritativeFor}, and the core's own
    /// service standing - all reported SEPARATELY.
    ///
    /// @dev The first three are composed from the same helpers {isAuthoritativeFor} folds rather than
    /// re-derived here, so the breakdown and the verdict cannot disagree.
    ///
    /// `serviceStandingEffective` is the FOURTH term and is deliberately outside that verdict. It comes
    /// from `core.effectiveService` rather than being re-derived, and answers whether the core's write
    /// predicate can still be satisfied for this service at all. A `false` is terminal whenever its cause
    /// is a `RETIRED` standing or a deprecated factory generation, and a `CLAIMED` record beside it is
    /// history rather than a live claim - so a consumer MUST read it before rendering a claim as current.
    ///
    /// Each term is a real observation or the call reverts — nothing here catches a failed read and
    /// returns `false` for it, because a swallowed failure rendered as a definite negative is exactly how
    /// "could not check" turns into "not verified". A consumer whose call reverts has learned that it
    /// could not check, which is the honest answer.
    function claimStanding(address service)
        external
        view
        returns (
            Disposition disposition,
            string memory domain,
            bool lineageRecognizesService,
            bool registryApprovesThisResolver,
            bool coreSelectsThisResolver,
            bool serviceStandingEffective
        )
    {
        DomainRecord storage r = _records[service];
        disposition = r.disposition;
        domain = r.domain;
        lineageRecognizesService = _lineageRecognizes(service);
        registryApprovesThisResolver = _resolverApproved();
        coreSelectsThisResolver = _resolverSelected(service);
        serviceStandingEffective = _serviceStandingIsEffective(service);
    }

    /// @notice How many services have ever had a record written here.
    function recordedServiceCount() external view returns (uint256) {
        return _recordedServices.length;
    }

    /// @notice A bounded page of the services that have ever had a record written, oldest first.
    /// @dev Bounded rather than returning the whole array, so enumeration cost cannot grow without limit
    /// as the fleet does. Mirrors the authority core's paging shape.
    function recordedServicePage(uint256 cursor, uint256 limit) external view returns (address[] memory) {
        uint256 length = _recordedServices.length;
        if (limit == 0 || limit > MAX_PAGE_SIZE || cursor > length) revert BadPage();
        uint256 end = limit > length - cursor ? length : cursor + limit;
        address[] memory page = new address[](end - cursor);
        for (uint256 i = cursor; i < end; i++) {
            page[i - cursor] = _recordedServices[i];
        }
        return page;
    }

    // ---------------------------------------------------------------------------------------------
    // Internals
    // ---------------------------------------------------------------------------------------------

    function _lineageRecognizes(address service) internal view returns (bool) {
        return router.isClone(service);
    }

    function _resolverApproved() internal view returns (bool) {
        return core.isResolverApproved(ProviderRegistry.ResolverKind.DOMAIN, address(this));
    }

    function _resolverSelected(address service) internal view returns (bool) {
        return core.service(service).domainResolver == address(this);
    }

    /// @dev The core's own standing axis, read from `effectiveService` rather than re-derived from the
    /// provider, service and generation records separately - a second derivation of a rule the core
    /// already publishes is exactly what this contract composes `canWriteService` to avoid.
    ///
    /// The two remaining fields (`ownerConfirmed`, `hasActiveIssuer`) are deliberately discarded: this
    /// term is about the SERVICE's standing, and folding in who currently owns it or whether it can still
    /// issue would answer a different question under one name.
    function _serviceStandingIsEffective(address service) internal view returns (bool) {
        (
            ProviderRegistry.Standing providerStanding,
            ProviderRegistry.Standing serviceStanding,
            bool factoryActive,,
        ) = core.effectiveService(service);
        return providerStanding == ProviderRegistry.Standing.ACTIVE
            && serviceStanding == ProviderRegistry.Standing.ACTIVE && factoryActive;
    }

    /// @dev The one place a write's stamp is applied, so `revision`, `updatedAt` and `updatedAtBlock` can
    /// never be advanced by one path and not another — which is what keeps
    /// `disposition == UNSET <=> revision == 0 <=> updatedAt == 0` true.
    function _stamp(DomainRecord storage r) internal returns (uint64 revision) {
        r.setBy = msg.sender;
        r.updatedAt = uint64(block.timestamp);
        r.updatedAtBlock = uint64(block.number);
        revision = r.revision + 1;
        r.revision = revision;
    }

    function _list(address service) internal {
        if (!_listed[service]) {
            _listed[service] = true;
            _recordedServices.push(service);
        }
    }

    function _requireCode(address dependency) internal view {
        if (dependency.code.length == 0) revert DependencyHasNoCode(dependency);
    }

    /// @dev Staticcalls `data` and requires a boolean-width answer. Returns the answer so a caller can
    /// additionally refuse a definite `true` from a stub, keeping "did not answer" and "answered yes to
    /// everything" as two diagnoses rather than one.
    function _probeBool(address dependency, bytes memory data, bytes4 selector)
        internal
        view
        returns (bool answer)
    {
        (bool ok, bytes memory ret) = dependency.staticcall(data);
        if (!ok || ret.length != 32) revert DependencyDoesNotAnswer(dependency, selector);
        uint256 word;
        assembly ("memory-safe") {
            word := mload(add(ret, 0x20))
        }
        if (word > 1) revert DependencyDoesNotAnswer(dependency, selector);
        return word == 1;
    }

    /// @dev The permission mask is resolved from the core, and a definite ZERO is refused separately from
    /// a non-answer. `canWriteService` returns `false` for `permission == 0` by its own guard, so a zero
    /// mask would make every write here refuse with {NotAuthorized} — an authorization diagnostic for a
    /// wiring fault, on a reference that is immutable and therefore unrepairable.
    function _readRecordPermission(address dependency) internal view returns (uint32) {
        (bool ok, bytes memory ret) =
            dependency.staticcall(abi.encodeCall(ICoreRecordPermission.SERVICE_PERMISSION_RECORD, ()));
        if (!ok || ret.length != 32) {
            revert DependencyDoesNotAnswer(
                dependency, ICoreRecordPermission.SERVICE_PERMISSION_RECORD.selector
            );
        }
        uint256 word;
        assembly ("memory-safe") {
            word := mload(add(ret, 0x20))
        }
        if (word == 0) revert CorePermissionIsZero(dependency);
        if (word > type(uint32).max) {
            revert DependencyDoesNotAnswer(
                dependency, ICoreRecordPermission.SERVICE_PERMISSION_RECORD.selector
            );
        }
        return uint32(word);
    }

    /// @dev The wider reads answer with several words rather than one, so they are probed for a decodable
    /// answer of at least `minWords` instead of a boolean one. An unattached address answers a zeroed
    /// struct and an all-zero standing tuple, which is exactly what {_resolverSelected} and
    /// {_serviceStandingIsEffective} need them to do, so the probe asserts the WIDTH and never a value.
    function _probeWords(address dependency, bytes memory data, bytes4 selector, uint256 minWords)
        internal
        view
    {
        (bool ok, bytes memory ret) = dependency.staticcall(data);
        if (!ok || ret.length < minWords * 32) revert DependencyDoesNotAnswer(dependency, selector);
    }

    // ---------------------------------------------------------------------------------------------
    // Domain validation — the grammar `IssuerDomainRegistry` established, preserved verbatim
    // ---------------------------------------------------------------------------------------------

    /// @dev Rejects anything a DNS resolver could not use verbatim as a query suffix. Enforcing this
    /// on-chain means the off-chain verifier never has to guess what a malformed claim meant: an
    /// unusable domain is unrepresentable rather than a silent lookup failure that would surface as
    /// "could not check" forever.
    ///
    /// Accepts lowercase LDH labels only (`a-z`, `0-9`, `-`), dot-separated, at least two labels, no
    /// empty label, no label starting or ending with `-`, total length <= 253. Uppercase is REJECTED
    /// rather than folded so the stored value is unambiguously the canonical query name.
    ///
    /// Rejecting the empty string is also what makes the `domain != "" <=> CLAIMED` invariant hold from
    /// the claim side.
    function _requireValidDomain(string calldata domain) internal pure {
        bytes calldata b = bytes(domain);
        if (b.length == 0 || b.length > MAX_DOMAIN_LENGTH) revert BadDomain();

        uint256 labelLen;
        uint256 labels;
        for (uint256 i; i < b.length; i++) {
            bytes1 c = b[i];
            if (c == ".") {
                if (labelLen == 0) revert BadDomain(); // empty label ("a..b", or a leading dot)
                if (b[i - 1] == "-") revert BadDomain(); // label ends with '-'
                labels++;
                labelLen = 0;
                continue;
            }
            bool ldh = (c >= "a" && c <= "z") || (c >= "0" && c <= "9") || c == "-";
            if (!ldh) revert BadDomain();
            if (labelLen == 0 && c == "-") revert BadDomain(); // label starts with '-'
            labelLen++;
            if (labelLen > 63) revert BadDomain();
        }
        // The final label must be non-empty (no trailing dot) and must not end with '-'.
        if (labelLen == 0) revert BadDomain();
        if (b[b.length - 1] == "-") revert BadDomain();
        labels++;
        if (labels < 2) revert BadDomain(); // a bare single label is never a resolvable issuer domain
    }
}
