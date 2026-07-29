package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.net.IssuerBindingState
import java.text.Normalizer
import java.util.Locale

/**
 * A coordinate already held by the phone.
 *
 * Absence is represented by `null`. In particular, `(0, 0)` is a real coordinate and must never be
 * repurposed as the sentinel for a provider that did not publish a location.
 */
data class GeoPoint(val lat: Double, val lng: Double) {
    val isUsable: Boolean
        get() = lat.isFinite() && lng.isFinite() && lat in -90.0..90.0 && lng in -180.0..180.0
}

/** Contact channels a provider may publish without publishing a location. */
data class ProviderContact(
    val phone: String? = null,
    val whatsapp: String? = null,
    val telegram: String? = null,
    val email: String? = null,
) {
    val hasAny: Boolean
        get() = listOf(phone, whatsapp, telegram, email).any { !it.isNullOrBlank() }
}

/**
 * The source-neutral row consumed by the two directory scopes.
 *
 * [bindingState] reuses the app's issuer-domain state machine. The central adapter may only produce
 * `NoDomainClaimed` for a blank domain or `Unavailable` otherwise; it has no chain-backed basis for a
 * positive claim.
 */
data class DirectoryProvider(
    val providerId: String,
    val kind: String,
    val name: String,
    val geo: GeoPoint?,
    val services: List<String>,
    val domain: String?,
    val active: Boolean?,
    val contact: ProviderContact,
    val bindingState: IssuerBindingState,
)

enum class DirectoryObservation { Live, Stored }

enum class DirectoryUnavailableReason {
    SourceUnavailable,
    MalformedResponse,
    InvalidSnapshot,
}

sealed interface ProviderDirectoryResult {
    data class Found(
        val providers: List<DirectoryProvider>,
        val observation: DirectoryObservation,
        val readAt: Long,
        val expiresAt: Long,
    ) : ProviderDirectoryResult

    data class Empty(
        val observation: DirectoryObservation,
        val readAt: Long,
        val expiresAt: Long,
    ) : ProviderDirectoryResult

    data class Unavailable(
        val reason: DirectoryUnavailableReason,
        val detail: String,
        val attemptedAt: Long,
    ) : ProviderDirectoryResult
}

enum class OriginSource { CurrentLocation, ChosenCoordinates }

sealed interface NearbyOriginState {
    data object AwaitingChoice : NearbyOriginState
    data object Locating : NearbyOriginState
    data object PermissionRefused : NearbyOriginState
    data object LocationUnavailable : NearbyOriginState
    data object InvalidChosenLocation : NearbyOriginState
    data class Available(val point: GeoPoint, val source: OriginSource) : NearbyOriginState
}

/** Distance/bearing supplied by the platform geo primitive, not recomputed by this rule layer. */
data class ProviderMeasurement(
    val providerId: String,
    val distanceKm: Double,
    val bearingDegrees: Double?,
)

data class NearbyRow(
    val provider: DirectoryProvider,
    val distanceKm: Double,
    val bearingDegrees: Double?,
) {
    /** A row reaches this type only after its real destination was validated. */
    val canOpenDirections: Boolean get() = provider.geo?.isUsable == true
}

sealed interface NearbyPresentation {
    data object LoadingDirectory : NearbyPresentation
    data class DirectoryUnavailable(val detail: String) : NearbyPresentation
    data class DirectoryEmpty(val observation: DirectoryObservation) : NearbyPresentation
    data object AwaitingOrigin : NearbyPresentation
    data object Locating : NearbyPresentation
    data object PermissionRefused : NearbyPresentation
    data object LocationUnavailable : NearbyPresentation
    data object InvalidChosenLocation : NearbyPresentation
    data class NoneWithinRange(
        val radiusKm: Double,
        val observation: DirectoryObservation,
    ) : NearbyPresentation

    data class NoNameMatch(
        val query: String,
        val observation: DirectoryObservation,
    ) : NearbyPresentation

    data class ProvidersFound(
        val rows: List<NearbyRow>,
        val observation: DirectoryObservation,
    ) : NearbyPresentation
}

sealed interface ContactDirectoryPresentation {
    data object LoadingDirectory : ContactDirectoryPresentation
    data class DirectoryUnavailable(val detail: String) : ContactDirectoryPresentation
    data class DirectoryEmpty(val observation: DirectoryObservation) : ContactDirectoryPresentation
    data class NoNameMatch(
        val query: String,
        val observation: DirectoryObservation,
    ) : ContactDirectoryPresentation

    data class ProvidersFound(
        val providers: List<DirectoryProvider>,
        val observation: DirectoryObservation,
    ) : ContactDirectoryPresentation
}

