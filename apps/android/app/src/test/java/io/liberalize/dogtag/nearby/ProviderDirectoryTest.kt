package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.net.Http
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
                "email": "care@example.test"
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
        assertTrue(contactOnly.contact.hasAny)
        assertTrue(contactOnly.bindingState === IssuerBindingState.NoDomainClaimed)

        val located = result.providers.last()
        assertEquals(GeoPoint(1.3039, 103.8318), located.geo)
        assertEquals(null, located.active)
        assertTrue(located.bindingState === IssuerBindingState.Unavailable)
        assertFalse(located.bindingState is IssuerBindingState.Verified)
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
    fun missingDomainIsMalformedRatherThanInventedAsNoDomainClaimed() {
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
