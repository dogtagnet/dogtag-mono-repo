import XCTest

/// Pins the no-silent-refusal rule for the export panel's "Approve & export" control.
///
/// The defect these tests exist for: the control was disabled whenever `selected == nil`, while the
/// only sentence explaining a missing selection lived inside the action the disabled control never
/// fires. So the state a holder actually meets first — an empty wallet — produced a full-colour
/// button that did nothing and said nothing. These cases pin that the explanation is a property of
/// the STATE (and so always rendered) rather than of a code path.
final class ExportAvailabilityTests: XCTestCase {

    /// Every state the type can be in. The switch below is what keeps this list honest: adding a
    /// case to `ExportAvailability` without adding it here fails to COMPILE, so this can never
    /// silently stop covering a state.
    private static let allStates: [ExportAvailability] = [.ready, .working, .noRecordsHeld, .noneSelected]

    private func isExhaustive(_ s: ExportAvailability) -> Bool {
        switch s {
        case .ready, .working, .noRecordsHeld, .noneSelected: return true
        }
    }

    func test_theCaseListCoversEveryState() {
        for s in Self.allStates { XCTAssertTrue(isExhaustive(s)) }
        XCTAssertEqual(Set(Self.allStates.map { "\($0)" }).count, Self.allStates.count,
                       "duplicate state in the list")
    }

    // MARK: - The invariant the defect violated

    /// THE LOAD-BEARING TEST. A control that cannot be pressed must say why — with exactly one
    /// exemption, `working`, because the panel already renders live progress there (ForgeWaitView).
    /// Any other blocked state without a reason is a dead button, which is the bug.
    func test_everyBlockedStateExplainsItself_exceptWorkingWhichShowsProgressInstead() {
        for s in Self.allStates where !s.canProceed {
            if s == .working {
                XCTAssertNil(s.blockedReason, "working surfaces progress, not a blocked notice")
                XCTAssertFalse(s.showsBlockedNotice)
                continue
            }
            XCTAssertNotNil(s.blockedReason, "\(s) disables the control but gives no reason")
            XCTAssertNotNil(s.nextStep, "\(s) says what is wrong but not what to do")
            XCTAssertTrue(s.showsBlockedNotice)
        }
    }

    /// The converse: a usable control must not display a refusal beside it.
    func test_theUsableStateShowsNoRefusal() {
        XCTAssertTrue(ExportAvailability.ready.canProceed)
        XCTAssertNil(ExportAvailability.ready.blockedReason)
        XCTAssertNil(ExportAvailability.ready.nextStep)
        XCTAssertFalse(ExportAvailability.ready.showsBlockedNotice)
    }

    // MARK: - The captain's actual state

    /// The exact state hit on KZG: a freshly installed wallet holding nothing, scanning a verifier's
    /// export request. Before the fix this rendered a live-looking button and no explanation.
    func test_anEmptyWalletIsToldItHasNothingToExportAndWhatToDoAboutIt() {
        let s = ExportAvailability.of(candidateCount: 0, hasSelection: false, working: false)
        XCTAssertEqual(s, .noRecordsHeld)
        XCTAssertFalse(s.canProceed)

        let reason = try? XCTUnwrap(s.blockedReason)
        XCTAssertNotNil(reason)
        // It must name the ABSENCE of records rather than blaming the selection.
        XCTAssertTrue(s.blockedReason!.lowercased().contains("no records"),
                      "the reason must say there are no records: \(s.blockedReason!)")

        // And it must name what he needs first — a record issued to him — not merely restate the fault.
        let step = s.nextStep!
        XCTAssertTrue(step.lowercased().contains("vet") || step.lowercased().contains("groomer"),
                      "the next step must name who issues a record: \(step)")
        XCTAssertTrue(step.lowercased().contains("scan"),
                      "the next step must say how to get one in: \(step)")
    }

    /// The two blocked states must not collapse: one needs an issuer, the other needs a tap.
    func test_havingNoRecordsAndHavingSelectedNoneAreDifferentMessages() {
        let empty = ExportAvailability.of(candidateCount: 0, hasSelection: false, working: false)
        let unpicked = ExportAvailability.of(candidateCount: 3, hasSelection: false, working: false)
        XCTAssertEqual(empty, .noRecordsHeld)
        XCTAssertEqual(unpicked, .noneSelected)
        XCTAssertNotEqual(empty.blockedReason, unpicked.blockedReason)
        XCTAssertNotEqual(empty.nextStep, unpicked.nextStep)
        // The unpicked case must NOT send the holder off to a vet — they already hold records.
        XCTAssertFalse(unpicked.nextStep!.lowercased().contains("vet"),
                       "a holder who already has records must not be told to go get one")
    }

    // MARK: - The decision itself

    func test_selectingARecordMakesTheControlUsable() {
        XCTAssertEqual(ExportAvailability.of(candidateCount: 3, hasSelection: true, working: false), .ready)
    }

    /// `working` outranks everything, so a re-render mid-flight cannot re-enable the control.
    func test_workingOutranksEveryOtherState() {
        XCTAssertEqual(ExportAvailability.of(candidateCount: 0, hasSelection: false, working: true), .working)
        XCTAssertEqual(ExportAvailability.of(candidateCount: 3, hasSelection: true, working: true), .working)
        XCTAssertFalse(ExportAvailability.working.canProceed)
    }

    /// A negative count is nonsense input; it must fail closed to "nothing to export", never to ready.
    func test_aNonsenseCountFailsClosed() {
        XCTAssertEqual(ExportAvailability.of(candidateCount: -1, hasSelection: true, working: false),
                       .noRecordsHeld)
    }
}
