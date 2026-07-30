package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.net.IssuerBindingState
import kotlinx.coroutines.runBlocking
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The offline provider-record fallback, case by case.
 *
 * This suite REPLACES slice S-3's `CachedProviderDirectory` cases. Those pinned a decorator around a
 * no-argument `read()` of the whole provider set - live-read-first, replay-as-empty, nested stored
 * results, wrapper deadline renewal - and that seam no longer exists: a nearest read is personalized
 * and paged, so there is no single "the directory" response to substitute for. Everything that was
 * really about the CODEC or the store's failure tolerance carried over, because the reshape does not
 * change those properties. What is new is the pair the captain's ruling turns on: a distance is never
 * persisted, and the stored order can never carry the ranking.
 *
 * The rule they all still serve: could-not-answer and "there is nothing" are different statements,
 * and no cache state may turn the first into the second.
 */
class DirectoryCacheTest {
    private val namespace = "central:https://api.dogtag.io"
    private val storedAt = 1_785_312_000_000L

    private fun provider(
        id: String,
        name: String,
        kind: String = "vet",
        geo: GeoPoint? = GeoPoint(1.3039, 103.8318),
        contact: ProviderContact = ProviderContact(phone = "+65 6123 4567"),
        active: Boolean? = null,
        bindingState: IssuerBindingState = IssuerBindingState.Unavailable,
    ) = DirectoryProvider(
        providerId = id,
        kind = kind,
        name = name,
        geo = geo,
        services = listOf("vaccination"),
        domain = null,
        active = active,
        contact = contact,
        bindingState = bindingState,
    )

    private fun cache(
        store: ProviderDirectoryCacheStore = MemoryProviderDirectoryCacheStore(),
        namespace: String = this.namespace,
    ) = ProviderRecordCache(store = store, namespace = namespace)

    // ---- The two properties the ruling turns on ----

