// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";
import {ProtocolVersions} from "./ProtocolVersions.sol";

/// @notice The four immutable bindings a deployed `VerificationRegistryConsent` exposes, typed as plain
/// addresses so this script can compare them without importing the registry's own interface types. The
/// selectors are the registry's public auto-getters, so this interface is ABI-identical to it.
interface IDeployedVerificationRegistry {
    function providerRegistry() external view returns (address);
    function sbt() external view returns (address);
    function rootIndex() external view returns (address);
    function zkVerifier() external view returns (address);
}

/// @notice Publishes the `dogtag-levelb/1` version to `ProtocolRegistry`. The record DATA is in
/// `ProtocolVersions` — the single source of truth this and the registry tests share.
///
/// # Two axes, so the version publishes THREE things
///
/// A [`ProtocolRegistry.DiscoverySet`], a [`ProtocolRegistry.ArtifactSet`], and the BINDING between
/// them. All three writes are timelocked but their timelocks run CONCURRENTLY: phase 1 proposes all
/// three, so this is still a TWO-phase rollout.
///
///   1. `PublishProtocolVersionsPropose` — preflights, then stages all three. Starts the registry's
///      immutable `PUBLISH_TIMELOCK` on each at once.
///   2. `PublishProtocolVersionsExecute` — run AFTER the timelock elapses. Sets FIRST, then the
///      binding (`executeArtifactBinding` requires both sides published AND active).
///
/// Both are `PUBLISHER_ROLE`-gated. Every input is a mandatory env var under the `PUBLISH_` namespace:
/// `PUBLISH_PROTOCOL_REGISTRY`, `PUBLISH_FACTORY`, `PUBLISH_VERIFICATION_REGISTRY`, `PUBLISH_SBT`,
/// `PUBLISH_VERIFIER`, `PUBLISH_PROVIDER_REGISTRY`, `PUBLISH_ZKEY_SHA256`,
/// `PUBLISH_WITNESS_MOBILE_SHA256`, `PUBLISH_R1CS_SHA256`, `PUBLISH_WASM_SHA256`,
/// `PUBLISH_ARTIFACTS_URL`, `PUBLISH_MIN_APP_VERSION`. There are no fallbacks and no default app floor:
/// a stale-network default is the one class of mistake this script cannot detect, because a wrong
/// address that is well-formed looks exactly like a right one.
///
/// The names are namespaced rather than bare (`SBT`, `VERIFIER`, `FACTORY`) because `vm.setEnv` mutates
/// the PROCESS environment and `forge test` runs test contracts in parallel, so two suites driving two
/// scripts through the same bare names interfere. That has produced a real, hard-to-attribute flake in
/// this repository before; the namespace removes it by construction rather than by timing.
///
/// # Why there is a coherence PREFLIGHT, and why it lives here
///
/// A discovery record is a set of addresses an operator transcribes, and a mis-transcribed one is the
/// unrecoverable failure this registry exists to avoid: a consumer that follows a wrong `factory`
/// resolves no root at all, and one that follows a wrong `providerRegistry` asks the wrong contract
/// whether an issuer was authorized. Nothing about either mistake looks wrong at publish time.
///
/// All four of the other addresses are checkable against the chain, because `VerificationRegistryConsent`
/// pins three of them in immutable slots and exposes each as a getter, and the fourth (`verifier`)
/// against its current `zkVerifier`. [`preflight`] reads them back and refuses to propose on any
/// disagreement. `factory` is checked against the registry's immutable `rootIndex`, which is the
/// property that makes it the factory whose roots that registry can resolve at all.
///
/// The preflight is in the SCRIPT and not in the registry deliberately. The registry stores data and
/// asserts nothing about the semantics of what it stores; binding it to one verification registry's ABI
/// would mean a later, differently-shaped one could not be published at all. It is `public view` rather
/// than internal so the invariant is testable against real deployed contracts without broadcasting.
abstract contract PublishBase is Script {
    function _registry() internal view returns (ProtocolRegistry) {
        return ProtocolRegistry(vm.envAddress("PUBLISH_PROTOCOL_REGISTRY"));
    }

    function _discoverySet() internal view returns (ProtocolRegistry.DiscoverySet memory) {
        return ProtocolVersions.levelBDiscovery(
            vm.envAddress("PUBLISH_FACTORY"),
            vm.envAddress("PUBLISH_VERIFICATION_REGISTRY"),
            vm.envAddress("PUBLISH_SBT"),
            vm.envAddress("PUBLISH_VERIFIER"),
            vm.envAddress("PUBLISH_PROVIDER_REGISTRY")
        );
    }

    function _artifactSet() internal view returns (ProtocolRegistry.ArtifactSet memory) {
        return ProtocolVersions.levelBArtifacts(
            vm.envBytes32("PUBLISH_ZKEY_SHA256"),
            vm.envBytes32("PUBLISH_WITNESS_MOBILE_SHA256"),
            vm.envBytes32("PUBLISH_R1CS_SHA256"),
            vm.envBytes32("PUBLISH_WASM_SHA256"),
            vm.envString("PUBLISH_ARTIFACTS_URL"),
            vm.envString("PUBLISH_MIN_APP_VERSION")
        );
    }

    /// @notice Assert the proposed set against what the deployed contracts actually say. Reverts with the
    /// disagreeing member named; a passing call proves the four checkable relations hold right now.
    ///
    /// "Right now" is why BOTH phases run this, not only the propose. See
    /// [`PublishProtocolVersionsExecute`] for which single relation can drift inside the window.
    ///
    /// Deliberately NOT checked: that the bound artifacts prove against `verifier`. Pins are
    /// byte-integrity and the verifier is a VK identity, so no on-chain read can relate them — that is a
    /// governance judgement, which is why the binding is timelocked rather than validated.
    function preflight(ProtocolRegistry.DiscoverySet memory d) public view {
        IDeployedVerificationRegistry reg = IDeployedVerificationRegistry(d.verificationRegistry);
        require(reg.rootIndex() == d.factory, "factory != verificationRegistry.rootIndex()");
        require(reg.sbt() == d.sbt, "sbt != verificationRegistry.sbt()");
        require(
            reg.providerRegistry() == d.providerRegistry,
            "providerRegistry != verificationRegistry.providerRegistry()"
        );
        require(reg.zkVerifier() == d.verifier, "verifier != verificationRegistry.zkVerifier()");
    }

    /// @notice Assert that the record STAGED during the propose phase is byte-identical to the record
    /// this environment describes. Reverts if they differ.
    ///
    /// This is what lets the execute phase preflight the environment and still speak about the record
    /// that is actually going to be written: `executeDiscoverySet` writes the staged bytes and never
    /// looks at the environment, so preflighting the environment ALONE would pass for an operator who
    /// reacted to a mid-window verifier swap by editing `PUBLISH_VERIFIER` - the preflight would agree
    /// with the chain while the staged record still named the retired verifier, and the retired
    /// verifier would be published as dogtag-certified.
    ///
    /// The comparison is over the whole struct because `ProtocolVersions.levelBDiscovery` zeroes
    /// `publishedAt`/`active` and `proposeDiscoverySet` stores its calldata verbatim, so the staged and
    /// env-derived encodings are equal exactly when nothing was edited. The remedy for a disagreement
    /// is a fresh propose, which is also the remedy for the drift below.
    function requireStagedMatchesEnv(ProtocolRegistry reg, ProtocolRegistry.DiscoverySet memory d)
        public
        view
    {
        require(
            keccak256(abi.encode(reg.getPendingDiscoverySet(d.discoverySetId))) == keccak256(abi.encode(d)),
            "staged discovery set differs from this environment"
        );
    }

    /// @notice The ARTIFACT-axis counterpart. Reverts naming the disagreeing member.
    ///
    /// The soundness argument that motivates the discovery-axis check does NOT apply here: pins are
    /// byte-integrity and the verifier is a VK identity, so there is no artifact preflight that could be
    /// misled into printing a green about a record it did not describe. What applies instead is the
    /// operator's reading of the ONE combined verdict this phase prints. Without this check an operator
    /// who edited `PUBLISH_ZKEY_SHA256` between the phases would read "staged bytes match" over a
    /// publication of the OLD staged pins, and only `minAppVersion` would be visible in the closing log.
    /// Checking both axes is cheaper than printing two verdicts and leaves nothing to interpret.
    ///
    /// Members are named individually because a pin disagreement is otherwise indistinguishable from a
    /// URL or floor disagreement, and the remedies differ. The whole-struct check after them is the
    /// catch-all: `publishedAt`/`active` are stamped at execute rather than taken from the environment,
    /// so they cannot differ today, but a member added later is covered without being remembered.
    function requireStagedArtifactsMatchEnv(ProtocolRegistry reg, ProtocolRegistry.ArtifactSet memory a)
        public
        view
    {
        ProtocolRegistry.ArtifactSet memory staged = reg.getPendingArtifactSet(a.artifactSetId);
        require(staged.artifactSetId == a.artifactSetId, "staged artifactSetId differs from this environment");
        require(staged.zkeySha256 == a.zkeySha256, "staged zkeySha256 differs from this environment");
        require(
            staged.witnessMobileSha256 == a.witnessMobileSha256,
            "staged witnessMobileSha256 differs from this environment"
        );
        require(
            staged.witnessServerR1csSha256 == a.witnessServerR1csSha256,
            "staged witnessServerR1csSha256 differs from this environment"
        );
        require(
            staged.witnessServerWasmSha256 == a.witnessServerWasmSha256,
            "staged witnessServerWasmSha256 differs from this environment"
        );
        require(
            keccak256(bytes(staged.artifactBaseUrl)) == keccak256(bytes(a.artifactBaseUrl)),
            "staged artifactBaseUrl differs from this environment"
        );
        require(
            keccak256(bytes(staged.minAppVersion)) == keccak256(bytes(a.minAppVersion)),
            "staged minAppVersion differs from this environment"
        );
        require(
            keccak256(abi.encode(staged)) == keccak256(abi.encode(a)),
            "staged artifact set differs from this environment"
        );
    }
}

