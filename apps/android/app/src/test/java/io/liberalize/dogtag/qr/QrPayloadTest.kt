package io.liberalize.dogtag.qr

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/**
 * `QrPayload.parse` is the app's ONLY trust boundary against a scanned QR: every byte it sees is
 * attacker-chosen, and the only acceptable failure mode is `Unknown` — never a crash. These tests pin
 * that contract, and are deliberately mirrored by `apps/ios/DogTagTests/QrPayloadTests.swift` so the
 * two platforms cannot silently disagree about what a given QR means.
 *
 * Robolectric is required because `parse` is built on the real `android.net.Uri`; a hand-rolled URL
 * stand-in would test the stand-in, not the parser that actually ships.
 */
@RunWith(RobolectricTestRunner::class)
class QrPayloadTest {

    // ---- the four recognised shapes -------------------------------------------------------------

    @Test
    fun `short import token parses`() {
        val p = QrPayload.parse("https://vet.example.com/r/${"ab".repeat(16)}")
        assertTrue(p is QrPayload.ImportRecordToken)
        p as QrPayload.ImportRecordToken
        assertEquals("https://vet.example.com", p.host)
        assertEquals("ab".repeat(16), p.token)
    }

    @Test
    fun `export session parses and keeps the port in the origin`() {
        val p = QrPayload.parse("https://groomer.example.com:8443/x/tok123?a=0xAbC")
        assertTrue(p is QrPayload.ExportSession)
        p as QrPayload.ExportSession
        assertEquals("https://groomer.example.com:8443", p.host)
        assertEquals("tok123", p.token)
        assertEquals("0xAbC", p.groomerAddr)
    }

    @Test
    fun `dog tag issuance session parses`() {
        val p = QrPayload.parse("https://vet.example.com/p/${"cd".repeat(16)}")
        assertTrue(p is QrPayload.DogTagIssueSession)
        assertEquals("cd".repeat(16), (p as QrPayload.DogTagIssueSession).token)
    }

    @Test
    fun `legacy jwt import parses`() {
        val p = QrPayload.parse("https://vet.example.com/r?t=header.payload.sig&i=rec-1")
        assertTrue(p is QrPayload.ImportRecord)
        p as QrPayload.ImportRecord
        assertEquals("rec-1", p.recordId)
        assertEquals("header.payload.sig", p.jwt)
    }

    // ---- duplicate query keys (the iOS E-7 trap; must be identical on both platforms) ------------

    /**
     * A duplicated query key is legal in a URL and fully attacker-controllable. iOS previously TRAPPED
     * here (`Dictionary(uniqueKeysWithValues:)`); both platforms must now keep the FIRST occurrence.
     */
    @Test
    fun `duplicate query key keeps the first value and does not crash`() {
        val p = QrPayload.parse("https://groomer.example.com/x/tok?a=0xFIRST&a=0xSECOND")
        assertTrue(p is QrPayload.ExportSession)
        assertEquals("0xFIRST", (p as QrPayload.ExportSession).groomerAddr)
    }

    @Test
    fun `duplicate keys on the legacy jwt shape keep the first value`() {
        val p = QrPayload.parse("https://vet.example.com/r?t=first&t=second&i=one&i=two")
        assertTrue(p is QrPayload.ImportRecord)
        p as QrPayload.ImportRecord
        assertEquals("first", p.jwt)
        assertEquals("one", p.recordId)
    }

    @Test
    fun `many duplicates of an irrelevant key still parse`() {
        val junk = (1..50).joinToString("&") { "x=$it" }
        val p = QrPayload.parse("https://groomer.example.com/x/tok?a=0xAddr&$junk")
        assertTrue(p is QrPayload.ExportSession)
        assertEquals("0xAddr", (p as QrPayload.ExportSession).groomerAddr)
    }

    // ---- malformed / hostile input: every one of these must be Unknown, never a crash -------------

    @Test
    fun `malformed input degrades to unknown`() {
        val hostile = listOf(
            "",
            "   ",
            "not a url at all",
            "://missing-scheme",
            "https://",
            "https://vet.example.com",              // no path
            "https://vet.example.com/r/",           // empty token
            "https://vet.example.com/x/tok",        // export without the required `a`
            "https://vet.example.com/x/?a=0xAddr",  // export without a token
            "https://vet.example.com/p/",           // empty issuance token
            "https://vet.example.com/p/tok?a=1",    // issuance must have NO query string
            "https://vet.example.com/r/tok?a=1",    // short import must have NO query string
            "https://vet.example.com/r?t=only",     // legacy jwt missing `i`
            "https://vet.example.com/r?i=only",     // legacy jwt missing `t`
            "https://vet.example.com/unknown/path",
            "javascript:alert(1)",
        )
        // NOTE: invalid percent-encoding (`?a=%ZZ`) is deliberately NOT asserted here. Android's `Uri`
        // lenient-decodes it to a garbage string while iOS's `URLComponents` treats it differently, so
        // the two platforms genuinely diverge. Neither CRASHES, which is what this file guards; the
        // resulting address is rejected downstream when it fails to parse as an EVM address.
        for (raw in hostile) {
            val p = QrPayload.parse(raw)
            assertTrue("expected Unknown for <$raw>, got $p", p is QrPayload.Unknown)
        }
    }

    @Test
    fun `surrounding whitespace is trimmed`() {
        val p = QrPayload.parse("  \n https://vet.example.com/p/${"ef".repeat(16)}  \t ")
        assertTrue(p is QrPayload.DogTagIssueSession)
    }

    @Test
    fun `trailing slash does not change the legacy jwt shape`() {
        val p = QrPayload.parse("https://vet.example.com/r/?t=a.b.c&i=rec")
        assertTrue(p is QrPayload.ImportRecord)
    }

    // ---- decodeJwtClaims must also never throw on hostile input ----------------------------------

    @Test
    fun `decodeJwtClaims returns empty json for malformed jwts`() {
        for (bad in listOf("", "onlyonepart", "a.!!!not-base64!!!.c", "a..c")) {
            assertEquals(0, QrPayload.decodeJwtClaims(bad).length())
        }
    }
}
