// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {ProtocolRegistryV2} from "../src/ProtocolRegistryV2.sol";
import {ProtocolVersionsV2} from "./ProtocolVersionsV2.sol";

/// @notice The four immutable bindings a deployed `VerificationRegistryConsent` exposes, typed as plain
/// addresses so this script can compare them without importing the registry's own interface types. The
/// selectors are the registry's public auto-getters, so this interface is ABI-identical to it.
interface IDeployedVerificationRegistry {
    function issuerRegistry() external view returns (address);
    function sbt() external view returns (address);
    function rootIndex() external view returns (address);
    function zkVerifier() external view returns (address);
}

/// @notice The router membership assertion — `CloneProvenanceRouter.isGeneration` is a public mapping.
interface IRootIndexGenerations {
    function isGeneration(address factory) external view returns (bool);
}

/// @notice Publishes the generation-2 `dogtag-levelb/2` version to `ProtocolRegistryV2`. The record DATA
/// is in `ProtocolVersionsV2` — the single source of truth this and the registry tests share.
///
/// # Two axes, so the version publishes THREE things
///
/// A [`ProtocolRegistryV2.DiscoverySet`], a [`ProtocolRegistryV2.ArtifactSet`], and the BINDING between
/// them. All three writes are timelocked but their timelocks run CONCURRENTLY: phase 1 proposes all
/// three, so this is still a TWO-phase rollout.
///
///   1. `PublishProtocolVersionsV2Propose` — preflights, then stages all three. Starts the registry's
///      immutable `PUBLISH_TIMELOCK` on each at once.
///   2. `PublishProtocolVersionsV2Execute` — run AFTER the timelock elapses. Sets FIRST, then the
///      binding (`executeArtifactBinding` requires both sides published AND active).
///
/// Both are `PUBLISHER_ROLE`-gated. Every input is a mandatory env var under the `GEN2_` namespace:
/// `GEN2_PROTOCOL_REGISTRY`, `GEN2_FACTORY`, `GEN2_VERIFICATION_REGISTRY`, `GEN2_SBT`, `GEN2_VERIFIER`,
/// `GEN2_PROVIDER_REGISTRY`, `GEN2_ROOT_INDEX`, `GEN2_ZKEY_SHA256`, `GEN2_WITNESS_MOBILE_SHA256`,
/// `GEN2_R1CS_SHA256`, `GEN2_WASM_SHA256`, `GEN2_ARTIFACTS_URL`, `GEN2_MIN_APP_VERSION`. There are no
/// stale-network fallbacks and no default app floor.
///
/// # Why the `GEN2_` namespace rather than generation 1's variable names
///
/// Two reasons, and the first is a real hazard rather than tidiness. Generation 1's publish script reads
/// `ROOT_INDEX`, and on that generation the root index IS the factory — so an operator with a
/// generation-1 `.env` loaded would publish generation 2 with a FACTORY address in its root-index slot,
/// which is precisely the mistake [`PublishV2Base.preflight`] exists to catch. Sharing a name whose
/// generation-1 value is wrong here makes that mistake likelier, not less likely. The same applies to
/// `SBT_CONSENT_ADDR` and `CONSENT_VERIFIER`: generation 2 reuses those contracts, but "reuses" is a fact
/// to state and have checked, not one to inherit silently from an environment.
///
/// Second, `vm.setEnv` mutates the PROCESS environment and `forge test` runs test contracts in parallel,
/// so two suites driving two scripts through the same variable names interfere: the generation-1 deploy
/// test sets `ROOT_INDEX` to its own factory, and this script would read it mid-test. That produced a
/// genuine flake (the preflight failing on `rootIndex` in roughly half of full-suite runs) which the
/// namespace removes by construction rather than by timing.
///
/// # Why there is a coherence PREFLIGHT, and why it lives here
///
/// A discovery record is a set of addresses an operator transcribes, and a mis-transcribed one is the
/// unrecoverable failure this whole generation is being deployed to avoid: a consumer that follows a
/// wrong `rootIndex` resolves no historical root, and one that follows a wrong `providerRegistry` asks
/// the wrong contract whether an issuer was authorized. Nothing about either mistake looks wrong at
/// publish time.
///
/// Three of the six addresses are checkable against the chain, because `VerificationRegistryConsent`
/// pins them in immutable slots and exposes each as a getter, and a fourth (`verifier`) against its
/// current `zkVerifier`. [`preflight`] reads them back and refuses to propose on any disagreement. The
/// `factory` is checked differently: it is not reachable from the registry in generation 2, so it is
/// asserted against the router's own generation membership, which is the property that makes it the
/// factory whose roots that root index can resolve.
///
/// The preflight is in the SCRIPT and not in the registry deliberately, following
/// `DeployIssuerDomainRegistry.s.sol`, which preflights `factory.registry()` the same way. The registry
/// stores data and asserts nothing about the semantics of what it stores; binding it to one registry's
/// ABI would mean a later generation with a differently-shaped verification registry could not be
/// published at all. It is `public view` rather than internal so the invariant is testable against real
/// deployed contracts without broadcasting.
abstract contract PublishV2Base is Script {
    function _registry() internal view returns (ProtocolRegistryV2) {
        return ProtocolRegistryV2(vm.envAddress("GEN2_PROTOCOL_REGISTRY"));
    }

    function _discoverySet() internal view returns (ProtocolRegistryV2.DiscoverySet memory) {
        return ProtocolVersionsV2.levelBV2Discovery(
            vm.envAddress("GEN2_FACTORY"),
            vm.envAddress("GEN2_VERIFICATION_REGISTRY"),
            vm.envAddress("GEN2_SBT"),
            vm.envAddress("GEN2_VERIFIER"),
            vm.envAddress("GEN2_PROVIDER_REGISTRY"),
            vm.envAddress("GEN2_ROOT_INDEX")
        );
    }

    function _artifactSet() internal view returns (ProtocolRegistryV2.ArtifactSet memory) {
        return ProtocolVersionsV2.levelBArtifacts(
            vm.envBytes32("GEN2_ZKEY_SHA256"),
            vm.envBytes32("GEN2_WITNESS_MOBILE_SHA256"),
            vm.envBytes32("GEN2_R1CS_SHA256"),
            vm.envBytes32("GEN2_WASM_SHA256"),
            vm.envString("GEN2_ARTIFACTS_URL"),
            vm.envString("GEN2_MIN_APP_VERSION")
        );
    }

    /// @notice Assert the proposed set against what the deployed contracts actually say. Reverts with the
    /// disagreeing member named; a passing call proves the five checkable relations hold right now.
    ///
    /// "Right now" is why BOTH phases run this, not only the propose. See
    /// [`PublishProtocolVersionsV2Execute`] for which single relation can drift inside the window.
    ///
    /// Deliberately NOT checked: that the bound artifacts prove against `verifier`. Pins are
    /// byte-integrity and the verifier is a VK identity, so no on-chain read can relate them — that is a
    /// governance judgement, which is why the binding is timelocked rather than validated.
    function preflight(ProtocolRegistryV2.DiscoverySet memory d) public view {
        IDeployedVerificationRegistry reg = IDeployedVerificationRegistry(d.verificationRegistry);
        require(reg.rootIndex() == d.rootIndex, "rootIndex != verificationRegistry.rootIndex()");
        require(reg.sbt() == d.sbt, "sbt != verificationRegistry.sbt()");
        require(
            reg.issuerRegistry() == d.providerRegistry,
            "providerRegistry != verificationRegistry.issuerRegistry()"
        );
        require(reg.zkVerifier() == d.verifier, "verifier != verificationRegistry.zkVerifier()");
        require(
            IRootIndexGenerations(d.rootIndex).isGeneration(d.factory),
            "factory is not a generation of rootIndex"
        );
    }

    /// @notice Assert that the record STAGED during the propose phase is byte-identical to the record
    /// this environment describes. Reverts if they differ.
    ///
    /// This is what lets the execute phase preflight the environment and still speak about the record
    /// that is actually going to be written: `executeDiscoverySet` writes the staged bytes and never
    /// looks at the environment, so preflighting the environment ALONE would pass for an operator who
    /// reacted to a mid-window verifier swap by editing `GEN2_VERIFIER` - the preflight would agree
    /// with the chain while the staged record still named the retired verifier, and the retired
    /// verifier would be published as dogtag-certified.
    ///
    /// The comparison is over the whole struct because `ProtocolVersionsV2.levelBV2Discovery` zeroes
    /// `publishedAt`/`active` and `proposeDiscoverySet` stores its calldata verbatim, so the staged and
    /// env-derived encodings are equal exactly when nothing was edited. The remedy for a disagreement
    /// is a fresh propose, which is also the remedy for the drift below.
    function requireStagedMatchesEnv(ProtocolRegistryV2 reg, ProtocolRegistryV2.DiscoverySet memory d)
        public
        view
    {
        require(
            keccak256(abi.encode(reg.getPendingDiscoverySet(d.discoverySetId))) == keccak256(abi.encode(d)),
            "staged discovery set differs from this environment"
        );
    }
}

