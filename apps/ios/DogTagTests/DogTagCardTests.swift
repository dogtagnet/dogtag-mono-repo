import XCTest

/// What the Profile screen's Dog-tags card is allowed to claim.
///
/// The defect these pin was found by driving a real phone: a full owner-hidden issuance completed -
/// tree folded on-device, `R` posted, vet anchored and minted, `profileRoot(dogTagId)` on chain equal
/// to the root the phone built - and the app still said "No dog tag yet", because the card rendered
/// from the imported-*pets* list and custodial issuance never writes a pet.
///
/// `testIssuedTagAppearsWithNoImportedCredential` is the named regression: it goes red the moment the
/// owner-secret store stops contributing rows. Android's `DogTagCardTest` mirrors these case for case.
final class DogTagCardTests: XCTestCase {

    private func owned(_ id: String, root: String? = nil) -> DogTagCard.OwnedTag {
        DogTagCard.OwnedTag(dogTagIdDec: id, rootHex: root ?? "0x\(id)aa")
    }

    private func records(_ tags: DogTagCard.OwnedTag...) -> DogTagCard.OwnedTagSource {
        .records(tags)
    }

    // MARK: - the reported defect

    /// THE regression. A tag the owner had issued exists ONLY in the owner-secret store - no pet was
    /// ever imported for it - and the fold must list it. Reverting THIS FOLD to the imported-pets
    /// source makes this the test that goes red.
    ///
    /// Note the boundary: the SwiftUI call site in `ProfileScreen` has no coverage on either platform,
    /// so reverting THAT to `pets.filter { … }` reddens nothing here. Extracting the decision is what
    /// makes any of it pinnable at all - same trade as `VerdictDisplay`.
    func testIssuedTagAppearsWithNoImportedCredential() throws {
        let state = DogTagCard.state(owned: records(owned("7")), imported: [])

        XCTAssertEqual(state.rows.count, 1)
        let row = try XCTUnwrap(state.rows.first)
        XCTAssertEqual(row.dogTagIdDec, "7")
        XCTAssertEqual(row.rootHex, "0x7aa")
        XCTAssertTrue(row.ownerSecretHeld)
        XCTAssertFalse(row.credentialImported)
        XCTAssertFalse(state.establishesNoTags, "a listed tag is not an absence")
    }

    /// The name is genuinely unknown for such a tag, and must not be filled with a stand-in.
    func testAnIssuedTagWithNoImportedCredentialHasNoName() {
        let state = DogTagCard.state(owned: records(owned("7")), imported: [])
        XCTAssertNil(state.rows.first?.name)
    }

    // MARK: - the merge

    /// A tag known to both sources is ONE row carrying what each source alone knows.
    func testATagInBothSourcesIsListedOnce() throws {
        let state = DogTagCard.state(
            owned: records(owned("7", root: "0xroot7")),
            imported: [DogTagCard.ImportedTag(dogTagIdDec: "7", name: "Rex")]
        )

        XCTAssertEqual(state.rows.count, 1)
        let row = try XCTUnwrap(state.rows.first)
        XCTAssertEqual(row.name, "Rex")
        XCTAssertEqual(row.rootHex, "0xroot7")
        XCTAssertTrue(row.ownerSecretHeld)
        XCTAssertTrue(row.credentialImported)
    }

    /// An imported pet with no owner-secret is still the owner's tag and stays listed.
    func testAnImportedTagWithNoOwnerSecretIsStillListed() throws {
        let state = DogTagCard.state(
            owned: records(),
            imported: [DogTagCard.ImportedTag(dogTagIdDec: "42", name: "Bella")]
        )

        let row = try XCTUnwrap(state.rows.first)
        XCTAssertEqual(state.rows.count, 1)
        XCTAssertEqual(row.dogTagIdDec, "42")
        XCTAssertEqual(row.name, "Bella")
        XCTAssertNil(row.rootHex, "no owner-secret record means no known root")
        XCTAssertFalse(row.ownerSecretHeld)
        XCTAssertTrue(row.credentialImported)
    }

    /// Highest handle first: the just-issued tag is the one the owner opened the card to find.
    func testTagsAreListedNewestFirst() {
        let state = DogTagCard.state(
            owned: records(owned("9"), owned("100"), owned("11")),
            imported: [DogTagCard.ImportedTag(dogTagIdDec: "2", name: "Old")]
        )
        XCTAssertEqual(state.rows.map(\.dogTagIdDec), ["100", "11", "9", "2"])
    }

    /// `dogTagIdDec` is unbounded, so ordering must not go through a fixed-width integer.
    func testOrderingSurvivesAHandleWiderThanAnInt64() {
        let huge = "99999999999999999999999999"
        let state = DogTagCard.state(owned: records(owned("8"), owned(huge)), imported: [])
        XCTAssertEqual(state.rows.map(\.dogTagIdDec), [huge, "8"])
    }

    /// Ordering is on the digit string, so leading zeros must be normalized away - otherwise "007"
    /// would sort as a three-character handle above "42". Pinned on both platforms because the two
    /// implementations must agree; see Android `DogTagCardTest`.
    func testLeadingZerosDoNotInflateAHandle() {
        let state = DogTagCard.state(owned: records(owned("007"), owned("42")), imported: [])
        XCTAssertEqual(state.rows.map(\.dogTagIdDec), ["42", "007"])
    }

