package io.liberalize.dogtag.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.time.Duration
import java.time.Instant

/**
 * Where the badge's expiry actually comes from.
 *
 * `VerdictBadgeStalenessTest` pins the RULE with `validUntil` handed in. This pins the other half: that
 * a stored credential can find its own `validity.validUntil` in the document it already holds.
 *
 * It is DERIVED, never persisted beside the verdict, and that is load-bearing. A stored copy would be
 * absent on every record imported before this shipped — so the expiry rule would silently not apply to
 * exactly the oldest records, the ones most likely to have lapsed — and it would be a second source of
 * truth able to drift from the Merkle-covered leaf that is the only tamper-evident one.
 *
 * Robolectric because the extraction goes through `org.json`.
 */
@RunWith(RobolectricTestRunner::class)
class CredentialExpiryFromDocTest {

    private val now: Instant = Instant.parse("2026-07-28T12:00:00Z")

    /** Leaves inside `data` are packed `"<salt>:<tag>:<value>"`. */
    private fun docJson(validityBlock: String): String =
        """{"version":"dogtag/1.0",
           "data":{"credentialSubject":{"dogTagId":"aa:2:42"$validityBlock}},
           "signature":{"type":"DogTagMerkleProof","targetHash":"0x11","proof":[],"merkleRoot":"0x11"},
           "privacy":{"obfuscated":[]},
           "issuer":{"documentStore":"0xabc","name":"Gov","domain":"gov.example","recordType":"TRAVEL_CLEARANCE"}}"""

    private fun credential(validityBlock: String, verdict: String = "VALID") = Credential(
        id = "rec-1",
        dogTagId = "42",
        group = CredentialGroup.Travel,
        recordType = "TRAVEL_CLEARANCE",
        title = "Travel Clearance",
        subtitle = "TRAVEL_CLEARANCE",
        issuer = "DogTag Government Authority",
        issuedOn = "",
        credentialRoot = "0x11",
        verdict = verdict,
        wrappedDocJson = docJson(validityBlock),
        lastCheckedAt = now.minus(Duration.ofMinutes(1)).toString(),
        verdictReason = "anchored on ROAX and not revoked",
    )

    private val lapsed = ""","validity":{"issuedOn":"bb:2:2026-01-01","validUntil":"cc:2:2026-06-30"}"""
    private val current = ""","validity":{"issuedOn":"bb:2:2026-01-01","validUntil":"cc:2:2027-06-30"}"""

    /** EU_HEALTH_CERT has NO `validity` block: its window is the flat Annex-IV `rabiesValidUntil`. */
    private val rabiesLapsed = ""","rabiesValidUntil":"dd:2:2026-06-30""""
    private val rabiesCurrent = ""","rabiesValidUntil":"dd:2:2027-06-30""""

    /** The packed leaf is unwrapped to its bare value, exactly as the receipt sheet reads it. */
    @Test
    fun readsThePackedValidUntilLeaf() {
        assertEquals("2026-06-30", WrappedDoc(docJson(lapsed)).validUntil)
    }

    /** A document with no validity window makes no expiry claim — blank, never "expired". */
    @Test
    fun aDocumentWithNoValidityBlockMakesNoExpiryClaim() {
        assertEquals("", WrappedDoc(docJson("")).validUntil)
        assertNull(credential("").validUntil)
    }

    /**
     * THE point of deriving it. This credential carries no expiry column of its own — it is a record as
     * stored before any of this shipped — and the badge still finds the lapse.
     *
     * Mutation check: this is the case that goes red if `Credential.validUntil` is changed to a
     * persisted constructor field, because a pre-existing record has nothing to persist.
     */
    @Test
    fun anAlreadyStoredRecordStillExpires() {
        val cred = credential(lapsed)
        assertEquals("2026-06-30", cred.validUntil)
        assertEquals("EXPIRED", cred.badge(now).label)
        assertEquals(VerdictDisplay.Tone.WARNING, cred.badge(now).tone)
    }

    /** …and one still inside its window is untouched: the rule only ever fires on a real lapse. */
    @Test
    fun aCredentialInsideItsWindowIsUnaffected() {
        assertEquals("VALID", credential(current).badge(now).label)
    }

    /**
     * The sub-line says WHEN, because "EXPIRED" alone is the first thing an owner asks about.
     *
     * The freshness half is asserted by shape, not by text: `lastCheckedLabel` renders against the real
     * wall clock (#87's behaviour, unchanged here), so pinning its exact words would make this test
     * flake on the minute boundary.
     */
    @Test
    fun theStatusLineNamesTheExpiryDate() {
        val line = credential(lapsed).statusLine(now)
        assertTrue(line, line.startsWith("Checked "))
        assertTrue(line, line.endsWith(" · expired 2026-06-30"))
    }

