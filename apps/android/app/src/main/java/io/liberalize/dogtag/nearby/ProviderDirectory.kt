package io.liberalize.dogtag.nearby

import android.content.Context
import io.liberalize.dogtag.data.AppConfig
import io.liberalize.dogtag.net.Http
import io.liberalize.dogtag.net.IssuerBindingState
import kotlinx.coroutines.CancellationException
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.URI

/** Native adapter for the canonical full-set provider-directory contract. */
interface ProviderDirectory {
    /**
     * Stable identity of the configured source, used ONLY to scope a stored copy.
     *
     * It must distinguish two distinct configured endpoints and two future chain/registry
     * configurations. It must never contain a user position or anything derived from one.
     */
    val cacheNamespace: String

    /**
     * Reads the same provider set for every caller. There is deliberately no query argument and no
     * request-shaped place for a current/chosen position or name search.
     */
    suspend fun read(): ProviderDirectoryResult
}

/**
 * Central `GET /v1/businesses` adapter. It is a SOURCE only: it holds no cache and keeps no snapshot.
 *
 * The local copy lives in [CachedProviderDirectory], one decorator wrapping this seam, so the future
 * on-chain directory inherits the same offline behaviour without reimplementing it. Two fused caches
 * would also have to reason about each other - the inner one hands the outer a snapshot already
 * labelled [DirectoryObservation.Stored], and treating that as a successful refresh renews a deadline
 * that is supposed to be hard.
 */
class CentralProviderDirectory(
    baseUrl: String,
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

    /**
     * Scopes a stored copy to this exact configured endpoint.
     *
     * Repointing `CENTRAL_API` changes it, so one deployment's persisted snapshot can never be
     * replayed as another's. A future on-chain directory must carry its own chain/registry identity
     * here for the same reason.
     */
    override val cacheNamespace: String get() = "central:$requestUrl"

    override suspend fun read(): ProviderDirectoryResult = try {
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
    } catch (cancelled: CancellationException) {
        // A cancelled read is the caller leaving the screen, not the directory failing. Mapping it
        // to `unavailable` would fabricate a source failure and make the wrapper replay a stored
        // copy for a request nobody is waiting on.
        throw cancelled
    } catch (error: Exception) {
        ProviderDirectoryResult.Unavailable(
            reason = DirectoryUnavailableReason.SourceUnavailable,
            detail = error.message?.trim()?.takeIf { it.isNotEmpty() }
                ?: "The provider directory could not be reached",
            attemptedAt = now(),
        )
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

        if (readAt < 0) throw MalformedDirectory("directory timestamp is unusable")
        // `expiresAt` is null because this source has no TTL of its own. The wrapper sets the hard
        // deadline from THIS observation time, so a slow read can never lengthen the maximum age.
        if (providers.isEmpty()) {
            ProviderDirectoryResult.Empty(
                observation = DirectoryObservation.Live,
                readAt = readAt,
                expiresAt = null,
            )
        } else {
            ProviderDirectoryResult.Found(
                providers = providers,
                observation = DirectoryObservation.Live,
                readAt = readAt,
                expiresAt = null,
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

/**
 * The configured directory, wrapped in the on-device local copy.
 *
 * Built per call rather than held in a `by lazy`: the stored copy now lives on disk, so there is no
 * process-lifetime state left to share, and a device-scoped singleton would only pin a `Context`.
 */
object ProviderDirectories {
    fun central(context: Context): ProviderDirectory = CachedProviderDirectory(
        delegate = CentralProviderDirectory(AppConfig.CENTRAL_API),
        store = FileProviderDirectoryCacheStore(
            // Cache dir, not files dir: this is a re-fetchable copy of a PUBLIC endpoint, so the OS
            // is welcome to evict it. Eviction then reads as a missing entry - `unavailable` - which
            // is the honest degradation, never a successful `empty`.
            File(context.cacheDir, FileProviderDirectoryCacheStore.FILE_NAME),
        ),
    )
}
