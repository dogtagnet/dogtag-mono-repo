import Foundation

/// What the Profile screen's **Dog-tags** card is allowed to state, derived from the two independent
/// places this device can learn that a tag exists.
///
/// # Why this exists at all
///
/// The card used to render from the imported-*pets* list alone, and **custodial issuance never writes
/// a pet**. So a phone that had scanned a real vet QR, folded the profile tree on-device from its own
/// wallet seed, posted `R`, and watched the vet anchor and mint it - with `profileRoot(dogTagId)` on
/// chain equal to the root the phone built - still said *"No dog tag yet"*. It survived an app
/// restart, because the two stores simply never met: `ProfileTreeStore.buildAndPersist` upserts an
/// `OwnerSecretRecord` and nothing on the custodial-bind path adds a pet, while `store.pets` is
/// populated by the separate record-import QR.
///
/// That is not a cosmetic gap. The owner holds a minted tag on chain and its owner-secret on this
/// device, and the product's own surface denies it - a false statement about the owner's own
/// property, rendered as absence. **A surface states an accurate observation, or says it could not
/// check**, and an empty card is an accurate observation only when every source actually answered.
///
/// # Why the card reads the store's THROWING accessor
///
/// `ProfileTreeStore.all()` is `(try? load()) ?? []`, so an existing-but-unreadable store answers
/// "no records". Here that must surface instead: the file is `.completeFileProtection`, so
/// `unreadableFile` covers a genuinely transient locked-device read as well as corruption, and
/// reporting either as "No dog tag yet" would be the same false absence in a second flavour.
/// `OwnedTagSource.unreadable` keeps that distinct from a store that answered "none", and
/// `.pending` keeps it distinct from a read that has not happened yet.
///
/// # Why it is pure
///
/// Foundation only - no FFI, no SwiftUI, no store - so it compiles into the host-less `DogTagTests`
/// bundle and the claim rules are pinned rather than argued. `DogTagCard.kt` is a case-for-case
/// mirror; the two must not drift on what the card claims.
enum DogTagCard {

    // ---- inputs ----------------------------------------------------------------------------------

    /// A tag this device created by issuance - the owner-secret store's view of it.
    ///
    /// Deliberately NOT `ProfileTreeStore.OwnerSecretRecord`: that type lives in a file that imports
    /// the FFI, and depending on it would pull this module out of the host-less test bundle. The
    /// caller projects the two fields the card can render.
    struct OwnedTag: Equatable {
        /// The human-facing decimal handle.
        let dogTagIdDec: String
        /// `R`, the write-once profile root the issuer anchored for this tag.
        let rootHex: String
    }

    /// A tag this device knows of because a credential naming it was imported.
    struct ImportedTag: Equatable {
        let dogTagIdDec: String
        /// The raw stored name; `realName` decides whether it is genuinely one.
        let name: String
    }

    /// What the device-local owner-secret store answered when the card asked it.
    ///
    /// Three cases, not two, because "there are no tags", "I could not read the store" and "I have
    /// not looked yet" are three different claims and only the first licenses an empty card.
    enum OwnedTagSource: Equatable {
        /// It answered. An empty array genuinely means this device created no tag.
        case records([OwnedTag])
        /// It did not answer: the store exists but could not be read or decoded. Any tag created by
        /// issuance is therefore unlisted, and the card must say so instead of reporting absence.
        case unreadable(String)
        /// Not asked yet - the read is file I/O and happens after the first frame.
        case pending
    }

    // ---- output ----------------------------------------------------------------------------------

    /// One tag the card lists, and exactly what is known about it.
    ///
    /// A tag can be known through issuance without any credential having been imported for it, so
    /// `name` is genuinely absent rather than filled with a stand-in.
    struct Row: Identifiable, Equatable {
        let dogTagIdDec: String
        /// The pet's name, or `nil` when this device does not know one. Never a placeholder.
        let name: String?
        /// `R`, known only for a tag whose owner-secret this device holds.
        let rootHex: String?
        /// This device holds the owner-secret, so it can still prove consent for this tag.
        let ownerSecretHeld: Bool
        /// A credential naming this tag has been imported here.
        let credentialImported: Bool

        var id: String { dogTagIdDec }
    }

    /// Everything the card renders, including whether it was able to check at all.
    struct State: Equatable {
        let rows: [Row]
        /// Why the owner-secret store could not be read, or `nil` when it answered.
        let ownerStoreUnavailable: String?
        /// The owner-secret store has not been read yet.
        let ownerStorePending: Bool

        /// The ONE condition under which the card may state that there is no dog tag: every source
        /// answered, and none of them knows one. An unread or unreadable store makes absence
        /// unproven, so the card reports what it could not check instead of asserting a negative.
        var establishesNoTags: Bool {
            rows.isEmpty && ownerStoreUnavailable == nil && !ownerStorePending
        }
    }

    // ---- names -----------------------------------------------------------------------------------

    /// Names the importer writes when it has none - the same set `LocalStore.isRealName` rejects.
    /// A card that trusted the raw string would render a placeholder as though it were the pet's
    /// actual name.
    private static let placeholderNamePrefixes = ["DogTag #", "DogTag#"]

