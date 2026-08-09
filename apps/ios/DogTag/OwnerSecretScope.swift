import Foundation

/// The deployment an owner-secret record's tag lives on. Mirrors Android
/// `profile/OwnerSecretRecord.kt`'s `DeploymentScope` — change both together.
///
/// A dog tag's on-chain identity is (chain, SBT contract, id): the SAME decimal id exists
/// independently on every deployment, because a redeploy stands up a fresh `DogTagSBTConsent`
/// whose id space starts empty. This pair is the SMALLEST honest key for "which deployment": the
/// SBT address alone names the contract the tag is literally a token on, and `chainId` closes the
/// cross-chain address-collision corner. Both come from the bundled `roax.json` (the ledger's own
/// key names), the one deployment fact the phone holds.
///
/// Records written before this existed carry `nil` — "recorded before deployments were tracked" —
/// which is a distinct fact, never assumed equal to any deployment (see `OwnerSecretScoping`).
///
/// FFI-free and Foundation-only on purpose, so the host-less `DogTagTests` bundle can pin the
/// decision table below without dragging `ProfileTreeStore` (and with it the FFI) into the target.
struct DeploymentScope: Codable, Equatable {
    let chainId: Int
    let sbtAddress: String

    func matches(_ other: DeploymentScope) -> Bool {
        chainId == other.chainId
            && sbtAddress.caseInsensitiveCompare(other.sbtAddress) == .orderedSame
    }

    /// The CURRENT deployment, from the bundled config values. `nil` when the bundle is
    /// stale/blank (every consumer already treats a blank address as could-not-check) — callers
    /// must then fail closed rather than write a record whose deployment is a guess.
    static func of(chainId: Int, sbtAddress: String) -> DeploymentScope? {
        sbtAddress.trimmingCharacters(in: .whitespaces).isEmpty
            ? nil
            : DeploymentScope(chainId: chainId, sbtAddress: sbtAddress)
    }
}

/// The minimal view of a stored record the scoping decisions need — `ProfileTreeStore
/// .OwnerSecretRecord` conforms; tests conform a two-field stand-in.
protocol DeploymentScopedRecord {
    var dogTagIdDec: String { get }
    var deployment: DeploymentScope? { get }
}

/// The pure deployment-scoping decisions, mirrored field for field with Android
/// `OwnerSecretRecords` (`sameScope` / `preferredFor` / `reuseDecision`). One redeploy used to
/// poison every low id on a handset permanently — tag 1 on the new deployment collided with tag 1
/// on the old one, and the phone refused "this dogTagId already has different private profile
/// metadata" for a tag that was genuinely free. These rules are what removed that, without ever
/// orphaning the old record (its attribute salts exist nowhere else).
enum OwnerSecretScoping {
    /// Do two records live on the same deployment? Two legacy (untracked) records do; a legacy
    /// and a scoped record NEVER do — "unknown" must not compare equal to any deployment, or the
    /// store's write-once check would refuse a genuinely free id on a fresh deployment.
    static func sameScope(_ a: DeploymentScope?, _ b: DeploymentScope?) -> Bool {
        switch (a, b) {
        case (nil, nil): return true
        case let (sa?, sb?): return sa.matches(sb)
        default: return false
        }
    }

    /// Which stored record answers for `dogTagIdDec` on the CURRENT deployment: an exact scope
    /// match first, else a legacy (untracked) record, NEVER a record scoped to a different
    /// deployment — that one is another deployment's tag.
    static func preferred<R: DeploymentScopedRecord>(
        _ records: [R],
        forDogTagIdDec dec: String,
        current: DeploymentScope?
    ) -> R? {
        let forTag = records.filter { $0.dogTagIdDec == dec }
        return forTag.first { $0.deployment != nil && sameScope($0.deployment, current) }
            ?? forTag.first { $0.deployment == nil }
    }

    /// The bind flow's verdict on a stored record for the id the vet just named.
    enum ReuseDecision: Equatable {
        /// Same session (content byte-identical): rebuild from the stored witness. Safe across a
        /// redeploy — `R` is a pure device-side commitment, and it is being anchored NOW on the
        /// current deployment. A legacy record reused this way gets STAMPED with the current
        /// deployment (the migration: the byte-identical vet-salted identity leaves are the
        /// evidence tying it here).
        case reuse
        /// A legacy record whose content differs, met while the current deployment IS known: the
        /// redeploy reading. The record is another deployment's tag (the vet's allocator never
        /// re-hands a minted id within one deployment), so this id is genuinely free here — build
        /// a fresh witness and keep the old record untouched beside it.
        case buildFresh
        /// Same deployment (or no way to tell one), same id, different content: a real conflict.
        /// Refuse before the stored witness — unrecoverable attribute salts — can be disturbed.
        case refuseConflict
    }

    static func reuseDecision(
        recordScope: DeploymentScope?,
        current: DeploymentScope?,
        contentMatches: Bool
    ) -> ReuseDecision {
        if contentMatches { return .reuse }
        if recordScope == nil && current != nil { return .buildFresh }
        return .refuseConflict
    }
}
