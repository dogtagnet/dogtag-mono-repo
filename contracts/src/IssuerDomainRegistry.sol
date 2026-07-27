// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

/// @dev The role surface of the live `IssuerRegistry` (AccessControl). Read-only here — this contract
/// never writes to the registry, it only asks whether a caller holds the KYC/whitelist authority.
interface IIssuerRegistryRoles {
    function hasRole(bytes32 role, address account) external view returns (bool);
}

/// @dev The read surface of the live `DogTagIssuerFactory`. `predictIssuer` is the whole reason this
/// contract can authorize a clone's own business WITHOUT any new clone implementation — see the
/// "spawning business" note below.
interface IDogTagIssuerFactoryView {
    function isClone(address candidate) external view returns (bool);
    function predictIssuer(bytes32 recordType, address business) external view returns (address);
}

/// @dev The one clone getter this contract needs.
interface IDogTagIssuerView {
    function recordType() external view returns (bytes32);
}

/// @title IssuerDomainRegistry — the on-chain half of the bidirectional issuer↔domain binding.
///
/// @notice An issuer clone asserts "my domain is D" HERE, on chain; the domain's DNS zone asserts "clone
/// C is mine" by publishing `<clone-address-lowercase>._dogtag.D  TXT "<free-form description>"`.
/// Neither half alone is sufficient, which is the point: to forge a displayed issuer identity an
/// attacker needs BOTH a key authorized here AND control of that DNS zone.
///
/// # Why the domain lives on chain and not in the credential document
///
/// The wrapped document's top-level `issuer` block (`name`, `domain`, `documentStore`) is OUTSIDE the
/// Merkle root — `check_integrity` hashes only `data` plus `privacy.obfuscated`. A relabelled document
/// therefore verifies. Reading the domain from the chain instead removes that attack entirely: a
/// relabelled document cannot move a value it does not carry.
///
/// # Why this is a NEW contract rather than a field on the clone or a mapping on IssuerRegistry
///
/// Both of those are far more expensive than they look, and the cost is not "a migration story":
///
///   * A field on `DogTagIssuer` needs a new implementation, but `DogTagIssuerFactory.implementation`
///     is `immutable` — so it needs a NEW FACTORY. `VerificationRegistryConsent.rootIndex` is ALSO
///     `immutable` and resolves the issuing clone for every proof via `rootIndex.rootIssuer(R)`. A new
///     factory therefore leaves the live verification registry reading the OLD index, and every root
///     issued by a new clone resolves to `address(0)` — owner-hidden consent verification breaks for
///     every credential issued after the swap. That is a protocol-wide v2 (new trio + a timelocked
///     `ProtocolRegistry` republish), not a one-field change.
///   * A mapping on `IssuerRegistry` needs that registry redeployed (it is a plain, non-upgradeable
///     AccessControl contract), and every existing clone pins `registry` at `initialize` with NO
///     setter — so the clones would keep gating writes on the OLD registry while the domain lived in
///     the new one. Split-brain governance.
///
/// This contract redeploys NOTHING. It is purely additive, exactly like `ProtocolRegistry`.
///
/// # Who may write a binding (three tiers, most-trusted first)
///
///   1. `WHITELIST_ADMIN` on the live `IssuerRegistry` — the protocol/KYC operator. This is the tier
///      that writes the initial value, because the admin onboarding flow ALREADY collects and
///      DNS-checks `(domain, documentStore)` before whitelisting an issuer.
///   2. The **spawning business** — the address a clone was created for. The factory never stores it,
///      but `createIssuer` salts the clone with `keccak256(recordType, business)`, so
///      `factory.predictIssuer(clone.recordType(), msg.sender) == clone` PROVES the claim. It is a
///      one-way check (verify, not enumerate), which is all an authorization needs. This is the
///      self-service tier: the issuer updates its own domain with no operator in the loop.
///   3. An appointed `domainAdmin[clone]` — set once by `WHITELIST_ADMIN`, self-updating thereafter.
///      This tier exists because tier 2 is only as strong as provisioning discipline: clones spawned
///      with `business` defaulted to the operator's own signer authorize the OPERATOR under tier 2,
///      not the organisation. Already-salted clones cannot be re-salted, so `domainAdmin` is how an
///      existing issuer gets genuine self-service.
///
/// # The write side of a THREE-LINK CHAIN
///
/// A binding is only worth anything if the contract it names actually came from the DogTag factory:
/// otherwise anyone can deploy their own contract, claim a domain for it, publish a matching TXT record
/// and present as "verified" — DNS would agree, this registry would agree, and none of it would mean
/// anything, because that contract never passed through the KYC-gated `createIssuer`.
///
/// Every write here therefore requires `factory.isClone(clone)`, so a binding for a non-clone cannot be
/// STORED at all. That is necessary but not sufficient: a verifier must ALSO re-check provenance at
/// verification time rather than trusting that this registry once did. A stored binding is a CLAIM, and
/// an app should not inherit trust it did not verify itself.
///
/// # What this contract deliberately does NOT hold
///
/// The human-readable DESCRIPTION is not stored here. It is the DNS record's VALUE, written by whoever
/// controls the zone. Keeping it off-chain is what makes the binding bidirectional rather than two
/// copies of one party's claim.
///
/// # Absence is a normal state, not an error
///
/// Most issuers will have no binding on day one. `getBinding` therefore does NOT revert on an unknown
/// clone — it returns a record with `updatedAt == 0`, which is the unambiguous "no binding published"
/// discriminator (a written binding always stamps a non-zero `block.timestamp`, and an empty domain is
/// unrepresentable — see [`_requireValidDomain`]). Callers MUST branch on `updatedAt != 0` and must
/// never treat a zeroed record as a verified one.
contract IssuerDomainRegistry {
    /// @dev Mirrors `IssuerRegistry.WHITELIST_ADMIN` — the KYC/whitelist authority.
    bytes32 public constant WHITELIST_ADMIN = keccak256("WHITELIST_ADMIN");

    /// @notice The live `DogTagIssuerFactory` — the clone-provenance oracle (`isClone`/`predictIssuer`).
    IDogTagIssuerFactoryView public immutable factory;
    /// @notice The live `IssuerRegistry` — consulted ONLY for `WHITELIST_ADMIN` membership.
    IIssuerRegistryRoles public immutable issuerRegistry;

    /// @notice A published issuer→domain claim. `updatedAt == 0` means "no binding" (see contract doc).
    ///
    /// Both a TIMESTAMP and a BLOCK NUMBER are stored, and they answer different questions. The
    /// timestamp is what a human reads; the block is what makes the claim REPRODUCIBLE. Because DNS
    /// changes over time and a clone may be superseded, "this issuer claims moh.gov.sg" is only
    /// meaningful with a block anchor: with `updatedAtBlock` a verifier can ask "what domain did this
    /// clone claim at block N" — via an `eth_call` pinned to N — rather than only "what does it claim
    /// now". A verification that says "verified" without saying WHEN, against a mutable world, is not
    /// auditable.
    struct Binding {
        string domain; // canonical lowercase DNS name, e.g. "moh.gov.sg"
        uint64 updatedAt; // block.timestamp of the write; 0 == no binding published
        uint64 updatedAtBlock; // block.number of the write — the anchor for historical queries
        address setBy; // which authority actually wrote it (audit trail for the admin console)
    }

    mapping(address => Binding) private _bindings;

    /// @notice clone -> the address allowed to self-update that clone's binding (tier 3). Appointed by
    /// `WHITELIST_ADMIN`; `address(0)` means no appointee.
    mapping(address => address) public domainAdmin;

    /// @dev Every clone that has ever had a binding written, for enumeration by the future
    /// daily re-check cron. A cleared binding stays listed (its record is zeroed, not removed) so the
    /// cron can still observe that a domain was withdrawn.
    address[] public boundClones;
    mapping(address => bool) private _listed;

    event DomainSet(address indexed clone, string domain, address indexed setBy, uint64 atBlock);
    event DomainCleared(address indexed clone, address indexed clearedBy);
    event DomainAdminSet(address indexed clone, address indexed admin, address indexed setBy);

    error NotAClone();
    error NotAuthorized();
    error BadDomain();
    error ZeroAddress();

    /// @param factory_ the live `DogTagIssuerFactory`.
    /// @param issuerRegistry_ the live `IssuerRegistry` whose `WHITELIST_ADMIN` gates tiers 1 and 3.
    constructor(address factory_, address issuerRegistry_) {
        if (factory_ == address(0) || issuerRegistry_ == address(0)) revert ZeroAddress();
        factory = IDogTagIssuerFactoryView(factory_);
        issuerRegistry = IIssuerRegistryRoles(issuerRegistry_);
    }

    // ---------------------------------------------------------------------------------------------
    // Authorization
    // ---------------------------------------------------------------------------------------------

    /// @notice True iff `who` may write `clone`'s binding — the union of the three tiers.
    /// @dev Exposed as a view so the admin console can render "can this key self-serve?" without
    /// simulating a write.
    function canSetDomain(address clone, address who) public view returns (bool) {
        if (!factory.isClone(clone)) return false;
        if (issuerRegistry.hasRole(WHITELIST_ADMIN, who)) return true; // tier 1
        if (domainAdmin[clone] == who && who != address(0)) return true; // tier 3
        return _isSpawningBusiness(clone, who); // tier 2
    }

    /// @dev Tier 2: `who` is the business `clone` was created for. The factory salts a clone with
    /// `keccak256(recordType, business)`, so recomputing the deterministic address from
    /// (`clone.recordType()`, `who`) and comparing PROVES the claim without the clone storing anything.
    function _isSpawningBusiness(address clone, address who) internal view returns (bool) {
        if (who == address(0)) return false;
        bytes32 rt = IDogTagIssuerView(clone).recordType();
        return factory.predictIssuer(rt, who) == clone;
    }

    // ---------------------------------------------------------------------------------------------
    // Writes
    // ---------------------------------------------------------------------------------------------

    /// @notice Publish or replace `clone`'s domain claim. `domain` must already be canonical lowercase
    /// (see [`_requireValidDomain`]) — this contract rejects rather than silently normalizes, so what a
    /// resolver reads is exactly what the writer intended.
    function setDomain(address clone, string calldata domain) external {
        if (!factory.isClone(clone)) revert NotAClone();
        if (!canSetDomain(clone, msg.sender)) revert NotAuthorized();
        _requireValidDomain(domain);

        _bindings[clone] = Binding({
            domain: domain,
            updatedAt: uint64(block.timestamp),
            updatedAtBlock: uint64(block.number),
            setBy: msg.sender
        });
        if (!_listed[clone]) {
            _listed[clone] = true;
            boundClones.push(clone);
        }
        emit DomainSet(clone, domain, msg.sender, uint64(block.number));
    }

    /// @notice Withdraw `clone`'s domain claim (the issuer moved domains, or the binding was wrong).
    /// Zeroes the record back to the "no binding" state; the clone stays in `boundClones` so the
    /// re-check cron can observe the withdrawal.
    function clearDomain(address clone) external {
        if (!canSetDomain(clone, msg.sender)) revert NotAuthorized();
        delete _bindings[clone];
        emit DomainCleared(clone, msg.sender);
    }

    /// @notice Appoint (or, with `address(0)`, revoke) the key that may self-update `clone`'s binding.
    /// Tier 1 only — this is the operator handing self-service to a KYC'd organisation.
    function setDomainAdmin(address clone, address admin) external {
        if (!factory.isClone(clone)) revert NotAClone();
        if (!issuerRegistry.hasRole(WHITELIST_ADMIN, msg.sender)) revert NotAuthorized();
        domainAdmin[clone] = admin;
        emit DomainAdminSet(clone, admin, msg.sender);
    }

    // ---------------------------------------------------------------------------------------------
    // Reads
    // ---------------------------------------------------------------------------------------------

    /// @notice `clone`'s published claim. Does NOT revert on an unknown clone: "no domain claimed" is a
    /// normal day-one state, not an error. Branch on `updatedAt != 0`.
    function getBinding(address clone) external view returns (Binding memory) {
        return _bindings[clone];
    }

    /// @notice The claimed domain, or `""` when no binding is published.
    function domainOf(address clone) external view returns (string memory) {
        return _bindings[clone].domain;
    }

    /// @notice True iff `clone` has a published domain claim right now.
    function hasBinding(address clone) external view returns (bool) {
        return _bindings[clone].updatedAt != 0;
    }

    /// @notice How many clones have ever had a binding written (enumerate via `boundClones`).
    function boundCloneCount() external view returns (uint256) {
        return boundClones.length;
    }

    // ---------------------------------------------------------------------------------------------
    // Domain validation
    // ---------------------------------------------------------------------------------------------

    /// @dev Rejects anything a DNS resolver could not use verbatim as a query suffix. Enforcing this
    /// on-chain means the off-chain verifier never has to guess what a malformed claim meant: an
    /// unusable domain is unrepresentable rather than a silent lookup failure that would surface as
    /// "could not check" forever.
    ///
    /// Accepts lowercase LDH labels only (`a-z`, `0-9`, `-`), dot-separated, at least two labels, no
    /// empty label, no label starting or ending with `-`, total length <= 253. Uppercase is REJECTED
    /// rather than folded so the stored value is unambiguously the canonical query name.
    function _requireValidDomain(string calldata domain) internal pure {
        bytes calldata b = bytes(domain);
        if (b.length == 0 || b.length > 253) revert BadDomain();

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
