// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {Ownable2Step, Ownable} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {DogTagIssuer} from "./DogTagIssuer.sol";

/// @title DogTagIssuerFactory — EIP-1167 clone deployer + write-once rootIssuer index.
/// @notice Permissioned (M-1): only the protocol multisig deploys clones, salt =
/// keccak256(recordType, business) (stops front-running/squatting). Also serves as the
/// protocol-global write-once `rootIssuer[R]` index (impl §11.10(a)): a clone calls
/// `registerRoot(R)` from inside `issue(R)`, and the VerificationRegistry resolves the issuing
/// clone FROM the root (recordType->clone is one-to-many, so it can't resolve by recordType).
contract DogTagIssuerFactory is Ownable2Step {
    address public immutable implementation;
    address public immutable registry;

    mapping(address => bool) public isClone; // deployed by this factory
    mapping(bytes32 => address) public rootIssuer; // R -> issuing clone (write-once)

    event IssuerCreated(address indexed clone, bytes32 indexed recordType, string name);
    event RootRegistered(bytes32 indexed root, address indexed clone);

    constructor(address impl, address registry_, address admin) Ownable(admin) {
        require(impl != address(0) && registry_ != address(0), "zero");
        implementation = impl;
        registry = registry_;
    }

    function _salt(bytes32 recordType, address business) internal pure returns (bytes32) {
        return keccak256(abi.encode(recordType, business));
    }

    function createIssuer(string calldata name, bytes32 recordType, address business)
        external
        onlyOwner
        returns (address clone)
    {
        clone = Clones.cloneDeterministic(implementation, _salt(recordType, business));
        isClone[clone] = true;
        DogTagIssuer(clone).initialize(name, recordType, registry, address(this));
        emit IssuerCreated(clone, recordType, name);
    }

    function predictIssuer(bytes32 recordType, address business) external view returns (address) {
        return Clones.predictDeterministicAddress(implementation, _salt(recordType, business), address(this));
    }

    /// @notice Write-once registration of `root -> issuing clone` (§11.10(a), audit-11 V4-C1/M1).
    function registerRoot(bytes32 root) external {
        require(isClone[msg.sender], "!clone");
        require(rootIssuer[root] == address(0), "root taken"); // strictly write-once
        rootIssuer[root] = msg.sender;
        emit RootRegistered(root, msg.sender);
    }
}
