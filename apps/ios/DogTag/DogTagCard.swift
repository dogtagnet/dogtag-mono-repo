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
        ///
        /// The payload is a CAUSE, never the failure's own message - see `reasonText`.
        case unreadable(OwnerStoreFailure)
        /// Not asked yet - the read is file I/O and happens after the first frame.
        case pending
    }

    /// Why the owner-secret store could not be read - a closed set the code itself names, so that
    /// what the card prints is always constructed rather than quoted from an error. See `reasonText`.
    /// `CaseIterable` so a test can walk every cause and pin that each renders a constructed
    /// sentence, mirroring Kotlin's `entries`; a cause added later cannot slip in as silence.
    enum OwnerStoreFailure: Equatable, CaseIterable {
        /// The stored bytes never came back: a locked (`.completeFileProtection`) file, or I/O.
        case couldNotRead
        /// They came back, but did not decode into owner-secret records.
        case couldNotDecode
    }

    /// What is known about this device's ability to prove consent for one listed tag.
    ///
    /// Three cases for the same reason `OwnedTagSource` has three: a row whose owner-secret record is
    /// missing because the store said so is a different claim from one whose record is missing
    /// because the store never answered, and only the first may be stated as a fact. Collapsing this
    /// to a `Bool` re-introduces the card's own defect one level down - a definite negative asserted
    /// over a source that was never read.
    enum OwnerSecretEvidence: Equatable {
        /// The store answered and holds this tag's owner-secret; consent can still be proved here.
        case held
        /// The store answered and holds no record for this tag. A definite negative, safe to state.
        case notHeld
        /// The store did not answer. Unestablished - the card must claim nothing either way.
        case unknown
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
        /// `R`, known only for a tag whose owner-secret this device holds - and therefore `nil` both
        /// when no such record exists AND when the store did not answer. A `nil` root renders no row
        /// at all rather than a negative one, so it asserts nothing on its own; `ownerSecret` is what
        /// says whether the store was in a position to answer.
        let rootHex: String?
        /// Whether this device holds the owner-secret for this tag - or could not establish it.
        let ownerSecret: OwnerSecretEvidence
        /// A credential naming this tag has been imported here.
        let credentialImported: Bool

        var id: String { dogTagIdDec }
    }

    /// Everything the card renders, including whether it was able to check at all.
    struct State: Equatable {
        let rows: [Row]
        /// The constructed sentence for a store that could not be read, or `nil` when it answered.
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
        // Whether the ABSENCE of an owner-secret record for a tag is established at all. Only a
        // store that answered can license the definite negative; under `.unreadable` and `.pending`
        // every row's `own` is nil for a reason that says nothing about that row.
        var storeAnswered = false
        switch owned {
        case let .records(tags): ownedTags = tags; storeAnswered = true
        case let .unreadable(cause): unavailable = shortReason(reasonText(cause))
        case .pending: pending = true
        }

        // Imported pets are the weaker source and need filtering: `RecordImporter` stores a 32-hex
        // share token in `dogTagId` whenever the wrapped doc carries no handle, and that is not a
        // tag. Owner-secret records need no such filter - the builder refuses anything but a decimal
        // handle, so a record in the store is a real tag by construction.
        var importedByTag: [String: ImportedTag] = [:]
        for tag in imported where isAsciiDecimal(tag.dogTagIdDec) {
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
                ownerSecret: own != nil ? .held : (storeAnswered ? .notHeld : .unknown),
                credentialImported: imp != nil
            )
        }
        .sorted { descendingHandle($0.dogTagIdDec, $1.dogTagIdDec) }

        return State(
            rows: rows,
            ownerStoreUnavailable: unavailable,
            ownerStorePending: pending
        )
    }

    /// What the card says when the owner-secret store did not answer - CONSTRUCTED here, never an
    /// underlying failure's own message.
    ///
    /// The store this reports on holds the owner-secret and the attribute salts, and an error caught
    /// while reading it is free to quote what it was reading. Here that is bounded - a `DecodingError`
    /// renders the coding path, which is field names and indices rather than values - but Android's
    /// `org.json` counterpart appends the tokenizer input outright, and by the time a decode fails
    /// there the decryption has already succeeded, so that input is the store's own plaintext. Rather
    /// than reason per platform about how much any given error happens to expose, the payload of
    /// `.unreadable` is a closed set of causes on both: there is no caller text for this to echo, by
    /// construction rather than by convention.
    ///
    /// Both sentences still state the consequence - a tag created by issuance is missing from the
    /// list - because saying nothing would recreate the false absence this card exists to close.
    static func reasonText(_ cause: OwnerStoreFailure) -> String {
        switch cause {
        // No "unlock and retry" here: this cause also covers a store that is genuinely damaged,
        // where retrying can never work, and promising a remedy that cannot deliver is the same
        // over-claim in a smaller place.
        case .couldNotRead:
            return "Could not read this device's owner-secret store, so any tag created by "
                + "issuance is missing from this list. The device may be locked, or the store damaged."
        case .couldNotDecode:
            return "This device's owner-secret store was read but its contents did not decode, so "
                + "any tag created by issuance is missing from this list."
        }
    }

    /// How much of a store-read failure the card prints. See `shortReason`.
    static let maxReasonChars = 160

    /// A store-read message collapsed to one printable line.
    ///
    /// Applied here rather than at the renderer so neither platform can forget it, and retained as
    /// the residual cap now that `reasonText` is the only thing feeding it: whatever a future cause's
    /// sentence grows into, the settings card gets one line rather than a paragraph that buries the
    /// part that matters ("any tag created by issuance is missing from this list"). Nothing
    /// actionable is lost to the cap - the remedy does not vary with the tail, the tags are still on
    /// chain, and recovery needs the seed plus the credential either way.
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
        guard isAsciiDecimal(s) else { return nil }
        let trimmed = String(s.drop(while: { $0 == "0" }))
        return trimmed.isEmpty ? "0" : trimmed
    }

    /// A non-empty run of ASCII `0`-`9`, which is what a `dogTagId` handle actually is.
    ///
    /// Deliberately NOT `\.isNumber` / Android's `Char::isDigit`: both admit Unicode digits beyond
    /// ASCII, and they admit DIFFERENT ones - `\.isNumber` covers Nd, Nl and No (so `½` and `Ⅸ`
    /// pass) while `isDigit` is Nd only (so `٣` U+0663 passes on both, `½` only here). A shared
    /// module whose two halves accept different handles would list different rows on the two
    /// platforms for one identical store, which is the exact drift this module exists to prevent.
    /// Nothing reaches either source with a non-ASCII digit today, so this is a tightening rather
    /// than a behaviour change.
    private static func isAsciiDecimal(_ s: String) -> Bool {
        !s.isEmpty && s.allSatisfy { $0.isASCII && $0.isNumber }
    }
}
