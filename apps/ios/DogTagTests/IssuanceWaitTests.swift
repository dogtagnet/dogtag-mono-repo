import XCTest

/// Mirrors Android's `IssuanceWaitTest` case for case — the two platforms must not diverge on what
/// an owner is told when the issuance wait ends.
final class IssuanceWaitTests: XCTestCase {
    func test_aDefiniteFailureCarriesTheServersDeviceSafeReason() {
        let reason = "This dog tag's root was anchored on-chain but the tag itself could not be "
            + "minted. The vet portal names the reason — ask the clinic to fix it and retry this "
            + "same issuance."
        XCTAssertEqual(IssuanceWait.failureText(reason: reason), reason)
    }

    func test_anEmptyServerReasonStillNamesWhereTheAnswerLives_neverGuesses() {
        let text = IssuanceWait.failureText(reason: "   ")
        XCTAssertTrue(text.contains("vet portal"), text)
        XCTAssertTrue(text.contains("failed"), text)
    }

    /// The old ending promised completion ("anchoring is still pending"); no ending may any more.
    func test_noTimeoutSentencePromisesTheWorkIsStillComing() {
        for status in ["bound", "minting", "pending", nil] as [String?] {
            let text = IssuanceWait.timeoutText(serverStatus: status, reason: nil)
            XCTAssertFalse(
                text.contains("is still pending"),
                "\(String(describing: status)): \(text)")
            XCTAssertTrue(text.contains("clinic") || text.contains("vet portal"), text)
        }
    }

    func test_aServerReportedErrorAtTimeoutIsTheFailureNotATimeout() {
        let text = IssuanceWait.timeoutText(serverStatus: "error", reason: "The vet's on-chain anchoring of this dog tag failed. The vet portal names the reason — ask the clinic to fix it and retry the issuance.")
        XCTAssertTrue(text.contains("anchoring of this dog tag failed"), text)
    }

    /// "The vet says bound but this phone cannot see it" is a DIFFERENT fact from "the vet is
    /// still working" and from "nothing answered" — each gets its own sentence.
    func test_theThreeNonErrorEndingsAreToldApart() {
        let bound = IssuanceWait.timeoutText(serverStatus: "bound", reason: nil)
        let minting = IssuanceWait.timeoutText(serverStatus: "minting", reason: nil)
        let unreachable = IssuanceWait.timeoutText(serverStatus: nil, reason: nil)
        XCTAssertTrue(bound.contains("reports this dog tag as issued"), bound)
        XCTAssertTrue(minting.contains("had not completed"), minting)
        XCTAssertTrue(unreachable.contains("could not be reached"), unreachable)
        XCTAssertNotEqual(bound, minting)
        XCTAssertNotEqual(minting, unreachable)
    }
}
