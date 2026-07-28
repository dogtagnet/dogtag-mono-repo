package io.liberalize.dogtag.profile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What the Profile screen's Dog-tags card is allowed to claim.
 *
 * The defect these pin was found by driving a real phone: a full owner-hidden issuance completed -
 * tree folded on-device, `R` posted, vet anchored and minted, `profileRoot(dogTagId)` on chain equal
 * to the root the phone built - and the app still said "No dog tag yet", because the card rendered
 * from the imported-*pets* list and custodial issuance never writes a pet.
 *
 * [issuedTagAppearsWithNoImportedCredential] is the named regression: it goes red the moment the
 * owner-secret store stops contributing rows. The iOS `DogTagCardTests` mirror these case for case.
 */
class DogTagCardTest {

    private fun owned(id: String, root: String = "0x${id}aa") = OwnedTag(dogTagIdDec = id, rootHex = root)

    private fun records(vararg tags: OwnedTag) = OwnedTagSource.Records(tags.toList())

    // ---- the reported defect --------------------------------------------------------------------

    /**
     * THE regression. A tag the owner had issued exists ONLY in the owner-secret store - no pet was
     * ever imported for it - and the fold must list it. Reverting THIS FOLD to the imported-pets
     * source makes this the test that goes red.
     *
     * Note the boundary: the Compose call site in `ProfileScreen` has no coverage on either platform,
     * so reverting THAT to `pets.filter { … }` reddens nothing here. Extracting the decision is what
     * makes any of it pinnable at all - same trade as `VerdictDisplay`.
     */
    @Test
    fun issuedTagAppearsWithNoImportedCredential() {
        val state = DogTagCard.state(owned = records(owned("7")), imported = emptyList())

        assertEquals(1, state.rows.size)
        val row = state.rows.single()
        assertEquals("7", row.dogTagIdDec)
        assertEquals("0x7aa", row.rootHex)
        assertTrue(row.ownerSecretHeld)
        assertFalse(row.credentialImported)
        assertFalse(
            "a listed tag is not an absence",
            state.establishesNoTags,
        )
    }

    /** The name is genuinely unknown for such a tag, and must not be filled with a stand-in. */
    @Test
    fun anIssuedTagWithNoImportedCredentialHasNoName() {
        val row = DogTagCard.state(records(owned("7")), emptyList()).rows.single()
        assertNull(row.name)
    }

    // ---- the merge -------------------------------------------------------------------------------

    /** A tag known to both sources is ONE row carrying what each source alone knows. */
    @Test
    fun aTagInBothSourcesIsListedOnce() {
        val state = DogTagCard.state(
            owned = records(owned("7", "0xroot7")),
            imported = listOf(ImportedTag("7", "Rex")),
        )

        assertEquals(1, state.rows.size)
        val row = state.rows.single()
        assertEquals("Rex", row.name)
        assertEquals("0xroot7", row.rootHex)
        assertTrue(row.ownerSecretHeld)
        assertTrue(row.credentialImported)
    }

    /** An imported pet with no owner-secret is still the owner's tag and stays listed. */
    @Test
    fun anImportedTagWithNoOwnerSecretIsStillListed() {
        val row = DogTagCard.state(records(), listOf(ImportedTag("42", "Bella"))).rows.single()

        assertEquals("42", row.dogTagIdDec)
        assertEquals("Bella", row.name)
        assertNull("no owner-secret record means no known root", row.rootHex)
        assertFalse(row.ownerSecretHeld)
        assertTrue(row.credentialImported)
    }

    /** Highest handle first: the just-issued tag is the one the owner opened the card to find. */
    @Test
    fun tagsAreListedNewestFirst() {
        val state = DogTagCard.state(
            owned = records(owned("9"), owned("100"), owned("11")),
            imported = listOf(ImportedTag("2", "Old")),
        )
        assertEquals(listOf("100", "11", "9", "2"), state.rows.map { it.dogTagIdDec })
    }

    /** `dogTagIdDec` is unbounded, so ordering must not go through a fixed-width integer. */
    @Test
    fun orderingSurvivesAHandleWiderThanALong() {
        val huge = "99999999999999999999999999"
        val state = DogTagCard.state(records(owned("8"), owned(huge)), emptyList())
        assertEquals(listOf(huge, "8"), state.rows.map { it.dogTagIdDec })
    }

    /**
     * Ordering is on the digit string, so leading zeros must be normalized away - otherwise "007"
     * would sort as a three-character handle above "42". Pinned on both platforms because the two
     * implementations must agree; see iOS `DogTagCardTests`.
     */
    @Test
    fun leadingZerosDoNotInflateAHandle() {
        val state = DogTagCard.state(records(owned("007"), owned("42")), emptyList())
        assertEquals(listOf("42", "007"), state.rows.map { it.dogTagIdDec })
    }