/**
 * What the Nearby and Provider contacts scopes are allowed to claim.
 *
 * Pure Kotlin: no Android, networking, clock, JSON, or location APIs. Android supplies measurements
 * with `Location.distanceBetween`; iOS supplies its platform measurements to the mirrored rule.
 */
object NearbyDecision {
    const val DEFAULT_RADIUS_KM = 50.0
    private const val KM_PER_MILE = 1.609344
    private const val FEET_PER_KM = 1000 / 0.3048
    private val imperialRegions = setOf("US", "GB", "LR", "MM")
    private val compassPoints = listOf("N", "NE", "E", "SE", "S", "SW", "W", "NW")

    enum class UnitSystem { Metric, Imperial }

    fun parseChosenOrigin(latitude: String, longitude: String): GeoPoint? {
        val lat = latitude.trim().toDoubleOrNull() ?: return null
        val lng = longitude.trim().toDoubleOrNull() ?: return null
        return GeoPoint(lat, lng).takeIf { it.isUsable }
    }

    fun nearby(
        directory: ProviderDirectoryResult?,
        origin: NearbyOriginState,
        measurements: List<ProviderMeasurement>,
        query: String,
        radiusKm: Double = DEFAULT_RADIUS_KM,
    ): NearbyPresentation {
        when (directory) {
            null -> return NearbyPresentation.LoadingDirectory
            is ProviderDirectoryResult.Unavailable ->
                return NearbyPresentation.DirectoryUnavailable(directory.detail)
            is ProviderDirectoryResult.Empty ->
                return NearbyPresentation.DirectoryEmpty(directory.observation)
            is ProviderDirectoryResult.Found -> Unit
        }

        val found = directory
        val needle = query.trim()
        val locatedCandidates = found.providers.filter { provider ->
            eligible(provider) && provider.geo?.isUsable == true
        }.let { located ->
            if (needle.isEmpty()) {
                located
            } else {
                val foldedNeedle = searchFold(needle)
                located.filter { searchFold(it.name).contains(foldedNeedle) }
            }
        }

        // Do not ask for an origin when the phone-local provider set proves that no row could be
        // shown. This keeps a refused/unavailable origin from obscuring an honest empty result and,
        // importantly, avoids inviting an unnecessary location permission prompt.
        if (locatedCandidates.isEmpty()) {
            return if (needle.isNotEmpty()) {
                NearbyPresentation.NoNameMatch(needle, found.observation)
            } else {
                val usableRadius = radiusKm.takeIf { it.isFinite() && it >= 0 } ?: DEFAULT_RADIUS_KM
                NearbyPresentation.NoneWithinRange(usableRadius, found.observation)
            }
        }

        when (origin) {
            NearbyOriginState.AwaitingChoice -> return NearbyPresentation.AwaitingOrigin
            NearbyOriginState.Locating -> return NearbyPresentation.Locating
            NearbyOriginState.PermissionRefused -> return NearbyPresentation.PermissionRefused
            NearbyOriginState.LocationUnavailable -> return NearbyPresentation.LocationUnavailable
            NearbyOriginState.InvalidChosenLocation ->
                return NearbyPresentation.InvalidChosenLocation
            is NearbyOriginState.Available -> {
                if (!origin.point.isUsable) return NearbyPresentation.LocationUnavailable
            }
        }

        val measuredById = measurements.associateBy { it.providerId }
        val located = locatedCandidates.mapNotNull { provider ->
            val measured = measuredById[provider.providerId] ?: return@mapNotNull null
            if (!measured.distanceKm.isFinite() || measured.distanceKm < 0) return@mapNotNull null
            NearbyRow(
                provider = provider,
                distanceKm = measured.distanceKm,
                bearingDegrees = measured.bearingDegrees
                    ?.takeIf { it.isFinite() }
                    ?.let { ((it % 360.0) + 360.0) % 360.0 },
            )
        }

        val visible = if (needle.isNotEmpty()) {
            // Name candidates were selected before asking for an origin. Search is deliberately not
            // a locality search, so every valid measurement is visible even beyond the default range.
            located
        } else {
            val usableRadius = radiusKm.takeIf { it.isFinite() && it >= 0 } ?: DEFAULT_RADIUS_KM
            located.filter { it.distanceKm <= usableRadius }
        // Kotlin's object-array sort is stable: equal-distance providers keep source-directory order.
        // Adding a name tiebreak would silently change the source's ordering policy.
        }.sortedBy { it.distanceKm }

        if (visible.isEmpty()) {
            return if (needle.isNotEmpty()) {
                NearbyPresentation.NoNameMatch(needle, found.observation)
            } else {
                val usableRadius = radiusKm.takeIf { it.isFinite() && it >= 0 } ?: DEFAULT_RADIUS_KM
                NearbyPresentation.NoneWithinRange(usableRadius, found.observation)
            }
        }
        return NearbyPresentation.ProvidersFound(visible, found.observation)
    }

