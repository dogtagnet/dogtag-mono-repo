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

/**
 * Contact channels a provider may publish without publishing a location.
 *
 * Membership and order mirror `PROVIDER_CONTACT_CHANNELS` in
 * `packages/ui/src/directory/channels.ts`, which Kotlin cannot import: keep them aligned by hand.
 * A channel the server serves and this type omits is worse than one never added - the parser drops
 * it, [hasAny] then reads false, and a provider that published a website is told to the owner as
 * having published nothing.
 */
data class ProviderContact(
    val phone: String? = null,
    val whatsapp: String? = null,
    val telegram: String? = null,
    val email: String? = null,
    val website: String? = null,
) {
    val hasAny: Boolean
        get() = listOf(phone, whatsapp, telegram, email, website).any { !it.isNullOrBlank() }
}

/**
 * The source-neutral row consumed by the two directory scopes.
 *
 * [bindingState] reuses the app's issuer-domain state machine. The central adapter may only produce
 * `NoDomainListed` for a blank domain or `Unavailable` otherwise; it reads no chain state, so it has
 * no basis for a positive claim and none for the on-chain `NoDomainClaimed` either.
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
        /**
         * `null` means this result came from a directory with no cache wrapper around it. It does
         * NOT mean the facts are permanent, and it is what lets [CachedProviderDirectory] tell a
         * fresh source read apart from a replay handed up by an inner wrapper.
         */
        val expiresAt: Long?,
    ) : ProviderDirectoryResult

    data class Empty(
        val observation: DirectoryObservation,
        val readAt: Long,
        val expiresAt: Long?,
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

    /**
     * [accuracyMetres] is the horizontal uncertainty the DEVICE reported for a
     * [OriginSource.CurrentLocation] fix, and is what stops a coarse fix from being rendered as a
     * precise distance. It is meaningless for [OriginSource.ChosenCoordinates], which the owner typed
     * exactly, so that source keeps ordinary coordinate-based precision.
     */
    data class Available(
        val point: GeoPoint,
        val source: OriginSource,
        val accuracyMetres: Double? = null,
    ) : NearbyOriginState
}

/**
 * What may be SAID about one measured distance, given how precise the origin actually is.
 *
 * A device fix carries real uncertainty, so the rendered number must not be finer than the fix
 * supports, and when nothing numeric is supportable the surface shows [Uncertain] rather than an
 * arbitrary confident figure.
 */
sealed interface DistanceClaim {
    /** [approximate] is true for every device fix, so the row can mark the number as such. */
    data class Measured(val label: String, val approximate: Boolean) : DistanceClaim {
        /**
         * What the row actually prints. Composed here, not at each call site, so the two apps cannot
         * disagree — and so a label that is already a bound is not double-marked as `~< 150 m`.
         */
        val display: String
            get() = if (approximate && !label.startsWith("<")) "~$label" else label
    }

    /** No distance may be stated at all; [reason] is shown in place of a number. */
    data class Uncertain(val reason: String) : DistanceClaim
}

/** Distance/bearing supplied by the platform geo primitive, not recomputed by this rule layer. */
data class ProviderMeasurement(
    val providerId: String,
    val distanceKm: Double,
    val bearingDegrees: Double?,
)

