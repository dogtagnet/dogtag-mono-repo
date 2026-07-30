// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {
    AccessControlDefaultAdminRules
} from "@openzeppelin/contracts/access/extensions/AccessControlDefaultAdminRules.sol";

/// @dev Safe production propose→execute delay, single-sourced so the deploy script's mainnet guard and
/// the env default cannot drift from the contract's documented default. Exposed on-chain as
/// `ProtocolRegistryV2.DEFAULT_PUBLISH_TIMELOCK`.
uint256 constant DEFAULT_PUBLISH_TIMELOCK_SECONDS_V2 = 2 days;

/// @dev The FLOOR every deployment must clear, enforced in the constructor. See the "The timelock is a
/// constructor invariant" note on [`ProtocolRegistryV2`] for the derivation — this is not a taste value.
uint256 constant MIN_PUBLISH_TIMELOCK_SECONDS_V2 = 1 hours;

/// @title ProtocolRegistryV2 — the generation-2 discovery TRUST ANCHOR.
/// @notice The dogtag-governed record of which contracts and which proving artifacts are current, read
/// by every app before it acts on a platform's version CLAIM. Same role as generation 1's
/// `ProtocolRegistry`; a SEPARATE deployment, because generation 1's record cannot describe generation 2.
///
/// # Why a new registry is FORCED, not chosen
///
/// `ProtocolRegistry.ContractSet` (`ProtocolRegistry.sol:97-106`) is a FIXED struct holding exactly
/// `contractSetId`, `factory`, `verificationRegistry`, `sbt`, `verifier`, `circuitId`, `publishedAt`,
/// `active`. It has no member for a provider-authority core, for the resolver layer, or for a
/// cross-generation provenance index — and the contract is not upgradeable (no proxy, no setter that can
/// widen a struct). A Solidity struct's shape is part of its storage layout and its ABI, so generation 2
/// cannot be published into it at all. The choice this slice actually makes is what the new record holds
/// and what delay protects it; that there must BE a new record is settled by the struct.
///
/// # What generation 2 adds, and why each is on THIS axis
///
/// Two addresses join the trio + verifier, and both are here rather than on a separate axis for the same
/// structural reason: `VerificationRegistryConsent` pins them in IMMUTABLE slots
/// (`VerificationRegistryConsent.sol:86-88`), so neither can move without moving the verification
/// registry, which is the definition of an on-chain-axis rotation.
///
///   * `providerRegistry` — the `ProviderRegistry` authority core. It occupies the registry's immutable
///     `issuerRegistry` slot (it answers `isWhitelistedFor(bytes32,address)` with generation 1's exact
///     shape, which is what lets it drop into that slot), and it is ALSO the root of the resolver layer:
///     a provider's directory resolver and a service's domain resolver are selected THROUGH this core
///     (`ProviderRegistry.setDirectoryResolver` / `setDomainResolver`, allowlisted per
///     `ResolverKind.DIRECTORY` / `ResolverKind.DOMAIN`).
///   * `rootIndex` — whatever occupies the verification registry's immutable `rootIndex` slot. In
///     generation 2 that is the `CloneProvenanceRouter`, which answers `rootIssuer(bytes32)` and
///     `isClone(address)` across factory generations. The field is defined by the SLOT rather than by
///     the router's name so a later generation with a different provenance shape still fits.
///
/// There is deliberately NO single `providerDirectory` or `serviceDomainResolver` member. Publishing one
/// address for either would be a false statement about the deployed design: the authority core
/// allowlists MANY resolvers per kind and each provider/service selects its own, so a consumer that
/// followed a protocol-wide resolver address would read the wrong resolver for every provider that
/// selected another. The core is the resolution root, and `providerRegistry` is how a consumer reaches
/// it. (A consumer must additionally re-check `isResolverApproved(kind, resolver)`, because the core
/// keeps a deapproved selection as history.)
///
/// `factory` no longer satisfies generation 1's `factory == verificationRegistry.rootIndex()` invariant.
/// In generation 2 the two are DIFFERENT contracts — the factory is where a provider's clone comes from,
/// the root index is what resolves an anchored root — so they are carried as separate members. Reading
/// `factory` where `rootIndex` is meant resolves generation-2-only roots and misses every historical
/// one, which is the failure the router exists to prevent.
///
/// # The renamed record is a STRUCTURAL guard, not cosmetics
///
/// Generation 1's `getContractSet(bytes32)` returns an 8-word static tuple; this record is 10 words. Had
/// the getter kept its name, a generation-1 client pointed at this registry would DISPATCH successfully
/// (identical selector) and decode the first 8 words as its own struct — silently reading `circuitId`
/// where `providerRegistry` sits and `publishedAt` where `circuitId` sits. That is the same
/// identical-shape/different-semantics trap this codebase has already paid for twice (the two
/// `recordVerificationZK` arities sharing selector `0xdd080593`; the stale hard-coded `isValid`
/// selector). So the record that CHANGED SHAPE is renamed — `DiscoverySet`, read via
/// [`getDiscoverySet`] / [`resolveDiscovery`] — and a generation-1 client reverts instead of
/// misdecoding, without depending on every client remembering to check the return width.
/// [`ArtifactSet`] is byte-for-byte generation 1's, so [`getArtifactSet`] and
/// [`getActiveArtifactSet`] deliberately KEEP their names and selectors: no misdecode is possible
/// there, and an artifact decoder written for generation 1 stays correct.
///
/// # The timelock is a constructor invariant, because it is immutable
///
/// The live generation-1 registry was deployed with `PUBLISH_TIMELOCK == 0`. Propose and execute can
/// land in the same block there, so its publisher key can repoint the entire declared protocol set in
/// one transaction with no window for anyone to notice — which is the one property a timelock exists to
/// provide. The value is `immutable`, so that cannot be repaired: a redeploy is the only remedy, and
/// this is the redeploy.
///
/// A zero is therefore not merely discouraged here, it is UNREPRESENTABLE: the constructor requires
/// `publishTimelock >= MIN_PUBLISH_TIMELOCK`. The guard is on the contract and not only in the deploy
/// script because a script guard is bypassable by a direct deployment, and the mistake it would let
/// through is unfixable.
///
/// The floor is 1 hour, derived rather than chosen. A delay is only a review window if a watcher can see
/// the proposal inside it, and the oversight indexer that surfaces governance events is finality-aware:
/// on ROAX the `finalized` tag sits ~80 blocks behind `latest`, so a proposal is not authoritatively
/// visible for minutes, before any human looks. A floor below that would be a timelock that exists only
/// in the getter — a guard satisfiable vacuously, which is worse than none because it reads as
/// protection. One hour clears finality plus a reaction with room to spare, and is still short enough to
/// rehearse a whole cutover in one sitting. Production uses [`DEFAULT_PUBLISH_TIMELOCK`] (2 days); the
/// deploy script requires exactly that unless a testnet opt-in is stated aloud.
///
/// # Everything else mirrors generation 1 deliberately
///
/// Two axes (R-5), one binding between them, `propose…`→timelock→`execute…` on every write a consumer
/// could be steered by, and immediate un-timelocked `deprecate…` as the emergency lever that never
/// deletes history but does cancel an in-flight proposal. That shape is reviewed, tested and understood;
/// this contract does not re-litigate it, so a reader can diff the two files and see only the record.
contract ProtocolRegistryV2 is AccessControlDefaultAdminRules {
    /// @dev Two-step admin handover delay — mirrors generation 1 and `VerificationRegistryConsent`.
    uint48 public constant ADMIN_TRANSFER_DELAY = 2 days;

    /// @notice May propose/execute/deprecate on either axis, and re-point bindings. Held by dogtag
    /// governance. Same derivation as generation 1: `keccak256("PUBLISHER")`, NOT
    /// `keccak256("PUBLISHER_ROLE")` — read it off the deployed contract rather than recomputing it from
    /// the variable name.
    bytes32 public constant PUBLISHER_ROLE = keccak256("PUBLISHER");

    /// @notice Safe production default (2 days), mirroring `VerificationRegistryConsent.ZK_TIMELOCK`.
    uint256 public constant DEFAULT_PUBLISH_TIMELOCK = DEFAULT_PUBLISH_TIMELOCK_SECONDS_V2;

    /// @notice The floor the constructor enforces, so a zero-delay registry cannot be deployed at all.
    /// See the contract-level note for why this is a constructor invariant and how the value is derived.
    uint256 public constant MIN_PUBLISH_TIMELOCK = MIN_PUBLISH_TIMELOCK_SECONDS_V2;

    /// @notice The immutable propose→execute delay selected when this registry is deployed. Always
    /// `>= MIN_PUBLISH_TIMELOCK`.
    uint256 public immutable PUBLISH_TIMELOCK;

    // ---------------------------------------------------------------------------------------------
    // Axis 1 — the ON-CHAIN discovery set
    // ---------------------------------------------------------------------------------------------

    /// @notice The on-chain half of what dogtag certifies for a protocol version: which deployed
    /// contracts belong together. Rotating this axis moves addresses; it says nothing about artifacts.
    ///
    /// Named `DiscoverySet` rather than generation 1's `ContractSet` because the record grew from the
    /// trio + verifier to the whole discovery layer, and because the different selector that rename
    /// yields is what stops a generation-1 client misdecoding this wider tuple (see the contract note).
    struct DiscoverySet {
        bytes32 discoverySetId; // keccak256("dogtag-levelb/2") — the map key, must be non-zero
        address factory; // where a provider's clone comes from. NOT the root index in generation 2.
        address verificationRegistry; // the registry a proof is submitted to — the anti-redirect anchor
        address sbt; // == verificationRegistry.sbt() (immutable there; the SHARED, reused SBT)
        address verifier; // Groth16Verifier* — the on-chain VK identity (NOT a hash)
        address providerRegistry; // == verificationRegistry.issuerRegistry(); also the resolver root
        address rootIndex; // == verificationRegistry.rootIndex(); the CloneProvenanceRouter
        bytes32 circuitId; // keccak256("consent.circom/DogTagConsent(6)")
        uint64 publishedAt; // stamped by executeDiscoverySet (block.timestamp), NOT from calldata
        bool active; // deprecateDiscoverySet flips this false; the record is never deleted
    }

    /// @notice discoverySetId -> the published set. Read the full record via [`getDiscoverySet`], which
    /// fails closed on an unknown id; this auto-getter answers a zeroed record instead.
    mapping(bytes32 => DiscoverySet) public discoverySets;

    /// @notice Every discoverySetId ever published — the enumerable list. A deprecated set stays here
    /// (history is pinned); a swap-republish does not duplicate its id.
    bytes32[] public discoverySetList;

    /// @dev Staged, not-yet-executed proposals, keyed per-id (several can be in flight).
    mapping(bytes32 => DiscoverySet) private _pendingDiscoverySet;
    /// @notice discoverySetId -> earliest execute timestamp (0 == nothing pending for this id).
    mapping(bytes32 => uint256) public discoverySetEta;

    // ---------------------------------------------------------------------------------------------
    // Axis 2 — the OFF-CHAIN proving-artifact set (byte-for-byte generation 1's)
    // ---------------------------------------------------------------------------------------------

    /// @notice The off-chain half: the proving artifacts an app fetches, their byte-integrity pins, and
    /// the minimum app version able to load them. Rotating this axis moves no address.
    ///
    /// UNCHANGED from generation 1, deliberately: the circuit, the ceremony and the frozen VK are
    /// untouched by the provider-registry work, so an artifact decoder written for generation 1 stays
    /// correct and the two axes' independence is preserved across the generation boundary.
    struct ArtifactSet {
        bytes32 artifactSetId; // keccak256("dogtag-levelb-artifacts/1") — the map key, non-zero
        bytes32 zkeySha256; // FETCH pin for the zkey (mandatory; NOT the VK)
        bytes32 witnessMobileSha256; // FETCH pin for the .graph (0 == unpinned — optional, unlike the zkey)
        bytes32 witnessServerR1csSha256; // FETCH pin for the .r1cs
        bytes32 witnessServerWasmSha256; // FETCH pin for the .wasm
        string artifactBaseUrl; // where the bytes live (any host; integrity via the pins)
        string minAppVersion; // min-app-version enforcement (semver string) — the deprecation lever
        uint64 publishedAt; // stamped by executeArtifactSet (block.timestamp), NOT from calldata
        bool active; // deprecateArtifactSet flips this false; the record is never deleted
    }

    /// @notice artifactSetId -> the published ArtifactSet. NOTE: the auto-getter answers an UNKNOWN id
    /// with a zeroed record rather than reverting, so a resolver must read via [`getArtifactSet`].
    mapping(bytes32 => ArtifactSet) public artifactSets;

    /// @notice Every artifactSetId ever published — the enumerable list. Deprecated sets stay.
    bytes32[] public artifactSetList;

    /// @dev Staged, not-yet-executed artifact-set proposals, keyed per-id.
    mapping(bytes32 => ArtifactSet) private _pendingArtifactSet;
    /// @notice artifactSetId -> earliest execute timestamp (0 == nothing pending for this id).
    mapping(bytes32 => uint256) public artifactSetEta;

    // ---------------------------------------------------------------------------------------------
    // The binding — the ONLY link between the two axes
    // ---------------------------------------------------------------------------------------------

    /// @notice discoverySetId -> the artifactSetId a resolver should currently use for it. This pointer
    /// is the whole coupling: it is what makes an artifact rotation a one-line governance change instead
    /// of a republish of the on-chain record.
    mapping(bytes32 => bytes32) public activeArtifactSetOf;

    /// @dev Staged, not-yet-executed binding changes: discoverySetId -> proposed artifactSetId.
    mapping(bytes32 => bytes32) private _pendingBinding;
    /// @notice discoverySetId -> earliest execute timestamp for a pending binding (0 == none pending).
    mapping(bytes32 => uint256) public bindingEta;

    event DiscoverySetProposed(bytes32 indexed discoverySetId, uint256 eta);
    event DiscoverySetPublished(bytes32 indexed discoverySetId, bool isNew);
    event DiscoverySetDeprecated(bytes32 indexed discoverySetId);

    event ArtifactSetProposed(bytes32 indexed artifactSetId, uint256 eta);
    event ArtifactSetPublished(bytes32 indexed artifactSetId, bool isNew);
    event ArtifactSetDeprecated(bytes32 indexed artifactSetId);

    event ArtifactBindingProposed(bytes32 indexed discoverySetId, bytes32 indexed artifactSetId, uint256 eta);
    event ArtifactBindingSet(bytes32 indexed discoverySetId, bytes32 indexed artifactSetId);

    error PublishTimelockBelowFloor(uint256 given, uint256 floor);

    /// @param admin The dogtag governance holder; receives `DEFAULT_ADMIN_ROLE` under the two-step ACDAR
    /// timelock. It alone can grant/revoke `PUBLISHER_ROLE`.
    /// @param publisher The initial `PUBLISHER_ROLE` holder (the publishing key).
    /// @param publishTimelock The immutable delay applied to every publish proposal. MUST be at least
    /// [`MIN_PUBLISH_TIMELOCK`]; production uses [`DEFAULT_PUBLISH_TIMELOCK`].
    constructor(address admin, address publisher, uint256 publishTimelock)
        AccessControlDefaultAdminRules(ADMIN_TRANSFER_DELAY, admin)
    {
        require(admin != address(0) && publisher != address(0), "zero");
        // The one invariant that cannot be fixed after deployment, so it is checked before there is
        // anything to fix. A named error rather than a string: an operator who trips this is holding a
        // value they believed was fine, and the two numbers are the whole diagnosis.
        if (publishTimelock < MIN_PUBLISH_TIMELOCK) {
            revert PublishTimelockBelowFloor(publishTimelock, MIN_PUBLISH_TIMELOCK);
        }
        PUBLISH_TIMELOCK = publishTimelock;
        _grantRole(PUBLISHER_ROLE, publisher);
    }

    // ---------------------------------------------------------------------------------------------
    // Axis 1 governance
    // ---------------------------------------------------------------------------------------------

    /// @notice Stage an on-chain discovery set for publication. After [`PUBLISH_TIMELOCK`] elapses, call
    /// [`executeDiscoverySet`]. Re-proposing an id before execute overwrites the pending record and
    /// RESETS its timelock. `publishedAt`/`active` in `d` are IGNORED — they are stamped at execute — so
    /// a publisher cannot back-date or pre-activate a set.
    ///
    /// Every address member is required non-zero. A zero would not be a smaller record, it would be a
    /// published claim that some component is at `address(0)`: a consumer reading it would `staticcall`
    /// nothing, get empty returndata, and have to decide what that means — the exact
    /// could-not-check-rendered-as-an-answer this registry exists to remove. A generation that genuinely
    /// has no such component publishes no set here.
    function proposeDiscoverySet(DiscoverySet calldata d) external onlyRole(PUBLISHER_ROLE) {
        require(d.discoverySetId != 0, "discoverySetId=0");
        require(
            d.factory != address(0) && d.verificationRegistry != address(0) && d.sbt != address(0)
                && d.verifier != address(0),
            "zero trio/verifier"
        );
        require(d.providerRegistry != address(0), "zero providerRegistry");
        require(d.rootIndex != address(0), "zero rootIndex");
        require(d.circuitId != 0, "circuitId=0");

        _pendingDiscoverySet[d.discoverySetId] = d;
        uint256 eta = block.timestamp + PUBLISH_TIMELOCK;
        discoverySetEta[d.discoverySetId] = eta;
        emit DiscoverySetProposed(d.discoverySetId, eta);
    }

    /// @notice Execute a previously-proposed discovery set once its timelock has elapsed. Appends to
    /// [`discoverySetList`] only on FIRST publication of an id (a swap-republish updates in place).
    /// Stamps `publishedAt = block.timestamp` and `active = true` authoritatively.
    function executeDiscoverySet(bytes32 id) external onlyRole(PUBLISHER_ROLE) {
        uint256 eta = discoverySetEta[id];
        require(eta != 0, "none pending");
        require(block.timestamp >= eta, "timelock");

        DiscoverySet memory d = _pendingDiscoverySet[id];
        d.publishedAt = uint64(block.timestamp);
        d.active = true;

        bool isNew = discoverySets[id].discoverySetId == 0;
        if (isNew) {
            discoverySetList.push(id);
        }
        discoverySets[id] = d;

        delete _pendingDiscoverySet[id];
        delete discoverySetEta[id];
        emit DiscoverySetPublished(id, isNew);
    }

    /// @notice Deprecate a published discovery set: flips `active=false`, NEVER deletes (history stays
    /// pinned so an old record still self-routes). Not timelocked — it is the lever you want to be able
    /// to pull immediately. Reverts only on an unknown id, not on an already-deprecated one.
    ///
    /// It ALSO cancels any in-flight proposal for `id`, which is what makes deprecate a true EMERGENCY
    /// HALT rather than a bit a stale proposal can flip straight back: without this, a swap proposed
    /// before the compromise was found — and whose timelock has already elapsed — could be executed
    /// afterwards and silently re-activate the retired set with no fresh review window. Cancelling a
    /// PENDING proposal is not deleting history: the published record and its place in
    /// [`discoverySetList`] are untouched.
    function deprecateDiscoverySet(bytes32 id) external onlyRole(PUBLISHER_ROLE) {
        require(discoverySets[id].discoverySetId != 0, "unknown discovery set");
        discoverySets[id].active = false;
        delete _pendingDiscoverySet[id];
        delete discoverySetEta[id];
        emit DiscoverySetDeprecated(id);
    }

    // ---------------------------------------------------------------------------------------------
    // Axis 2 governance
    // ---------------------------------------------------------------------------------------------

    /// @notice Stage an off-chain artifact set for publication. Same propose→timelock→execute shape as
    /// the on-chain axis; `publishedAt`/`active` are stamped at execute.
    function proposeArtifactSet(ArtifactSet calldata a) external onlyRole(PUBLISHER_ROLE) {
        require(a.artifactSetId != 0, "artifactSetId=0");
        // The zkey is the ceremony-bound artifact; a set with no zkey pin is unrepresentable (a swapped
        // key would silently prove against the wrong VK). The graph pin MAY be 0: a deployment may
        // publish its set before it has a graph identity to attest, and that window must be
        // representable, so it is deliberately NOT required here.
        require(a.zkeySha256 != 0, "zkeySha256=0");

        _pendingArtifactSet[a.artifactSetId] = a;
        uint256 eta = block.timestamp + PUBLISH_TIMELOCK;
        artifactSetEta[a.artifactSetId] = eta;
        emit ArtifactSetProposed(a.artifactSetId, eta);
    }

    /// @notice Execute a previously-proposed artifact set once its timelock has elapsed. Appends to
    /// [`artifactSetList`] only on FIRST publication of an id.
    function executeArtifactSet(bytes32 id) external onlyRole(PUBLISHER_ROLE) {
        uint256 eta = artifactSetEta[id];
        require(eta != 0, "none pending");
        require(block.timestamp >= eta, "timelock");

        ArtifactSet memory a = _pendingArtifactSet[id];
        a.publishedAt = uint64(block.timestamp);
        a.active = true;

        bool isNew = artifactSets[id].artifactSetId == 0;
        if (isNew) {
            artifactSetList.push(id);
        }
        artifactSets[id] = a;

        delete _pendingArtifactSet[id];
        delete artifactSetEta[id];
        emit ArtifactSetPublished(id, isNew);
    }

    /// @notice Deprecate a published artifact set: flips `active=false`, never deletes. Not timelocked.
    ///
    /// Like [`deprecateDiscoverySet`] it ALSO cancels any in-flight proposal for `id`. This matters more
    /// on this axis: a deprecate leaves [`activeArtifactSetOf`] still pointing at the retired set, so a
    /// re-activation would instantly restore compromised artifacts as the live ones for every discovery
    /// set bound to it.
    function deprecateArtifactSet(bytes32 id) external onlyRole(PUBLISHER_ROLE) {
        require(artifactSets[id].artifactSetId != 0, "unknown artifact set");
        artifactSets[id].active = false;
        delete _pendingArtifactSet[id];
        delete artifactSetEta[id];
        emit ArtifactSetDeprecated(id);
    }

    // ---------------------------------------------------------------------------------------------
    // Binding governance
    // ---------------------------------------------------------------------------------------------

    /// @notice Stage "discovery set `discoverySetId` should now resolve to artifact set `artifactSetId`".
    /// TIMELOCKED because this pointer is what an app follows to decide which bytes to fetch, so a
    /// one-transaction repoint by a compromised publisher key is exactly the attack the window catches.
    ///
    /// The pointer can never dangle: both ids must be PUBLISHED and ACTIVE, enforced at EXECUTE (see
    /// [`executeArtifactBinding`]) rather than here. Checking at execute is both stricter — a set
    /// deprecated during the window cannot slip through — and what lets a FIRST rollout run its three
    /// timelocks CONCURRENTLY: propose the two sets and their binding together, wait once, then execute
    /// sets-then-binding.
    ///
    /// Whether the artifacts are CRYPTOGRAPHICALLY compatible with the set's `verifier` is a governance
    /// judgement this contract deliberately does not (and cannot) make: pins are byte-integrity, the
    /// verifier is VK identity.
    function proposeArtifactBinding(bytes32 discoverySetId, bytes32 artifactSetId)
        external
        onlyRole(PUBLISHER_ROLE)
    {
        require(discoverySetId != 0, "discoverySetId=0");
        require(artifactSetId != 0, "artifactSetId=0");

        _pendingBinding[discoverySetId] = artifactSetId;
        uint256 eta = block.timestamp + PUBLISH_TIMELOCK;
        bindingEta[discoverySetId] = eta;
        emit ArtifactBindingProposed(discoverySetId, artifactSetId, eta);
    }

    /// @notice Execute a previously-proposed binding once its timelock has elapsed. This is the write
    /// that makes an artifact rotation visible to resolvers — and it touches NO `DiscoverySet` field.
    /// BOTH sides must be published AND ACTIVE AT THIS MOMENT.
    function executeArtifactBinding(bytes32 discoverySetId) external onlyRole(PUBLISHER_ROLE) {
        uint256 eta = bindingEta[discoverySetId];
        require(eta != 0, "none pending");
        require(block.timestamp >= eta, "timelock");

        bytes32 artifactSetId = _pendingBinding[discoverySetId];
        require(discoverySets[discoverySetId].discoverySetId != 0, "unknown discovery set");
        require(discoverySets[discoverySetId].active, "inactive discovery set");
        require(artifactSets[artifactSetId].artifactSetId != 0, "unknown artifact set");
        require(artifactSets[artifactSetId].active, "inactive artifact set");
        activeArtifactSetOf[discoverySetId] = artifactSetId;

        delete _pendingBinding[discoverySetId];
        delete bindingEta[discoverySetId];
        emit ArtifactBindingSet(discoverySetId, artifactSetId);
    }

    // ---------------------------------------------------------------------------------------------
    // Resolution
    // ---------------------------------------------------------------------------------------------

    /// @notice The FULL published on-chain record for `id`. Fails closed on an unknown id (reverts) so a
    /// consumer never mistakes a zeroed struct for a valid set. THIS is the axis an on-chain resolver —
    /// anything checking the trio, the authority core, the root index, the verifier or the circuitId —
    /// must read.
    ///
    /// Deliberately NOT named `getContractSet`: see the structural-guard note on the contract.
    function getDiscoverySet(bytes32 id) external view returns (DiscoverySet memory) {
        DiscoverySet memory d = discoverySets[id];
        require(d.discoverySetId != 0, "unknown discovery set");
        return d;
    }

    /// @notice The published artifact record for `id`, REVERTING when there is none — unlike the
    /// `artifactSets` auto-getter, which answers a zeroed record that would read as an unpinned set at
    /// version "" rather than as "no such set". Same name and same return shape as generation 1.
    function getArtifactSet(bytes32 id) external view returns (ArtifactSet memory) {
        ArtifactSet memory a = artifactSets[id];
        require(a.artifactSetId != 0, "unknown artifact set");
        return a;
    }

    /// @notice Follow the binding: the artifact set currently published for `discoverySetId`. Reverts if
    /// no binding is set (a discovery set with no artifacts bound yet is a valid intermediate state
    /// during a two-axis rollout, and a resolver must fail closed rather than read a zeroed record).
    /// Same name and same return shape as generation 1.
    function getActiveArtifactSet(bytes32 discoverySetId) external view returns (ArtifactSet memory) {
        bytes32 artifactSetId = activeArtifactSetOf[discoverySetId];
        require(artifactSetId != 0, "no artifact binding");
        ArtifactSet memory a = artifactSets[artifactSetId];
        require(a.artifactSetId != 0, "unknown artifact set");
        return a;
    }

    /// @notice One-call discovery for an app: both halves of what dogtag certifies for `discoverySetId`.
    /// A convenience over [`getDiscoverySet`] + [`getActiveArtifactSet`] — the axes stay independent,
    /// this only saves a round trip. Renamed from generation 1's `resolve` for the same structural reason
    /// as [`getDiscoverySet`]: its first return member changed shape.
    function resolveDiscovery(bytes32 discoverySetId)
        external
        view
        returns (DiscoverySet memory discoverySet, ArtifactSet memory artifactSet)
    {
        discoverySet = discoverySets[discoverySetId];
        require(discoverySet.discoverySetId != 0, "unknown discovery set");

        bytes32 artifactSetId = activeArtifactSetOf[discoverySetId];
        require(artifactSetId != 0, "no artifact binding");
        artifactSet = artifactSets[artifactSetId];
        require(artifactSet.artifactSetId != 0, "unknown artifact set");
    }

    /// @notice The full pending (proposed, not-yet-executed) discovery set for `id`. Reverts if nothing
    /// is pending. Lets governance inspect exactly what an execute would write during the window — which
    /// is what makes the delay a review period rather than just a delay.
    function getPendingDiscoverySet(bytes32 id) external view returns (DiscoverySet memory) {
        require(discoverySetEta[id] != 0, "none pending");
        return _pendingDiscoverySet[id];
    }

    /// @notice The full pending artifact set for `id`. Reverts if nothing is pending.
    function getPendingArtifactSet(bytes32 id) external view returns (ArtifactSet memory) {
        require(artifactSetEta[id] != 0, "none pending");
        return _pendingArtifactSet[id];
    }

    /// @notice The pending binding target for `discoverySetId`. Reverts if nothing is pending.
    function getPendingBinding(bytes32 discoverySetId) external view returns (bytes32) {
        require(bindingEta[discoverySetId] != 0, "none pending");
        return _pendingBinding[discoverySetId];
    }

    /// @notice How many distinct on-chain sets have ever been published (enumerate via
    /// `discoverySetList`).
    function discoverySetCount() external view returns (uint256) {
        return discoverySetList.length;
    }

    /// @notice How many distinct artifact sets have ever been published (enumerate via
    /// `artifactSetList`).
    function artifactSetCount() external view returns (uint256) {
        return artifactSetList.length;
    }
}
