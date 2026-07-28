// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";
import {ProtocolVersions} from "./ProtocolVersions.sol";

/// @notice Publish `witnessMobileSha256` for the ALREADY-PUBLISHED `dogtag-levelb-artifacts/1` set -
/// moving the owner-hidden witness graph from UNPINNED (`0`) to pinned, and touching nothing else.
///
/// Implements `docs/ARTIFACT_PIN_RUNBOOK.md` Step 2 as an ARTIFACT-AXIS-ONLY operation.
///
/// # Why this is not `PublishProtocolVersions.s.sol`
///
/// That script publishes the version on BOTH axes plus the binding: its Propose phase sends
/// `proposeContractSet` + `proposeArtifactSet` + `proposeArtifactBinding`, and its Execute phase sends
/// the three matching executes - SIX transactions. Pinning the graph is an artifact-axis change, so four
/// of those six re-publish a `ContractSet` and a binding that nobody asked to change. That is not
/// harmless: `executeContractSet` restamps `publishedAt` and re-emits `ContractSetPublished`, rewriting
/// the on-chain provenance of the trio for a change that moves no trio address. This script sends
/// exactly TWO transactions, both on the artifact axis.
///
/// # In-place re-publish, not a new artifact set
///
/// `executeArtifactSet` assigns `artifactSets[id] = a` unconditionally and uses `isNew` only to decide
/// whether to append to `artifactSetList`; `ArtifactSetPublished(id, isNew)` exists precisely to tell a
/// re-publish from a first publication. So re-publishing `…-artifacts/1` in place is the contract's own
/// designed path, and it is the right one here because pinning changes NO artifact's bytes - it
/// publishes an identity that was previously omitted. A `…-artifacts/2` would move `artifactSetId`,
/// which both mobile `AnchorResolver`s decode, and would require a binding update - real churn and real
/// strand-the-app risk, bought for nothing.
///
/// # The guard is what keeps this inside its authorization
///
/// Re-publishing restates ALL of the set's fields, so a stale env var here would silently rewrite a pin
/// or the base URL under cover of "pinning the graph". The script therefore reads the CURRENT on-chain
/// record and requires that every field except `witnessMobileSha256` is unchanged, and that the graph is
/// currently unpinned. Anything else reverts BEFORE broadcasting. A rotation is a different operation
/// with a different authorization; this script cannot perform one.
///
/// # Env (all MANDATORY - no defaults, matching `PublishProtocolVersions.s.sol`)
///
///   `PROTOCOL_REGISTRY`, `CONSENT_ZKEY_SHA256`, `CONSENT_WITNESS_MOBILE_SHA256`,
///   `CONSENT_R1CS_SHA256`, `CONSENT_WASM_SHA256`, `DOGTAG_ARTIFACTS_URL`
///
/// Broadcast with the PUBLISHER_ROLE key (governance):
///   forge script script/PinConsentWitnessGraph.s.sol:PinConsentWitnessGraph \
///     --rpc-url $ROAX_RPC --broadcast --legacy --private-key $GOVERNANCE_PRIVATE_KEY
contract PinConsentWitnessGraph is Script {
    function run() external {
        ProtocolRegistry reg = ProtocolRegistry(vm.envAddress("PROTOCOL_REGISTRY"));
        bytes32 id = ProtocolVersions.levelBArtifactsId();

        ProtocolRegistry.ArtifactSet memory next = ProtocolVersions.levelBArtifacts(
            vm.envBytes32("CONSENT_ZKEY_SHA256"),
            vm.envBytes32("CONSENT_WITNESS_MOBILE_SHA256"),
            vm.envBytes32("CONSENT_R1CS_SHA256"),
            vm.envBytes32("CONSENT_WASM_SHA256"),
            vm.envString("DOGTAG_ARTIFACTS_URL")
        );

        requireOnlyTheGraphPinMoves(reg, id, next);

        uint256 timelock = reg.PUBLISH_TIMELOCK();
        uint256 pendingEta = reg.artifactSetEta(id);
        console2.log("--- Pin consent witness graph (artifact axis only) ---");
        console2.log("ProtocolRegistry ", address(reg));
        console2.log("Publish timelock ", timelock, "seconds");

        if (pendingEta == 0) {
            vm.startBroadcast();
            reg.proposeArtifactSet(next);
            // A zero timelock (the ROAX testnet opt-in) lets the execute follow immediately. Under a
            // REAL delay `executeArtifactSet` would revert "timelock", so the propose is left staged
            // and the operator re-runs this script after the printed ETA to finish it.
            if (timelock == 0) {
                reg.executeArtifactSet(id);
            }
            vm.stopBroadcast();
        } else if (block.timestamp >= pendingEta) {
            // A proposal from an earlier run has ripened. Re-proposing here would reset
            // `artifactSetEta` to `block.timestamp + PUBLISH_TIMELOCK` and silently restart the whole
            // delay, so the second run of a two-phase operation must EXECUTE, never propose again.
            console2.log("Pending proposal has ripened (ETA elapsed) - executing it, not re-proposing.");
            vm.startBroadcast();
            reg.executeArtifactSet(id);
            vm.stopBroadcast();
        } else {
            console2.log("A proposal is already staged and its timelock has NOT elapsed. ETA (unix):", pendingEta);
            console2.log("Re-run this script at/after that ETA to execute it. Nothing was broadcast.");
            revert("proposal pending - re-run after the printed ETA to execute");
        }

        if (reg.artifactSetEta(id) != 0) {
            console2.log("PROPOSED ONLY - execute is still OUTSTANDING. ETA (unix):", reg.artifactSetEta(id));
            console2.log("The graph is NOT pinned on chain until executeArtifactSet runs.");
            console2.log("Re-run this script at/after that ETA; it will execute rather than re-propose.");
            return;
        }

        // Read-back. `forge script` simulates `run()` in full and only broadcasts once simulation
        // succeeds, so this really does gate the transactions rather than merely report on them - which
        // is what makes executing an opaque `_pendingArtifactSet` from an earlier run safe: a proposal
        // staging anything other than `next` is caught here, before anything reaches the chain.
        ProtocolRegistry.ArtifactSet memory got = reg.getArtifactSet(id);
        console2.log("PUBLISHED. witnessMobileSha256 is now:");
        console2.logBytes32(got.witnessMobileSha256);
        require(got.witnessMobileSha256 == next.witnessMobileSha256, "read-back mismatch: graph pin");
        require(got.zkeySha256 == next.zkeySha256, "read-back mismatch: zkey pin");
        require(got.witnessServerR1csSha256 == next.witnessServerR1csSha256, "read-back mismatch: r1cs pin");
        require(got.witnessServerWasmSha256 == next.witnessServerWasmSha256, "read-back mismatch: wasm pin");
        require(
            keccak256(bytes(got.artifactBaseUrl)) == keccak256(bytes(next.artifactBaseUrl)),
            "read-back mismatch: artifactBaseUrl"
        );
        require(
            keccak256(bytes(got.minAppVersion)) == keccak256(bytes(next.minAppVersion)),
            "read-back mismatch: minAppVersion"
        );
        require(got.active, "set must remain active");
    }

    /// @dev Refuse to broadcast unless this is exactly "pin the graph on an otherwise-identical set".
    /// `public` so `contracts/test/PinConsentWitnessGraph.t.sol` can pin every revert arm directly,
    /// without broadcasting - the same shape as `DeployProtocolRegistry.validatePublishTimelock`.
    function requireOnlyTheGraphPinMoves(
        ProtocolRegistry reg,
        bytes32 id,
        ProtocolRegistry.ArtifactSet memory next
    ) public view {
        // Publishedness is probed through the auto-getter, which returns a zeroed record for an unknown
        // id. `getArtifactSet` reverts "unknown artifact set" instead, so asking it first would make
        // this script's own refusal unreachable and report a registry error for an operator mistake.
        (bytes32 publishedId,,,,,,,,) = reg.artifactSets(id);
        require(publishedId == id, "artifact set is not published - this script only re-publishes");

        ProtocolRegistry.ArtifactSet memory cur = reg.getArtifactSet(id);
        require(cur.active, "artifact set is deprecated - re-activating is a separate decision");
        require(cur.witnessMobileSha256 == bytes32(0), "graph already pinned - a change here is a ROTATION");
        require(next.witnessMobileSha256 != bytes32(0), "refusing to publish an empty graph pin");

        // Everything else must be a byte-for-byte restatement of what is already published.
        require(next.zkeySha256 == cur.zkeySha256, "zkey pin would change");
        require(next.witnessServerR1csSha256 == cur.witnessServerR1csSha256, "r1cs pin would change");
        require(next.witnessServerWasmSha256 == cur.witnessServerWasmSha256, "wasm pin would change");
        require(
            keccak256(bytes(next.artifactBaseUrl)) == keccak256(bytes(cur.artifactBaseUrl)),
            "artifactBaseUrl would change"
        );
        require(
            keccak256(bytes(next.minAppVersion)) == keccak256(bytes(cur.minAppVersion)),
            "minAppVersion would change"
        );
    }
}