    /**
     * The unranked directory. Every eligible vet/groomer may appear here whether or not it published
     * a location; this is the deliberate home for contact-only providers.
     */
    fun contacts(
        directory: ProviderDirectoryResult?,
        query: String,
    ): ContactDirectoryPresentation {
        when (directory) {
            null -> return ContactDirectoryPresentation.LoadingDirectory
            is ProviderDirectoryResult.Unavailable ->
                return ContactDirectoryPresentation.DirectoryUnavailable(directory.detail)
            is ProviderDirectoryResult.Empty ->
                return ContactDirectoryPresentation.DirectoryEmpty(directory.observation)
            is ProviderDirectoryResult.Found -> Unit
        }
        val found = directory
        val needle = query.trim()
        val providers = found.providers
            .filter(::eligible)
            .filter {
                needle.isEmpty() || searchFold(it.name).contains(searchFold(needle))
            }
            .sortedWith(
                compareBy<DirectoryProvider, String>(String.CASE_INSENSITIVE_ORDER) { it.name }
                    .thenBy { it.providerId },
            )

        if (providers.isEmpty()) {
            return if (needle.isNotEmpty()) {
                ContactDirectoryPresentation.NoNameMatch(needle, found.observation)
            } else {
                ContactDirectoryPresentation.DirectoryEmpty(found.observation)
            }
        }
        return ContactDirectoryPresentation.ProvidersFound(providers, found.observation)
    }

    private fun eligible(provider: DirectoryProvider): Boolean {
        if (provider.active == false) return false
        return provider.kind.trim().lowercase(Locale.ROOT) in setOf("vet", "groomer")
    }

    /** Case- and diacritic-insensitive matching, over the already-held provider names only. */
    private fun searchFold(value: String): String =
        Normalizer.normalize(value, Normalizer.Form.NFD)
            .replace(Regex("\\p{M}+"), "")
            .lowercase(Locale.ROOT)

    fun unitSystemForRegion(regionOrLocale: String?): UnitSystem {
        if (regionOrLocale.isNullOrBlank()) return UnitSystem.Metric
        val parts = regionOrLocale.split(Regex("[-_]"))
        for (part in parts.drop(1)) {
            if (
                part.length == 2 &&
                part.all { it.isLetter() } &&
                part.uppercase(Locale.ROOT) in imperialRegions
            ) {
                return UnitSystem.Imperial
            }
        }
        if (
            parts.size == 1 &&
            parts[0].uppercase(Locale.ROOT) in imperialRegions
        ) return UnitSystem.Imperial
        return UnitSystem.Metric
    }

    /**
     * Honest display precision matching the canonical geo core: roughly ten metres near the user,
     * coarser with distance, and no plausible string for an unusable measurement.
     */
    fun formatDistanceKm(km: Double?, unit: UnitSystem = UnitSystem.Metric): String? {
        if (km == null || !km.isFinite() || km < 0) return null
        if (unit == UnitSystem.Imperial) {
            val miles = km / KM_PER_MILE
            if (miles < 0.1) {
                val feet = km * FEET_PER_KM
                if (feet < 25) return "< 25 ft"
                return "${roundTo(feet, 25.0).toLong()} ft"
            }
            val oneDecimal = Math.round(miles * 10) / 10.0
            if (oneDecimal < 10) return String.format(Locale.US, "%.1f mi", oneDecimal)
            return "${Math.round(miles)} mi"
        }

        if (km < 1) {
            val metres = km * 1000
            if (metres < 10) return "< 10 m"
            val rounded = roundTo(metres, 10.0)
            if (rounded < 1000) return "${rounded.toLong()} m"
        }
        val oneDecimal = Math.round(km * 10) / 10.0
        if (oneDecimal < 10) return String.format(Locale.US, "%.1f km", oneDecimal)
        return "${Math.round(km)} km"
    }

    /** Eight-point compass label for the platform-provided initial bearing. */
    fun formatBearing(bearingDegrees: Double?): String? {
        if (bearingDegrees == null || !bearingDegrees.isFinite()) return null
        val normalized = ((bearingDegrees % 360.0) + 360.0) % 360.0
        val index = Math.round(normalized / 45.0).toInt() % compassPoints.size
        return compassPoints[index]
    }

    private fun roundTo(value: Double, step: Double): Double = Math.round(value / step) * step
}
