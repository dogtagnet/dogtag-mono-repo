package io.liberalize.dogtag.profile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/**
 * The whole owner-facing promise of M-2b, end to end: an Android owner builds a tag on-device, loses
 * the phone, and gets the SAME tag back from their 24 words plus the backup file.
 *
 * The pieces are each covered on their own - [ProfileTreeParityTest] proves the seed rebuilds the
 * same `R`, [OwnerSecretCodecTest] proves a record survives the JSON codec - but nothing joined them,
 * and the join is where the promise actually lives. Recovery only works if the value the codec
 * returns is the value the builder consumes: the canonical `dogTagIdHex` must go back through
 * [ProfileTreeBuilder.buildForIdField] (NOT [ProfileTreeBuilder.build], which would re-run the
 * decimal→field conversion and bind the rebuilt tree to a different id), the attribute salts must
 * come back byte-exact, and the type tags must survive the `UByte`↔`Int` hop. Any one of those
 * slipping rebuilds a well-formed but DIFFERENT root, and since `DogTagSBTConsent.profileRoot` is
 * write-once on-chain there is no repair - the owner's tag is simply unprovable forever.
 *
 * Runs the REAL Rust core over UniFFI (like [ProfileTreeParityTest]) and the REAL `org.json` codec
 * under Robolectric (like [OwnerSecretCodecTest]), so it is the two halves as the owner meets them,
 * not two stand-ins agreeing with each other.
 *
 * @see docs/MOBILE_OWNER_SECRET.md for the recovery contract this asserts
 */
@RunWith(RobolectricTestRunner::class)
class OwnerSecretRecoveryJourneyTest {

    private companion object {
        /** The owner's 24 words, as the 64-byte BIP-39 seed they expand to. TEST MATERIAL ONLY. */
        const val PHRASE_SEED = "DogTag demo wallet seed - TEST MATERIAL ONLY, never hold value"
        const val WRONG_PHRASE_SEED = "a stranger's 24 words - TEST MATERIAL ONLY, never hold value"

        const val OWNER_ADDRESS = "0x00000000000000000000000000000000deadbeef"

        /** A production-path decimal handle, so the tree binds through the canonical dogTagId field. */
        const val DOG_TAG_ID_DEC = "777001"

        val TAG_STRING: UByte = 2u

        fun hex(s: String): String =
            "0x" + s.toByteArray(Charsets.UTF_8).joinToString("") { "%02x".format(it) }

        /** Show enough of a secret to see two values agree, without printing the secret itself. */
        fun peek(hexValue: String): String = hexValue.take(12) + "…"
    }

    /**
     * The pet's disclosable leaves. Their salts are per-leaf random in production and are NOT
     * seed-derivable - which is exactly why the backup file has to exist at all, and why this
     * journey restores from the file rather than from the phrase alone.
     */
    private val attributes = listOf(
        ProfileTreeBuilder.Attribute("credentialSubject.name", "0x" + "2f".repeat(16), TAG_STRING, "Rex"),
        ProfileTreeBuilder.Attribute("credentialSubject.breedLabel", "0x" + "91".repeat(16), TAG_STRING, "Shiba Inu"),
        ProfileTreeBuilder.Attribute("credentialSubject.dateOfBirth", "0x" + "c4".repeat(16), TAG_STRING, "2021-04-02"),
    )

