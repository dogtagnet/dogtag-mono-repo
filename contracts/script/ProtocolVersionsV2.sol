// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {ProtocolRegistryV2} from "../src/ProtocolRegistryV2.sol";

/// @title ProtocolVersionsV2 — the generation-2 records, as published to `ProtocolRegistryV2`.
/// @notice Authors the `dogtag-levelb/2` discovery set, the artifact set it binds, and the binding.
/// Both the publish script and the registry tests import these builders so the published shape and the
/// tested shape cannot drift.
///
/// # Why the DISCOVERY key bumps and the ARTIFACT key does not
///
/// `dogtag-levelb/2` is a new key: generation 2 rotates the factory, the verification registry, the
/// authority core and the root index, and the two generations must be tellable apart by a reader looking
/// at a claim or at the registry.
///
/// The artifact key stays `dogtag-levelb-artifacts/1`, and that is a correctness choice rather than
/// laziness. Two reasons, in order of weight:
///
///   1. **It is the truth.** The circuit, the ceremony, the frozen VK and all four artifact pins are
///      byte-for-byte unchanged by the provider-registry work. Minting `…-artifacts/2` for identical
///      bytes would publish a second identity for one artifact set, which is exactly what this
///      codebase's own rule forbids ("reserve a new id for a real bytes rotation, where apps must be
///      able to tell the sets apart").
///   2. **It keeps the app-gate diagnostic actionable.** Both mobile anchor resolvers carry the artifact
///      identity as a compile-time constant and `validate` refuses an anchor whose `artifactSet` does not
///      hash to its `artifactSetId`. Had the artifact key moved, an old build reaching the new registry
///      would fail with a caller-integrity error about a stitched anchor — true, and useless to the
///      holder. With the key unchanged it reaches the check that was designed to speak to this case and
///      refuses with `AppTooOld`, naming the floor.
///
/// `minAppVersion` therefore differs between the two registries for the same artifact id, and it should:
/// the floor is a statement about which app BUILD may act on a generation, not part of the artifacts'
/// identity. Which is also why it is a mandatory argument here with no default — the generation-2 floor
/// is the release that reads the root index instead of the bundled factory, and that release number is
/// not knowable from this file. Guessing one would publish a number nobody verified.
///
/// Every address is likewise an argument, not a constant, so a fresh deployment cannot silently publish
/// a stale ROAX address.
library ProtocolVersionsV2 {
    /// @notice The generation-2 discovery key. Generation 1's `dogtag-levelb/1` is NOT re-published here.
    string internal constant LEVEL_B_V2_VERSION = "dogtag-levelb/2";

    /// @notice The artifact key — deliberately generation 1's, because the artifacts are generation 1's.
    /// A DIFFERENT namespace from the discovery keys, so the two axes can never collide.
    string internal constant LEVEL_B_ARTIFACTS = "dogtag-levelb-artifacts/1";

    /// @notice The frozen circuit identity. Unchanged: no part of the provider-registry work touches the
    /// circuit, so a moved `circuitId` would be a false claim about which circuit generation 2 proves.
    string internal constant CONSENT_CIRCUIT_ID = "consent.circom/DogTagConsent(6)";

    function levelBV2Id() internal pure returns (bytes32) {
        return keccak256(bytes(LEVEL_B_V2_VERSION));
    }

    function levelBArtifactsId() internal pure returns (bytes32) {
        return keccak256(bytes(LEVEL_B_ARTIFACTS));
    }

    function consentCircuitId() internal pure returns (bytes32) {
        return keccak256(bytes(CONSENT_CIRCUIT_ID));
    }

    /// @notice Builds the generation-2 discovery set from the freshly deployed component addresses.
    ///
    /// @param factory The generation-2 `DogTagIssuerFactoryV2` — where a provider's clone comes from.
    /// @param verificationRegistry The generation-2 `VerificationRegistryConsent`.
    /// @param sbt The SHARED, REUSED `DogTagSBTConsent`. A fresh SBT holds no `profileRoot` for any
    /// existing tag, and that binding is write-once, so every existing tag would stop verifying.
    /// @param verifier The existing `Groth16VerifierConsent` — the frozen ceremony VK's on-chain identity.
    /// @param providerRegistry The `ProviderRegistry` authority core, in the registry's immutable
    /// `issuerRegistry` slot; also the root of the resolver layer.
    /// @param rootIndex The `CloneProvenanceRouter`, in the registry's immutable `rootIndex` slot. NOT
    /// `factory`: in generation 2 they are different contracts, and passing the factory here would
    /// resolve generation-2 roots only and lose every historical one.
    function levelBV2Discovery(
        address factory,
        address verificationRegistry,
        address sbt,
        address verifier,
        address providerRegistry,
        address rootIndex
    ) internal pure returns (ProtocolRegistryV2.DiscoverySet memory) {
        return ProtocolRegistryV2.DiscoverySet({
            discoverySetId: keccak256(bytes(LEVEL_B_V2_VERSION)),
            factory: factory,
            verificationRegistry: verificationRegistry,
            sbt: sbt,
            verifier: verifier,
            providerRegistry: providerRegistry,
            rootIndex: rootIndex,
            circuitId: keccak256(bytes(CONSENT_CIRCUIT_ID)),
            publishedAt: 0, // stamped by executeDiscoverySet
            active: false // set true by executeDiscoverySet
        });
    }

    /// @notice Builds the artifact set generation 2 binds. The four pins are the frozen consent
    /// artifacts', unchanged.
    ///
    /// @param minAppVersion The generation-2 app floor — mandatory, with no default. See the library note.
    function levelBArtifacts(
        bytes32 zkeySha256,
        bytes32 witnessMobileSha256,
        bytes32 witnessServerR1csSha256,
        bytes32 witnessServerWasmSha256,
        string memory artifactBaseUrl,
        string memory minAppVersion
    ) internal pure returns (ProtocolRegistryV2.ArtifactSet memory) {
        return ProtocolRegistryV2.ArtifactSet({
            artifactSetId: keccak256(bytes(LEVEL_B_ARTIFACTS)),
            zkeySha256: zkeySha256,
            witnessMobileSha256: witnessMobileSha256,
            witnessServerR1csSha256: witnessServerR1csSha256,
            witnessServerWasmSha256: witnessServerWasmSha256,
            artifactBaseUrl: artifactBaseUrl,
            minAppVersion: minAppVersion,
            publishedAt: 0, // stamped by executeArtifactSet
            active: false // set true by executeArtifactSet
        });
    }
}
