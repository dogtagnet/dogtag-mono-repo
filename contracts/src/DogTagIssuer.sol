// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Initializable} from "@openzeppelin/contracts/proxy/utils/Initializable.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {IssuerRegistry} from "./IssuerRegistry.sol";

/// @notice The protocol-global write-once root->clone index (impl §11.10(a), architecture §13.9).
interface IRootIndex {
    function registerRoot(bytes32 root) external;
}

/// @title DogTagIssuer — per-record-type anchoring contract (EIP-1167 clone implementation).
/// @notice Issues/revokes bytes32 Poseidon roots `R`; every write gated by IssuerRegistry
/// (impl §11.1: C-1 locked impl, H-1 originator binding). Issuance adds ZERO on-chain hashing —
/// it stores a bytes32 and registers the write-once rootIssuer[R] index (§11.10(a), CHANGESPEC-v4:
/// no zkCommit/kecOf/ZkCommitment — a single Poseidon root, isValid(R) checked directly).
contract DogTagIssuer is Initializable {
    IssuerRegistry public registry;
    IRootIndex public rootIndex; // the factory (write-once rootIssuer index)
    bytes32 public recordType;
    string public name;

    mapping(bytes32 => uint256) public issuedAt; // 0 = not issued
    mapping(bytes32 => uint256) public revokedAt; // 0 = not revoked
    mapping(bytes32 => address) public issuedBy; // H-1 originator

    event RootIssued(bytes32 indexed root, address indexed by, uint256 ts);
    event RootRevoked(bytes32 indexed root, address indexed by, uint256 ts);

    error NotWhitelisted();
    error BadRoot();
    error NotOriginatorOrAdmin();

    constructor() {
        _disableInitializers(); // C-1: lock the implementation (clones initialize)
    }

    modifier onlyWhitelisted() {
        if (!registry.isWhitelistedFor(recordType, msg.sender)) revert NotWhitelisted();
        _;
    }

    function initialize(string calldata n, bytes32 rt, address reg, address index) external initializer {
        require(reg != address(0) && index != address(0), "zero");
        name = n;
        recordType = rt;
        registry = IssuerRegistry(reg);
        rootIndex = IRootIndex(index);
    }

    function issue(bytes32 r) public onlyWhitelisted {
        if (r == bytes32(0) || issuedAt[r] != 0) revert BadRoot();
        issuedAt[r] = block.timestamp;
        issuedBy[r] = msg.sender;
        rootIndex.registerRoot(r); // write-once rootIssuer[r] = this clone (§11.10(a))
        emit RootIssued(r, msg.sender, block.timestamp);
    }

    function revoke(bytes32 r) public onlyWhitelisted {
        if (issuedAt[r] == 0 || revokedAt[r] != 0) revert BadRoot();
        // H-1: only the original issuer OR the protocol admin (registry DEFAULT_ADMIN) may revoke.
        if (msg.sender != issuedBy[r] && !IAccessControl(address(registry)).hasRole(0x00, msg.sender)) {
            revert NotOriginatorOrAdmin();
        }
        revokedAt[r] = block.timestamp;
        emit RootRevoked(r, msg.sender, block.timestamp);
    }

    function bulkIssue(bytes32[] calldata rs) external onlyWhitelisted {
        for (uint256 i; i < rs.length; i++) {
            issue(rs[i]);
        }
    }

    function bulkRevoke(bytes32[] calldata rs) external onlyWhitelisted {
        for (uint256 i; i < rs.length; i++) {
            revoke(rs[i]);
        }
    }

    /// @notice Admin mass-revoke for a compromised signer (delisting is forward-only — §13.3).
    /// Bypasses originator binding; gated by the registry's protocol admin only.
    function adminRevoke(bytes32[] calldata rs) external {
        require(IAccessControl(address(registry)).hasRole(0x00, msg.sender), "!admin");
        for (uint256 i; i < rs.length; i++) {
            bytes32 r = rs[i];
            if (issuedAt[r] != 0 && revokedAt[r] == 0) {
                revokedAt[r] = block.timestamp;
                emit RootRevoked(r, msg.sender, block.timestamp);
            }
        }
    }

    function isIssued(bytes32 r) external view returns (bool) {
        return issuedAt[r] != 0;
    }

    function isRevoked(bytes32 r) external view returns (bool) {
        return revokedAt[r] != 0;
    }

    function isValid(bytes32 r) external view returns (bool) {
        return issuedAt[r] != 0 && revokedAt[r] == 0;
    }
}
