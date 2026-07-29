import XCTest

final class NearbyDecisionTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_785_312_000)

    private func provider(
        id: String,
        name: String,
        kind: String = "vet",
        geo: NearbyPoint? = NearbyPoint(lat: 1, lng: 1),
        active: Bool? = nil,
        contact: ProviderContact = ProviderContact(),
        bindingState: IssuerBindingState = .unavailable
    ) -> DirectoryProvider {
        DirectoryProvider(
            providerId: id,
            kind: kind,
            name: name,
            geo: geo,
            services: [],
            domain: nil,
            active: active,
            contact: contact,
            bindingState: bindingState
        )
    }

    private func found(_ providers: [DirectoryProvider]) -> ProviderDirectoryResult {
        .found(ProviderDirectorySnapshot(
            source: .central,
            providers: providers,
            observation: .live,
            blockNumber: nil,
            readAt: now,
            expiresAt: now.addingTimeInterval(3600)
        ))
    }

    private var empty: ProviderDirectoryResult {
        .empty(ProviderDirectorySnapshot(
            source: .central,
            providers: [],
            observation: .live,
            blockNumber: nil,
            readAt: now,
            expiresAt: now.addingTimeInterval(3600)
        ))
    }

    private var unavailable: ProviderDirectoryResult {
        .unavailable(ProviderDirectoryUnavailable(
            source: .central,
            reason: .sourceUnavailable,
            detail: "directory timed out",
            attemptedAt: now
        ))
    }

    private let origin = NearbyOrigin.chosen(NearbyPoint(lat: 0, lng: 0))

    func test_centralDirectoryEndpointIsTheExactFullSetRouteWithoutAQuery() {
        let endpoint = CentralProviderDirectory.endpoint(baseURL: "https://api.dogtag.io")
        XCTAssertEqual(endpoint, "https://api.dogtag.io/v1/businesses")
        XCTAssertNil(endpoint.flatMap { URLComponents(string: $0)?.query })

        XCTAssertEqual(
            CentralProviderDirectory.endpoint(baseURL: "https://example.test/api/"),
            "https://example.test/api/v1/businesses"
        )
        XCTAssertNil(CentralProviderDirectory.endpoint(baseURL: "https://api.dogtag.io?lat=1"))
    }

    /// The TTL belongs to the cache wrapper, not to the source: the source has no lifetime of its own
    /// and says so by declaring no deadline, which is what lets the wrapper time from the observation.
    ///
    /// Pinned so the value cannot drift back silently. It bounds ONLY how long an offline owner may
    /// be shown a remembered directory - the wrapper re-checks live on every read - so shortening it
    /// buys no freshness and only cuts that owner off sooner.
    func test_theStoredCopyUsesTheSharedSevenDayOfflineWindow() {
        XCTAssertEqual(CachedProviderDirectory.defaultTtl, 7 * 24 * 60 * 60)
    }

    /// A replay is labelled with a coarse age, and the rounding never makes it look fresher than it
    /// is. An age that cannot be derived says nothing rather than inventing a number.
    func test_theStoredAgeIsCoarseAndNeverUnderstatesStaleness() {
        let readAt = Date(timeIntervalSince1970: 1_000_000)
        func age(_ elapsed: TimeInterval) -> String? {
            NearbyDecision.formatStoredAge(readAt: readAt, now: readAt.addingTimeInterval(elapsed))
        }

        XCTAssertEqual(age(0), "less than a minute ago")
        XCTAssertEqual(age(59), "less than a minute ago")
        XCTAssertEqual(age(60), "1 minute ago")
        // Rounds outward: 61 seconds is stated as two minutes, never as one.
        XCTAssertEqual(age(61), "2 minutes ago")
        // The ceiling promotes rather than printing "60 minutes ago" or "24 hours ago".
        XCTAssertEqual(age(3_599), "1 hour ago")
        XCTAssertEqual(age(3_600), "1 hour ago")
        XCTAssertEqual(age(3_601), "2 hours ago")
        XCTAssertEqual(age(86_399), "1 day ago")
        XCTAssertEqual(age(86_400), "1 day ago")
        XCTAssertEqual(age(6 * 86_400), "6 days ago")

        // A snapshot read in the future is a backwards clock, not a fresh copy.
        XCTAssertNil(age(-1))
    }

    func test_centralParserPreservesContactOnlyAndRealZeroZeroLocations() {
        let parsed = CentralProviderDirectory.parseProviders([
            "businesses": [
                [
                    "businessId": "contact",
                    "type": "vet",
                    "name": "Contact Vet",
                    "services": ["general"],
                    "domain": "",
                    "contact": ["phone": "+65 6123 4567"],
                ] as [String: Any],
                [
                    "businessId": "zero",
                    "type": "groomer",
                    "name": "Zero Groomer",
                    "geo": ["lat": 0, "lng": 0],
                    "services": [],
                    "domain": "example.test",
                ] as [String: Any],
            ],
        ])

        XCTAssertEqual(parsed?.count, 2)
        XCTAssertNil(parsed?.first?.geo)
        // A directory row is not a chain read, so a blank domain column may not claim the on-chain
        // fact `noDomainClaimed`; it says only that this listing carries no domain.
        XCTAssertEqual(parsed?.first?.bindingState, .noDomainListed)
        XCTAssertFalse(IssuerBinding(state: .noDomainListed).line.contains("on-chain"))
        XCTAssertEqual(parsed?.first?.contact.phone, "+65 6123 4567")
        XCTAssertEqual(parsed?.last?.geo, NearbyPoint(lat: 0, lng: 0))
        XCTAssertEqual(parsed?.last?.bindingState, .unavailable)
        XCTAssertNil(parsed?.last?.active)
    }

    /// A provider reachable ONLY by website must not be reported as having published nothing.
    ///
    /// The server serves all five channels; this parser once read four, so `website` was dropped on
    /// the floor, `hasAny` read false, and the screen said "No contact details published." about a
    /// provider that had published exactly one. Mirrored by Android
    /// `aWebsiteOnlyProviderIsContactableRatherThanReportedAsPublishingNothing`.
    func test_aWebsiteOnlyProviderIsContactableRatherThanReportedAsPublishingNothing() {
        let parsed = CentralProviderDirectory.parseProviders([
            "businesses": [
                [
                    "businessId": "web-only",
                    "type": "groomer",
                    "name": "Web Only Grooming",
                    "services": [],
                    "domain": "",
                    "contact": ["website": "https://web-only.test"],
                ] as [String: Any],
            ],
        ])

        XCTAssertEqual(parsed?.first?.contact.website, "https://web-only.test")
        XCTAssertNil(parsed?.first?.contact.phone)
        XCTAssertEqual(parsed?.first?.contact.hasAny, true)
    }

    /// Consistent with the four sibling channels: a non-string channel is a malformed row.
    func test_aNonTextWebsiteIsMalformedLikeEveryOtherChannel() {
        XCTAssertNil(CentralProviderDirectory.parseProviders([
            "businesses": [
                [
                    "businessId": "one",
                    "type": "vet",
                    "name": "One",
                    "services": [],
                    "domain": "",
                    "contact": ["website": 42],
                ] as [String: Any],
            ],
        ]))
    }

    /// Mirrors the Kotlin adapter's duplicate-id refusal. `providerId` is the list identity both
    /// scopes render with, so a repeated one is a bad response, not two rows to draw on top of
    /// each other.
    func test_centralParserRefusesARepeatedProviderIdRatherThanRenderingBothRows() {
        let row: [String: Any] = [
            "businessId": "same",
            "type": "vet",
            "name": "One Vet",
            "services": [],
            "domain": "",
        ]
        var impostor = row
        impostor["name"] = "Impostor Vet"

        XCTAssertNil(CentralProviderDirectory.parseProviders(["businesses": [row, impostor]]))
        XCTAssertEqual(CentralProviderDirectory.parseProviders(["businesses": [row]])?.count, 1)
    }

    func test_centralParserRejectsPresentNullCoordinateObjectsAndMissingDomains() {
        var base: [String: Any] = [
            "businessId": "one",
            "type": "vet",
            "name": "One Vet",
            "services": [],
            "domain": "",
        ]

        var nullCoordinate = base
        nullCoordinate["geo"] = ["lat": NSNull(), "lng": NSNull()]
        XCTAssertNil(CentralProviderDirectory.parseProviders(
            ["businesses": [nullCoordinate]]
        ))

        base.removeValue(forKey: "domain")
        XCTAssertNil(CentralProviderDirectory.parseProviders(
            ["businesses": [base]]
        ))

        base["domain"] = NSNull()
        XCTAssertNil(CentralProviderDirectory.parseProviders(
            ["businesses": [base]]
        ))
    }

    private func presentation(
        _ directory: ProviderDirectoryResult?,
        location: NearbyLocationState? = nil,
        query: String = "",
        radiusKm: Double = NearbyDecision.defaultRadiusKm,
        distance: @escaping NearbyDecision.DistanceKm = { _, destination in abs(destination.lng) }
    ) -> NearbyDecision.Presentation {
        NearbyDecision.presentation(
            directory: directory,
            location: location ?? .ready(origin),
            query: query,
            unitSystem: .metric,
            radiusKm: radiusKm,
            distanceKm: distance
        )
    }

    private func rows(_ presentation: NearbyDecision.Presentation) -> [NearbyDecision.Row] {
        guard case .providersFound(let rows, _) = presentation else { return [] }
        return rows
    }

    func test_loadingDirectoryIsAnExplicitNearbyState() {
        XCTAssertEqual(presentation(nil), .loadingDirectory)
    }

    func test_foundProvidersAreSortedByMeasuredDistance() {
        let result = presentation(found([
            provider(id: "far", name: "Far Vet", geo: NearbyPoint(lat: 0, lng: 12)),
            provider(id: "near", name: "Near Vet", geo: NearbyPoint(lat: 0, lng: 1)),
            provider(id: "middle", name: "Middle Groomer", kind: "groomer", geo: NearbyPoint(lat: 0, lng: 5)),
        ]))

        XCTAssertEqual(
            rows(result).map(\.provider.providerId),
            ["near", "middle", "far"]
        )
        XCTAssertEqual(rows(result).map(\.bearingLabel), ["E", "E", "E"])
        XCTAssertTrue(rows(result).allSatisfy(\.allowsDirections))
    }

    func test_defaultBrowseExcludesProvidersOutsideFiftyKilometres() {
        let result = presentation(found([
            provider(id: "far", name: "Far Vet", geo: NearbyPoint(lat: 0, lng: 51)),
        ]))

        XCTAssertEqual(result, .noneWithinRange(radiusKm: 50, observation: .live))
    }

    func test_radiusBoundaryIsInclusiveAndInvalidRadiusFallsBackToDefault() {
        let edge = provider(id: "edge", name: "Edge", geo: NearbyPoint(lat: 0, lng: 50))
        XCTAssertEqual(rows(presentation(found([edge]))).map(\.provider.providerId), ["edge"])
        let outside = provider(id: "outside", name: "Outside", geo: NearbyPoint(lat: 0, lng: 51))
        XCTAssertEqual(
            presentation(found([outside]), radiusKm: -.infinity),
            .noneWithinRange(radiusKm: 50, observation: .live)
        )
    }

    func test_nameSearchRunsAcrossAllLocatedProvidersNotOnlyTheDefaultRange() {
        let result = presentation(
            found([
                provider(id: "near", name: "Local Vet", geo: NearbyPoint(lat: 0, lng: 2)),
                provider(id: "far", name: "Seaport Animal Clinic", geo: NearbyPoint(lat: 0, lng: 80)),
            ]),
            query: "animal"
        )

        XCTAssertEqual(rows(result).map(\.provider.providerId), ["far"])
        XCTAssertEqual(rows(result).first?.distanceKm, 80)
    }

    func test_nameSearchIsLocalCaseAndDiacriticInsensitive() {
        let result = presentation(
            found([provider(id: "one", name: "Clínica São Bento")]),
            query: "CLINICA sao"
        )
        XCTAssertEqual(rows(result).map(\.provider.providerId), ["one"])
    }

    func test_missingLocationIsNeverInNearbyAndZeroZeroIsNotASentinel() {
        let result = presentation(
            found([
                provider(id: "contact", name: "Contact Only", geo: nil),
                provider(id: "zero", name: "Published Zero", geo: NearbyPoint(lat: 0, lng: 0)),
            ]),
            location: .ready(.chosen(NearbyPoint(lat: 1, lng: 0))),
            distance: { origin, destination in abs(origin.lat - destination.lat) }
        )

        XCTAssertEqual(rows(result).map(\.provider.providerId), ["zero"])
        XCTAssertEqual(rows(result).first?.distanceKm, 1)
    }

    func test_missingLocalCandidateWinsBeforeAnyOriginPromptOrRefusal() {
        let contactOnly = provider(id: "contact", name: "Contact Only", geo: nil)
        XCTAssertEqual(
            presentation(found([contactOnly]), location: .notRequested),
            .noneWithinRange(radiusKm: 50, observation: .live)
        )

        let located = provider(id: "located", name: "North Star Vet")
        XCTAssertEqual(
            presentation(found([located]), location: .refused, query: "missing"),
            .noNameMatch(query: "missing", observation: .live)
        )
        XCTAssertEqual(
            presentation(found([located]), location: .notRequested, query: "north"),
            .awaitingOrigin
        )
    }

    func test_contactDirectoryIsUnrankedAndIncludesLocatedAndContactOnlyProviders() {
        let phone = ProviderContact(phone: "+65 6123 4567")
        let presentation = NearbyDecision.contactPresentation(
            directory: found([
                provider(id: "located", name: "Located Vet", geo: NearbyPoint(lat: 1, lng: 1)),
                provider(id: "contact", name: "Contact Only", geo: nil, contact: phone),
            ]),
            query: ""
        )

        guard case .providersFound(let providers, .live) = presentation else {
            return XCTFail("expected provider contacts")
        }
        XCTAssertEqual(providers.map(\.providerId), ["contact", "located"])
        XCTAssertNil(providers.first(where: { $0.providerId == "contact" })?.geo)
        XCTAssertEqual(
            providers.first(where: { $0.providerId == "contact" })?.contact.phone,
            "+65 6123 4567"
        )
    }

    func test_contactDirectoryLoadingUnavailableEmptyAndNoNameStayDistinct() {
        XCTAssertEqual(
            NearbyDecision.contactPresentation(directory: nil, query: ""),
            .loadingDirectory
        )
        XCTAssertEqual(
            NearbyDecision.contactPresentation(directory: unavailable, query: ""),
            .directoryUnavailable("directory timed out")
        )
        XCTAssertEqual(
            NearbyDecision.contactPresentation(directory: empty, query: ""),
            .directoryEmpty(.live)
        )
        XCTAssertEqual(
            NearbyDecision.contactPresentation(
                directory: found([provider(id: "one", name: "One Vet")]),
                query: "missing"
            ),
            .noNameMatch(query: "missing", observation: .live)
        )
    }

    func test_onlyVetsAndGroomersThatAreNotKnownInactiveAreEligible() {
        let result = presentation(found([
            provider(id: "vet", name: "Vet", active: nil),
            provider(id: "groomer", name: "Groomer", kind: "GROOMER", active: true),
            provider(id: "inactive", name: "Inactive", active: false),
            provider(id: "authority", name: "Authority", kind: "authority"),
        ]))

        XCTAssertEqual(Set(rows(result).map(\.provider.providerId)), Set(["vet", "groomer"]))
    }

    func test_directoryUnavailableNeverReadsAsNothingNearby() {
        let result = presentation(unavailable)
        XCTAssertEqual(result, .directoryUnavailable("directory timed out"))
        XCTAssertNotEqual(result, .noneWithinRange(radiusKm: 50, observation: .live))
    }

    func test_successfulEmptyDirectoryHasItsOwnState() {
        XCTAssertEqual(presentation(empty), .directoryEmpty(.live))
        XCTAssertNotEqual(presentation(empty), .noneWithinRange(radiusKm: 50, observation: .live))
        XCTAssertNotEqual(presentation(empty), .directoryUnavailable("directory timed out"))
    }

    func test_permissionRefusalDoesNotReadAsNothingNearby() {
        let denied = presentation(
            found([provider(id: "one", name: "One Vet")]),
            location: .refused
        )
        XCTAssertEqual(denied, .permissionRefused)
        XCTAssertNotEqual(denied, .noneWithinRange(radiusKm: 50, observation: .live))
    }

    func test_notRequestedLocatingAndLocationFailureStayDistinct() {
        let directory = found([provider(id: "one", name: "One Vet")])
        XCTAssertEqual(presentation(directory, location: .notRequested), .awaitingOrigin)
        XCTAssertEqual(presentation(directory, location: .locating), .locating)
        XCTAssertEqual(presentation(directory, location: .unavailable), .locationUnavailable)
        XCTAssertEqual(
            presentation(directory, location: .invalidChosenLocation),
            .invalidChosenLocation
        )
    }

    func test_noNameMatchIsNotTheSameClaimAsNoneWithinRange() {
        let directory = found([provider(id: "one", name: "One Vet")])
        XCTAssertEqual(
            presentation(directory, query: "missing"),
            .noNameMatch(query: "missing", observation: .live)
        )
        XCTAssertNotEqual(
            presentation(directory, query: "missing"),
            .noneWithinRange(radiusKm: 50, observation: .live)
        )
    }

    func test_equalDistancesKeepDirectoryOrder() {
        let result = presentation(
            found([
                provider(id: "first", name: "First"),
                provider(id: "second", name: "Second"),
            ]),
            distance: { _, _ in 3 }
        )
        XCTAssertEqual(rows(result).map(\.provider.providerId), ["first", "second"])
    }

    func test_provenanceStatePassesThroughWithoutAListingSpecificEnum() {
        let result = presentation(found([
            provider(id: "plain", name: "Plain Vet", bindingState: .noDomainClaimed),
        ]))
        guard let state = rows(result).first?.provider.bindingState else {
            return XCTFail("expected provider row")
        }
        XCTAssertEqual(state, .noDomainClaimed)
        XCTAssertEqual(IssuerBinding(state: state).tone, .neutral)
    }

    /// Mirrored by Android `aCoarseFixNeverRendersAFinerNumberThanItSupports`.
    ///
    /// Coarse collection is only honest if the display admits how coarse it is: the same 3.44 km
    /// measurement must read differently from an exact chosen coordinate, a ten-metre fix and a
    /// hundred-metre fix.
    func test_aCoarseFixNeverRendersAFinerNumberThanItSupports() {
        XCTAssertEqual(
            NearbyDecision.distanceClaim(
                3.44, accuracyMetres: nil, fromDeviceFix: false, unitSystem: .metric
            ),
            .measured(label: "3.4 km", approximate: false)
        )
        XCTAssertEqual(
            NearbyDecision.distanceClaim(
                3.44, accuracyMetres: 8, fromDeviceFix: true, unitSystem: .metric
            ),
            .measured(label: "3.4 km", approximate: true)
        )
        XCTAssertEqual(
            NearbyDecision.distanceClaim(
                3.44, accuracyMetres: 900, fromDeviceFix: true, unitSystem: .metric
            ),
            .measured(label: "3 km", approximate: true)
        )
        XCTAssertEqual(
            NearbyDecision.distanceClaim(
                0.823, accuracyMetres: 90, fromDeviceFix: true, unitSystem: .metric
            ),
            .measured(label: "0.8 km", approximate: true)
        )
    }

    func test_aFixTooCoarseOrTooBrokenToPlaceAProviderStatesUncertaintyInsteadOfANumber() {
        for accuracy in [nil, Double.nan, -1] as [Double?] {
            let claim = NearbyDecision.distanceClaim(
                3.44, accuracyMetres: accuracy, fromDeviceFix: true, unitSystem: .metric
            )
            guard case .uncertain = claim else {
                return XCTFail("expected uncertain for accuracy \(String(describing: accuracy))")
            }
        }
        let tooCoarse = NearbyDecision.distanceClaim(
            30, accuracyMetres: 25_000, fromDeviceFix: true, unitSystem: .metric
        )
        guard case .uncertain(let reason) = tooCoarse else {
            return XCTFail("expected uncertain beyond the coarsest usable fix")
        }
        XCTAssertTrue(reason.contains("25.0 km"), reason)
    }

    func test_aProviderInsideTheFixesOwnErrorIsBoundedNotStatedAsAPointValue() {
        let bounded = NearbyDecision.distanceClaim(
            0.05, accuracyMetres: 150, fromDeviceFix: true, unitSystem: .metric
        )
        XCTAssertEqual(bounded, .measured(label: "< 500 m", approximate: true))
        // A bound already reads as imprecise, so it is not additionally marked "~< 500 m".
        XCTAssertEqual(bounded.display, "< 500 m")
        XCTAssertEqual(
            NearbyDecision.distanceClaim(
                3.44, accuracyMetres: 900, fromDeviceFix: true, unitSystem: .metric
            ).display,
            "~3 km"
        )
        XCTAssertEqual(
            NearbyDecision.distanceClaim(
                3.44, accuracyMetres: nil, fromDeviceFix: false, unitSystem: .metric
            ).display,
            "3.4 km"
        )
        XCTAssertNil(DistanceClaim.uncertain(reason: "nope").display)
    }

    /// Mirrored by Android `noPositiveDistanceCollapsesToAConfidentZero`.
    ///
    /// Rounding a distance to a step coarser than twice the distance itself yields zero, which the
    /// bands then print as a confident "0 km". Every rung has such a window, and for a coarse-only
    /// grant the 1 km rung's window covers most of the browse radius.
    func test_noPositiveDistanceCollapsesToAConfidentZero() {
        let windows: [(Double, Double, String)] = [
            (1_200, 3.0, "< 5.0 km"),
            (150, 0.3, "< 500 m"),
            (30, 0.04, "< 50 m"),
            (3, 0.004, "< 10 m"),
        ]
        for (accuracy, km, expected) in windows {
            XCTAssertEqual(
                NearbyDecision.distanceClaim(
                    km, accuracyMetres: accuracy, fromDeviceFix: true, unitSystem: .metric
                ),
                .measured(label: expected, approximate: true),
                "\(accuracy)/\(km)"
            )
        }

        let imperialWindows: [(Double, Double, String)] = [
            (1_200, 3.0, "2 mi"),
            (150, 0.3, "0.2 mi"),
            (8, 0.05, "< 275 ft"),
        ]
        for (accuracy, km, expected) in imperialWindows {
            XCTAssertEqual(
                NearbyDecision.distanceClaim(
                    km, accuracyMetres: accuracy, fromDeviceFix: true, unitSystem: .imperial
                ),
                .measured(label: expected, approximate: true),
                "\(accuracy)/\(km)"
            )
        }
    }

    /// Just above the bound is the region the collapse used to occupy, on every rung of both units.
    func test_aDistanceJustAboveTheBoundStillStatesANonZeroNumber() {
        let cases: [(NearbyUnitSystem, [(Double, Double)])] = [
            (.metric, [(0.009, 8), (0.06, 30), (0.6, 150), (6.0, 1_200)]),
            (.imperial, [(0.005, 3), (0.09, 80), (0.9, 300), (9.0, 1_200)]),
        ]
        for (unitSystem, pairs) in cases {
            for (km, accuracy) in pairs {
                let claim = NearbyDecision.distanceClaim(
                    km, accuracyMetres: accuracy, fromDeviceFix: true, unitSystem: unitSystem
                )
                guard case .measured(let label, _) = claim else {
                    return XCTFail("\(unitSystem) \(accuracy)/\(km) was not measured")
                }
                XCTAssertFalse(label.hasPrefix("0 "), "\(accuracy)/\(km) -> \(label)")
                XCTAssertFalse(label.hasPrefix("0.0 "), "\(accuracy)/\(km) -> \(label)")
            }
        }
    }

    /// A fix beyond the coarsest usable step says so. Because that ceiling is per unit, an imperial
    /// fix between the two ceilings must still place the provider rather than report a failed read.
    func test_theCoarsestUsableFixIsPerUnitAndItsRefusalNamesTheAccuracy() {
        guard case .uncertain(let metric) = NearbyDecision.distanceClaim(
            30, accuracyMetres: 10_001, fromDeviceFix: true, unitSystem: .metric
        ) else { return XCTFail("expected uncertain beyond the metric ceiling") }
        XCTAssertTrue(metric.contains("too coarse"), metric)

        XCTAssertEqual(
            NearbyDecision.distanceClaim(
                30, accuracyMetres: 12_000, fromDeviceFix: true, unitSystem: .imperial
            ),
            .measured(label: "20 mi", approximate: true)
        )

        guard case .uncertain(let imperial) = NearbyDecision.distanceClaim(
            30, accuracyMetres: 17_000, fromDeviceFix: true, unitSystem: .imperial
        ) else { return XCTFail("expected uncertain beyond the imperial ceiling") }
        XCTAssertTrue(imperial.contains("too coarse"), imperial)
    }

    /// Mirrored by Android `aBoundLabelIsNeverTighterThanTheDistanceItAdmitted`.
    ///
    /// The gate admits everything up to the bound, so a label rounded to the NEAREST display step
    /// can name a distance the gate already let past: at 94 m accuracy a provider measured at 92 m
    /// used to render "< 90 m". Every one of these pairs rounds down under nearest.
    func test_aBoundLabelIsNeverTighterThanTheDistanceItAdmitted() {
        let metric: [(Double, Double, String, String)] = [
            (94, 0.092, "< 100 m", "< 90 m"),
            (944, 0.942, "< 950 m", "< 940 m"),
            (5_040, 5.03, "< 5.1 km", "< 5.0 km"),
        ]
        let imperial: [(Double, Double, String, String)] = [
            (100, 0.0995, "< 350 ft", "< 325 ft"),
            (850, 0.84, "< 0.6 mi", "< 0.5 mi"),
        ]

        for (unitSystem, cases) in [(NearbyUnitSystem.metric, metric), (.imperial, imperial)] {
            for (accuracy, km, expected, understated) in cases {
                XCTAssertEqual(
                    NearbyDecision.distanceClaim(
                        km, accuracyMetres: accuracy, fromDeviceFix: true, unitSystem: unitSystem
                    ),
                    .measured(label: expected, approximate: true),
                    "\(accuracy)/\(km)"
                )
                XCTAssertGreaterThanOrEqual(boundMetres(expected), km * 1000, "\(accuracy)/\(km)")
                XCTAssertLessThan(boundMetres(understated), km * 1000, "\(accuracy)/\(km)")
            }
        }
    }

    /// The general property behind the cases above, over every rung of both ladders: a `< bound` may
    /// never name less than the accuracy it was derived from. A value that already sits on a step
    /// must also not be bumped outward, which is what the imperial rungs - none of them exactly
    /// representable as a double - would otherwise do.
    func test_everyBoundLabelCoversTheAccuracyItWasDerivedFrom() {
        let accuracies: [Double] = [
            3, 9.4, 40, 94, 100, 160.9344, 400, 850, 944, 1_609.344, 5_040, 9_400,
        ]
        for unitSystem in [NearbyUnitSystem.metric, .imperial] {
            for accuracy in accuracies {
                let claim = NearbyDecision.distanceClaim(
                    accuracy / 1000,
                    accuracyMetres: accuracy,
                    fromDeviceFix: true,
                    unitSystem: unitSystem
                )
                guard case .measured(let label, _) = claim, label.hasPrefix("< ") else {
                    return XCTFail("\(unitSystem) \(accuracy) produced no bound")
                }
                XCTAssertGreaterThanOrEqual(
                    boundMetres(label), accuracy - 1e-6, "\(unitSystem) \(accuracy) -> \(label)"
                )
            }
        }
        XCTAssertEqual(
            NearbyDecision.distanceClaim(
                1.5, accuracyMetres: 1_609.344, fromDeviceFix: true, unitSystem: .imperial
            ),
            .measured(label: "< 1.0 mi", approximate: true)
        )
    }

    private func boundMetres(_ label: String) -> Double {
        let parts = label.replacingOccurrences(of: "< ", with: "").split(separator: " ")
        guard let value = Double(parts[0]) else { return .nan }
        switch parts[1] {
        case "m": return value
        case "km": return value * 1_000
        case "ft": return value * 0.3048
        case "mi": return value * 1_609.344
        default: return .nan
        }
    }

    /// Mirrored by Android `contactOrderIsTheSameFoldedKeyOnBothPlatforms`.
    func test_contactOrderIsTheSameFoldedKeyOnBothPlatforms() {
        let presentation = NearbyDecision.contactPresentation(
            directory: found([
                provider(id: "bxx", name: "Bxx Grooming", geo: nil),
                provider(id: "z-same", name: "Ávila veterinary", geo: nil),
                provider(id: "avila", name: "Ávila Veterinary", geo: nil),
            ]),
            query: ""
        )
        guard case .providersFound(let providers, _) = presentation else {
            return XCTFail("expected provider contacts")
        }
        XCTAssertEqual(providers.map(\.providerId), ["avila", "z-same", "bxx"])
    }

    func test_rowsCarryTheOriginsPrecisionAndKeepTheRawMeasurementForOrdering() {
        let directory = found([provider(id: "one", name: "One Vet", geo: NearbyPoint(lat: 0, lng: 3.44))])
        let coarse = presentation(
            directory,
            location: .ready(NearbyOrigin(
                point: NearbyPoint(lat: 0, lng: 0),
                source: .currentLocation,
                accuracyMetres: 900
            ))
        )
        guard let row = rows(coarse).first else { return XCTFail("expected a row") }
        XCTAssertEqual(row.distanceKm, 3.44)
        XCTAssertEqual(row.distance, .measured(label: "3 km", approximate: true))

        // A typed coordinate carries no measurement error, so it keeps ordinary precision.
        let chosen = presentation(directory)
        XCTAssertEqual(
            rows(chosen).first?.distance,
            .measured(label: "3.4 km", approximate: false)
        )
        XCTAssertNil(NearbyDecision.accuracyNote(nil, unitSystem: .metric))
        XCTAssertEqual(NearbyDecision.accuracyNote(38, unitSystem: .metric), "±40 m")
    }

    func test_distanceFormattingMatchesTheSharedDisplayBands() {
        XCTAssertEqual(NearbyDecision.formatDistanceKm(0, unitSystem: .metric), "< 10 m")
        XCTAssertEqual(NearbyDecision.formatDistanceKm(0.823, unitSystem: .metric), "820 m")
        XCTAssertEqual(NearbyDecision.formatDistanceKm(3.44, unitSystem: .metric), "3.4 km")
        XCTAssertEqual(NearbyDecision.formatDistanceKm(27.4, unitSystem: .metric), "27 km")
        XCTAssertEqual(NearbyDecision.formatDistanceKm(0, unitSystem: .imperial), "< 25 ft")
        XCTAssertEqual(NearbyDecision.formatDistanceKm(1.609344, unitSystem: .imperial), "1.0 mi")
        XCTAssertEqual(NearbyDecision.formatDistanceKm(50, unitSystem: .imperial), "31 mi")
        XCTAssertNil(NearbyDecision.formatDistanceKm(nil, unitSystem: .metric))
    }

    func test_unitSystemAcceptsARegionCodeOrLocaleIdentifier() {
        XCTAssertEqual(NearbyUnitSystem.forRegion("US"), .imperial)
        XCTAssertEqual(NearbyUnitSystem.forRegion("en-US"), .imperial)
        XCTAssertEqual(NearbyUnitSystem.forRegion("en_SG"), .metric)
        XCTAssertEqual(NearbyUnitSystem.forRegion(nil), .metric)
    }

    func test_chosenOriginParserAcceptsARealZeroZeroRatherThanTreatingItAsBlank() {
        XCTAssertEqual(
            NearbyDecision.parseChosenOrigin(lat: " 0 ", lng: "0.0"),
            NearbyPoint(lat: 0, lng: 0)
        )
    }

    func test_chosenOriginParserRejectsMissingNonfiniteAndOutOfRangeCoordinates() {
        for pair in [
            ("", "103.8"),
            ("north", "103.8"),
            ("nan", "103.8"),
            ("91", "0"),
            ("0", "181"),
        ] {
            XCTAssertNil(NearbyDecision.parseChosenOrigin(lat: pair.0, lng: pair.1), "\(pair)")
        }
    }

    func test_constructedInvalidReadyOriginIsLocationUnavailable() {
        let directory = found([provider(id: "one", name: "One Vet")])
        XCTAssertEqual(
            presentation(
                directory,
                location: .ready(.chosen(NearbyPoint(lat: 91, lng: 0)))
            ),
            .locationUnavailable
        )
    }
}
