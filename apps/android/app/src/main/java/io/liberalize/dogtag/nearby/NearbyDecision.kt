package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.net.IssuerBindingState
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
        /** Server-computed distances keyed by provider id; insertion/order follows [providers]. */
        val distancesKm: Map<String, Double> = emptyMap(),
        val observation: DirectoryObservation,
        val readAt: Long,
        val expiresAt: Long,
        val total: Int = providers.size,
        val limit: Int = providers.size.coerceAtLeast(1),
        val offset: Int = 0,
        val hasMore: Boolean = false,
        /**
         * Why the LAST load-more attempt did not answer, when the pages already loaded remain valid.
         *
         * Only a transient source failure lands here: "the network dropped on page 5" is a lost place
         * in the list, not evidence that the earlier pages moved, so discarding them would report a
         * could-not-reach as an emptied result. A response proving the underlying set changed still
         * invalidates the whole accumulation and arrives as [Unavailable].
         *
         * It is cleared by the next successful append, so a retried-and-succeeded page never keeps
         * announcing a failure that is over. It is a transient UI marker and is never persisted.
         */
        val pageLoadFailure: String? = null,
    ) : ProviderDirectoryResult

    data class Empty(
        val observation: DirectoryObservation,
        val readAt: Long,
        val expiresAt: Long,
        val total: Int = 0,
        val limit: Int = DEFAULT_PROVIDER_PAGE_SIZE,
        val offset: Int = 0,
        val hasMore: Boolean = false,
    ) : ProviderDirectoryResult

    data class Unavailable(
        val reason: DirectoryUnavailableReason,
        val detail: String,
        val attemptedAt: Long,
    ) : ProviderDirectoryResult
}

sealed interface NearbyOriginState {
    data object AwaitingChoice : NearbyOriginState
    data object Locating : NearbyOriginState
    data object PermissionRefused : NearbyOriginState
    data object LocationUnavailable : NearbyOriginState

