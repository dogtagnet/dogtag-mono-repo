import XCTest

final class NearbyDecisionTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_785_312_000)

    private func provider(
        id: String,
        name: String,
        kind: String = "vet",
        geo: NearbyPoint? = NearbyPoint(lat: 1, lng: 1),
        distanceKm: Double? = nil,
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
            bindingState: bindingState,
            distanceKm: distanceKm
        )
    }

    private func found(_ providers: [DirectoryProvider]) -> ProviderDirectoryResult {
        .found(ProviderDirectorySnapshot(
            source: .central,
            providers: providers,
            observation: .live,
            blockNumber: nil,
            readAt: now,
            expiresAt: nil,
            page: ProviderDirectoryPage(
                total: providers.count,
                limit: OwnerProviderDirectoryRequest.pageSize,
                offset: 0,
                hasMore: false
            )
        ))
    }

    private func pageResult(
        _ providers: [DirectoryProvider],
        total: Int,
        limit: Int,
        offset: Int,
        hasMore: Bool
    ) -> ProviderDirectoryResult {
        let snapshot = ProviderDirectorySnapshot(
            source: .central,
            providers: providers,
            observation: .live,
            blockNumber: nil,
            readAt: now,
            expiresAt: nil,
            page: ProviderDirectoryPage(
                total: total,
                limit: limit,
                offset: offset,
                hasMore: hasMore
            )
        )
        return providers.isEmpty ? .empty(snapshot) : .found(snapshot)
    }

    private var ready: NearbyLocationState {
        .ready(NearbyOrigin(
            point: NearbyPoint(lat: 1.293, lng: 103.852),
            accuracyMetres: 100
        ))
    }

    private func rows(_ presentation: NearbyDecision.Presentation) -> [NearbyDecision.Row] {
        guard case .providersFound(let rows, _) = presentation else { return [] }
        return rows
    }

    func test_currentPositionIsCoarsenedToThreeDecimalsOnDevice() {
        XCTAssertEqual(
            NearbyPoint(lat: 1.29349, lng: 103.85251).coarsenedForProviderSearch(),
            NearbyPoint(lat: 1.293, lng: 103.853)
        )
        XCTAssertEqual(
            NearbyPoint(lat: -33.86551, lng: -0.0001).coarsenedForProviderSearch(),
            NearbyPoint(lat: -33.866, lng: 0)
        )
        XCTAssertEqual(
            NearbyPoint(lat: 89.9996, lng: 179.9996).coarsenedForProviderSearch(),
            NearbyPoint(lat: 90, lng: 180)
        )
        XCTAssertNil(NearbyPoint(lat: 91, lng: 0).coarsenedForProviderSearch())
        XCTAssertNil(NearbyPoint(lat: .nan, lng: 0).coarsenedForProviderSearch())
    }

    func test_nearestRequestHasExactOwnerKindsPagingNameAndApproximateBody() throws {
        let request = try XCTUnwrap(OwnerProviderDirectoryRequest.nearest(
            location: NearbyPoint(lat: 1.29349, lng: 103.85251),
            accuracyMetres: 73,
            name: " Clínica & Sons ",
            offset: 50
        ))
        let wire = try XCTUnwrap(CentralProviderDirectory.wireRequest(
            baseURL: "https://api.dogtag.io",
            request: request
        ))

        XCTAssertEqual(
            wire.url,
            "https://api.dogtag.io/v1/businesses/nearest?kind=vet&kind=groomer&limit=25&offset=50&name=Cl%C3%ADnica%20%26%20Sons"
        )
        let bodyData = try XCTUnwrap(wire.body?.data(using: .utf8))
        let body = try XCTUnwrap(
            JSONSerialization.jsonObject(with: bodyData) as? [String: NSNumber]
        )
        XCTAssertEqual(body["approximateLat"]?.doubleValue, 1.293)
        XCTAssertEqual(body["approximateLng"]?.doubleValue, 103.853)
        XCTAssertFalse(try XCTUnwrap(wire.body).contains("1.29349"))
        XCTAssertFalse(try XCTUnwrap(wire.body).contains("103.85251"))
    }

    func test_contactNameRequestHasNoPositionBodyAndStillRestrictsOwnerKinds() throws {
        let request = try XCTUnwrap(OwnerProviderDirectoryRequest.contacts(
            name: "North Star",
            offset: 25
        ))
        let wire = try XCTUnwrap(CentralProviderDirectory.wireRequest(
            baseURL: "https://example.test/api/",
            request: request
        ))

        XCTAssertEqual(
            wire.url,
            "https://example.test/api/v1/businesses?kind=vet&kind=groomer&limit=25&offset=25&name=North%20Star"
        )
        XCTAssertNil(wire.body)
        XCTAssertFalse(wire.url.contains("Lat"))
        XCTAssertFalse(wire.url.contains("Lng"))
    }

    func test_requestSurfaceHasNoChosenLocationMapRadiusOrPlaceHint() {
        XCTAssertNil(OwnerProviderDirectoryRequest.contacts(name: nil, offset: -1))
        XCTAssertNil(OwnerProviderDirectoryRequest.nearest(
            location: NearbyPoint(lat: 91, lng: 0),
            accuracyMetres: 50,
            name: nil,
            offset: 0
        ))
        XCTAssertNil(CentralProviderDirectory.wireRequest(
            baseURL: "https://api.dogtag.io?place=London",
            request: OwnerProviderDirectoryRequest.contacts(name: nil, offset: 0)!
        ))
    }

    /// Mirrors Android `NearbyDecisionTest.disclosurePlainlyStatesSendPurposeAndRetentionAtTheGrantAction`.
    /// The nearest search reversed a privacy property the earlier slices were built for, so the sentence
    /// beside the permission action is part of the contract rather than decoration: it must name what
    /// leaves the device, why, and that it is not retained. Pinned byte-for-byte against Android because
    /// two platforms telling the owner different things about the same transfer is the failure mode.
    func test_disclosurePlainlyStatesSendPurposeAndRetentionAtTheGrantAction() {
        XCTAssertEqual(
            "Your approximate location is sent to DogTag to find nearby vets and groomers. "
                + "It is not stored.",
            NearbyDecision.locationDisclosure
        )
    }

    func test_nearestParserRequiresFiniteNonnegativeDistanceAndPreservesServerOrder() throws {
        let object: [String: Any] = [
            "businesses": [
                businessRow(id: "z-name", name: "Zulu", distanceKm: 0.8),
                businessRow(id: "a-name", name: "Alpha", kind: "groomer", distanceKm: 2.4),
            ],
            "total": 2,
            "limit": 25,
            "offset": 0,
            "hasMore": false,
        ]
        let parsed = try XCTUnwrap(CentralProviderDirectory.parsePage(
            object,
            requiresDistance: true,
            expectedOffset: 0
        ))

        XCTAssertEqual(parsed.providers.map(\.providerId), ["z-name", "a-name"])
        XCTAssertEqual(parsed.providers.map(\.distanceKm), [0.8, 2.4])
        XCTAssertEqual(
            parsed.page,
            ProviderDirectoryPage(total: 2, limit: 25, offset: 0, hasMore: false)
        )

        var missing = object
        missing["businesses"] = [businessRow(id: "missing", name: "Missing")]
        missing["total"] = 1
        XCTAssertNil(CentralProviderDirectory.parsePage(missing, requiresDistance: true))

        var negative = object
        negative["businesses"] = [
            businessRow(id: "negative", name: "Negative", distanceKm: -0.1),
        ]
        negative["total"] = 1
        XCTAssertNil(CentralProviderDirectory.parsePage(negative, requiresDistance: true))

        var outOfOrder = object
        outOfOrder["businesses"] = [
            businessRow(id: "far", name: "Far", distanceKm: 5),
            businessRow(id: "near", name: "Near", distanceKm: 1),
        ]
        XCTAssertNil(CentralProviderDirectory.parsePage(outOfOrder, requiresDistance: true))
    }

    func test_pageMetadataMustDescribeTheReturnedPage() {
        let row = businessRow(id: "one", name: "One", distanceKm: 1)
        let valid: [String: Any] = [
            "businesses": [row],
            "total": 2,
            "limit": 1,
            "offset": 0,
            "hasMore": true,
        ]
        XCTAssertNotNil(CentralProviderDirectory.parsePage(valid, requiresDistance: true))

        for mutation: (String, Any) in [
            ("total", -1),
            ("limit", 0),
            ("offset", -1),
            ("hasMore", false),
        ] {
            var bad = valid
            bad[mutation.0] = mutation.1
            XCTAssertNil(
                CentralProviderDirectory.parsePage(bad, requiresDistance: true),
                "\(mutation.0)"
            )
        }
        XCTAssertNil(CentralProviderDirectory.parsePage(
            valid,
            requiresDistance: true,
            expectedOffset: 25
        ))
    }

    func test_pageParserAcceptsADeepEmptyPageButNotRowsBeyondTotal() throws {
        let deepEmpty: [String: Any] = [
            "businesses": [],
            "total": 2,
            "limit": 25,
            "offset": 100,
            "hasMore": false,
        ]
        let parsed = try XCTUnwrap(CentralProviderDirectory.parsePage(
            deepEmpty,
            requiresDistance: false,
            expectedOffset: 100
        ))
        XCTAssertTrue(parsed.providers.isEmpty)
        XCTAssertEqual(
            parsed.page,
            ProviderDirectoryPage(total: 2, limit: 25, offset: 100, hasMore: false)
        )

        var impossibleRows = deepEmpty
        impossibleRows["businesses"] = [businessRow(id: "late", name: "Late")]
        XCTAssertNil(CentralProviderDirectory.parsePage(
            impossibleRows,
            requiresDistance: false,
            expectedOffset: 100
        ))
    }

    func test_pageMergeRequiresContiguousStableMetadataAndDistanceOrder() {
        let current = pageResult(
            [provider(id: "first", name: "First", distanceKm: 1)],
            total: 3,
            limit: 1,
            offset: 0,
            hasMore: true
        )
        let validNext = pageResult(
            [provider(id: "second", name: "Second", distanceKm: 2)],
            total: 3,
            limit: 1,
            offset: 1,
            hasMore: true
        )
        let merged = ProviderDirectoryPaging.merge(
            current: current,
            incoming: validNext,
            reset: false,
            requiresDistance: true,
            attemptedAt: now
        )
        guard case .found(let snapshot) = merged else {
            return XCTFail("expected a valid contiguous page to merge")
        }
        XCTAssertEqual(snapshot.providers.map(\.providerId), ["first", "second"])

        let mismatches = [
            pageResult(
                [provider(id: "offset", name: "Offset", distanceKm: 2)],
                total: 3,
                limit: 1,
                offset: 0,
                hasMore: true
            ),
            pageResult(
                [provider(id: "limit", name: "Limit", distanceKm: 2)],
                total: 3,
                limit: 2,
                offset: 1,
                hasMore: true
            ),
            pageResult(
                [provider(id: "total", name: "Total", distanceKm: 2)],
                total: 4,
                limit: 1,
                offset: 1,
                hasMore: true
            ),
            pageResult(
                [provider(id: "nearer", name: "Nearer", distanceKm: 0.5)],
                total: 3,
                limit: 1,
                offset: 1,
                hasMore: true
            ),
            pageResult(
                [provider(id: "missing-distance", name: "Missing")],
                total: 3,
                limit: 1,
                offset: 1,
                hasMore: true
            ),
        ]
        for mismatch in mismatches {
            let result = ProviderDirectoryPaging.merge(
                current: current,
                incoming: mismatch,
                reset: false,
                requiresDistance: true,
                attemptedAt: now
            )
            guard case .unavailable(let failure) = result else {
                XCTFail("expected page mismatch to be rejected")
                continue
            }
            XCTAssertEqual(failure.reason, .inconsistentSource)
        }
    }

    func test_contactParserPreservesNullLocationContactsAndNoDistance() throws {
        let parsed = try XCTUnwrap(CentralProviderDirectory.parsePage([
            "businesses": [
                [
                    "businessId": "contact",
                    "type": "vet",
                    "name": "Contact Vet",
                    "services": ["general"],
                    "domain": "",
                    "geo": NSNull(),
                    "contact": [
                        "phone": "+65 6123 4567",
                        "website": "https://contact.test",
                    ],
                ] as [String: Any],
            ],
            "total": 1,
            "limit": 25,
            "offset": 0,
            "hasMore": false,
        ], requiresDistance: false))

        XCTAssertNil(parsed.providers[0].geo)
        XCTAssertNil(parsed.providers[0].distanceKm)
        XCTAssertEqual(parsed.providers[0].contact.phone, "+65 6123 4567")
        XCTAssertEqual(parsed.providers[0].contact.website, "https://contact.test")
        XCTAssertTrue(parsed.providers[0].contact.hasAny)
        XCTAssertEqual(parsed.providers[0].bindingState, .noDomainListed)
    }

    func test_parserRejectsRepeatedIdsAndMalformedCoordinates() {
        let row = businessRow(id: "same", name: "One", distanceKm: 1)
        XCTAssertNil(CentralProviderDirectory.parseProviders(
            ["businesses": [row, row]],
            requiresDistance: true
        ))

        var malformed = row
        malformed["geo"] = ["lat": NSNull(), "lng": 1]
        XCTAssertNil(CentralProviderDirectory.parseProviders(
            ["businesses": [malformed]],
            requiresDistance: true
        ))
    }

    func test_nearbyUsesServerDistanceAndOrderWithoutRecomputing() {
        let presentation = NearbyDecision.presentation(
            directory: found([
                provider(id: "first", name: "Zulu", distanceKm: 0.8),
                provider(id: "second", name: "Alpha", kind: "groomer", distanceKm: 2.4),
            ]),
            location: ready,
            query: "",
            unitSystem: .metric
        )

        XCTAssertEqual(rows(presentation).map(\.provider.providerId), ["first", "second"])
        XCTAssertEqual(rows(presentation).map(\.distanceKm), [0.8, 2.4])
    }

    func test_ownerAppNeverSurfacesAdminOrGovernmentEvenIfServiceMisbehaves() {
        let presentation = NearbyDecision.presentation(
            directory: found([
                provider(id: "vet", name: "Vet", distanceKm: 1),
                provider(id: "groomer", name: "Groomer", kind: "GROOMER", distanceKm: 2),
                provider(id: "admin", name: "Admin", kind: "admin", distanceKm: 3),
                provider(id: "gov", name: "Government", kind: "government", distanceKm: 4),
            ]),
            location: ready,
            query: "",
            unitSystem: .metric
        )
        XCTAssertEqual(
            rows(presentation).map(\.provider.providerId),
            ["vet", "groomer"]
        )
    }

    func test_locationAndDirectoryFailuresRemainDistinct() {
        let unavailable = ProviderDirectoryResult.unavailable(ProviderDirectoryUnavailable(
            source: .central,
            reason: .sourceUnavailable,
            detail: "directory timed out",
            attemptedAt: now
        ))

        XCTAssertEqual(
            NearbyDecision.presentation(
                directory: nil,
                location: .notRequested,
                query: "",
                unitSystem: .metric
            ),
            .awaitingOrigin
        )
        XCTAssertEqual(
            NearbyDecision.presentation(
                directory: nil,
                location: .locating,
                query: "",
                unitSystem: .metric
            ),
            .locating
        )
        XCTAssertEqual(
            NearbyDecision.presentation(
                directory: unavailable,
                location: ready,
                query: "",
                unitSystem: .metric
            ),
            .directoryUnavailable("directory timed out")
        )
    }

    func test_serverDistanceDisplayAdmitsCurrentFixUncertainty() {
        let presentation = NearbyDecision.presentation(
            directory: found([
                provider(id: "one", name: "One", distanceKm: 3.44),
            ]),
            location: .ready(NearbyOrigin(
                point: NearbyPoint(lat: 1.293, lng: 103.852),
                accuracyMetres: 900
            )),
            query: "",
            unitSystem: .metric
        )

        XCTAssertEqual(rows(presentation)[0].distanceKm, 3.44)
        XCTAssertEqual(
            rows(presentation)[0].distance,
            .measured(label: "3 km", approximate: true)
        )
        XCTAssertEqual(rows(presentation)[0].distance.display, "~3 km")
    }

    func test_distanceDisplayIsNeverFinerThanTheCoordinateSentToTheServer() {
        let presentation = NearbyDecision.presentation(
            directory: found([
                provider(id: "one", name: "One", distanceKm: 0.823),
            ]),
            location: .ready(NearbyOrigin(
                point: NearbyPoint(lat: 1.293, lng: 103.852),
                accuracyMetres: 5
            )),
            query: "",
            unitSystem: .metric
        )

        XCTAssertEqual(rows(presentation)[0].distanceKm, 0.823)
        XCTAssertEqual(
            rows(presentation)[0].distance,
            .measured(label: "0.8 km", approximate: true)
        )
        XCTAssertEqual(rows(presentation)[0].distance.display, "~0.8 km")
    }

    func test_distanceFormattingRetainsMetricAndImperialBands() {
        XCTAssertEqual(NearbyDecision.formatDistanceKm(0, unitSystem: .metric), "< 10 m")
        XCTAssertEqual(NearbyDecision.formatDistanceKm(0.823, unitSystem: .metric), "820 m")
        XCTAssertEqual(NearbyDecision.formatDistanceKm(3.44, unitSystem: .metric), "3.4 km")
        XCTAssertEqual(
            NearbyDecision.formatDistanceKm(1.609344, unitSystem: .imperial),
            "1.0 mi"
        )
        XCTAssertNil(NearbyDecision.formatDistanceKm(nil, unitSystem: .metric))
        XCTAssertEqual(NearbyDecision.accuracyNote(38, unitSystem: .metric), "±40 m")
    }

    func test_noPositiveDistanceCollapsesToAConfidentZero() {
        let cases: [(Double, Double, String)] = [
            (1_200, 3.0, "< 5.0 km"),
            (150, 0.3, "< 500 m"),
            (30, 0.04, "< 50 m"),
            (3, 0.004, "< 10 m"),
        ]
        for (accuracy, km, expected) in cases {
            XCTAssertEqual(
                NearbyDecision.distanceClaim(
                    km,
                    accuracyMetres: accuracy,
                    fromDeviceFix: true,
                    unitSystem: .metric
                ),
                .measured(label: expected, approximate: true)
            )
        }
    }

    func test_contactPresentationPreservesServerOrderAndOwnerKinds() {
        let presentation = NearbyDecision.contactPresentation(
            directory: found([
                provider(id: "z", name: "Zulu", geo: nil),
                provider(id: "a", name: "Alpha", kind: "groomer", geo: nil),
                provider(id: "gov", name: "Government", kind: "government", geo: nil),
            ]),
            query: ""
        )
        guard case .providersFound(let providers, .live) = presentation else {
            return XCTFail("expected provider contacts")
        }
        XCTAssertEqual(providers.map(\.providerId), ["z", "a"])
    }

    private func businessRow(
        id: String,
        name: String,
        kind: String = "vet",
        distanceKm: Double? = nil
    ) -> [String: Any] {
        var row: [String: Any] = [
            "businessId": id,
            "type": kind,
            "name": name,
            "geo": ["lat": 1.3, "lng": 103.8],
            "services": [],
            "domain": "",
            "contact": [:],
        ]
        if let distanceKm {
            row["distanceKm"] = distanceKm
        }
        return row
    }
}
