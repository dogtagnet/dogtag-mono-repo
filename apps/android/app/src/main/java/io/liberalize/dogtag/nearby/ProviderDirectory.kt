package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.data.AppConfig
import io.liberalize.dogtag.net.Http
import io.liberalize.dogtag.net.IssuerBindingState
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI

/** Native adapter for the canonical full-set provider-directory contract. */
interface ProviderDirectory {
    /**
     * Reads the same provider set for every caller. There is deliberately no query argument and no
     * request-shaped place for a current/chosen position or name search.
     */
    suspend fun read(): ProviderDirectoryResult
}

/**
 * Central `GET /v1/businesses` adapter with a hard-lived in-process snapshot.
 *
 * The cache is the same semantic contract as `packages/ui`: a failed live read may replay the last
 * unexpired successful snapshot as [DirectoryObservation.Stored], without renewing its deadline.
 */
class CentralProviderDirectory(
    baseUrl: String,
    private val ttlMs: Long = 15 * 60 * 1_000L,
    private val now: () -> Long = System::currentTimeMillis,
    private val fetch: suspend (String) -> Http.Response = { Http.getJson(it) },
) : ProviderDirectory {
    private val configuredBase = baseUrl.trim().also { configured ->
        require(configured.isNotEmpty()) { "provider directory base URL must not be blank" }
        val parsed = URI(configured)
        require(parsed.query == null && parsed.fragment == null) {
            "provider directory base URL must not contain a query or fragment"
        }
    }
    internal val requestUrl: String = "${configuredBase.trimEnd('/')}/v1/businesses"

    private data class CacheEntry(
        val result: ProviderDirectoryResult,
        val readAt: Long,
        val expiresAt: Long,
    )

    private var cache: CacheEntry? = null

    init {
        require(ttlMs > 0) { "provider directory cache ttlMs must be greater than zero" }
    }

    override suspend fun read(): ProviderDirectoryResult {
        val live = try {
            val response = fetch(requestUrl)
            if (!response.ok) {
                ProviderDirectoryResult.Unavailable(
                    reason = DirectoryUnavailableReason.SourceUnavailable,
                    detail = "The provider directory returned HTTP ${response.code}",
                    attemptedAt = now(),
                )
            } else {
                decode(response.body, now())
            }
        } catch (error: Exception) {
            ProviderDirectoryResult.Unavailable(
                reason = DirectoryUnavailableReason.SourceUnavailable,
                detail = error.message?.trim()?.takeIf { it.isNotEmpty() }
                    ?: "The provider directory could not be reached",
                attemptedAt = now(),
            )
        }

        when (live) {
            is ProviderDirectoryResult.Found -> {
                cache = CacheEntry(live, live.readAt, live.expiresAt)
                return live
            }
            is ProviderDirectoryResult.Empty -> {
                cache = CacheEntry(live, live.readAt, live.expiresAt)
                return live
            }
            is ProviderDirectoryResult.Unavailable -> Unit
        }

        val current = now()
        val saved = cache
        if (
            saved == null ||
            saved.readAt > current ||
            saved.expiresAt < saved.readAt ||
            saved.expiresAt - saved.readAt > ttlMs ||
            current >= saved.expiresAt
        ) {
            cache = null
            return live
        }
        return when (val snapshot = saved.result) {
            is ProviderDirectoryResult.Found -> snapshot.copy(observation = DirectoryObservation.Stored)
            is ProviderDirectoryResult.Empty -> snapshot.copy(observation = DirectoryObservation.Stored)
            is ProviderDirectoryResult.Unavailable -> {
                cache = null
                live
            }
        }
    }

    /**
     * All-or-nothing decode. One malformed provider makes the read unavailable; it never degrades a
     * bad response into the materially different claim that the source was successfully empty.
     */
    internal fun decode(json: String, readAt: Long): ProviderDirectoryResult = try {
        val root = JSONObject(json)
        val rows = root.opt("businesses") as? JSONArray
            ?: throw MalformedDirectory("response has no businesses array")
        val providers = ArrayList<DirectoryProvider>(rows.length())
        val ids = HashSet<String>()
        for (index in 0 until rows.length()) {
            val row = rows.opt(index) as? JSONObject
                ?: throw MalformedDirectory("businesses[$index] is not an object")
            val provider = decodeProvider(row)
            if (!ids.add(provider.providerId)) {
                throw MalformedDirectory("businesses contains a duplicate provider id")
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
            )
        } else {
            ProviderDirectoryResult.Found(
                providers = providers,
                observation = DirectoryObservation.Live,
                readAt = readAt,
                expiresAt = expiresAt,
            )
        }
    } catch (error: MalformedDirectory) {
        ProviderDirectoryResult.Unavailable(
            reason = DirectoryUnavailableReason.MalformedResponse,
            detail = "The provider directory returned an invalid response; it was not treated as empty",
            attemptedAt = readAt,
        )
    } catch (_: Exception) {
        ProviderDirectoryResult.Unavailable(
            reason = DirectoryUnavailableReason.MalformedResponse,
            detail = "The provider directory returned an invalid response; it was not treated as empty",
            attemptedAt = readAt,
        )
    }

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
            // The central response carries no listing-state assertion. Even if a future response
            // contains an extra field named "active", this adapter must not silently promote it into
            // the source-neutral maintained/delisted fact.
            active = null,
            contact = contact,
            // Central supplies no clone/root/observed binding and reads no chain state at all. These
            // are the only honest states it may derive locally; it can never synthesize Verified from
            // the domain string, and a blank column is a fact about THIS LISTING, not an on-chain one.
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

    private class MalformedDirectory(message: String) : IllegalArgumentException(message)
}

/** One process-wide cache shared across screen visits. */
object ProviderDirectories {
    val central: ProviderDirectory by lazy {
        CentralProviderDirectory(AppConfig.CENTRAL_API)
    }
}
