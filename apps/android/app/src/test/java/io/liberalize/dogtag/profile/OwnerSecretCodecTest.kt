package io.liberalize.dogtag.profile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/**
 * The owner-secret store's JSON codec.
 *
 * This is the one part of the store that is easy to get quietly wrong - a mistyped field name or the
 * `UByte`↔`Int` hop on an attribute's type tag would encode fine and decode into a DIFFERENT witness,
 * which rebuilds a different `R`. Because `profileRoot` is write-once on-chain, that is not a bug the
 * owner can recover from: their tag simply stops being provable. So the round-trip is asserted rather
 * than assumed.
 *
 * Robolectric is required for the same reason `QrPayloadTest` needs it: on the unit-test classpath
 * `org.json` is Android's stub, whose every method throws "not mocked". The codec runs against the
 * REAL `org.json` the app ships with, not a stand-in.
 *
 * The Keystore envelope and the `noBackupFilesDir` placement around this codec are genuinely
 * device-side and are NOT covered here; they need an instrumented test.
 */
@RunWith(RobolectricTestRunner::class)
class OwnerSecretCodecTest {

    private fun record(
        idDec: String = "424242",
        idHex: String = "0x67932",
        root: String = "0xaaaa",
    ) = OwnerSecretRecord(
        dogTagIdHex = idHex,
        dogTagIdDec = idDec,
        ownerSecretHex = "0xbbbb",
        rootHex = root,
        ownerAddress = "0x00000000000000000000000000000000deadbeef",
        attributes = listOf(
            BackedUpAttribute("credentialSubject.name", "0x0102", 2u, "Rex"),
            // typeTag 5 is `Bytes` - the reserved tag, and the highest value in use. It is the one
            // most likely to be mangled by a sloppy Int conversion, so it belongs in the fixture.
            BackedUpAttribute("credentialSubject.photo", "0x0304", 5u, "0xdeadbeef"),
        ),
        derivationVersion = ProfileTreeStore.DERIVATION_VERSION,
        savedAt = "2026-07-20T00:00:00Z",
    )

    /**
     * The whole record must survive a write/read cycle. `OwnerSecretRecord` is a data class, so this
     * single equality covers every field - including any added later, which is the point: a new field
     * that the codec forgets to write fails here automatically.
     */
    @Test
    fun `encode then decode round-trips every field`() {
        val records = listOf(record(), record(idDec = "515151", idHex = "0x7dd57", root = "0xbeef"))
        assertEquals(records, OwnerSecretRecords.decode(OwnerSecretRecords.encode(records)))
    }

    @Test
    fun `an empty store round-trips`() {
        assertEquals(emptyList<OwnerSecretRecord>(), OwnerSecretRecords.decode(OwnerSecretRecords.encode(emptyList())))
    }

    /**
     * The type tag specifically. It is the only numeric field, and it crosses `UByte`→`Int`→`UByte`;
     * the round-trip above would still pass if BOTH directions were wrong in the same way, so assert
     * the decoded value against the literal.
     */
    @Test
    fun `attribute type tags survive the UByte conversion`() {
        val decoded = OwnerSecretRecords.decode(OwnerSecretRecords.encode(listOf(record())))
        assertEquals(listOf<UByte>(2u, 5u), decoded[0].attributes.map { it.tag })
    }

    /**
     * A malformed payload must THROW, never decode to an empty list. `ProfileTreeStore.load` turns
     * this into `UnreadableStoreException` precisely so a corrupt store cannot be reported as "no
     * records" and then overwritten by the next upsert - which would destroy attribute salts that are
     * not seed-derivable and exist nowhere else on the device.
     */
    @Test
    fun `a malformed payload throws rather than decoding to empty`() {
        assertThrows(Exception::class.java) { OwnerSecretRecords.decode("not json") }
        assertThrows(Exception::class.java) { OwnerSecretRecords.decode("[{\"dogTagIdHex\":\"0x1\"}]") }
    }

    /** Sanity: the encoding is not lossy in a way that collapses two distinct tags into one. */
    @Test
    fun `distinct records stay distinct through the codec`() {
        val decoded = OwnerSecretRecords.decode(
            OwnerSecretRecords.encode(listOf(record(), record(idDec = "515151", idHex = "0x7dd57", root = "0xbeef"))),
        )
        assertEquals(2, decoded.size)
        assertNotEquals(decoded[0].dogTagIdHex, decoded[1].dogTagIdHex)
    }
}