data class NearbyRow(
    val provider: DirectoryProvider,
    /** The RAW platform measurement. Ordering uses it; only [distance] is coarsened for display. */
    val distanceKm: Double,
    val bearingDegrees: Double?,
    val distance: DistanceClaim,
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
    private const val DISTANCE_UNAVAILABLE = "This provider's distance could not be measured."
    private const val CEIL_TOLERANCE = 1e-9

    // Each rung below is the metre width of one display band, derived from the same constant that
    // band's own formatting divides by. A value rounded to a rung therefore always lands on a
    // numeral its band can express, which is what keeps a positive distance from printing as zero.
    private const val FOOT_STEP_M = 25_000.0 / FEET_PER_KM
    private const val TENTH_MILE_M = KM_PER_MILE * 100
    private const val MILE_M = KM_PER_MILE * 1_000
    private const val MINUTE_MS = 60L * 1_000
    private const val HOUR_MS = 60L * MINUTE_MS
    private const val DAY_MS = 24L * HOUR_MS
    private const val TEN_METRE_STEP_M = 10.0
    private const val TENTH_KM_M = 100.0
    private const val KM_STEP_M = 1_000.0

    /** The coarsest fix that can still place a provider at all. Beyond it, no number is claimed. */
    private const val COARSEST_METRIC_M = 10 * KM_STEP_M
    private const val COARSEST_IMPERIAL_M = 10 * MILE_M

    /** Rounding steps a device fix may be shown at. The chosen step is never finer than the fix. */
    private val METRIC_LADDER_M = listOf(TEN_METRE_STEP_M, TENTH_KM_M, KM_STEP_M, COARSEST_METRIC_M)
    private val IMPERIAL_LADDER_M =
        listOf(FOOT_STEP_M, TENTH_MILE_M, MILE_M, COARSEST_IMPERIAL_M)
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
        unit: UnitSystem = UnitSystem.Metric,
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

        val available = when (origin) {
            NearbyOriginState.AwaitingChoice -> return NearbyPresentation.AwaitingOrigin
            NearbyOriginState.Locating -> return NearbyPresentation.Locating
            NearbyOriginState.PermissionRefused -> return NearbyPresentation.PermissionRefused
            NearbyOriginState.LocationUnavailable -> return NearbyPresentation.LocationUnavailable
            NearbyOriginState.InvalidChosenLocation ->
                return NearbyPresentation.InvalidChosenLocation
            is NearbyOriginState.Available -> {
                if (!origin.point.isUsable) return NearbyPresentation.LocationUnavailable
                origin
            }
        }

        val fromDeviceFix = available.source == OriginSource.CurrentLocation
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
                distance = distanceClaim(
                    km = measured.distanceKm,
                    accuracyMetres = available.accuracyMetres,
                    fromDeviceFix = fromDeviceFix,
                    unit = unit,
                ),
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
            .sortedWith(compareBy<DirectoryProvider>({ sortKey(it.name) }, { it.providerId }))

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

    /**
     * The contact list's ordering key, mirrored by iOS.
     *
     * It is the SAME locale-independent fold the on-device name search uses. A collating comparator
     * would order the two apps differently for the same directory, because each platform's collation
     * is its own; folding first leaves one order that both can reproduce.
     */
    private fun sortKey(value: String): String = searchFold(value.trim())

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
     *
     * [minGranularityMetres] raises that floor so a band finer than the origin's own uncertainty is
     * never reached. It can only make a label COARSER; coarser is always honest, finer never is.
     * The default 0.0 is the exact coordinate-arithmetic path and is byte-identical to before.
     */
    fun formatDistanceKm(
        km: Double?,
        unit: UnitSystem = UnitSystem.Metric,
        minGranularityMetres: Double = 0.0,
    ): String? {
        if (km == null || !km.isFinite() || km < 0) return null
        if (unit == UnitSystem.Imperial) {
            val miles = km / KM_PER_MILE
            if (minGranularityMetres <= FOOT_STEP_M && miles < 0.1) {
                val feet = km * FEET_PER_KM
                if (feet < 25) return "< 25 ft"
                return "${roundTo(feet, 25.0).toLong()} ft"
            }
            val oneDecimal = Math.round(miles * 10) / 10.0
            if (minGranularityMetres <= TENTH_MILE_M && oneDecimal < 10) {
                return String.format(Locale.US, "%.1f mi", oneDecimal)
            }
            if (minGranularityMetres <= COARSEST_IMPERIAL_M) return "${Math.round(miles)} mi"
            return null
        }

        if (minGranularityMetres <= TEN_METRE_STEP_M && km < 1) {
            val metres = km * 1000
            if (metres < 10) return "< 10 m"
            val rounded = roundTo(metres, TEN_METRE_STEP_M)
            if (rounded < 1000) return "${rounded.toLong()} m"
        }
        val oneDecimal = Math.round(km * 10) / 10.0
        if (minGranularityMetres <= TENTH_KM_M && oneDecimal < 10) {
            return String.format(Locale.US, "%.1f km", oneDecimal)
        }
        if (minGranularityMetres <= COARSEST_METRIC_M) return "${Math.round(km)} km"
        return null
    }

    /**
     * How old a stored directory replay is, coarsely.
     *
     * The offline window is measured in days, so "stored" and "recent" are no longer the same
     * statement and the surface has to say which. The ladder is deliberately blunt - under a minute,
     * then minutes, hours, days - because a remembered public directory supports no finer claim.
     *
     * Rounds the age OUTWARD, so the stated age is never smaller than the true one and a remembered
     * copy is never described as fresher than it is. That is the same direction [uncertaintyLabel]
     * rounds a distance bound, and the safe one here: understating staleness under-warns.
     *
     * Derived from the snapshot's own `readAt`, never from `expiresAt` minus the TTL - the deadline
     * is the MINIMUM of the local window and any the source declared, so that subtraction is wrong
     * whenever the source declared a shorter one. A `readAt` in the future is not derivable and
     * answers null, which the surface renders as saying nothing rather than as "0 minutes ago".
     */
    fun formatStoredAge(readAtMillis: Long, nowMillis: Long): String? {
        val elapsedMs = nowMillis - readAtMillis
        if (elapsedMs < 0) return null
        if (elapsedMs < MINUTE_MS) return "less than a minute ago"
        val minutes = ceilDiv(elapsedMs, MINUTE_MS)
        if (minutes < 60) return agePhrase(minutes, "minute")
        val hours = ceilDiv(elapsedMs, HOUR_MS)
        if (hours < 24) return agePhrase(hours, "hour")
        return agePhrase(ceilDiv(elapsedMs, DAY_MS), "day")
    }

    private fun ceilDiv(value: Long, unit: Long): Long = (value + unit - 1) / unit

    private fun agePhrase(count: Long, unit: String): String =
        if (count == 1L) "1 $unit ago" else "$count ${unit}s ago"

    /**
     * What this origin's precision permits the row to say about one measured distance.
     *
     * Chosen coordinates are exact arithmetic on numbers the owner typed, so they keep the ordinary
     * bands. A device fix does not: its label is rounded to a step no finer than the reported
     * horizontal accuracy, and a fix whose accuracy is missing, nonsensical, or coarser than any
     * usable step yields no number at all.
     *
     * Anything nearer than the coarser of the accuracy and half that rounding step is stated as a
     * BOUND. Half the step is the load-bearing half of that pair: below it the rounding collapses to
     * zero, which the bands would then print as a confident `0 km`. The gate and the bound label are
     * computed from the same value so they can never describe different distances.
     */
    fun distanceClaim(
        km: Double?,
        accuracyMetres: Double?,
        fromDeviceFix: Boolean,
        unit: UnitSystem = UnitSystem.Metric,
    ): DistanceClaim {
        if (km == null || !km.isFinite() || km < 0) {
            return DistanceClaim.Uncertain(DISTANCE_UNAVAILABLE)
        }
        if (!fromDeviceFix) {
            val label = formatDistanceKm(km, unit)
                ?: return DistanceClaim.Uncertain(DISTANCE_UNAVAILABLE)
            return DistanceClaim.Measured(label, approximate = false)
        }
        if (accuracyMetres == null || !accuracyMetres.isFinite() || accuracyMetres < 0) {
            return DistanceClaim.Uncertain(
                "This location fix reported no usable accuracy, so no distance is claimed.",
            )
        }
        val ladder = if (unit == UnitSystem.Imperial) IMPERIAL_LADDER_M else METRIC_LADDER_M
        val step = ladder.firstOrNull { it >= accuracyMetres }
            ?: return DistanceClaim.Uncertain(
                "This location fix is accurate only to ${uncertaintyLabel(accuracyMetres, unit)}, " +
                    "which is too coarse to state a distance.",
            )
        val metres = km * 1_000.0
        val bound = maxOf(accuracyMetres, step / 2)
        if (metres <= bound) {
            // Nearer than the evidence can resolve is still something the fix DOES establish. State
            // that bound rather than a point value, and rather than withholding an answer we have.
            return DistanceClaim.Measured(
                "< ${uncertaintyLabel(bound, unit)}",
                approximate = true,
            )
        }
        val label = formatDistanceKm(roundTo(metres, step) / 1_000.0, unit, accuracyMetres)
            ?: return DistanceClaim.Uncertain(DISTANCE_UNAVAILABLE)
        return DistanceClaim.Measured(label, approximate = true)
    }

    /** How the fix's own uncertainty is written, e.g. `±40 m`. Never finer than the value itself. */
    fun accuracyNote(accuracyMetres: Double?, unit: UnitSystem = UnitSystem.Metric): String? {
        if (accuracyMetres == null || !accuracyMetres.isFinite() || accuracyMetres < 0) return null
        return "±${uncertaintyLabel(accuracyMetres, unit)}"
    }

    /**
     * A distance that IS the uncertainty. Rendered from the value itself so its own granularity can
     * never over-claim, unlike running it through the measured-distance bands.
     *
     * It rounds OUTWARD, and every caller is why: a `< bound`, a `±error` and an "accurate only to"
     * all state a ceiling, so a label one display step below its own value understates exactly what
     * it exists to disclose. Rounding to nearest put a provider measured at 92 m behind `< 90 m`.
     *
     * The branch is chosen from the ROUNDED value, not the raw one, so a bound that rounds up to a
     * whole kilometre reads `1.0 km` rather than `1000 m`.
     */
    private fun uncertaintyLabel(metres: Double, unit: UnitSystem): String =
        if (unit == UnitSystem.Imperial) {
            val feet = maxOf(ceilTo(metres * FEET_PER_KM / 1_000.0, 25.0), 25.0)
            if (feet < 1_000) {
                "${feet.toLong()} ft"
            } else {
                String.format(
                    Locale.US,
                    "%.1f mi",
                    ceilTo(metres, TENTH_MILE_M) / 1_000.0 / KM_PER_MILE,
                )
            }
        } else {
            val rounded = maxOf(ceilTo(metres, TEN_METRE_STEP_M), TEN_METRE_STEP_M)
            if (rounded < 1_000) {
                "${rounded.toLong()} m"
            } else {
                String.format(Locale.US, "%.1f km", ceilTo(metres, TENTH_KM_M) / 1_000.0)
            }
        }

    /** Eight-point compass label for the platform-provided initial bearing. */
    fun formatBearing(bearingDegrees: Double?): String? {
        if (bearingDegrees == null || !bearingDegrees.isFinite()) return null
        val normalized = ((bearingDegrees % 360.0) + 360.0) % 360.0
        val index = Math.round(normalized / 45.0).toInt() % compassPoints.size
        return compassPoints[index]
    }

    private fun roundTo(value: Double, step: Double): Double = Math.round(value / step) * step

    /**
     * Rounds up to a whole number of [step]s, tolerating a value that already IS a multiple of a
     * step no double can hold exactly. `MILE_M` is `KM_PER_MILE * 1_000`, which is not exactly
     * 1609.344, so without that tolerance a whole mile would bump itself to `1.1 mi`. The tolerance
     * is expressed in steps, so it can understate by at most a billionth of one - sub-micron here.
     */
    private fun ceilTo(value: Double, step: Double): Double =
        Math.ceil(value / step - CEIL_TOLERANCE) * step
}
