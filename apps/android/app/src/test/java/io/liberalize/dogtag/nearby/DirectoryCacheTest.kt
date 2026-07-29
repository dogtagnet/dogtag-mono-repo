package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.net.IssuerBindingState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.runBlocking
import org.json.JSONException
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The on-device local copy, case by case.
 *
 * Mirrors `packages/ui/test/providerDirectory.test.ts`'s cache cases and the iOS
 * `DirectoryCacheTests`. The rule these all serve: `unavailable` and `empty` are different
 * statements, and no cache state may turn the first into the second.
 */
class DirectoryCacheTest {
    private val located = DirectoryProvider(
        providerId = "located",
        kind = "vet",
        name = "Harbour Vet",
        geo = GeoPoint(1.3039, 103.8318),
        services = listOf("vaccination"),
        domain = "harbour.test",
        active = null,
        contact = ProviderContact(phone = "+65 6123 4567"),
        bindingState = IssuerBindingState.Unavailable,
    )

    private val contactOnly = DirectoryProvider(
        providerId = "contact-only",
        kind = "groomer",
        name = "Call-only Grooming",
        geo = null,
        services = emptyList(),
        domain = null,
        active = null,
        contact = ProviderContact(website = "https://call-only.test"),
        bindingState = IssuerBindingState.NoDomainListed,
    )

    private class FakeDirectory(
        override val cacheNamespace: String = "central:https://central.test/v1/businesses",
        var next: () -> ProviderDirectoryResult,
    ) : ProviderDirectory {
        var reads = 0
        override suspend fun read(): ProviderDirectoryResult {
            reads += 1
            return next()
        }
    }

    private fun found(readAt: Long, expiresAt: Long? = null) = ProviderDirectoryResult.Found(
        providers = listOf(located, contactOnly),
        observation = DirectoryObservation.Live,
        readAt = readAt,
        expiresAt = expiresAt,
    )

    private fun unavailable(at: Long) = ProviderDirectoryResult.Unavailable(
        reason = DirectoryUnavailableReason.SourceUnavailable,
        detail = "offline",
        attemptedAt = at,
    )

    /** The one live shape the wrapper refuses outright, so it is how the clear path is reached. */
    private fun emptyFound(readAt: Long) = ProviderDirectoryResult.Found(
        providers = emptyList(),
        observation = DirectoryObservation.Live,
        readAt = readAt,
        expiresAt = null,
    )

    // ---- the statement each state is entitled to make ------------------------------------------

    /**
     * The headline property. An unreachable directory must not read as an empty one: telling an
     * owner there is no help nearby when the truth is "we could not ask" sends them away.
     */
    @Test
    fun anUnreachableDirectoryWithNothingStoredStaysUnavailableAndNeverBecomesEmpty() = runBlocking {
        val directory = CachedProviderDirectory(
            delegate = FakeDirectory(next = { unavailable(5_000) }),
            store = MemoryProviderDirectoryCacheStore(),
            now = { 5_000 },
        )
        val result = directory.read()
        assertTrue(result is ProviderDirectoryResult.Unavailable)
        assertEquals(
            DirectoryUnavailableReason.SourceUnavailable,
            (result as ProviderDirectoryResult.Unavailable).reason,
        )
    }

    /** A source that really did answer "none" keeps saying so, stored or live. */
    @Test
    fun aStoredEmptyReplaysAsEmptyRatherThanUnavailable() = runBlocking {
        var time = 1_000L
        val delegate = FakeDirectory(next = {
            ProviderDirectoryResult.Empty(DirectoryObservation.Live, readAt = 1_000, expiresAt = null)
        })
        val directory = CachedProviderDirectory(
            delegate = delegate,
            store = MemoryProviderDirectoryCacheStore(),
            ttlMs = 10_000,
            now = { time },
        )
        assertTrue(directory.read() is ProviderDirectoryResult.Empty)

        time = 2_000
        delegate.next = { unavailable(2_000) }
        val replayed = directory.read()
        assertTrue(replayed is ProviderDirectoryResult.Empty)
        assertEquals(
            DirectoryObservation.Stored,
            (replayed as ProviderDirectoryResult.Empty).observation,
        )
    }

