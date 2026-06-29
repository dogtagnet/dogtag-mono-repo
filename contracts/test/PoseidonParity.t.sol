// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";
import {PoseidonT3} from "poseidon-solidity/PoseidonT3.sol";
import {PoseidonT4} from "poseidon-solidity/PoseidonT4.sol";
import {PoseidonT6} from "poseidon-solidity/PoseidonT6.sol";

/// @notice Gate A (Solidity leg) — assert the on-chain Poseidon equals the shared
/// circuits/poseidon-vectors.json bit-for-bit at every arity (architecture §13.9(b), impl §11.2(d)/§9):
///   - fold  (2 in / t=3) -> poseidon-solidity PoseidonT3
///   - node  (3 in / t=4) -> poseidon-solidity PoseidonT4
///   - leaf  (5 in / t=6) -> poseidon-solidity PoseidonT6
///   - nullifier (6 in / t=7) -> circomlib-exact Poseidon(6), deployed from circomlibjs initcode
///     (poseidon-solidity 0.0.5 ships only T2..T6; the T7 bytecode here is circomlib-identical and
///      is what the VerificationRegistry normal path deploys for the on-chain nullifier — Phase 2.5).
/// circomlib is the reference-of-record; any mismatch fails the CI/lockfile gate.
contract PoseidonParityTest is Test {
    using stdJson for string;

    string json;
    // circomlibjs poseidonContract Poseidon(6): function poseidon(uint256[6]) returns (uint256)
    address poseidon6;

    function setUp() public {
        json = vm.readFile("../circuits/poseidon-vectors.json");
        bytes memory initcode = vm.parseBytes(vm.readFile("test/poseidon6.initcode"));
        address a;
        assembly {
            a := create(0, add(initcode, 0x20), mload(initcode))
        }
        require(a != address(0), "Poseidon(6) deploy failed");
        poseidon6 = a;
    }

    function _inputs(uint256 i) internal view returns (uint256[] memory) {
        string memory base = string.concat(".vectors[", vm.toString(i), "].in");
        return json.readUintArray(base);
    }

    function _want(uint256 i) internal view returns (uint256) {
        return json.readUint(string.concat(".vectors[", vm.toString(i), "].out_dec"));
    }

    function _arity(uint256 i) internal view returns (uint256) {
        return json.readUint(string.concat(".vectors[", vm.toString(i), "].arity"));
    }

    function _count() internal view returns (uint256) {
        // vectors is a small fixed array; probe until a read reverts is awkward, so read a known length.
        return json.readUint(".vector_count");
    }

    function _callPoseidon6(uint256[6] memory in_) internal view returns (uint256 out) {
        (bool ok, bytes memory ret) =
            poseidon6.staticcall(abi.encodeWithSignature("poseidon(uint256[6])", in_));
        require(ok, "poseidon6 call failed");
        out = abi.decode(ret, (uint256));
    }

    function test_poseidon_parity_all_vectors() public view {
        uint256 n = _count();
        require(n > 0, "no vectors");
        bool sawAnchor;
        bool sawNullifier;
        for (uint256 i = 0; i < n; i++) {
            uint256[] memory in_ = _inputs(i);
            uint256 arity = _arity(i);
            require(in_.length == arity, "arity/in mismatch");
            uint256 got;
            if (arity == 2) {
                got = PoseidonT3.hash([in_[0], in_[1]]);
            } else if (arity == 3) {
                got = PoseidonT4.hash([in_[0], in_[1], in_[2]]);
            } else if (arity == 5) {
                got = PoseidonT6.hash([in_[0], in_[1], in_[2], in_[3], in_[4]]);
            } else if (arity == 6) {
                got = _callPoseidon6([in_[0], in_[1], in_[2], in_[3], in_[4], in_[5]]);
                sawNullifier = true;
            } else {
                revert("unexpected arity");
            }
            uint256 want = _want(i);
            assertEq(got, want, "poseidon parity mismatch");
            if (in_.length == 2 && in_[0] == 1 && in_[1] == 2) {
                sawAnchor = true;
                assertEq(got, 0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a, "anchor");
            }
        }
        assertTrue(sawAnchor, "anchor poseidon([1,2]) missing");
        assertTrue(sawNullifier, "t=7 nullifier vector missing");
    }
}
