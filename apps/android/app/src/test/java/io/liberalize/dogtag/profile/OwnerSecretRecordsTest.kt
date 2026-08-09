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
        deployment: DeploymentScope? = null,
    ) = OwnerSecretRecord(
        dogTagIdHex = idHex,
        dogTagIdDec = idDec,
        ownerSecretHex = secret,
        rootHex = root,
        ownerAddress = "0x00000000000000000000000000000000deadbeef",
        attributes = listOf(BackedUpAttribute("credentialSubject.name", "0x0102", 2u, "Rex")),
        derivationVersion = ProfileTreeStore.DERIVATION_VERSION,
        savedAt = "2026-07-20T00:00:00Z",
        deployment = deployment,
    )

    /** The deployment this suite's scoped fixtures live on, and a second one a redeploy creates. */
    private val depA = DeploymentScope(135, "0x00000000000000000000000000000000000000dd")
    private val depB = DeploymentScope(135, "0x00000000000000000000000000000000000000d1")

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

    // ---- deployment scoping (mirrors iOS OwnerSecretScopingTests case for case) ----------------

    /**
     * THE REDEPLOY CASE: the same decimal id on a NEW deployment is a NEW tag. The two records
     * coexist — refusing here is how one redeploy used to poison every low id on a handset, and
     * overwriting would destroy the old tag's unrecoverable salts.
     */
    @Test
    fun `the same id on a different deployment is a different tag and both records are kept`() {
        val old = OwnerSecretRecords.upsert(emptyList(), record(root = "0xaaaa", deployment = depA))
        val both = OwnerSecretRecords.upsert(old, record(root = "0xdead", deployment = depB))
        assertEquals(2, both.size)
    }

    /** A legacy record is "deployment unknown", which is NOT equal to any deployment: a scoped
     * record with a different root coexists with it rather than tripping the write-once check. */
    @Test
    fun `a legacy record does not collide with a scoped record for the same id`() {
        val legacy = OwnerSecretRecords.upsert(emptyList(), record(root = "0xaaaa"))
        val both = OwnerSecretRecords.upsert(legacy, record(root = "0xdead", deployment = depA))
        assertEquals(2, both.size)
        assertNull(both[0].deployment)
        assertEquals(depA, both[1].deployment)
    }

    /** The write-once invariant is unchanged WITHIN a deployment. */
    @Test
    fun `a different root for a known id on the SAME deployment is still refused`() {
        val existing = OwnerSecretRecords.upsert(emptyList(), record(deployment = depA))
        try {
            OwnerSecretRecords.upsert(existing, record(root = "0xdead", deployment = depA))
            fail("a conflicting root within one deployment must be refused")
        } catch (e: OwnerSecretRecords.ConflictingRootException) {
            assertEquals("0xaaaa", e.existing)
        }
    }

    /**
     * MIGRATION: upserting a scoped record whose root equals a legacy record's root REPLACES the
     * legacy entry — the record gains its deployment instead of duplicating the tag.
     */
    @Test
    fun `a scoped upsert with the legacy record's own root stamps it rather than duplicating`() {
        val legacy = OwnerSecretRecords.upsert(emptyList(), record(root = "0xaaaa"))
        val stamped = OwnerSecretRecords.upsert(legacy, record(root = "0xaaaa", deployment = depA))
        assertEquals(1, stamped.size)
        assertEquals(depA, stamped[0].deployment)
    }

    /** Selection: an exact scope match wins; a foreign record NEVER answers; legacy answers when
     * nothing scoped does. */
    @Test
    fun `preferredFor returns the current deployment's record then legacy and never a foreign one`() {
        val records = listOf(
            record(root = "0x01", deployment = depB), // foreign
            record(root = "0x02"), // legacy
            record(root = "0x03", deployment = depA), // current
        )
        assertEquals("0x03", OwnerSecretRecords.preferredFor(records, "424242", depA)?.rootHex)
        val withoutCurrent = records.dropLast(1)
        assertEquals("0x02", OwnerSecretRecords.preferredFor(withoutCurrent, "424242", depA)?.rootHex)
        val foreignOnly = records.take(1)
        assertNull(OwnerSecretRecords.preferredFor(foreignOnly, "424242", depA))
    }

    /** With NO way to establish the current deployment, only a legacy record answers — a scoped
     * record must not be assumed to belong to a deployment the app cannot name. */
    @Test
    fun `preferredFor with an unknown current deployment returns only legacy records`() {
        val records = listOf(record(root = "0x01", deployment = depA), record(root = "0x02"))
        assertEquals("0x02", OwnerSecretRecords.preferredFor(records, "424242", null)?.rootHex)
        assertNull(OwnerSecretRecords.preferredFor(records.take(1), "424242", null))
    }

    /** The bind flow's decision table, pinned arm by arm. */
    @Test
    fun `reuseDecision reuses on content match and frees a legacy id only when the deployment is known`() {
        // Byte-identical content = the same session: reuse, whatever the record's scope.
        assertEquals(
            OwnerSecretRecords.ReuseDecision.REUSE,
            OwnerSecretRecords.reuseDecision(null, depA, contentMatches = true),
        )
        assertEquals(
            OwnerSecretRecords.ReuseDecision.REUSE,
            OwnerSecretRecords.reuseDecision(depA, depA, contentMatches = true),
        )
        // Legacy + different content + a KNOWN current deployment: another deployment's tag —
        // build fresh instead of refusing the owner forever.
        assertEquals(
            OwnerSecretRecords.ReuseDecision.BUILD_FRESH,
            OwnerSecretRecords.reuseDecision(null, depA, contentMatches = false),
        )
        // Same deployment, different content: a real conflict.
        assertEquals(
            OwnerSecretRecords.ReuseDecision.REFUSE_CONFLICT,
            OwnerSecretRecords.reuseDecision(depA, depA, contentMatches = false),
        )
        // No way to tell deployments apart: fail closed, exactly as before scoping existed.
        assertEquals(
            OwnerSecretRecords.ReuseDecision.REFUSE_CONFLICT,
            OwnerSecretRecords.reuseDecision(null, null, contentMatches = false),
        )
    }

    /**
     * THE CAPTAIN'S AFTERNOON, AS A SEQUENCE: a tag issued before scoping existed (a legacy
     * record), then a redeploy, then the vet hands out the same low id for a DIFFERENT pet. The
     * old rule refused forever; the new rules build fresh, keep the old record beside the new one,
     * and each deployment's lookup finds its own tag.
     */
    @Test
    fun `a redeploy treats the same id as a new tag while the old record survives`() {
        // Issued pre-scoping: tag 1, pet A.
        val store0 = OwnerSecretRecords.upsert(emptyList(), record(idDec = "1", root = "0x0a"))

        // After the redeploy the app runs against depB and the vet allocates tag 1 to pet B.
        // Different content on a legacy record with a known deployment: build fresh, not refuse.
        val decision = OwnerSecretRecords.reuseDecision(
            store0[0].deployment,
            depB,
            contentMatches = false,
        )
        assertEquals(OwnerSecretRecords.ReuseDecision.BUILD_FRESH, decision)

        // The fresh witness persists beside the old record - nothing is lost, nothing collides.
        val store1 = OwnerSecretRecords.upsert(store0, record(idDec = "1", root = "0x0b", deployment = depB))
        assertEquals(2, store1.size)

        // The new deployment's lookup answers with the new tag; the old record still exists (its
        // salts live nowhere else) and still answers for a phone that never learned a deployment.
        assertEquals("0x0b", OwnerSecretRecords.preferredFor(store1, "1", depB)?.rootHex)
        assertEquals("0x0a", OwnerSecretRecords.preferredFor(store1, "1", null)?.rootHex)
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
