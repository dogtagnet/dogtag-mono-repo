package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.net.IssuerBindingState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * The on-device local copy of the provider directory.
 *
 * This is the Kotlin port of `packages/ui/src/directory/cache.ts`, and its semantics are that file's,
 * not new ones: `found | empty | unavailable`, `live | stored`, re-check-first, expiry at the exact
 * boundary, and a stored replay that never renews its own deadline.
 *
 * What it buys that the previous in-process snapshot did not: it survives the process. A cache held
 * in a field is empty on every cold launch, which is exactly the state a phone is in when the owner
 * opens the app somewhere with no signal - so it did nothing in the one case the captain asked for.
 */

/** Persistence seam. Pure policy lives above it so the whole decision is JVM-testable. */
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
 * unlike the owner-secret store this holds one public endpoint's response, contains no owner position
 * and nothing the phone could not fetch again, so borrowing those flags would misstate its
 * sensitivity. It is in the cache dir, so the OS may evict it at any time.
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

/** A stored snapshot with the identity needed to decide whether it may be replayed at all. */
data class ProviderDirectoryCacheEntry(
    val namespace: String,
    val snapshot: ProviderDirectoryResult,
    val readAt: Long,
    val expiresAt: Long,
)

/**
 * The integrity a snapshot must have to be written OR replayed.
 *
 * Running it on the replay path is the load-bearing half here, and more so than on web. TypeScript
 * makes a non-empty `found` unrepresentable (`readonly [P, ...P[]]`); Kotlin's `List` cannot express
 * that, so a `found` carrying zero providers is possible the moment a snapshot arrives from disk
 * rather than from the live path - and a `found` with no providers renders as "no vets near you",
 * which is precisely the false absence this module exists to prevent. The guard is the only thing
 * standing there.
 */
fun providerDirectorySnapshotIsWellFormed(result: ProviderDirectoryResult): Boolean = when (result) {
    is ProviderDirectoryResult.Found -> result.readAt >= 0 && result.providers.isNotEmpty()
    is ProviderDirectoryResult.Empty -> result.readAt >= 0
    // `unavailable` is not a snapshot. It is never stored and never replayed.
    is ProviderDirectoryResult.Unavailable -> false
}

/**
 * Re-check the directory, replaying the last successful snapshot only when the live read could not
 * answer and the hard TTL has not elapsed.
 *
 * A successful LIVE read replaces the stored copy. A stored replay never renews its hard deadline.
 * An entry AT its exact expiry is expired. Missing, wrong-namespace, and expired entries all leave
 * the live `unavailable` exactly as it was - none of them may turn it into `empty`.
 */
