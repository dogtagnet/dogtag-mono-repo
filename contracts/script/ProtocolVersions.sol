// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";

/// @title ProtocolVersions — the CURRENT protocol records, as published to `ProtocolRegistry`.
/// @notice Single source of truth for the `dogtag-levela/1` + `dogtag-levelb/1` records (M7 P3, §5).
/// BOTH the publish script (`PublishProtocolVersions.s.sol`) and the registry test import this, so the
/// published data and the tested data can never drift.
///
/// # Two axes (R-5) — this library authors each level as THREE separate values
///
/// Nothing here is a single blob any more. Each protocol level is:
///   * a [`ProtocolRegistry.ContractSet`] keyed by `keccak256("dogtag-levelb/1")` — the trio +
///     verifier + on-chain circuitId; and
///   * a [`ProtocolRegistry.ArtifactSet`] keyed by `keccak256("dogtag-levelb-artifacts/1")` — the four
///     fetch-pins + base URL + `minAppVersion`; plus
///   * the BINDING that points the first at the second.
/// Rotating a zkey means authoring a NEW artifact set (`…-artifacts/2`) and re-pointing the binding.
/// The contract set is not touched, so no trio address moves and no contract is redeployed. This is the
/// FIRST publication, so it uses the two-axis scheme from the start — there is nothing to migrate.
///
/// # Where every value comes from (all confirmed, none invented)
///
///   * TRIO addresses — `contracts/deployments/roax.json`. Level-B is the canonical M5 pairing
///     (`_m5_custodial_issuance.constructor_wiring`): registry `0xb9B313C1`, sbt `0x96Cba458`
///     (== registry.sbt()), verifier `0x272be146` (== registry.zkVerifier()), factory/rootIndex
///     `0xd3179AbB` (== registry.rootIndex()). Level-A is the still-live trio: registry `0x4E2f0996`,
///     sbt `0x1FB89865`, verifier `0xEEFCfAF0` (the active v2 Groth16Verifier since executeZkVerifier
///     on 2026-07-02, `_zk_ceremony`), factory/rootIndex `0xd3179AbB` (the SHARED factory).
///   * ARTIFACT PINS — `crates/dogtag-prover-rs/src/artifact.rs` (`LEVEL_A_V1_DESCRIPTOR` /
///     `LEVEL_B_V1_DESCRIPTOR`). Those pins are file-verified against the committed
///     `circuits/build/*` artifacts by the crate's `*_descriptor_pins_match_the_real_artifacts` tests,
///     which stream-hash even the 24-64 MB zkeys cheaply. This library reuses the SAME hex, and the
///     registry test re-hashes the (feasible) committed `.wasm` in-EVM to demonstrate the tie.
///   * `witnessMobileSha256 = 0` — the `.graph` is NOT committed (CI fetches it), so there is no
///     byte-stable in-tree hash to pin. `0` means "unpinned"; the mobile resolver pins it from the
///     anchor at fetch time (§3.5). This matches `artifact.rs`'s `witness_graph.sha256 = None`.
///   * `verifier` is the ON-CHAIN VK IDENTITY (§3.2), NOT a hash. The off-chain `verification_key.json`
///     hash (`27879dd7…` / `4a4cee60…`) is deliberately absent here — it lives only in the signed
///     manifest (`crates/dogtag-prover-rs/src/manifest.rs`).
library ProtocolVersions {
    string internal constant LEVEL_A_VERSION = "dogtag-levela/1";
    string internal constant LEVEL_B_VERSION = "dogtag-levelb/1";

    /// @notice The artifact-axis ids. Deliberately a DIFFERENT namespace from the contract-set ids so
    /// the two axes can never collide, and so an artifact rotation reads as what it is
    /// (`…-artifacts/2`) rather than as a protocol-level bump.
    string internal constant LEVEL_A_ARTIFACTS = "dogtag-levela-artifacts/1";
    string internal constant LEVEL_B_ARTIFACTS = "dogtag-levelb-artifacts/1";

    function levelAId() internal pure returns (bytes32) {
        return keccak256(bytes(LEVEL_A_VERSION));
    }

    function levelBId() internal pure returns (bytes32) {
        return keccak256(bytes(LEVEL_B_VERSION));
    }

    function levelAArtifactsId() internal pure returns (bytes32) {
        return keccak256(bytes(LEVEL_A_ARTIFACTS));
    }

    function levelBArtifactsId() internal pure returns (bytes32) {
        return keccak256(bytes(LEVEL_B_ARTIFACTS));
    }

    /// @notice `dogtag-levela/1` on-chain set — the pre-M7 Level-A verification trio (live until cutover).
    function levelAContracts() internal pure returns (ProtocolRegistry.ContractSet memory) {
        return ProtocolRegistry.ContractSet({
            contractSetId: keccak256(bytes(LEVEL_A_VERSION)),
            factory: 0xd3179AbBfb0274D0a5F7017d76015A93C159511D,
            verificationRegistry: 0x4E2f0996e1CB4E24F1053346f3da2186906835E8,
            sbt: 0x1FB8986573Ac36d532cF7d5a5352202B094D4233,
            verifier: 0xEEFCfAF026931b7325472A88fd14Ee780Da13559,
            circuitId: keccak256("verification.circom/DogTagVerification(24,5)"),
            publishedAt: 0, // stamped by executeContractSet
            active: false // set true by executeContractSet
        });
    }

    /// @notice `dogtag-levela-artifacts/1` — the Level-A proving artifacts currently bound to it.
    function levelAArtifacts() internal pure returns (ProtocolRegistry.ArtifactSet memory) {
        return ProtocolRegistry.ArtifactSet({
            artifactSetId: keccak256(bytes(LEVEL_A_ARTIFACTS)),
            zkeySha256: 0x9e3636b9c12b57b8662e34505a01e19bfc87a99189c994b0d87bc2e3dcdcd992,
            witnessMobileSha256: bytes32(0), // .graph not committed — unpinned (§3.5)
            witnessServerR1csSha256: 0x8890e52860b5b43535143682892bd6fa87ffefab3084b4eb2186b91c99be6fe8,
            witnessServerWasmSha256: 0xa2a2d7e28899e59019663f416c1ec1429ca0a35ead9363c7f832d9e476df874f,
            artifactBaseUrl: "https://artifacts.dogtag.io/levela1",
            minAppVersion: "1.0.0",
            publishedAt: 0, // stamped by executeArtifactSet
            active: false // set true by executeArtifactSet
        });
    }

    /// @notice `dogtag-levelb/1` on-chain set — the canonical M5 Level-B owner-blind consent trio.
    function levelBContracts() internal pure returns (ProtocolRegistry.ContractSet memory) {
        return ProtocolRegistry.ContractSet({
            contractSetId: keccak256(bytes(LEVEL_B_VERSION)),
            factory: 0xd3179AbBfb0274D0a5F7017d76015A93C159511D,
            verificationRegistry: 0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87,
            sbt: 0x96Cba4580D79bc9b8e51Fc1B3a044A29592AfFFc,
            verifier: 0x272be146C0aEd6401000E9Aa8241201F6f0fdF1a,
            circuitId: keccak256("consent.circom/DogTagConsent(6)"),
            publishedAt: 0, // stamped by executeContractSet
            active: false // set true by executeContractSet
        });
    }

    /// @notice `dogtag-levelb-artifacts/1` — the Level-B proving artifacts currently bound to it.
    function levelBArtifacts() internal pure returns (ProtocolRegistry.ArtifactSet memory) {
        return ProtocolRegistry.ArtifactSet({
            artifactSetId: keccak256(bytes(LEVEL_B_ARTIFACTS)),
            zkeySha256: 0xf83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868,
            witnessMobileSha256: bytes32(0), // .graph not committed — unpinned (§3.5)
            witnessServerR1csSha256: 0x828e2923a159b04f2de421d4b447f8c85356677f4f83a5af55b42eb2b4f9b6b7,
            witnessServerWasmSha256: 0x482debcff5a4325c008dd00e4476bba011d0a706da955e3129d114f996a913e6,
            artifactBaseUrl: "https://artifacts.dogtag.io/levelb1",
            minAppVersion: "1.4.0", // M-4 PR4 app release floor (iOS + Android)
            publishedAt: 0, // stamped by executeArtifactSet
            active: false // set true by executeArtifactSet
        });
    }
}
