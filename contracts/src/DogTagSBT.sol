// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {ERC721} from "@openzeppelin/contracts/token/ERC721/ERC721.sol";
import {AccessControlEnumerable} from "@openzeppelin/contracts/access/extensions/AccessControlEnumerable.sol";
import {EIP712} from "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {IERC5192} from "./IERC5192.sol";

/// @title DogTagSBT — the pet identity (ERC-721 + ERC-5192 soulbound, granular lifecycle).
/// @notice Granular least-privilege roles + immutable originator binding + authority override +
/// soft status (NEVER burn for lifecycle) + signature-authorized recovery preserving tokenId
/// (impl §11.7(a), architecture §4.2/§13.6). `dogTagId` is a non-personal random/sequential id —
/// NEVER any hash of the microchip (allocated off-chain; asserted by CI).
contract DogTagSBT is ERC721, AccessControlEnumerable, EIP712, IERC5192 {
    enum Status {
        Active,
        Lost,
        TransferPending,
        Deceased,
        Revoked
    }

    bytes32 public constant ISSUER_ROLE = keccak256("ISSUER");
    bytes32 public constant UPDATER_ROLE = keccak256("UPDATER");
    bytes32 public constant AUTHORITY_ROLE = keccak256("AUTHORITY");
    bytes32 public constant RECOVERY_ROLE = keccak256("RECOVERY");

    bytes32 private constant CLAIM_TYPEHASH =
        keccak256("Claim(uint256 dogTagId,address newOwner,uint256 nonce,uint256 deadline)");
    // The CURRENT holder's explicit consent to a recovery/rebind (audit H1). Distinct typehash so a
    // destination's acceptance signature can never double as a current-owner authorization.
    bytes32 private constant RECOVER_CONSENT_TYPEHASH = keccak256(
        "RecoverConsent(uint256 dogTagId,address currentOwner,address newOwner,uint256 nonce,uint256 deadline)"
    );

    mapping(uint256 => address) public issuerOf; // immutable, set at mint
    mapping(uint256 => bytes32) public profileRoot;
    mapping(uint256 => Status) public status;
    mapping(uint256 => uint256) public recoverNonce;

    bool private _inRecovery;

    event Issued(uint256 indexed dogTagId, address indexed issuer);
    event StatusChanged(uint256 indexed dogTagId, Status from, Status to, address by, string reason);
    event Recovered(uint256 indexed dogTagId, address indexed newOwner);
    event Burned(uint256 indexed dogTagId);

    error Soulbound();
    error NotIssuerOrAuthority();
    error Terminal();

    constructor(address admin) ERC721("DogTag", "DTAG") EIP712("DogTag", "1") {
        _grantRole(DEFAULT_ADMIN_ROLE, admin); // protocol multisig
    }

    modifier issuerOrAuthority(uint256 id) {
        if (msg.sender != issuerOf[id] && !hasRole(AUTHORITY_ROLE, msg.sender)) revert NotIssuerOrAuthority();
        _;
    }

    function mint(address to, uint256 id, bytes32 root) external onlyRole(ISSUER_ROLE) {
        _safeMint(to, id);
        issuerOf[id] = msg.sender;
        status[id] = Status.Active;
        profileRoot[id] = root;
        emit Locked(id);
        emit Issued(id, msg.sender);
    }

    function setProfileRoot(uint256 id, bytes32 r) external issuerOrAuthority(id) {
        require(status[id] == Status.Active, "!active");
        profileRoot[id] = r;
    }

    /// @notice Status transitions; Deceased/Revoked are terminal. The OWNER can NEVER call this.
    function setStatus(uint256 id, Status s, string calldata reason) external issuerOrAuthority(id) {
        Status f = status[id];
        if (f == Status.Deceased || f == Status.Revoked) revert Terminal();
        status[id] = s;
        emit StatusChanged(id, f, s, msg.sender, reason);
    }

    /// @notice Lost-key / sale recovery — PRESERVES tokenId + issuerOf so referencing creds survive.
    /// @dev Requires TWO EIP-712 authorizations, both binding chainId + this contract (domain) and the
    /// per-token nonce + deadline: `currentOwnerSig` is the CURRENT holder consenting to the rebind, and
    /// `ownerSig` is the DESTINATION accepting it. RECOVERY_ROLE can only EXECUTE a recovery the current
    /// holder has explicitly authorized — it can no longer unilaterally confiscate a soulbound token
    /// (audit H1). A genuinely lost key (holder cannot sign) is therefore an admin/AUTHORITY concern,
    /// not a RECOVERY_ROLE one.
    function recover(
        uint256 id,
        address newOwner,
        uint256 nonce,
        uint256 deadline,
        bytes calldata currentOwnerSig,
        bytes calldata ownerSig
    ) external onlyRole(RECOVERY_ROLE) {
        require(block.timestamp <= deadline, "expired");
        require(newOwner != address(0), "zero newOwner");
        require(nonce == recoverNonce[id]++, "bad nonce");
        address currentOwner = ownerOf(id); // reverts if the token does not exist

        bytes32 consentDigest = _hashTypedDataV4(
            keccak256(abi.encode(RECOVER_CONSENT_TYPEHASH, id, currentOwner, newOwner, nonce, deadline))
        );
        require(ECDSA.recover(consentDigest, currentOwnerSig) == currentOwner, "bad current-owner sig");

        bytes32 acceptDigest =
            _hashTypedDataV4(keccak256(abi.encode(CLAIM_TYPEHASH, id, newOwner, nonce, deadline)));
        require(ECDSA.recover(acceptDigest, ownerSig) == newOwner, "bad sig");

        Status f = status[id];
        status[id] = Status.TransferPending;
        _recoveryRebind(newOwner, id);
        status[id] = Status.Active;
        if (f != Status.Active) emit StatusChanged(id, f, Status.Active, msg.sender, "recover");
        emit Recovered(id, newOwner);
    }

    /// @notice GDPR-erasure ONLY (admin); drops the live ownerOf<->wallet link (§11.1 erasure flow).
    function burn(uint256 id) external onlyRole(DEFAULT_ADMIN_ROLE) {
        _burn(id);
        emit Burned(id);
    }

    function locked(uint256) external pure returns (bool) {
        return true;
    }

    function _recoveryRebind(address to, uint256 id) internal {
        _inRecovery = true;
        _update(to, id, address(0)); // auth=0 skips owner check; _inRecovery bypasses soulbound lock
        _inRecovery = false;
    }

    /// @dev Soulbound: block holder->holder transfers; allow mint (from==0), burn (to==0), recovery.
    function _update(address to, uint256 id, address auth) internal override returns (address) {
        address from = _ownerOf(id);
        if (from != address(0) && to != address(0) && !_inRecovery) revert Soulbound();
        return super._update(to, id, auth);
    }

    function supportsInterface(bytes4 i)
        public
        view
        override(ERC721, AccessControlEnumerable)
        returns (bool)
    {
        return i == 0xb45a3c0e || super.supportsInterface(i); // ERC-5192
    }
}