class CachedProviderDirectory(
    private val delegate: ProviderDirectory,
    private val store: ProviderDirectoryCacheStore,
    private val ttlMs: Long = DEFAULT_TTL_MS,
    private val now: () -> Long = System::currentTimeMillis,
) : ProviderDirectory {
    init {
        require(ttlMs > 0) { "provider directory cache ttlMs must be greater than zero" }
    }

    override val cacheNamespace: String get() = delegate.cacheNamespace

    // Every store touch hops to IO. `suspend` does NOT change dispatcher - it runs on the caller's,
    // and `NearbyScreen`'s `LaunchedEffect` is on Main. The delegate was safe there only because
    // `Http.getJson` does its own `withContext(Dispatchers.IO)`; a synchronous file read added beside
    // it would have been disk I/O on the UI thread. Kept out of `ProviderDirectoryCacheStore` itself
    // so the seam stays a plain non-suspend interface for the in-memory store and the tests.
    private suspend fun storedDocument(): String? = withContext(Dispatchers.IO) { store.read() }

    private suspend fun storeDocument(document: String) =
        withContext(Dispatchers.IO) { store.write(document) }

    private suspend fun clearStore() = withContext(Dispatchers.IO) { store.clear() }

    override suspend fun read(): ProviderDirectoryResult {
        val live = try {
            delegate.read()
        } catch (cancelled: CancellationException) {
            // The caller walked away. Not a directory failure, so it must not spend a replay.
            throw cancelled
        } catch (error: Exception) {
            // The seam only ASKS an implementation to resolve rather than throw, and Kotlin has no
            // checked exceptions to enforce it. Letting a throw escape here would skip the replay
            // branch below in exactly the unexpected-failure case the stored copy exists for.
            ProviderDirectoryResult.Unavailable(
                reason = DirectoryUnavailableReason.SourceUnavailable,
                detail = threwDetail(error),
                attemptedAt = now(),
            )
        }

        val currentTime = now()

        if (live is ProviderDirectoryResult.Unavailable) return replay(live, currentTime)

        if (!providerDirectorySnapshotIsWellFormed(live)) {
            clearStore()
            return ProviderDirectoryResult.Unavailable(
                reason = DirectoryUnavailableReason.InvalidSnapshot,
                detail = "The directory returned an invalid provider list or timestamp",
                attemptedAt = currentTime,
            )
        }

        val observation = observationOf(live)
        val readAt = readAtOf(live)
        if (observation == DirectoryObservation.Stored) {
            // A replay handed up by an inner wrapper is not a successful refresh. Passing it through
            // preserves the original hard deadline; storing it here would let stacked wrappers renew
            // stale data forever.
            val inherited = expiresAtOf(live)
            if (inherited == null || currentTime >= inherited) {
                return ProviderDirectoryResult.Unavailable(
                    reason = DirectoryUnavailableReason.InvalidSnapshot,
                    detail = "The directory returned a stored snapshot without an unexpired hard TTL",
                    attemptedAt = currentTime,
                )
            }
            return live
        }

        // The TTL runs from the SOURCE's observation, not from insertion, so a slow paged read or an
        // outer wrapper can never silently lengthen the hard maximum age.
        val localExpiry = readAt + ttlMs
        val declared = expiresAtOf(live)
        val expiresAt = if (declared == null) localExpiry else minOf(localExpiry, declared)
        if (currentTime >= expiresAt) {
            clearStore()
            return ProviderDirectoryResult.Unavailable(
                reason = DirectoryUnavailableReason.InvalidSnapshot,
                detail = "The live directory snapshot was already outside its hard TTL",
                attemptedAt = currentTime,
            )
        }

        val stamped = withExpiry(live, expiresAt)
        storeDocument(
            ProviderDirectoryCacheCodec.encode(
                ProviderDirectoryCacheEntry(
                    namespace = delegate.cacheNamespace,
                    snapshot = stamped,
                    readAt = readAt,
                    expiresAt = expiresAt,
                ),
            ),
        )
        return stamped
    }

    private suspend fun replay(
        live: ProviderDirectoryResult.Unavailable,
        currentTime: Long,
    ): ProviderDirectoryResult {
        val document = storedDocument() ?: return live
        val entry = ProviderDirectoryCacheCodec.decode(document)
        if (entry == null || entry.namespace != delegate.cacheNamespace) {
            // A source swap is a trust-boundary change, not a cache hit.
            clearStore()
            return live
        }
        if (
            !providerDirectorySnapshotIsWellFormed(entry.snapshot) ||
            readAtOf(entry.snapshot) != entry.readAt ||
            expiresAtOf(entry.snapshot) != entry.expiresAt ||
            // A snapshot read in the future means the clock moved backwards. Trusting the stored
            // absolute deadline there would extend the hard window by however far it jumped.
            entry.readAt > currentTime ||
            entry.expiresAt > entry.readAt + ttlMs ||
            currentTime >= entry.expiresAt
        ) {
            clearStore()
            return live
        }
        return asStored(entry.snapshot)
    }

    private fun threwDetail(error: Exception): String {
        val message = error.message?.trim().orEmpty()
        val suffix = if (message.isEmpty()) "" else ": $message"
        return "The directory threw instead of resolving unavailable$suffix"
    }

    companion object {
        const val DEFAULT_TTL_MS: Long = 15 * 60 * 1_000L

        internal fun observationOf(result: ProviderDirectoryResult): DirectoryObservation? =
            when (result) {
                is ProviderDirectoryResult.Found -> result.observation
                is ProviderDirectoryResult.Empty -> result.observation
                is ProviderDirectoryResult.Unavailable -> null
            }

        internal fun readAtOf(result: ProviderDirectoryResult): Long = when (result) {
            is ProviderDirectoryResult.Found -> result.readAt
            is ProviderDirectoryResult.Empty -> result.readAt
            is ProviderDirectoryResult.Unavailable -> result.attemptedAt
        }

        internal fun expiresAtOf(result: ProviderDirectoryResult): Long? = when (result) {
            is ProviderDirectoryResult.Found -> result.expiresAt
            is ProviderDirectoryResult.Empty -> result.expiresAt
            is ProviderDirectoryResult.Unavailable -> null
        }

        internal fun withExpiry(
            result: ProviderDirectoryResult,
            expiresAt: Long,
        ): ProviderDirectoryResult = when (result) {
            is ProviderDirectoryResult.Found -> result.copy(expiresAt = expiresAt)
            is ProviderDirectoryResult.Empty -> result.copy(expiresAt = expiresAt)
            is ProviderDirectoryResult.Unavailable -> result
        }

        internal fun asStored(result: ProviderDirectoryResult): ProviderDirectoryResult =
            when (result) {
                is ProviderDirectoryResult.Found ->
                    result.copy(observation = DirectoryObservation.Stored)
                is ProviderDirectoryResult.Empty ->
                    result.copy(observation = DirectoryObservation.Stored)
                is ProviderDirectoryResult.Unavailable -> result
            }
    }
}

