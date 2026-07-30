package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.data.AppConfig
import io.liberalize.dogtag.net.Http
import io.liberalize.dogtag.net.IssuerBindingState
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.URI
import java.net.URLEncoder
import java.nio.charset.StandardCharsets

/**
 * A current-position value accepted by the provider-directory request.
 *
 * Captain's ruling, 2026-07-30: the position is sent EXACTLY and is NOT rounded. An earlier revision
 * coarsened both axes to three decimal places here; that is gone, and nothing may reintroduce it
 * silently - a value named "approximate" that is really metre-precise would overstate the privacy the
 * request provides, which is the class of false claim this app refuses everywhere else.
 *
 * The private constructor still earns its keep: it is the one place a raw [GeoPoint] is admitted, so
 * an unusable fix cannot reach the wire. What protects the position now is confinement, not
 * imprecision - body-only, never logged, never stored (see the service's own guarantees).
 */
class CallerPosition private constructor(
    val lat: Double,
    val lng: Double,
) {
    companion object {
        fun from(point: GeoPoint): CallerPosition? {
            if (!point.isUsable) return null
            return CallerPosition(point.lat, point.lng)
        }
    }
}

/** Caller-owned filtering and paging. The service supplies no implicit owner-app kind policy. */
data class ProviderDirectoryQuery(
    val kinds: List<String>,
    val name: String? = null,
    val limit: Int = DEFAULT_PROVIDER_PAGE_SIZE,
    val offset: Int = 0,
) {
    init {
        require(kinds.isNotEmpty()) { "provider kinds must not be empty" }
        require(kinds.all { it.isNotBlank() }) { "provider kinds must not contain blanks" }
        require(limit > 0) { "provider page limit must be positive" }
        require(offset >= 0) { "provider page offset must not be negative" }
    }

    internal fun encodedQuery(): String {
        val fields = ArrayList<Pair<String, String>>()
        kinds.forEach { fields += "kind" to it.trim() }
        fields += "limit" to limit.toString()
        fields += "offset" to offset.toString()
        name?.trim()?.takeIf { it.isNotEmpty() }?.let { fields += "name" to it }
        return fields.joinToString("&") { (key, value) ->
            "${percentEncode(key)}=${percentEncode(value)}"
        }
    }
}

const val DEFAULT_PROVIDER_PAGE_SIZE = 25

/** Native adapter for the central provider-directory search contract. */
interface ProviderDirectory {
    /**
     * Sends the current position in a POST body. The result is server-ranked and carries server
     * distances; callers must preserve that order and must not recompute distance.
     */
    suspend fun nearest(
        position: CallerPosition,
        query: ProviderDirectoryQuery,
    ): ProviderDirectoryResult

    /** Name/contact search. No position is accepted or sent on this path. */
    suspend fun contacts(query: ProviderDirectoryQuery): ProviderDirectoryResult
}

/**
 * Central directory adapter.
 *
 * Personalized nearest responses are deliberately not cached here or persisted anywhere. Each
 * explicit request is sent live, and only the calling screen retains the response while rendering it.
 * The body carries the position so ordinary URL/access logs never receive a coordinate.
 */
