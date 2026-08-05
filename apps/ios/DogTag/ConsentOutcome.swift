import Foundation

/// The terminal result of presenting a credential to a verifier, as a VALUE carrying plain-language
/// copy - so the consent screen can say, unmissably, which of three things happened.
///
/// WHY THIS TYPE EXISTS. The flow used to end by assigning a raw string to `status`, rendered as
/// 12pt muted text below the button. When the anti-redirect check correctly refused a verifier whose
/// claimed chain did not match DogTag's on-chain anchor, the holder saw a Face ID tick and then -
/// as far as he could tell - nothing at all. The app had protected him and could not tell him so.
/// A refusal the holder cannot see is functionally a silent failure, and this one sits on the screen
/// where a medical record is released.
///
/// THE THREE KINDS ARE THE WHOLE POINT, and they must never collapse:
///
///   * `succeeded`        - it worked. The record was shared and the verification recorded.
///   * `refused`          - DogTag deliberately declined. Nothing was shared, and that is the
///                          protection working. NOT an error: nothing is broken.
///   * `couldNotComplete` - we could not finish the check. Nothing was shared either, but this is
///                          not a judgement about the verifier - could-not-check is never a verdict.
///
/// A holder who cannot tell "we refused you for a good reason" from "it broke" will either hand the
/// record over somewhere else, or distrust a product that was working correctly.
///
/// Pure (Foundation only, no SwiftUI, no FFI) so `DogTagTests` compiles it and the copy rules below
/// are pinned rather than argued. `ScanScreen` owns the rendering.
enum ConsentOutcome: Equatable {

    /// Why DogTag declined to release the record. Every case here is a DELIBERATE protective refusal.
    enum Refusal: Equatable {
        /// The session arrived without the discovery claims the anchor check needs.
        case sessionMissingClaims
        /// THE ANTI-REDIRECT TRIP. The verifier's claims disagreed with DogTag's own on-chain record.
        /// `detail` is the raw comparison (e.g. `claimed chainId 0 does not match anchor chainId 135`)
        /// and is shown as supporting detail, never as the headline.
        case verifierFailedAnchorCheck(detail: String)
        /// The verifier holds no authorization to run this kind of check.
        case verifierNotAuthorized
        /// The verifier's domain could not be confirmed as theirs.
        case verifierDomainUnverified
        /// The one-time consent had already been spent - the replay guard.
        case alreadyRecorded
    }

    /// Why the flow could not finish. None of these is a judgement about the verifier.
    enum Failure: Equatable {
        case authentication(String?)
        /// Either the version is unpublished or the chain could not be reached; the app cannot tell
        /// those apart, and must not claim either.
        case anchorUnavailable
        case walletLocked
        case noOwnerSecret
        case unsupportedSecretVersion
        case provingArtifactMissing
        case proofRejected(String)
        case recordingFailed(String)
        case unexpected(String)
    }

    case succeeded(txHash: String?)
    /// Submitted and accepted, but the on-chain recording has not confirmed yet.
    case awaitingConfirmation
    case refused(Refusal)
    case couldNotComplete(Failure)

    // MARK: - What the holder is told

    /// The headline. Short enough to read at a glance and unambiguous about which of the three
    /// kinds this is.
    var title: String {
        switch self {
        case .succeeded: return "Shared and verified"
        case .awaitingConfirmation: return "Shared - confirming on-chain"
        case .refused: return "Not shared - DogTag stopped this"
        case .couldNotComplete: return "Not shared - couldn't complete the check"
        }
    }