    /**
     * The whole reason this stores records rather than a snapshot.
     *
     * A distance is computed from the owner's position, so persisting one would put a position
     * derivative on disk AND let a later offline read state a distance measured from somewhere the
     * owner no longer is. Asserted against the raw document rather than the decoded value, because
     * "the decoded type has no distance field" is exactly what a later refactor could quietly undo.
     */
    @Test
    fun aDistanceIsNeverWrittenToDisk() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("a", "Alpha Vet"), provider("b", "Beta Vet")), storedAt)

        val document = store.read()
        assertNotNull(document)
        assertFalse("no distance may reach the document", document!!.contains("distance"))
        // Nor the position such a distance would have been measured from.
        assertFalse(document.contains("approximate"))
    }

    /**
     * The array order of a nearest page IS the ranking, so replaying it verbatim would present a
     * stale ordering as current - invisibly, because a sorted list simply looks sorted.
     */
    @Test
    fun theStoredOrderIsNameOrderNotTheServerRanking() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        // As a nearest page arrives: closest first, which here is alphabetically backwards.
        val ranked = listOf(
            provider("z", "Zulu Vet"),
            provider("m", "Mike Vet"),
            provider("a", "Alpha Vet"),
        )
        cache(store).remember(ranked, storedAt)

        assertEquals(
            listOf("Alpha Vet", "Mike Vet", "Zulu Vet"),
            cache(store).recall(storedAt + 1)?.providers?.map { it.name },
        )
    }

    /** Re-sorting on READ is what makes that hold for a document this build did not write. */
    @Test
    fun aReorderedDocumentIsStillReadBackInNameOrder() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("a", "Alpha Vet"), provider("z", "Zulu Vet")), storedAt)

        val document = JSONObject(store.read()!!)
        val providers = document.getJSONArray("providers")
        val reversed = JSONArray()
        for (index in providers.length() - 1 downTo 0) reversed.put(providers.get(index))
        store.write(document.put("providers", reversed).toString())

        assertEquals(
            listOf("Alpha Vet", "Zulu Vet"),
            cache(store).recall(storedAt + 1)?.providers?.map { it.name },
        )
    }

    /** "Minimal" was the instruction twice over: the stored set cannot grow with the directory. */
    @Test
    fun theStoredSetIsCappedAtOnePage() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember((1..80).map { provider("p$it", "Vet %02d".format(it)) }, storedAt)

        assertEquals(DEFAULT_PROVIDER_PAGE_SIZE, ProviderRecordCache.MAX_RECORDS)
        assertEquals(
            ProviderRecordCache.MAX_RECORDS,
            cache(store).recall(storedAt + 1)?.providers?.size,
        )
    }

    // ---- Could-not-answer never becomes an established absence ----

    @Test
    fun nothingStoredIsNullRatherThanAnEmptyRecordSet() = runBlocking {
        assertNull(cache().recall(storedAt))
    }

    @Test
    fun anUnreadableDocumentIsNothingStored() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        store.write("{ not json")

        assertNull(cache(store).recall(storedAt))
    }

    @Test
    fun aStoredSetIsNeverReplayedForADifferentlyConfiguredDeployment() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("a", "Alpha Vet")), storedAt)

        assertNull(cache(store, namespace = "central:https://other.example").recall(storedAt + 1))
    }

    /** At the exact deadline it is expired, matching the web cache and the one this replaces. */
    @Test
    fun anEntryExpiresAtItsExactDeadlineNotAfterIt() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("a", "Alpha Vet")), storedAt)
        val deadline = storedAt + ProviderRecordCache.DEFAULT_TTL_MILLIS

        assertNotNull(cache(store).recall(deadline - 1))
        assertNull(cache(store).recall(deadline))
    }

    /** A stored time in the future is a backwards clock jump, not a fresh copy. */
    @Test
    fun aSetStoredInTheFutureIsDroppedRatherThanTrusted() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("a", "Alpha Vet")), storedAt)

        assertNull(cache(store).recall(storedAt - 1))
    }

    /**
     * Version 1 was S-3's full-set snapshot, whose `providers` array was in server order - the
     * ranking. Dropping rather than migrating is what stops it being reinterpreted as a record set.
     */
    @Test
    fun aDocumentFromTheEarlierSnapshotShapeIsDroppedRatherThanMigrated() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("a", "Alpha Vet")), storedAt)
        store.write(JSONObject(store.read()!!).put("version", 1).toString())

        assertNull(cache(store).recall(storedAt + 1))
    }

    @Test
    fun aStoredSetCarryingNoProvidersIsMalformedRatherThanAnEmptyAnswer() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("a", "Alpha Vet")), storedAt)
        store.write(JSONObject(store.read()!!).put("providers", JSONArray()).toString())

        assertNull(cache(store).recall(storedAt + 1))
    }

    /** A nameless row renders as a list entry the owner cannot act on. */
    @Test
    fun aBlankNameOrIdIsRefusedOnWriteAndOnReplay() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("", "No Id"), provider("b", "   ")), storedAt)

        assertNull("nothing well-formed was offered, so nothing is stored", store.read())
        assertFalse(providerRecordIsWellFormed(provider("", "No Id")))
        assertFalse(providerRecordIsWellFormed(provider("b", " ")))
        assertTrue(providerRecordIsWellFormed(provider("b", "Beta Vet")))
    }

    /**
     * An empty live page means this query matched nothing, which is not evidence that the previously
     * remembered providers ceased to exist - so it neither writes nor clears.
     */
    @Test
    fun anEmptyLivePageLeavesThePreviouslyRememberedSetAlone() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("a", "Alpha Vet")), storedAt)
        cache(store).remember(emptyList(), storedAt + 1)

        assertEquals(
            listOf("Alpha Vet"),
            cache(store).recall(storedAt + 2)?.providers?.map { it.name },
        )
    }

    /** Replace, not accumulate: offline shows the last providers seen, deliberately not a history. */
    @Test
    fun rememberReplacesRatherThanAccumulating() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("a", "Alpha Vet")), storedAt)
        cache(store).remember(listOf(provider("b", "Beta Vet")), storedAt + 1)

        assertEquals(
            listOf("Beta Vet"),
            cache(store).recall(storedAt + 2)?.providers?.map { it.name },
        )
    }

    // ---- Codec fidelity (carried over from S-3) ----

    @Test
    fun theStoredDocumentRoundTripsEveryFieldIncludingAbsentLocation() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(
            listOf(
                provider(
                    id = "contact-only",
                    name = "Contact Only Vet",
                    geo = null,
                    contact = ProviderContact(
                        phone = "+65 1",
                        whatsapp = "+65 2",
                        telegram = "@three",
                        email = "four@example.test",
                        website = "https://five.example.test",
                    ),
                    active = true,
                    bindingState = IssuerBindingState.NoDomainListed,
                ),
                provider("placed", "Placed Groomer", kind = "groomer", geo = GeoPoint(0.0, 0.0)),
            ),
            storedAt,
        )

        val recalled = cache(store).recall(storedAt + 1)?.providers
        assertEquals(2, recalled?.size)
        val contactOnly = recalled?.single { it.providerId == "contact-only" }
        assertNull(contactOnly?.geo)
        assertEquals(IssuerBindingState.NoDomainListed, contactOnly?.bindingState)
        assertEquals("+65 1", contactOnly?.contact?.phone)
        assertEquals("https://five.example.test", contactOnly?.contact?.website)
        assertEquals(true, contactOnly?.active)
        // `0,0` is a real coordinate and survives as one.
        val placed = recalled?.single { it.providerId == "placed" }
        assertEquals(GeoPoint(0.0, 0.0), placed?.geo)
        assertEquals("groomer", placed?.kind)
        assertNull(placed?.active)
    }

    @Test
    fun anAbsentLocationIsStoredAsAbsentNeverAsTheRealCoordinateZeroZero() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("c", "Contact Only", geo = null)), storedAt)

        val row = JSONObject(store.read()!!).getJSONArray("providers").getJSONObject(0)
        assertFalse("absence must be absence, not a pin off the coast of Ghana", row.has("geo"))
    }

    /** A stored `Verified` would claim a DNS check nobody performed. */
    @Test
    fun aStoredBindingStateCanNeverClaimVerified() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(
            listOf(provider("v", "Verified Vet", bindingState = IssuerBindingState.Verified("listed"))),
            storedAt,
        )

        assertEquals(
            IssuerBindingState.Unavailable,
            cache(store).recall(storedAt + 1)?.providers?.single()?.bindingState,
        )
    }

    /** The document records WHEN it was stored, never that it was live: a replay is a replay. */
    @Test
    fun aStoredDocumentCannotDeclareItselfLive() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(listOf(provider("a", "Alpha Vet")), storedAt)

        val document = store.read()!!
        assertFalse(document.contains("observation"))
        assertFalse(document.contains("\"live\""))
        assertEquals(storedAt, cache(store).recall(storedAt + 1)?.storedAt)
    }

    // ---- A store failure costs the fallback, never the live answer ----

    @Test
    fun aStoreThatCannotWriteIsNotAnError() = runBlocking {
        val store = object : ProviderDirectoryCacheStore {
            override fun read(): String? = null
            override fun write(document: String) = throw IllegalStateException("disk full")
            override fun clear() {}
        }
        cache(store).remember(listOf(provider("a", "Alpha Vet")), storedAt)

        assertNull(cache(store).recall(storedAt + 1))
    }

    @Test
    fun aStoreThatCannotBeReadIsNothingStored() = runBlocking {
        val store = object : ProviderDirectoryCacheStore {
            override fun read(): String = throw IllegalStateException("unreadable")
            override fun write(document: String) {}
            override fun clear() {}
        }

        assertNull(cache(store).recall(storedAt))
    }

    @Test
    fun aStoreThatCannotBeClearedIsNotAnError() = runBlocking {
        val store = object : ProviderDirectoryCacheStore {
            override fun read(): String? = null
            override fun write(document: String) {}
            override fun clear() = throw IllegalStateException("locked")
        }

        cache(store).clear()
    }
}
