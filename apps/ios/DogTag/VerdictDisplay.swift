import Foundation

/// What a credential badge is ALLOWED to claim, given how the verdict was reached and how long ago.
///
/// The list surfaces (Documents, Home, Travel, the credential detail) render a `verdict` that was
/// determined by a chain read at some point in the past and then persisted. Nothing re-reads the chain
/// when those screens appear, so a record REVOKED after the last check kept showing a full-strength
/// green VALID forever. #87 made the age of that answer visible in a small grey sub-line, but the badge
/// itself - the loud part - still asserted VALID with no discount at all. Two claims on one row, and
/// the louder one was the one that could be wrong.
///
/// The rule this file enforces: **a surface states an accurate observation, or says it could not
/// check.** A green VALID on the strength of a check run days ago is a claim stronger than the
/// evidence. Equally, an answer that has merely gone stale must NOT flip to INVALID - "I have not
/// looked recently" is its own state and must never render as either neighbour.
///
/// Ordering, most severe first:
///
/// | Condition | Badge | Tone |
/// |---|---|---|
/// | verdict INVALID | `INVALID` | negative |
/// | `validUntil` lapsed | `EXPIRED` | warning |
/// | verdict VALID, last check older than `freshFor` | `VALID · STALE` | neutral |
/// | verdict VALID, checked recently | `VALID` | positive |
/// | anything else (UNVERIFIED) | as-is | neutral |
///
/// INVALID sits above EXPIRED because revoked beats expired - a revoked credential is revoked whatever
/// its validity window says - and above staleness because softening an ESTABLISHED negative on the
/// strength of a non-answer is exactly the laundering #94 closed on the refresh path, pointed at the
/// badge instead. Age may only make a claim weaker, never a record look better.
///
/// EXPIRED sits above staleness because expiry needs no chain access: `validity.validUntil` is a
/// Merkle-covered leaf of the stored document, so it is tamper-evident and knowable offline. It cannot
/// itself go stale, which makes it strictly more informative than "I could not check".
///
/// Pure and clock-injected on purpose: no `Date()` inside, no SwiftUI types, no JSON.
///
/// Mirrors Android `VerdictDisplay` in VerdictDisplay.kt, branch for branch - see the PR body's table.
/// Android's twin is behaviourally tested and mutation-checked; this port has NO test target able to
/// reach it, so it is written to be a line-by-line mirror rather than a second implementation.
enum VerdictDisplay {

    /// How long a chain-read verdict may be presented as a live claim before the badge stops asserting
    /// it.
    ///
    /// This is a DISPLAY POLICY, not a security boundary: nothing downstream trusts a badge, and a
    /// verifier always makes its own read. It is set to an hour because the failure the audit named was
    /// a badge speaking for a check run *days* ago, and because erring short only ever makes the app
    /// under-claim - the safe direction for every rule in this file.
    static let freshFor: TimeInterval = 60 * 60

    /// Which of the four semantic colours a badge takes. Kept out of the UI layer so the RULE and the
    /// palette cannot drift apart across the five surfaces that render one.
    enum Tone { case positive, warning, negative, neutral }

    struct Badge: Equatable {
        let label: String
        let tone: Tone
    }

    /// The badge for a stored verdict. `now` is injected so the whole decision is a pure function of
    /// its inputs.
    static func badge(verdict: String, lastCheckedAt: String?, validUntil: String?, now: Date) -> Badge {
        if verdict == "INVALID" { return Badge(label: "INVALID", tone: .negative) }
        if lapsed(validUntil, now: now) { return Badge(label: "EXPIRED", tone: .warning) }
        if verdict == "VALID", !isFresh(lastCheckedAt, now: now) {
            return Badge(label: "VALID · STALE", tone: .neutral)
        }
        if verdict == "VALID" { return Badge(label: "VALID", tone: .positive) }
        // UNVERIFIED - and any unrecognised stored string - is already a non-asserting state. Age adds
        // nothing to it, and relabelling it would lose the reason the refresher recorded.
        return Badge(label: verdict, tone: .neutral)
    }

    /// Whether a stored check is recent enough for its answer to still be asserted.
    ///
    /// A missing stamp is NOT fresh: records stored before #87 shipped carry none, and inventing
    /// freshness for them is the same over-claim in a different costume.
    ///
    /// A stamp in the FUTURE is not fresh either. A forward clock skew is not evidence that a check ran
    /// recently, and treating it as such would hand anyone with a wrong device clock a permanently
    /// green badge. Under-claiming is the correct direction to fail.
    static func isFresh(_ lastCheckedAt: String?, now: Date) -> Bool {
        guard let checked = Stamp.parse(lastCheckedAt) else { return false }
        let age = now.timeIntervalSince(checked)
        return age >= 0 && age < freshFor
    }

    /// Whether the document's own validity window has closed, as of `now`.
    ///
    /// THE single implementation of "is this expired". The travel receipt used to carry its own copy of
    /// this comparison, which is how the list badges came to ignore expiry entirely while the receipt
    /// enforced it - two implementations of one rule, the shape #94 removed from the verdict fold.
    ///
    /// Date-only, UTC, lexicographic - ISO-8601 dates sort correctly as strings, and the leaf is a date,
    /// not an instant, so comparing at day granularity is what the document actually claims. A record is
    /// expired only once the day AFTER `validUntil` has begun; `validUntil` itself is an inclusive last
    /// day.
    ///
    /// A blank, short or unparseable value means the document makes NO expiry claim, which is never the
    /// same as an expired one: the non-answer rule, pointed at expiry.
    static func lapsed(_ validUntil: String?, now: Date) -> Bool {
        guard let raw = validUntil?.trimmingCharacters(in: .whitespacesAndNewlines),
              raw.count >= 10 else { return false }
        return String(raw.prefix(10)) < dateOnlyUtc(now)
    }

    /// `now` as a bare UTC `yyyy-MM-dd`, the form the `validUntil` leaf is written in.
    static func dateOnlyUtc(_ now: Date) -> String { dateOnly.string(from: now) }

    private static let dateOnly: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd"
        f.timeZone = TimeZone(identifier: "UTC")
        f.locale = Locale(identifier: "en_US_POSIX")
        return f
    }()
}