    /**
     * A `found` carrying zero providers renders as "nothing found" and is the exact false absence
     * this module exists to prevent. TypeScript makes it unrepresentable; Kotlin's `List` cannot, so
     * the guard must run on the way IN and on the way back OUT of storage.
     */
    @Test
    fun aFoundCarryingNoProvidersIsRefusedOnWriteAndOnReplay() = runBlocking {
        val empty = ProviderDirectoryResult.Found(
            providers = emptyList(),
            observation = DirectoryObservation.Live,
            readAt = 1_000,
            expiresAt = null,
        )
        assertTrue(!providerDirectorySnapshotIsWellFormed(empty))

        val store = MemoryProviderDirectoryCacheStore()
        val directory = CachedProviderDirectory(
            delegate = FakeDirectory(next = { empty }),
            store = store,
            now = { 1_000 },
        )
        val result = directory.read()
        assertEquals(
            DirectoryUnavailableReason.InvalidSnapshot,
            (result as ProviderDirectoryResult.Unavailable).reason,
        )
        assertNull("a corrupt snapshot is never stored", store.read())

        // And the same document arriving from disk cannot be replayed either.
        val forged = JSONObject(
            ProviderDirectoryCacheCodec.encode(
                ProviderDirectoryCacheEntry(
                    namespace = "central:https://central.test/v1/businesses",
                    snapshot = found(1_000, 5_000),
                    readAt = 1_000,
                    expiresAt = 5_000,
                ),
            ),
        )
        forged.put("providers", org.json.JSONArray())
        store.write(forged.toString())
        val replayed = CachedProviderDirectory(
            delegate = FakeDirectory(next = { unavailable(2_000) }),
            store = store,
            ttlMs = 10_000,
            now = { 2_000 },
        ).read()
        assertTrue(
            "a stored found-with-zero-providers must not reach the list",
            replayed is ProviderDirectoryResult.Unavailable,
        )
    }

    // ---- the two review findings ---------------------------------------------------------------

    /**
     * Review finding 1. The seam ASKS implementations to resolve rather than throw and Kotlin cannot
     * enforce it, so an unexpected throw must still reach the stored copy - otherwise the offline
     * fallback is missing in precisely the unexpected-failure case it exists for.
     */
    @Test
    fun anUnexpectedThrowStillConsultsTheStoredCopy() = runBlocking {
        var time = 1_000L
        val delegate = FakeDirectory(next = { found(1_000) })
        val directory = CachedProviderDirectory(
            delegate = delegate,
            store = MemoryProviderDirectoryCacheStore(),
            ttlMs = 10_000,
            now = { time },
        )
        assertTrue(directory.read() is ProviderDirectoryResult.Found)

        time = 2_000
        delegate.next = { throw IllegalStateException("socket exploded") }
        val replayed = directory.read()
        assertTrue("a throw must not skip the replay", replayed is ProviderDirectoryResult.Found)
        assertEquals(
            DirectoryObservation.Stored,
            (replayed as ProviderDirectoryResult.Found).observation,
        )
    }

    /** With nothing stored, that same throw degrades to `unavailable` and never to `empty`. */
    @Test
    fun anUnexpectedThrowWithNothingStoredIsUnavailableAndNamesItself() = runBlocking {
        val result = CachedProviderDirectory(
            delegate = FakeDirectory(next = { throw IllegalStateException("socket exploded") }),
            store = MemoryProviderDirectoryCacheStore(),
            now = { 1_000 },
        ).read()
        result as ProviderDirectoryResult.Unavailable
        assertEquals(DirectoryUnavailableReason.SourceUnavailable, result.reason)
        assertTrue(result.detail.contains("threw instead of resolving unavailable"))
        assertTrue(result.detail.contains("socket exploded"))
    }

    /**
     * `CancellationException` is a plain `Exception` on the JVM, so a blanket catch swallows it and
     * turns "the owner left the screen" into a fabricated source failure that then spends a replay.
     */
    @Test
    fun aCancelledReadPropagatesRatherThanFakingASourceFailure() {
        val directory = CachedProviderDirectory(
            delegate = FakeDirectory(next = { throw CancellationException("left the screen") }),
            store = MemoryProviderDirectoryCacheStore(),
            now = { 1_000 },
        )
        assertThrows(CancellationException::class.java) { runBlocking { directory.read() } }
    }

