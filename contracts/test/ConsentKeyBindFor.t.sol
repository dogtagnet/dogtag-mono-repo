// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {ConsentKeyRegistry} from "../src/ConsentKeyRegistry.sol";

/// @notice Tests the gasless meta-tx bind `bindConsentKeyFor`: the wallet OWNER signs the EIP-712
/// BindConsentKey digest off-chain, and a DIFFERENT address (the relayer) submits the bind on-chain.
/// Mirrors the off-chain `sign_bind_key` helper in stacks/vet/api/tests/verify_onchain.rs.
contract ConsentKeyBindForTest is Test {
    ConsentKeyRegistry consentKeys;

    uint256 ownerPk = uint256(keccak256("owner-key"));
    address owner;
    address relayer = address(0x9E1A);

    bytes32 keyHash = keccak256("babyjub-keyhash");

    function setUp() public {
        consentKeys = new ConsentKeyRegistry();
        owner = vm.addr(ownerPk);
    }

    /// EIP-712 digest for BindConsentKey(hash, wallet, nonce) over the DogTag/1 domain.
    function _bindDigest(address wallet, bytes32 h, uint256 nonce) internal view returns (bytes32) {
        bytes32 domainSep = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256(bytes("DogTag")),
                keccak256(bytes("1")),
                block.chainid,
                address(consentKeys)
            )
        );
        bytes32 structHash = keccak256(
            abi.encode(
                keccak256("BindConsentKey(bytes32 babyJubPubKeyHash,address wallet,uint256 nonce)"),
                h,
                wallet,
                nonce
            )
        );
        return keccak256(abi.encodePacked("\x19\x01", domainSep, structHash));
    }

    function _sign(uint256 pk, bytes32 digest) internal pure returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, digest);
        return abi.encodePacked(r, s, v);
    }

    // ---- positive: owner signs, relayer submits ----
    function test_bindConsentKeyFor_relayer_submits_owner_sig() public {
        uint256 nonce = consentKeys.bindNonce(owner);
        bytes memory sig = _sign(ownerPk, _bindDigest(owner, keyHash, nonce));

        vm.expectEmit(true, false, false, true);
        emit ConsentKeyRegistry.ConsentKeyBound(owner, keyHash, nonce);
        vm.prank(relayer); // DIFFERENT address than the signer
        consentKeys.bindConsentKeyFor(owner, keyHash, sig);

        assertEq(consentKeys.keyOf(owner), keyHash, "key must be bound to owner");
        assertEq(consentKeys.bindNonce(owner), nonce + 1, "nonce must advance");
    }

    // ---- negative: wrong signer (not the owner) reverts ----
    function test_bindConsentKeyFor_wrong_signer_reverts() public {
        uint256 attackerPk = uint256(keccak256("attacker-key"));
        uint256 nonce = consentKeys.bindNonce(owner);
        // attacker signs a digest claiming to bind `owner`, but ECDSA recovers the attacker, not owner.
        bytes memory sig = _sign(attackerPk, _bindDigest(owner, keyHash, nonce));

        vm.prank(relayer);
        vm.expectRevert("bad sig");
        consentKeys.bindConsentKeyFor(owner, keyHash, sig);
    }

    // ---- negative: zero wallet reverts ----
    function test_bindConsentKeyFor_zero_wallet_reverts() public {
        bytes memory sig = _sign(ownerPk, _bindDigest(address(0), keyHash, 0));
        vm.prank(relayer);
        vm.expectRevert("zero wallet");
        consentKeys.bindConsentKeyFor(address(0), keyHash, sig);
    }

    // ---- replay guard: reusing the same signature reverts (nonce advanced) ----
    function test_bindConsentKeyFor_replay_reverts() public {
        uint256 nonce = consentKeys.bindNonce(owner);
        bytes memory sig = _sign(ownerPk, _bindDigest(owner, keyHash, nonce));
        vm.prank(relayer);
        consentKeys.bindConsentKeyFor(owner, keyHash, sig);

        vm.prank(relayer);
        vm.expectRevert("bad sig"); // nonce moved on -> old digest recovers a different/non-owner address
        consentKeys.bindConsentKeyFor(owner, keyHash, sig);
    }
}
