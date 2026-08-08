package io.liberalize.dogtag.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Mirrors iOS's `IssuanceWaitTests` case for case — the two platforms must not diverge on what an
 * owner is told when the issuance wait ends.
 */
class IssuanceWaitTest {
    @Test
    fun aDefiniteFailureCarriesTheServersDeviceSafeReason() {
        val reason = "This dog tag's root was anchored on-chain but the tag itself could not be " +
            "minted. The vet portal names the reason — ask the clinic to fix it and retry this " +
            "same issuance."
        assertEquals(reason, IssuanceWait.failureText(reason))
    }

    @Test
    fun anEmptyServerReasonStillNamesWhereTheAnswerLives_neverGuesses() {
        val text = IssuanceWait.failureText("   ")
        assertTrue(text, text.contains("vet portal"))
        assertTrue(text, text.contains("failed"))
    }

    /** The old ending promised completion ("anchoring is still pending"); no ending may any more. */
    @Test
    fun noTimeoutSentencePromisesTheWorkIsStillComing() {
        for (status in listOf("bound", "minting", "pending", null)) {
            val text = IssuanceWait.timeoutText(status, null)
            assertFalse("$status: $text", text.contains("is still pending"))
            assertTrue(text, text.contains("clinic") || text.contains("vet portal"))
        }
    }

    @Test
    fun aServerReportedErrorAtTimeoutIsTheFailureNotATimeout() {
        val text = IssuanceWait.timeoutText(
            "error",
            "The vet's on-chain anchoring of this dog tag failed. The vet portal names the " +
                "reason — ask the clinic to fix it and retry the issuance.",
        )
        assertTrue(text, text.contains("anchoring of this dog tag failed"))
    }

    /**
     * "The vet says bound but this phone cannot see it" is a DIFFERENT fact from "the vet is
     * still working" and from "nothing answered" — each gets its own sentence.
     */
    @Test
    fun theThreeNonErrorEndingsAreToldApart() {
        val bound = IssuanceWait.timeoutText("bound", null)
        val minting = IssuanceWait.timeoutText("minting", null)
        val unreachable = IssuanceWait.timeoutText(null, null)
        assertTrue(bound, bound.contains("reports this dog tag as issued"))
        assertTrue(minting, minting.contains("had not completed"))
        assertTrue(unreachable, unreachable.contains("could not be reached"))
        assertNotEquals(bound, minting)
        assertNotEquals(minting, unreachable)
    }
}
