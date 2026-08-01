// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Script, console} from "forge-std/Script.sol";
import {CutoverSequence} from "./CutoverSequence.sol";

/// @title RehearseCutover — broadcasts the on-chain cutover so the transaction list can be captured.
///
/// @notice REHEARSAL ONLY. This is pointed at a forked `anvil`, never at ROAX. It exists to produce
/// `broadcast/RehearseCutover.s.sol/135/run-latest.json`, which is the exact ordered transaction
/// list the live cutover replays — derived from what the DEPLOYED bytecode permits rather than from
/// what the source suggests.
///
/// Run it through `scripts/rehearse-cutover.sh`, which starts the fork, records anvil's PID, and
/// kills that PID and nothing else. Do not invoke it against a live RPC.
///
/// # WHY THIS IS A SEPARATE ARTEFACT FROM THE ASSERTIONS
///
/// The transaction list says what will be SENT. The fork test
/// (`rehearsal/CutoverRehearsal.t.sol`) says what the result BEHAVES like. Neither substitutes for
/// the other, and they are only comparable because both call `CutoverSequence` — see the note there
/// on why that sharing is load-bearing.
///
/// # WHAT IS AND IS NOT IN THE LIST
///
/// In: every on-chain step from C-1 to C-8 that this repo can execute today.
///
/// Deliberately absent, and each for a stated reason rather than an oversight:
///
///   - The rest of C-2 (register providers, attach clones, mirror issuance grants) is blocked on
///     KYC inputs that do not exist, and for the five existing generation-1 clones is not executable
///     against this core at all. See `CutoverSequence.c2_recordFactoryGeneration`.
///   - **C-6 (migrate the `VERIFY:` relayer capabilities)** is one transaction per
///     `(purpose, relayer)`, and which relayer serves which provider for which purpose is plan §4
///     item 5 — a KYC fact, not a chain fact. Roughly 7 of the 33 live grants are on this axis.
///   - C-9 and C-10 are client repointing and an app release: no transactions.
///   - C-10b (SBT `ISSUER_ROLE` for the generation-2 signers), C-11 (issuance grants) and C-12
///     (the generation-1 freeze) are one transaction PER SIGNER, from the same reconciliation.
///
/// Every one of those is a per-signer or per-relayer list whose membership is the KYC work, so
/// publishing it with invented addresses would be worse than publishing none: it would read as
/// ready. Each one's MECHANICS are nonetheless exercised against the fork, so absence from the list
/// is not absence of evidence — C-6 is assertion 5 (applied and withheld on one credential), C-10b
/// is the SBT `grantRole` in the fork test's `setUp`, C-11 is `setIssuanceCapability` in the
/// generation-2 anchoring helper, and C-12 is assertion 7 (a real `delistFor`, then new issuance
/// refused while every historical root still verifies). What is not claimed is knowledge of whose
/// addresses go in them.
///
/// `scripts/render-cutover-txlist.py` prints this exclusion list beside the transactions, so the
/// rendered list cannot be read as the whole cutover.
contract RehearseCutover is Script {
    /// @dev The live generation-1 set. Read from `contracts/deployments/roax.json` at run time
    /// rather than transcribed, because a transcribed address is a copy that goes stale silently —
    /// this repo has a standing rule about exactly that.
    function _generation1() internal view returns (CutoverSequence.Generation1 memory gen1) {
        string memory ledger = vm.readFile("deployments/roax.json");
        gen1.factory = vm.parseJsonAddress(ledger, ".DogTagIssuerFactory");
        gen1.issuerRegistry = vm.parseJsonAddress(ledger, ".IssuerRegistry");
        gen1.sbt = vm.parseJsonAddress(ledger, ".DogTagSBTConsent");
        gen1.verifier = vm.parseJsonAddress(ledger, ".Groth16VerifierConsent");
        gen1.governance = vm.parseJsonAddress(ledger, ".admin");
    }

    function run() external {
        // WHAT THESE TWO GUARDS ACTUALLY ENFORCE — read them together, because the first one alone
        // does NOT keep this script off a live endpoint and must never be described as if it did.
        //
        // `block.chainid == 135` excludes OTHER CHAINS only. 135 is live ROAX's own chain id, so a
        // chain-id check can never distinguish a fork of ROAX from ROAX itself: it passes on exactly
        // the endpoint it would need to refuse. On its own it is worse than no guard, because it
        // reads as protective in front of an operation that cannot be undone —
        // `CutoverSequence.c4b_appendGeneration` is irreversible, the router has no removal path,
        // and `c2_recordFactoryGeneration` can only ever be deprecated, never repointed.
        //
        // `block.number == pinnedBlock` is the guard that carries the safety. A live chain's head is
        // already far past the pinned block and only ever advances, so live can never satisfy it,
        // while a fork pinned at that block always does. The pinned block is READ FROM THE FIXTURE
        // rather than restated here, so there is one source and it cannot drift from the block the
        // assertions and the historical-root inventory were derived at.
        //
        // The equality is STRICT deliberately, so it also fires on a local fork that has had a block
        // mined on it since it was created. That is a second, entirely benign cause, so the refusal
        // NAMES BOTH rather than asserting the alarming one: telling an operator on their own anvil
        // that they pointed an irreversible broadcast at live ROAX is a wrong verdict inside the one
        // guard whose whole value is being trustworthy. Relaxing it to a range would weaken the
        // safety property to buy a better message, which is the wrong trade.
        //
        // `scripts/rehearse-cutover.sh` additionally only ever passes a local fork URL, but that is
        // defence in depth: a script invoked by hand must protect itself.
        require(block.chainid == 135, "rehearsal expects the ROAX chain id (135)");

        uint256 pinnedBlock =
            vm.parseJsonUint(vm.readFile("rehearsal/fixtures/historical-roots.json"), ".pinnedBlock");
        require(
            block.number == pinnedBlock,
            "REFUSING: head is not the pinned rehearsal block. Either this is a LIVE endpoint, or a "
            "block has been mined on your fork since it was created - re-fork at the pinned block "
            "(scripts/rehearse-cutover.sh does that for you)."
        );

        CutoverSequence.Generation1 memory gen1 = _generation1();

        // The rehearsal deliberately uses the FLOOR (1 hour) rather than the production default
        // (2 days), so a rehearsal can be walked end to end in one sitting. The floor is enforced in
        // the constructor, so zero is unrepresentable either way.
        //
        // The live ROAX C-8 also took the floor, with the testnet opt-in stated aloud - the reasoning
        // is in `_s14_cutover` in deployments/roax.json, and the value is IMMUTABLE there. So do not
        // read this line as saying the live registry matches the production default.
        // MAINNET MUST USE 2 DAYS: `DeployProtocolRegistryV2.s.sol` enforces exactly that unless a
        // testnet deploy is stated.
        uint256 publishTimelock = vm.envOr("REHEARSAL_PUBLISH_TIMELOCK", uint256(1 hours));

        console.log("generation-1 factory        ", gen1.factory);
        console.log("generation-1 issuerRegistry ", gen1.issuerRegistry);
        console.log("shared SBT     (REUSED)     ", gen1.sbt);
        console.log("shared verifier(REUSED)     ", gen1.verifier);
        console.log("governance                  ", gen1.governance);

        vm.startBroadcast();
        CutoverSequence.Deployment memory d = CutoverSequence.runOnchainSequence(gen1, publishTimelock);
        vm.stopBroadcast();

        console.log("");
        console.log("C-1  ProviderRegistry        ", address(d.core));
        console.log("C-3a DogTagIssuerV2 impl     ", address(d.issuerImpl));
        console.log("C-4  CloneProvenanceRouter   ", address(d.router));
        console.log("C-3b DogTagIssuerFactoryV2   ", address(d.factoryV2));
        console.log("C-5  VerificationRegistry V2 ", address(d.registryV2));
        console.log("C-8  ProtocolRegistryV2      ", address(d.discoveryV2));
    }
}
