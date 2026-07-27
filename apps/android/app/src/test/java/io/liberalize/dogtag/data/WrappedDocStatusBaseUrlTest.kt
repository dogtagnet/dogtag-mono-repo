package io.liberalize.dogtag.data

import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/**
 * The receipt QR's base URL.
 *
 * AUDIT FINDING (2026-07-27): the travel-receipt QR encoded `https://gov.example/r/<id>` — NXDOMAIN —
 * because it was built from `issuer.domain`. That field is a `did:web` IDENTITY, a stable name that
 * need not resolve and need not serve anything; the shipped default is RFC-2606 reserved. The QR read
 * as a working live-status check a border official could scan, and led nowhere.
 *
 * The reachable base now travels in `protocol.statusBaseUrl`, stamped at issuance from the issuer's
 * `DEPLOYMENT_URL`. These pin that the accessor reads that field and ONLY that field.
 */
@RunWith(RobolectricTestRunner::class)
class WrappedDocStatusBaseUrlTest {

    private fun doc(protocolBlock: String?): WrappedDoc {
        val protocol = protocolBlock?.let { ""","protocol":$it""" } ?: ""
        return WrappedDoc(
            """{"version":"dogtag/1.0","data":{},
               "signature":{"type":"DogTagMerkleProof","targetHash":"0x00","proof":[],"merkleRoot":"0x00"},
               "privacy":{"obfuscated":[]},
               "issuer":{"documentStore":"0xabc","name":"Gov","domain":"gov.example","recordType":"TRAVEL_CLEARANCE"}
               $protocol}""",
        )
    }

    @Test
    fun readsTheStampedReachableBase() {
        val d = doc("""{"statusBaseUrl":"https://receipts.example.org"}""")
        assertEquals("https://receipts.example.org", d.statusBaseUrl)
    }

    /** A trailing slash would mint `//r/<id>`; trimmed at the source so all three clients agree. */
    @Test
    fun trimsATrailingSlashSoTheMintedUrlIsNeverDoubleSlashed() {
        val d = doc("""{"statusBaseUrl":"https://receipts.example.org/"}""")
        assertEquals("https://receipts.example.org", d.statusBaseUrl)
    }

    /**
     * THE regression: a pre-stamping document has no base, and the identity domain must NOT stand in
     * for one. Blank is what makes the receipt card say "no public status page" instead of printing a
     * dead link — degrading honestly rather than plausibly.
     */
    @Test
    fun aDocumentWithNoProtocolBlockHasNoBaseAndNeverFallsBackToTheIdentityDomain() {
        val d = doc(null)
        assertEquals("", d.statusBaseUrl)
        assertEquals("gov.example", d.issuerDomain) // the identity is still there, just not the base
    }

    /** A `protocol` block from before the field existed is the same story. */
    @Test
    fun aProtocolBlockWithoutTheFieldAlsoHasNoBase() {
        val d = doc("""{"chainId":135,"version":"dogtag-levelb/1","issuerClone":"0xabc"}""")
        assertEquals("", d.statusBaseUrl)
    }
}
