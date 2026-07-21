// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";

/// @title ProtocolVersions — the CURRENT protocol records, as published to `ProtocolRegistry`.
/// @notice Authors the single owner-hidden `dogtag-levelb/1` record. The identifier is retained
/// byte-for-byte as an internal compatibility key. Both the publish script and registry tests import
/// these builders so the published shape and tested shape cannot drift.
///
/// # Two axes
///
/// The protocol version is represented as:
///   * a [`ProtocolRegistry.ContractSet`] keyed by `keccak256("dogtag-levelb/1")` — the trio +
///     verifier + on-chain circuitId; and
///   * a [`ProtocolRegistry.ArtifactSet`] keyed by `keccak256("dogtag-levelb-artifacts/1")` — the four
///     fetch-pins + base URL + `minAppVersion`; plus
///   * the BINDING that points the first at the second.
/// Rotating a zkey means authoring a NEW artifact set (`…-artifacts/2`) and re-pointing the binding.
/// The contract set is not touched, so no trio address moves and no contract is redeployed. This is the
/// FIRST publication, so it uses the two-axis scheme from the start — there is nothing to migrate.
///
/// Addresses, artifact pins, and the artifact base URL are arguments rather than network-specific
/// constants. `PublishProtocolVersions.s.sol` obtains every one from mandatory environment variables,
/// preventing a fresh deployment from silently publishing a stale ROAX address or artifact location.
library ProtocolVersions {
    string internal constant LEVEL_B_VERSION = "dogtag-levelb/1";

    /// @notice The artifact-axis ids. Deliberately a DIFFERENT namespace from the contract-set ids so
    /// the two axes can never collide, and so an artifact rotation reads as what it is
    /// (`…-artifacts/2`) rather than as a protocol-level bump.
    string internal constant LEVEL_B_ARTIFACTS = "dogtag-levelb-artifacts/1";

    function levelBId() internal pure returns (bytes32) {
        return keccak256(bytes(LEVEL_B_VERSION));
    }

    function levelBArtifactsId() internal pure returns (bytes32) {
        return keccak256(bytes(LEVEL_B_ARTIFACTS));
    }

    /// @notice Builds the owner-hidden on-chain set from the freshly deployed component addresses.
    function levelBContracts(address factory, address verificationRegistry, address sbt, address verifier)
        internal
        pure
        returns (ProtocolRegistry.ContractSet memory)
    {
        return ProtocolRegistry.ContractSet({
            contractSetId: keccak256(bytes(LEVEL_B_VERSION)),
            factory: factory,
            verificationRegistry: verificationRegistry,
            sbt: sbt,
            verifier: verifier,
            circuitId: keccak256("consent.circom/DogTagConsent(6)"),
            publishedAt: 0, // stamped by executeContractSet
            active: false // set true by executeContractSet
        });
    }

    /// @notice Builds the independently rotatable owner-hidden proving artifact set.
    function levelBArtifacts(
        bytes32 zkeySha256,
        bytes32 witnessMobileSha256,
        bytes32 witnessServerR1csSha256,
        bytes32 witnessServerWasmSha256,
        string memory artifactBaseUrl
    ) internal pure returns (ProtocolRegistry.ArtifactSet memory) {
        return ProtocolRegistry.ArtifactSet({
            artifactSetId: keccak256(bytes(LEVEL_B_ARTIFACTS)),
            zkeySha256: zkeySha256,
            witnessMobileSha256: witnessMobileSha256,
            witnessServerR1csSha256: witnessServerR1csSha256,
            witnessServerWasmSha256: witnessServerWasmSha256,
            artifactBaseUrl: artifactBaseUrl,
            minAppVersion: "1.4.0", // M-4 PR4 app release floor (iOS + Android)
            publishedAt: 0, // stamped by executeArtifactSet
            active: false // set true by executeArtifactSet
        });
    }
}