/// @notice Phase 1 — preflight, then propose the version on both axes plus its binding.
///   forge script .../PublishProtocolVersionsV2.s.sol:PublishProtocolVersionsV2Propose \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
///
/// # FIRST ROLLOUT ONLY - never reach for this to move ONE axis
///
/// This contract stages all three writes unconditionally, so with its `PublishProtocolVersionsV2Execute`
/// half it sends SIX transactions. Using it for a later artifact-only rotation therefore re-proposes AND
/// re-executes the discovery set: `executeDiscoverySet` assigns `discoverySets[id]` unconditionally, so it
/// restamps `publishedAt` and re-emits `DiscoverySetPublished` for a change that moves no address -
/// rewriting the generation's on-chain provenance and destroying the previous `publishedAt` with nothing
/// recording it. The same trap is recorded for generation 1's script in AGENTS.md.
///
/// A later artifact-only rotation is `proposeArtifactSet` + `proposeArtifactBinding` and their two
/// executes, with NO `DiscoverySet` write at all. `contracts/script/PinConsentWitnessGraph.s.sol` is the
/// narrow single-axis shape to copy: it sends exactly two transactions and cannot touch the other axis.
contract PublishProtocolVersionsV2Propose is PublishV2Base {
    function run() external {
        ProtocolRegistryV2 reg = _registry();
        ProtocolRegistryV2.DiscoverySet memory discoverySet = _discoverySet();
        ProtocolRegistryV2.ArtifactSet memory artifactSet = _artifactSet();

        console2.log("--- Phase 1: PREFLIGHT, then PROPOSE discovery set + artifact set + binding ---");
        console2.log("ProtocolRegistryV2", address(reg));
        console2.log("Publish timelock (seconds)", reg.PUBLISH_TIMELOCK());
        preflight(discoverySet);
        console2.log("Preflight OK: rootIndex/sbt/providerRegistry/verifier agree, factory is a generation");

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
        console2.log("Next: after the timelock, run PublishProtocolVersionsV2Execute.");
    }
}