/**
 * The stored document's wire form.
 *
 * Everything decodes strictly and any failure is `null`, which the caller reads as "nothing stored".
 * A cache that guessed at a half-understood document would be inventing directory content.
 */
object ProviderDirectoryCacheCodec {
    /**
     * Bump on ANY change to the stored shape, including a change to what a field means.
     *
     * A stale-version document is dropped rather than migrated. The concrete reason this exists from
     * day one: S-1 made `geo` nullable, and before it a location-less provider was persisted as the
     * real coordinate `0,0`. AGENTS.md is explicit that such a row cannot be safely reinterpreted by
     * code, so a stored copy from an older shape must never be replayed as if it were this one.
     */
    const val VERSION = 1

    fun encode(entry: ProviderDirectoryCacheEntry): String {
        val root = JSONObject()
        root.put("version", VERSION)
        root.put("namespace", entry.namespace)
        root.put("readAt", entry.readAt)
        root.put("expiresAt", entry.expiresAt)
        root.put(
            "state",
            when (entry.snapshot) {
                is ProviderDirectoryResult.Found -> "found"
                is ProviderDirectoryResult.Empty -> "empty"
                is ProviderDirectoryResult.Unavailable -> "unavailable"
            },
        )
        val providers = JSONArray()
        if (entry.snapshot is ProviderDirectoryResult.Found) {
            entry.snapshot.providers.forEach { providers.put(encodeProvider(it)) }
        }
        root.put("providers", providers)
        return root.toString()
    }

    fun decode(document: String): ProviderDirectoryCacheEntry? = try {
        val root = JSONObject(document)
        if (root.optInt("version", -1) != VERSION) {
            null
        } else {
            val namespace = root.optString("namespace", "")
            val readAt = requireLong(root, "readAt")
            val expiresAt = requireLong(root, "expiresAt")
            val providersJson = root.opt("providers") as? JSONArray
                ?: throw MalformedCache("providers is not an array")
            val providers = (0 until providersJson.length()).map { index ->
                decodeProvider(
                    providersJson.opt(index) as? JSONObject
                        ?: throw MalformedCache("providers[$index] is not an object"),
                )
            }
            if (namespace.isEmpty()) throw MalformedCache("namespace is missing")
            val snapshot = when (root.optString("state", "")) {
                // Observation is not stored: a document read back from disk is a replay by
                // definition, and the wrapper relabels it. Persisting the field would let a
                // hand-edited `"live"` present a remembered answer as a fresh one.
                "found" -> ProviderDirectoryResult.Found(
                    providers = providers,
                    observation = DirectoryObservation.Stored,
                    readAt = readAt,
                    expiresAt = expiresAt,
                )
                "empty" -> {
                    if (providers.isNotEmpty()) throw MalformedCache("empty carries providers")
                    ProviderDirectoryResult.Empty(
                        observation = DirectoryObservation.Stored,
                        readAt = readAt,
                        expiresAt = expiresAt,
                    )
                }
                else -> throw MalformedCache("unrecognised stored state")
            }
            ProviderDirectoryCacheEntry(namespace, snapshot, readAt, expiresAt)
        }
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Exception) {
        null
    }

    private fun encodeProvider(provider: DirectoryProvider): JSONObject {
        val row = JSONObject()
        row.put("providerId", provider.providerId)
        row.put("kind", provider.kind)
        row.put("name", provider.name)
        // Absence is stored as absence. It is never written as `0,0`, which is a real coordinate off
        // the coast of Ghana and would place a contact-only provider on the map.
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
