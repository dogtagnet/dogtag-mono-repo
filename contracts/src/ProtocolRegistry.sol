// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {
    AccessControlDefaultAdminRules
} from "@openzeppelin/contracts/access/extensions/AccessControlDefaultAdminRules.sol";

/// @title ProtocolRegistry — the dogtag-governed discovery TRUST ANCHOR (M7 §5.1, lock B).
/// @notice A small, read-mostly, dogtag-governed contract that maps a protocol VERSION
/// (`keccak256("dogtag-levelb/1")`) to its trio + verifier + artifact fetch-pins. It is the on-chain
/// ROOT OF TRUTH an app validates a platform's version CLAIM against, so a platform can never steer a
/// proof onto a contract dogtag did not publish (§5.3 step 4).
///
/// It is ADDITIVE: it neither deploys nor changes the trio/registry/verifier it references, and it does
/// not touch the frozen circuit/VK/ceremony. It only records, per version, WHICH addresses+artifacts
/// dogtag has certified as belonging together.
///
/// # On-chain VK identity vs fetch pins (§3.2 — a conflation this contract keeps distinct)
///
/// Two DIFFERENT things live in the [`Version`] struct and must never be conflated:
///   * `verifier` + `verificationRegistry` — the **on-chain VK IDENTITY**. The authoritative verifying
///     key is the one embedded in the on-chain `Groth16Verifier*` at `verifier`'s address; a proof is
///     ultimately checked against it. The VK is identified by an ADDRESS, never by a hash.
///   * `zkeySha256` / `witness*Sha256` — **FETCH PINS** (byte-integrity of the downloaded artifact). An
///     app hashes the bytes it fetched from any host against these before loading; a lying/compromised
///     CDN is then a denial-of-service at worst, never a wrong-proof.
/// The `verification_key.json` file hash (the OFF-CHAIN VK identity the prover crate carries) is
/// deliberately NOT an on-chain field — on-chain the VK identity is the `verifier` address. It is
/// carried only by the signed-manifest fallback (§5.1 1B).
///
/// # Governance: timelocked publish, immediate deprecate
///
/// `publishVersion` is realised as a `proposeVersion` → (2-day timelock) → `executeVersion` flow that
/// MIRRORS `VerificationRegistryConsent.proposeZkVerifier`/`executeZkVerifier` (§5.1): a version cannot
/// be published or SWAPPED instantly, so a compromised publisher key cannot repoint discovery in one
/// transaction — governance has the timelock window to react. `deprecateVersion` is NOT timelocked: it
/// only sets `active=false` (a safety lever you want to be able to pull immediately) and NEVER deletes,
/// so history stays pinned (§5.1 — "deprecate without deleting").
contract ProtocolRegistry is AccessControlDefaultAdminRules {
    /// @dev Two-step admin handover delay — mirrors `VerificationRegistryConsent` (2 days).
    uint48 public constant ADMIN_TRANSFER_DELAY = 2 days;

    /// @notice May propose/execute/deprecate versions. Held by dogtag governance.
    bytes32 public constant PUBLISHER_ROLE = keccak256("PUBLISHER");

    /// @dev The propose→execute delay. MIRRORS `VerificationRegistryConsent.ZK_TIMELOCK` (2 days): the
    /// established registry timelock this contract is told to reuse, not a new one (§5.1).
    uint256 public constant PUBLISH_TIMELOCK = 2 days;

    /// @notice The full record dogtag certifies for one protocol version (§5.1 / §5.2 TRUST tier).
    struct Version {
        bytes32 versionId; // keccak256("dogtag-levelb/1") — the map key, must be non-zero
        address factory; // trio leg (== verificationRegistry.rootIndex()); resolves rootIssuer[R]
        address verificationRegistry; // trio leg — the registry a proof is submitted to
        address sbt; // trio leg (== verificationRegistry.sbt())
        address verifier; // Groth16Verifier* — the on-chain VK identity (NOT a hash)
        bytes32 circuitId; // keccak256("consent.circom/DogTagConsent(6)")
        bytes32 zkeySha256; // FETCH pin for the zkey (mandatory; NOT the VK)
        bytes32 witnessMobileSha256; // FETCH pin for the .graph (0 == unpinned: not committed, §3.5)
        bytes32 witnessServerR1csSha256; // FETCH pin for the .r1cs
        bytes32 witnessServerWasmSha256; // FETCH pin for the .wasm
        string artifactBaseUrl; // where the bytes live (any host; integrity via the pins)
        string minAppVersion; // min-app-version enforcement (semver string) — the deprecation lever
        uint64 publishedAt; // stamped by executeVersion (block.timestamp), NOT trusted from calldata
        bool active; // deprecate flips this false; the record is never deleted
    }

    /// @notice versionId -> the published Version. NOTE: the auto-getter OMITS the string members
    /// (`artifactBaseUrl`, `minAppVersion`); read the FULL record via [`getVersion`].
    mapping(bytes32 => Version) public versions;

    /// @notice Every versionId ever published — the enumerable "which versions exist" list. A
    /// deprecated version stays here (history is pinned); a swap-republish does not duplicate its id.
    bytes32[] public versionList;

    /// @dev Staged, not-yet-executed proposals, keyed per-versionId (a mapping, not a single slot —
    /// several versions can be in-flight at once, unlike the registry's single pending verifier).
    mapping(bytes32 => Version) private _pending;
    /// @notice versionId -> earliest execute timestamp (0 == nothing pending for this id).
    mapping(bytes32 => uint256) public publishEta;

    event VersionProposed(bytes32 indexed versionId, uint256 eta);
    event VersionPublished(bytes32 indexed versionId, bool isNew);
    event VersionDeprecated(bytes32 indexed versionId);

    /// @param admin The dogtag governance multisig; receives `DEFAULT_ADMIN_ROLE` under the two-step
    /// ACDAR timelock. It alone can grant/revoke `PUBLISHER_ROLE`.
    /// @param publisher The initial `PUBLISHER_ROLE` holder (dogtag governance / the publishing key).
    constructor(address admin, address publisher) AccessControlDefaultAdminRules(ADMIN_TRANSFER_DELAY, admin) {
        require(admin != address(0) && publisher != address(0), "zero");
        _grantRole(PUBLISHER_ROLE, publisher);
    }

    /// @notice Stage a version for publication. After [`PUBLISH_TIMELOCK`] elapses, call
    /// [`executeVersion`]. Re-proposing an id before execute overwrites the pending record and RESETS
    /// its timelock (mirrors `proposeZkVerifier`). `publishedAt`/`active` in `v` are IGNORED — they are
    /// stamped at execute — so a publisher cannot back-date or pre-activate a version.
    function proposeVersion(Version calldata v) external onlyRole(PUBLISHER_ROLE) {
        require(v.versionId != 0, "versionId=0");
        require(
            v.factory != address(0) && v.verificationRegistry != address(0) && v.sbt != address(0)
                && v.verifier != address(0),
            "zero trio/verifier"
        );
        require(v.circuitId != 0, "circuitId=0");
        // The zkey is the ceremony-bound artifact; a version with no zkey pin is unrepresentable (a
        // swapped key would silently prove against the wrong VK). The graph pin MAY be 0 (the .graph is
        // not committed — the mobile resolver pins it from the anchor at fetch time; §3.5), so it is
        // deliberately NOT required here.
        require(v.zkeySha256 != 0, "zkeySha256=0");

        _pending[v.versionId] = v;
        uint256 eta = block.timestamp + PUBLISH_TIMELOCK;
        publishEta[v.versionId] = eta;
        emit VersionProposed(v.versionId, eta);
    }

    /// @notice Execute a previously-proposed version once its timelock has elapsed. Appends to
    /// [`versionList`] only on FIRST publication of an id (a swap-republish updates in place, no dup).
    /// Stamps `publishedAt = block.timestamp` and `active = true` authoritatively.
    function executeVersion(bytes32 id) external onlyRole(PUBLISHER_ROLE) {
        uint256 eta = publishEta[id];
        require(eta != 0, "none pending");
        require(block.timestamp >= eta, "timelock");

        Version memory v = _pending[id];
        v.publishedAt = uint64(block.timestamp);
        v.active = true;

        bool isNew = versions[id].versionId == 0;
        if (isNew) {
            versionList.push(id);
        }
        versions[id] = v;

        delete _pending[id];
        delete publishEta[id];
        emit VersionPublished(id, isNew);
    }

    /// @notice Deprecate a published version: flips `active=false`, NEVER deletes (history is pinned so
    /// an old record still self-routes and verifies; §7.3). Not timelocked — deprecation is a safety
    /// lever. Idempotent-safe: reverts only on an unknown id, not on an already-deprecated one.
    function deprecateVersion(bytes32 id) external onlyRole(PUBLISHER_ROLE) {
        require(versions[id].versionId != 0, "unknown version");
        versions[id].active = false;
        emit VersionDeprecated(id);
    }

    /// @notice The FULL published record for `id`, including the string members the auto-getter omits.
    /// Fails closed on an unknown id (reverts) so a consumer never mistakes a zeroed struct for a valid
    /// version.
    function getVersion(bytes32 id) external view returns (Version memory) {
        Version memory v = versions[id];
        require(v.versionId != 0, "unknown version");
        return v;
    }

    /// @notice The full pending (proposed, not-yet-executed) record for `id`. Reverts if nothing is
    /// pending. Lets governance inspect exactly what an `executeVersion` would write during the window.
    function getPendingVersion(bytes32 id) external view returns (Version memory) {
        require(publishEta[id] != 0, "none pending");
        return _pending[id];
    }

    /// @notice How many distinct versions have ever been published (enumerate via `versionList`).
    function versionCount() external view returns (uint256) {
        return versionList.length;
    }
}