/// @notice Phase 2 — execute the version (only valid AFTER the timelock). Sets first, then binding.
///   forge script .../PublishProtocolVersionsV2.s.sol:PublishProtocolVersionsV2Execute \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
///
/// # Why this phase re-preflights, and which single relation that catches
///
/// The preflight is a snapshot of relations that held when the record was STAGED, and the record is not
/// activated until a whole timelock later. Four of the five relations cannot move in between:
/// `issuerRegistry`, `sbt` and `rootIndex` are `immutable` on `VerificationRegistryConsent`, and the
/// router's `isGeneration` is append-only monotone, so a factory that was a generation stays one.
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
contract PublishProtocolVersionsV2Execute is PublishV2Base {
    function run() external {
        ProtocolRegistryV2 reg = _registry();
        console2.log("--- Phase 2: EXECUTE sets, then binding (after timelock) ---");
        console2.log("ProtocolRegistryV2", address(reg));

        ProtocolRegistryV2.DiscoverySet memory discoverySet = _discoverySet();
        requireStagedMatchesEnv(reg, discoverySet);
        preflight(discoverySet);
        console2.log("Re-preflight OK: the staged record still describes the live deployment");

        vm.startBroadcast();
        reg.executeDiscoverySet(ProtocolVersionsV2.levelBV2Id());
        reg.executeArtifactSet(ProtocolVersionsV2.levelBArtifactsId());
        reg.executeArtifactBinding(ProtocolVersionsV2.levelBV2Id());
        vm.stopBroadcast();

        console2.log("Published discovery sets:", reg.discoverySetCount());
        console2.log("Published artifact sets:", reg.artifactSetCount());
        console2.log("dogtag-levelb/2 active", reg.getDiscoverySet(ProtocolVersionsV2.levelBV2Id()).active);
        console2.log(
            "dogtag-levelb/2 minAppVersion",
            reg.getActiveArtifactSet(ProtocolVersionsV2.levelBV2Id()).minAppVersion
        );
    }
}