    /** …and says nothing about expiry while the window is still open. */
    @Test
    fun theStatusLineOmitsExpiryWhileTheWindowIsOpen() {
        assertFalse(credential(current).statusLine(now).contains("expired"))
    }

    /** The reason still travels with a non-VALID verdict, alongside the expiry. #87's line, intact. */
    @Test
    fun theStatusLineStillCarriesTheVerdictReason() {
        val cred = credential(lapsed, verdict = "UNVERIFIED")
            .copy(verdictReason = "could not reach the chain (rpc 502)")
        assertEquals(
            "expired 2026-06-30 · could not reach the chain (rpc 502)",
            cred.statusLine(now).substringAfter(" · "),
        )
    }

    /** A record whose document cannot be parsed has no expiry claim, and must not blow up a list
     *  render trying to find one. */
    @Test
    fun anUnparseableDocumentYieldsNoExpiryClaimRatherThanThrowing() {
        val broken = credential(lapsed).copy(wrappedDocJson = "{not json")
        assertNull(broken.validUntil)
        assertEquals("VALID", broken.badge(now).label)
    }

    /**
     * EU_HEALTH_CERT carries NO `validity` block at all — the government API issues its window as the
     * flat Annex-IV `credentialSubject.rabiesValidUntil` leaf. Reading only the nested leaf exempted
     * the whole record type from the rule, so a lapsed EU health certificate badged a full-strength
     * green VALID on Documents, Home, Travel and the detail header. `CredentialGroup` maps
     * EU_HEALTH_CERT to Travel, so those records really do reach the badged surfaces.
     */
    @Test
    fun aLapsedEuHealthCertificateExpiresFromTheFlatRabiesLeaf() {
        val cred = credential(rabiesLapsed)
        assertEquals("2026-06-30", cred.validUntil)
        assertEquals("EXPIRED", cred.badge(now).label)
        assertEquals(VerdictDisplay.Tone.WARNING, cred.badge(now).tone)
    }

    /** …and one still inside its rabies window is untouched, exactly as for the nested leaf. */
    @Test
    fun anEuHealthCertificateInsideItsWindowIsUnaffected() {
        assertEquals("2027-06-30", credential(rabiesCurrent).validUntil)
        assertEquals("VALID", credential(rabiesCurrent).badge(now).label)
    }

    /**
     * Precedence, asserted in BOTH directions — a one-directional test passes by accident under a
     * reversed implementation. `validity.validUntil` wins whenever it is present, mirroring the web
     * wallet's `pick("validity.validUntil") || pick("rabiesValidUntil")`.
     */
    @Test
    fun theNestedValidityLeafWinsWhenBothArePresent() {
        assertEquals("2027-06-30", credential(current + rabiesLapsed).validUntil)
        assertEquals("VALID", credential(current + rabiesLapsed).badge(now).label)

        assertEquals("2026-06-30", credential(lapsed + rabiesCurrent).validUntil)
        assertEquals("EXPIRED", credential(lapsed + rabiesCurrent).badge(now).label)
    }

    /**
     * The non-answer rule, kept intact through the fallback: a document carrying NEITHER leaf still
     * claims nothing. Manufacturing an expiry out of missing data is the same defect inverted.
     *
     * A present-but-empty nested leaf must fall THROUGH to the flat one rather than suppress it —
     * emptiness is tested on the unwrapped value, not on the packed `"<salt>:<tag>:<value>"` string.
     */
    @Test
    fun anEmptyNestedLeafFallsThroughRatherThanSuppressingTheFallback() {
        assertNull(credential("").validUntil)
        assertEquals("VALID", credential("").badge(now).label)

        val emptyNested = ""","validity":{"validUntil":"cc:2:"}"""
        assertNull(credential(emptyNested).validUntil)
        assertEquals("2026-06-30", credential(emptyNested + rabiesLapsed).validUntil)
    }

    /** `by lazy` must stay out of the generated equality: forcing it on one instance may not make two
     *  otherwise-identical records compare unequal. */
    @Test
    fun theDerivedFieldDoesNotDisturbDataClassEquality() {
        val a = credential(lapsed)
        val b = credential(lapsed)
        a.validUntil // force the lazy on one side only
        assertEquals(a, b)
        assertEquals(a.hashCode(), b.hashCode())
    }
}