    /// The stored name if it is genuinely a name, else `nil` - never a substitute that reads like
    /// data. Mirrors `LocalStore.isRealName` and Android `DogTagCard.realName`; keep all three in step.
    static func realName(_ raw: String?) -> String? {
        let trimmed = (raw ?? "").trimmingCharacters(in: .whitespaces)
        if trimmed.isEmpty || trimmed == "Unnamed" { return nil }
        if placeholderNamePrefixes.contains(where: { trimmed.hasPrefix($0) }) { return nil }
        return trimmed
    }

    // ---- the fold --------------------------------------------------------------------------------

    /// Fold the two sources into what the card may say.
    ///
    /// The union is keyed by the decimal handle, so a tag known to BOTH sources is listed once, with
    /// each source contributing what only it knows: the owner-secret store contributes `R` and the
    /// fact that this device can still prove consent for the tag, the imported credential contributes
    /// the pet's name.
    static func state(owned: OwnedTagSource, imported: [ImportedTag]) -> State {
        var ownedTags: [OwnedTag] = []
        var unavailable: String?
        var pending = false
        switch owned {
        case let .records(tags): ownedTags = tags
        case let .unreadable(reason): unavailable = reason
        case .pending: pending = true
        }

        // Imported pets are the weaker source and need filtering: `RecordImporter` stores a 32-hex
        // share token in `dogTagId` whenever the wrapped doc carries no handle, and that is not a
        // tag. Owner-secret records need no such filter - the builder refuses anything but a decimal
        // handle, so a record in the store is a real tag by construction.
        var importedByTag: [String: ImportedTag] = [:]
        for tag in imported where !tag.dogTagIdDec.isEmpty && tag.dogTagIdDec.allSatisfy(\.isNumber) {
            importedByTag[tag.dogTagIdDec] = tag
        }
        var ownedByTag: [String: OwnedTag] = [:]
        for tag in ownedTags where !tag.dogTagIdDec.isEmpty {
            ownedByTag[tag.dogTagIdDec] = tag
        }

        let rows = Set(ownedByTag.keys).union(importedByTag.keys).map { id -> Row in
            let own = ownedByTag[id]
            let imp = importedByTag[id]
            return Row(
                dogTagIdDec: id,
                name: realName(imp?.name),
                rootHex: own?.rootHex,
                ownerSecretHeld: own != nil,
                credentialImported: imp != nil
            )
        }
        .sorted { descendingHandle($0.dogTagIdDec, $1.dogTagIdDec) }

        return State(
            rows: rows,
            ownerStoreUnavailable: unavailable.map(shortReason),
            ownerStorePending: pending
        )
    }

    /// How much of a store-read failure the card prints. See `shortReason`.
    static let maxReasonChars = 160

    /// A store-read failure collapsed to one printable line.
    ///
    /// Applied here rather than at the renderer so neither platform can forget it. The underlying
    /// errors are verbose - a `DecodingError` alone renders ~400 characters of nested
    /// `Context(codingPath:…)` - and pasting that into a settings card buries the sentence that
    /// matters ("any tag created by issuance is missing from this list") under a stack of Foundation
    /// internals. The head is the part worth keeping: it names the file and the class of failure.
    /// Nothing actionable is lost, because the remedy does not vary with the parse detail - the tags
    /// are still on chain, and recovery needs the seed plus the credential either way.
    static func shortReason(_ raw: String) -> String {
        let collapsed = raw
            .split(whereSeparator: \.isWhitespace)
            .joined(separator: " ")
        guard collapsed.count > maxReasonChars else { return collapsed }
        var head = String(collapsed.prefix(maxReasonChars))
        while let last = head.last, last.isWhitespace { head.removeLast() }
        return head + "…"
    }

    // ---- ordering --------------------------------------------------------------------------------

    /// Highest handle first: ids are allocated in ascending order, so the tag the owner just had
    /// issued - the one they opened this card to find - is the one at the top.
    ///
    /// Ordered on the digit string rather than through a fixed-width integer. `dogTagIdDec` is the
    /// operator-facing handle and small today, but nothing in the type bounds it, and an overflowing
    /// parse would silently reorder the list rather than fail. Longer normalized digit string means
    /// larger; same length compares lexicographically, which for equal-length digits IS the numeric
    /// order. Deliberately the same five lines as Android `DogTagCard.descendingHandle`.
    ///
    /// Returns whether `a` sorts before `b`.
    private static func descendingHandle(_ a: String, _ b: String) -> Bool {
        switch (normalizedHandle(a), normalizedHandle(b)) {
        case let (na?, nb?):
            if na.count != nb.count { return na.count > nb.count }
            return na > nb
        // A non-numeric handle cannot reach here from either source; it sorts last, never traps.
        case (_?, nil): return true
        case (nil, _?): return false
        case (nil, nil): return a < b
        }
    }

    /// Digits with leading zeros stripped, or `nil` when the handle is not a decimal number.
    private static func normalizedHandle(_ s: String) -> String? {
        guard !s.isEmpty, s.allSatisfy(\.isNumber) else { return nil }
        let trimmed = String(s.drop(while: { $0 == "0" }))
        return trimmed.isEmpty ? "0" : trimmed
    }
}
