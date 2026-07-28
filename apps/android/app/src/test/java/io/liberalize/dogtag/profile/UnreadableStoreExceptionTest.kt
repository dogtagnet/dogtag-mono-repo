package io.liberalize.dogtag.profile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What [ProfileTreeStore.UnreadableStoreException]'s MESSAGE is allowed to contain.
 *
 * This guards the source rather than one screen, because the message has several renderers and the
 * property has to hold for all of them: `ScanScreen` builds user-facing text with `${e.message}` on
 * the issuance catch, the verify catch and the record-picker catch, none of which caps or sanitizes
 * what it interpolates. The Dog-tags card does not read the message at all - it renders a sentence
 * constructed from [ProfileTreeStore.UnreadableStoreException.Kind] - so a test written against the
 * card would prove nothing about the paths that actually leaked.
 *
 * What leaks, concretely: by the time a DECODE fails the decryption has already succeeded, so
 * `org.json` is handed the store's own plaintext and quotes it back - `JSONTokener.syntaxError`
 * appends the whole tokenizer input - putting `ownerSecretHex` into any string built from the
 * message. The store is the one file whose contents must never leave the device.
 */
class UnreadableStoreExceptionTest {

    /** A cause message shaped like what `org.json` hands over: the store's decrypted contents. */
    private val leakedPlaintext =
        """Unterminated object at character 87 of [{"dogTagIdDec":"7","ownerSecretHex":""" +
            """"0xdeadbeefcafebabe","rootHex":"0xabc","attributes":[{"saltHex":"00112233"}]}]"""

    private fun thrown(kind: ProfileTreeStore.UnreadableStoreException.Kind) =
        ProfileTreeStore.UnreadableStoreException(IllegalStateException(leakedPlaintext), kind)

    /**
     * THE regression. Reinstating `${'$'}{cause.message}` in the constructor's message reddens this.
     */
    @Test
    fun theMessageNeverQuotesTheCause() {
        ProfileTreeStore.UnreadableStoreException.Kind.entries.forEach { kind ->
            val message = thrown(kind).message.orEmpty()

            assertFalse(
                "$kind quoted the cause, so a screen interpolating e.message renders the store",
                message.contains(leakedPlaintext),
            )
            listOf("ownerSecretHex", "0xdeadbeefcafebabe", "saltHex", "Unterminated").forEach {
                assertFalse("$kind leaked \"$it\"", message.contains(it))
            }
        }
    }

    /**
     * The message still has to be worth rendering: it names the file, states the refusal that
     * protects the salts, and says which step failed. Only the untrusted cause text was the problem,
     * so a fix that emptied the message would trade a leak for a useless diagnostic.
     */
    @Test
    fun theMessageStillCarriesItsOperationalSignal() {
        val message = thrown(ProfileTreeStore.UnreadableStoreException.Kind.CouldNotDecode)
            .message.orEmpty()

        assertTrue("names the file", message.contains("dogtag-owner-secrets.json"))
        assertTrue("states the refusal", message.contains("refusing to overwrite it"))
        assertTrue(
            "names the failing step",
            message.contains(ProfileTreeStore.UnreadableStoreException.Kind.CouldNotDecode.detail),
        )
    }

    /** And the two steps are distinguishable, or the kind buys the message nothing. */
    @Test
    fun theTwoStepsDoNotReadAlike() {
        assertNotEquals(
            thrown(ProfileTreeStore.UnreadableStoreException.Kind.CouldNotRead).message,
            thrown(ProfileTreeStore.UnreadableStoreException.Kind.CouldNotDecode).message,
        )
    }

    /**
     * Debuggability is not what was traded away: the raw throwable stays attached as the cause, so
     * stack traces and the logcat line still carry it. Only the RENDERED message was narrowed.
     */
    @Test
    fun theRawCauseIsStillAttached() {
        val cause = IllegalStateException(leakedPlaintext)
        assertSame(cause, ProfileTreeStore.UnreadableStoreException(cause).cause)
    }

    /** An unnamed step is the read half - the conservative default for a partially-wired caller. */
    @Test
    fun theDefaultKindIsTheReadHalf() {
        assertEquals(
            ProfileTreeStore.UnreadableStoreException.Kind.CouldNotRead,
            ProfileTreeStore.UnreadableStoreException(IllegalStateException("boom")).kind,
        )
    }
}
