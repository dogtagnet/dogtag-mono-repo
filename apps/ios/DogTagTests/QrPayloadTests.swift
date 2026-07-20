import XCTest

/// `QrPayload.parse` is the app's ONLY trust boundary against a scanned QR: every byte it sees is
/// attacker-chosen, and the only acceptable failure mode is `.unknown` — never a crash. These tests
/// pin that contract, and are deliberately mirrored by
/// `apps/android/app/src/test/java/io/liberalize/dogtag/qr/QrPayloadTest.kt` so the two platforms
/// cannot silently disagree about what a given QR means.
///
/// The regression that motivated this file: `parse` used `Dictionary(uniqueKeysWithValues:)` over the
/// query items, which documents distinct keys as a PRECONDITION and TRAPS when it is violated. A
/// Swift trap is uncatchable, so a QR containing `?a=1&a=2` crashed the scanner outright, with no iOS
/// analogue of Android's `catch -> Unknown`.
final class QrPayloadTests: XCTestCase {

    // MARK: - the four recognised shapes

    func testShortImportTokenParses() {
        guard case let .importRecordToken(host, token) = QrPayload.parse("https://vet.example.com/r/\(String(repeating: "ab", count: 16))") else {
            return XCTFail("expected .importRecordToken")
        }
        XCTAssertEqual(host, "https://vet.example.com")
        XCTAssertEqual(token, String(repeating: "ab", count: 16))
    }

    func testExportSessionParsesAndKeepsThePortInTheOrigin() {
        guard case let .exportSession(host, token, addr) = QrPayload.parse("https://groomer.example.com:8443/x/tok123?a=0xAbC") else {
            return XCTFail("expected .exportSession")
        }
        XCTAssertEqual(host, "https://groomer.example.com:8443")
        XCTAssertEqual(token, "tok123")
        XCTAssertEqual(addr, "0xAbC")
    }

    func testDogTagIssuanceSessionParses() {
        guard case let .dogTagIssueSession(_, token) = QrPayload.parse("https://vet.example.com/p/\(String(repeating: "cd", count: 16))") else {
            return XCTFail("expected .dogTagIssueSession")
        }
        XCTAssertEqual(token, String(repeating: "cd", count: 16))
    }

    func testLegacyJwtImportParses() {
        guard case let .importRecord(_, recordId, jwt) = QrPayload.parse("https://vet.example.com/r?t=header.payload.sig&i=rec-1") else {
            return XCTFail("expected .importRecord")
        }
        XCTAssertEqual(recordId, "rec-1")
        XCTAssertEqual(jwt, "header.payload.sig")
    }

    // MARK: - duplicate query keys (the E-7 trap; must behave identically on both platforms)

    /// Before the fix this call did not fail an assertion — it TRAPPED and took the process down.
    /// Both platforms must now keep the FIRST occurrence (Android via `Uri.getQueryParameter`).
    func testDuplicateQueryKeyKeepsTheFirstValueAndDoesNotCrash() {
        guard case let .exportSession(_, _, addr) = QrPayload.parse("https://groomer.example.com/x/tok?a=0xFIRST&a=0xSECOND") else {
            return XCTFail("expected .exportSession")
        }
        XCTAssertEqual(addr, "0xFIRST")
    }

    func testDuplicateKeysOnTheLegacyJwtShapeKeepTheFirstValue() {
        guard case let .importRecord(_, recordId, jwt) = QrPayload.parse("https://vet.example.com/r?t=first&t=second&i=one&i=two") else {
            return XCTFail("expected .importRecord")
        }
        XCTAssertEqual(jwt, "first")
        XCTAssertEqual(recordId, "one")
    }

    func testManyDuplicatesOfAnIrrelevantKeyStillParse() {
        let junk = (1...50).map { "x=\($0)" }.joined(separator: "&")
        guard case let .exportSession(_, _, addr) = QrPayload.parse("https://groomer.example.com/x/tok?a=0xAddr&\(junk)") else {
            return XCTFail("expected .exportSession")
        }
        XCTAssertEqual(addr, "0xAddr")
    }

    // MARK: - malformed / hostile input: every one of these must be .unknown, never a crash

    func testMalformedInputDegradesToUnknown() {
        let hostile = [
            "",
            "   ",
            "not a url at all",
            "://missing-scheme",
            "https://",
            "https://vet.example.com",              // no path
            "https://vet.example.com/r/",           // empty token
            "https://vet.example.com/x/tok",        // export without the required `a`
            "https://vet.example.com/x/?a=0xAddr",  // export without a token
            "https://vet.example.com/p/",           // empty issuance token
            "https://vet.example.com/p/tok?a=1",    // issuance must have NO query string
            "https://vet.example.com/r/tok?a=1",    // short import must have NO query string
            "https://vet.example.com/r?t=only",     // legacy jwt missing `i`
            "https://vet.example.com/r?i=only",     // legacy jwt missing `t`
            "https://vet.example.com/unknown/path",
            "javascript:alert(1)",
        ]
        // NOTE: invalid percent-encoding (`?a=%ZZ`) is deliberately NOT asserted here. Android's `Uri`
        // lenient-decodes it to a garbage string while iOS's `URLComponents` treats it differently, so
        // the two platforms genuinely diverge. Neither CRASHES, which is what this file guards; the
        // resulting address is rejected downstream when it fails to parse as an EVM address.
        for raw in hostile {
            guard case .unknown = QrPayload.parse(raw) else {
                return XCTFail("expected .unknown for <\(raw)>")
            }
        }
    }

    func testSurroundingWhitespaceIsTrimmed() {
        guard case .dogTagIssueSession = QrPayload.parse("  \n https://vet.example.com/p/\(String(repeating: "ef", count: 16))  \t ") else {
            return XCTFail("expected .dogTagIssueSession")
        }
    }

    func testTrailingSlashDoesNotChangeTheLegacyJwtShape() {
        guard case .importRecord = QrPayload.parse("https://vet.example.com/r/?t=a.b.c&i=rec") else {
            return XCTFail("expected .importRecord")
        }
    }
}