    /// Plain words. No enum names, no `Invalid(...)`, no jargon - the raw comparison lives in
    /// `technicalDetail`.
    var explanation: String {
        switch self {
        case .succeeded:
            return "Your record was shared with this verifier and the check was recorded on-chain. Your identity stayed hidden."
        case .awaitingConfirmation:
            return "Your record was shared. The on-chain record is still confirming - this can take a few moments."
        case let .refused(r):
            switch r {
            case .sessionMissingClaims:
                return "This request didn't include the details DogTag needs to check who is asking, so nothing was shared."
            case .verifierFailedAnchorCheck:
                return "This verifier's details don't match DogTag's official on-chain record, so nothing was shared. That's the protection working - it's what stops a fake or redirected verifier from collecting your record."
            case .verifierNotAuthorized:
                return "This verifier isn't authorised to run this kind of check, so nothing was shared."
            case .verifierDomainUnverified:
                return "DogTag couldn't confirm this verifier's website really belongs to them, so nothing was shared."
            case .alreadyRecorded:
                return "This request was already used once. A consent can only be spent a single time, so nothing was shared again."
            }
        case let .couldNotComplete(f):
            switch f {
            case .authentication:
                return "You weren't signed in, so nothing was shared. Try again."
            case .anchorUnavailable:
                return "DogTag couldn't reach its on-chain records to check this verifier, so nothing was shared. This is usually a connection problem - it doesn't mean the verifier is bad."
            case .walletLocked:
                return "Your wallet couldn't be unlocked, so nothing was shared. Authenticate and try again."
            case .noOwnerSecret:
                return "This phone doesn't hold the private owner proof for this dog tag, so it can't prove consent. Nothing was shared."
            case .unsupportedSecretVersion:
                return "This dog tag's owner proof was made by a newer version of DogTag. Update the app to use it. Nothing was shared."
            case .provingArtifactMissing:
                return "This build is missing part of its proving setup, so consent couldn't be proven. Nothing was shared."
            case .proofRejected:
                return "The verifier rejected the proof, so nothing was recorded."
            case .recordingFailed:
                return "The proof was sent but couldn't be recorded on-chain."
            case .unexpected:
                return "Something went wrong before anything could be shared."
            }
        }
    }

    /// The raw machine detail - shown small, UNDER the plain explanation, never instead of it. It is
    /// what makes a support conversation possible without making the headline unreadable.
    var technicalDetail: String? {
        switch self {
        case let .succeeded(tx): return tx.map { "tx \($0)" }
        case .awaitingConfirmation: return nil
        case let .refused(r):
            switch r {
            case let .verifierFailedAnchorCheck(detail): return detail
            default: return nil
            }
        case let .couldNotComplete(f):
            switch f {
            case let .authentication(m): return m
            case let .proofRejected(m): return m
            case let .recordingFailed(m): return m
            case let .unexpected(m): return m
            default: return nil
            }
        }
    }

    /// THE REASSURANCE. True whenever the record did not leave the phone. Rendered as its own line,
    /// because "did my medical record go out?" is the first thing a holder wants answered and it must
    /// not be buried inside a paragraph.
    var nothingWasShared: Bool {
        switch self {
        case .succeeded, .awaitingConfirmation: return false
        case .refused, .couldNotComplete: return true
        }
    }

    /// How the card should read at a glance. Three visually distinct treatments for the three kinds -
    /// a refusal is deliberately NOT the failure colour, because nothing is broken.
    enum Tone: Equatable { case success, blocked, failure }

    var tone: Tone {
        switch self {
        case .succeeded, .awaitingConfirmation: return .success
        case .refused: return .blocked
        case .couldNotComplete: return .failure
        }
    }

    /// SF Symbol for the card, so the three kinds differ by shape and not by colour alone.
    var iconName: String {
        switch tone {
        case .success: return "checkmark.seal.fill"
        case .blocked: return "hand.raised.fill"
        case .failure: return "exclamationmark.triangle.fill"
        }
    }

    /// Pull the readable comparison out of the FFI error's description.
    ///
    /// `DiscoveryError` crosses UniFFI as a STRING (AGENTS.md records this), so on device the raw
    /// value reads `Invalid("claimed chainId 0 does not match anchor chainId 135")`. The inner
    /// sentence is the useful part; the Swift enum wrapper is noise to a holder. Falls back to the
    /// whole string rather than dropping it - losing the detail entirely would be worse than showing
    /// it unpolished, since it is the only thing a support conversation has to go on.
    static func discoveryDetail(_ raw: String) -> String {
        // First double-quoted run, if there is one.
        if let open = raw.firstIndex(of: "\""),
           case let rest = raw.index(after: open),
           let close = raw[rest...].firstIndex(of: "\"") {
            let inner = String(raw[rest..<close])
            if !inner.isEmpty { return inner }
        }
        return raw.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Whether the holder can sensibly retry. A refusal is a decision, not a transient fault - and
    /// offering "try again" against a verifier we just refused would invite the holder to keep
    /// pushing at exactly the wall that is protecting them.
    var suggestsRetry: Bool {
        switch self {
        case .succeeded, .awaitingConfirmation, .refused: return false
        case .couldNotComplete: return true
        }
    }
}
