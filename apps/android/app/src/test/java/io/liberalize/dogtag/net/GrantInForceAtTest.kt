package io.liberalize.dogtag.net

import io.liberalize.dogtag.net.RoaxRpc.GrantAtIssuance
import io.liberalize.dogtag.net.RoaxRpc.GrantEvent
import io.liberalize.dogtag.net.RoaxRpc.LogPoint
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * DELISTING IS FORWARD-ONLY, at the fold the mobile issuer-whitelist pillar rests on.
 *
 * `DogTagIssuer.sol:82` states the rule in the contract's own source and `adminRevoke` is the
 * retroactive lever, so a credential anchored while its signer held the grant stays genuine when that
 * grant is later withdrawn - an ordinary key rotation, a retirement, a lapsed practice licence. Before
 * this rule moved, the pillar read `IssuerRegistry.isWhitelistedFor`, a CURRENT-state getter, so any
 * such withdrawal retroactively rendered every credential that signer had ever issued a forgery.
 *
 * `RoaxRpc.grantInForceAt` is a pure mirror of Rust `dogtag_standard::verify::grant_in_force_at`,
 * TS `grantInForceAt` and Swift `Net.grantInForceAt`. It is the one part of the mobile pillar that can
 * be exercised without a chain, so it is where the ordering rule is pinned; the reads around it need a
 * live RPC and are covered by the backends' and the web suites' equivalents.
 */
class GrantInForceAtTest {

    private val anchored = LogPoint(blockNumber = 200, logIndex = 3)
    private fun granted(block: Long) = GrantEvent(LogPoint(block, 0), granted = true)
    private fun delisted(block: Long) = GrantEvent(LogPoint(block, 0), granted = false)

    @Test
    fun aSignerDelistedAfterTheAnchoringWasStillAuthorisedWhenItActed() {
        assertEquals(
            GrantAtIssuance.Authorized,
            RoaxRpc.grantInForceAt(listOf(granted(100), delisted(700)), anchored),
        )
    }

    @Test
    fun aSignerDelistedBeforeTheAnchoringWasNot() {
        assertEquals(
            GrantAtIssuance.NotAuthorized,
            RoaxRpc.grantInForceAt(listOf(granted(100), delisted(199)), anchored),
        )
    }

    /** The mirror of the forward-only rule: a later grant cannot authorise an earlier anchoring. */
    @Test
    fun aGrantIssuedAfterTheAnchoringDoesNotAuthoriseItRetroactively() {
        assertEquals(
            GrantAtIssuance.NotAuthorized,
            RoaxRpc.grantInForceAt(listOf(granted(201)), anchored),
        )
    }

    /**
     * An empty history is an ANSWER - the registry recorded no grant - not an absence of one. The
     * could-not-read case never reaches this function; the caller returns `Undetermined` for it.
     */
    @Test
    fun anEmptyHistoryIsADefiniteRefusalNotAnUndeterminedOne() {
        assertEquals(GrantAtIssuance.NotAuthorized, RoaxRpc.grantInForceAt(emptyList(), anchored))
    }

    /**
     * `logIndex` is block-scoped and therefore comparable ACROSS contracts within one block, which is
     * the only reason a registry grant and a clone's issuance landing in the same block can be
     * sequenced at all. Inclusive at the anchoring point.
     */
    @Test
    fun grantsAndAnchoringsInOneBlockAreSequencedByLogIndex() {
        fun at(logIndex: Long, granted: Boolean) =
            GrantEvent(LogPoint(anchored.blockNumber, logIndex), granted)

        assertEquals(
            GrantAtIssuance.Authorized,
            RoaxRpc.grantInForceAt(listOf(at(anchored.logIndex, true)), anchored),
        )
        assertEquals(
            GrantAtIssuance.Authorized,
            RoaxRpc.grantInForceAt(listOf(at(0, true), at(anchored.logIndex + 1, false)), anchored),
        )
        assertEquals(
            GrantAtIssuance.NotAuthorized,
            RoaxRpc.grantInForceAt(listOf(at(0, true), at(anchored.logIndex - 1, false)), anchored),
        )
    }

    /** The fold takes the LAST event at or before the anchoring, whatever order it arrives in. */
    @Test
    fun theAnswerIsTheLastEventAtOrBeforeTheAnchoringRegardlessOfInputOrder() {
        val events = listOf(delisted(199), granted(100), granted(150))
        assertEquals(GrantAtIssuance.NotAuthorized, RoaxRpc.grantInForceAt(events, anchored))
        assertEquals(GrantAtIssuance.NotAuthorized, RoaxRpc.grantInForceAt(events.reversed(), anchored))
    }

    // ---- the generation guard on the empty-history refusal --------------------------------------
    //
    // The fold above answers a definite `NotAuthorized` for an empty history, which is right for a
    // generation-1 `IssuerRegistry` and WRONG for its successor: `Whitelisted(bytes32 indexed
    // recordType, address indexed signer)` puts the record-type key in `topic1`, and
    // `ProviderRegistry` records `IssuanceCapabilitySet(service, signer, allowed)` instead, so that
    // filter matches nothing there and every genuine generation-2 credential arrives empty.
    //
    // Mirrors Swift `GrantAtIssuanceTests` case for case.

