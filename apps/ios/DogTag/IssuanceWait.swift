import Foundation

/// The dog-tag issuance wait's ENDING sentences — the one decision behind what the owner is told
/// when the chain poll stops, mirrored byte-for-byte with Android's `IssuanceWait.kt` (change one,
/// change both; each side's tests pin the cases).
///
/// Why this is a pure, tested pair: the poll used to end in ONE sentence — "Submitted; anchoring is
/// still pending. Check the vet portal for completion." — which implies the work is still coming.
/// Measured live (2026-08-07), the bind had FAILED minutes earlier (`mintCustodial` reverted at
/// estimation) and the reason sat in a session row nobody could reach. The device now peeks
/// `GET /p/<token>/status` (the same one-time token, retained consumed server-side), so the ending
/// states what was actually observed — a definite failure with the server's device-safe reason, or
/// an honest could-not-confirm — and never a promise.
///
/// FFI-free and SwiftUI-free on purpose, so the host-less `DogTagTests` bundle can compile it.
enum IssuanceWait {
    /// A mid-poll or end-of-poll DEFINITE failure: the server settled the session to "error". The
    /// server's `reason` is already the device-safe stage sentence; an empty one still names where
    /// the answer lives rather than guessing.
    static func failureText(reason: String) -> String {
        let trimmed = reason.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty { return trimmed }
        return "This issuance failed at the vet's backend. The vet portal names the reason — ask the clinic."
    }

    /// The poll ran out without the anchor becoming visible. `status` is the server peek's answer
    /// ("pending" / "minting" / "bound" / "error"), or nil when the peek itself could not be
    /// reached. Every sentence states the OBSERVATION and where the outcome lives — none promises
    /// the work is still coming.
    static func timeoutText(serverStatus: String?, reason: String?) -> String {
        switch serverStatus {
        case "error":
            return failureText(reason: reason ?? "")
        case "bound":
            return "The vet reports this dog tag as issued, but its anchor was not yet visible "
                + "on-chain from this phone. Check again later, or ask the clinic to confirm on "
                + "the vet portal."
        case "minting":
            return "The vet's anchoring had not completed when this screen stopped waiting. The "
                + "vet portal shows the outcome — ask the clinic."
        case "pending":
            return "The vet backend accepted the scan but did not record this bind. Ask the "
                + "clinic to check the vet portal and start again if needed."
        default:
            return "Could not confirm this issuance completed: the anchor was not visible "
                + "on-chain and the vet backend could not be reached. The vet portal shows the "
                + "outcome — ask the clinic."
        }
    }
}
