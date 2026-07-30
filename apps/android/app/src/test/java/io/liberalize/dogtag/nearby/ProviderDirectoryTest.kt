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
    private val nearestPage = """
        {
          "businesses": [
            {
              "businessId": "groomer",
              "type": "groomer",
              "name": "Good Dog Grooming",
              "geo": {"lat": 1.3039, "lng": 103.8318},
              "services": ["grooming"],
              "domain": "good-dog.test",
              "distanceKm": 0.8
            },
            {
              "businessId": "vet",
              "type": "vet",
              "name": "Ávila Veterinary",
              "geo": {"lat": 1.35, "lng": 103.82},
              "services": ["wellness"],
              "domain": "",
              "distanceKm": 4.2
            }
          ],
          "total": 52,
          "limit": 25,
          "offset": 50,
          "hasMore": false
        }
    """.trimIndent()

    private val contactsPage = """
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
            }
          ],
          "total": 1,
          "limit": 25,
          "offset": 0,
          "hasMore": false
        }
    """.trimIndent()

    @Test
    fun nearestRoundsBeforeBuildingTheExactBodyAndPreservesServerDistanceOrder() = runBlocking {
        val posts = mutableListOf<Pair<String, String>>()
        var getCalls = 0
        val directory = CentralProviderDirectory(
            baseUrl = "https://central.test/api/",
            now = { 1_000 },
            get = {
                getCalls += 1
                Http.Response(500, "")
            },
            post = { url, body ->
                posts += url to body
                Http.Response(200, nearestPage)
            },
        )
        val position = requireNotNull(
            ApproximateCallerPosition.from(GeoPoint(1.35249, 103.81951)),
        )
        val query = ProviderDirectoryQuery(
            kinds = listOf("vet", "groomer"),
            name = "Clínica & Spa",
            limit = 25,
            offset = 50,
        )

        val result = directory.nearest(position, query) as ProviderDirectoryResult.Found

        assertEquals(0, getCalls)
        assertEquals(
            "https://central.test/api/v1/businesses/nearest" +
                "?kind=vet&kind=groomer&limit=25&offset=50&name=Cl%C3%ADnica%20%26%20Spa",
            posts.single().first,
        )
        assertEquals(
            """{"approximateLat":1.352,"approximateLng":103.82}""",
            posts.single().second,
        )
        assertEquals(listOf("groomer", "vet"), result.providers.map { it.providerId })
        assertEquals(listOf(0.8, 4.2), result.providers.map { result.distancesKm[it.providerId] })
        assertEquals(52, result.total)
        assertEquals(50, result.offset)
        assertFalse(result.hasMore)
    }

    @Test
    fun contactsSendsOwnerKindsNameAndPagingWithoutAnyPositionOrPost() = runBlocking {
        val gets = mutableListOf<String>()
        var postCalls = 0
        val directory = CentralProviderDirectory(
            baseUrl = "https://central.test",
            now = { 1_000 },
            get = { url ->
                gets += url
                Http.Response(200, contactsPage)
            },
            post = { _, _ ->
                postCalls += 1
                Http.Response(500, "")
            },
        )

        val result = directory.contacts(
            ProviderDirectoryQuery(
                kinds = listOf("vet", "groomer"),
                name = "Call only",
            ),
        ) as ProviderDirectoryResult.Found

        assertEquals(
            listOf(
                "https://central.test/v1/businesses" +
                    "?kind=vet&kind=groomer&limit=25&offset=0&name=Call%20only",
            ),
            gets,
        )
        assertEquals(0, postCalls)
        assertTrue(result.distancesKm.isEmpty())
        val provider = result.providers.single()
        assertEquals(null, provider.geo)
        assertEquals("+65 6123 4567", provider.contact.phone)
        assertEquals("https://call-only.test", provider.contact.website)
        assertTrue(provider.contact.hasAny)
        assertTrue(provider.bindingState === IssuerBindingState.NoDomainListed)
        assertFalse(provider.bindingState === IssuerBindingState.NoDomainClaimed)
        assertFalse(
            IssuerBinding(state = provider.bindingState, domain = "").line.contains("on-chain"),
        )
    }

    @Test
    fun nearestResponsesAreNeverReplayedFromAnAdapterCache() = runBlocking {
        var calls = 0
        val directory = CentralProviderDirectory(
            baseUrl = "https://central.test",
            post = { _, _ ->
                calls += 1
                if (calls == 1) Http.Response(200, nearestPage) else Http.Response(503, "")
            },
        )
        val position = requireNotNull(ApproximateCallerPosition.from(GeoPoint(1.35, 103.82)))
        val query = ProviderDirectoryQuery(listOf("vet", "groomer"), offset = 50)

        assertTrue(directory.nearest(position, query) is ProviderDirectoryResult.Found)
        assertTrue(directory.nearest(position, query) is ProviderDirectoryResult.Unavailable)
        assertEquals(2, calls)
    }

    @Test
    fun nearestRequiresEveryDistanceToBeFiniteAndNonnegative() {
        val directory = CentralProviderDirectory("https://central.test")
        for (distance in listOf("null", "-1", "\"near\"")) {
            val body = """
                {
                  "businesses":[{
                    "businessId":"one","type":"vet","name":"One",
                    "geo":{"lat":0,"lng":0},"services":[],"domain":"",
                    "distanceKm":$distance
                  }],
                  "total":1,"limit":25,"offset":0,"hasMore":false
                }
            """.trimIndent()
            val result = directory.decode(body, 1_000, requireDistance = true)
            assertEquals(
                distance,
                DirectoryUnavailableReason.MalformedResponse,
                (result as ProviderDirectoryResult.Unavailable).reason,
            )
        }

        val contactOnlyWithInventedDistance = """
            {
              "businesses":[{
                "businessId":"one","type":"vet","name":"One",
                "geo":null,"services":[],"domain":"","distanceKm":1.0
              }],
              "total":1,"limit":25,"offset":0,"hasMore":false
            }
        """.trimIndent()
        val result = directory.decode(
            contactOnlyWithInventedDistance,
            1_000,
            requireDistance = true,
        )
        assertEquals(
            DirectoryUnavailableReason.MalformedResponse,
            (result as ProviderDirectoryResult.Unavailable).reason,
        )
    }

    @Test
    fun inconsistentPagingIsMalformedRatherThanSilentlyTruncated() {
        val body = contactsPage.replace("\"hasMore\": false", "\"hasMore\": true")
        val result = CentralProviderDirectory("https://central.test")
            .decode(body, 1_000, requireDistance = false)
        assertEquals(
            DirectoryUnavailableReason.MalformedResponse,
            (result as ProviderDirectoryResult.Unavailable).reason,
        )
    }

    @Test
    fun nearestPageDistancesMustBeNondecreasing() {
        val outOfOrder = nearestPage
            .replace("\"distanceKm\": 0.8", "\"distanceKm\": 5.0")
        val result = CentralProviderDirectory("https://central.test")
            .decode(outOfOrder, 1_000, requireDistance = true)
        assertEquals(
            DirectoryUnavailableReason.MalformedResponse,
            (result as ProviderDirectoryResult.Unavailable).reason,
        )
    }

    @Test
    fun nonemptyPageCannotExceedTotalButEmptyOffsetBeyondTotalIsValid() {
        val overflowing = contactsPage.replace("\"total\": 1", "\"total\": 0")
        val invalid = CentralProviderDirectory("https://central.test")
            .decode(overflowing, 1_000, requireDistance = false)
        assertEquals(
            DirectoryUnavailableReason.MalformedResponse,
            (invalid as ProviderDirectoryResult.Unavailable).reason,
        )

        val beyondEnd = """
            {
              "businesses":[],
              "total":1,
              "limit":25,
              "offset":99,
              "hasMore":false
            }
        """.trimIndent()
        val valid = CentralProviderDirectory("https://central.test")
            .decode(beyondEnd, 1_000, requireDistance = false)
            as ProviderDirectoryResult.Empty
        assertEquals(99, valid.offset)
        assertEquals(1, valid.total)
        assertFalse(valid.hasMore)
    }

    @Test
    fun aWebsiteOnlyProviderIsContactableRatherThanReportedAsPublishingNothing() {
        val body = """
            {
              "businesses":[{
                "businessId":"web-only","type":"groomer","name":"Web Only Grooming",
                "geo":null,"services":[],"domain":"",
                "contact":{"website":"https://web-only.test"}
              }],
              "total":1,"limit":25,"offset":0,"hasMore":false
            }
        """.trimIndent()
        val result = CentralProviderDirectory("https://central.test")
            .decode(body, 1_000, requireDistance = false) as ProviderDirectoryResult.Found
        val contact = result.providers.single().contact
        assertEquals("https://web-only.test", contact.website)
        assertEquals(null, contact.phone)
        assertTrue(contact.hasAny)
    }

    @Test
    fun queryAndBaseUrlRejectAmbiguousInputs() {
        assertThrows(IllegalArgumentException::class.java) {
            ProviderDirectoryQuery(emptyList())
        }
        assertThrows(IllegalArgumentException::class.java) {
            ProviderDirectoryQuery(listOf("vet", " "))
        }
        assertThrows(IllegalArgumentException::class.java) {
            ProviderDirectoryQuery(listOf("vet"), limit = 0)
        }
        assertThrows(IllegalArgumentException::class.java) {
            ProviderDirectoryQuery(listOf("vet"), offset = -1)
        }
        assertThrows(IllegalArgumentException::class.java) {
            CentralProviderDirectory("https://central.test?near=1,2")
        }
        assertThrows(IllegalArgumentException::class.java) {
            CentralProviderDirectory("https://central.test/#location")
        }
    }
}
