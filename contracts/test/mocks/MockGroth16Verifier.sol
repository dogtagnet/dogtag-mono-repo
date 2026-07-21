// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {IGroth16VerifierConsent} from "../../src/VerificationRegistryConsent.sol";

/// @notice Test double for the snarkjs-generated Groth16Verifier. The real verifier is produced from
/// the consent circuit's phase-2 zkey; this lets the registry's owner-hidden binding logic be tested
/// independently from pairing verification.
contract MockGroth16Verifier is IGroth16VerifierConsent {
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
