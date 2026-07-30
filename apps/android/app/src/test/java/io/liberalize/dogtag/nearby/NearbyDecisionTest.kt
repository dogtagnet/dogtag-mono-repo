package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.net.IssuerBindingState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NearbyDecisionTest {
    private val origin = NearbyOriginState.Available(
        point = GeoPoint(1.3521, 103.8198),
        accuracyMetres = 8.0,
    )

    private fun provider(
        id: String,
        name: String = id,
        kind: String = "vet",
        geo: GeoPoint? = GeoPoint(1.35, 103.82),
        active: Boolean? = null,
        contact: ProviderContact = ProviderContact(),
    ) = DirectoryProvider(
        providerId = id,
        kind = kind,
        name = name,
        geo = geo,
        services = listOf("wellness"),
        domain = "example.test",
        active = active,
        contact = contact,
        bindingState = IssuerBindingState.Unavailable,
    )

    private fun found(
        providers: List<DirectoryProvider>,
        distances: Map<String, Double> = emptyMap(),
    ) = ProviderDirectoryResult.Found(
        providers = providers,
        distancesKm = distances,
        observation = DirectoryObservation.Live,
        readAt = 1_000,
        expiresAt = 61_000,
    )

    @Test
    fun serverDistanceAndOrderArePreservedWithoutDeviceRecomputation() {
        val first = provider("first")
        val second = provider("second")
        val state = NearbyDecision.nearby(
            directory = found(
                providers = listOf(first, second),
                distances = linkedMapOf("first" to 0.8, "second" to 4.2),
            ),
            origin = origin,
            query = "",
        ) as NearbyPresentation.ProvidersFound

        assertEquals(listOf("first", "second"), state.rows.map { it.provider.providerId })
        assertEquals(listOf(0.8, 4.2), state.rows.map { it.distanceKm })
    }

    @Test
    fun nearestPaginationRequiresMetadataContinuationAndNondecreasingBoundary() {
        val first = provider("first")
        val second = provider("second")
        val third = provider("third")
        val fourth = provider("fourth")
        val current = ProviderDirectoryResult.Found(
            providers = listOf(first, second),
            distancesKm = linkedMapOf("first" to 1.0, "second" to 2.0),
            observation = DirectoryObservation.Live,
            readAt = 1_000,
            expiresAt = 61_000,
            total = 4,
            limit = 2,
            offset = 0,
            hasMore = true,
        )
        val equalBoundary = ProviderDirectoryResult.Found(
            providers = listOf(third, fourth),
            distancesKm = linkedMapOf("third" to 2.0, "fourth" to 3.0),
            observation = DirectoryObservation.Live,
            readAt = 2_000,
            expiresAt = 62_000,
            total = 4,
            limit = 2,
            offset = 2,
            hasMore = false,
        )

        val merged = appendDirectoryPage(current, equalBoundary)
            as ProviderDirectoryResult.Found
        assertEquals(
            listOf("first", "second", "third", "fourth"),
            merged.providers.map { it.providerId },
        )
        assertEquals(listOf(1.0, 2.0, 2.0, 3.0), merged.providers.map {
            merged.distancesKm[it.providerId]
        })

        val decreasing = equalBoundary.copy(
            distancesKm = linkedMapOf("third" to 1.9, "fourth" to 3.0),
        )
        assertEquals(
            DirectoryUnavailableReason.InvalidSnapshot,
            (appendDirectoryPage(current, decreasing) as ProviderDirectoryResult.Unavailable).reason,
        )

        for (broken in listOf(
            equalBoundary.copy(offset = 3),
            equalBoundary.copy(limit = 3),
            equalBoundary.copy(total = 5),
        )) {
            assertEquals(
                DirectoryUnavailableReason.InvalidSnapshot,
                (appendDirectoryPage(current, broken) as ProviderDirectoryResult.Unavailable).reason,
            )
        }
    }

    /**
     * Mirrors iOS `test_aTransientPageFailureKeepsTheLoadedPagesButAnInvalidatingOneDoesNot`.
     *
     * The two arms must not collapse into each other. A network blip on page 5 loses the owner's place
     * and nothing else, so throwing away four good pages would report a could-not-reach as an emptied
     * list; a response proving the set moved really does make those pages untrustworthy.
     */
    @Test
    fun aTransientPageFailureKeepsTheLoadedPagesButAnInvalidatingOneDoesNot() {
        val current = found(listOf(provider("first")), linkedMapOf("first" to 1.0))
            .copy(total = 2, limit = 1, hasMore = true)

        val transient = appendDirectoryPage(
            current,
            ProviderDirectoryResult.Unavailable(
                reason = DirectoryUnavailableReason.SourceUnavailable,
                detail = "The provider directory could not be reached",
                attemptedAt = 2_000,
            ),
        ) as ProviderDirectoryResult.Found
        assertEquals(listOf("first"), transient.providers.map { it.providerId })
        // `hasMore` survives, so the retry affordance the owner needs is still on screen.
        assertTrue(transient.hasMore)
        assertEquals("The provider directory could not be reached", transient.pageLoadFailure)

        // A retried page that succeeds clears the marker; otherwise the screen would go on announcing
        // a failure that is over.
        val retried = appendDirectoryPage(
            transient,
            found(listOf(provider("second")), linkedMapOf("second" to 2.0))
                .copy(total = 2, limit = 1, offset = 1, hasMore = false),
        ) as ProviderDirectoryResult.Found
        assertEquals(listOf("first", "second"), retried.providers.map { it.providerId })
        assertNull(retried.pageLoadFailure)

        for (invalidating in listOf(
            DirectoryUnavailableReason.MalformedResponse,
            DirectoryUnavailableReason.InvalidSnapshot,
        )) {
            val discarded = appendDirectoryPage(
                current,
                ProviderDirectoryResult.Unavailable(
                    reason = invalidating,
                    detail = "changed underneath",
                    attemptedAt = 2_000,
                ),
            )
            assertEquals(
                invalidating,
                (discarded as ProviderDirectoryResult.Unavailable).reason,
            )
        }
    }

    @Test
    fun contactPaginationPreservesServerOrderWithoutInventingDistances() {
        val first = provider("first", geo = null)
        val second = provider("second", geo = null)
        val current = found(listOf(first)).copy(total = 2, limit = 1, hasMore = true)
        val next = found(listOf(second)).copy(
            total = 2,
            limit = 1,
            offset = 1,
            hasMore = false,
        )
        val merged = appendDirectoryPage(current, next) as ProviderDirectoryResult.Found
        assertEquals(listOf("first", "second"), merged.providers.map { it.providerId })
        assertTrue(merged.distancesKm.isEmpty())
    }

    @Test
    fun ownerSurfaceKeepsOnlyVetsAndGroomersAsADefensiveSecondBoundary() {
        val vet = provider("vet", kind = "VeT")
        val groomer = provider("groomer", kind = "groomer")
        val admin = provider("admin", kind = "admin")
        val government = provider("government", kind = "government")
        val state = NearbyDecision.nearby(
            directory = found(
                listOf(vet, groomer, admin, government),
                linkedMapOf(
                    "vet" to 1.0,
                    "groomer" to 2.0,
                    "admin" to 0.1,
                    "government" to 0.2,
                ),
            ),
            origin = origin,
            query = "",
        ) as NearbyPresentation.ProvidersFound

        assertEquals(listOf("vet", "groomer"), state.rows.map { it.provider.providerId })
    }

    @Test
    fun theOnlyDisplayFloorIsTheFixesOwnAccuracy() {
        val clinic = provider("clinic")

        // Captain's ruling, 2026-07-30: the exact fix is sent, so the request introduces no coarseness
        // of its own. The former 100-metre floor came from the three-decimal approximation and would
        // now overstate uncertainty; an 8-metre fix may state a 40-metre distance.
        val precise = NearbyDecision.nearby(
            directory = found(listOf(clinic), mapOf("clinic" to 0.04)),
            origin = origin.copy(accuracyMetres = 8.0),
            query = "",
        ) as NearbyPresentation.ProvidersFound
        assertEquals(
            DistanceClaim.Measured("40 m", approximate = true),
            precise.rows.single().distance,
        )

        // A coarse fix still floors the label at its own accuracy: a 250-metre fix may not print 40 m.
        // The bound itself rounds OUTWARD onto the display ladder, so 250 m is stated as "< 500 m" -
        // a bound may never read tighter than the accuracy that produced it.
        val coarse = NearbyDecision.nearby(
            directory = found(listOf(clinic), mapOf("clinic" to 0.04)),
            origin = origin.copy(accuracyMetres = 250.0),
            query = "",
        ) as NearbyPresentation.ProvidersFound
        assertEquals(
            DistanceClaim.Measured("< 500 m", approximate = true),
            coarse.rows.single().distance,
        )
    }

    @Test
    fun malformedOrMissingServerDistanceNeverBecomesADeviceMeasurement() {
        val missing = provider("missing")
        val invalid = provider("invalid")
        val state = NearbyDecision.nearby(
            directory = found(
                listOf(missing, invalid),
                mapOf("invalid" to Double.NaN),
            ),
            origin = origin,
            query = "",
        )
        assertEquals(
            NearbyPresentation.NoNearbyProviders(DirectoryObservation.Live),
            state,
        )
    }

    @Test
    fun permissionAndDirectoryFailureStatesDoNotCollapse() {
        assertEquals(
            NearbyPresentation.AwaitingOrigin,
            NearbyDecision.nearby(null, NearbyOriginState.AwaitingChoice, ""),
        )
        assertEquals(
            NearbyPresentation.PermissionRefused,
            NearbyDecision.nearby(null, NearbyOriginState.PermissionRefused, ""),
        )
        assertEquals(
            NearbyPresentation.LocationUnavailable,
            NearbyDecision.nearby(null, NearbyOriginState.LocationUnavailable, ""),
        )

        val unavailable = ProviderDirectoryResult.Unavailable(
            DirectoryUnavailableReason.SourceUnavailable,
            "central offline",
            2_000,
        )
        assertEquals(
            NearbyPresentation.DirectoryUnavailable("central offline"),
            NearbyDecision.nearby(unavailable, origin, ""),
        )
    }

    @Test
    fun emptyNameSearchAndEmptyNearbyResultRemainDistinct() {
        val empty = ProviderDirectoryResult.Empty(
            DirectoryObservation.Live,
            1_000,
            61_000,
        )
        assertEquals(
            NearbyPresentation.NoNameMatch("north star", DirectoryObservation.Live),
            NearbyDecision.nearby(empty, origin, " north star "),
        )
        assertEquals(
            NearbyPresentation.NoNearbyProviders(DirectoryObservation.Live),
            NearbyDecision.nearby(empty, origin, ""),
        )
    }

    @Test
    fun contactSearchPreservesServerOrderAndIncludesLocationlessProviders() {
        val contactOnly = provider(
            id = "contact",
            name = "Call-only Vet",
            geo = null,
            contact = ProviderContact(phone = "+65 6123 4567"),
        )
        val located = provider("located", kind = "groomer")
        val state = NearbyDecision.contacts(found(listOf(contactOnly, located)), "call")
            as ContactDirectoryPresentation.ProvidersFound

        assertEquals(listOf("contact", "located"), state.providers.map { it.providerId })
        assertTrue(state.providers.first().contact.hasAny)
    }

    @Test
    fun delistedRowsStayHiddenWithoutInventingDelistingForUnknownState() {
        val delisted = provider("off", active = false)
        val unknown = provider("unknown", active = null)
        val state = NearbyDecision.contacts(found(listOf(delisted, unknown)), "")
            as ContactDirectoryPresentation.ProvidersFound
        assertEquals(listOf("unknown"), state.providers.map { it.providerId })
    }

    @Test
    fun disclosurePlainlyStatesSendPurposeAndRetentionAtTheGrantAction() {
        assertEquals(
            "Your location is sent to DogTag to find nearby vets and groomers. It is not stored.",
            NearbyDecision.LOCATION_DISCLOSURE,
        )
    }

    /**
     * The captain allowed Directions on offline stored rows on condition that the stored-not-current
     * labelling stayed on them, so this sentence is part of that ruling rather than decoration: a
     * bare Directions button on a remembered row would read as a destination just confirmed with the
     * service. Pinned byte-for-byte against iOS for the same reason the disclosure above is.
     */
    @Test
    fun theStoredDirectionsOfferSaysTheAddressMayBeOutOfDate() {
        assertEquals(
            "Saved on this phone - this address may be out of date.",
            NearbyDecision.STORED_DIRECTIONS_NOTE,
        )
    }

    @Test
    fun zeroZeroRemainsARealCoordinateAndCanBeCoarsened() {
        val point = GeoPoint(0.0, 0.0)
        assertTrue(point.isUsable)
        val approximate = CallerPosition.from(point)
        assertEquals(0.0, approximate?.lat ?: Double.NaN, 0.0)
        assertEquals(0.0, approximate?.lng ?: Double.NaN, 0.0)
    }

    @Test
    fun coarseOrMissingFixAccuracyNeverProducesAFalsePreciseNumber() {
        val missing = NearbyDecision.distanceClaim(3.44, null, fromDeviceFix = true)
        assertTrue(missing is DistanceClaim.Uncertain)

        val coarse = NearbyDecision.distanceClaim(3.44, 900.0, fromDeviceFix = true)
        assertEquals(DistanceClaim.Measured("3 km", approximate = true), coarse)

        val tooCoarse = NearbyDecision.distanceClaim(30.0, 25_000.0, fromDeviceFix = true)
        assertTrue(tooCoarse is DistanceClaim.Uncertain)
    }

    @Test
    fun metricAndImperialFormattingDoNotCollapsePositiveDistancesToZero() {
        assertEquals(
            DistanceClaim.Measured("< 500 m", approximate = true),
            NearbyDecision.distanceClaim(0.05, 150.0, fromDeviceFix = true),
        )
        val imperial = NearbyDecision.distanceClaim(
            0.05,
            8.0,
            fromDeviceFix = true,
            unit = NearbyDecision.UnitSystem.Imperial,
        )
        assertTrue(imperial is DistanceClaim.Measured)
        assertFalse((imperial as DistanceClaim.Measured).display.contains("0 ft"))
    }

    @Test
    fun localeRegionControlsUnitsWithoutUsingProviderData() {
        assertEquals(
            NearbyDecision.UnitSystem.Imperial,
            NearbyDecision.unitSystemForRegion("en-US"),
        )
        assertEquals(
            NearbyDecision.UnitSystem.Metric,
            NearbyDecision.unitSystemForRegion("en-SG"),
        )
    }

    @Test
    fun aCoarseFixNeverRendersAFinerNumberThanItSupports() {
        assertEquals(
            DistanceClaim.Measured("3.4 km", approximate = false),
            NearbyDecision.distanceClaim(3.44, null, fromDeviceFix = false),
        )
        assertEquals(
            DistanceClaim.Measured("3.4 km", approximate = true),
            NearbyDecision.distanceClaim(3.44, 8.0, fromDeviceFix = true),
        )
        assertEquals(
            DistanceClaim.Measured("3 km", approximate = true),
            NearbyDecision.distanceClaim(3.44, 900.0, fromDeviceFix = true),
        )
    }

    @Test
    fun noPositiveDistanceCollapsesToAConfidentZero() {
        val windows = listOf(
            Triple(1_200.0, 3.0, "< 5.0 km"),
            Triple(150.0, 0.3, "< 500 m"),
            Triple(30.0, 0.04, "< 50 m"),
            Triple(3.0, 0.004, "< 10 m"),
        )
        for ((accuracy, km, expected) in windows) {
            val claim = NearbyDecision.distanceClaim(km, accuracy, fromDeviceFix = true)
            assertEquals("$accuracy/$km", DistanceClaim.Measured(expected, true), claim)
        }

        val imperialWindows = listOf(
            Triple(1_200.0, 3.0, "2 mi"),
            Triple(150.0, 0.3, "0.2 mi"),
            Triple(8.0, 0.05, "< 275 ft"),
        )
        for ((accuracy, km, expected) in imperialWindows) {
            val claim = NearbyDecision.distanceClaim(
                km,
                accuracy,
                fromDeviceFix = true,
                unit = NearbyDecision.UnitSystem.Imperial,
            )
            assertEquals("$accuracy/$km", DistanceClaim.Measured(expected, true), claim)
        }
    }

    @Test
    fun aDistanceJustAboveTheBoundStillStatesANonZeroNumber() {
        val cases = listOf(
            NearbyDecision.UnitSystem.Metric to listOf(
                0.009 to 8.0,
                0.06 to 30.0,
                0.6 to 150.0,
                6.0 to 1_200.0,
            ),
            NearbyDecision.UnitSystem.Imperial to listOf(
                0.005 to 3.0,
                0.09 to 80.0,
                0.9 to 300.0,
                9.0 to 1_200.0,
            ),
        )
        for ((unit, pairs) in cases) {
            for ((km, accuracy) in pairs) {
                val claim = NearbyDecision.distanceClaim(km, accuracy, true, unit)
                    as DistanceClaim.Measured
                assertFalse("$unit $accuracy/$km -> ${claim.label}", claim.label.startsWith("0 "))
                assertFalse("$unit $accuracy/$km -> ${claim.label}", claim.label.startsWith("0.0 "))
            }
        }
    }

    @Test
    fun theCoarsestUsableFixIsPerUnitAndItsRefusalNamesTheAccuracy() {
        val metric = NearbyDecision.distanceClaim(30.0, 10_001.0, fromDeviceFix = true)
        assertTrue(metric is DistanceClaim.Uncertain)
        assertTrue((metric as DistanceClaim.Uncertain).reason.contains("too coarse"))

        assertEquals(
            DistanceClaim.Measured("20 mi", approximate = true),
            NearbyDecision.distanceClaim(
                30.0,
                12_000.0,
                fromDeviceFix = true,
                unit = NearbyDecision.UnitSystem.Imperial,
            ),
        )
        val beyondImperial = NearbyDecision.distanceClaim(
            30.0,
            17_000.0,
            fromDeviceFix = true,
            unit = NearbyDecision.UnitSystem.Imperial,
        )
        assertTrue(beyondImperial is DistanceClaim.Uncertain)
    }

    @Test
    fun everyBoundLabelCoversTheAccuracyItWasDerivedFrom() {
        val accuracies = listOf(
            3.0,
            9.4,
            40.0,
            94.0,
            100.0,
            160.9344,
            400.0,
            850.0,
            944.0,
            1_609.344,
            5_040.0,
            9_400.0,
        )
        for (unit in NearbyDecision.UnitSystem.entries) {
            for (accuracy in accuracies) {
                val claim = NearbyDecision.distanceClaim(accuracy / 1_000.0, accuracy, true, unit)
                    as DistanceClaim.Measured
                assertTrue("$unit $accuracy -> ${claim.label}", claim.label.startsWith("< "))
                assertTrue(
                    "$unit $accuracy -> ${claim.label}",
                    boundMetres(claim.label) >= accuracy - 1e-6,
                )
            }
        }
    }

    @Test
    fun serverDistanceCarriesOriginPrecisionIntoTheRenderedClaim() {
        val clinic = provider("clinic")
        val state = NearbyDecision.nearby(
            found(listOf(clinic), mapOf("clinic" to 3.44)),
            NearbyOriginState.Available(
                point = GeoPoint(1.3521, 103.8198),
                accuracyMetres = 900.0,
            ),
            "",
        ) as NearbyPresentation.ProvidersFound

        assertEquals(3.44, state.rows.single().distanceKm, 0.0)
        assertEquals(
            DistanceClaim.Measured("3 km", approximate = true),
            state.rows.single().distance,
        )
        assertNull(NearbyDecision.accuracyNote(null))
        assertEquals("±40 m", NearbyDecision.accuracyNote(38.0))
    }

    @Test
    fun distanceFormattingDoesNotOverclaimPrecision() {
        assertEquals("< 10 m", NearbyDecision.formatDistanceKm(0.003))
        assertEquals("820 m", NearbyDecision.formatDistanceKm(0.823))
        assertEquals("3.4 km", NearbyDecision.formatDistanceKm(3.36))
        assertEquals("27 km", NearbyDecision.formatDistanceKm(27.2))
        assertEquals(
            "< 25 ft",
            NearbyDecision.formatDistanceKm(0.003, NearbyDecision.UnitSystem.Imperial),
        )
        assertNull(NearbyDecision.formatDistanceKm(Double.NaN))
    }

    private fun boundMetres(label: String): Double {
        val parts = label.removePrefix("< ").split(" ")
        val value = parts[0].toDouble()
        return when (parts[1]) {
            "m" -> value
            "km" -> value * 1_000
            "ft" -> value * 0.3048
            "mi" -> value * 1_609.344
            else -> throw AssertionError("unexpected unit in $label")
        }
    }

    // ---- The offline stored fallback (captain's cache ruling, 2026-07-30) ----

    /**
     * The remembered set is UNRANKED and carries no distance, so it must not be routed through
     * [NearbyDecision.nearby]: that drops every provider it has no server distance for and then
     * reports "no nearby providers", which would state an absence about providers the phone is
     * holding. This is the case that pins the separate presentation.
     */
    @Test
    fun theStoredFallbackPresentsRememberedProvidersWithoutDistanceOrRanking() {
        val records = StoredProviderRecords(
            providers = listOf(provider("a", "Alpha Vet"), provider("b", "Beta Groomer", kind = "groomer")),
            storedAt = 1_000_000L,
        )

        val shown = NearbyDecision.storedFallback(records, "", 1_000_000L + 120_000L)
        assertEquals(listOf("Alpha Vet", "Beta Groomer"), shown?.providers?.map { it.name })
        assertEquals("2 minutes ago", shown?.storedAge)

        // Routed through the live presentation instead, the same records would claim there are none.
        val throughNearby = NearbyDecision.nearby(
            directory = ProviderDirectoryResult.Found(
                providers = records.providers,
                distancesKm = emptyMap(),
                observation = DirectoryObservation.Stored,
                readAt = 0L,
                expiresAt = 0L,
            ),
            origin = NearbyOriginState.Available(GeoPoint(1.35, 103.82), accuracyMetres = 50.0),
            query = "",
        )
        assertTrue(throughNearby is NearbyPresentation.NoNearbyProviders)
    }

    /** Nothing remembered, or nothing matching, keeps the live could-not-check rather than answering. */
    @Test
    fun theStoredFallbackDeclinesRatherThanAnsweringAnEmptyList() {
        assertNull(NearbyDecision.storedFallback(null, "", 1_000_000L))
        val records = StoredProviderRecords(listOf(provider("a", "Alpha Vet")), storedAt = 1_000_000L)
        assertNull(NearbyDecision.storedFallback(records, "no such provider", 1_000_000L))
    }

    /** A delisted provider stays hidden offline too; an unknown listing state remains eligible. */
    @Test
    fun theStoredFallbackStillHidesDelistedProvidersAndOwnerForeignKinds() {
        val records = StoredProviderRecords(
            providers = listOf(
                provider("off", "Closed Vet", active = false),
                provider("gov", "Ministry", kind = "government"),
                provider("ok", "Open Vet", active = null),
            ),
            storedAt = 1_000_000L,
        )

        assertEquals(
            listOf("Open Vet"),
            NearbyDecision.storedFallback(records, "", 1_000_000L)?.providers?.map { it.name },
        )
    }

    /**
     * The age rounds OUTWARD so a replay never reads fresher than it is, and a stored time in the
     * future is a backwards clock jump rather than a fresh copy, so it says nothing at all.
     * Mirrors iOS `test_theStoredAgeIsCoarseAndNeverUnderstatesStaleness`.
     */
    @Test
    fun theStoredAgeIsCoarseAndNeverUnderstatesStaleness() {
        val at = 1_000_000L
        fun age(elapsedMs: Long) = NearbyDecision.formatStoredAge(at, at + elapsedMs)

        assertEquals("less than a minute ago", age(0))
        assertEquals("less than a minute ago", age(59_000))
        assertEquals("1 minute ago", age(60_000))
        // 61 seconds is stated as two minutes, never as one.
        assertEquals("2 minutes ago", age(61_000))
        assertEquals("1 hour ago", age(3_599_000))
        assertEquals("1 hour ago", age(3_600_000))
        assertEquals("2 hours ago", age(3_601_000))
        assertEquals("1 day ago", age(86_399_000))
        assertEquals("1 day ago", age(86_400_000))
        assertEquals("6 days ago", age(6 * 86_400_000L))

        assertNull(age(-1))
    }

    // ---- The Directions handoff ----

    /**
     * THE property of this affordance: the URI carries the provider's published destination and no
     * trace of where the owner is. The `geo:` scheme has no source parameter and none is synthesised,
     * so a URI handed to another application can never disclose the owner's own position - the same
     * confinement the body-only nearest request exists to provide.
     *
     * Mirrors iOS `test_theDirectionsHandoffCarriesTheDestinationAndNeverTheOrigin`.
     */
    @Test
    fun theDirectionsHandoffCarriesTheDestinationAndNeverTheOrigin() {
        val uri = NearbyDecision.directionsUri(provider("a", geo = GeoPoint(1.35249, 103.81951)))

        assertEquals("geo:1.352490,103.819510?q=1.352490,103.819510", uri)
        // The owner's own fix from this file's `origin` is 1.3521,103.8198 - no part of it may appear.
        assertFalse("the origin must never reach the maps handoff", uri!!.contains("1.3521"))
        assertFalse("the origin must never reach the maps handoff", uri.contains("103.8198"))
        assertFalse("the geo: scheme has no source parameter", uri.contains("saddr"))
    }

    /**
     * A provider that published no location offers no Directions. Absence is `geo == null` and only
     * that: `(0, 0)` is a real coordinate off the coast of Ghana, so it routes like anywhere else.
     * Reading it as absence is the bug this repo already fixed once in the admin directory.
     */
    @Test
    fun onlyAnAbsentLocationWithholdsDirectionsAndZeroZeroIsARealDestination() {
        assertNull(NearbyDecision.directionsUri(provider("contact-only", geo = null)))
        assertEquals(
            "geo:0.000000,0.000000?q=0.000000,0.000000",
            NearbyDecision.directionsUri(provider("gulf-of-guinea", geo = GeoPoint(0.0, 0.0))),
        )
        // An unusable coordinate is not a destination either.
        assertNull(NearbyDecision.directionsUri(provider("broken", geo = GeoPoint(91.0, 0.0))))
        assertNull(NearbyDecision.directionsUri(provider("nan", geo = GeoPoint(Double.NaN, 0.0))))
    }

    /**
     * Fixed-point, locale-independent, and signed. `Double.toString()` would emit `1.0E-5` just off
     * the meridian - which no maps app parses - and a default-locale formatter would emit `1,35` in a
     * comma-decimal locale, silently splitting the pair into two coordinates.
     */
    @Test
    fun directionsCoordinatesAreFixedPointAndSurviveBothSigns() {
        assertEquals(
            "geo:-33.865510,-151.209900?q=-33.865510,-151.209900",
            NearbyDecision.directionsUri(provider("s", geo = GeoPoint(-33.86551, -151.2099))),
        )
        assertEquals(
            "geo:0.000010,0.000000?q=0.000010,0.000000",
            NearbyDecision.directionsUri(provider("meridian", geo = GeoPoint(0.00001, 0.0))),
        )
    }
}
