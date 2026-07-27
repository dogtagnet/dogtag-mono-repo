package io.liberalize.dogtag.data

import java.time.Duration
import java.time.Instant
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter

/**
 * What a credential badge is ALLOWED to claim, given how the verdict was reached and how long ago.
 *
 * The list surfaces (Documents, Home, Travel, the credential detail) render a `verdict` that was
 * determined by a chain read at some point in the past and then persisted. Nothing re-reads the chain
 * when those screens appear, so a record REVOKED after the last check kept showing a full-strength
 * green VALID forever. #87 made the age of that answer visible in a small grey sub-line, but the badge
 * itself - the loud part - still asserted VALID with no discount at all. Two claims on one row, and
 * the louder one was the one that could be wrong.
 *
 * The rule this file enforces: **a surface states an accurate observation, or says it could not
 * check.** A green VALID on the strength of a check run days ago is a claim stronger than the
 * evidence. Equally, an answer that has merely gone stale must NOT flip to INVALID - "I have not
 * looked recently" is its own state and must never render as either neighbour.
 *
 * Ordering, most severe first:
 *
 * | Condition | Badge | Tone |
 * |---|---|---|
 * | verdict INVALID | `INVALID` | negative |
 * | [validUntil] lapsed | `EXPIRED` | warning |
 * | verdict VALID, last check older than [FRESH_FOR] | `VALID · STALE` | neutral |
 * | verdict VALID, checked recently | `VALID` | positive |
 * | anything else (UNVERIFIED) | as-is | neutral |
 *
 * INVALID sits above EXPIRED because revoked beats expired - a revoked credential is revoked whatever
 * its validity window says - and above staleness because softening an ESTABLISHED negative on the
 * strength of a non-answer is exactly the laundering #94 closed on the refresh path, pointed at the
 * badge instead. Age may only make a claim weaker, never a record look better.
 *
 * EXPIRED sits above staleness because expiry needs no chain access: the validity leaf (see
 * [WrappedDoc.validUntil]) is Merkle-covered by the stored document, so it is tamper-evident and
 * knowable offline. It cannot itself go stale, which makes it strictly more informative than "I could
 * not check".
 *
 * Pure and clock-injected on purpose: no `Instant.now()` inside, no Android types, no JSON. That is
 * what lets a plain JUnit test drive every branch including the exact boundary — and the iOS port is
 * Foundation-only for exactly the same reason, so `VerdictDisplayTests` pins it in the host-less
 * bundle. Neither side is an untested mirror of the other; both are covered, case for case.
 *
 * Mirrors iOS `VerdictDisplay` in VerdictDisplay.swift.
 */
object VerdictDisplay {

    /**
     * How long a chain-read verdict may be presented as a live claim before the badge stops asserting
     * it.
     *
     * This is a DISPLAY POLICY, not a security boundary: nothing downstream trusts a badge, and a
     * verifier always makes its own read. It is set to an hour because the failure the audit named was
     * a badge speaking for a check run *days* ago, and because erring short only ever makes the app
     * under-claim - the safe direction for every rule in this file.
     */
    val FRESH_FOR: Duration = Duration.ofHours(1)

    /** Which of the four semantic colours a badge takes. Kept out of the UI layer so the RULE and the
     *  palette cannot drift apart across the five surfaces that render one. */
    enum class Tone { POSITIVE, WARNING, NEGATIVE, NEUTRAL }

    data class Badge(val label: String, val tone: Tone)

    /**
     * The badge for a stored verdict. [now] is injected so the boundary case is testable and so the
     * whole decision is a pure function of its inputs.
     */
    fun badge(verdict: String, lastCheckedAt: String?, validUntil: String?, now: Instant): Badge = when {
        verdict == "INVALID" -> Badge("INVALID", Tone.NEGATIVE)
        lapsed(validUntil, now) -> Badge("EXPIRED", Tone.WARNING)
        verdict == "VALID" && !isFresh(lastCheckedAt, now) -> Badge("VALID · STALE", Tone.NEUTRAL)
        verdict == "VALID" -> Badge("VALID", Tone.POSITIVE)
        // UNVERIFIED - and any unrecognised stored string - is already a non-asserting state. Age adds
        // nothing to it, and relabelling it would lose the reason the refresher recorded.
        else -> Badge(verdict, Tone.NEUTRAL)
    }

    /**
     * Whether a stored check is recent enough for its answer to still be asserted.
     *
     * A missing stamp is NOT fresh: records stored before #87 shipped carry none, and inventing
     * freshness for them is the same over-claim in a different costume.
     *
     * A stamp in the FUTURE is not fresh either. A forward clock skew is not evidence that a check ran
     * recently, and treating it as such would hand anyone with a wrong device clock a permanently
     * green badge. Under-claiming is the correct direction to fail.
     */
    fun isFresh(lastCheckedAt: String?, now: Instant): Boolean {
        val checked = Stamp.parse(lastCheckedAt) ?: return false
        val age = Duration.between(checked, now)
        return !age.isNegative && age < FRESH_FOR
    }

    /**
     * Whether the document's own validity window has closed, as of [now].
     *
     * THE single implementation of "is this expired". The travel receipt used to carry its own copy of
     * this comparison, which is how the list badges came to ignore expiry entirely while the receipt
     * enforced it - two implementations of one rule, the shape #94 removed from the verdict fold.
     *
     * Date-only, UTC, lexicographic - ISO-8601 dates sort correctly as strings, and the leaf is a date,
     * not an instant, so comparing at day granularity is what the document actually claims. A record is
     * expired only once the day AFTER `validUntil` has begun; `validUntil` itself is an inclusive last
     * day.
     *
     * A blank, short or unparseable value means the document makes NO expiry claim, which is never the
     * same as an expired one: the non-answer rule, pointed at expiry.
     */
    fun lapsed(validUntil: String?, now: Instant): Boolean {
        val raw = validUntil?.trim() ?: return false
        if (raw.length < 10) return false
        return raw.substring(0, 10) < dateOnlyUtc(now)
    }

    /** `now` as a bare UTC `yyyy-MM-dd`, the form the `validUntil` leaf is written in. */
    fun dateOnlyUtc(now: Instant): String = DATE_ONLY.format(now)

    private val DATE_ONLY: DateTimeFormatter =
        DateTimeFormatter.ofPattern("yyyy-MM-dd").withZone(ZoneOffset.UTC)
}