class CentralProviderDirectory(
    baseUrl: String,
    private val ttlMs: Long = 15 * 60 * 1_000L,
    private val now: () -> Long = System::currentTimeMillis,
    private val get: suspend (String) -> Http.Response = { Http.getJson(it) },
    private val post: suspend (String, String) -> Http.Response = { url, body ->
        Http.postJson(url, body)
    },
) : ProviderDirectory {
    private val configuredBase = baseUrl.trim().also { configured ->
        require(configured.isNotEmpty()) { "provider directory base URL must not be blank" }
        val parsed = URI(configured)
        require(parsed.query == null && parsed.fragment == null) {
            "provider directory base URL must not contain a query or fragment"
        }
    }.trimEnd('/')

    init {
        require(ttlMs > 0) { "provider directory ttlMs must be greater than zero" }
    }

    override suspend fun nearest(
        position: CallerPosition,
        query: ProviderDirectoryQuery,
    ): ProviderDirectoryResult {
        val url = "$configuredBase/v1/businesses/nearest?${query.encodedQuery()}"
        val body = buildString {
            append("{\"lat\":")
            append(position.lat)
            append(",\"lng\":")
            append(position.lng)
            append('}')
        }
        return request(requireDistance = true) { post(url, body) }
    }

    override suspend fun contacts(query: ProviderDirectoryQuery): ProviderDirectoryResult {
        val url = "$configuredBase/v1/businesses?${query.encodedQuery()}"
        return request(requireDistance = false) { get(url) }
    }

    private suspend fun request(
        requireDistance: Boolean,
        fetch: suspend () -> Http.Response,
    ): ProviderDirectoryResult {
        val attemptedAt = now()
        return try {
            val response = fetch()
            if (!response.ok) {
                ProviderDirectoryResult.Unavailable(
                    reason = DirectoryUnavailableReason.SourceUnavailable,
                    detail = "The provider directory returned HTTP ${response.code}",
                    attemptedAt = attemptedAt,
                )
            } else {
                decode(response.body, attemptedAt, requireDistance)
            }
        } catch (error: Exception) {
            ProviderDirectoryResult.Unavailable(
                reason = DirectoryUnavailableReason.SourceUnavailable,
                detail = error.message?.trim()?.takeIf { it.isNotEmpty() }
                    ?: "The provider directory could not be reached",
                attemptedAt = attemptedAt,
            )
        }
    }

    /**
     * All-or-nothing decode. One malformed provider, page field, or required server distance makes
     * the read unavailable; it never degrades a bad response into a successful empty result.
     */
    internal fun decode(
        json: String,
        readAt: Long,
        requireDistance: Boolean,
    ): ProviderDirectoryResult = try {
        val root = JSONObject(json)
        val rows = root.opt("businesses") as? JSONArray
            ?: throw MalformedDirectory("response has no businesses array")
        val total = root.requiredNonnegativeInt("total")
        val limit = root.requiredNonnegativeInt("limit").also {
            if (it == 0) throw MalformedDirectory("page limit must be positive")
        }
        val offset = root.requiredNonnegativeInt("offset")
        val hasMore = root.requiredBoolean("hasMore")
        if (rows.length() > limit) throw MalformedDirectory("page contains more rows than its limit")
        val pageEnd = offset.toLong() + rows.length().toLong()
        if (rows.length() > 0 && pageEnd > total.toLong()) {
            throw MalformedDirectory("page rows exceed total")
        }
        if (rows.length() == 0 && hasMore) {
            throw MalformedDirectory("empty page cannot advance pagination")
        }
        val expectedHasMore = pageEnd < total.toLong()
        if (hasMore != expectedHasMore) {
            throw MalformedDirectory("page hasMore is inconsistent with total and offset")
        }

        val providers = ArrayList<DirectoryProvider>(rows.length())
        val distances = LinkedHashMap<String, Double>(rows.length())
        val ids = HashSet<String>()
        var previousDistance: Double? = null
        for (index in 0 until rows.length()) {
            val row = rows.opt(index) as? JSONObject
                ?: throw MalformedDirectory("businesses[$index] is not an object")
            val provider = decodeProvider(row)
            if (!ids.add(provider.providerId)) {
                throw MalformedDirectory("businesses contains a duplicate provider id")
            }
            val distance = row.optionalDistance()
            if (requireDistance && distance == null) {
                throw MalformedDirectory("nearest provider has no distanceKm")
            }
            if (requireDistance && provider.geo == null) {
                throw MalformedDirectory("nearest provider has no published location")
            }
            if (distance != null) {
                if (
                    requireDistance &&
                    previousDistance != null &&
                    distance < previousDistance
                ) {
                    throw MalformedDirectory("nearest distances are not nondecreasing")
                }
                distances[provider.providerId] = distance
                previousDistance = distance
            }
            providers += provider
        }

        if (readAt < 0 || readAt > Long.MAX_VALUE - ttlMs) {
            throw MalformedDirectory("directory timestamp is unusable")
        }
        val expiresAt = readAt + ttlMs
        if (providers.isEmpty()) {
            ProviderDirectoryResult.Empty(
                observation = DirectoryObservation.Live,
                readAt = readAt,
                expiresAt = expiresAt,
                total = total,
                limit = limit,
                offset = offset,
                hasMore = hasMore,
            )
        } else {
            ProviderDirectoryResult.Found(
                providers = providers,
                distancesKm = distances,
                observation = DirectoryObservation.Live,
                readAt = readAt,
                expiresAt = expiresAt,
                total = total,
                limit = limit,
                offset = offset,
                hasMore = hasMore,
            )
        }
    } catch (_: MalformedDirectory) {
        malformed(readAt)
    } catch (_: Exception) {
        malformed(readAt)
    }

    private fun malformed(attemptedAt: Long) = ProviderDirectoryResult.Unavailable(
        reason = DirectoryUnavailableReason.MalformedResponse,
        detail = "The provider directory returned an invalid response; it was not treated as empty",
        attemptedAt = attemptedAt,
    )

    private fun decodeProvider(row: JSONObject): DirectoryProvider {
        val providerId = row.requiredString("businessId")
        val kind = row.requiredString("type")
        val name = row.requiredString("name")
        val servicesJson = row.opt("services") as? JSONArray
            ?: throw MalformedDirectory("provider services is not an array")
        val services = (0 until servicesJson.length()).map { index ->
            (servicesJson.opt(index) as? String)
                ?: throw MalformedDirectory("provider service is not text")
        }

        val geo = when {
            !row.has("geo") || row.isNull("geo") -> null
            else -> {
                val objectValue = row.opt("geo") as? JSONObject
                    ?: throw MalformedDirectory("provider geo is not an object")
                val lat = (objectValue.opt("lat") as? Number)?.toDouble()
                    ?: throw MalformedDirectory("provider latitude is not numeric")
                val lng = (objectValue.opt("lng") as? Number)?.toDouble()
                    ?: throw MalformedDirectory("provider longitude is not numeric")
                GeoPoint(lat, lng).takeIf { it.isUsable }
                    ?: throw MalformedDirectory("provider coordinate is out of range")
            }
        }

        val contact = when {
            !row.has("contact") || row.isNull("contact") -> ProviderContact()
            else -> {
                val objectValue = row.opt("contact") as? JSONObject
                    ?: throw MalformedDirectory("provider contact is not an object")
                ProviderContact(
                    phone = objectValue.optionalText("phone"),
                    whatsapp = objectValue.optionalText("whatsapp"),
                    telegram = objectValue.optionalText("telegram"),
                    email = objectValue.optionalText("email"),
                    website = objectValue.optionalText("website"),
                )
            }
        }

        // The central wire requires a string, with blank carrying the ordinary domain-less case.
        // Missing/null is malformed and must not be upgraded into the on-chain fact "no claim".
        val domain = row.requiredString("domain").trim().ifBlank { null }

        return DirectoryProvider(
            providerId = providerId,
            kind = kind,
            name = name,
            geo = geo,
            services = services,
            domain = domain,
            active = null,
            contact = contact,
            bindingState = if (domain == null) {
                IssuerBindingState.NoDomainListed
            } else {
                IssuerBindingState.Unavailable
            },
        )
    }

    private fun JSONObject.requiredString(key: String): String {
        if (!has(key) || isNull(key)) throw MalformedDirectory("provider $key is missing")
        return opt(key) as? String
            ?: throw MalformedDirectory("provider $key is not text")
    }

    private fun JSONObject.optionalText(key: String): String? {
        if (!has(key) || isNull(key)) return null
        val value = opt(key) as? String
            ?: throw MalformedDirectory("provider $key is not text")
        return value.trim().ifBlank { null }
    }

    private fun JSONObject.optionalDistance(): Double? {
        if (!has("distanceKm") || isNull("distanceKm")) return null
        val distance = (opt("distanceKm") as? Number)?.toDouble()
            ?: throw MalformedDirectory("provider distanceKm is not numeric")
        if (!distance.isFinite() || distance < 0) {
            throw MalformedDirectory("provider distanceKm is unusable")
        }
        return distance
    }

    private fun JSONObject.requiredNonnegativeInt(key: String): Int {
        val number = opt(key) as? Number ?: throw MalformedDirectory("$key is not numeric")
        val value = number.toDouble()
        if (!value.isFinite() || value < 0 || value % 1.0 != 0.0 || value > Int.MAX_VALUE) {
            throw MalformedDirectory("$key is not a nonnegative integer")
        }
        return value.toInt()
    }

    private fun JSONObject.requiredBoolean(key: String): Boolean =
        opt(key) as? Boolean ?: throw MalformedDirectory("$key is not boolean")

    private class MalformedDirectory(message: String) : IllegalArgumentException(message)
}

private fun percentEncode(value: String): String =
    URLEncoder.encode(value, StandardCharsets.UTF_8.toString()).replace("+", "%20")

/** One process-wide stateless adapter shared across screen visits. */
object ProviderDirectories {
    val central: ProviderDirectory by lazy {
        CentralProviderDirectory(AppConfig.CENTRAL_API)
    }

    /**
     * Scopes remembered records to this exact configured endpoint, so one deployment's stored copy can
     * never be replayed as another's. Built from the base URL alone: a namespace keyed by a nearest
     * query would put the owner's position into the cache key, which the ruling forbids.
     *
     * The cache is deliberately NOT a decorator around [central] (see [ProviderRecordCache]) - the
     * caller holds both and drives them explicitly.
     */
    fun recordCache(cacheDir: File): ProviderRecordCache = ProviderRecordCache(
        store = FileProviderDirectoryCacheStore(
            File(cacheDir, FileProviderDirectoryCacheStore.FILE_NAME),
        ),
        namespace = "central:${AppConfig.CENTRAL_API}",
    )
}
