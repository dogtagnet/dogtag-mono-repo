package io.liberalize.dogtag.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Duration
import java.time.Instant

/**
 * AUDIT RECS 2 AND 7 (2026-07-27 mock audit), the half #87 and #94 left open.
 *
 * #87 gave the app a refresh and a `lastCheckedAt`; #94 made the verdict fold monotone. Neither
 * touched what the BADGE renders. Documents, Home, the credential detail and Travel all drew a
 * full-strength green `VALID` straight from the stored verdict string, so:
 *
 *  - a credential revoked after the last check kept a green badge until someone tapped refresh, and
 *  - an EXPIRED credential read green on every one of those surfaces, because `validity.validUntil`
 *    was enforced in exactly three places and no badge was one of them.
 *
 * These cases pin the rule: **a surface states an accurate observation, or says it could not check.**
 * A green VALID on the strength of a check run days ago is a claim stronger than the evidence — and a
 * verdict that has merely gone stale must not flip to INVALID either, because "I have not looked
 * recently" is its own state and may not render as either neighbour.
 *
 * The clock is injected, so the boundary cases are exact rather than wall-clock flaky.
 */
class VerdictBadgeStalenessTest {

    private val now: Instant = Instant.parse("2026-07-28T12:00:00Z")

    private fun ago(d: Duration): String = now.minus(d).toString()

    private fun badge(
        verdict: String,
        lastCheckedAt: String? = ago(Duration.ofMinutes(1)),
        validUntil: String? = null,
    ) = VerdictDisplay.badge(verdict, lastCheckedAt, validUntil, now)

    // ---- staleness: only ever weakens a claim -----------------------------------------------------

    /**
     * THE regression. A verdict read off the chain minutes ago may still be asserted.
     *
     * Mutation check: this is the case that stays green when the fix is removed, so it is here to stop
     * an over-eager staleness rule turning every badge grey.
     */
    @Test
    fun aRecentlyCheckedValidStillReadsValid() {
        assertEquals(
            VerdictDisplay.Badge("VALID", VerdictDisplay.Tone.POSITIVE),
            badge("VALID", lastCheckedAt = ago(Duration.ofMinutes(5))),
        )
    }

    /**
     * THE defect, stated. A chain read from three days ago is not evidence about the chain now, and a
     * revocation in between is exactly the case the audit named. The badge must stop asserting.
     *
     * Mutation check: delete the `!isFresh(...)` arm from `VerdictDisplay.badge` and this reverts to
     * `VALID` / POSITIVE — the shipped defect.
     */
    @Test
    fun aValidCheckedDaysAgoNoLongerAssertsValid() {
        val out = badge("VALID", lastCheckedAt = ago(Duration.ofDays(3)))
        assertEquals("VALID · STALE", out.label)
        assertEquals(VerdictDisplay.Tone.NEUTRAL, out.tone)
    }

    /**
     * The other direction of the same rule, and the mistake this codebase has already made twice. A
     * badge that has not been refreshed must NOT read as revoked: "could not check" is its own state
     * and must never render as either neighbour.
     */
    @Test
    fun aStaleValidNeverRendersAsEitherNeighbour() {
        val out = badge("VALID", lastCheckedAt = ago(Duration.ofDays(30)))
        assertTrue("must not claim validity", out.tone != VerdictDisplay.Tone.POSITIVE)
        assertTrue("must not accuse", out.tone != VerdictDisplay.Tone.NEGATIVE)
        assertTrue("the last answer stays visible", out.label.contains("VALID"))
        assertTrue("and is visibly discounted", out.label.contains("STALE"))
    }

    /**
     * #94's rule, pointed at the badge. Staleness is a NON-ANSWER, and a non-answer may not raise a
     * verdict's severity — nor lower it. An established INVALID stays INVALID however old it is; a
     * future refactor that "simplifies" staleness into a blanket neutral would launder a known-revoked
     * credential into "could not check" on the very surface the owner reads.
     */
    @Test
    fun anEstablishedInvalidIsNotSoftenedByAge() {
        assertEquals(
            VerdictDisplay.Badge("INVALID", VerdictDisplay.Tone.NEGATIVE),
            badge("INVALID", lastCheckedAt = ago(Duration.ofDays(400))),
        )
    }

    /** UNVERIFIED is already a non-asserting state; age cannot make it any weaker, and relabelling it
     *  would throw away the reason the refresher recorded beside it. */
    @Test
    fun unverifiedIsUnchangedByAge() {
        assertEquals(
            VerdictDisplay.Badge("UNVERIFIED", VerdictDisplay.Tone.NEUTRAL),
            badge("UNVERIFIED", lastCheckedAt = ago(Duration.ofDays(9))),
        )
    }

    /** A record stored before `lastCheckedAt` shipped has no stamp. Absent evidence is not fresh
     *  evidence — inventing freshness for exactly the oldest records is the same over-claim. */
    @Test
    fun aRecordWithNoCheckStampIsNotFresh() {
        assertEquals("VALID · STALE", badge("VALID", lastCheckedAt = null).label)
        assertEquals("VALID · STALE", badge("VALID", lastCheckedAt = "").label)
        assertEquals("VALID · STALE", badge("VALID", lastCheckedAt = "not-a-date").label)
    }

