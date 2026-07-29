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

    private let origin = NearbyPoint(lat: 0, lng: 0)

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

    func test_centralDirectoryUsesTheSharedFifteenMinuteHardTtl() {
        XCTAssertEqual(CentralProviderDirectory.defaultTtl, 15 * 60)
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
        XCTAssertEqual(parsed?.first?.bindingState, .noDomainClaimed)
        XCTAssertEqual(parsed?.first?.contact.phone, "+65 6123 4567")
        XCTAssertEqual(parsed?.last?.geo, NearbyPoint(lat: 0, lng: 0))
        XCTAssertEqual(parsed?.last?.bindingState, .unavailable)
        XCTAssertNil(parsed?.last?.active)
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
            location: .ready(NearbyPoint(lat: 1, lng: 0)),
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
                location: .ready(NearbyPoint(lat: 91, lng: 0))
            ),
            .locationUnavailable
        )
    }
}
