package io.liberalize.dogtag.nearby

import io.liberalize.dogtag.net.IssuerBindingState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Mirrored by iOS `NearbyDecisionTests`: both apps are pinned to the same visibility and claim
 * policy without either test needing a device, location service, network, or FFI.
 */
class NearbyDecisionTest {
    private val origin = NearbyOriginState.Available(
        GeoPoint(1.3521, 103.8198),
        OriginSource.ChosenCoordinates,
    )

    private fun provider(
        id: String,
        name: String = id,
        kind: String = "vet",
        geo: GeoPoint? = GeoPoint(1.35, 103.82),
        active: Boolean? = null,
        contact: ProviderContact = ProviderContact(),
        bindingState: IssuerBindingState = IssuerBindingState.Unavailable,
    ) = DirectoryProvider(
        providerId = id,
        kind = kind,
        name = name,
        geo = geo,
        services = listOf("wellness"),
        domain = "example.test",
        active = active,
        contact = contact,
        bindingState = bindingState,
    )

    private fun found(vararg providers: DirectoryProvider) = ProviderDirectoryResult.Found(
        providers = providers.toList(),
        observation = DirectoryObservation.Live,
        readAt = 1_000,
        expiresAt = 61_000,
    )

    private fun measured(provider: DirectoryProvider, km: Double, bearing: Double? = 45.0) =
        ProviderMeasurement(provider.providerId, km, bearing)

    @Test
    fun aLocationlessProviderNeverAppearsNearbyButRemainsReachableInContacts() {
        val contactOnly = provider(
            id = "contact",
            name = "Call-only Vet",
            geo = null,
            contact = ProviderContact(phone = "+65 6123 4567"),
        )

        val nearby = NearbyDecision.nearby(found(contactOnly), origin, emptyList(), "", 50.0)
        assertTrue(nearby is NearbyPresentation.NoneWithinRange)

        val contacts = NearbyDecision.contacts(found(contactOnly), "")
        assertTrue(contacts is ContactDirectoryPresentation.ProvidersFound)
        contacts as ContactDirectoryPresentation.ProvidersFound
        assertEquals(listOf("contact"), contacts.providers.map { it.providerId })
        assertTrue(contacts.providers.single().contact.hasAny)
    }

    @Test
    fun zeroZeroIsARealCoordinateNotTheMissingLocationSentinel() {
        val gulf = provider("gulf", geo = GeoPoint(0.0, 0.0))
        assertTrue(gulf.geo?.isUsable == true)

        val state = NearbyDecision.nearby(
            found(gulf),
            origin,
            listOf(measured(gulf, 12.0, 270.0)),
            "",
            50.0,
        )
        assertTrue(state is NearbyPresentation.ProvidersFound)
        state as NearbyPresentation.ProvidersFound
        assertTrue(state.rows.single().canOpenDirections)
        assertEquals(270.0, state.rows.single().bearingDegrees!!, 0.0)
    }

    @Test
    fun nameSearchCoversEveryLocatedProviderNotOnlyTheDefaultRadius() {
        val near = provider("near", name = "Neighbourhood Vet")
        val far = provider("far", name = "North Star Veterinary")
        val directory = found(near, far)
        val measurements = listOf(measured(near, 2.0), measured(far, 420.0))

        val locality = NearbyDecision.nearby(directory, origin, measurements, "", 50.0)
        locality as NearbyPresentation.ProvidersFound
        assertEquals(listOf("near"), locality.rows.map { it.provider.providerId })

        val search = NearbyDecision.nearby(directory, origin, measurements, "north star", 50.0)
        search as NearbyPresentation.ProvidersFound
        assertEquals(listOf("far"), search.rows.map { it.provider.providerId })
        assertEquals(420.0, search.rows.single().distanceKm, 0.0)
    }

    @Test
    fun nameSearchIsDiacriticInsensitive() {
        val clinic = provider("accented", name = "Clínica São Francisco")
        val search = NearbyDecision.nearby(
            found(clinic),
            origin,
            listOf(measured(clinic, 70.0)),
            "clinica sao",
        )
        search as NearbyPresentation.ProvidersFound
        assertEquals(listOf("accented"), search.rows.map { it.provider.providerId })
    }

