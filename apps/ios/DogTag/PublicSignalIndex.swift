import Foundation

/// Frozen public-signal order for `consent.circom/DogTagConsent(6)`, mirrored on-chain by
/// `VerificationRegistryConsent`.
///
/// [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
enum PublicSignalIndex {
    static let count = 7

    enum ownerHidden {
        static let dogTagId = 0
        static let purpose = 1
        static let relayer = 2
        static let nullifier = 3
        static let root = 4
        static let recordType = 5
        static let deadline = 6
    }
}
