// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";

/// @title ProtocolVersions — the records published to `ProtocolRegistry`.
/// @notice Authors the `dogtag-levelb/1` discovery set, the artifact set it binds, and the binding.
/// Both the publish script and the registry tests import these builders so the published shape and the
/// tested shape cannot drift.
///
/// # Two keys, two namespaces, two rotation rates
///
/// The discovery key and the artifact key are DELIBERATELY different namespaces so the two axes can
/// never collide, and they rotate independently: a zkey rotation mints a new artifact id and moves no
/// address, while an address rotation mints a new discovery id and leaves the artifacts alone.
///
/// A new id is reserved for a REAL change. Minting one for identical bytes publishes a second identity
/// for one set, which is how a consumer comes to believe two things differ when they do not. That rule
/// binds both keys equally.
///
/// `minAppVersion` is a mandatory argument with no default. It is a statement about which app BUILD may
/// act on this discovery set — not part of the artifacts' identity — and it is not knowable from this
/// file. Guessing one would publish a floor nobody verified.
///
/// Every address is likewise an argument, not a constant, so a fresh deployment cannot silently publish
/// a stale address.
library ProtocolVersions {
    /// @notice The discovery key. It is the SAME string the SDK stamps into every credential's
    /// `protocol.version` (`dogtag_standard::LEVEL_B_VERSION`) and the same key the prover's artifact
    /// registry and both mobile bundles resolve, so a credential's own claim and the on-chain discovery
    /// record name one protocol version. A client validating a claim compares the two directly; if they
    /// were allowed to differ, every discovery validation would fail closed.
    string internal constant LEVEL_B_VERSION = "dogtag-levelb/1";

    /// @notice The artifact key. A DIFFERENT namespace from the discovery key, so the two axes can never
    /// collide even at the same version number.
    string internal constant LEVEL_B_ARTIFACTS = "dogtag-levelb-artifacts/1";

    /// @notice The frozen circuit identity, from the consent ceremony. Moving this would be a false
    /// claim about which circuit the published set proves against.
    string internal constant CONSENT_CIRCUIT_ID = "consent.circom/DogTagConsent(6)";

    function levelBId() internal pure returns (bytes32) {
        return keccak256(bytes(LEVEL_B_VERSION));
    }

    function levelBArtifactsId() internal pure returns (bytes32) {
        return keccak256(bytes(LEVEL_B_ARTIFACTS));
    }

    function consentCircuitId() internal pure returns (bytes32) {
        return keccak256(bytes(CONSENT_CIRCUIT_ID));
    }

    /// @notice Builds the discovery set from the deployed component addresses.
    ///
    /// @param factory The `DogTagIssuerFactory` — where a provider's clone comes from, and the address
    /// the verification registry's immutable `rootIndex` resolves every anchored root through. The
    /// publish preflight asserts that equality against the chain.
    /// @param verificationRegistry The `VerificationRegistryConsent` a proof is submitted to.
    /// @param sbt The `DogTagSBTConsent`. `profileRoot` is write-once, so an SBT that did not mint a
    /// tag can never answer for it — this must be the SBT the verification registry names.
    /// @param verifier The `Groth16VerifierConsent` — the frozen ceremony VK's on-chain identity.
    /// @param providerRegistry The `ProviderRegistry` authority core; also the root of the resolver layer.
    function levelBDiscovery(
        address factory,
        address verificationRegistry,
        address sbt,
        address verifier,
        address providerRegistry
    ) internal pure returns (ProtocolRegistry.DiscoverySet memory) {
        return ProtocolRegistry.DiscoverySet({
            discoverySetId: keccak256(bytes(LEVEL_B_VERSION)),
            factory: factory,
            verificationRegistry: verificationRegistry,
            sbt: sbt,
            verifier: verifier,
            providerRegistry: providerRegistry,
            circuitId: keccak256(bytes(CONSENT_CIRCUIT_ID)),
            publishedAt: 0, // stamped by executeDiscoverySet
            active: false // set true by executeDiscoverySet
        });
    }

    /// @notice Builds the artifact set the discovery set binds. The four pins are the frozen consent
    /// artifacts'.
    ///
    /// @param minAppVersion The app floor — mandatory, with no default. See the library note.
    function levelBArtifacts(
        bytes32 zkeySha256,
        bytes32 witnessMobileSha256,
        bytes32 witnessServerR1csSha256,
        bytes32 witnessServerWasmSha256,
        string memory artifactBaseUrl,
        string memory minAppVersion
    ) internal pure returns (ProtocolRegistry.ArtifactSet memory) {
        return ProtocolRegistry.ArtifactSet({
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
