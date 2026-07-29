package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.net.Http
import io.liberalize.dogtag.net.IssuerBinding
import io.liberalize.dogtag.net.IssuerBindingState
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class ProviderDirectoryTest {
    private val rows = """
        {
          "businesses": [
            {
              "businessId": "contact-only",
              "type": "vet",
              "name": "Call-only Vet",
              "geo": null,
              "services": ["telehealth"],
              "domain": "",
              "contact": {
                "phone": "+65 6123 4567",
                "whatsapp": "6561234567",
                "telegram": "callonlyvet",
                "email": "care@example.test",
                "website": "https://call-only.test"
              }
            },
            {
              "businessId": "located",
              "type": "groomer",
              "name": "Good Dog Grooming",
              "geo": {"lat": 1.3039, "lng": 103.8318},
              "services": ["grooming"],
              "domain": "good-dog.test"
            }
          ]
        }
    """.trimIndent()

    @Test
    fun requestIsTheExactFullSetPathWithNoPositionOrQuery() = runBlocking {
        val requested = mutableListOf<String>()
        val directory = CentralProviderDirectory(
            baseUrl = "https://central.test/api/",
            now = { 1_000 },
            fetch = { url ->
                requested += url
                Http.Response(200, rows)
            },
        )

        val result = directory.read()
        assertTrue(result is ProviderDirectoryResult.Found)
        assertEquals(listOf("https://central.test/api/v1/businesses"), requested)
        val url = requested.single()
        for (forbidden in listOf("?", "near=", "radius=", "lat=", "lng=", "geohash=")) {
            assertFalse("request must not contain $forbidden", url.contains(forbidden))
        }
    }

    @Test
    fun contactOnlyAndDomainlessRowsSurviveWithoutASyntheticCoordinateOrVerifiedClaim() {
        val directory = CentralProviderDirectory("https://central.test", now = { 1_000 })
        val result = directory.decode(rows, 1_000)
        result as ProviderDirectoryResult.Found

        val contactOnly = result.providers.first()
        assertEquals(null, contactOnly.geo)
        assertEquals("+65 6123 4567", contactOnly.contact.phone)
        assertEquals("https://call-only.test", contactOnly.contact.website)
        assertTrue(contactOnly.contact.hasAny)
        // A directory row is not a chain read, so a blank domain column may not claim the on-chain
        // fact `NoDomainClaimed`; it says only that this listing carries no domain.
        assertTrue(contactOnly.bindingState === IssuerBindingState.NoDomainListed)
        assertFalse(contactOnly.bindingState === IssuerBindingState.NoDomainClaimed)
        assertFalse(
            IssuerBinding(state = contactOnly.bindingState, domain = "").line.contains("on-chain"),
        )

        val located = result.providers.last()
        assertEquals(GeoPoint(1.3039, 103.8318), located.geo)
        assertEquals(null, located.active)
        assertTrue(located.bindingState === IssuerBindingState.Unavailable)
        assertFalse(located.bindingState is IssuerBindingState.Verified)
    }

    /**
     * A provider reachable ONLY by website must not be reported as having published nothing.
     *
     * The server serves all five channels; this parser once read four, so `website` was dropped on
     * the floor, `hasAny` read false, and the screen said "No contact details published." about a
     * provider that had published exactly one. Mirrored by iOS
     * `test_aWebsiteOnlyProviderIsContactableRatherThanReportedAsPublishingNothing`.
     */
    @Test
    fun aWebsiteOnlyProviderIsContactableRatherThanReportedAsPublishingNothing() {
        val body = """
            {"businesses":[{
              "businessId":"web-only","type":"groomer","name":"Web Only Grooming",
              "geo":null,"services":[],"domain":"",
              "contact":{"website":"https://web-only.test"}
            }]}
        """.trimIndent()
        val result = CentralProviderDirectory("https://central.test").decode(body, 1_000)
            as ProviderDirectoryResult.Found
        val contact = result.providers.single().contact
        assertEquals("https://web-only.test", contact.website)
        assertEquals(null, contact.phone)
        assertTrue("a website-only provider is contactable", contact.hasAny)
    }

    /** Consistent with the four sibling channels: a non-string channel is a malformed row. */
    @Test
    fun aNonTextWebsiteIsMalformedLikeEveryOtherChannel() {
        val body = """
            {"businesses":[{
              "businessId":"one","type":"vet","name":"One",
              "geo":null,"services":[],"domain":"","contact":{"website":42}
            }]}
        """.trimIndent()
        val result = CentralProviderDirectory("https://central.test").decode(body, 1_000)
        assertEquals(
            DirectoryUnavailableReason.MalformedResponse,
            (result as ProviderDirectoryResult.Unavailable).reason,
        )
    }

    @Test
    fun centralDoesNotInventAListingStateFromAnExtraWireField() {
        val body = """
            {"businesses":[{
              "businessId":"one","type":"vet","name":"One",
              "geo":{"lat":0,"lng":0},"services":[],"domain":"","active":false
            }]}
        """.trimIndent()
        val result = CentralProviderDirectory("https://central.test").decode(body, 1_000)
            as ProviderDirectoryResult.Found
        assertEquals(null, result.providers.single().active)
    }

    @Test
    fun missingDomainIsMalformedRatherThanInventedAsADomainlessListing() {
        val body = """
            {"businesses":[{
              "businessId":"one","type":"vet","name":"One",
              "geo":null,"services":[]
            }]}
        """.trimIndent()
        val result = CentralProviderDirectory("https://central.test").decode(body, 1_000)
        assertEquals(
            DirectoryUnavailableReason.MalformedResponse,
            (result as ProviderDirectoryResult.Unavailable).reason,
        )
    }

    @Test
    fun failedRefreshReplaysStoredOnlyUntilTheHardDeadline() = runBlocking {
        var time = 1_000L
        var call = 0
        val directory = CentralProviderDirectory(
            baseUrl = "https://central.test",
            ttlMs = 1_000,
            now = { time },
            fetch = {
                call += 1
                if (call == 1) Http.Response(200, rows) else Http.Response(503, "")
            },
        )

        val live = directory.read() as ProviderDirectoryResult.Found
        assertEquals(DirectoryObservation.Live, live.observation)
        assertEquals(2_000, live.expiresAt)

        time = 1_999
        val stored = directory.read() as ProviderDirectoryResult.Found
        assertEquals(DirectoryObservation.Stored, stored.observation)
        assertEquals(1_000, stored.readAt)
        assertEquals(2_000, stored.expiresAt)

        time = 2_000
        val expired = directory.read()
        assertTrue(expired is ProviderDirectoryResult.Unavailable)
    }

    @Test
    fun configuredBaseCannotSmuggleAQueryOrFragment() {
        assertThrows(IllegalArgumentException::class.java) {
            CentralProviderDirectory("https://central.test?near=1,2")
        }
        assertThrows(IllegalArgumentException::class.java) {
            CentralProviderDirectory("https://central.test/#location")
        }
    }
}