    /**
     * [accuracyMetres] is the horizontal uncertainty the device reported for its current fix, and is
     * now the ONLY bound on how finely a distance may be stated: the request sends the exact fix, so
     * it introduces no coarseness of its own to floor against.
     */
    data class Available(
        val point: GeoPoint,
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
    /** [approximate] is true for a device fix, whose own accuracy makes the number inexact. */
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

data class NearbyRow(
    val provider: DirectoryProvider,
    /** Server-computed distance. The device preserves server order and only formats the text. */
    val distanceKm: Double,
    val distance: DistanceClaim,
)

sealed interface NearbyPresentation {
    data object LoadingDirectory : NearbyPresentation
    data class DirectoryUnavailable(val detail: String) : NearbyPresentation
    data class DirectoryEmpty(val observation: DirectoryObservation) : NearbyPresentation
    data object AwaitingOrigin : NearbyPresentation
    data object Locating : NearbyPresentation
    data object PermissionRefused : NearbyPresentation
    data object LocationUnavailable : NearbyPresentation
    data class NoNearbyProviders(val observation: DirectoryObservation) : NearbyPresentation

    data class NoNameMatch(
        val query: String,
        val observation: DirectoryObservation,
    ) : NearbyPresentation

    data class ProvidersFound(
        val rows: List<NearbyRow>,
        val observation: DirectoryObservation,
    ) : NearbyPresentation

    /**
     * The offline fallback: providers the owner has seen before, with NO distance and NO ranking.
     *
     * Deliberately its own case rather than a [ProvidersFound] carrying empty distances. [nearby]
     * drops any provider it has no server distance for and then reports [NoNearbyProviders], so
     * routing remembered records through it would render "no vets near you" about providers the phone
     * is holding in its hand - the false absence this layer exists to prevent.
     *
     * [storedAge] may be `null`: an age that could not be derived says nothing rather than inventing
     * a number. The rows are unranked, so the caller must not present them as nearest-first.
     */
    data class StoredProvidersOnly(
        val providers: List<DirectoryProvider>,
        val storedAge: String?,
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
 * Appends one validated response page without changing server order.
 *
 * Metadata must continue exactly. Nearest pages are identified by a complete distance map; their
 * boundary must also be nondecreasing so pagination cannot turn a malformed response into a list
 * that only appears nearest-first. Contact pages carry no distances and preserve source order.
 *
 * A failure that did not answer at all is told apart from one that proves the result set moved.
 * [DirectoryUnavailableReason.SourceUnavailable] is the transient case: the loaded pages are still the
 * pages the service sent, so they are kept, `hasMore` is left alone so the retry affordance survives,
 * and the reason is carried as [ProviderDirectoryResult.Found.pageLoadFailure]. Every other reason
 * says the response itself cannot be trusted to continue this list, so the accumulation is discarded.
 * Collapsing the two would be the same could-not-tell-them-apart defect this layer exists to prevent.
 */
internal fun appendDirectoryPage(
    current: ProviderDirectoryResult.Found,
    next: ProviderDirectoryResult,
): ProviderDirectoryResult = when (next) {
    is ProviderDirectoryResult.Unavailable ->
        if (next.reason == DirectoryUnavailableReason.SourceUnavailable) {
            current.copy(pageLoadFailure = next.detail)
        } else {
            next
        }
    is ProviderDirectoryResult.Empty -> {
        if (!pageContinues(current, next.offset, next.limit, next.total)) {
            invalidContinuation(next.readAt)
        } else {
            current.copy(hasMore = false, expiresAt = next.expiresAt, pageLoadFailure = null)
        }
    }
    is ProviderDirectoryResult.Found -> {
        if (!pageContinues(current, next.offset, next.limit, next.total)) {
            invalidContinuation(next.readAt)
        } else {
            val existingIds = current.providers.mapTo(HashSet()) { it.providerId }
            when {
                next.providers.any { !existingIds.add(it.providerId) } ->
                    ProviderDirectoryResult.Unavailable(
                        reason = DirectoryUnavailableReason.InvalidSnapshot,
                        detail = "The provider directory repeated a provider across pages; refresh to retry",
                        attemptedAt = next.readAt,
                    )
                !distanceBoundaryContinues(current, next) ->
                    ProviderDirectoryResult.Unavailable(
                        reason = DirectoryUnavailableReason.InvalidSnapshot,
                        detail = "The provider directory returned an out-of-order distance page; refresh to retry",
                        attemptedAt = next.readAt,
                    )
                else -> {
                    val distances = LinkedHashMap(current.distancesKm)
                    distances.putAll(next.distancesKm)
                    current.copy(
                        providers = current.providers + next.providers,
                        distancesKm = distances,
                        hasMore = next.hasMore,
                        expiresAt = next.expiresAt,
                        pageLoadFailure = null,
                    )
                }
            }
        }
    }
}

private fun pageContinues(
    current: ProviderDirectoryResult.Found,
    nextOffset: Int,
    nextLimit: Int,
    nextTotal: Int,
): Boolean {
    val expectedOffset = current.offset + current.providers.size
    return nextOffset == expectedOffset &&
        nextLimit == current.limit &&
        nextTotal == current.total
}

private fun distanceBoundaryContinues(
    current: ProviderDirectoryResult.Found,
    next: ProviderDirectoryResult.Found,
): Boolean {
    val currentIsNearest = current.distancesKm.isNotEmpty()
    val nextIsNearest = next.distancesKm.isNotEmpty()
    if (!currentIsNearest && !nextIsNearest) return true
    if (
        current.distancesKm.size != current.providers.size ||
        next.distancesKm.size != next.providers.size
    ) return false
    val previous = current.providers.lastOrNull()
        ?.let { current.distancesKm[it.providerId] }
        ?: return false
    val following = next.providers.firstOrNull()
        ?.let { next.distancesKm[it.providerId] }
        ?: return false
    return previous <= following
}

private fun invalidContinuation(attemptedAt: Long) = ProviderDirectoryResult.Unavailable(
    reason = DirectoryUnavailableReason.InvalidSnapshot,
    detail = "The provider directory changed while loading the next page; refresh to retry",
    attemptedAt = attemptedAt,
)

/**
 * What the Nearby and Provider contacts scopes are allowed to claim.
 *
 * Pure Kotlin: no Android, networking, clock, JSON, or location APIs. The server supplies ordered
 * distances; this layer controls only eligibility, states, and honest display precision.
 */
object NearbyDecision {
    const val LOCATION_DISCLOSURE =
        "Your location is sent to DogTag to find nearby vets and groomers. It is not stored."
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

    enum class UnitSystem { Metric, Imperial }

    fun nearby(
        directory: ProviderDirectoryResult?,
        origin: NearbyOriginState,
        query: String,
        unit: UnitSystem = UnitSystem.Metric,
    ): NearbyPresentation {
        val available = when (origin) {
            NearbyOriginState.AwaitingChoice -> return NearbyPresentation.AwaitingOrigin
            NearbyOriginState.Locating -> return NearbyPresentation.Locating
            NearbyOriginState.PermissionRefused -> return NearbyPresentation.PermissionRefused
            NearbyOriginState.LocationUnavailable -> return NearbyPresentation.LocationUnavailable
            is NearbyOriginState.Available -> {
                if (!origin.point.isUsable) return NearbyPresentation.LocationUnavailable
                origin
            }
        }

        when (directory) {
            null -> return NearbyPresentation.LoadingDirectory
            is ProviderDirectoryResult.Unavailable ->
                return NearbyPresentation.DirectoryUnavailable(directory.detail)
            is ProviderDirectoryResult.Empty -> {
                val needle = query.trim()
                return if (needle.isNotEmpty()) {
                    NearbyPresentation.NoNameMatch(needle, directory.observation)
                } else {
                    NearbyPresentation.NoNearbyProviders(directory.observation)
                }
            }
            is ProviderDirectoryResult.Found -> Unit
        }

        val found = directory
        val needle = query.trim()
        // Only the fix's OWN uncertainty bounds the label now. The former 100-metre floor existed
        // because the service received a three-decimal coordinate, so no distance computed from it
        // could be finer than that; the captain's exact-position ruling removed that coarsening, and
        // keeping the floor would overstate uncertainty the request no longer introduces. A fix with
        // no usable accuracy still yields `null`, which the claim layer renders as uncertain rather
        // than as a confident number.
        val effectiveAccuracy = available.accuracyMetres
            ?.takeIf { it.isFinite() && it >= 0 }
        val visible = found.providers.mapNotNull { provider ->
            if (!eligible(provider)) return@mapNotNull null
            val serverDistance = found.distancesKm[provider.providerId] ?: return@mapNotNull null
            if (!serverDistance.isFinite() || serverDistance < 0) return@mapNotNull null
            NearbyRow(
                provider = provider,
                distanceKm = serverDistance,
                distance = distanceClaim(
                    km = serverDistance,
                    accuracyMetres = effectiveAccuracy,
                    fromDeviceFix = true,
                    unit = unit,
                ),
            )
        }

        if (visible.isEmpty()) {
            return if (needle.isNotEmpty()) {
                NearbyPresentation.NoNameMatch(needle, found.observation)
            } else {
                NearbyPresentation.NoNearbyProviders(found.observation)
            }
        }
        // Preserve the server's distance order byte-for-byte. Sorting here would both waste work and
        // blur which tier is responsible for the nearest-first contract.
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
            is ProviderDirectoryResult.Empty -> {
                val needle = query.trim()
                return if (needle.isNotEmpty()) {
                    ContactDirectoryPresentation.NoNameMatch(needle, directory.observation)
                } else {
                    ContactDirectoryPresentation.DirectoryEmpty(directory.observation)
                }
            }
            is ProviderDirectoryResult.Found -> Unit
        }
        val found = directory
        val providers = found.providers
            .filter(::eligible)

        if (providers.isEmpty()) {
            val needle = query.trim()
            return if (needle.isNotEmpty()) {
                ContactDirectoryPresentation.NoNameMatch(needle, found.observation)
            } else {
                ContactDirectoryPresentation.DirectoryEmpty(found.observation)
            }
        }
        return ContactDirectoryPresentation.ProvidersFound(providers, found.observation)
    }

    /**
     * What may be shown when a live read could not answer at all and records were remembered.
     *
     * Returns `null` when there is nothing honest to show - nothing remembered, or nothing remembered
     * that matches this search - so the caller keeps the live "could not check" rather than replacing
     * it with a reassuring list. That direction matters: a fallback that quietly answered an empty
     * result would turn could-not-check into an established absence.
     *
     * No distance is computed or carried here, and the order is whatever position-free order the
     * cache stored. Mirrors iOS `NearbyDecision.storedFallback`.
     */
    fun storedFallback(
        records: StoredProviderRecords?,
        query: String,
        nowMillis: Long,
    ): NearbyPresentation.StoredProvidersOnly? {
        if (records == null) return null
        val needle = query.trim().lowercase(Locale.ROOT)
        val providers = records.providers
            .filter(::eligible)
            .filter { needle.isEmpty() || it.name.lowercase(Locale.ROOT).contains(needle) }
        if (providers.isEmpty()) return null
        return NearbyPresentation.StoredProvidersOnly(
            providers = providers,
            storedAge = formatStoredAge(records.storedAt, nowMillis),
        )
    }

    /**
     * The maps-app handoff for one provider: its PUBLISHED destination, and nothing else.
     *
     * This is a handoff, not a map. The owner leaves the app and the map belongs to whichever app the
     * chooser opens, so it costs nothing and needs no key - the 2026-07-29 captain decision that
     * declined an embedded map and a hosted place search kept this affordance as the thing that ships
     * instead.
     *
     * **The origin is never included, and that is the whole privacy property.** The `geo:` scheme has
     * no source parameter and none is synthesised here: the owner's own position must never reach a URI
     * handed to another application, which is precisely the disclosure the body-only nearest request
     * exists to avoid. The destination is public directory data the phone already holds, so handing it
     * over discloses nothing about the owner - which is why this survives the server-nearest pivot
     * untouched: that ruling changed WHO RANKS, not whether a row can open a map. A stored (offline)
     * row may use this too, for the same reason.
     *
     * Returns `null` when the provider published no location. Absence is `geo == null` and ONLY that:
     * `(0, 0)` is a real coordinate off the coast of Ghana, so it is a destination like any other.
     * Mirrors iOS `NearbyDecision.directionsURL`; keep the two in step by hand.
     */
    fun directionsUri(provider: DirectoryProvider): String? {
        val geo = provider.geo?.takeIf { it.isUsable } ?: return null
        // Coordinates only - deliberately no `?q=lat,lng(Name)` label. A provider name is
        // operator-entered free text, so the label form needs a percent-encoder for `&`, `#` and
        // spaces, and that encoder is a correctness surface with nothing to gain: the pin is in the
        // same place either way. It also keeps this URI trivially auditable as "the destination and
        // nothing else", which is the property that actually matters here.
        return "geo:${coordinate(geo.lat)},${coordinate(geo.lng)}"
    }

    /**
     * Fixed-point and locale-independent. `Double.toString()` would emit `1.0E-5` near the meridian,
     * which no maps app parses, and the default locale would emit `1,35` in a comma-decimal locale,
     * silently splitting the coordinate pair in two.
     */
    private fun coordinate(value: Double): String = String.format(Locale.ROOT, "%.6f", value)

    /**
     * How old a remembered record set is, in words.
     *
     * Rounds OUTWARD so a replay never reads fresher than it is, and promotes at the ceiling rather
     * than printing "60 minutes ago". A stored time in the future is a backwards clock jump, not a
     * fresh copy, so it says nothing. Mirrors iOS; keep the two in step by hand.
     */
    fun formatStoredAge(storedAtMillis: Long, nowMillis: Long): String? {
        val elapsed = nowMillis - storedAtMillis
        if (elapsed < 0) return null
        if (elapsed < 60_000) return "less than a minute ago"
        val minutes = ceilDiv(elapsed, 60_000)
        if (minutes < 60) return agePhrase(minutes, "minute")
        val hours = ceilDiv(elapsed, 3_600_000)
        if (hours < 24) return agePhrase(hours, "hour")
        return agePhrase(ceilDiv(elapsed, 86_400_000), "day")
    }

    private fun ceilDiv(value: Long, divisor: Long): Long = (value + divisor - 1) / divisor

    private fun agePhrase(count: Long, unit: String): String =
        if (count == 1L) "1 $unit ago" else "$count ${unit}s ago"

    private fun eligible(provider: DirectoryProvider): Boolean {
        if (provider.active == false) return false
        return provider.kind.trim().lowercase(Locale.ROOT) in setOf("vet", "groomer")
    }

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
     * What this origin's precision permits the row to say about one measured distance.
     *
     * A current-position result is rounded to a step no finer than the device-reported horizontal
     * accuracy, which is now the only bound: the request sends the exact fix and so contributes no
     * coarseness of its own. A fix whose accuracy is missing, nonsensical, or coarser than any usable
     * step yields no number at all.
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