    @Test
    fun radiusBoundaryIsInclusiveAndRowsAreNearestFirst() {
        val edge = provider("edge")
        val nearer = provider("nearer")
        val state = NearbyDecision.nearby(
            found(edge, nearer),
            origin,
            listOf(measured(edge, 50.0), measured(nearer, 1.5)),
            "",
            50.0,
        )
        state as NearbyPresentation.ProvidersFound
        assertEquals(listOf("nearer", "edge"), state.rows.map { it.provider.providerId })
    }

    @Test
    fun equalDistancesKeepDirectoryOrder() {
        val zulu = provider("z", name = "Zulu Vet")
        val alpha = provider("a", name = "Alpha Vet")
        val state = NearbyDecision.nearby(
            found(zulu, alpha),
            origin,
            listOf(measured(zulu, 5.0), measured(alpha, 5.0)),
            "",
        )
        state as NearbyPresentation.ProvidersFound
        assertEquals(listOf("z", "a"), state.rows.map { it.provider.providerId })
    }

    @Test
    fun delistedIsHiddenButUnknownActiveStateDoesNotInventDelisting() {
        val delisted = provider("off", active = false)
        val unknown = provider("unknown", active = null)
        val measurements = listOf(measured(delisted, 1.0), measured(unknown, 2.0))

        val nearby = NearbyDecision.nearby(found(delisted, unknown), origin, measurements, "", 50.0)
        nearby as NearbyPresentation.ProvidersFound
        assertEquals(listOf("unknown"), nearby.rows.map { it.provider.providerId })

        val contacts = NearbyDecision.contacts(found(delisted, unknown), "")
        contacts as ContactDirectoryPresentation.ProvidersFound
        assertEquals(listOf("unknown"), contacts.providers.map { it.providerId })
    }

    @Test
    fun onlyVetsAndGroomersAreEligible() {
        val vet = provider("vet", kind = "VeT")
        val groomer = provider("groomer", kind = "groomer")
        val government = provider("government", kind = "government")
        val state = NearbyDecision.nearby(
            found(vet, groomer, government),
            origin,
            listOf(measured(vet, 1.0), measured(groomer, 2.0), measured(government, 0.5)),
            "",
        )
        state as NearbyPresentation.ProvidersFound
        assertEquals(listOf("vet", "groomer"), state.rows.map { it.provider.providerId })
    }

    @Test
    fun anUnusableMeasurementCannotBecomeADistanceOrBearingClaim() {
        val clinic = provider("clinic")
        val state = NearbyDecision.nearby(
            found(clinic),
            origin,
            listOf(measured(clinic, Double.NaN, 0.0)),
            "",
        )
        assertTrue(state is NearbyPresentation.NoneWithinRange)

        val noBearing = NearbyDecision.nearby(
            found(clinic),
            origin,
            listOf(measured(clinic, 1.0, Double.NaN)),
            "",
        )
        noBearing as NearbyPresentation.ProvidersFound
        assertNull(noBearing.rows.single().bearingDegrees)
    }

    @Test
    fun directoryFailureEmptyAndPermissionRefusalNeverCollapse() {
        val unavailable = NearbyDecision.nearby(
            ProviderDirectoryResult.Unavailable(
                DirectoryUnavailableReason.SourceUnavailable,
                "central offline",
                2_000,
            ),
            NearbyOriginState.PermissionRefused,
            emptyList(),
            "",
        )
        assertEquals(
            NearbyPresentation.DirectoryUnavailable("central offline"),
            unavailable,
        )

        val empty = NearbyDecision.nearby(
            ProviderDirectoryResult.Empty(DirectoryObservation.Live, 1_000, 61_000),
            origin,
            emptyList(),
            "",
        )
        assertEquals(
            NearbyPresentation.DirectoryEmpty(DirectoryObservation.Live),
            empty,
        )

        val clinic = provider("clinic")
        val refused = NearbyDecision.nearby(
            found(clinic),
            NearbyOriginState.PermissionRefused,
            emptyList(),
            "",
        )
        assertEquals(NearbyPresentation.PermissionRefused, refused)
    }