/// @notice Phase 1 — preflight, then propose the version on both axes plus its binding.
///   forge script .../PublishProtocolVersions.s.sol:PublishProtocolVersionsPropose \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
///
/// # FIRST ROLLOUT ONLY - never reach for this to move ONE axis
///
/// This contract stages all three writes unconditionally, so with its `PublishProtocolVersionsExecute`
/// half it sends SIX transactions. Using it for a later artifact-only rotation therefore re-proposes AND
/// re-executes the discovery set: `executeDiscoverySet` assigns `discoverySets[id]` unconditionally, so it
/// restamps `publishedAt` and re-emits `DiscoverySetPublished` for a change that moves no address -
/// rewriting the generation's on-chain provenance and destroying the previous `publishedAt` with nothing
/// recording it.
///
/// A later artifact-only rotation is `proposeArtifactSet` + `proposeArtifactBinding` and their two
/// executes, with NO `DiscoverySet` write at all. `contracts/script/PinConsentWitnessGraph.s.sol` is the
/// narrow single-axis shape to copy: it sends exactly two transactions and cannot touch the other axis.
contract PublishProtocolVersionsPropose is PublishBase {
    function run() external {
        ProtocolRegistry reg = _registry();
        ProtocolRegistry.DiscoverySet memory discoverySet = _discoverySet();
        ProtocolRegistry.ArtifactSet memory artifactSet = _artifactSet();

        console2.log("--- Phase 1: PREFLIGHT, then PROPOSE discovery set + artifact set + binding ---");
        console2.log("ProtocolRegistry", address(reg));
        console2.log("Publish timelock (seconds)", reg.PUBLISH_TIMELOCK());
        preflight(discoverySet);
        console2.log("Preflight OK: factory/sbt/providerRegistry/verifier all agree with the registry");

        vm.startBroadcast();
        reg.proposeDiscoverySet(discoverySet);
        reg.proposeArtifactSet(artifactSet);
        reg.proposeArtifactBinding(discoverySet.discoverySetId, artifactSet.artifactSetId);
        vm.stopBroadcast();

        console2.log(
            "dogtag-levelb/2 discovery-set ETA (unix)", reg.discoverySetEta(discoverySet.discoverySetId)
        );
        console2.log("artifact-set ETA (unix)", reg.artifactSetEta(artifactSet.artifactSetId));
        console2.log("binding ETA (unix)", reg.bindingEta(discoverySet.discoverySetId));
        console2.log("Next: after the timelock, run PublishProtocolVersionsExecute.");
    }
}

