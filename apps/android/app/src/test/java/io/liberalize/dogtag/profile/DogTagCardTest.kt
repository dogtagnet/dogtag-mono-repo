package io.liberalize.dogtag.profile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
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
        assertEquals(OwnerSecretEvidence.Held, row.ownerSecret)
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
        assertEquals(OwnerSecretEvidence.Held, row.ownerSecret)
        assertTrue(row.credentialImported)
    }

    /** An imported pet with no owner-secret is still the owner's tag and stays listed. */
    @Test
    fun anImportedTagWithNoOwnerSecretIsStillListed() {
        val row = DogTagCard.state(records(), listOf(ImportedTag("42", "Bella"))).rows.single()

        assertEquals("42", row.dogTagIdDec)
        assertEquals("Bella", row.name)
        assertNull("no owner-secret record means no known root", row.rootHex)
        assertEquals(
            "the store answered, so this absence IS established",
            OwnerSecretEvidence.NotHeld,
            row.ownerSecret,
        )
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

    /**
     * A handle is ASCII `0`-`9`, and both platforms must read it that way or one identical store
     * lists different rows on each. `Char::isDigit` is Unicode Nd, so `٣` (U+0663) used to pass HERE
     * as well as on iOS; `\.isNumber` adds Nl and No, so `½` used to pass on iOS ONLY. Both are now
     * rejected on both sides. Mirrored by iOS `testANonAsciiDigitIsNotAHandle`.
     */
    @Test
    fun aNonAsciiDigitIsNotAHandle() {
        val state = DogTagCard.state(
            owned = records(),
            imported = listOf(
                ImportedTag("٣", "Arabic-Indic three"), // Nd - both platforms once accepted it
                ImportedTag("½", "Vulgar half"), // No - iOS once accepted it, Android did not
                ImportedTag("1٣", "Mixed"),
            ),
        )
        assertTrue(state.rows.isEmpty())
    }

    // ---- "could not check" is not "none" ---------------------------------------------------------

    /**
     * An unreadable store must NOT read as an empty one. `ProfileTreeStore.all()` answers
     * `emptyList()` in exactly this case, which is why the card reads the throwing `load()`.
     */
    @Test
    fun anUnreadableStoreIsNotAnAbsence() {
        val state = DogTagCard.state(
            owned = OwnedTagSource.Unreadable(OwnerStoreFailure.CouldNotRead),
            imported = emptyList(),
        )

        assertTrue(state.rows.isEmpty())
        assertEquals(
            DogTagCard.reasonText(OwnerStoreFailure.CouldNotRead),
            state.ownerStoreUnavailable,
        )
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
            owned = OwnedTagSource.Unreadable(OwnerStoreFailure.CouldNotDecode),
            imported = listOf(ImportedTag("42", "Bella")),
        )

        assertEquals(listOf("42"), state.rows.map { it.dogTagIdDec })
        assertEquals(
            DogTagCard.reasonText(OwnerStoreFailure.CouldNotDecode),
            state.ownerStoreUnavailable,
        )
        assertFalse(state.establishesNoTags)
    }

    /**
     * ...and the rows it lists claim NOTHING about the owner-secret. `own` is null for every row
     * when the store did not answer, so a boolean "held" would render the definite negative "this
     * phone holds no owner-secret for this tag" - could-not-check dressed as a fact, which is this
     * card's own defect one level down. Collapsing [OwnerSecretEvidence] back to a Bool reddens this.
     */
    @Test
    fun anUnreadableStoreClaimsNothingAboutARowsOwnerSecret() {
        val row = DogTagCard.state(
            owned = OwnedTagSource.Unreadable(OwnerStoreFailure.CouldNotRead),
            imported = listOf(ImportedTag("42", "Bella")),
        ).rows.single()

        assertEquals(OwnerSecretEvidence.Unknown, row.ownerSecret)
        assertNull("nor may a missing root imply anything either", row.rootHex)
    }

    /** Same for the frame before the read lands: unasked is not answered. */
    @Test
    fun aPendingReadClaimsNothingAboutARowsOwnerSecret() {
        val row = DogTagCard.state(
            owned = OwnedTagSource.Pending,
            imported = listOf(ImportedTag("42", "Bella")),
        ).rows.single()

        assertEquals(OwnerSecretEvidence.Unknown, row.ownerSecret)
        assertNull(row.rootHex)
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
     * THE privacy property, and the reason [OwnedTagSource.Unreadable] carries a cause rather than a
     * string: by the time a decode fails the decryption has SUCCEEDED, and `org.json` quotes the
     * input it choked on - here the owner-secret store's own plaintext. Every cause must render a
     * sentence the code wrote.
     *
     * Note what actually enforces this: the payload's TYPE. There is no raw-text case to assert
     * against because a caller cannot express one, which is the guarantee - a test could only ever
     * check the causes that exist. This walks all of them and pins that each says what is missing,
     * so a future cause cannot be added as silence either.
     */
    @Test
    fun everyStoreFailureRendersAConstructedSentence() {
        OwnerStoreFailure.entries.forEach { cause ->
            val shown = DogTagCard.state(OwnedTagSource.Unreadable(cause), emptyList())
                .ownerStoreUnavailable

            assertEquals(DogTagCard.reasonText(cause), shown)
            assertTrue(
                "$cause must still state the consequence, or absence returns as silence",
                shown!!.contains("missing from this list"),
            )
        }
    }

    /**
     * The two causes are distinguishable to the reader - one is "the bytes never came back", the
     * other "they came back and did not decode", and they have different remedies. Folding them
     * into one message would make the card's only diagnostic useless.
     */
    @Test
    fun theTwoStoreFailuresDoNotReadAlike() {
        assertNotEquals(
            DogTagCard.reasonText(OwnerStoreFailure.CouldNotRead),
            DogTagCard.reasonText(OwnerStoreFailure.CouldNotDecode),
        )
    }

    /**
     * The residual cap. Nothing variable reaches it now that the sentences are constructed, so this
     * pins the helper directly: whatever a future cause's wording grows into, the card gets one
     * printable line rather than a paragraph burying the part that matters.
     */
    @Test
    fun aVerboseStoreMessageIsCollapsedToOnePrintableLine() {
        val shown = DogTagCard.shortReason(
            "Could not read this device's owner-secret store;\n\tany tag created by " +
                "issuance is missing from this list. " + "x".repeat(400),
        )

        assertEquals(DogTagCard.MAX_REASON_CHARS + 1, shown.length) // + the ellipsis
        assertTrue("keeps the head, which names the failure", shown.startsWith("Could not read this device's"))
        assertTrue(shown.endsWith("…"))
        assertFalse("newlines and tabs collapse to single spaces", shown.contains("\n") || shown.contains("\t"))
    }

    /** A message that already fits is passed through unchanged apart from whitespace collapsing. */
    @Test
    fun aShortStoreMessageIsNotTruncated() {
        assertEquals("keystore key invalidated", DogTagCard.shortReason("  keystore key   invalidated "))
    }

    /** And every constructed sentence is already inside the cap, so none of them is ever elided. */
    @Test
    fun noConstructedSentenceIsTruncated() {
        OwnerStoreFailure.entries.forEach { cause ->
            assertFalse(
                "$cause is too long for the card and would be cut mid-remedy",
                DogTagCard.state(OwnedTagSource.Unreadable(cause), emptyList())
                    .ownerStoreUnavailable!!.endsWith("…"),
            )
        }
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