    /// `RecordImporter` stores a 32-hex share token in `dogTagId` when the wrapped doc carries no
    /// handle. That is not a tag and must not be listed.
    func testANonNumericImportedHandleIsNotATag() {
        let token = "0a1b2c3d4e5f60718293a4b5c6d7e8f9"
        let state = DogTagCard.state(
            owned: records(),
            imported: [
                DogTagCard.ImportedTag(dogTagIdDec: token, name: "Rex"),
                DogTagCard.ImportedTag(dogTagIdDec: "", name: "Blank"),
            ]
        )
        XCTAssertTrue(state.rows.isEmpty)
        XCTAssertTrue(state.establishesNoTags, "both sources answered and neither knows a tag")
    }

    // MARK: - "could not check" is not "none"

    /// An unreadable store must NOT read as an empty one. `ProfileTreeStore.all()` answers `[]` in
    /// exactly this case, which is why the card reads the throwing `loadActive()`.
    func testAnUnreadableStoreIsNotAnAbsence() {
        let state = DogTagCard.state(owned: .unreadable("file protection: device locked"), imported: [])

        XCTAssertTrue(state.rows.isEmpty)
        XCTAssertEqual(state.ownerStoreUnavailable, "file protection: device locked")
        XCTAssertFalse(
            state.establishesNoTags,
            "an unread store leaves absence unproven, so the card may not claim it"
        )
    }

    /// Nor may the first frame, before the read has landed, claim an absence.
    func testAPendingReadIsNotAnAbsence() {
        let state = DogTagCard.state(owned: .pending, imported: [])

        XCTAssertTrue(state.rows.isEmpty)
        XCTAssertTrue(state.ownerStorePending)
        XCTAssertNil(state.ownerStoreUnavailable)
        XCTAssertFalse(state.establishesNoTags)
    }

    /// Imported tags still render while the owner-secret store is unreadable - but not as complete.
    func testAnUnreadableStoreStillListsWhatTheOtherSourceKnows() {
        let state = DogTagCard.state(
            owned: .unreadable("corrupt"),
            imported: [DogTagCard.ImportedTag(dogTagIdDec: "42", name: "Bella")]
        )

        XCTAssertEqual(state.rows.map(\.dogTagIdDec), ["42"])
        XCTAssertEqual(state.ownerStoreUnavailable, "corrupt")
        XCTAssertFalse(state.establishesNoTags)
    }

    /// The only state that licenses "No dog tag yet".
    func testAnAnsweredEmptyStoreWithNoPetsEstablishesNoTags() {
        let state = DogTagCard.state(owned: records(), imported: [])

        XCTAssertTrue(state.rows.isEmpty)
        XCTAssertNil(state.ownerStoreUnavailable)
        XCTAssertFalse(state.ownerStorePending)
        XCTAssertTrue(state.establishesNoTags)
    }

    /// The underlying errors are verbose (a `DecodingError` renders ~400 characters), and a settings
    /// card that pastes one buries the sentence that matters. The head is kept because it names the
    /// file and the class of failure.
    func testAVerboseStoreFailureIsCollapsedToOnePrintableLine() throws {
        let raw = "dogtag-owner-secrets.json exists but could not be read;\n\trefusing to "
            + "overwrite it (it holds recovery secrets): " + String(repeating: "x", count: 400)
        let state = DogTagCard.state(owned: .unreadable(raw), imported: [])
        let shown = try XCTUnwrap(state.ownerStoreUnavailable)

        XCTAssertEqual(shown.count, DogTagCard.maxReasonChars + 1) // + the ellipsis
        XCTAssertTrue(
            shown.hasPrefix("dogtag-owner-secrets.json exists"),
            "keeps the head, which names the file"
        )
        XCTAssertTrue(shown.hasSuffix("…"))
        XCTAssertFalse(
            shown.contains("\n") || shown.contains("\t"),
            "newlines and tabs collapse to single spaces"
        )
    }

    /// A reason that already fits is passed through unchanged apart from whitespace collapsing.
    func testAShortStoreFailureIsNotTruncated() {
        let state = DogTagCard.state(owned: .unreadable("  keystore key   invalidated "), imported: [])
        XCTAssertEqual(state.ownerStoreUnavailable, "keystore key invalidated")
    }

    // MARK: - names

    /// `Pet` decoding defaults the name to "Unnamed" and the on-import fallback writes
    /// "DogTag #<id>". `LocalStore.isRealName` rejects both; this must too, or the card contradicts
    /// every other surface that names a pet.
    func testPlaceholderNamesAreNotNames() {
        XCTAssertNil(DogTagCard.realName("Unnamed"))
        XCTAssertNil(DogTagCard.realName("DogTag #7"))
        XCTAssertNil(DogTagCard.realName("DogTag#7"))
        XCTAssertNil(DogTagCard.realName("   "))
        XCTAssertNil(DogTagCard.realName(""))
        XCTAssertNil(DogTagCard.realName(nil))
    }

    func testARealNameIsKeptAndTrimmed() {
        XCTAssertEqual(DogTagCard.realName("  Rex  "), "Rex")
    }

    /// And the rejection reaches the row, not just the helper.
    func testAPlaceholderNamedPetContributesNoNameToItsRow() throws {
        let state = DogTagCard.state(
            owned: records(owned("7")),
            imported: [DogTagCard.ImportedTag(dogTagIdDec: "7", name: "Unnamed")]
        )
        let row = try XCTUnwrap(state.rows.first)
        XCTAssertNil(row.name)
        XCTAssertTrue(row.credentialImported, "the tag itself is still listed")
    }
}
