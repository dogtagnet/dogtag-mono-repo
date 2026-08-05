import XCTest

/// Pins the consent screen's terminal messaging.
///
/// The defect: the anti-redirect check correctly refused a verifier whose claimed chain disagreed
/// with DogTag's on-chain anchor, and said so as `Owner-hidden verification refused: Invalid("claimed
/// chainId 0 does not match anchor chainId 135")` in 12pt muted text below the button. The holder saw
/// a Face ID tick and read the result as "nothing happened". A refusal nobody can see is a silent
/// failure, and this one guards the release of a medical record.
final class ConsentOutcomeTests: XCTestCase {

    private static let refusals: [ConsentOutcome.Refusal] = [
        .sessionMissingClaims,
        .verifierFailedAnchorCheck(detail: "claimed chainId 0 does not match anchor chainId 135"),
        .verifierNotAuthorized,
        .verifierDomainUnverified,
        .alreadyRecorded,
    ]
    private static let failures: [ConsentOutcome.Failure] = [
        .authentication("auth failed"), .anchorUnavailable, .walletLocked, .noOwnerSecret,
        .unsupportedSecretVersion, .provingArtifactMissing, .proofRejected("409"),
        .recordingFailed("reverted"), .unexpected("boom"),
    ]
    private static var all: [ConsentOutcome] {
        [.succeeded(txHash: "0xabc…"), .succeeded(txHash: nil), .awaitingConfirmation]
        + refusals.map { .refused($0) } + failures.map { .couldNotComplete($0) }
    }

    /// Compile-time exhaustiveness: a new case anywhere fails to build until it is covered here.
    func test_theCaseListsCoverEveryState() {
        for o in Self.all {
            switch o {
            case .succeeded, .awaitingConfirmation, .refused, .couldNotComplete: break
            }
        }
        for r in Self.refusals {
            switch r {
            case .sessionMissingClaims, .verifierFailedAnchorCheck, .verifierNotAuthorized,
                 .verifierDomainUnverified, .alreadyRecorded: break
            }
        }
        for f in Self.failures {
            switch f {
            case .authentication, .anchorUnavailable, .walletLocked, .noOwnerSecret,
                 .unsupportedSecretVersion, .provingArtifactMissing, .proofRejected,
                 .recordingFailed, .unexpected: break
            }
        }
    }

    // MARK: - Every outcome speaks

    /// No terminal state may be silent. This is the invariant the old shape broke.
    func test_everyOutcomeSaysSomethingInPlainWords() {
        for o in Self.all {
            XCTAssertFalse(o.title.isEmpty, "\(o) has no headline")
            XCTAssertFalse(o.explanation.isEmpty, "\(o) has no explanation")
            // Plain words means no enum/FFI vocabulary leaking into what the holder reads.
            for banned in ["Invalid(", "Optional(", "refused(", "couldNotComplete", "Error"] {
                XCTAssertFalse(o.title.contains(banned), "headline leaks `\(banned)`: \(o.title)")
                XCTAssertFalse(o.explanation.contains(banned),
                               "explanation leaks `\(banned)`: \(o.explanation)")
            }
        }
    }

    // MARK: - The three kinds must be distinguishable at a glance

    func test_theThreeKindsDifferByToneAndIcon() {
        let ok = ConsentOutcome.succeeded(txHash: nil)
        let blocked = ConsentOutcome.refused(.verifierNotAuthorized)
        let broke = ConsentOutcome.couldNotComplete(.anchorUnavailable)
        XCTAssertEqual(ok.tone, .success)
        XCTAssertEqual(blocked.tone, .blocked)
        XCTAssertEqual(broke.tone, .failure)
        // Colour alone is not a distinction - the icons must differ too.
        XCTAssertEqual(Set([ok.iconName, blocked.iconName, broke.iconName]).count, 3)
    }

    /// A deliberate refusal must NOT wear the failure tone. "We stopped this" and "it broke" send the
    /// holder to different places, and painting a refusal as an error teaches distrust of a product
    /// that was working exactly as designed.
    func test_aRefusalIsNeverStyledAsAFailure() {
        for r in Self.refusals {
            XCTAssertEqual(ConsentOutcome.refused(r).tone, .blocked, "\(r) styled as something else")
            XCTAssertNotEqual(ConsentOutcome.refused(r).tone, .failure)
        }
    }