/// @notice Phase 2 — execute the version (only valid AFTER the timelock). Sets first, then binding.
///   forge script .../PublishProtocolVersions.s.sol:PublishProtocolVersionsExecute \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
///
/// # Why this phase re-preflights, and which single relation that catches
///
/// The preflight is a snapshot of relations that held when the record was STAGED, and the record is not
/// activated until a whole timelock later. Three of the four relations cannot move in between:
/// `providerRegistry`, `sbt` and `rootIndex` are `immutable` on `VerificationRegistryConsent`.
///
/// The single drifting relation is `verifier == verificationRegistry.zkVerifier()`, and it drifts for a
/// specific reason: `zkVerifier` is the one mutable member, swappable behind `ZK_TIMELOCK = 2 days` -
/// the SAME length as the mainnet `PUBLISH_TIMELOCK`. So a verifier swap proposed shortly before a
/// publish executes squarely inside the publish window, and without this re-check `executeDiscoverySet`
/// would write a `verifier` the registry no longer uses and publish it as dogtag-certified, with nothing
/// downstream in a position to notice. The registry itself already prefers execute-time checks for
/// exactly this reason: `executeArtifactBinding` re-validates published-and-active at execute rather
/// than at propose, so a set deprecated during the window cannot slip through.
///
/// This is defence in depth on an operator's transcription, not a substitute for reading the chain. The
/// remedy after a refusal here is a FRESH PROPOSE naming the new verifier - and therefore a fresh
/// timelock - never a bypass: the record is being refused because it no longer describes the deployment,
/// and editing the environment to agree with the chain would leave the staged bytes untouched (which is
/// what [`requireStagedMatchesEnv`] refuses, so that the preflight below really does speak about the
/// record this phase is about to activate).
///
/// It runs BEFORE `vm.startBroadcast()` so a refusal spends no gas and leaves no half-open broadcast.
///
/// # This phase needs the SAME environment phase 1 had
///
/// Phase 2 used to read only `PUBLISH_PROTOCOL_REGISTRY`. The re-preflight and the two staged-versus-env
/// checks mean it now reads every publish variable: the six `GEN2_*` addresses, the four pins, and
/// `PUBLISH_ARTIFACTS_URL` / `PUBLISH_MIN_APP_VERSION`. On mainnet that is two days after phase 1, plausibly in
/// a different shell, so load the same `.env` rather than only the registry address. A missing variable
/// reverts inside `vm.envAddress`/`vm.envBytes32` before anything is broadcast.
contract PublishProtocolVersionsExecute is PublishBase {
    function run() external {
        ProtocolRegistry reg = _registry();
        console2.log("--- Phase 2: EXECUTE sets, then binding (after timelock) ---");
        console2.log("ProtocolRegistry", address(reg));

        ProtocolRegistry.DiscoverySet memory discoverySet = _discoverySet();
        ProtocolRegistry.ArtifactSet memory artifactSet = _artifactSet();
        requireStagedMatchesEnv(reg, discoverySet);
        requireStagedArtifactsMatchEnv(reg, artifactSet);
        preflight(discoverySet);
        console2.log("Re-preflight OK: the staged record still describes the live deployment");
        console2.log("Staged bytes on BOTH axes still match this environment");

        vm.startBroadcast();
        reg.executeDiscoverySet(ProtocolVersions.levelBId());
        reg.executeArtifactSet(ProtocolVersions.levelBArtifactsId());
        reg.executeArtifactBinding(ProtocolVersions.levelBId());
        vm.stopBroadcast();

        console2.log("Published discovery sets:", reg.discoverySetCount());
        console2.log("Published artifact sets:", reg.artifactSetCount());
        console2.log("dogtag-levelb/2 active", reg.getDiscoverySet(ProtocolVersions.levelBId()).active);
        console2.log(
            "dogtag-levelb/2 minAppVersion",
            reg.getActiveArtifactSet(ProtocolVersions.levelBId()).minAppVersion
        );
    }
}
