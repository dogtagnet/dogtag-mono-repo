// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {ProviderRegistry} from "./ProviderRegistry.sol";

/// @title ProviderDirectory — the typed DIRECTORY resolver: pins, contacts and profile anchors.
/// @notice Selected per provider through `ProviderRegistry.setDirectoryResolver` and approved
/// fleet-wide through `ProviderRegistry.setResolverApproved(ResolverKind.DIRECTORY, ...)`.
///
/// ## Provider enumeration is SEPARATE from the pin scan, and that separation is the point
///
/// A provider may publish contacts and no location at all. If the listing were derived from the pin
/// set — the natural implementation, and the one both source reports reached for — a contact-only
/// provider would be invisible to every consumer, which quietly downgrades "the provider is listed"
/// into "we will not put a fake pin on the map". Those are different promises.
///
/// So there are two independent, independently paged sequences:
///
/// - `_listings` — every provider that has ever published anything here. Append-only, so a cursor
///   into it stays valid across writes, and a provider is never removed by withdrawing content.
/// - `_pinScan` — one word per published pin, and the only thing distance is ever computed from.
///
/// A contact-only provider is in the first and absent from the second. `pinTotal()` does not move
/// when it publishes, and it is still enumerated with `pinCount == 0`.
///
/// ## A pin is one storage slot, and a scan is one SLOAD per record
///
/// The packed pin word is exactly 32 bytes with no spare bits, so a pin costs one slot to store and
/// one cold `SLOAD` to scan — measured at ~2,690 gas per record all-in, against the 2,100 the cold
/// `SLOAD` alone costs. `pinPage` returns `bytes32[]`, which for a one-word record is byte-identical
/// to hand-packed `bytes` — the 3.3x ABI-padding penalty a `(address, int32, int32)` struct array
/// would pay is avoided by construction rather than by hand-packing.
///
/// Widening `providerId` past 20 bytes, or adding a field that does not fit the spare bits, spills
/// the record into a second slot and halves the records a single `eth_call` can return. Neither is a
/// tuning knob; both are the reason the layout is packed to the byte.
///
/// ## What a read here does and does not establish
///
/// `pinPage` answers *where has this provider published a location*. It deliberately does NOT answer
/// *should this provider be shown*. Provider standing is the core's fact, so folding it in would
/// either mean a staticcall per pin (which destroys the scan economics above) or a mirrored copy of
/// standing in this contract (which is a second source that drifts). A consumer MUST join standing —
/// `listingPage` carries it, read live from the core and never stored, so one page-aligned join
/// covers a whole page.
///
/// Nothing here establishes that a provider occupies a coordinate. No signature, chain or circuit
/// does. `AddressProvenance` records who asserted the address and on what basis so a consumer can
/// state the observation; a consumer MUST NOT render a location as verified.
///
/// A registrar's confirmation is about a STREET ADDRESS, and that text is in the profile blob rather
/// than on chain, so binding the coordinate alone would let a provider rewrite the text underneath a
/// standing confirmation. Confirmations are therefore stamped with the anchor revision they were made
/// against and read back through `pinAddressProvenance`, which reports whether the confirmation still
/// covers the text the provider currently publishes.
///
/// ## A provider that loses ACTIVE standing has its content FROZEN, not deleted
///
/// Every provider write goes through the core's `canWriteProvider`, which requires ACTIVE standing,
/// so a suspended or retired provider can no longer edit or withdraw what it published — and there is
/// deliberately no registrar override that could do it for them. That mirrors the core's own refusal
/// to let a registrar rewrite a frozen service's published claims: withdrawing what a stale record
/// still means to a consumer is the resolver allowlist's job and the consumer's standing join, not a
/// power to edit history. The cost is that such pins keep their slot in the scan; the alternative is
/// an authority that can silently rewrite a provider's own signed claims, which is worse.
///
/// The one registrar write here, `setPinAddressProvenance`, does NOT require ACTIVE standing,
/// specifically so a confirmation can still be RETRACTED after a provider is suspended.
///
/// ## Paging
///
/// `_pinScan` is compacted by swap-and-pop, so removing a pin reorders the tail. Page every pin at
/// ONE pinned block; `atBlock` is returned so a consumer can prove it did. `_listings` is
/// append-only and needs no such pinning.
contract ProviderDirectory {
    /// @notice Scaled by 1e6, giving 11.13 cm resolution and 91.6% headroom against `int32`
    /// (180e6 against a 2,147,483,647 max). 1e7 would give 1.11 cm and only 16.2% headroom; a
    /// clinic pin does not need centimetres, and the headroom is worth more than the digits.
    int32 public constant COORDINATE_SCALE = 1_000_000;
    int32 public constant LATITUDE_LIMIT = 90 * COORDINATE_SCALE;
    int32 public constant LONGITUDE_LIMIT = 180 * COORDINATE_SCALE;

    /// @notice One core `provider()` staticcall per record, so this mirrors the core's own page cap.
    /// MEASURED cold: ~21,700 gas per record, ~2.17M for a full page — about eight times a pin record,
    /// which is why this cap is the smaller of the two rather than by oversight.
    uint256 public constant MAX_LISTING_PAGE_SIZE = 100;
    /// @notice Bare storage reads. MEASURED cold: ~2,690 gas per record, ~2.69M for a full page, well
    /// inside a node's ~50M `eth_call` cap. Both figures come from
    /// `ProviderDirectoryPageCostTest`, which seeds in `setUp` so the read pays COLD storage; the same
    /// loop measured in one transaction reports ~680k and is measuring warmth, not the real read.
    uint256 public constant MAX_PIN_PAGE_SIZE = 1000;

    uint256 public constant MAX_CONTENTHASH_LENGTH = 256;

    /// @dev A provider is issued location numbers from a monotone counter that is never reset and
    /// never reuses a number, so the width bounds every location a provider will ever publish,
    /// including removed ones. This value is the first number NOT issued, so a provider gets 65,535
    /// of them (0 through 65,534) and exhaustion reverts loudly instead of wrapping.
    uint16 public constant MAX_LOCATION_NUMBER = type(uint16).max;

    uint8 internal constant PIN_FLAG_ACTIVE = 1 << 0;
    uint8 internal constant PIN_PROVENANCE_SHIFT = 1;
    uint8 internal constant PIN_PROVENANCE_MASK = 0x06;
    uint8 internal constant PIN_FLAG_RESERVED_MASK = 0xF8;

    uint8 internal constant LISTING_FLAG_PROFILE_ANCHOR = 1 << 0;
    uint8 internal constant LISTING_FLAG_RESOLVER_SELECTED = 1 << 1;

    /// @dev Field-width masks. Spelled `type(uintN).max` rather than as hex literals: a 40-hex-digit
    /// literal is parsed by solc as an ADDRESS literal and fails the build on its checksum.
    uint256 internal constant MASK_8 = type(uint8).max;
    uint256 internal constant MASK_16 = type(uint16).max;
    uint256 internal constant MASK_32 = type(uint32).max;
    uint256 internal constant MASK_64 = type(uint64).max;
    uint256 internal constant MASK_160 = type(uint160).max;

    /// @notice Words per packed listing record returned by `listingPage`.
    uint256 public constant LISTING_RECORD_WORDS = 2;

    /// @notice How the address on a pin came to be believed, never a verified/unverified binary.
    ///
    /// A provider may only ever assert `SELF_DECLARED`. Raising this is the registrar's assertion
    /// about a provider, so a provider being able to set it would be a provider vouching for itself.
    enum AddressProvenance {
        /// @notice The provider published it. The default, and the only value a provider can write.
        SELF_DECLARED,
        /// @notice An operator checked it against the regulator's published record at onboarding.
        MATCHED_LICENSING_REGISTER,
        /// @notice A code mailed to the address was returned. The only tier that binds key to place.
        POSTAL_CONFIRMED
    }

    /// @notice The unpacked form of one 32-byte pin word. Layout, big-endian:
    ///
    /// ```
    /// [ 0..20)  providerId  bytes20
    /// [20..24)  lat         int32   degrees x 1e6
    /// [24..28)  lng         int32   degrees x 1e6
    /// [28..30)  locationNo  uint16
    /// [30]      kind        uint8   opaque; 0 means NOT STATED and is never inferred
    /// [31]      flags       uint8   bit0 active, bits1-2 provenance, bits3-7 reserved zero
    /// ```
    struct Pin {
        bytes20 providerId;
        int32 lat;
        int32 lng;
        uint16 locationNo;
        uint8 kind;
        uint8 flags;
    }

    /// @notice The content anchor behind a listing: contacts, address text, hours, services and logo
    /// live in the addressed blob, not on chain. Only the integrity anchor is published here.
    ///
    /// This is NOT `ProviderRegistry.PublicIdentityAnchor`. That one is a registrar-written identity
    /// statement; this one is provider-written content carrying no registrar attestation whatsoever.
    /// `name` is deliberately absent — `DogTagIssuer.name()` is already the authoritative name, and a
    /// second copy here would be free to drift from the trusted one.
    struct ProfileAnchor {
        bytes32 digest;
        uint32 schema;
        uint16 codec;
        uint8 hashAlgorithm;
        uint64 revision;
        uint64 updatedAtBlock;
        address setBy;
        bytes contenthash;
    }

    ProviderRegistry public immutable core;
    /// @dev Snapshotted at deploy from the core's own constant rather than restated as a literal.
    uint32 public immutable providerRecordPermission;

    bytes20[] private _listings;
    mapping(bytes20 => bool) private _listed;
    mapping(bytes20 => ProfileAnchor) private _profileAnchors;

    /// @dev One word per existing pin. This is the ONLY place a pin word is stored: a second copy
    /// for the per-key lookup would be a drift source, and would double the SLOADs a scan pays.
    bytes32[] private _pinScan;
    /// @dev 1-based index into `_pinScan`; 0 means this provider has no such location.
    mapping(bytes20 => mapping(uint16 => uint256)) private _pinIndex;
    mapping(bytes20 => uint16) private _nextLocationNumber;
    mapping(bytes20 => uint16) private _pinCount;

    /// @dev The profile-anchor revision a registrar's confirmation was made against. Meaningful only
    /// when the pin's provenance is not `SELF_DECLARED` — the provenance value is what records that a
    /// confirmation exists, so this field does not have to encode its own presence, which matters
    /// because revision 0 is the legitimate "no anchor has ever been published" state.
    ///
    /// It is stored here rather than in the pin word because that word is full to the bit and is the
    /// distance primitive; provenance detail is a per-pin read, never a scan field.
    mapping(bytes20 => mapping(uint16 => uint64)) private _provenanceAnchorRevision;

    event ProviderListed(bytes20 indexed providerId, uint256 index);
    event PinPublished(
        bytes20 indexed providerId,
        uint16 indexed locationNo,
        int32 lat,
        int32 lng,
        uint8 kind,
        bool active,
        address setBy,
        uint64 atBlock
    );
    event PinUpdated(
        bytes20 indexed providerId,
        uint16 indexed locationNo,
        int32 lat,
        int32 lng,
        uint8 kind,
        bool active,
        AddressProvenance provenance,
        bool provenanceReset,
        address setBy,
        uint64 atBlock
    );
    event PinRemoved(bytes20 indexed providerId, uint16 indexed locationNo, address setBy, uint64 atBlock);
    event PinAddressProvenanceSet(
        bytes20 indexed providerId,
        uint16 indexed locationNo,
        AddressProvenance oldProvenance,
        AddressProvenance newProvenance,
        bytes32 anchorDigest,
        uint64 anchorRevision,
        address setBy,
        uint64 atBlock
    );
    event ProfileAnchorSet(
        bytes20 indexed providerId,
        bytes32 indexed oldDigest,
        bytes32 indexed newDigest,
        uint32 schema,
        uint16 codec,
        uint8 hashAlgorithm,
        bytes contenthash,
        uint64 revision,
        address setBy,
        uint64 atBlock
    );
    event ProfileAnchorCleared(
        bytes20 indexed providerId, bytes32 indexed oldDigest, uint64 revision, address setBy, uint64 atBlock
    );

    error ZeroAddress();
    error UnknownProvider();
    error UnknownLocation();
    error ResolverNotApproved();
    error ResolverNotSelected();
    error Unauthorized();
    error LatitudeOutOfRange(int32 lat);
    error LongitudeOutOfRange(int32 lng);
    error ReservedFlagBitsSet(uint8 flags);
    error UnknownProvenance(uint8 provenance);
    error UnknownStanding(uint8 standing);
    error LocationNumbersExhausted();
    error UnexpectedCoordinates(int32 lat, int32 lng);
    error UnexpectedProfileAnchor(bytes32 digest);
    error BadProfileAnchor();
    error ContenthashTooLong();
    error NoProfileAnchor();
    error NoChange();
    error BadPage();

    constructor(ProviderRegistry core_) {
        if (address(core_) == address(0)) revert ZeroAddress();
        core = core_;
        providerRecordPermission = core_.PROVIDER_PERMISSION_RECORD();
    }

    // ---------------------------------------------------------------------------------------------
    // Codec — public so the wire format is self-documenting on chain and testable off it
    // ---------------------------------------------------------------------------------------------

    /// @notice Validates and packs one pin into its single storage/wire word.
    /// @dev Validation lives in the codec on purpose: every write path packs through here, so a word
    /// carrying an out-of-range coordinate, an unknown provenance or a set reserved bit is not
    /// representable by any sanctioned path.
    function packPin(Pin memory value) public pure returns (bytes32 word) {
        requireCoordinates(value.lat, value.lng);
        if (value.flags & PIN_FLAG_RESERVED_MASK != 0) revert ReservedFlagBitsSet(value.flags);
        uint8 provenance = (value.flags & PIN_PROVENANCE_MASK) >> PIN_PROVENANCE_SHIFT;
        if (provenance > uint8(AddressProvenance.POSTAL_CONFIRMED)) revert UnknownProvenance(provenance);

        // `uint32(int32)` is a same-width reinterpretation, not a truncation: it is exactly the
        // two's-complement bit pattern `unpackPin` reads back with the inverse cast.
        word = bytes32(
            (uint256(uint160(value.providerId)) << 96) | (uint256(uint32(value.lat)) << 64)
                | (uint256(uint32(value.lng)) << 32) | (uint256(value.locationNo) << 16)
                | (uint256(value.kind) << 8) | uint256(value.flags)
        );
    }

    /// @notice Splits one pin word back into its fields. Total field width is exactly 256 bits, so
    /// every bit of the word is accounted for and a word carries its own `(providerId, locationNo)`
    /// identity — which is what lets the scan's swap-and-pop repair a moved record's index from the
    /// word alone rather than from a second bookkeeping structure.
    /// @dev Each field is masked to its own width before the narrowing cast, so the cast cannot
    /// discard a bit that belongs to a neighbouring field. The masks are the field widths, stated at
    /// the point of extraction, and they are what justify the lint exemption below: narrowing IS the
    /// operation here, and the mask bounds what it can narrow away.
    // forge-lint: disable-next-item(unsafe-typecast)
    function unpackPin(bytes32 word) public pure returns (Pin memory value) {
        uint256 raw = uint256(word);
        value.providerId = bytes20(uint160((raw >> 96) & MASK_160));
        value.lat = int32(uint32((raw >> 64) & MASK_32));
        value.lng = int32(uint32((raw >> 32) & MASK_32));
        value.locationNo = uint16((raw >> 16) & MASK_16);
        value.kind = uint8((raw >> 8) & MASK_8);
        value.flags = uint8(raw & MASK_8);
    }

    /// @dev Masked to the flags byte before narrowing, as in `unpackPin`.
    // forge-lint: disable-next-item(unsafe-typecast)
    function pinIsActive(bytes32 word) public pure returns (bool) {
        return uint8(uint256(word) & MASK_8) & PIN_FLAG_ACTIVE != 0;
    }

    /// @dev Masked to the flags byte before narrowing, as in `unpackPin`.
    // forge-lint: disable-next-item(unsafe-typecast)
    function pinProvenance(bytes32 word) public pure returns (AddressProvenance) {
        return _provenanceOf(uint8(uint256(word) & MASK_8));
    }

    /// @notice `(0, 0)` is a REAL coordinate off the coast of Ghana and is accepted as one. Absence of
    /// a location is represented by having no pin at all, never by a zero pair — which is precisely
    /// the confusion that made a blank admin location render as a pin in the Gulf of Guinea.
    function requireCoordinates(int32 lat, int32 lng) public pure {
        if (lat < -LATITUDE_LIMIT || lat > LATITUDE_LIMIT) revert LatitudeOutOfRange(lat);
        if (lng < -LONGITUDE_LIMIT || lng > LONGITUDE_LIMIT) revert LongitudeOutOfRange(lng);
    }

    // ---------------------------------------------------------------------------------------------
    // Provider writes — pins
    // ---------------------------------------------------------------------------------------------

    /// @notice Publishes a new location and issues it the provider's next location number.
    /// Provenance is forced to `SELF_DECLARED`: a provider cannot vouch for its own address.
    function publishPin(bytes20 providerId, int32 lat, int32 lng, uint8 kind, bool active)
        external
        returns (uint16 locationNo)
    {
        _requireProviderWrite(providerId);

        locationNo = _issueLocationNumber(_nextLocationNumber[providerId]);
        _nextLocationNumber[providerId] = locationNo + 1;

        bytes32 word = packPin(
            Pin({
                providerId: providerId,
                lat: lat,
                lng: lng,
                locationNo: locationNo,
                kind: kind,
                flags: active ? PIN_FLAG_ACTIVE : 0
            })
        );

        _pinScan.push(word);
        _pinIndex[providerId][locationNo] = _pinScan.length;
        _pinCount[providerId] += 1;
        _list(providerId);

        emit PinPublished(providerId, locationNo, lat, lng, kind, active, msg.sender, uint64(block.number));
    }

    /// @notice Rewrites an existing location, keeping its number.
    /// @dev Moving the coordinates RESETS provenance to `SELF_DECLARED`. A registrar checked one
    /// address; carrying its confirmation over to a different one would attribute a check that was
    /// never made. Changing only `kind` or `active` preserves it, because neither restates the
    /// address the registrar actually checked.
    function updatePin(bytes20 providerId, uint16 locationNo, int32 lat, int32 lng, uint8 kind, bool active)
        external
    {
        _requireProviderWrite(providerId);
        (uint256 index, Pin memory existing) = _requirePin(providerId, locationNo);

        bool moved = existing.lat != lat || existing.lng != lng;
        AddressProvenance old = _provenanceOf(existing.flags);
        AddressProvenance provenance = moved ? AddressProvenance.SELF_DECLARED : old;

        uint8 flags = uint8(uint8(provenance) << PIN_PROVENANCE_SHIFT);
        if (active) flags |= PIN_FLAG_ACTIVE;

        bytes32 word = packPin(
            Pin({
                providerId: providerId, lat: lat, lng: lng, locationNo: locationNo, kind: kind, flags: flags
            })
        );
        if (word == _pinScan[index]) revert NoChange();
        _pinScan[index] = word;
        // A move drops the confirmation, so its stamped revision must go with it rather than linger
        // beside a SELF_DECLARED provenance where a later read could pair the two.
        if (moved) delete _provenanceAnchorRevision[providerId][locationNo];

        emit PinUpdated(
            providerId,
            locationNo,
            lat,
            lng,
            kind,
            active,
            provenance,
            moved && old != AddressProvenance.SELF_DECLARED,
            msg.sender,
            uint64(block.number)
        );
    }

    /// @notice Withdraws a location. Its number is never reissued, so `(providerId, locationNo)` can
    /// never come to mean a different place. The provider stays enumerated — withdrawing content is
    /// not leaving the directory.
    function removePin(bytes20 providerId, uint16 locationNo) external {
        _requireProviderWrite(providerId);
        _requirePin(providerId, locationNo);

        _removeFromScan(providerId, locationNo);
        _pinCount[providerId] -= 1;
        delete _provenanceAnchorRevision[providerId][locationNo];

        emit PinRemoved(providerId, locationNo, msg.sender, uint64(block.number));
    }

    // ---------------------------------------------------------------------------------------------
    // Registrar writes — address provenance
    // ---------------------------------------------------------------------------------------------

    /// @notice Records the registrar's basis for believing a pin's address.
    /// @dev The three expected-value arguments are transaction guards, not selectors: they bind the
    /// assertion to exactly what the registrar checked, so a provider editing in the same block cannot
    /// capture a confirmation meant for something else. `expectedLat`/`expectedLng` cover the
    /// coordinate; `expectedAnchorDigest` covers the blob, because the street address these provenance
    /// values are ABOUT is text inside that blob and not on chain at all.
    ///
    /// The confirmation is stamped with the anchor revision it was made against, which is what makes a
    /// later rewrite of that text detectable — see `pinAddressProvenance`.
    function setPinAddressProvenance(
        bytes20 providerId,
        uint16 locationNo,
        int32 expectedLat,
        int32 expectedLng,
        bytes32 expectedAnchorDigest,
        AddressProvenance provenance
    ) external {
        _requireLiveResolver(providerId);
        if (msg.sender != core.owner()) revert Unauthorized();
        if (uint8(provenance) > uint8(AddressProvenance.POSTAL_CONFIRMED)) {
            revert UnknownProvenance(uint8(provenance));
        }

        (uint256 index, Pin memory existing) = _requirePin(providerId, locationNo);
        if (existing.lat != expectedLat || existing.lng != expectedLng) {
            revert UnexpectedCoordinates(existing.lat, existing.lng);
        }
        ProfileAnchor storage anchor = _profileAnchors[providerId];
        if (anchor.digest != expectedAnchorDigest) revert UnexpectedProfileAnchor(anchor.digest);

        AddressProvenance old = _provenanceOf(existing.flags);
        if (old == provenance) revert NoChange();

        existing.flags = (existing.flags & PIN_FLAG_ACTIVE) | uint8(uint8(provenance) << PIN_PROVENANCE_SHIFT);
        _pinScan[index] = packPin(existing);
        if (provenance == AddressProvenance.SELF_DECLARED) {
            delete _provenanceAnchorRevision[providerId][locationNo];
        } else {
            _provenanceAnchorRevision[providerId][locationNo] = anchor.revision;
        }

        emit PinAddressProvenanceSet(
            providerId,
            locationNo,
            old,
            provenance,
            anchor.digest,
            anchor.revision,
            msg.sender,
            uint64(block.number)
        );
    }

    // ---------------------------------------------------------------------------------------------
    // Provider writes — the profile / contact anchor
    // ---------------------------------------------------------------------------------------------

    /// @notice Publishes the integrity anchor for this provider's contact and profile blob.
    /// @dev Content addressing gives integrity, not privacy. Only a publication-safe blob may be
    /// anchored; nothing private belongs behind a digest anyone can fetch.
    function setProfileAnchor(
        bytes20 providerId,
        bytes32 digest,
        uint32 schema,
        uint16 codec,
        uint8 hashAlgorithm,
        bytes calldata contenthash
    ) external {
        _requireProviderWrite(providerId);
        if (digest == bytes32(0) || schema == 0 || hashAlgorithm == 0) revert BadProfileAnchor();
        if (contenthash.length > MAX_CONTENTHASH_LENGTH) revert ContenthashTooLong();

        ProfileAnchor storage anchor = _profileAnchors[providerId];
        bytes32 oldDigest = anchor.digest;
        uint64 revision = anchor.revision + 1;
        anchor.digest = digest;
        anchor.schema = schema;
        anchor.codec = codec;
        anchor.hashAlgorithm = hashAlgorithm;
        anchor.contenthash = contenthash;
        anchor.revision = revision;
        anchor.updatedAtBlock = uint64(block.number);
        anchor.setBy = msg.sender;
        _list(providerId);

        emit ProfileAnchorSet(
            providerId,
            oldDigest,
            digest,
            schema,
            codec,
            hashAlgorithm,
            contenthash,
            revision,
            msg.sender,
            uint64(block.number)
        );
    }

    /// @notice Withdraws the anchor. The revision still advances, so a consumer can tell "withdrawn"
    /// from "never published" — an absent digest at revision 0 has never been set, and an absent
    /// digest at a higher revision was taken down.
    function clearProfileAnchor(bytes20 providerId) external {
        _requireProviderWrite(providerId);

        ProfileAnchor storage anchor = _profileAnchors[providerId];
        bytes32 oldDigest = anchor.digest;
        if (oldDigest == bytes32(0)) revert NoProfileAnchor();

        uint64 revision = anchor.revision + 1;
        anchor.digest = bytes32(0);
        anchor.schema = 0;
        anchor.codec = 0;
        anchor.hashAlgorithm = 0;
        anchor.contenthash = "";
        anchor.revision = revision;
        anchor.updatedAtBlock = uint64(block.number);
        anchor.setBy = msg.sender;

        emit ProfileAnchorCleared(providerId, oldDigest, revision, msg.sender, uint64(block.number));
    }

    // ---------------------------------------------------------------------------------------------
    // Reads
    // ---------------------------------------------------------------------------------------------

    /// @notice Whether this contract is still both fleet-approved and this provider's selection.
    /// @dev Both halves are required. The core never clears a stored selector when a resolver is
    /// deapproved, so reading the selector alone would defeat the registry authority's fleet-wide
    /// `setResolverApproved(DIRECTORY, this, false)` lever.
    function isLiveFor(bytes20 providerId) public view returns (bool) {
        if (!core.isResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, address(this))) return false;
        return core.provider(providerId).directoryResolver == address(this);
    }

    function resolverApproved() public view returns (bool) {
        return core.isResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, address(this));
    }

    function profileAnchor(bytes20 providerId) external view returns (ProfileAnchor memory) {
        return _profileAnchors[providerId];
    }

    function listingTotal() external view returns (uint256) {
        return _listings.length;
    }

    function isListed(bytes20 providerId) external view returns (bool) {
        return _listed[providerId];
    }

    function pinTotal() external view returns (uint256) {
        return _pinScan.length;
    }

    function pinCount(bytes20 providerId) external view returns (uint16) {
        return _pinCount[providerId];
    }

    function nextLocationNumber(bytes20 providerId) external view returns (uint16) {
        return _nextLocationNumber[providerId];
    }

    function hasPin(bytes20 providerId, uint16 locationNo) public view returns (bool) {
        return _pinIndex[providerId][locationNo] != 0;
    }

    function pinWord(bytes20 providerId, uint16 locationNo) public view returns (bytes32) {
        uint256 oneBased = _pinIndex[providerId][locationNo];
        if (oneBased == 0) revert UnknownLocation();
        return _pinScan[oneBased - 1];
    }

    function pin(bytes20 providerId, uint16 locationNo) external view returns (Pin memory) {
        return unpackPin(pinWord(providerId, locationNo));
    }

    /// @notice A pin's address provenance together with whether it still covers the address text the
    /// provider currently publishes.
    ///
    /// `MATCHED_LICENSING_REGISTER` and `POSTAL_CONFIRMED` are assertions about a STREET ADDRESS, and
    /// that text lives in the provider-rewritable profile blob rather than on chain — so binding the
    /// coordinate is not enough. Each confirmation is stamped with the anchor revision it was made
    /// against, and every `setProfileAnchor` or `clearProfileAnchor` advances that revision, so a
    /// rewrite of the text makes `coversCurrentAddressText` false with no loop over the provider's pins
    /// and no cooperation from the provider.
    ///
    /// **A stale confirmation is NOT silently downgraded to `SELF_DECLARED`.** The registrar really did
    /// confirm something, and erasing that would be a false statement in the other direction; the
    /// honest report is "confirmed, against an earlier revision of the address text". A consumer MUST
    /// require `coversCurrentAddressText` before presenting a confirmation as describing what it now
    /// shows, exactly as a stale-but-valid credential keeps its label and loses its freshness rather
    /// than collapsing to invalid.
    ///
    /// `coversCurrentAddressText` is false for `SELF_DECLARED` because there is no confirmation to
    /// cover anything — branch on `provenance` first, never read the flag alone.
    function pinAddressProvenance(bytes20 providerId, uint16 locationNo)
        external
        view
        returns (
            AddressProvenance provenance,
            uint64 confirmedAtAnchorRevision,
            uint64 currentAnchorRevision,
            bool coversCurrentAddressText
        )
    {
        provenance = pinProvenance(pinWord(providerId, locationNo));
        currentAnchorRevision = _profileAnchors[providerId].revision;
        // The STORED stamp, never a hardcoded zero for the self-declared case. Zeroing it there would
        // read identically whenever the "no stamp without a confirmation" invariant holds and hide the
        // one case where it does not — which makes a leftover stamp unobservable and any test of the
        // clearing paths vacuous.
        confirmedAtAnchorRevision = _provenanceAnchorRevision[providerId][locationNo];
        coversCurrentAddressText = provenance != AddressProvenance.SELF_DECLARED
            && confirmedAtAnchorRevision == currentAnchorRevision;
    }

    /// @notice One 32-byte word per pin, in scan order.
    ///
    /// Returns EVERY published pin, including inactive ones and including pins whose provider is not
    /// in ACTIVE standing. A consumer filters `pinIsActive` itself and joins standing from
    /// `listingPage`; this read makes no claim about whether a pin should be presented.
    ///
    /// Page every pin at one pinned block — `_pinScan` is compacted by swap-and-pop, so a removal
    /// between two unpinned pages can move a record past a cursor that has already gone by.
    function pinPage(uint256 cursor, uint256 limit)
        external
        view
        returns (bytes32[] memory words, uint256 nextCursor, uint256 total, bool approved, uint64 atBlock)
    {
        total = _pinScan.length;
        _checkPage(cursor, limit, total, MAX_PIN_PAGE_SIZE);
        uint256 end = _pageEnd(cursor, limit, total);
        words = new bytes32[](end - cursor);
        for (uint256 i = cursor; i < end; i++) {
            words[i - cursor] = _pinScan[i];
        }
        nextCursor = end;
        approved = resolverApproved();
        atBlock = uint64(block.number);
    }

    /// @notice Two 32-byte words per listing record, in append order. Layout, big-endian:
    ///
    /// ```
    /// word 0: [ 0..20) providerId      bytes20
    ///         [20..28) anchorRevision  uint64
    ///         [28..30) pinCount        uint16
    ///         [30]     standing        uint8   ProviderRegistry.Standing, read LIVE from the core
    ///         [31]     flags           uint8   bit0 anchor present, bit1 this resolver selected
    /// word 1: profileDigest            bytes32 zero when no anchor is currently published
    /// ```
    ///
    /// A record is a LIVE claim only if the `selected` bit AND the page-level `approved` both hold.
    /// The two are returned separately rather than pre-combined because they are separate levers: the
    /// provider owns the selection and the registry authority owns the approval, and a consumer that
    /// saw only their conjunction could not tell a provider moving to another resolver from the
    /// authority pulling this one.
    ///
    /// `standing` is read from the core on every call and never stored here, so it cannot drift from
    /// the authority that owns it. It is carried in this page rather than left to the consumer
    /// because the pin scan deliberately omits it, and the join has to be affordable to be made.
    function listingPage(uint256 cursor, uint256 limit)
        external
        view
        returns (bytes32[] memory words, uint256 nextCursor, uint256 total, bool approved, uint64 atBlock)
    {
        total = _listings.length;
        _checkPage(cursor, limit, total, MAX_LISTING_PAGE_SIZE);
        uint256 end = _pageEnd(cursor, limit, total);

        approved = resolverApproved();
        words = new bytes32[]((end - cursor) * LISTING_RECORD_WORDS);
        for (uint256 i = cursor; i < end; i++) {
            bytes20 providerId = _listings[i];
            ProfileAnchor storage anchor = _profileAnchors[providerId];
            ProviderRegistry.Provider memory p = core.provider(providerId);

            uint8 flags;
            if (anchor.digest != bytes32(0)) flags |= LISTING_FLAG_PROFILE_ANCHOR;
            // Reported as the raw pointer fact and deliberately NOT and-ed with `approved`: they are
            // two separate levers, and folding them here would leave a consumer unable to tell "this
            // provider moved to another resolver" from "the registry authority pulled this one".
            if (p.directoryResolver == address(this)) flags |= LISTING_FLAG_RESOLVER_SELECTED;

            uint256 slot = (i - cursor) * LISTING_RECORD_WORDS;
            words[slot] = bytes32(
                (uint256(uint160(providerId)) << 96) | (uint256(anchor.revision) << 32)
                    | (uint256(_pinCount[providerId]) << 16) | (uint256(uint8(p.standing)) << 8)
                    | uint256(flags)
            );
            words[slot + 1] = anchor.digest;
        }
        nextCursor = end;
        atBlock = uint64(block.number);
    }

    /// @notice Splits one `listingPage` record. Mirrors `unpackPin`'s role for the other sequence.
    /// @dev Field-masked before narrowing, as in `unpackPin`; same reason for the exemption.
    // forge-lint: disable-next-item(unsafe-typecast)
    function unpackListing(bytes32 word0, bytes32 word1)
        public
        pure
        returns (
            bytes20 providerId,
            uint64 anchorRevision,
            uint16 pinCount_,
            ProviderRegistry.Standing standing,
            bool anchorPresent,
            bool resolverSelected,
            bytes32 profileDigest
        )
    {
        uint256 raw = uint256(word0);
        providerId = bytes20(uint160((raw >> 96) & MASK_160));
        anchorRevision = uint64((raw >> 32) & MASK_64);
        pinCount_ = uint16((raw >> 16) & MASK_16);
        uint8 standingByte = uint8((raw >> 8) & MASK_8);
        if (standingByte > uint8(type(ProviderRegistry.Standing).max)) revert UnknownStanding(standingByte);
        standing = ProviderRegistry.Standing(standingByte);
        uint8 flags = uint8(raw & MASK_8);
        anchorPresent = flags & LISTING_FLAG_PROFILE_ANCHOR != 0;
        resolverSelected = flags & LISTING_FLAG_RESOLVER_SELECTED != 0;
        profileDigest = word1;
    }

    // ---------------------------------------------------------------------------------------------
    // Internals
    // ---------------------------------------------------------------------------------------------

    /// @dev Every write requires all three: this resolver is still fleet-approved, it is still this
    /// provider's selection, and the caller may write this provider's records. The first two are the
    /// resolver's own liveness and the third is the core's authority predicate; none substitutes for
    /// another.
    function _requireProviderWrite(bytes20 providerId) internal view {
        _requireLiveResolver(providerId);
        if (!core.canWriteProvider(providerId, msg.sender, providerRecordPermission)) {
            revert Unauthorized();
        }
    }

    function _requireLiveResolver(bytes20 providerId) internal view {
        ProviderRegistry.Provider memory p = core.provider(providerId);
        if (p.controller == address(0)) revert UnknownProvider();
        if (!core.isResolverApproved(ProviderRegistry.ResolverKind.DIRECTORY, address(this))) {
            revert ResolverNotApproved();
        }
        if (p.directoryResolver != address(this)) revert ResolverNotSelected();
    }

    /// @dev Range-checked before the enum cast, so a hand-built word reaching `pinProvenance` from an
    /// untrusted mirror is refused by this contract's own named error rather than an opaque panic.
    function _provenanceOf(uint8 flags) internal pure returns (AddressProvenance) {
        uint8 provenance = (flags & PIN_PROVENANCE_MASK) >> PIN_PROVENANCE_SHIFT;
        if (provenance > uint8(AddressProvenance.POSTAL_CONFIRMED)) revert UnknownProvenance(provenance);
        return AddressProvenance(provenance);
    }

    /// @dev Split out so the exhaustion arm is reachable by a test without exposing the counter,
    /// which is the only other way to reach a state 65,535 publications away.
    function _issueLocationNumber(uint16 current) internal pure returns (uint16) {
        if (current == MAX_LOCATION_NUMBER) revert LocationNumbersExhausted();
        return current;
    }

    function _requirePin(bytes20 providerId, uint16 locationNo)
        internal
        view
        returns (uint256 index, Pin memory existing)
    {
        uint256 oneBased = _pinIndex[providerId][locationNo];
        if (oneBased == 0) revert UnknownLocation();
        index = oneBased - 1;
        existing = unpackPin(_pinScan[index]);
    }

    function _list(bytes20 providerId) internal {
        if (_listed[providerId]) return;
        _listed[providerId] = true;
        _listings.push(providerId);
        emit ProviderListed(providerId, _listings.length - 1);
    }

    /// @dev Swap-and-pop. The moved word carries its own `(providerId, locationNo)`, so its index is
    /// repaired from the word itself.
    function _removeFromScan(bytes20 providerId, uint16 locationNo) internal {
        uint256 index = _pinIndex[providerId][locationNo] - 1;
        uint256 last = _pinScan.length - 1;
        if (index != last) {
            bytes32 moved = _pinScan[last];
            _pinScan[index] = moved;
            Pin memory movedPin = unpackPin(moved);
            _pinIndex[movedPin.providerId][movedPin.locationNo] = index + 1;
        }
        _pinScan.pop();
        delete _pinIndex[providerId][locationNo];
    }

    function _checkPage(uint256 cursor, uint256 limit, uint256 length, uint256 maxPage) internal pure {
        if (limit == 0 || limit > maxPage || cursor > length) revert BadPage();
    }

    function _pageEnd(uint256 cursor, uint256 limit, uint256 length) internal pure returns (uint256) {
        return limit > length - cursor ? length : cursor + limit;
    }
}