    /**
     * `RecordImporter` stores a 32-hex share token in `dogTagId` when the wrapped doc carries no
     * handle. That is not a tag and must not be listed.
     */
    @Test
    fun aNonNumericImportedHandleIsNotATag() {
        val token = "0a1b2c3d4e5f60718293a4b5c6d7e8f9"
        val state = DogTagCard.state(records(), listOf(ImportedTag(token, "Rex"), ImportedTag("", "Blank")))
        assertTrue(state.rows.isEmpty())
        assertTrue("both sources answered and neither knows a tag", state.establishesNoTags)
    }

    // ---- "could not check" is not "none" ---------------------------------------------------------

    /**
     * An unreadable store must NOT read as an empty one. `ProfileTreeStore.all()` answers
     * `emptyList()` in exactly this case, which is why the card reads the throwing `load()`.
     */
    @Test
    fun anUnreadableStoreIsNotAnAbsence() {
        val state = DogTagCard.state(
            owned = OwnedTagSource.Unreadable("keystore key invalidated"),
            imported = emptyList(),
        )

        assertTrue(state.rows.isEmpty())
        assertEquals("keystore key invalidated", state.ownerStoreUnavailable)
        assertFalse(
            "an unread store leaves absence unproven, so the card may not claim it",
            state.establishesNoTags,
        )
    }

    /** Nor may the first frame, before the off-main-thread read has landed, claim an absence. */
    @Test
    fun aPendingReadIsNotAnAbsence() {
        val state = DogTagCard.state(owned = OwnedTagSource.Pending, imported = emptyList())

        assertTrue(state.rows.isEmpty())
        assertTrue(state.ownerStorePending)
        assertNull(state.ownerStoreUnavailable)
        assertFalse(state.establishesNoTags)
    }

    /** Imported tags still render while the owner-secret store is unreadable - but not as complete. */
    @Test
    fun anUnreadableStoreStillListsWhatTheOtherSourceKnows() {
        val state = DogTagCard.state(
            owned = OwnedTagSource.Unreadable("corrupt"),
            imported = listOf(ImportedTag("42", "Bella")),
        )

        assertEquals(listOf("42"), state.rows.map { it.dogTagIdDec })
        assertEquals("corrupt", state.ownerStoreUnavailable)
        assertFalse(state.establishesNoTags)
    }

    /** The only state that licenses "No dog tag yet". */
    @Test
    fun anAnsweredEmptyStoreWithNoPetsEstablishesNoTags() {
        val state = DogTagCard.state(owned = records(), imported = emptyList())

        assertTrue(state.rows.isEmpty())
        assertNull(state.ownerStoreUnavailable)
        assertFalse(state.ownerStorePending)
        assertTrue(state.establishesNoTags)
    }

    /**
     * The underlying throwables are verbose (iOS's `DecodingError` renders ~400 characters), and a
     * settings card that pastes one buries the sentence that matters. The head is kept because it
     * names the file and the class of failure.
     */
    @Test
    fun aVerboseStoreFailureIsCollapsedToOnePrintableLine() {
        val raw = "dogtag-owner-secrets.json exists but could not be read;\n\trefusing to " +
            "overwrite it (it holds recovery secrets): " + "x".repeat(400)
        val state = DogTagCard.state(OwnedTagSource.Unreadable(raw), emptyList())
        val shown = state.ownerStoreUnavailable!!

        assertEquals(DogTagCard.MAX_REASON_CHARS + 1, shown.length) // + the ellipsis
        assertTrue("keeps the head, which names the file", shown.startsWith("dogtag-owner-secrets.json exists"))
        assertTrue(shown.endsWith("…"))
        assertFalse("newlines and tabs collapse to single spaces", shown.contains("\n") || shown.contains("\t"))
    }

    /** A reason that already fits is passed through unchanged apart from whitespace collapsing. */
    @Test
    fun aShortStoreFailureIsNotTruncated() {
        val state = DogTagCard.state(OwnedTagSource.Unreadable("  keystore key   invalidated "), emptyList())
        assertEquals("keystore key invalidated", state.ownerStoreUnavailable)
    }

    // ---- names -----------------------------------------------------------------------------------

    /**
     * `Pet.fromJson`/`fromCentral` default the name to the literal "Unnamed" and the on-import
     * fallback writes "DogTag #<id>". iOS `LocalStore.isRealName` rejects both; Android must too, or
     * the two platforms name the same tag differently.
     */
    @Test
    fun placeholderNamesAreNotNames() {
        assertNull(DogTagCard.realName("Unnamed"))
        assertNull(DogTagCard.realName("DogTag #7"))
        assertNull(DogTagCard.realName("DogTag#7"))
        assertNull(DogTagCard.realName("   "))
        assertNull(DogTagCard.realName(""))
        assertNull(DogTagCard.realName(null))
    }

    @Test
    fun aRealNameIsKeptAndTrimmed() {
        assertEquals("Rex", DogTagCard.realName("  Rex  "))
    }

    /** And the rejection reaches the row, not just the helper. */
    @Test
    fun aPlaceholderNamedPetContributesNoNameToItsRow() {
        val row = DogTagCard.state(records(owned("7")), listOf(ImportedTag("7", "Unnamed"))).rows.single()
        assertNull(row.name)
        assertTrue("the tag itself is still listed", row.credentialImported)
    }
}
