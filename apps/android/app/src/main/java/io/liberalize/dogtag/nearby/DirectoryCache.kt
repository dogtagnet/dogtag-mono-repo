package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.net.IssuerBindingState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * The on-device local copy of provider RECORDS - never of a ranking.
 *
 * Captain's ruling, 2026-07-30: "its purpose is only for UX, so that some cache results can be shown
 * when the device is offline, keep the cache data to minimal, and minimal usage." So this is a
 * fallback that stops the screen being blank, not a feature, and it is deliberately narrower than the
 * full-set snapshot cache it replaces (slice S-3, PR #112).
 *
 * What it stores: provider identity and published contact details for the providers the owner most
 * recently saw. What it must NEVER store: the nearest ordering, a distance, or anything else derived
 * from the owner's position. Two reasons, and the second is the one that bites:
 *
 *  1. Persisting a position derivative would put the owner's whereabouts on disk, which the ruling
 *     that introduced server-side nearest explicitly refused ("the position MUST NOT be logged, and
 *     MUST NOT be persisted").
 *  2. A stale ranking replayed offline would present "nearest to you" ordering computed from a
 *     position the owner no longer occupies, as though it were current. That is the could-not-check
 *     rendered as a definite answer that this fleet has spent a week removing, and unlike a stale
 *     phone number it is invisible: the list simply looks sorted.
 *
 * So offline the owner sees remembered providers UNRANKED, labelled as stored rather than current,
 * and no distance is offered at all. The ordering here is by name, which is position-free - and it is
 * re-applied on READ as well as on write, so a document written by an older build or edited by hand
 * still cannot smuggle a ranking back in through its array order.
 */

/**
 * Persistence seam. Pure policy lives above it so the whole decision is JVM-testable.
 *
 * An implementation is asked to swallow its own failures, and Kotlin has no checked exceptions to
 * enforce that, so [ProviderRecordCache] tolerates a throw from any of the three: a copy that could
 * not be stored, read or cleared may only ever cost a replay, never the live answer the owner is
 * waiting on. Cancellation is the exception and still propagates.
 */
interface ProviderDirectoryCacheStore {
    /** The stored document, or `null` when nothing is stored or it could not be read. */
    fun read(): String?
    fun write(document: String)
    fun clear()
}

/** In-memory store for tests and for a caller that deliberately wants no disk copy. */
class MemoryProviderDirectoryCacheStore : ProviderDirectoryCacheStore {
    private var document: String? = null
    override fun read(): String? = document
    override fun write(document: String) {
        this.document = document
    }

    override fun clear() {
        document = null
    }
}

/**
 * One JSON file. Every failure is swallowed into "nothing stored", which can only ever cost a replay.
 *
 * Writes go to a sibling and are renamed in, so a kill mid-write leaves the previous copy rather than
 * a truncated document. There is no `.completeFileProtection` / backup-exclusion here, deliberately:
 * unlike the owner-secret store this holds public provider listings, contains no owner position and
 * nothing the phone could not fetch again, so borrowing those flags would misstate its sensitivity.
 * It is in the cache dir, so the OS may evict it at any time.
 *
 * The file name is deliberately UNCHANGED from the full-set cache this replaces. Renaming it would
 * strand the old document on disk holding an entire provider set nobody reads or expires; reusing the
 * path means the first write overwrites it, and until then the version bump in
 * [ProviderDirectoryCacheCodec] makes it undecodable, which reads as nothing stored.
 */
class FileProviderDirectoryCacheStore(private val file: File) : ProviderDirectoryCacheStore {
    override fun read(): String? = try {
        if (file.exists()) file.readText().takeIf { it.isNotBlank() } else null
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Exception) {
        null
    }

    override fun write(document: String) {
        try {
            val staging = File(file.parentFile, "${file.name}.tmp")
            staging.writeText(document)
            if (!staging.renameTo(file)) {
                file.writeText(document)
                staging.delete()
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            // A copy that could not be stored simply is not stored. The live result still stands.
        }
    }

    override fun clear() {
        try {
            file.delete()
            File(file.parentFile, "${file.name}.tmp").delete()
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            // Best effort. Every replay re-validates, so a survivor cannot be replayed unchecked.
        }
    }

    companion object {
        const val FILE_NAME = "dogtag-provider-directory.json"
    }
}

/**
 * Remembered provider records plus when they were remembered.
 *
 * There is no `distancesKm` and no page here, and their absence is the type-level half of the ruling:
 * a caller cannot render a stored distance because it has none to render.
 */
data class StoredProviderRecords(
    val providers: List<DirectoryProvider>,
    val storedAt: Long,
)

/**
 * The integrity a remembered record must have to be written OR replayed.
 *
 * Running it on the replay path is the load-bearing half: a record arriving from disk was not
 * produced by the live parser, so a blank id or name is possible there in a way it is not on the live
 * path - and a nameless row renders as an empty list entry the owner cannot act on.
 */
fun providerRecordIsWellFormed(provider: DirectoryProvider): Boolean =
    provider.providerId.isNotBlank() && provider.name.isNotBlank()

/**
 * The position-free order. Applied on write AND on read, so the array order can never carry a
 * ranking. Ties break on `providerId` so the order is total and a page cannot shuffle between reads.
 */
internal fun providerRecordsInStoredOrder(
    providers: List<DirectoryProvider>,
): List<DirectoryProvider> = providers.sortedWith(
    compareBy({ it.name.lowercase() }, { it.providerId }),
)

/**
 * The offline fallback: remember the records the owner just saw, replay them when a live read cannot
 * answer at all.
 *
 * Deliberately NOT a decorator around the directory seam, unlike the `CachedProviderDirectory` it
 * replaces. That shape wrapped a no-argument `read()` of the whole set, which no longer exists: a
 * nearest read is personalized and paged, so there is no single response that is "the directory" to
 * substitute for. The caller therefore drives this explicitly - remember after a live read, recall
 * only when the live read is `Unavailable` - which also keeps the recall out of the success path.
 */
class ProviderRecordCache(
    private val store: ProviderDirectoryCacheStore,
    private val namespace: String,
    private val ttlMillis: Long = DEFAULT_TTL_MILLIS,
    private val maxRecords: Int = MAX_RECORDS,
) {
    /**
     * Replace the remembered set with what the owner is looking at now.
     *
     * Replace rather than accumulate, because "minimal" was the instruction twice over: a union would
     * grow without bound and would remember providers from a city the owner visited once. The
     * consequence is honest and worth stating - offline shows the last providers seen, not a history.
     *
     * An empty live page writes nothing and clears nothing: it means this query matched nothing, which
     * is not evidence that the previously remembered providers ceased to exist. The TTL bounds how
     * long they may survive.
     */
    suspend fun remember(providers: List<DirectoryProvider>, now: Long) {
        val records = providerRecordsInStoredOrder(
            providers.filter(::providerRecordIsWellFormed).distinctBy { it.providerId },
        ).take(maxRecords)
        if (records.isEmpty() || now < 0) return
        val document = ProviderDirectoryCacheCodec.encode(
            ProviderRecordCacheEntry(
                namespace = namespace,
                providers = records,
                storedAt = now,
                expiresAt = now + ttlMillis,
            ),
        )
        withContext(Dispatchers.IO) {
            try {
                store.write(document)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                // Storing is best effort; the live answer the owner already has is unaffected.
            }
        }
    }

    /**
     * The remembered records, or `null` when there is nothing replayable.
     *
     * `null` covers every could-not-answer: nothing stored, an unreadable or undecodable document, a
     * different configured source, an elapsed deadline, and a stored time in the future (a backwards
     * clock jump, which is not a fresh copy). None of them may become an empty success - a caller that
     * turned `null` into "no providers near you" would state an absence nothing established.
     */
    suspend fun recall(now: Long): StoredProviderRecords? {
        val document = withContext(Dispatchers.IO) {
            try {
                store.read()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                null
            }
        } ?: return null
        val entry = ProviderDirectoryCacheCodec.decode(document) ?: return null
        if (entry.namespace != namespace) return null
        // At the exact deadline it is expired, matching the web and the previous cache.
        if (now >= entry.expiresAt) return null
        if (now < entry.storedAt) return null
        val records = providerRecordsInStoredOrder(
            entry.providers.filter(::providerRecordIsWellFormed).distinctBy { it.providerId },
        ).take(maxRecords)
        if (records.isEmpty()) return null
        return StoredProviderRecords(providers = records, storedAt = entry.storedAt)
    }

    suspend fun clear() {
        withContext(Dispatchers.IO) {
            try {
                store.clear()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                // Best effort. Every replay re-validates, so a survivor cannot be replayed unchecked.
            }
        }
    }

    companion object {
        /**
         * How long a remembered set may still be shown offline. It bounds ONLY the fallback - every
         * read re-checks live first - so shortening it buys no freshness and only cuts off an offline
         * owner sooner.
         */
        const val DEFAULT_TTL_MILLIS: Long = 7L * 24 * 60 * 60 * 1000

        /**
         * The cap that makes "minimal" concrete: one page's worth of providers.
         *
         * The whole point of the server-side pivot was that the directory may hold hundreds of
         * thousands of rows, so a cache that grew with the directory would reintroduce on the disk
         * exactly the bulk the ruling removed from the wire.
         */
        const val MAX_RECORDS: Int = DEFAULT_PROVIDER_PAGE_SIZE
    }
}

/** A stored record set with the identity needed to decide whether it may be replayed at all. */
data class ProviderRecordCacheEntry(
    val namespace: String,
    val providers: List<DirectoryProvider>,
    val storedAt: Long,
    val expiresAt: Long,
)

object ProviderDirectoryCacheCodec {
    /**
     * Bump on ANY change to the stored shape, including a change to what a field means.
     *
     * A stale-version document is dropped rather than migrated. Version 1 was slice S-3's full-set
     * snapshot, whose `providers` array was in server order - which for a nearest response IS the
     * ranking. Reading one under this shape would replay that ranking as a remembered record set, so
     * the bump is not bookkeeping: it is what stops the previous build's document being reinterpreted
     * as something it is not. (Version 1 also predates S-1 making `geo` nullable, where a
     * location-less provider was persisted as the real coordinate `0,0`.)
     */
    const val VERSION = 2

    fun encode(entry: ProviderRecordCacheEntry): String {
        val root = JSONObject()
        root.put("version", VERSION)
        root.put("namespace", entry.namespace)
        root.put("storedAt", entry.storedAt)
        root.put("expiresAt", entry.expiresAt)
        val providers = JSONArray()
        entry.providers.forEach { providers.put(encodeProvider(it)) }
        root.put("providers", providers)
        return root.toString()
    }

    fun decode(document: String): ProviderRecordCacheEntry? = try {
        val root = JSONObject(document)
        if (root.optInt("version", -1) != VERSION) {
            null
        } else {
            val namespace = root.optString("namespace", "")
            if (namespace.isEmpty()) throw MalformedCache("namespace is missing")
            val storedAt = requireLong(root, "storedAt")
            val expiresAt = requireLong(root, "expiresAt")
            val providersJson = root.opt("providers") as? JSONArray
                ?: throw MalformedCache("providers is not an array")
            val providers = (0 until providersJson.length()).map { index ->
                decodeProvider(
                    providersJson.opt(index) as? JSONObject
                        ?: throw MalformedCache("providers[$index] is not an object"),
                )
            }
            if (providers.isEmpty()) throw MalformedCache("a stored record set carries no providers")
            ProviderRecordCacheEntry(namespace, providers, storedAt, expiresAt)
        }
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Exception) {
        null
    }

    /**
     * Identity and published contacts only.
     *
     * There is deliberately no `distanceKm` key, and its absence is a requirement rather than an
     * omission: a distance is computed from the owner's position, so writing one would persist a
     * position derivative and let a later read present a distance measured from somewhere the owner
     * no longer is. On Android the server distances arrive in a separate `distancesKm` map on
     * `Found`, so there is nothing on `DirectoryProvider` to accidentally carry here - keep it that
     * way, and if a distance is ever moved onto the row, it must still be dropped here.
     */
    private fun encodeProvider(provider: DirectoryProvider): JSONObject {
        val row = JSONObject()
        row.put("providerId", provider.providerId)
        row.put("kind", provider.kind)
        row.put("name", provider.name)
        // Absence is stored as absence. It is never written as `0,0`, which is a real coordinate off
        // the coast of Ghana and would place a contact-only provider somewhere specific.
        provider.geo?.let { row.put("geo", JSONObject().put("lat", it.lat).put("lng", it.lng)) }
        row.put("services", JSONArray().also { array -> provider.services.forEach(array::put) })
        provider.domain?.let { row.put("domain", it) }
        provider.active?.let { row.put("active", it) }
        val contact = JSONObject()
        provider.contact.phone?.let { contact.put("phone", it) }
        provider.contact.whatsapp?.let { contact.put("whatsapp", it) }
        provider.contact.telegram?.let { contact.put("telegram", it) }
        provider.contact.email?.let { contact.put("email", it) }
        provider.contact.website?.let { contact.put("website", it) }
        row.put("contact", contact)
        row.put("bindingState", encodeBindingState(provider.bindingState))
        return row
    }

    private fun decodeProvider(row: JSONObject): DirectoryProvider {
        val geo = if (!row.has("geo") || row.isNull("geo")) {
            null
        } else {
            val objectValue = row.opt("geo") as? JSONObject
                ?: throw MalformedCache("stored geo is not an object")
            val lat = (objectValue.opt("lat") as? Number)?.toDouble()
                ?: throw MalformedCache("stored latitude is not numeric")
            val lng = (objectValue.opt("lng") as? Number)?.toDouble()
                ?: throw MalformedCache("stored longitude is not numeric")
            GeoPoint(lat, lng).takeIf { it.isUsable }
                ?: throw MalformedCache("stored coordinate is out of range")
        }
        val servicesJson = row.opt("services") as? JSONArray
            ?: throw MalformedCache("stored services is not an array")
        val services = (0 until servicesJson.length()).map { index ->
            servicesJson.opt(index) as? String ?: throw MalformedCache("stored service is not text")
        }
        val contactJson = row.opt("contact") as? JSONObject ?: JSONObject()
        return DirectoryProvider(
            providerId = requiredText(row, "providerId"),
            kind = requiredText(row, "kind"),
            name = requiredText(row, "name"),
            geo = geo,
            services = services,
            domain = optionalText(row, "domain"),
            active = if (!row.has("active") || row.isNull("active")) {
                null
            } else {
                row.opt("active") as? Boolean ?: throw MalformedCache("stored active is not boolean")
            },
            contact = ProviderContact(
                phone = optionalText(contactJson, "phone"),
                whatsapp = optionalText(contactJson, "whatsapp"),
                telegram = optionalText(contactJson, "telegram"),
                email = optionalText(contactJson, "email"),
                website = optionalText(contactJson, "website"),
            ),
            bindingState = decodeBindingState(row.optString("bindingState", "")),
        )
    }

    /**
     * Only the two states a directory source may honestly produce round-trip.
     *
     * A stored `Verified` would be a claim about a DNS/chain check nobody performed, so the encoder
     * cannot write one and the decoder cannot read one - anything unrecognised degrades to
     * `Unavailable`, which asserts nothing.
     */
    private fun encodeBindingState(state: IssuerBindingState): String = when (state) {
        IssuerBindingState.NoDomainListed -> "noDomainListed"
        else -> "unavailable"
    }

    private fun decodeBindingState(raw: String): IssuerBindingState = when (raw) {
        "noDomainListed" -> IssuerBindingState.NoDomainListed
        else -> IssuerBindingState.Unavailable
    }

    private fun requireLong(root: JSONObject, key: String): Long {
        if (!root.has(key) || root.isNull(key)) throw MalformedCache("$key is missing")
        return (root.opt(key) as? Number)?.toLong() ?: throw MalformedCache("$key is not numeric")
    }

    private fun requiredText(row: JSONObject, key: String): String {
        if (!row.has(key) || row.isNull(key)) throw MalformedCache("stored $key is missing")
        return row.opt(key) as? String ?: throw MalformedCache("stored $key is not text")
    }

    private fun optionalText(row: JSONObject, key: String): String? {
        if (!row.has(key) || row.isNull(key)) return null
        val value = row.opt(key) as? String ?: throw MalformedCache("stored $key is not text")
        return value.trim().ifBlank { null }
    }

    private class MalformedCache(message: String) : IllegalArgumentException(message)
}