    @Test
    fun aMissingLocalCandidateWinsBeforeAnyOriginPromptOrRefusal() {
        val contactOnly = provider("contact", geo = null)
        assertTrue(
            NearbyDecision.nearby(
                found(contactOnly),
                NearbyOriginState.AwaitingChoice,
                emptyList(),
                "",
            ) is NearbyPresentation.NoneWithinRange,
        )
        assertTrue(
            NearbyDecision.nearby(
                found(contactOnly),
                NearbyOriginState.PermissionRefused,
                emptyList(),
                "missing",
            ) is NearbyPresentation.NoNameMatch,
        )

        val clinic = provider("clinic", name = "North Star Veterinary")
        assertTrue(
            NearbyDecision.nearby(
                found(clinic),
                NearbyOriginState.AwaitingChoice,
                emptyList(),
                "other practice",
            ) is NearbyPresentation.NoNameMatch,
        )
        assertEquals(
            NearbyPresentation.AwaitingOrigin,
            NearbyDecision.nearby(
                found(clinic),
                NearbyOriginState.AwaitingChoice,
                emptyList(),
                "north",
            ),
        )
    }

    @Test
    fun noNameMatchIsNotReportedAsNoneWithinRange() {
        val clinic = provider("clinic", name = "North Star Veterinary")
        val state = NearbyDecision.nearby(
            found(clinic),
            origin,
            listOf(measured(clinic, 1.0)),
            "other practice",
        )
        assertTrue(state is NearbyPresentation.NoNameMatch)
        assertFalse(state is NearbyPresentation.NoneWithinRange)
    }

    @Test
    fun chosenCoordinatesAreParsedLocallyAndValidatedWithoutASentinel() {
        assertEquals(GeoPoint(0.0, 0.0), NearbyDecision.parseChosenOrigin(" 0 ", "0"))
        assertEquals(GeoPoint(-33.86, 151.21), NearbyDecision.parseChosenOrigin("-33.86", "151.21"))
        assertNull(NearbyDecision.parseChosenOrigin("", "103.8"))
        assertNull(NearbyDecision.parseChosenOrigin("91", "0"))
        assertNull(NearbyDecision.parseChosenOrigin("1", "181"))
        assertNull(NearbyDecision.parseChosenOrigin("NaN", "1"))
    }

    @Test
    fun aConstructedInvalidAvailableOriginIsLocationUnavailable() {
        val clinic = provider("clinic")
        val state = NearbyDecision.nearby(
            found(clinic),
            NearbyOriginState.Available(
                GeoPoint(91.0, 0.0),
                OriginSource.ChosenCoordinates,
            ),
            listOf(measured(clinic, 1.0)),
            "",
        )
        assertEquals(NearbyPresentation.LocationUnavailable, state)
    }

    @Test
    fun contactsAreUnrankedNameSortedAndSearchIsPhoneLocal() {
        val zulu = provider("z", name = "Zulu Grooming", geo = null)
        val alpha = provider("a", name = "Alpha Vet", geo = GeoPoint(1.0, 1.0))
        val all = NearbyDecision.contacts(found(zulu, alpha), "")
        all as ContactDirectoryPresentation.ProvidersFound
        assertEquals(listOf("a", "z"), all.providers.map { it.providerId })

        val search = NearbyDecision.contacts(found(zulu, alpha), "GROOM")
        search as ContactDirectoryPresentation.ProvidersFound
        assertEquals(listOf("z"), search.providers.map { it.providerId })
    }

    @Test
    fun existingBindingStatePassesThroughWithoutAListingSpecificEnum() {
        val noDomain = provider(
            id = "plain",
            bindingState = IssuerBindingState.NoDomainClaimed,
        )
        val state = NearbyDecision.nearby(
            found(noDomain),
            origin,
            listOf(measured(noDomain, 1.0)),
            "",
        )
        state as NearbyPresentation.ProvidersFound
        assertTrue(state.rows.single().provider.bindingState === IssuerBindingState.NoDomainClaimed)
    }

    /**
     * Mirrored by iOS `test_aCoarseFixNeverRendersAFinerNumberThanItSupports`.
     *
     * Coarse collection is only honest if the display admits how coarse it is: the same 3.44 km
     * measurement must read differently from an exact chosen coordinate, a ten-metre fix and a
     * hundred-metre fix, and the raw measurement must survive untouched for ordering.
     */
    @Test
    fun aCoarseFixNeverRendersAFinerNumberThanItSupports() {
        val exact = NearbyDecision.distanceClaim(3.44, null, fromDeviceFix = false)
        assertEquals(DistanceClaim.Measured("3.4 km", approximate = false), exact)

        val fine = NearbyDecision.distanceClaim(3.44, 8.0, fromDeviceFix = true)
        assertEquals(DistanceClaim.Measured("3.4 km", approximate = true), fine)

        val coarse = NearbyDecision.distanceClaim(3.44, 900.0, fromDeviceFix = true)
        assertEquals(DistanceClaim.Measured("3 km", approximate = true), coarse)

        val nearFix = NearbyDecision.distanceClaim(0.823, 90.0, fromDeviceFix = true)
        assertEquals(DistanceClaim.Measured("0.8 km", approximate = true), nearFix)
    }

