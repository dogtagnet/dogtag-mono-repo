package io.liberalize.dogtag.profile

import io.liberalize.dogtag.wallet.SeedBackup
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * The owner-secret store's pure logic: the write-once-root invariant and the seed-backup
 * fingerprint.
 *
 * `ProfileTreeStore` itself needs a `Context` and the Android Keystore, so the parts worth pinning
 * were kept `Context`-free ([OwnerSecretRecords], [SeedBackup.fingerprint]) precisely so they could
 * be covered here without an emulator. The Keystore envelope and file placement are exercised on
 * device.
 *
 * Pure JVM; no Rust core and no Android runtime needed.
 */
class OwnerSecretRecordsTest {

    private fun record(
        idDec: String = "424242",
        idHex: String = "0x67932",
        root: String = "0xaaaa",
        secret: String = "0xbbbb",
    ) = OwnerSecretRecord(
        dogTagIdHex = idHex,
        dogTagIdDec = idDec,
        ownerSecretHex = secret,
        rootHex = root,
        ownerAddress = "0x00000000000000000000000000000000deadbeef",
        attributes = listOf(BackedUpAttribute("credentialSubject.name", "0x0102", 2u, "Rex")),
        derivationVersion = ProfileTreeStore.DERIVATION_VERSION,
        savedAt = "2026-07-20T00:00:00Z",
    )

    @Test
    fun `a new tag is appended`() {
        val out = OwnerSecretRecords.upsert(emptyList(), record())
        assertEquals(1, out.size)
        assertEquals("424242", out[0].dogTagIdDec)
    }

    /**
     * Re-persisting the SAME root is an idempotent retry - the issuance flow can be resumed after a
     * network failure without the store rejecting it.
     */
    @Test
    fun `the same root replaces the record idempotently`() {
        val first = OwnerSecretRecords.upsert(emptyList(), record())
        val again = OwnerSecretRecords.upsert(first, record(secret = "0xcccc"))
        assertEquals(1, again.size)
        assertEquals("0xcccc", again[0].ownerSecretHex)
    }

    /**
     * The write-once invariant. `DogTagSBTConsent.profileRoot` can never be moved once sealed, so a
     * store that accepted a DIFFERENT root for a known id would strand the tag: the witness that
     * rebuilds the sealed root would be gone, and the attribute salts it dropped are not
     * seed-derivable and exist nowhere else on the device.
     */
    @Test
    fun `a different root for a known id is refused`() {
        val existing = OwnerSecretRecords.upsert(emptyList(), record())
        try {
            OwnerSecretRecords.upsert(existing, record(root = "0xdead"))
            fail("a conflicting root must be refused")
        } catch (e: OwnerSecretRecords.ConflictingRootException) {
            assertEquals("0xaaaa", e.existing)
            assertEquals("0xdead", e.proposed)
        }
        // The original witness is untouched.
        assertEquals("0xaaaa", existing[0].rootHex)
    }

    /** Ids are matched case-insensitively, so hex casing cannot smuggle in a duplicate record. */
    @Test
    fun `the id match ignores hex case`() {
        val existing = OwnerSecretRecords.upsert(emptyList(), record(idHex = "0xAB12"))
        val out = OwnerSecretRecords.upsert(existing, record(idHex = "0xab12"))
        assertEquals(1, out.size)
    }

    /** Distinct tags coexist - one wallet holds several. */
    @Test
    fun `distinct tags are kept side by side`() {
        val a = OwnerSecretRecords.upsert(emptyList(), record())
        val b = OwnerSecretRecords.upsert(a, record(idDec = "515151", idHex = "0x7dd57", root = "0xbeef"))
        assertEquals(2, b.size)
    }

    // ---- seed-backup fingerprint ---------------------------------------------------------------

    /**
     * The confirmation is bound to the seed, so a preferences backup restored onto a new phone
     * cannot silently confirm a DIFFERENT wallet whose seed never migrated - which would re-open the
     * exact silent-loss hole the gate closes.
     */
    @Test
    fun `the seed backup fingerprint is bound to the seed`() {
        val a = SeedBackup.fingerprint("0x00112233")
        val b = SeedBackup.fingerprint("0x00112234")
        assertNotEquals(a, b)
        // Stable, and indifferent to the 0x prefix.
        assertEquals(a, SeedBackup.fingerprint("00112233"))
        assertTrue(a!!.length == 64)
    }

    /** Malformed hex yields no fingerprint, so `isConfirmed` fails safe rather than matching null. */
    @Test
    fun `malformed seed hex has no fingerprint`() {
        assertNull(SeedBackup.fingerprint(""))
        assertNull(SeedBackup.fingerprint("0x1"))
        assertNull(SeedBackup.fingerprint("0xzz"))
    }
}
