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

        _requireOnlyTheGraphPinMoves(reg, id, next);

        uint256 timelock = reg.PUBLISH_TIMELOCK();
        console2.log("--- Pin consent witness graph (artifact axis only) ---");
        console2.log("ProtocolRegistry ", address(reg));
        console2.log("Publish timelock ", timelock, "seconds");

        vm.startBroadcast();
        reg.proposeArtifactSet(next);
        // A zero timelock (the ROAX testnet opt-in) lets the execute follow immediately. Under a REAL
        // delay `executeArtifactSet` would revert "timelock", so the propose is left staged and the
        // operator must run the execute after the printed ETA - the pin is NOT live until they do.
        if (timelock == 0) {
            reg.executeArtifactSet(id);
        }
        vm.stopBroadcast();

        if (timelock == 0) {
            ProtocolRegistry.ArtifactSet memory got = reg.getArtifactSet(id);
            console2.log("PUBLISHED. witnessMobileSha256 is now:");
            console2.logBytes32(got.witnessMobileSha256);
            require(got.witnessMobileSha256 == next.witnessMobileSha256, "read-back mismatch");
            require(got.active, "set must remain active");
        } else {
            console2.log("PROPOSED ONLY - execute is still OUTSTANDING. ETA (unix):", reg.artifactSetEta(id));
            console2.log("The graph is NOT pinned on chain until executeArtifactSet runs.");
        }
    }

    /// @dev Refuse to broadcast unless this is exactly "pin the graph on an otherwise-identical set".
    /// Split out so the guard is directly testable without broadcasting.
    function _requireOnlyTheGraphPinMoves(
        ProtocolRegistry reg,
        bytes32 id,
        ProtocolRegistry.ArtifactSet memory next
    ) internal view {
        ProtocolRegistry.ArtifactSet memory cur = reg.getArtifactSet(id);
        require(cur.artifactSetId == id, "artifact set is not published - this script only re-publishes");
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
