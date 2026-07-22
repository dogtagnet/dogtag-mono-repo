// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";

/// @notice Regression gate for the owner-hidden public contract surface.
contract OwnerHiddenSurfaceTest is Test {
    function _containsBefore(bytes memory haystack, bytes memory needle, uint256 end)
        internal
        pure
        returns (bool)
    {
        if (needle.length == 0 || needle.length > end) return false;
        for (uint256 i; i + needle.length <= end; i++) {
            bool match_ = true;
            for (uint256 j; j < needle.length; j++) {
                if (haystack[i + j] != needle[j]) {
                    match_ = false;
                    break;
                }
            }
            if (match_) return true;
        }
        return false;
    }

    function _abiContains(string memory artifact, string memory needle) internal view returns (bool) {
        bytes memory json = bytes(vm.readFile(artifact));
        bytes memory bytecodeMarker = bytes("],\"bytecode\":");
        uint256 abiEnd = type(uint256).max;
        for (uint256 i; i + bytecodeMarker.length <= json.length; i++) {
            bool markerMatch = true;
            for (uint256 j; j < bytecodeMarker.length; j++) {
                if (json[i + j] != bytecodeMarker[j]) {
                    markerMatch = false;
                    break;
                }
            }
            if (markerMatch) {
                abiEnd = i;
                break;
            }
        }
        require(abiEnd != type(uint256).max, "artifact ABI boundary missing");
        return _containsBefore(json, bytes(needle), abiEnd);
    }

    function test_only_owner_hidden_mint_and_verified_abis_remain() public {
        assertFalse(vm.exists("src/DogTagSBT.sol"), "owner-revealing SBT source remains");
        assertFalse(vm.exists("src/VerificationRegistry.sol"), "subject-bearing registry source remains");

        string memory sbtArtifact = "out/DogTagSBTConsent.sol/DogTagSBTConsent.json";
        assertTrue(_abiContains(sbtArtifact, "\"name\":\"mintCustodial\""), "custodial mint ABI missing");
        assertFalse(_abiContains(sbtArtifact, "\"name\":\"mint\""), "mint(to,...) ABI remains");

        string memory registryArtifact =
            "out/VerificationRegistryConsent.sol/VerificationRegistryConsent.json";
        assertTrue(_abiContains(registryArtifact, "\"name\":\"Verified\""), "Verified event missing");
        assertFalse(_abiContains(registryArtifact, "\"name\":\"subject\""), "subject-bearing ABI remains");
    }
}
