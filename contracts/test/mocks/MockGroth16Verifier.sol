// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {IGroth16Verifier} from "../../src/VerificationRegistry.sol";

/// @notice Test double for the snarkjs-generated Groth16Verifier. The real verifier is produced from
/// the circom circuit's phase-2 zkey (circuits/); this lets the registry's ZK-path binding logic
/// (relayer/whitelist/range/keyOf/ownerOf/nullifier/rootIssuer/isValid) be tested independently.
contract MockGroth16Verifier is IGroth16Verifier {
    bool public result = true;

    function setResult(bool r) external {
        result = r;
    }

    function verifyProof(
        uint256[2] calldata,
        uint256[2][2] calldata,
        uint256[2] calldata,
        uint256[7] calldata
    ) external view returns (bool) {
        return result;
    }
}