    /**
     * Review finding 2. The namespace must actually distinguish deployments, or one backend's
     * providers are replayed as another's. It is derived from the configured endpoint, so repointing
     * `CENTRAL_API` changes it.
     */
    @Test
    fun aStoredCopyIsNeverReplayedForADifferentlyConfiguredDeployment() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        var time = 1_000L
        CachedProviderDirectory(
            delegate = FakeDirectory(cacheNamespace = "central:https://a.test/v1/businesses") {
                found(1_000)
            },
            store = store,
            ttlMs = 10_000,
            now = { time },
        ).read()
        assertNotNull(store.read())

        time = 2_000
        val other = CachedProviderDirectory(
            delegate = FakeDirectory(cacheNamespace = "central:https://b.test/v1/businesses") {
                unavailable(2_000)
            },
            store = store,
            ttlMs = 10_000,
            now = { time },
        ).read()
        assertTrue("a foreign deployment's copy is not a cache hit", other is ProviderDirectoryResult.Unavailable)
        assertNull("and the mismatched entry is dropped", store.read())
    }

    /** Two real central endpoints really do produce two namespaces. */
    @Test
    fun theCentralNamespaceFollowsTheConfiguredEndpoint() {
        assertEquals(
            "central:https://a.test/v1/businesses",
            CentralProviderDirectory("https://a.test").cacheNamespace,
        )
        assertTrue(
            CentralProviderDirectory("https://a.test").cacheNamespace !=
                CentralProviderDirectory("https://b.test").cacheNamespace,
        )
    }

    // ---- TTL semantics -------------------------------------------------------------------------

    /** Re-check first: a reachable source is always asked, and its answer always wins. */
    @Test
    fun aLiveReadIsAlwaysAttemptedAndAlwaysReplacesTheStoredCopy() = runBlocking {
        var time = 1_000L
        val delegate = FakeDirectory(next = { found(time) })
        val directory = CachedProviderDirectory(
            delegate = delegate,
            store = MemoryProviderDirectoryCacheStore(),
            ttlMs = 10_000,
            now = { time },
        )
        directory.read()
        time = 2_000
        val second = directory.read() as ProviderDirectoryResult.Found
        assertEquals(2, delegate.reads)
        assertEquals(DirectoryObservation.Live, second.observation)
        assertEquals(2_000, second.readAt)
        assertEquals(12_000L, second.expiresAt)
    }

    /** Expiry is at the exact boundary: usable strictly before it, expired the instant it arrives. */
    @Test
    fun anEntryExpiresAtItsExactDeadlineNotAfterIt() = runBlocking {
        var time = 1_000L
        val delegate = FakeDirectory(next = { found(1_000) })
        val store = MemoryProviderDirectoryCacheStore()
        val directory = CachedProviderDirectory(delegate, store, ttlMs = 1_000, now = { time })
        directory.read()

        delegate.next = { unavailable(time) }
        time = 1_999
        assertTrue(directory.read() is ProviderDirectoryResult.Found)

        time = 2_000
        assertTrue(
            "at the deadline itself the entry is expired",
            directory.read() is ProviderDirectoryResult.Unavailable,
        )
        assertNull(store.read())
    }

    /**
     * A cache that renewed its deadline on every replay would never expire, so an offline phone
     * would keep showing a stale directory forever while calling it merely "stored".
     */
    @Test
    fun aStoredReplayNeverRenewsTheHardDeadline() = runBlocking {
        var time = 1_000L
        val delegate = FakeDirectory(next = { found(1_000) })
        val directory = CachedProviderDirectory(
            delegate = delegate,
            store = MemoryProviderDirectoryCacheStore(),
            ttlMs = 1_000,
            now = { time },
        )
        directory.read()
        delegate.next = { unavailable(time) }

        for (instant in listOf(1_200L, 1_500L, 1_900L)) {
            time = instant
            val replay = directory.read() as ProviderDirectoryResult.Found
            assertEquals("the deadline is fixed at the original read", 2_000L, replay.expiresAt)
            assertEquals(1_000, replay.readAt)
        }

        time = 2_000
        assertTrue(directory.read() is ProviderDirectoryResult.Unavailable)
    }

    /**
     * A backwards clock jump must not extend the window. The stored deadline is absolute, so a
     * snapshot "read" in the future would otherwise stay usable for however far the clock moved.
     */
    @Test
    fun aSnapshotReadInTheFutureIsDroppedRatherThanTrusted() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        store.write(
            ProviderDirectoryCacheCodec.encode(
                ProviderDirectoryCacheEntry(
                    namespace = "central:https://central.test/v1/businesses",
                    snapshot = found(9_000, 19_000),
                    readAt = 9_000,
                    expiresAt = 19_000,
                ),
            ),
        )
        val result = CachedProviderDirectory(
            delegate = FakeDirectory(next = { unavailable(1_000) }),
            store = store,
            ttlMs = 10_000,
            now = { 1_000 },
        ).read()
        assertTrue(result is ProviderDirectoryResult.Unavailable)
        assertNull(store.read())
    }

    /**
     * Shortening the TTL in a later build discards previously-stored entries rather than clamping
     * them. That is fail-closed, and it is what keeps "a replay never renews" true across upgrades.
     */
    @Test
    fun aStoredDeadlineLongerThanTheCurrentTtlIsRefused() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        store.write(
            ProviderDirectoryCacheCodec.encode(
                ProviderDirectoryCacheEntry(
                    namespace = "central:https://central.test/v1/businesses",
                    snapshot = found(1_000, 100_000),
                    readAt = 1_000,
                    expiresAt = 100_000,
                ),
            ),
        )
        val result = CachedProviderDirectory(
            delegate = FakeDirectory(next = { unavailable(2_000) }),
            store = store,
            ttlMs = 1_000,
            now = { 2_000 },
        ).read()
        assertTrue(result is ProviderDirectoryResult.Unavailable)
    }

    /**
     * An inner wrapper's replay is not a successful refresh. Storing it would let stacked wrappers
     * renew stale data indefinitely, which is the same never-expires defect by another route.
     */
    @Test
    fun aNestedStoredResultIsPassedThroughAndNotRestored() = runBlocking {
        val store = MemoryProviderDirectoryCacheStore()
        val nested = ProviderDirectoryResult.Found(
            providers = listOf(located),
            observation = DirectoryObservation.Stored,
            readAt = 1_000,
            expiresAt = 2_000,
        )
        val passed = CachedProviderDirectory(
            delegate = FakeDirectory(next = { nested }),
            store = store,
            ttlMs = 10_000,
            now = { 1_500 },
        ).read()
        assertEquals(nested, passed)
        assertNull("a replay must not be written back as a fresh copy", store.read())
    }

    /** A stored result whose own deadline has already passed is not a usable answer. */
    @Test
    fun aNestedStoredResultPastItsDeadlineIsRefused() = runBlocking {
        val nested = ProviderDirectoryResult.Found(
            providers = listOf(located),
            observation = DirectoryObservation.Stored,
            readAt = 1_000,
            expiresAt = 2_000,
        )
        val result = CachedProviderDirectory(
            delegate = FakeDirectory(next = { nested }),
            store = MemoryProviderDirectoryCacheStore(),
            ttlMs = 10_000,
            now = { 2_000 },
        ).read()
        assertEquals(
            DirectoryUnavailableReason.InvalidSnapshot,
            (result as ProviderDirectoryResult.Unavailable).reason,
        )
    }

    // ---- the stored document -------------------------------------------------------------------

    /** Everything a row carries survives a round trip, absence included. */
    @Test
    fun theStoredDocumentRoundTripsEveryFieldIncludingAbsentLocation() {
        val entry = ProviderDirectoryCacheEntry(
            namespace = "central:https://central.test/v1/businesses",
            snapshot = found(1_000, 5_000),
            readAt = 1_000,
            expiresAt = 5_000,
        )
        val decoded = ProviderDirectoryCacheCodec.decode(
            ProviderDirectoryCacheCodec.encode(entry),
        )
        assertNotNull(decoded)
        val snapshot = decoded!!.snapshot as ProviderDirectoryResult.Found
        assertEquals(entry.namespace, decoded.namespace)
        assertEquals(2, snapshot.providers.size)

        val first = snapshot.providers.first()
        assertEquals(GeoPoint(1.3039, 103.8318), first.geo)
        assertEquals("+65 6123 4567", first.contact.phone)
        assertEquals(null, first.active)

        val second = snapshot.providers.last()
        assertNull("a contact-only provider stays location-less", second.geo)
        assertEquals("https://call-only.test", second.contact.website)
        assertTrue(second.contact.hasAny)
        assertTrue(second.bindingState === IssuerBindingState.NoDomainListed)
    }

    /**
     * `0,0` is a legal coordinate off the coast of Ghana and was, before S-1, what a blank location
     * became. Writing absence as `0,0` here would resurrect that pin from the phone's own storage.
     */
    @Test
    fun anAbsentLocationIsStoredAsAbsentNeverAsTheRealCoordinateZeroZero() {
        val document = ProviderDirectoryCacheCodec.encode(
            ProviderDirectoryCacheEntry(
                namespace = "n",
                snapshot = ProviderDirectoryResult.Found(
                    providers = listOf(contactOnly),
                    observation = DirectoryObservation.Live,
                    readAt = 1_000,
                    expiresAt = 5_000,
                ),
                readAt = 1_000,
                expiresAt = 5_000,
            ),
        )
        val row = JSONObject(document).getJSONArray("providers").getJSONObject(0)
        assertTrue("absence is written as absence", !row.has("geo") || row.isNull("geo"))
    }

    /** A replay is a replay however the document is edited: observation is never persisted. */
    @Test
    fun aStoredDocumentCannotDeclareItselfLive() {
        val document = JSONObject(
            ProviderDirectoryCacheCodec.encode(
                ProviderDirectoryCacheEntry("n", found(1_000, 5_000), 1_000, 5_000),
            ),
        )
        assertTrue("observation is not part of the stored shape", !document.has("observation"))
        document.put("observation", "live")
        val decoded = ProviderDirectoryCacheCodec.decode(document.toString())!!
        assertEquals(
            DirectoryObservation.Stored,
            (decoded.snapshot as ProviderDirectoryResult.Found).observation,
        )
    }

    /**
     * A directory source performs no DNS or chain check, so a stored `Verified` would be a claim
     * about work nobody did. It cannot be written and cannot be read back.
     */
    @Test
    fun aStoredBindingStateCanNeverClaimVerified() {
        val document = JSONObject(
            ProviderDirectoryCacheCodec.encode(
                ProviderDirectoryCacheEntry("n", found(1_000, 5_000), 1_000, 5_000),
            ),
        )
        document.getJSONArray("providers").getJSONObject(0).put("bindingState", "verified")
        val decoded = ProviderDirectoryCacheCodec.decode(document.toString())!!
        val provider = (decoded.snapshot as ProviderDirectoryResult.Found).providers.first()
        assertTrue(provider.bindingState === IssuerBindingState.Unavailable)
        assertTrue(provider.bindingState !is IssuerBindingState.Verified)
    }

    /**
     * A document from an older stored shape is dropped, not migrated. Pre-S-1 rows encode a blank
     * location as `0,0`, which AGENTS.md records as unreinterpretable by code.
     */
    @Test
    fun aDocumentFromAnEarlierStoredShapeIsDroppedRatherThanMigrated() = runBlocking {
        val document = JSONObject(
            ProviderDirectoryCacheCodec.encode(
                ProviderDirectoryCacheEntry(
                    namespace = "central:https://central.test/v1/businesses",
                    snapshot = found(1_000, 5_000),
                    readAt = 1_000,
                    expiresAt = 5_000,
                ),
            ),
        )
        document.put("version", ProviderDirectoryCacheCodec.VERSION - 1)
        assertNull(ProviderDirectoryCacheCodec.decode(document.toString()))

        val store = MemoryProviderDirectoryCacheStore()
        store.write(document.toString())
        val result = CachedProviderDirectory(
            delegate = FakeDirectory(next = { unavailable(2_000) }),
            store = store,
            ttlMs = 10_000,
            now = { 2_000 },
        ).read()
        assertTrue(result is ProviderDirectoryResult.Unavailable)
    }

    /** Garbage on disk is "nothing stored", never a guess at what it might have meant. */
    @Test
    fun anUnreadableDocumentIsNothingStored() {
        assertNull(ProviderDirectoryCacheCodec.decode("not json at all"))
        assertNull(ProviderDirectoryCacheCodec.decode("{}"))
        assertNull(ProviderDirectoryCacheCodec.decode("""{"version":1,"namespace":"n"}"""))
    }

    /** A stored `empty` may not smuggle providers in beside it. */
    @Test
    fun aStoredEmptyCarryingProvidersIsMalformed() {
        val document = JSONObject(
            ProviderDirectoryCacheCodec.encode(
                ProviderDirectoryCacheEntry("n", found(1_000, 5_000), 1_000, 5_000),
            ),
        )
        document.put("state", "empty")
        assertNull(ProviderDirectoryCacheCodec.decode(document.toString()))
    }

    /** `unavailable` is not a snapshot: it is never stored, so it can never be replayed as one. */
    @Test
    fun unavailableIsNeverAWellFormedSnapshot() {
        assertTrue(!providerDirectorySnapshotIsWellFormed(unavailable(1_000)))
    }

    /**
     * Pinned so the offline window cannot drift back silently, the way this class first inherited
     * fifteen minutes from the in-process snapshot it replaced. It bounds ONLY how long an offline
     * owner may be shown a remembered directory - the wrapper re-checks live on every read - so
     * shortening it buys no freshness and only cuts that owner off sooner.
     */
    @Test
    fun theStoredCopyUsesTheSharedSevenDayOfflineWindow() {
        assertEquals(7L * 24 * 60 * 60 * 1_000, CachedProviderDirectory.DEFAULT_TTL_MS)
    }

    @Test
    fun aNonPositiveTtlIsRefusedRatherThanSilentlyDisablingTheCopy() {
        for (ttl in listOf(0L, -1L)) {
            assertThrows(IllegalArgumentException::class.java) {
                CachedProviderDirectory(
                    delegate = FakeDirectory(next = { found(1_000) }),
                    store = MemoryProviderDirectoryCacheStore(),
                    ttlMs = ttl,
                )
            }
        }
    }

    // ---- keeping the copy's own failures off the live answer -------------------------------------

    /** A store that fails on the touch named by [failing], and answers normally otherwise. */
    private class FailingStore(
        private val failing: Set<String>,
        seed: String? = null,
        private val error: () -> Throwable = { IllegalStateException("store fault") },
    ) : ProviderDirectoryCacheStore {
        private val inner = MemoryProviderDirectoryCacheStore().also { store ->
            seed?.let(store::write)
        }
        var clears = 0
            private set

        override fun read(): String? {
            if ("read" in failing) throw error()
            return inner.read()
        }

        override fun write(document: String) {
            if ("write" in failing) throw error()
            inner.write(document)
        }

        override fun clear() {
            clears += 1
            if ("clear" in failing) throw error()
            inner.clear()
        }
    }

    /**
     * Nothing on the wrapper path rejects a non-finite coordinate - the well-formed check reads only
     * the timestamp and the provider count, and the range check lives in the SOURCE, which is the
     * seam a second directory replaces. So a snapshot the codec cannot express must cost the stored
     * copy and nothing else: an owner who was successfully served a directory may not be handed a
     * crash because the phone could not write it down.
     */
    @Test
    fun aSnapshotThisCodecCannotExpressCostsTheStoredCopyNotTheLiveAnswer() = runBlocking {
        val unencodable = ProviderDirectoryResult.Found(
            providers = listOf(located.copy(geo = GeoPoint(Double.NaN, Double.NaN))),
            observation = DirectoryObservation.Live,
            readAt = 1_000,
            expiresAt = null,
        )
        // Pinned so this case cannot pass for the wrong reason: without a throwing encode the
        // assertions below would hold whether or not the guard exists. The exception TYPE is pinned
        // too, so an unrelated throw from some other line cannot stand in for the one being guarded.
        assertTrue(providerDirectorySnapshotIsWellFormed(unencodable))
        assertThrows(JSONException::class.java) {
            ProviderDirectoryCacheCodec.encode(
                ProviderDirectoryCacheEntry("n", unencodable, 1_000, 11_000),
            )
        }

        val store = MemoryProviderDirectoryCacheStore()
        val result = CachedProviderDirectory(
            delegate = FakeDirectory(next = { unencodable }),
            store = store,
            ttlMs = 10_000,
            now = { 1_000 },
        ).read()
        assertTrue(
            "an unstorable snapshot is still a live answer",
            result is ProviderDirectoryResult.Found,
        )
        assertEquals(
            DirectoryObservation.Live,
            (result as ProviderDirectoryResult.Found).observation,
        )
        assertNull("a failed encode must leave no partial document behind", store.read())
    }

    /** Same rule one layer down: a store that cannot persist the copy does not fail the read. */
    @Test
    fun aStoreThatCannotWriteDoesNotFailTheLiveRead() = runBlocking {
        val result = CachedProviderDirectory(
            delegate = FakeDirectory(next = { found(1_000) }),
            store = FailingStore(failing = setOf("write")),
            ttlMs = 10_000,
            now = { 1_000 },
        ).read()
        assertTrue(result is ProviderDirectoryResult.Found)
        assertEquals(
            DirectoryObservation.Live,
            (result as ProviderDirectoryResult.Found).observation,
        )
    }

    /**
     * The offline path is the one this whole copy exists for, so a store that cannot be read must
     * degrade to the live `unavailable` - never a throw the screen has to survive, and never `empty`.
     */
    @Test
    fun aStoreThatCannotBeReadDegradesToUnavailableRatherThanEmpty() = runBlocking {
        // A good document is seeded so the classification choice is visible: a failed read is Absent,
        // which leaves the store alone, rather than Unreadable, which would clear it and throw away a
        // copy that may be perfectly fine over what may be a transient fault.
        val store = FailingStore(
            failing = setOf("read"),
            seed = ProviderDirectoryCacheCodec.encode(
                ProviderDirectoryCacheEntry(
                    namespace = "central:https://central.test/v1/businesses",
                    snapshot = found(1_000, 11_000),
                    readAt = 1_000,
                    expiresAt = 11_000,
                ),
            ),
        )
        val result = CachedProviderDirectory(
            delegate = FakeDirectory(next = { unavailable(2_000) }),
            store = store,
            ttlMs = 10_000,
            now = { 2_000 },
        ).read()
        result as ProviderDirectoryResult.Unavailable
        assertEquals(DirectoryUnavailableReason.SourceUnavailable, result.reason)
        assertEquals("a failed read must not discard the stored copy", 0, store.clears)
    }

    /** A store that cannot be cleared is best effort too: every replay re-validates anyway. */
    @Test
    fun aStoreThatCannotBeClearedDoesNotFailTheRead() = runBlocking {
        val result = CachedProviderDirectory(
            delegate = FakeDirectory(next = { emptyFound(1_000) }),
            store = FailingStore(failing = setOf("clear")),
            ttlMs = 10_000,
            now = { 1_000 },
        ).read()
        result as ProviderDirectoryResult.Unavailable
        assertEquals(DirectoryUnavailableReason.InvalidSnapshot, result.reason)
    }

    /**
     * The guards above must not become a blanket `runCatching`: `CancellationException` is a
     * `RuntimeException` on the JVM, so swallowing it here would report the owner leaving the screen
     * as a stored-copy fault and let the read continue on their behalf.
     *
     * Each touch needs the live result that actually reaches it: a failed live read reaches `read`, a
     * good one reaches `write`, and a snapshot the guard refuses reaches `clear`.
     */
    @Test
    fun aCancelledStoreTouchPropagatesRatherThanBeingSwallowed() {
        val cases: List<Pair<String, () -> ProviderDirectoryResult>> = listOf(
            "read" to { unavailable(1_000) },
            "write" to { found(1_000) },
            "clear" to { emptyFound(1_000) },
        )
        for ((touch, live) in cases) {
            val directory = CachedProviderDirectory(
                delegate = FakeDirectory(next = live),
                store = FailingStore(
                    failing = setOf(touch),
                    error = { CancellationException("left the screen") },
                ),
                ttlMs = 10_000,
                now = { 1_000 },
            )
            assertThrows(
                "a cancelled $touch must not be swallowed",
                CancellationException::class.java,
            ) { runBlocking { directory.read() } }
        }
    }
}
