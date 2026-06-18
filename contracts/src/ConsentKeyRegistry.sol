// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {EIP712} from "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";

/// @title ConsentKeyRegistry — one-time (rotatable) BabyJubjub <-> secp256k1 binding (impl §11.8(b)).
/// @notice The secp256k1 wallet authorizes a BabyJubjub consent key via a cheap on-chain ecrecover;
/// the ZK path then proves `keyHash == Poseidon(Ax,Ay)` and the registry checks `keyOf[subject]==keyHash`
/// — binding the consent key to `subject` WITHOUT putting secp256k1 in-circuit. Rotation is supported
/// (§11.9(j): not one-time-irrevocable → avoids lost-key lockout) via a per-wallet bind nonce.
contract ConsentKeyRegistry is EIP712 {
    mapping(address => bytes32) public keyOf; // userWallet => Poseidon(Ax,Ay)
    mapping(address => uint256) public bindNonce; // replay guard for rotation

    bytes32 private constant BIND_TYPEHASH =
        keccak256("BindConsentKey(bytes32 babyJubPubKeyHash,address wallet,uint256 nonce)");

    event ConsentKeyBound(address indexed wallet, bytes32 babyJubPubKeyHash, uint256 nonce);

    constructor() EIP712("DogTag", "1") {}

    /// @param babyJubPubKeyHash Poseidon(Ax, Ay) of the per-pet BabyJubjub consent pubkey (§11.9(j)).
    /// @param ecdsaSig EIP-712 signature by `msg.sender` over BindConsentKey(hash, msg.sender, nonce).
    function bindConsentKey(bytes32 babyJubPubKeyHash, bytes calldata ecdsaSig) external {
        uint256 nonce = bindNonce[msg.sender];
        bytes32 digest =
            _hashTypedDataV4(keccak256(abi.encode(BIND_TYPEHASH, babyJubPubKeyHash, msg.sender, nonce)));
        require(ECDSA.recover(digest, ecdsaSig) == msg.sender, "bad sig");
        keyOf[msg.sender] = babyJubPubKeyHash;
        bindNonce[msg.sender] = nonce + 1;
        emit ConsentKeyBound(msg.sender, babyJubPubKeyHash, nonce);
    }
}
