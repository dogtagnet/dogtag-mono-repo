// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

/// @notice ERC-5192 Minimal Soulbound NFT interface. interfaceId = 0xb45a3c0e.
interface IERC5192 {
    /// @notice Emitted when the locking status is changed to locked.
    event Locked(uint256 tokenId);
    /// @notice Emitted when the locking status is changed to unlocked.
    event Unlocked(uint256 tokenId);
    /// @notice Returns the locking status of a Soulbound Token.
    function locked(uint256 tokenId) external view returns (bool);
}