    @Test
    fun `a tag rebuilds from the phrase and the backup after the phone is gone`() {
        // ---- phone A: the owner is issued a tag -------------------------------------------------
        val onDevice = ProfileTreeBuilder.build(
            seedHex = hex(PHRASE_SEED),
            dogTagIdDec = DOG_TAG_ID_DEC,
            ownerAddress = OWNER_ADDRESS,
            attributes = attributes,
        )
        val dogTagIdHex = ProfileTreeBuilder.dogTagIdField(DOG_TAG_ID_DEC)

        // What the store persists. `R` is the ONLY field that ever leaves the device (the issuer
        // seals it as profileRoot); everything else here is owner-private.
        val backup = OwnerSecretRecords.encode(
            listOf(
                OwnerSecretRecord(
                    dogTagIdHex = dogTagIdHex,
                    dogTagIdDec = DOG_TAG_ID_DEC,
                    ownerSecretHex = onDevice.ownerSecretHex,
                    rootHex = onDevice.rootHex,
                    ownerAddress = OWNER_ADDRESS,
                    attributes = attributes.map {
                        BackedUpAttribute(it.keyPath, it.saltHex, it.tag, it.value)
                    },
                    derivationVersion = ProfileTreeStore.DERIVATION_VERSION,
                    savedAt = "2026-07-20T00:00:00Z",
                ),
            ),
        )

        // ---- phone B: nothing survives but the 24 words and the backup --------------------------
        val restored = OwnerSecretRecords.decode(backup).single()
        val rebuilt = ProfileTreeBuilder.buildForIdField(
            seedHex = hex(PHRASE_SEED),
            dogTagIdFieldHex = restored.dogTagIdHex,
            ownerAddress = restored.ownerAddress,
            attributes = restored.attributes.map {
                ProfileTreeBuilder.Attribute(it.keyPath, it.saltHex, it.tag, it.value)
            },
        )

        println("recovery journey | dogTagId              = $DOG_TAG_ID_DEC (field ${peek(dogTagIdHex)})")
        println("recovery journey | R on the lost phone   = ${onDevice.rootHex}")
        println("recovery journey | R on the new phone    = ${rebuilt.rootHex}")
        println("recovery journey | R sealed in the backup= ${restored.rootHex}")
        println("recovery journey | ownerSecret rebuilt   = ${peek(rebuilt.ownerSecretHex)} (matches stored: ${rebuilt.ownerSecretHex == restored.ownerSecretHex})")

        assertEquals("the recovered tag must be the SAME tag", onDevice.rootHex, rebuilt.rootHex)
        assertEquals("the backup's R must be what the device built", onDevice.rootHex, restored.rootHex)
        assertEquals(
            "the nullifier secret must regenerate from the phrase",
            restored.ownerSecretHex,
            rebuilt.ownerSecretHex,
        )
        assertEquals(onDevice.keyHashHex, rebuilt.keyHashHex)
    }

    /**
     * The negative leg. Without it, "recovery works" would still pass if the seed were being ignored
     * and the root were a pure function of the backup file - i.e. if the phrase, the one thing the
     * owner is told to guard, did nothing.
     */
    @Test
    fun `the backup file alone does not rebuild the tag`() {
        val onDevice = ProfileTreeBuilder.build(
            seedHex = hex(PHRASE_SEED),
            dogTagIdDec = DOG_TAG_ID_DEC,
            ownerAddress = OWNER_ADDRESS,
            attributes = attributes,
        )
        val restored = OwnerSecretRecords.decode(
            OwnerSecretRecords.encode(
                listOf(
                    OwnerSecretRecord(
                        dogTagIdHex = ProfileTreeBuilder.dogTagIdField(DOG_TAG_ID_DEC),
                        dogTagIdDec = DOG_TAG_ID_DEC,
                        ownerSecretHex = onDevice.ownerSecretHex,
                        rootHex = onDevice.rootHex,
                        ownerAddress = OWNER_ADDRESS,
                        attributes = attributes.map {
                            BackedUpAttribute(it.keyPath, it.saltHex, it.tag, it.value)
                        },
                        derivationVersion = ProfileTreeStore.DERIVATION_VERSION,
                        savedAt = "2026-07-20T00:00:00Z",
                    ),
                ),
            ),
        ).single()

        val withWrongPhrase = ProfileTreeBuilder.buildForIdField(
            seedHex = hex(WRONG_PHRASE_SEED),
            dogTagIdFieldHex = restored.dogTagIdHex,
            ownerAddress = restored.ownerAddress,
            attributes = restored.attributes.map {
                ProfileTreeBuilder.Attribute(it.keyPath, it.saltHex, it.tag, it.value)
            },
        )

        assertNotEquals(onDevice.rootHex, withWrongPhrase.rootHex)
        assertNotEquals(onDevice.ownerSecretHex, withWrongPhrase.ownerSecretHex)
    }
}