    @Test
    fun onlyAnExecutionRevertIdentifiesGenerationOne() {
        assertEquals(
            RoaxRpc.AuthorityGeneration.Legacy,
            RoaxRpc.generationFromProbe(RoaxRpc.CallResult.Err("execution reverted", answered = true)),
        )
        // The typed discriminator: geth's execution-reverted code, confirmed against ROAX with the
        // exact production case (`isRecognizedIssuer` on the deployed generation-1 IssuerRegistry).
        assertEquals(true, RoaxRpc.isExecutionRevert(3, "execution reverted"))
        // …and the message arm, for a client that reports the same revert under another code.
        assertEquals(true, RoaxRpc.isExecutionRevert(-32000, "execution reverted"))
    }

    /**
     * A NODE-LEVEL ERROR IS NOT A CONTRACT ANSWER. The first cut keyed `answered` on "a 200 carrying
     * a JSON-RPC error member", which a rate limit satisfies - so one would have left an empty grant
     * history standing as a definite refusal, i.e. a forgery verdict against a genuine credential
     * produced by a call the contract never ran.
     *
     * Mirrors Swift `test_aNodeErrorThatIsNotARevertIsNotAContractAnswer` and the Rust
     * `a_node_error_that_is_not_a_revert_is_undetermined_never_generation_one`.
     */
    @Test
    fun aNodeErrorThatIsNotARevertIsNotAContractAnswer() {
        val nodeErrors = listOf(
            -32005 to "limit exceeded",
            -32603 to "internal error",
            -32601 to "the method does not exist/is not available",
            -32002 to "resource unavailable",
        )
        for ((code, message) in nodeErrors) {
            assertEquals(
                "$code is the node speaking about itself, not the contract executing anything",
                false,
                RoaxRpc.isExecutionRevert(code, message),
            )
            assertEquals(
                "$code must not license the generation-1 conclusion",
                RoaxRpc.AuthorityGeneration.Undetermined,
                RoaxRpc.generationFromProbe(
                    RoaxRpc.CallResult.Err(message, answered = RoaxRpc.isExecutionRevert(code, message)),
                ),
            )
        }
    }

    @Test
    fun aProbeThatAnswersIdentifiesTheSuccessor() {
        assertEquals(
            RoaxRpc.AuthorityGeneration.Successor,
            RoaxRpc.generationFromProbe(RoaxRpc.CallResult.Ok("0x01")),
        )
    }

    /**
     * A probe that never arrived establishes NOTHING. Reading it as generation 1 hands the empty
     * history back to the fold, which correctly refuses it - so one timeout becomes a forgery verdict
     * against a genuine credential. `answered` defaults to false precisely because that is the safe
     * reading.
     */
    @Test
    fun aProbeThatCouldNotBeDeliveredIsUndeterminedNeverGenerationOne() {
        assertEquals(
            RoaxRpc.AuthorityGeneration.Undetermined,
            RoaxRpc.generationFromProbe(RoaxRpc.CallResult.Err("rpc 503")),
        )
        assertEquals(
            RoaxRpc.AuthorityGeneration.Undetermined,
            RoaxRpc.generationFromProbe(RoaxRpc.CallResult.Err("timeout", answered = false)),
        )
    }

    @Test
    fun anEmptyHistoryIsARefusalOnlyWhenTheAuthorityAnsweredThatItIsGenerationOne() {
        assertEquals(
            GrantAtIssuance.NotAuthorized,
            RoaxRpc.grantAtIssuance(emptyList(), anchored, RoaxRpc.AuthorityGeneration.Legacy),
        )
        assertEquals(
            GrantAtIssuance.Undetermined,
            RoaxRpc.grantAtIssuance(emptyList(), anchored, RoaxRpc.AuthorityGeneration.Successor),
        )
        assertEquals(
            GrantAtIssuance.Undetermined,
            RoaxRpc.grantAtIssuance(emptyList(), anchored, RoaxRpc.AuthorityGeneration.Undetermined),
        )
    }

    /**
     * The guard is scoped to the EMPTY case, and that scoping is load-bearing: a NON-EMPTY history is
     * itself proof the authority speaks generation 1, because those events came out of it. So no
     * generation may perturb an answer the forward-only rule already established - including the
     * delisted-AFTER pass, which is the one #127 exists to protect.
     */
    @Test
    fun aNonEmptyHistoryIsUnaffectedByWhateverTheProbeSaid() {
        val delistedAfter = listOf(granted(100), delisted(700))
        val delistedBefore = listOf(granted(100), delisted(199))
        for (g in RoaxRpc.AuthorityGeneration.values()) {
            assertEquals(
                GrantAtIssuance.Authorized,
                RoaxRpc.grantAtIssuance(delistedAfter, anchored, g),
            )
            assertEquals(
                GrantAtIssuance.NotAuthorized,
                RoaxRpc.grantAtIssuance(delistedBefore, anchored, g),
            )
        }
    }
}