    /** A device clock set forward is not evidence that a check ran recently. Under-claim instead. */
    @Test
    fun aCheckStampInTheFutureIsNotFresh() {
        assertFalse(VerdictDisplay.isFresh(now.plus(Duration.ofHours(2)).toString(), now))
        assertEquals("VALID · STALE", badge("VALID", lastCheckedAt = now.plus(Duration.ofDays(1)).toString()).label)
    }

    /** The window boundary, exactly. Pins [VerdictDisplay.FRESH_FOR] as the single knob. */
    @Test
    fun theFreshnessWindowBoundaryIsExact() {
        val edge = VerdictDisplay.FRESH_FOR
        assertTrue(VerdictDisplay.isFresh(ago(edge.minusSeconds(1)), now))
        assertFalse("the window is exclusive at its far edge", VerdictDisplay.isFresh(ago(edge), now))
        assertFalse(VerdictDisplay.isFresh(ago(edge.plusSeconds(1)), now))
    }

    // ---- expiry: a definite, offline, root-covered negative ---------------------------------------

    /**
     * AUDIT REC 7. `validity.validUntil` is a Merkle-covered leaf, so it is tamper-evident and readable
     * with no chain access at all — yet every list badge ignored it, and an expired credential read
     * green everywhere except the receipt sheet.
     *
     * Mutation check: delete the `lapsed(...)` arm from `VerdictDisplay.badge` and this reverts to
     * `VALID` / POSITIVE.
     */
    @Test
    fun anExpiredCredentialDoesNotReadAsValid() {
        val out = badge("VALID", lastCheckedAt = ago(Duration.ofSeconds(1)), validUntil = "2026-07-01")
        assertEquals("EXPIRED", out.label)
        assertEquals(VerdictDisplay.Tone.WARNING, out.tone)
    }

    /** Expiry needs no chain read, so it outranks staleness: it is a definite fact where staleness is
     *  the absence of one. A stale AND expired record reports the thing actually known. */
    @Test
    fun expiryOutranksStalenessBecauseItIsKnowableOffline() {
        assertEquals("EXPIRED", badge("VALID", lastCheckedAt = ago(Duration.ofDays(5)), validUntil = "2020-01-01").label)
    }

    /** …and it outranks UNVERIFIED for the same reason: "expired" is strictly more informative than
     *  "I could not check". */
    @Test
    fun expiryIsReportedEvenWhenTheChainCouldNotBeReached() {
        assertEquals("EXPIRED", badge("UNVERIFIED", validUntil = "2026-07-27").label)
    }

    /** Revoked beats expired. An expired credential is not authorised; a revoked one is repudiated,
     *  and softening that to amber would understate it. */
    @Test
    fun revocationOutranksExpiry() {
        assertEquals("INVALID", badge("INVALID", validUntil = "2020-01-01").label)
    }

    /** `validUntil` is the INCLUSIVE last day: a credential good "until 2026-07-28" is good ON the
     *  28th. Off-by-one here would expire every credential a day early, in the app's face. */
    @Test
    fun theValidUntilDayItselfIsStillValid() {
        assertEquals("VALID", badge("VALID", validUntil = "2026-07-28").label)
        assertEquals("EXPIRED", badge("VALID", validUntil = "2026-07-27").label)
    }

    /** A full timestamp is compared at day granularity, because the leaf is a date claim. */
    @Test
    fun aFullTimestampIsComparedAtDayGranularity() {
        assertFalse(VerdictDisplay.lapsed("2026-07-28T00:00:01Z", now))
        assertTrue(VerdictDisplay.lapsed("2026-07-27T23:59:59Z", now))
    }

    /**
     * The non-answer rule, pointed at expiry. A document that makes NO validity claim is not an expired
     * document, and a blank or truncated leaf must never be read as one — that would be this branch's
     * own defect class inverted, manufacturing a negative out of missing data.
     */
    @Test
    fun aMissingOrUnusableValidUntilIsNotAnExpiryClaim() {
        for (v in listOf(null, "", "   ", "2026", "2026-07")) {
            assertFalse("`$v` is not an expiry claim", VerdictDisplay.lapsed(v, now))
        }
        assertEquals("VALID", badge("VALID", validUntil = null).label)
        assertEquals("VALID", badge("VALID", validUntil = "").label)
    }

    /** UTC, not device-local: two phones either side of the date line must agree about the same
     *  document. The comparison instant is the only clock involved. */
    @Test
    fun theExpiryComparisonIsUtc() {
        assertEquals("2026-07-28", VerdictDisplay.dateOnlyUtc(Instant.parse("2026-07-28T23:59:59Z")))
        assertEquals("2026-07-29", VerdictDisplay.dateOnlyUtc(Instant.parse("2026-07-29T00:00:00Z")))
    }
}