    /// And a could-not-check must never be styled as a refusal - it is not a verdict about anyone.
    func test_aCouldNotCompleteIsNeverStyledAsARefusal() {
        for f in Self.failures {
            XCTAssertEqual(ConsentOutcome.couldNotComplete(f).tone, .failure, "\(f) styled as a refusal")
        }
    }

    // MARK: - The reassurance

    /// Whenever the record did not leave the phone, the outcome must say so. This is the first thing
    /// a holder wants to know after a screen that just asked for their face.
    func test_nothingLeftThePhoneIsStatedForEveryNonSuccess() {
        for r in Self.refusals { XCTAssertTrue(ConsentOutcome.refused(r).nothingWasShared) }
        for f in Self.failures { XCTAssertTrue(ConsentOutcome.couldNotComplete(f).nothingWasShared) }
        XCTAssertFalse(ConsentOutcome.succeeded(txHash: nil).nothingWasShared)
        XCTAssertFalse(ConsentOutcome.awaitingConfirmation.nothingWasShared)
    }

    // MARK: - The captain's actual case

    /// The exact refusal observed on KZG. It must read as protection, name what was compared only as
    /// supporting detail, and promise the record stayed put.
    func test_theAnchorMismatchReadsAsProtectionNotAsBreakage() {
        let raw = #"Invalid("claimed chainId 0 does not match anchor chainId 135")"#
        let o = ConsentOutcome.refused(.verifierFailedAnchorCheck(detail: ConsentOutcome.discoveryDetail(raw)))

        XCTAssertEqual(o.tone, .blocked)
        XCTAssertTrue(o.nothingWasShared)
        XCTAssertTrue(o.title.lowercased().contains("not shared"),
                      "the headline must lead with the record not going out: \(o.title)")
        // The explanation must frame it as the protection working, in words with no jargon.
        let e = o.explanation.lowercased()
        XCTAssertTrue(e.contains("don't match") || e.contains("does not match") || e.contains("doesn't match"),
                      "must say what disagreed: \(o.explanation)")
        XCTAssertTrue(e.contains("protection") || e.contains("stops"),
                      "must frame it as protection, not breakage: \(o.explanation)")
        // The raw comparison survives as DETAIL, stripped of the Swift/FFI wrapper.
        XCTAssertEqual(o.technicalDetail, "claimed chainId 0 does not match anchor chainId 135")
        XCTAssertFalse(o.technicalDetail!.contains("Invalid("))
        // And we must not invite the holder to keep pushing at the wall protecting them.
        XCTAssertFalse(o.suggestsRetry)
    }

    /// An unreachable chain is NOT a verdict about the verifier - it must read as our problem.
    func test_anUnreachableAnchorDoesNotAccuseTheVerifier() {
        let o = ConsentOutcome.couldNotComplete(.anchorUnavailable)
        XCTAssertEqual(o.tone, .failure)
        XCTAssertTrue(o.suggestsRetry)
        XCTAssertTrue(o.explanation.lowercased().contains("doesn't mean the verifier is bad")
                      || o.explanation.lowercased().contains("connection"),
                      "must not read as an accusation: \(o.explanation)")
    }

    // MARK: - Detail extraction

    func test_discoveryDetailUnwrapsTheFfiStringAndNeverLosesIt() {
        XCTAssertEqual(ConsentOutcome.discoveryDetail(#"Invalid("chain mismatch")"#), "chain mismatch")
        // No quotes: keep the whole thing rather than dropping the only diagnostic there is.
        XCTAssertEqual(ConsentOutcome.discoveryDetail("bare message"), "bare message")
        // Empty quotes must not swallow the string into nothing.
        XCTAssertEqual(ConsentOutcome.discoveryDetail(#"Invalid("")"#), #"Invalid("")"#)
    }

    // MARK: - Retry

    func test_onlyAFailureInvitesARetry() {
        for r in Self.refusals { XCTAssertFalse(ConsentOutcome.refused(r).suggestsRetry, "\(r)") }
        for f in Self.failures { XCTAssertTrue(ConsentOutcome.couldNotComplete(f).suggestsRetry, "\(f)") }
        XCTAssertFalse(ConsentOutcome.succeeded(txHash: nil).suggestsRetry)
    }
}
