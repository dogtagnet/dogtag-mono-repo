// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {
    AccessControlDefaultAdminRules
} from "@openzeppelin/contracts/access/extensions/AccessControlDefaultAdminRules.sol";

/// @title ProtocolRegistry — the dogtag-governed discovery TRUST ANCHOR (M7 §5.1, lock B).
/// @notice A small, read-mostly, dogtag-governed contract that records what dogtag has certified for a
/// protocol version. It is the on-chain ROOT OF TRUTH an app validates a platform's version CLAIM
/// against, so a platform can never steer a proof onto a contract dogtag did not publish (§5.3 step 4).
///
/// It is ADDITIVE: it neither deploys nor changes the trio/registry/verifier it references, and it does
/// not touch the frozen circuit/VK/ceremony. It only records WHICH addresses+artifacts dogtag has
/// certified as belonging together.
///
/// # TWO INDEPENDENT VERSION AXES (R-5)
///
/// The certified record is split across two SEPARATELY-KEYED, SEPARATELY-ROTATABLE axes:
///
///   * the **on-chain axis** — a [`ContractSet`], keyed by `contractSetId`
///     (`keccak256("dogtag-levelb/1")`). It holds ONLY things that live on-chain: the trio addresses,
///     the `verifier` (the on-chain VK identity), and the on-chain `circuitId`.
///   * the **off-chain artifact axis** — an [`ArtifactSet`], keyed by `artifactSetId`
///     (`keccak256("dogtag-levelb-artifacts/1")`). It holds ONLY things an app fetches and runs
///     off-chain: the four artifact fetch-pins, the `artifactBaseUrl`, and `minAppVersion` (the app
///     gate is a property of the proving artifacts an app must be new enough to load, not of the
///     deployed contracts).
///
/// Neither record references the other. The ONLY link is [`activeArtifactSetOf`], a governed pointer
/// `contractSetId -> artifactSetId` that tells a resolver which artifact set is current for a given
/// on-chain set. That means:
///
///   * rotating the proving artifacts (a new zkey, a new host, a raised `minAppVersion`) is ONE
///     `ArtifactSet` publish plus ONE re-point. The `ContractSet` record is not written, so no trio or
///     verifier address moves and no contract is redeployed.
///   * rotating the on-chain set (new trio addresses) is ONE `ContractSet` publish. Every `ArtifactSet`
///     record is untouched, and no app is forced to update.
///
/// Governance, not this contract, owns the SEMANTIC compatibility of a binding: the registry stores
/// data and never asserts that a given zkey proves against a given verifier (see the VK note below).
/// That is exactly why re-pointing is timelocked.
///
/// # On-chain VK identity vs fetch pins (§3.2 — a conflation this contract keeps distinct)
///
/// Two DIFFERENT things live on the two axes and must never be conflated:
///   * `verifier` + `verificationRegistry` (on-chain axis) — the **on-chain VK IDENTITY**. The
///     authoritative verifying key is the one embedded in the on-chain `Groth16Verifier*` at
///     `verifier`'s address; a proof is ultimately checked against it. The VK is identified by an
///     ADDRESS, never by a hash.
///   * `zkeySha256` / `witness*Sha256` (artifact axis) — **FETCH PINS** (byte-integrity of the
///     downloaded artifact). An app hashes the bytes it fetched from any host against these before
///     loading; a lying/compromised CDN is then a denial-of-service at worst, never a wrong-proof.
/// The `verification_key.json` file hash (the OFF-CHAIN VK identity the prover crate carries) is
/// deliberately NOT an on-chain field — on-chain the VK identity is the `verifier` address. It is
/// carried only by the signed-manifest fallback (§5.1 1B).
///
/// # Governance: timelocked publish, immediate deprecate
///
/// Every WRITE that a consumer could be steered by — publishing either kind of set, and re-pointing a
/// binding — is a `propose…` → (2-day timelock) → `execute…` flow that MIRRORS
/// `VerificationRegistryConsent.proposeZkVerifier`/`executeZkVerifier` (§5.1): nothing can be published
/// or SWAPPED instantly, so a compromised publisher key cannot repoint discovery in one transaction —
/// governance has the timelock window to react. The `deprecate…` calls are NOT timelocked: they only
/// set `active=false` (a safety lever you want to be able to pull immediately) and NEVER delete, so
/// history stays pinned (§5.1 — "deprecate without deleting").
contract ProtocolRegistry is AccessControlDefaultAdminRules {
    /// @dev Two-step admin handover delay — mirrors `VerificationRegistryConsent` (2 days).
    uint48 public constant ADMIN_TRANSFER_DELAY = 2 days;

    /// @notice May propose/execute/deprecate on either axis, and re-point bindings. Held by dogtag
    /// governance.
    bytes32 public constant PUBLISHER_ROLE = keccak256("PUBLISHER");

    /// @dev The propose→execute delay. MIRRORS `VerificationRegistryConsent.ZK_TIMELOCK` (2 days): the
    /// established registry timelock this contract is told to reuse, not a new one (§5.1).
    uint256 public constant PUBLISH_TIMELOCK = 2 days;

    // ---------------------------------------------------------------------------------------------
    // Axis 1 — the ON-CHAIN contract set
    // ---------------------------------------------------------------------------------------------

    /// @notice The on-chain half of what dogtag certifies for a protocol version: which deployed
    /// contracts belong together. Rotating this axis moves addresses; it says nothing about artifacts.
    struct ContractSet {
        bytes32 contractSetId; // keccak256("dogtag-levelb/1") — the map key, must be non-zero
        address factory; // trio leg (== verificationRegistry.rootIndex()); resolves rootIssuer[R]
        address verificationRegistry; // trio leg — the registry a proof is submitted to
        address sbt; // trio leg (== verificationRegistry.sbt())
        address verifier; // Groth16Verifier* — the on-chain VK identity (NOT a hash)
        bytes32 circuitId; // keccak256("consent.circom/DogTagConsent(6)")
        uint64 publishedAt; // stamped by executeContractSet (block.timestamp), NOT from calldata
        bool active; // deprecateContractSet flips this false; the record is never deleted
    }

    /// @notice contractSetId -> the published ContractSet. Read the full record via [`getContractSet`].
    mapping(bytes32 => ContractSet) public contractSets;

    /// @notice Every contractSetId ever published — the enumerable "which on-chain sets exist" list. A
    /// deprecated set stays here (history is pinned); a swap-republish does not duplicate its id.
    bytes32[] public contractSetList;

    /// @dev Staged, not-yet-executed contract-set proposals, keyed per-id (several can be in flight).
    mapping(bytes32 => ContractSet) private _pendingContractSet;
    /// @notice contractSetId -> earliest execute timestamp (0 == nothing pending for this id).
    mapping(bytes32 => uint256) public contractSetEta;

    // ---------------------------------------------------------------------------------------------
    // Axis 2 — the OFF-CHAIN proving-artifact set
    // ---------------------------------------------------------------------------------------------

    /// @notice The off-chain half: the proving artifacts an app fetches, their byte-integrity pins, and
    /// the minimum app version able to load them. Rotating this axis moves no address.
    struct ArtifactSet {
        bytes32 artifactSetId; // keccak256("dogtag-levelb-artifacts/1") — the map key, non-zero
        bytes32 zkeySha256; // FETCH pin for the zkey (mandatory; NOT the VK)
        bytes32 witnessMobileSha256; // FETCH pin for the .graph (0 == unpinned: not committed, §3.5)
        bytes32 witnessServerR1csSha256; // FETCH pin for the .r1cs
        bytes32 witnessServerWasmSha256; // FETCH pin for the .wasm
        string artifactBaseUrl; // where the bytes live (any host; integrity via the pins)
        string minAppVersion; // min-app-version enforcement (semver string) — the deprecation lever
        uint64 publishedAt; // stamped by executeArtifactSet (block.timestamp), NOT from calldata
        bool active; // deprecateArtifactSet flips this false; the record is never deleted
    }

    /// @notice artifactSetId -> the published ArtifactSet. NOTE: the auto-getter OMITS the string
    /// members (`artifactBaseUrl`, `minAppVersion`); read the FULL record via [`getArtifactSet`].
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

    /// @notice contractSetId -> the artifactSetId a resolver should currently use for it. This pointer
    /// is the whole coupling: it is what makes an artifact rotation a one-line governance change
    /// instead of a republish of the on-chain record.
    mapping(bytes32 => bytes32) public activeArtifactSetOf;

    /// @dev Staged, not-yet-executed binding changes: contractSetId -> proposed artifactSetId.
    mapping(bytes32 => bytes32) private _pendingBinding;
    /// @notice contractSetId -> earliest execute timestamp for a pending binding (0 == none pending).
    mapping(bytes32 => uint256) public bindingEta;

    event ContractSetProposed(bytes32 indexed contractSetId, uint256 eta);
    event ContractSetPublished(bytes32 indexed contractSetId, bool isNew);
    event ContractSetDeprecated(bytes32 indexed contractSetId);

    event ArtifactSetProposed(bytes32 indexed artifactSetId, uint256 eta);
    event ArtifactSetPublished(bytes32 indexed artifactSetId, bool isNew);
    event ArtifactSetDeprecated(bytes32 indexed artifactSetId);

    event ArtifactBindingProposed(bytes32 indexed contractSetId, bytes32 indexed artifactSetId, uint256 eta);
    event ArtifactBindingSet(bytes32 indexed contractSetId, bytes32 indexed artifactSetId);

    /// @param admin The dogtag governance multisig; receives `DEFAULT_ADMIN_ROLE` under the two-step
    /// ACDAR timelock. It alone can grant/revoke `PUBLISHER_ROLE`.
    /// @param publisher The initial `PUBLISHER_ROLE` holder (dogtag governance / the publishing key).
    constructor(address admin, address publisher) AccessControlDefaultAdminRules(ADMIN_TRANSFER_DELAY, admin) {
        require(admin != address(0) && publisher != address(0), "zero");
        _grantRole(PUBLISHER_ROLE, publisher);
    }

    // ---------------------------------------------------------------------------------------------
    // Axis 1 governance
    // ---------------------------------------------------------------------------------------------

    /// @notice Stage an on-chain contract set for publication. After [`PUBLISH_TIMELOCK`] elapses, call
    /// [`executeContractSet`]. Re-proposing an id before execute overwrites the pending record and
    /// RESETS its timelock (mirrors `proposeZkVerifier`). `publishedAt`/`active` in `c` are IGNORED —
    /// they are stamped at execute — so a publisher cannot back-date or pre-activate a set.
    function proposeContractSet(ContractSet calldata c) external onlyRole(PUBLISHER_ROLE) {
        require(c.contractSetId != 0, "contractSetId=0");
        require(
            c.factory != address(0) && c.verificationRegistry != address(0) && c.sbt != address(0)
                && c.verifier != address(0),
            "zero trio/verifier"
        );
        require(c.circuitId != 0, "circuitId=0");

        _pendingContractSet[c.contractSetId] = c;
        uint256 eta = block.timestamp + PUBLISH_TIMELOCK;
        contractSetEta[c.contractSetId] = eta;
        emit ContractSetProposed(c.contractSetId, eta);
    }

    /// @notice Execute a previously-proposed contract set once its timelock has elapsed. Appends to
    /// [`contractSetList`] only on FIRST publication of an id (a swap-republish updates in place).
    /// Stamps `publishedAt = block.timestamp` and `active = true` authoritatively.
    function executeContractSet(bytes32 id) external onlyRole(PUBLISHER_ROLE) {
        uint256 eta = contractSetEta[id];
        require(eta != 0, "none pending");
        require(block.timestamp >= eta, "timelock");

        ContractSet memory c = _pendingContractSet[id];
        c.publishedAt = uint64(block.timestamp);
        c.active = true;

        bool isNew = contractSets[id].contractSetId == 0;
        if (isNew) {
            contractSetList.push(id);
        }
        contractSets[id] = c;

        delete _pendingContractSet[id];
        delete contractSetEta[id];
        emit ContractSetPublished(id, isNew);
    }

    /// @notice Deprecate a published contract set: flips `active=false`, NEVER deletes (history is
    /// pinned so an old record still self-routes and verifies; §7.3). Not timelocked. Reverts only on
    /// an unknown id, not on an already-deprecated one.
    function deprecateContractSet(bytes32 id) external onlyRole(PUBLISHER_ROLE) {
        require(contractSets[id].contractSetId != 0, "unknown contract set");
        contractSets[id].active = false;
        emit ContractSetDeprecated(id);
    }

    // ---------------------------------------------------------------------------------------------
    // Axis 2 governance
    // ---------------------------------------------------------------------------------------------

    /// @notice Stage an off-chain artifact set for publication. Same propose→timelock→execute shape as
    /// the on-chain axis; `publishedAt`/`active` are stamped at execute.
    function proposeArtifactSet(ArtifactSet calldata a) external onlyRole(PUBLISHER_ROLE) {
        require(a.artifactSetId != 0, "artifactSetId=0");
        // The zkey is the ceremony-bound artifact; a set with no zkey pin is unrepresentable (a
        // swapped key would silently prove against the wrong VK). The graph pin MAY be 0 (the .graph is
        // not committed — the mobile resolver pins it from the anchor at fetch time; §3.5), so it is
        // deliberately NOT required here.
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
    function deprecateArtifactSet(bytes32 id) external onlyRole(PUBLISHER_ROLE) {
        require(artifactSets[id].artifactSetId != 0, "unknown artifact set");
        artifactSets[id].active = false;
        emit ArtifactSetDeprecated(id);
    }

    // ---------------------------------------------------------------------------------------------
    // Binding governance
    // ---------------------------------------------------------------------------------------------

    /// @notice Stage "contract set `contractSetId` should now resolve to artifact set `artifactSetId`".
    /// TIMELOCKED for the same reason `proposeZkVerifier` is: this pointer is what an app follows to
    /// decide which bytes to fetch, so a one-transaction repoint by a compromised publisher key is
    /// exactly the attack the window exists to catch.
    ///
    /// The pointer can never dangle: both ids must be PUBLISHED, and that is enforced at EXECUTE (see
    /// [`executeArtifactBinding`]) rather than here. Checking at execute is both stricter — a set
    /// deprecated-and-never-republished during the window cannot slip through — and what lets the FIRST
    /// rollout run its three timelocks CONCURRENTLY: propose the two sets and their binding together,
    /// wait once, then execute sets-then-binding.
    ///
    /// Whether the artifacts are CRYPTOGRAPHICALLY compatible with the set's `verifier` is a governance
    /// judgement this contract deliberately does not (and cannot) make: pins are byte-integrity, the
    /// verifier is VK identity.
    function proposeArtifactBinding(bytes32 contractSetId, bytes32 artifactSetId)
        external
        onlyRole(PUBLISHER_ROLE)
    {
        require(contractSetId != 0, "contractSetId=0");
        require(artifactSetId != 0, "artifactSetId=0");

        _pendingBinding[contractSetId] = artifactSetId;
        uint256 eta = block.timestamp + PUBLISH_TIMELOCK;
        bindingEta[contractSetId] = eta;
        emit ArtifactBindingProposed(contractSetId, artifactSetId, eta);
    }

    /// @notice Execute a previously-proposed binding once its timelock has elapsed. This is the write
    /// that makes an artifact rotation visible to resolvers — and it touches NO `ContractSet` field.
    /// BOTH sides must be published AT THIS MOMENT, so the pointer can never be made to dangle.
    function executeArtifactBinding(bytes32 contractSetId) external onlyRole(PUBLISHER_ROLE) {
        uint256 eta = bindingEta[contractSetId];
        require(eta != 0, "none pending");
        require(block.timestamp >= eta, "timelock");

        bytes32 artifactSetId = _pendingBinding[contractSetId];
        require(contractSets[contractSetId].contractSetId != 0, "unknown contract set");
        require(artifactSets[artifactSetId].artifactSetId != 0, "unknown artifact set");
        activeArtifactSetOf[contractSetId] = artifactSetId;

        delete _pendingBinding[contractSetId];
        delete bindingEta[contractSetId];
        emit ArtifactBindingSet(contractSetId, artifactSetId);
    }

    // ---------------------------------------------------------------------------------------------
    // Resolution
    // ---------------------------------------------------------------------------------------------

    /// @notice The FULL published on-chain record for `id`. Fails closed on an unknown id (reverts) so
    /// a consumer never mistakes a zeroed struct for a valid set. THIS is the axis an on-chain
    /// resolver — anything checking trio addresses, the verifier, or the circuitId — must read.
    function getContractSet(bytes32 id) external view returns (ContractSet memory) {
        ContractSet memory c = contractSets[id];
        require(c.contractSetId != 0, "unknown contract set");
        return c;
    }

    /// @notice The FULL published artifact record for `id`, including the string members the
    /// auto-getter omits. THIS is the axis an artifact/app-gate resolver — anything fetching a zkey or
    /// enforcing `minAppVersion` — must read.
    function getArtifactSet(bytes32 id) external view returns (ArtifactSet memory) {
        ArtifactSet memory a = artifactSets[id];
        require(a.artifactSetId != 0, "unknown artifact set");
        return a;
    }

    /// @notice Follow the binding: the artifact set currently published for `contractSetId`. Reverts if
    /// no binding is set (an on-chain set with no artifacts bound yet is a valid intermediate state
    /// during a two-axis rollout, and a resolver must fail closed rather than read a zeroed record).
    function getActiveArtifactSet(bytes32 contractSetId) external view returns (ArtifactSet memory) {
        bytes32 artifactSetId = activeArtifactSetOf[contractSetId];
        require(artifactSetId != 0, "no artifact binding");
        ArtifactSet memory a = artifactSets[artifactSetId];
        require(a.artifactSetId != 0, "unknown artifact set");
        return a;
    }

    /// @notice One-call discovery for an app: both halves of what dogtag certifies for `contractSetId`.
    /// A convenience over [`getContractSet`] + [`getActiveArtifactSet`] — the axes stay independent,
    /// this only saves a round trip.
    function resolve(bytes32 contractSetId)
        external
        view
        returns (ContractSet memory contractSet, ArtifactSet memory artifactSet)
    {
        contractSet = contractSets[contractSetId];
        require(contractSet.contractSetId != 0, "unknown contract set");

        bytes32 artifactSetId = activeArtifactSetOf[contractSetId];
        require(artifactSetId != 0, "no artifact binding");
        artifactSet = artifactSets[artifactSetId];
        require(artifactSet.artifactSetId != 0, "unknown artifact set");
    }

    /// @notice The full pending (proposed, not-yet-executed) contract set for `id`. Reverts if nothing
    /// is pending. Lets governance inspect exactly what an execute would write during the window.
    function getPendingContractSet(bytes32 id) external view returns (ContractSet memory) {
        require(contractSetEta[id] != 0, "none pending");
        return _pendingContractSet[id];
    }

    /// @notice The full pending artifact set for `id`. Reverts if nothing is pending.
    function getPendingArtifactSet(bytes32 id) external view returns (ArtifactSet memory) {
        require(artifactSetEta[id] != 0, "none pending");
        return _pendingArtifactSet[id];
    }

    /// @notice The pending binding target for `contractSetId`. Reverts if nothing is pending.
    function getPendingBinding(bytes32 contractSetId) external view returns (bytes32) {
        require(bindingEta[contractSetId] != 0, "none pending");
        return _pendingBinding[contractSetId];
    }

    /// @notice How many distinct on-chain sets have ever been published (enumerate via
    /// `contractSetList`).
    function contractSetCount() external view returns (uint256) {
        return contractSetList.length;
    }

    /// @notice How many distinct artifact sets have ever been published (enumerate via
    /// `artifactSetList`).
    function artifactSetCount() external view returns (uint256) {
        return artifactSetList.length;
    }
}
