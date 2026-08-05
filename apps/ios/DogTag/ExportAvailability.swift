import Foundation

/// Why the export panel's "Approve & export" control can or cannot proceed - as a VALUE, so the
/// reason is rendered declaratively beside the control instead of living in a code path that only
/// runs once the control has already been pressed.
///
/// WHY THIS TYPE EXISTS. The control was `.disabled(selected == nil || working)` while the only
/// sentence explaining a missing selection sat inside `presentExport`'s `guard let credential =
/// selected else { status = "Select a record first." }`. That guard is UNREACHABLE in exactly the
/// state it describes: the button is disabled precisely when `selected == nil`, so the action never
/// fires and the message can never render. A holder with an empty wallet got a full-colour button
/// that did nothing and said nothing - the refuses-without-saying-why shape this codebase removes
/// everywhere else (`NearbyDecision`, `VerdictDisplay`, and the provider portal's
/// `actionAvailability`, whose own note records that the FIRST-RUN state is the one that ships
/// broken).
///
/// TWO BLOCKED STATES, NEVER ONE. `noRecordsHeld` and `noneSelected` both make `selected == nil`,
/// and collapsing them would send the holder to the wrong place: one needs a vet to issue them a
/// record, the other just needs a tap. Different remedies, so different sentences.
///
/// Pure (Foundation only, no SwiftUI, no FFI), so `DogTagTests` compiles it directly and the
/// no-silent-refusal invariant below is pinned rather than argued.
enum ExportAvailability: Equatable {
    /// A record is selected and nothing is in flight - the control is live.
    case ready
    /// A presentation is already running; the control shows "Working…" and must not re-fire.
    case working
    /// The wallet holds no records at all, so there is nothing that could be selected. This is the
    /// first-run state, and the one that shipped with a dead button.
    case noRecordsHeld
    /// Records are on offer but the holder has not picked one.
    case noneSelected

    /// The single decision. `candidateCount` is the number of records the panel is offering.
    static func of(candidateCount: Int, hasSelection: Bool, working: Bool) -> ExportAvailability {
        if working { return .working }
        if candidateCount <= 0 { return .noRecordsHeld }
        if !hasSelection { return .noneSelected }
        return .ready
    }

    /// Whether the control may be pressed. The view binds `.disabled(!canProceed)` to this so the
    /// enabled-ness and the explanation below can never disagree.
    var canProceed: Bool { self == .ready }

    /// What is wrong, in the holder's terms. Non-nil for every state that blocks - see the
    /// exhaustiveness test; that equivalence is the whole point of the type.
    var blockedReason: String? {
        switch self {
        case .ready: return nil
        case .working: return nil
        case .noRecordsHeld:
            return "No records on this phone yet, so there is nothing to export."
        case .noneSelected:
            return "No record selected yet."
        }
    }

    /// What to do about it. Separate from `blockedReason` because naming the fault without naming the
    /// remedy is half a message, and the remedies here are genuinely different.
    var nextStep: String? {
        switch self {
        case .ready, .working: return nil
        case .noRecordsHeld:
            return "Ask your vet or groomer for a record QR, scan it to import, then scan this request again."
        case .noneSelected:
            return "Tap one of the records above to continue."
        }
    }

    /// True when the control is inert AND that is something the holder should be told about.
    /// `working` is inert too, but the panel already surfaces live progress, so it needs no notice.
    var showsBlockedNotice: Bool { blockedReason != nil }
}