    @Test
    fun aFixTooCoarseOrTooBrokenToPlaceAProviderStatesUncertaintyInsteadOfANumber() {
        for (accuracy in listOf(null, Double.NaN, -1.0)) {
            val claim = NearbyDecision.distanceClaim(3.44, accuracy, fromDeviceFix = true)
            assertTrue("$accuracy", claim is DistanceClaim.Uncertain)
        }
        val tooCoarse = NearbyDecision.distanceClaim(30.0, 25_000.0, fromDeviceFix = true)
        assertTrue(tooCoarse is DistanceClaim.Uncertain)
        assertTrue((tooCoarse as DistanceClaim.Uncertain).reason.contains("25.0 km"))
    }

    @Test
    fun aProviderInsideTheFixesOwnErrorIsBoundedNotStatedAsAPointValue() {
        val claim = NearbyDecision.distanceClaim(0.05, 150.0, fromDeviceFix = true)
        assertEquals(DistanceClaim.Measured("< 150 m", approximate = true), claim)
        // A bound already reads as imprecise, so it is not additionally marked "~< 150 m".
        assertEquals("< 150 m", (claim as DistanceClaim.Measured).display)
        assertEquals(
            "~3 km",
            (NearbyDecision.distanceClaim(3.44, 900.0, fromDeviceFix = true)
                as DistanceClaim.Measured).display,
        )
        assertEquals(
            "3.4 km",
            (NearbyDecision.distanceClaim(3.44, null, fromDeviceFix = false)
                as DistanceClaim.Measured).display,
        )
    }

    @Test
    fun rowsCarryTheOriginsPrecisionAndKeepTheRawMeasurementForOrdering() {
        val clinic = provider("clinic")
        val coarse = NearbyDecision.nearby(
            found(clinic),
            NearbyOriginState.Available(
                GeoPoint(1.3521, 103.8198),
                OriginSource.CurrentLocation,
                accuracyMetres = 900.0,
            ),
            listOf(measured(clinic, 3.44)),
            "",
        )
        coarse as NearbyPresentation.ProvidersFound
        val row = coarse.rows.single()
        assertEquals(3.44, row.distanceKm, 0.0)
        assertEquals(DistanceClaim.Measured("3 km", approximate = true), row.distance)

        // A typed coordinate carries no measurement error, so it keeps ordinary precision.
        val chosen = NearbyDecision.nearby(
            found(clinic),
            origin,
            listOf(measured(clinic, 3.44)),
            "",
        )
        chosen as NearbyPresentation.ProvidersFound
        assertEquals(
            DistanceClaim.Measured("3.4 km", approximate = false),
            chosen.rows.single().distance,
        )
        assertNull(NearbyDecision.accuracyNote(null))
        assertEquals("±40 m", NearbyDecision.accuracyNote(38.0))
    }

    @Test
    fun distanceAndBearingFormattingDoNotOverclaimPrecision() {
        assertEquals("< 10 m", NearbyDecision.formatDistanceKm(0.003))
        assertEquals("820 m", NearbyDecision.formatDistanceKm(0.823))
        assertEquals("3.4 km", NearbyDecision.formatDistanceKm(3.36))
        assertEquals("27 km", NearbyDecision.formatDistanceKm(27.2))
        assertEquals(
            "< 25 ft",
            NearbyDecision.formatDistanceKm(0.003, NearbyDecision.UnitSystem.Imperial),
        )
        assertNull(NearbyDecision.formatDistanceKm(Double.NaN))

        assertEquals("N", NearbyDecision.formatBearing(0.0))
        assertEquals("NE", NearbyDecision.formatBearing(45.0))
        assertEquals("NW", NearbyDecision.formatBearing(-45.0))
        assertNull(NearbyDecision.formatBearing(null))
    }
}
