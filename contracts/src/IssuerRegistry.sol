// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {AccessControlDefaultAdminRules} from
    "@openzeppelin/contracts/access/extensions/AccessControlDefaultAdminRules.sol";

/// @title IssuerRegistry — the central whitelist gate (impl §11.1, architecture §4.3, §13.1 C-2/H-3).
/// @notice Per-recordType, per-signer scoping (no global boolean). One logical issuer entity may map
/// to MANY whitelisted signer addresses (backend-derived + browser wallet) — the contract grants a
/// capability to an address; the issuer<->signers mapping is an off-chain view. The same machinery
/// scopes verifier capability under the `VERIFY:`-prefixed key namespace (architecture §4.3) — a
/// groomer can verify a purpose without holding any issuer role.
contract IssuerRegistry is AccessControlDefaultAdminRules {
    bytes32 public constant WHITELIST_ADMIN = keccak256("WHITELIST_ADMIN");
    /// @notice Dedicated role for SBT profile mint/update — distinct from record issuers (§13.1 C-2).
    bytes32 public constant PROFILE_ISSUER_ROLE = keccak256("PROFILE_ISSUER_ROLE");

    mapping(bytes32 => mapping(address => bool)) private _wl; // recordType (or VERIFY:key) => signer => ok

    event Whitelisted(bytes32 indexed recordType, address indexed signer);
    event Delisted(bytes32 indexed recordType, address indexed signer);

    /// @param adminMultisig protocol multisig — receives DEFAULT_ADMIN (two-step + 3-day delay) + WHITELIST_ADMIN.
    constructor(address adminMultisig) AccessControlDefaultAdminRules(3 days, adminMultisig) {
        _grantRole(WHITELIST_ADMIN, adminMultisig);
    }

    function whitelistFor(bytes32 recordType, address signer) external onlyRole(WHITELIST_ADMIN) {
        _wl[recordType][signer] = true;
        emit Whitelisted(recordType, signer);
    }

    function delistFor(bytes32 recordType, address signer) external onlyRole(WHITELIST_ADMIN) {
        _wl[recordType][signer] = false;
        emit Delisted(recordType, signer);
    }

    function isWhitelistedFor(bytes32 recordType, address signer) external view returns (bool) {
        return _wl[recordType][signer];
    }
}
