// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Ownable, Ownable2Step} from "@openzeppelin/contracts/access/Ownable2Step.sol";

/// @dev The only factory fact the authority core trusts. A factory address is resolved from a
/// registrar-pinned generation; callers never supply a factory alongside a service claim.
interface IProviderRegistryFactory {
    function isClone(address candidate) external view returns (bool);
}

/// @dev Owner-bearing issuer surface required of services attached to this generation.
interface IProviderRegistryService {
    function owner() external view returns (address);
    function recordType() external view returns (bytes32);
}

/// @notice Interface consumed by the owner-bearing issuer/factory generation.
interface IProviderRegistry {
    function canCreateService(bytes20 providerId, bytes32 recordType, address caller)
        external
        view
        returns (bool);

    function canIssue(address service, address signer) external view returns (bool);
    function canRevoke(address service, address signer) external view returns (bool);
    function isRecognizedIssuer(address service, address signer) external view returns (bool);
    function isWhitelistedFor(bytes32 key, address signer) external view returns (bool);
    function hasRole(bytes32 role, address account) external view returns (bool);
    function rightsOf(address account) external view returns (uint256);
}

/// @title ProviderRegistry — canonical provider identity and service authority core.
/// @notice Holds only stable identity, standing and authority facts. Domain claims and directory
/// content deliberately live in typed resolvers selected through this core.
///
/// `providerId` is an opaque KYC registrar assignment. It MUST NOT be derived from a provider name,
/// domain, controller, signer, service clone, factory salt or factory generation.
///
/// The contract intentionally has one authority key for this generation. `Ownable2Step` makes every
/// key switch request/accept, and `hasRole` projects that one current key into the two role reads
/// consumers make (`DEFAULT_ADMIN_ROLE` and `WHITELIST_ADMIN`). There is no independently grantable role
/// that could leave the old key behind after a rotation.
contract ProviderRegistry is Ownable2Step, IProviderRegistry {
    bytes32 public constant DEFAULT_ADMIN_ROLE = bytes32(0);
    bytes32 public constant WHITELIST_ADMIN = keccak256("WHITELIST_ADMIN");

    uint256 public constant MAX_PAGE_SIZE = 100;
    uint256 public constant MAX_CONTENTHASH_LENGTH = 256;

    uint32 public constant PROVIDER_PERMISSION_RECORD = 1 << 0;
    uint32 public constant PROVIDER_PERMISSION_DIRECTORY_RESOLVER = 1 << 1;
    uint32 public constant PROVIDER_PERMISSION_CREATE_SERVICE = 1 << 2;

    uint32 public constant SERVICE_PERMISSION_RECORD = 1 << 0;
    uint32 public constant SERVICE_PERMISSION_DOMAIN_RESOLVER = 1 << 1;
    uint32 public constant SERVICE_PERMISSION_REPOINT = 1 << 2;

    // ---------------------------------------------------------------------------------------------
    // Address rights bitmask — see `rightsOf`
    //
    // ONE FLAT NAMESPACE. Every bit below is pinned to this exact position FOREVER. A bit is never
    // reused for a second meaning and never reordered, because a bit whose meaning changed silently
    // grants the WRONG right: no revert, no error, just wrong. That is the same failure class as the
    // `getContractSet` -> `getDiscoverySet` rename this codebase already paid for, where a stale client
    // decoded one slot out of a record whose shape had moved underneath it.
    //
    // The remedy is the one this codebase already chose there: SHAPE AND GETTER NAME MOVE TOGETHER. If
    // this layout ever changes — a bit retired and its position recycled, the meanings reordered, the
    // stored/derived split moved — the lookup TAKES A NEW NAME with it, so a client built against the
    // old layout reverts at the dispatcher rather than reading a plausible wrong answer. Adding a bit at
    // a fresh, previously-unused position is the one change that does NOT require a rename: an old
    // client masks it off and reads every bit it knows about correctly.
    // ---------------------------------------------------------------------------------------------

    /// @notice May anchor new roots, and (with the further terms `canRevoke` folds) invalidate them.
    /// The ONLY stored bit: written by the registrar through `setRights`.
    uint256 public constant RIGHT_ISSUE = 1 << 0;

    /// @notice Holds this core's protocol-admin authority — exactly what `hasRole` answers for both
    /// role names. DERIVED from `owner()`, so a two-step key rotation moves it with no second write.
    uint256 public constant RIGHT_PROTOCOL_ADMIN = 1 << 1;

    /// @notice The registrar has attached this address as a service. DERIVED.
    uint256 public constant RIGHT_SERVICE_ATTACHED = 1 << 2;

    /// @notice This service's live `owner()` still equals the owner the registrar confirmed. DERIVED.
    uint256 public constant RIGHT_SERVICE_OWNER_CONFIRMED = 1 << 3;

    /// @notice This service, its provider, and its factory generation are all currently effective.
    /// DERIVED.
    uint256 public constant RIGHT_SERVICE_STANDING = 1 << 4;

    /// @notice This service is its provider's current selection for its record type. DERIVED.
    uint256 public constant RIGHT_SERVICE_CURRENT = 1 << 5;

    /// @notice Exactly the bits `setRights` accepts. Every other bit in `rightsOf` is DERIVED from state
    /// this core already holds, so writing one would be recording a claim rather than a grant — the
    /// stored value and the fact would then be free to disagree, with the stored one winning.
    uint256 public constant SETTABLE_RIGHTS = RIGHT_ISSUE;

    enum Standing {
        NONE,
        PENDING,
        ACTIVE,
        SUSPENDED,
        RETIRED
    }

    enum ResolverKind {
        DIRECTORY,
        DOMAIN
    }

    struct Provider {
        address controller;
        address pendingController;
        bool pendingControllerAccepted;
        bool pendingControllerRequestedByRegistrar;
        address directoryResolver;
        uint64 controllerEpoch;
        uint64 controllerRequestNonce;
        Standing standing;
    }

    /// @dev The contenthash must resolve only a publication-safe identity statement. Content
    /// addressing provides integrity, not privacy; private KYC submissions must never be anchored.
    struct PublicIdentityAnchor {
        bytes32 digest;
        uint32 schema;
        uint16 codec;
        uint8 hashAlgorithm;
        bytes contenthash;
        uint64 revision;
        uint64 updatedAtBlock;
    }

    struct FactoryGeneration {
        address factory;
        bool active;
        uint64 addedAtBlock;
        uint64 deprecatedAtBlock;
    }

    struct Service {
        bytes20 providerId;
        bytes32 factoryGeneration;
        bytes32 recordType;
        address confirmedOwner;
        address domainResolver;
        uint64 ownerEpoch;
        Standing standing;
    }

    struct Delegation {
        uint32 permissions;
        uint64 epoch;
        uint64 validUntil;
    }

    mapping(bytes20 => Provider) private _providers;
    mapping(bytes20 => PublicIdentityAnchor) private _publicIdentityAnchors;
    bytes20[] private _providerIds;

    mapping(bytes32 => FactoryGeneration) private _factoryGenerations;
    mapping(address => bytes32) public generationOfFactory;
    bytes32[] private _factoryGenerationIds;

    mapping(address => Service) private _services;
    address[] private _serviceAddresses;
    mapping(bytes20 => address[]) private _providerServices;
    mapping(bytes20 => mapping(address => uint256)) private _providerServiceIndex;
    mapping(bytes20 => mapping(bytes32 => address)) private _currentService;

    mapping(bytes20 => mapping(address => Delegation)) private _providerDelegates;
    mapping(address => mapping(address => Delegation)) private _serviceDelegates;

    /// @dev The registrar's grants, keyed on the ADDRESS alone. This is the whole re-keying: it was
    /// `_issuanceCapabilities[service][signer]`, and the service dimension is gone. Holds only bits in
    /// `SETTABLE_RIGHTS`; `rightsOf` masks anyway, so a future widening of that constant cannot
    /// retroactively activate a bit some historical write happened to leave set.
    mapping(address => uint256) private _grantedRights;

    /// @dev How many addresses currently hold `RIGHT_ISSUE`. It replaces the per-service
    /// `_activeIssuerCount`, which is not expressible once the grant carries no service: "some signer
    /// can issue here" is now a property of the whole registry rather than of one service. Kept so
    /// `effectiveService.hasActiveIssuer` still answers the question it always answered instead of
    /// quietly becoming a restatement of its four neighbours.
    uint256 private _issueRightHolders;

    mapping(bytes32 => mapping(address => bool)) private _verifierCapabilities;
    mapping(bytes20 => mapping(bytes32 => bool)) private _serviceCreationApprovals;

    mapping(ResolverKind => mapping(address => bool)) private _approvedResolvers;
    mapping(ResolverKind => address[]) private _resolverAddresses;
    mapping(ResolverKind => mapping(address => bool)) private _resolverListed;

    event ProviderRegistered(bytes20 indexed providerId, address indexed controller);
    event ProviderStandingChanged(bytes20 indexed providerId, Standing oldStanding, Standing newStanding);
    event PublicIdentityAnchorSet(
        bytes20 indexed providerId,
        bytes32 indexed oldDigest,
        bytes32 indexed newDigest,
        uint32 schema,
        uint16 codec,
        uint8 hashAlgorithm,
        bytes contenthash,
        uint64 revision,
        uint64 atBlock
    );
    event ControllerTransferRequested(
        bytes20 indexed providerId,
        address indexed currentController,
        address indexed pendingController,
        address requestedBy,
        uint64 requestNonce
    );
    event ControllerTransferAccepted(
        bytes20 indexed providerId, address indexed pendingController, uint64 requestNonce
    );
    event ControllerTransferCancelled(
        bytes20 indexed providerId,
        address indexed currentController,
        address indexed cancelledController,
        address cancelledBy,
        uint64 requestNonce
    );
    event ControllerTransferConfirmed(
        bytes20 indexed providerId,
        address indexed oldController,
        address indexed newController,
        uint64 controllerEpoch
    );
    event ProviderDelegateSet(
        bytes20 indexed providerId,
        address indexed delegate,
        uint32 permissions,
        uint64 validUntil,
        uint64 controllerEpoch
    );

    event FactoryGenerationAdded(bytes32 indexed generationId, address indexed factory, uint64 atBlock);
    event FactoryGenerationDeprecated(bytes32 indexed generationId, address indexed factory, uint64 atBlock);

    event ServiceAttached(
        address indexed service,
        bytes20 indexed providerId,
        bytes32 indexed factoryGeneration,
        bytes32 recordType,
        address confirmedOwner
    );
    event ServiceProviderReassigned(
        address indexed service,
        bytes20 indexed oldProviderId,
        bytes20 indexed newProviderId,
        address reassignedBy
    );
    event ServiceStandingChanged(address indexed service, Standing oldStanding, Standing newStanding);
    event ServiceOwnerConfirmed(
        address indexed service, address indexed oldOwner, address indexed newOwner, uint64 ownerEpoch
    );
    event ServiceDelegateSet(
        address indexed service,
        address indexed delegate,
        uint32 permissions,
        uint64 validUntil,
        uint64 ownerEpoch
    );
    event CurrentServiceChanged(
        bytes20 indexed providerId,
        bytes32 indexed recordType,
        address indexed oldService,
        address newService,
        address setBy
    );

    /// @dev The whole grant history for one address, in this core's own log. `rights` is the address's
    /// COMPLETE settable mask after the write, not a delta, so folding the sequence needs no prior
    /// state — the last event at or before a block IS the mask in force then. That is what the
    /// at-issuance verification pillar reconstructs, and why the mask (not a per-bit bool) is emitted:
    /// a reader interested in one bit masks the word, and a bit added later needs no second event.
    event RightsSet(address indexed account, uint256 rights);
    event VerifierCapabilitySet(
        bytes32 indexed purpose, bytes32 indexed compatibilityKey, address indexed relayer, bool allowed
    );
    event ServiceCreationApprovalSet(bytes20 indexed providerId, bytes32 indexed recordType, bool allowed);

    event ResolverApprovalSet(ResolverKind indexed kind, address indexed resolver, bool approved);
    event DirectoryResolverSet(
        bytes20 indexed providerId, address indexed oldResolver, address indexed newResolver, address setBy
    );
    event DomainResolverSet(
        address indexed service, address indexed oldResolver, address indexed newResolver, address setBy
    );

    error ZeroAddress();
    error ZeroProviderId();
    error AlreadyRegistered();
    error UnknownProvider();
    error UnknownService();
    error UnknownFactoryGeneration();
    error DuplicateFactory();
    error FactoryGenerationInactive();
    error NotFactoryClone();
    error InvalidServiceMetadata();
    error Unauthorized();
    error InvalidStanding();
    error RetiredStanding();
    error NoChange();
    /// @dev `setRights` was handed a bit outside `SETTABLE_RIGHTS` — a derived bit is not a grant.
    error UnsettableRights();
    error NoPendingController();
    error PendingControllerNotAccepted();
    error NotPendingController();
    error UnexpectedControllerTransfer();
    error UnexpectedServiceOwner();
    error UnexpectedServiceProvider();
    error BadIdentityAnchor();
    error ContenthashTooLong();
    error ResolverNotApproved();
    error BadPage();
    error RenounceDisabled();

    constructor(address authority) Ownable(authority) {}

    /// @notice Disables the inherited one-transaction path that would strand every registrar surface.
    function renounceOwnership() public pure override {
        revert RenounceDisabled();
    }

    /// @notice Exact AccessControl-shaped compatibility view used by issuer revocation and the domain
    /// resolver. Both role names intentionally resolve to the same two-step authority key.
    function hasRole(bytes32 role, address account) public view override returns (bool) {
        return account == owner() && (role == DEFAULT_ADMIN_ROLE || role == WHITELIST_ADMIN);
    }

    // ---------------------------------------------------------------------------------------------
    // Provider identity and controller authority
    // ---------------------------------------------------------------------------------------------

    function registerProvider(
        bytes20 providerId,
        address controller,
        bytes32 publicIdentityDigest,
        uint32 publicIdentitySchema,
        uint16 codec,
        uint8 hashAlgorithm,
        bytes calldata contenthash
    ) external onlyOwner {
        if (providerId == bytes20(0)) revert ZeroProviderId();
        if (controller == address(0)) revert ZeroAddress();
        if (_providers[providerId].controller != address(0)) revert AlreadyRegistered();

        _providers[providerId] = Provider({
            controller: controller,
            pendingController: address(0),
            pendingControllerAccepted: false,
            pendingControllerRequestedByRegistrar: false,
            directoryResolver: address(0),
            controllerEpoch: 1,
            controllerRequestNonce: 0,
            standing: Standing.PENDING
        });
        _providerIds.push(providerId);
        emit ProviderRegistered(providerId, controller);
        _setPublicIdentityAnchor(
            providerId, publicIdentityDigest, publicIdentitySchema, codec, hashAlgorithm, contenthash
        );
    }

    function setPublicIdentityAnchor(
        bytes20 providerId,
        bytes32 digest,
        uint32 schema,
        uint16 codec,
        uint8 hashAlgorithm,
        bytes calldata contenthash
    ) external onlyOwner {
        _requireProvider(providerId);
        _setPublicIdentityAnchor(providerId, digest, schema, codec, hashAlgorithm, contenthash);
    }

    function _setPublicIdentityAnchor(
        bytes20 providerId,
        bytes32 digest,
        uint32 schema,
        uint16 codec,
        uint8 hashAlgorithm,
        bytes calldata contenthash
    ) internal {
        if (digest == bytes32(0) || schema == 0 || hashAlgorithm == 0) {
            revert BadIdentityAnchor();
        }
        if (contenthash.length > MAX_CONTENTHASH_LENGTH) revert ContenthashTooLong();

        PublicIdentityAnchor storage anchor = _publicIdentityAnchors[providerId];
        bytes32 oldDigest = anchor.digest;
        uint64 revision = anchor.revision + 1;
        anchor.digest = digest;
        anchor.schema = schema;
        anchor.codec = codec;
        anchor.hashAlgorithm = hashAlgorithm;
        anchor.contenthash = contenthash;
        anchor.revision = revision;
        anchor.updatedAtBlock = uint64(block.number);
        emit PublicIdentityAnchorSet(
            providerId,
            oldDigest,
            digest,
            schema,
            codec,
            hashAlgorithm,
            contenthash,
            revision,
            uint64(block.number)
        );
    }

    function setProviderStanding(bytes20 providerId, Standing newStanding) external onlyOwner {
        Provider storage p = _requireProvider(providerId);
        _requireSettableStanding(newStanding);
        if (p.standing == Standing.RETIRED) revert RetiredStanding();
        if (p.standing == newStanding) revert NoChange();
        Standing oldStanding = p.standing;
        p.standing = newStanding;
        emit ProviderStandingChanged(providerId, oldStanding, newStanding);
    }

    /// @notice A normal transfer is requested by the current controller. The registrar may also
    /// request a recovery transfer, but cannot complete it until the candidate proves key control.
    function requestControllerTransfer(bytes20 providerId, address nextController) external {
        Provider storage p = _requireProvider(providerId);
        bool requestedByRegistrar = msg.sender == owner();
        if (!requestedByRegistrar && msg.sender != p.controller) revert Unauthorized();
        // A compromised controller cannot race or continually reset the registrar's recovery path.
        if (!requestedByRegistrar && p.pendingControllerRequestedByRegistrar) revert Unauthorized();
        if (nextController == address(0)) revert ZeroAddress();
        if (nextController == p.controller) revert NoChange();
        p.pendingController = nextController;
        p.pendingControllerAccepted = false;
        p.pendingControllerRequestedByRegistrar = requestedByRegistrar;
        p.controllerRequestNonce++;
        emit ControllerTransferRequested(
            providerId, p.controller, nextController, msg.sender, p.controllerRequestNonce
        );
    }

    function acceptControllerTransfer(bytes20 providerId) external {
        Provider storage p = _requireProvider(providerId);
        if (msg.sender != p.pendingController) revert NotPendingController();
        p.pendingControllerAccepted = true;
        emit ControllerTransferAccepted(providerId, msg.sender, p.controllerRequestNonce);
    }

    /// @notice Lets the registrar withdraw a stale or mistaken recovery request without forcing the
    /// candidate to accept it. Incrementing the nonce invalidates any reviewed confirmation call.
    function cancelControllerTransfer(bytes20 providerId) external onlyOwner {
        Provider storage p = _requireProvider(providerId);
        address cancelledController = p.pendingController;
        if (cancelledController == address(0)) revert NoPendingController();
        p.pendingController = address(0);
        p.pendingControllerAccepted = false;
        p.pendingControllerRequestedByRegistrar = false;
        p.controllerRequestNonce++;
        emit ControllerTransferCancelled(
            providerId, p.controller, cancelledController, msg.sender, p.controllerRequestNonce
        );
    }

    /// @notice KYC/recovery confirmation is the only step that changes the controller or its epoch.
    /// The expected address and request nonce are transaction guards; the stored accepted request
    /// remains authoritative and a replacement between review and mining makes this call revert.
    function confirmControllerTransfer(
        bytes20 providerId,
        address expectedController,
        uint64 expectedRequestNonce
    ) external onlyOwner {
        Provider storage p = _requireProvider(providerId);
        if (p.pendingController == address(0) || !p.pendingControllerAccepted) {
            revert PendingControllerNotAccepted();
        }
        if (p.pendingController != expectedController || p.controllerRequestNonce != expectedRequestNonce) {
            revert UnexpectedControllerTransfer();
        }
        address oldController = p.controller;
        address newController = p.pendingController;
        p.controller = newController;
        p.pendingController = address(0);
        p.pendingControllerAccepted = false;
        p.pendingControllerRequestedByRegistrar = false;
        p.controllerEpoch++;
        emit ControllerTransferConfirmed(providerId, oldController, newController, p.controllerEpoch);
    }

    function setProviderDelegate(bytes20 providerId, address delegate, uint32 permissions, uint64 validUntil)
        external
    {
        Provider storage p = _requireProvider(providerId);
        if (msg.sender != p.controller) revert Unauthorized();
        if (delegate == address(0)) revert ZeroAddress();
        if (permissions != 0 && p.standing != Standing.ACTIVE) revert Unauthorized();
        if (permissions != 0 && validUntil != 0 && validUntil <= block.timestamp) revert Unauthorized();

        if (permissions == 0) {
            delete _providerDelegates[providerId][delegate];
            validUntil = 0;
        } else {
            _providerDelegates[providerId][delegate] =
                Delegation({permissions: permissions, epoch: p.controllerEpoch, validUntil: validUntil});
        }
        emit ProviderDelegateSet(providerId, delegate, permissions, validUntil, p.controllerEpoch);
    }

    function canWriteProvider(bytes20 providerId, address caller, uint32 permission)
        public
        view
        returns (bool)
    {
        Provider storage p = _providers[providerId];
        if (permission == 0 || p.standing != Standing.ACTIVE) return false;
        if (caller == p.controller) return true;
        Delegation storage d = _providerDelegates[providerId][caller];
        return d.epoch == p.controllerEpoch && d.epoch != 0 && (d.permissions & permission) == permission
            && (d.validUntil == 0 || block.timestamp <= d.validUntil);
    }

    // ---------------------------------------------------------------------------------------------
    // Factory generations and service attachment
    // ---------------------------------------------------------------------------------------------

    /// @notice Pins a generation to one factory forever. Deprecation is separate and terminal; there
    /// is deliberately no function that can replace `factory` for an existing generation ID.
    function addFactoryGeneration(bytes32 generationId, address factory) external onlyOwner {
        if (generationId == bytes32(0)) revert UnknownFactoryGeneration();
        if (factory == address(0)) revert ZeroAddress();
        if (factory.code.length == 0) revert InvalidServiceMetadata();
        if (_factoryGenerations[generationId].factory != address(0)) revert AlreadyRegistered();
        if (generationOfFactory[factory] != bytes32(0)) revert DuplicateFactory();

        _factoryGenerations[generationId] = FactoryGeneration({
            factory: factory, active: true, addedAtBlock: uint64(block.number), deprecatedAtBlock: 0
        });
        generationOfFactory[factory] = generationId;
        _factoryGenerationIds.push(generationId);
        emit FactoryGenerationAdded(generationId, factory, uint64(block.number));
    }

    /// @notice Terminal and generation-wide: there is deliberately no reactivation path. Beyond
    /// stopping new issuance it freezes EVERY service-level write on that generation, because
    /// `canWriteService` requires `_serviceStandingIsEffective`, which requires an active generation.
    /// `setDomainResolver` is included, so a service's last-selected domain resolver can no longer be
    /// changed or cleared - not by its owner, not by a delegate, and deliberately not by a registrar
    /// override, which would be an authority to rewrite a frozen service's published claims.
    ///
    /// The provider axis is deliberately ASYMMETRIC: `Provider.directoryResolver` stays writable and
    /// clearable throughout, because `canWriteProvider` consults provider standing only and a provider
    /// outlives any one generation of its services. That asymmetry is the intended shape rather than an
    /// oversight - a frozen service's stored selector is history, not a live claim (see
    /// `setDomainResolver`), and withdrawing what such a selector still resolves is the typed resolver
    /// allowlist's job, not this write predicate's.
    function deprecateFactoryGeneration(bytes32 generationId) external onlyOwner {
        FactoryGeneration storage generation = _requireFactoryGeneration(generationId);
        if (!generation.active) revert FactoryGenerationInactive();
        generation.active = false;
        generation.deprecatedAtBlock = uint64(block.number);
        emit FactoryGenerationDeprecated(generationId, generation.factory, uint64(block.number));
    }

    /// @notice Registrar attachment accepts no claimed factory or record type. It resolves the
    /// pinned factory from `generationId`, proves `factory.isClone(service)`, then reads immutable
    /// `recordType()` and live `owner()` from the service itself. `expectedOwner` is only a
    /// transaction guard against a second ownership handover after KYC review; the resolved owner
    /// remains authoritative.
    function attachService(
        bytes20 providerId,
        address serviceAddress,
        bytes32 generationId,
        address expectedOwner
    ) external onlyOwner {
        _requireProvider(providerId);
        if (serviceAddress == address(0)) revert ZeroAddress();
        if (expectedOwner == address(0)) revert ZeroAddress();
        if (_services[serviceAddress].providerId != bytes20(0)) revert AlreadyRegistered();

        FactoryGeneration storage generation = _requireFactoryGeneration(generationId);
        if (!generation.active) revert FactoryGenerationInactive();
        if (!_factoryRecognizes(generation.factory, serviceAddress)) revert NotFactoryClone();

        (bool metadataOk, bytes32 recordType, address liveOwner) = _readServiceMetadata(serviceAddress);
        if (!metadataOk || recordType == bytes32(0) || liveOwner == address(0)) {
            revert InvalidServiceMetadata();
        }
        if (liveOwner != expectedOwner) revert UnexpectedServiceOwner();

        _services[serviceAddress] = Service({
            providerId: providerId,
            factoryGeneration: generationId,
            recordType: recordType,
            confirmedOwner: liveOwner,
            domainResolver: address(0),
            ownerEpoch: 1,
            standing: Standing.PENDING
        });
        _serviceAddresses.push(serviceAddress);
        _addProviderService(providerId, serviceAddress);
        emit ServiceAttached(serviceAddress, providerId, generationId, recordType, liveOwner);
    }

    /// @notice Corrects the one attachment fact chain provenance cannot verify. Factory generation,
    /// record type and confirmed owner stay resolved from the clone and are untouched here; only the
    /// registrar-supplied provider binding moves. The mistaken provider's current pointer is cleared
    /// rather than carried across, and the corrected provider's pointer is deliberately NOT set —
    /// publication remains the clone owner's `repointService` decision, so a correction can never
    /// publish a service under a provider that never selected it.
    function reassignServiceProvider(
        address serviceAddress,
        bytes20 expectedProviderId,
        bytes20 newProviderId
    ) external onlyOwner {
        Service storage s = _requireService(serviceAddress);
        _requireProvider(newProviderId);
        bytes20 oldProviderId = s.providerId;
        if (oldProviderId != expectedProviderId) revert UnexpectedServiceProvider();
        if (oldProviderId == newProviderId) revert NoChange();

        if (_currentService[oldProviderId][s.recordType] == serviceAddress) {
            delete _currentService[oldProviderId][s.recordType];
            emit CurrentServiceChanged(
                oldProviderId, s.recordType, serviceAddress, address(0), msg.sender
            );
        }
        _removeProviderService(oldProviderId, serviceAddress);
        s.providerId = newProviderId;
        _addProviderService(newProviderId, serviceAddress);
        emit ServiceProviderReassigned(serviceAddress, oldProviderId, newProviderId, msg.sender);
    }

    function setServiceStanding(address serviceAddress, Standing newStanding) external onlyOwner {
        Service storage s = _requireService(serviceAddress);
        _requireSettableStanding(newStanding);
        if (s.standing == Standing.RETIRED) revert RetiredStanding();
        if (s.standing == newStanding) revert NoChange();
        Standing oldStanding = s.standing;
        s.standing = newStanding;
        emit ServiceStandingChanged(serviceAddress, oldStanding, newStanding);
    }

    /// @notice Confirms a completed two-step transfer on the clone. The resolved `owner()` is
    /// authoritative; `expectedOwner` only guards the registrar-reviewed transaction. Until this
    /// call, live != confirmed quarantines all service writes and issuance.
    function confirmServiceOwner(address serviceAddress, address expectedOwner, uint64 expectedOwnerEpoch)
        external
        onlyOwner
    {
        Service storage s = _requireService(serviceAddress);
        (bool ok, address liveOwner) = _readServiceOwner(serviceAddress);
        if (!ok || liveOwner == address(0)) revert InvalidServiceMetadata();
        // The expected values guard the registrar's reviewed transition against a second clone
        // rotation or concurrent confirmation. Authority still comes from the resolved owner().
        if (liveOwner != expectedOwner || s.ownerEpoch != expectedOwnerEpoch) {
            revert UnexpectedServiceOwner();
        }
        if (liveOwner == s.confirmedOwner) revert NoChange();
        address oldOwner = s.confirmedOwner;
        s.confirmedOwner = liveOwner;
        s.ownerEpoch++;
        emit ServiceOwnerConfirmed(serviceAddress, oldOwner, liveOwner, s.ownerEpoch);
    }

    function setServiceDelegate(
        address serviceAddress,
        address delegate,
        uint32 permissions,
        uint64 validUntil
    ) external {
        Service storage s = _requireService(serviceAddress);
        (bool ownerOk, address liveOwner) = _readServiceOwner(serviceAddress);
        if (!ownerOk || msg.sender != liveOwner || liveOwner != s.confirmedOwner) revert Unauthorized();
        if (delegate == address(0)) revert ZeroAddress();
        if (permissions != 0 && !_serviceStandingIsEffective(serviceAddress, s)) revert Unauthorized();
        if (permissions != 0 && validUntil != 0 && validUntil <= block.timestamp) revert Unauthorized();

        if (permissions == 0) {
            delete _serviceDelegates[serviceAddress][delegate];
            validUntil = 0;
        } else {
            _serviceDelegates[serviceAddress][delegate] =
                Delegation({permissions: permissions, epoch: s.ownerEpoch, validUntil: validUntil});
        }
        emit ServiceDelegateSet(serviceAddress, delegate, permissions, validUntil, s.ownerEpoch);
    }

    /// @notice Captain-approved two-predicate authorization:
    /// (1) the attached provider/service is currently cleared; AND
    /// (2) caller is the confirmed live clone owner or an owner-epoch-scoped delegate.
    /// An issuance signer, provider controller or registrar receives no ordinary content-write bypass.
    function canWriteService(address serviceAddress, address caller, uint32 permission)
        public
        view
        returns (bool)
    {
        Service storage s = _services[serviceAddress];
        if (permission == 0 || !_serviceStandingIsEffective(serviceAddress, s)) return false;
        (bool ownerOk, address liveOwner) = _readServiceOwner(serviceAddress);
        if (!ownerOk || liveOwner != s.confirmedOwner) return false;
        if (caller == liveOwner) return true;
        Delegation storage d = _serviceDelegates[serviceAddress][caller];
        return d.epoch == s.ownerEpoch && d.epoch != 0 && (d.permissions & permission) == permission
            && (d.validUntil == 0 || block.timestamp <= d.validUntil);
    }

    /// @notice Selects the provider's current service for a record type. The candidate must already
    /// be registrar-attached, and `canWriteService` re-resolves its factory provenance and live owner.
    /// Historical roots are never resolved through this pointer.
    function repointService(address serviceAddress) external {
        Service storage s = _requireService(serviceAddress);
        if (!canWriteService(serviceAddress, msg.sender, SERVICE_PERMISSION_REPOINT)) {
            revert Unauthorized();
        }
        address oldService = _currentService[s.providerId][s.recordType];
        if (oldService == serviceAddress) revert NoChange();
        _currentService[s.providerId][s.recordType] = serviceAddress;
        emit CurrentServiceChanged(s.providerId, s.recordType, oldService, serviceAddress, msg.sender);
    }

    // ---------------------------------------------------------------------------------------------
    // Capabilities: service issuance and orthogonal verifier relayers
    // ---------------------------------------------------------------------------------------------

    /// @notice Grants or withdraws rights on ONE ADDRESS. `rights` is the address's complete settable
    /// mask afterwards, so withdrawing everything is `setRights(account, 0)`.
    ///
    /// The address is deliberately not classified. It may be an externally-owned account or a contract,
    /// and this function does not look at `account.code.length` to find out — for two reasons, and the
    /// second is the one that matters. Such a check is UNRELIABLE (a contract calling from inside its
    /// own constructor reports zero code length and reads as an EOA), and it is WRONG FOR THE PRODUCT:
    /// that indifference is exactly what lets a clinic — a company contract, or a multisig — and an
    /// individual practitioner on a plain wallet participate on identical terms.
    ///
    /// Only bits in `SETTABLE_RIGHTS` may be written; every other bit `rightsOf` reports is derived from
    /// state this core already holds and is not a grant to make.
    ///
    /// A grant made here is NOT scoped to a service, and that is the point rather than an omission: at
    /// the moment a registrar approves an applicant, that applicant's clone does not exist yet, so there
    /// is nothing to key a grant against. See `rightsOf` for what the unscoped grant widens.
    function setRights(address account, uint256 rights) external onlyOwner {
        if (account == address(0)) revert ZeroAddress();
        if (rights & ~SETTABLE_RIGHTS != 0) revert UnsettableRights();
        uint256 oldRights = _grantedRights[account];
        if (oldRights == rights) revert NoChange();

        bool hadIssue = oldRights & RIGHT_ISSUE != 0;
        bool hasIssue = rights & RIGHT_ISSUE != 0;
        if (hasIssue && !hadIssue) {
            _issueRightHolders++;
        } else if (!hasIssue && hadIssue) {
            _issueRightHolders--;
        }

        _grantedRights[account] = rights;
        emit RightsSet(account, rights);
    }

    /// @notice THE rights lookup: one address in, one bitmask out, and nothing else.
    ///
    /// It takes no service, no record type, no purpose and no caller context. It answers "what may this
    /// address do AT ALL", never "may this address act through that particular contract". Where narrower
    /// authority is genuinely required, the caller combines this answer with what it already knows —
    /// which is what `canIssue` and `canRevoke` below do with the service facts this core holds, and what
    /// a clone would do with its own `owner()`. Adding a scoping parameter here would collapse that
    /// separation back into the lookup.
    ///
    /// The bits are pinned at the constants above; read the never-reuse/never-reorder rule there before
    /// changing anything about this layout.
    ///
    /// The same function serves an EOA and a contract, and it never asks which it is holding. A plain
    /// wallet typically reports the granted bits alone; an address the registrar has attached as a
    /// service additionally reports that service's live lifecycle bits. The difference comes from what
    /// this core KNOWS about the address — a registrar fact — never from sniffing its code.
    ///
    /// # What the unscoped grant widens, stated rather than left to be discovered
    ///
    /// `RIGHT_ISSUE` was `_issuanceCapabilities[service][signer]`. A signer holding it now holds it
    /// wherever it is checked, so it may anchor a root on ANY service in effective standing, not only on
    /// the one it was approved alongside. `canIssue` still folds every lifecycle term it ever folded —
    /// none was dropped — but none of those terms is about WHICH provider the signer belongs to, because
    /// after this change no such term exists. The verification pillar reads this same grant from the
    /// `RightsSet` log and therefore answers `Authorized` for such a root: correct under this model, and
    /// a genuine reduction against the per-service grant it replaces.
    function rightsOf(address account) public view override returns (uint256 rights) {
        rights = _grantedRights[account] & SETTABLE_RIGHTS;
        if (account == owner()) rights |= RIGHT_PROTOCOL_ADMIN;

        Service storage s = _services[account];
        // Not an attached service: there are no service facts to report, and the external reads below
        // would be made against an address this core knows nothing about.
        if (s.providerId == bytes20(0)) return rights;

        rights |= RIGHT_SERVICE_ATTACHED;
        if (_isOwnerConfirmed(account, s)) rights |= RIGHT_SERVICE_OWNER_CONFIRMED;
        if (_serviceStandingIsEffective(account, s)) rights |= RIGHT_SERVICE_STANDING;
        if (_currentService[s.providerId][s.recordType] == account) rights |= RIGHT_SERVICE_CURRENT;
        return rights;
    }

    /// The three issuance-axis reads are a deliberate ladder, widest first, and each rung answers a
    /// different question. `isRecognizedIssuer` ⊇ `canRevoke` ⊇ `canIssue`. Never substitute one for
    /// another: the extra terms each rung folds are exactly the terms that would be wrong to fold
    /// into the rung above it.

    /// @notice The issuer-status question in the PRESENT TENSE, with `whitelistFor`/`delistFor`
    /// semantics. It folds exactly two facts: the registrar attached this clone, and the registrar's
    /// forward-only `RIGHT_ISSUE` grant for this signer still stands.
    ///
    /// The `serviceAddress` argument survives the re-keying to `rightsOf` and still carries the FIRST of
    /// those two facts — the clone is attached here at all — so a stranger address is still refused. What
    /// it no longer does is select WHICH grant is read: there is one grant per signer now, so this rung
    /// answers the same for every attached service. That is the widening, and `rightsOf` states it.
    ///
    /// CORRECTION, and it matters because the earlier wording sent a reader the wrong way. This was
    /// documented as "THE historical issuer-status question, and the only sound migration target for
    /// a direct `isWhitelistedFor(recordTypeKey, signer)` reader such as the mandatory issuer-whitelist
    /// verification pillar". It is NOT: the pillar asks whether a grant was in force AT THE BLOCK A ROOT
    /// WAS ANCHORED, reconstructed from this contract's own `RightsSet` log, because withdrawing a grant
    /// is forward-only and a current-state read would refuse every credential a since-rotated signer ever
    /// issued (`adminRevoke` is the retroactive lever, not this).
    ///
    /// This function reads CURRENT STORAGE — `_grantedRights[signer]`, with no block and no root
    /// parameter — so it cannot answer that question and is NOT the pillar's migration target. Consuming
    /// its boolean there would revert the pillar to a current-state getter under a new name. Verifiers use
    /// it only to establish that an authority speaks this generation's vocabulary at all; the historical
    /// question is answered from this contract's own `RightsSet` log.
    ///
    /// It remains the sound target for a PRESENT-TENSE record-type reader that is not a pre-issue
    /// gate — a console asking "is this signer a recognized issuer for this service now".
    ///
    /// It folds NO lifecycle or liveness state, deliberately. A repoint, a KYC suspension, a service
    /// retirement, a generation deprecation and an unconfirmed clone-owner handover all say nothing
    /// about whether a root anchored earlier was genuinely issued — yet the pillar reads a definite
    /// `false` as "resolved but not authorized", an authenticity failure that REFUSES the credential.
    /// Folding any of them here would turn every ordinary lifecycle event into a fleet-wide forgery
    /// verdict against genuine credentials. Only an explicit registrar revocation of the grant flips
    /// this, exactly as `delistFor` did.
    ///
    /// It reads storage alone and makes NO external call, which matters for a verification input: a
    /// clone whose `owner()` reverts or answers a malformed word cannot make this rung unanswerable.
    function isRecognizedIssuer(address serviceAddress, address signer)
        public
        view
        override
        returns (bool)
    {
        return _isRecognizedIssuer(_services[serviceAddress], signer);
    }

    /// @notice Preserves an originator's ability to revoke roots on a replaced service without
    /// reopening that service for new issuance. `DogTagIssuer.revoke` calls this path; a single
    /// `isWhitelistedFor` selector cannot distinguish an issue call from a revoke call.
    ///
    /// Above `isRecognizedIssuer` it adds only the confirmed-live-owner term, which the registrar can
    /// clear at any time — `confirmServiceOwner` is gated by neither standing nor generation. Every
    /// term it deliberately omits is either irreversible (a deprecated factory generation, a RETIRED
    /// standing)
    /// or a review state that must not block invalidation (a suspended provider, a repointed pointer).
    /// Revocation is the safety direction: no lifecycle event may strand a root as unrevocable by the
    /// originator that anchored it.
    function canRevoke(address serviceAddress, address signer) public view override returns (bool) {
        Service storage s = _services[serviceAddress];
        return _isRecognizedIssuer(s, signer) && _isOwnerConfirmed(serviceAddress, s);
    }

    /// @notice CURRENT eligibility to mint a NEW root: everything `canRevoke` requires, plus every
    /// live lifecycle term — provider and service standing, an active factory generation, and the
    /// provider's current pointer for this record type. It answers "may this signer issue now", so it
    /// is a pre-issue gate ONLY and never a verification input.
    function canIssue(address serviceAddress, address signer) public view override returns (bool) {
        Service storage s = _services[serviceAddress];
        return _isRecognizedIssuer(s, signer) && _serviceIssuanceEligible(serviceAddress, s);
    }

    /// @dev Takes no service ADDRESS any more, only the attached service RECORD. That is the re-keying
    /// visible in a signature: the address used to select which grant was read, and there is now one
    /// grant per signer. The record is still required, so an address the registrar never attached is
    /// still refused.
    function _isRecognizedIssuer(Service storage s, address signer) internal view returns (bool) {
        return s.providerId != bytes20(0) && (_grantedRights[signer] & RIGHT_ISSUE) != 0;
    }

    /// @dev The signer-agnostic half of `canIssue`, shared with `effectiveService` so the gate and the
    /// oversight read cannot drift: a read that folded fewer terms than the gate would report a
    /// service as ready to issue while every issuance through it reverts.
    function _serviceIssuanceEligible(address serviceAddress, Service storage s)
        internal
        view
        returns (bool)
    {
        return _isOwnerConfirmed(serviceAddress, s) && _serviceStandingIsEffective(serviceAddress, s)
            && _currentService[s.providerId][s.recordType] == serviceAddress;
    }

    /// @dev Fails closed intrinsically rather than relying on each caller evaluating some other term
    /// first. An address the registrar never attached whose `owner()` answers the zero address would
    /// otherwise match its unwritten `confirmedOwner` and report a confirmed owner for a service that
    /// does not exist here. Neither added guard can fire for an attached service - `attachService` and
    /// `confirmServiceOwner` both refuse a zero `owner()`, and a later renounce to zero already fails
    /// the equality - so this removes the fail-open without narrowing any live answer.
    function _isOwnerConfirmed(address serviceAddress, Service storage s) internal view returns (bool) {
        if (s.providerId == bytes20(0)) return false;
        (bool ownerOk, address liveOwner) = _readServiceOwner(serviceAddress);
        return ownerOk && liveOwner != address(0) && liveOwner == s.confirmedOwner;
    }

    function setVerifierCapability(bytes32 purpose, address relayer, bool allowed) external onlyOwner {
        if (relayer == address(0)) revert ZeroAddress();
        bytes32 key = verificationKey(purpose);
        if (_verifierCapabilities[key][relayer] == allowed) revert NoChange();
        _verifierCapabilities[key][relayer] = allowed;
        emit VerifierCapabilitySet(purpose, key, relayer, allowed);
    }

    function canVerify(bytes32 purpose, address relayer) public view returns (bool) {
        return _verifierCapabilities[verificationKey(purpose)][relayer];
    }

    function verificationKey(bytes32 purpose) public pure returns (bytes32) {
        return keccak256(abi.encode("VERIFY:", purpose));
    }

    /// @notice A caller-aware issuance-scoping read. A registered service caller
    /// gets only its own service grant; other consumers read the orthogonal VERIFY-key capability.
    function isWhitelistedFor(bytes32 key, address signer) external view override returns (bool) {
        Service storage callerService = _services[msg.sender];
        if (callerService.providerId != bytes20(0)) {
            return key == callerService.recordType && canIssue(msg.sender, signer);
        }
        return _verifierCapabilities[key][signer];
    }

    /// @notice Pre-authorizes a cleared provider/controller to ask the self-service factory for a
    /// clone of one record type. This does not grant issuance and does not attach the future clone.
    function setServiceCreationApproval(bytes20 providerId, bytes32 recordType, bool allowed)
        external
        onlyOwner
    {
        _requireProvider(providerId);
        if (recordType == bytes32(0)) revert InvalidServiceMetadata();
        if (_serviceCreationApprovals[providerId][recordType] == allowed) revert NoChange();
        _serviceCreationApprovals[providerId][recordType] = allowed;
        emit ServiceCreationApprovalSet(providerId, recordType, allowed);
    }

    function canCreateService(bytes20 providerId, bytes32 recordType, address caller)
        public
        view
        override
        returns (bool)
    {
        bytes32 generationId = generationOfFactory[msg.sender];
        return generationId != bytes32(0) && _factoryGenerations[generationId].active
            && _serviceCreationApprovals[providerId][recordType]
            && canWriteProvider(providerId, caller, PROVIDER_PERMISSION_CREATE_SERVICE);
    }

    // ---------------------------------------------------------------------------------------------
    // Typed resolver approval and selection
    // ---------------------------------------------------------------------------------------------

    function setResolverApproved(ResolverKind kind, address resolver, bool approved) external onlyOwner {
        if (resolver == address(0)) revert ZeroAddress();
        if (approved && resolver.code.length == 0) revert ResolverNotApproved();
        if (_approvedResolvers[kind][resolver] == approved) revert NoChange();
        _approvedResolvers[kind][resolver] = approved;
        if (!_resolverListed[kind][resolver]) {
            _resolverListed[kind][resolver] = true;
            _resolverAddresses[kind].push(resolver);
        }
        emit ResolverApprovalSet(kind, resolver, approved);
    }

    function isResolverApproved(ResolverKind kind, address resolver) public view returns (bool) {
        return _approvedResolvers[kind][resolver];
    }

    function setDirectoryResolver(bytes20 providerId, address resolver) external {
        Provider storage p = _requireProvider(providerId);
        if (!canWriteProvider(providerId, msg.sender, PROVIDER_PERMISSION_DIRECTORY_RESOLVER)) {
            revert Unauthorized();
        }
        if (resolver != address(0) && !_approvedResolvers[ResolverKind.DIRECTORY][resolver]) {
            revert ResolverNotApproved();
        }
        address oldResolver = p.directoryResolver;
        if (oldResolver == resolver) revert NoChange();
        p.directoryResolver = resolver;
        emit DirectoryResolverSet(providerId, oldResolver, resolver, msg.sender);
    }

    /// @notice Selects one service's typed DOMAIN resolver. The stored selector is a HISTORICAL record
    /// of the last selection this core accepted: it is never cleared here, neither by resolver
    /// deapproval nor by the generation deprecation that freezes this write permanently (see
    /// `deprecateFactoryGeneration`).
    ///
    /// So a consumer computing a service's EFFECTIVE domain resolver must require BOTH that this stored
    /// selector still names the resolver AND that `isResolverApproved(ResolverKind.DOMAIN, resolver)`
    /// still holds. Reading the selector alone would defeat the registry authority's fleet-wide
    /// `setResolverApproved(DOMAIN, resolver, false)` lever, which is the only way to disable a resolver
    /// for the services this predicate has already frozen.
    function setDomainResolver(address serviceAddress, address resolver) external {
        Service storage s = _requireService(serviceAddress);
        if (!canWriteService(serviceAddress, msg.sender, SERVICE_PERMISSION_DOMAIN_RESOLVER)) {
            revert Unauthorized();
        }
        if (resolver != address(0) && !_approvedResolvers[ResolverKind.DOMAIN][resolver]) {
            revert ResolverNotApproved();
        }
        address oldResolver = s.domainResolver;
        if (oldResolver == resolver) revert NoChange();
        s.domainResolver = resolver;
        emit DomainResolverSet(serviceAddress, oldResolver, resolver, msg.sender);
    }

    // ---------------------------------------------------------------------------------------------
    // Reads and bounded enumeration
    // ---------------------------------------------------------------------------------------------

    function provider(bytes20 providerId) external view returns (Provider memory) {
        return _providers[providerId];
    }

    function publicIdentityAnchor(bytes20 providerId) external view returns (PublicIdentityAnchor memory) {
        return _publicIdentityAnchors[providerId];
    }

    function factoryGeneration(bytes32 generationId) external view returns (FactoryGeneration memory) {
        return _factoryGenerations[generationId];
    }

    function service(address serviceAddress) external view returns (Service memory) {
        return _services[serviceAddress];
    }

    function currentService(bytes20 providerId, bytes32 recordType) external view returns (address) {
        return _currentService[providerId][recordType];
    }

    /// @notice How many addresses currently hold `RIGHT_ISSUE` anywhere in this registry. It takes no
    /// service, because after the re-keying there is no per-service issuer set to count: any holder can
    /// anchor on any service in effective standing. Kept as the honest replacement for the removed
    /// per-service `activeIssuerCount`, which would now always have answered this same number.
    function issueRightHolders() external view returns (uint256) {
        return _issueRightHolders;
    }

    function providerDelegate(bytes20 providerId, address delegate)
        external
        view
        returns (Delegation memory)
    {
        return _providerDelegates[providerId][delegate];
    }

    function serviceDelegate(address serviceAddress, address delegate)
        external
        view
        returns (Delegation memory)
    {
        return _serviceDelegates[serviceAddress][delegate];
    }

    /// @notice The per-term breakdown behind the issuance ladder, for oversight. `ownerConfirmed` and
    /// `hasActiveIssuer` are COMPOSED from the same helpers `canIssue` folds rather than re-derived
    /// here, so `hasActiveIssuer` is exactly `canIssue` with the per-signer grant replaced by the
    /// service's active-issuer count. Never re-inline those terms to save the duplicated `isClone`
    /// staticcall: this is a `view`, and a second copy of the predicate is what drifts.
    function effectiveService(address serviceAddress)
        external
        view
        returns (
            Standing providerStanding,
            Standing serviceStanding,
            bool factoryActive,
            bool ownerConfirmed,
            bool hasActiveIssuer
        )
    {
        Service storage s = _services[serviceAddress];
        providerStanding = _providers[s.providerId].standing;
        serviceStanding = s.standing;
        FactoryGeneration storage generation = _factoryGenerations[s.factoryGeneration];
        factoryActive = generation.active && _factoryRecognizes(generation.factory, serviceAddress);
        ownerConfirmed = _isOwnerConfirmed(serviceAddress, s);
        hasActiveIssuer =
            _issueRightHolders != 0 && _serviceIssuanceEligible(serviceAddress, s);
    }

    function providerCount() external view returns (uint256) {
        return _providerIds.length;
    }

    function providerPage(uint256 cursor, uint256 limit)
        external
        view
        returns (bytes20[] memory values, uint256 nextCursor)
    {
        _checkPage(cursor, limit, _providerIds.length);
        uint256 end = _pageEnd(cursor, limit, _providerIds.length);
        values = new bytes20[](end - cursor);
        for (uint256 i = cursor; i < end; i++) {
            values[i - cursor] = _providerIds[i];
        }
        return (values, end);
    }

    function factoryGenerationCount() external view returns (uint256) {
        return _factoryGenerationIds.length;
    }

    function factoryGenerationPage(uint256 cursor, uint256 limit)
        external
        view
        returns (bytes32[] memory values, uint256 nextCursor)
    {
        _checkPage(cursor, limit, _factoryGenerationIds.length);
        uint256 end = _pageEnd(cursor, limit, _factoryGenerationIds.length);
        values = new bytes32[](end - cursor);
        for (uint256 i = cursor; i < end; i++) {
            values[i - cursor] = _factoryGenerationIds[i];
        }
        return (values, end);
    }

    function serviceCount() external view returns (uint256) {
        return _serviceAddresses.length;
    }

    function servicePage(uint256 cursor, uint256 limit)
        external
        view
        returns (address[] memory values, uint256 nextCursor)
    {
        return _addressPage(_serviceAddresses, cursor, limit);
    }

    function providerServiceCount(bytes20 providerId) external view returns (uint256) {
        return _providerServices[providerId].length;
    }

    function providerServicePage(bytes20 providerId, uint256 cursor, uint256 limit)
        external
        view
        returns (address[] memory values, uint256 nextCursor)
    {
        return _addressPage(_providerServices[providerId], cursor, limit);
    }

    function resolverCount(ResolverKind kind) external view returns (uint256) {
        return _resolverAddresses[kind].length;
    }

    function resolverPage(ResolverKind kind, uint256 cursor, uint256 limit)
        external
        view
        returns (address[] memory values, uint256 nextCursor)
    {
        return _addressPage(_resolverAddresses[kind], cursor, limit);
    }

    function _addressPage(address[] storage source, uint256 cursor, uint256 limit)
        internal
        view
        returns (address[] memory values, uint256 nextCursor)
    {
        _checkPage(cursor, limit, source.length);
        uint256 end = _pageEnd(cursor, limit, source.length);
        values = new address[](end - cursor);
        for (uint256 i = cursor; i < end; i++) {
            values[i - cursor] = source[i];
        }
        return (values, end);
    }

    // ---------------------------------------------------------------------------------------------
    // Internal authoritative reads
    // ---------------------------------------------------------------------------------------------

    function _requireProvider(bytes20 providerId) internal view returns (Provider storage p) {
        p = _providers[providerId];
        if (p.controller == address(0)) revert UnknownProvider();
    }

    function _requireService(address serviceAddress) internal view returns (Service storage s) {
        s = _services[serviceAddress];
        if (s.providerId == bytes20(0)) revert UnknownService();
    }

    function _requireFactoryGeneration(bytes32 generationId)
        internal
        view
        returns (FactoryGeneration storage generation)
    {
        generation = _factoryGenerations[generationId];
        if (generation.factory == address(0)) revert UnknownFactoryGeneration();
    }

    function _addProviderService(bytes20 providerId, address serviceAddress) internal {
        address[] storage list = _providerServices[providerId];
        list.push(serviceAddress);
        _providerServiceIndex[providerId][serviceAddress] = list.length;
    }

    /// @dev O(1) removal keyed by a 1-based index, so correcting a provider that already enumerates
    /// many services can never be priced out of the only path that fixes a misbinding.
    function _removeProviderService(bytes20 providerId, address serviceAddress) internal {
        address[] storage list = _providerServices[providerId];
        uint256 position = _providerServiceIndex[providerId][serviceAddress];
        // Unreachable while every attachment goes through `_addProviderService`, and it fails closed
        // rather than leaving the service enumerated under both providers at once.
        if (position == 0) revert UnknownService();
        uint256 lastPosition = list.length;
        if (position != lastPosition) {
            address moved = list[lastPosition - 1];
            list[position - 1] = moved;
            _providerServiceIndex[providerId][moved] = position;
        }
        list.pop();
        delete _providerServiceIndex[providerId][serviceAddress];
    }

    function _requireSettableStanding(Standing standing) internal pure {
        if (standing != Standing.ACTIVE && standing != Standing.SUSPENDED && standing != Standing.RETIRED) {
            revert InvalidStanding();
        }
    }

    function _serviceStandingIsEffective(address serviceAddress, Service storage s)
        internal
        view
        returns (bool)
    {
        if (
            s.providerId == bytes20(0) || s.standing != Standing.ACTIVE
                || _providers[s.providerId].standing != Standing.ACTIVE
        ) return false;
        FactoryGeneration storage generation = _factoryGenerations[s.factoryGeneration];
        return generation.active && _factoryRecognizes(generation.factory, serviceAddress);
    }

    function _factoryRecognizes(address factory, address candidate) internal view returns (bool) {
        (bool ok, bytes memory data) =
            factory.staticcall(abi.encodeCall(IProviderRegistryFactory.isClone, (candidate)));
        if (!ok || data.length < 32) return false;
        uint256 recognized;
        assembly ("memory-safe") {
            recognized := mload(add(data, 0x20))
        }
        return recognized == 1;
    }

    function _readServiceMetadata(address serviceAddress)
        internal
        view
        returns (bool ok, bytes32 recordType, address liveOwner)
    {
        (bool recordTypeOk, bytes memory recordTypeData) =
            serviceAddress.staticcall(abi.encodeCall(IProviderRegistryService.recordType, ()));
        (bool ownerOk, address owner_) = _readServiceOwner(serviceAddress);
        if (!recordTypeOk || recordTypeData.length < 32 || !ownerOk) return (false, bytes32(0), address(0));
        return (true, abi.decode(recordTypeData, (bytes32)), owner_);
    }

    function _readServiceOwner(address serviceAddress) internal view returns (bool ok, address liveOwner) {
        bytes memory data;
        (ok, data) = serviceAddress.staticcall(abi.encodeCall(IProviderRegistryService.owner, ()));
        if (!ok || data.length < 32) return (false, address(0));
        uint256 ownerWord;
        assembly ("memory-safe") {
            ownerWord := mload(add(data, 0x20))
        }
        if (ownerWord > type(uint160).max) return (false, address(0));
        return (true, address(uint160(ownerWord)));
    }

    function _checkPage(uint256 cursor, uint256 limit, uint256 length) internal pure {
        if (limit == 0 || limit > MAX_PAGE_SIZE || cursor > length) revert BadPage();
    }

    function _pageEnd(uint256 cursor, uint256 limit, uint256 length) internal pure returns (uint256) {
        return limit > length - cursor ? length : cursor + limit;
    }
}
