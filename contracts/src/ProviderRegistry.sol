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
    function isWhitelistedFor(bytes32 key, address signer) external view returns (bool);
    function hasRole(bytes32 role, address account) external view returns (bool);
}

/// @title ProviderRegistry — canonical provider identity and service authority core.
/// @notice Holds only stable identity, standing and authority facts. Domain claims and directory
/// content deliberately live in typed resolvers selected through this core.
///
/// `providerId` is an opaque KYC registrar assignment. It MUST NOT be derived from a provider name,
/// domain, controller, signer, service clone, factory salt or factory generation.
///
/// The contract intentionally has one authority key for this generation. `Ownable2Step` makes every
/// key switch request/accept, and `hasRole` projects that one current key into the two legacy role
/// reads required by existing Dogtag consumers. There is no independently grantable role that could
/// leave the old key behind after a rotation.
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
    mapping(bytes20 => mapping(bytes32 => address)) private _currentService;

    mapping(bytes20 => mapping(address => Delegation)) private _providerDelegates;
    mapping(address => mapping(address => Delegation)) private _serviceDelegates;

    mapping(address => mapping(address => bool)) private _issuanceCapabilities;
    mapping(address => uint256) private _activeIssuerCount;
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

    event IssuanceCapabilitySet(address indexed service, address indexed signer, bool allowed);
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
    error NoPendingController();
    error PendingControllerNotAccepted();
    error NotPendingController();
    error UnexpectedControllerTransfer();
    error UnexpectedServiceOwner();
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
        _providerServices[providerId].push(serviceAddress);
        emit ServiceAttached(serviceAddress, providerId, generationId, recordType, liveOwner);
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

    function setIssuanceCapability(address serviceAddress, address signer, bool allowed) external onlyOwner {
        _requireService(serviceAddress);
        if (signer == address(0)) revert ZeroAddress();
        bool oldAllowed = _issuanceCapabilities[serviceAddress][signer];
        if (oldAllowed == allowed) revert NoChange();
        _issuanceCapabilities[serviceAddress][signer] = allowed;
        if (allowed) {
            _activeIssuerCount[serviceAddress]++;
        } else {
            _activeIssuerCount[serviceAddress]--;
        }
        emit IssuanceCapabilitySet(serviceAddress, signer, allowed);
    }

    function canIssue(address serviceAddress, address signer) public view override returns (bool) {
        if (!_canOperateService(serviceAddress, signer)) return false;
        Service storage s = _services[serviceAddress];
        if (_currentService[s.providerId][s.recordType] != serviceAddress) return false;
        return true;
    }

    /// @notice Preserves an originator's ability to revoke roots on a superseded service without
    /// reopening that service for new issuance. S-7 must call this path from `revoke`; the legacy
    /// `isWhitelistedFor` selector cannot distinguish an issue call from a revoke call.
    function canRevoke(address serviceAddress, address signer) public view override returns (bool) {
        return _canOperateService(serviceAddress, signer);
    }

    function _canOperateService(address serviceAddress, address signer) internal view returns (bool) {
        Service storage s = _services[serviceAddress];
        if (!_issuanceCapabilities[serviceAddress][signer] || !_serviceStandingIsEffective(serviceAddress, s))
        {
            return false;
        }
        (bool ownerOk, address liveOwner) = _readServiceOwner(serviceAddress);
        return ownerOk && liveOwner == s.confirmedOwner;
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

    /// @notice Exact legacy shape with caller-aware issuance scoping. A registered service caller
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

    function issuanceCapability(address serviceAddress, address signer) external view returns (bool) {
        return _issuanceCapabilities[serviceAddress][signer];
    }

    function activeIssuerCount(address serviceAddress) external view returns (uint256) {
        return _activeIssuerCount[serviceAddress];
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
        (bool ownerOk, address liveOwner) = _readServiceOwner(serviceAddress);
        ownerConfirmed =
            s.providerId != bytes20(0) && ownerOk && liveOwner != address(0) && liveOwner == s.confirmedOwner;
        hasActiveIssuer = _activeIssuerCount[serviceAddress] != 0 && providerStanding == Standing.ACTIVE
            && serviceStanding == Standing.ACTIVE && factoryActive && ownerConfirmed
            && _currentService[s.providerId][s.recordType] == serviceAddress;
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
        _checkPage(cursor, limit, _serviceAddresses.length);
        uint256 end = _pageEnd(cursor, limit, _serviceAddresses.length);
        values = new address[](end - cursor);
        for (uint256 i = cursor; i < end; i++) {
            values[i - cursor] = _serviceAddresses[i];
        }
        return (values, end);
    }

    function providerServiceCount(bytes20 providerId) external view returns (uint256) {
        return _providerServices[providerId].length;
    }

    function providerServicePage(bytes20 providerId, uint256 cursor, uint256 limit)
        external
        view
        returns (address[] memory values, uint256 nextCursor)
    {
        address[] storage source = _providerServices[providerId];
        _checkPage(cursor, limit, source.length);
        uint256 end = _pageEnd(cursor, limit, source.length);
        values = new address[](end - cursor);
        for (uint256 i = cursor; i < end; i++) {
            values[i - cursor] = source[i];
        }
        return (values, end);
    }

    function resolverCount(ResolverKind kind) external view returns (uint256) {
        return _resolverAddresses[kind].length;
    }

    function resolverPage(ResolverKind kind, uint256 cursor, uint256 limit)
        external
        view
        returns (address[] memory values, uint256 nextCursor)
    {
        address[] storage source = _resolverAddresses[kind];
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
