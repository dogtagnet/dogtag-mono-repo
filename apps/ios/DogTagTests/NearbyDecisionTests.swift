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

    /// Captain's ruling, 2026-07-30: the EXACT fix is sent and is NOT rounded. The device-side gate is
    /// now validity only, so an unusable coordinate still cannot reach the wire while a precise one
    /// passes through untouched. Mirrors Android `CallerPosition.from`.
    func test_theExactPositionSurvivesTheDeviceGateAndAnUnusableOneDoesNot() {
        let precise = NearbyPoint(lat: 1.29349, lng: 103.85251)
        XCTAssertEqual(precise.validatedForProviderSearch(), precise)

        let negative = NearbyPoint(lat: -33.86551, lng: -0.0001)
        XCTAssertEqual(negative.validatedForProviderSearch(), negative)

        XCTAssertNil(NearbyPoint(lat: 91, lng: 0).validatedForProviderSearch())
        XCTAssertNil(NearbyPoint(lat: .nan, lng: 0).validatedForProviderSearch())
        XCTAssertNil(NearbyPoint(lat: 0, lng: .infinity).validatedForProviderSearch())
    }

    func test_nearestRequestHasExactOwnerKindsPagingNameAndExactPositionBody() throws {
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
        // Full precision reaches the BODY, and the URL still carries no coordinate at all - which is
        // what keeps the position out of ordinary access logs now that it is not coarsened.
        XCTAssertEqual(body["lat"]?.doubleValue, 1.29349)
        XCTAssertEqual(body["lng"]?.doubleValue, 103.85251)
        XCTAssertFalse(wire.url.contains("1.29"))
        XCTAssertFalse(wire.url.contains("103.85"))
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
            "Your location is sent to DogTag to find nearby vets and groomers. It is not stored.",
            NearbyDecision.locationDisclosure
        )
    }

    /// The captain allowed Directions on offline stored rows on condition that the
    /// stored-not-current labelling stayed on them, so this sentence is part of that ruling rather
    /// than decoration: a bare Directions button on a remembered row would read as a destination
    /// just confirmed with the service. Pinned byte-for-byte against Android for the same reason the
    /// disclosure above is.
    func test_theStoredDirectionsOfferSaysTheAddressMayBeOutOfDate() {
        XCTAssertEqual(
            "Saved on this phone - this address may be out of date.",
            NearbyDecision.storedDirectionsNote
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

    /// Mirrors Android `aTransientPageFailureKeepsTheLoadedPagesButAnInvalidatingOneDoesNot`.
    ///
    /// The two arms must not collapse into each other. A network blip on page 5 loses the owner's
    /// place and nothing else, so throwing away four good pages would report a could-not-reach as an
    /// emptied list; a response proving the set moved really does make those pages untrustworthy.
    func test_aTransientPageFailureKeepsTheLoadedPagesButAnInvalidatingOneDoesNot() {
        let current = pageResult(
            [provider(id: "first", name: "First", distanceKm: 1)],
            total: 2,
            limit: 1,
            offset: 0,
            hasMore: true
        )
        let transient = ProviderDirectoryPaging.merge(
            current: current,
            incoming: .unavailable(ProviderDirectoryUnavailable(
                source: .central,
                reason: .sourceUnavailable,
                detail: "The provider directory could not be reached",
                attemptedAt: now
            )),
            reset: false,
            requiresDistance: true,
            attemptedAt: now
        )
        guard case .found(let kept) = transient else {
            return XCTFail("a transient page failure must not discard the loaded pages")
        }
        XCTAssertEqual(kept.providers.map(\.providerId), ["first"])
        // `hasMore` survives, so the retry affordance the owner needs is still on screen.
        XCTAssertEqual(kept.page?.hasMore, true)
        XCTAssertEqual(kept.pageLoadFailure, "The provider directory could not be reached")

        // A retried page that succeeds clears the marker; otherwise the screen would go on announcing
        // a failure that is over.
        let retried = ProviderDirectoryPaging.merge(
            current: transient,
            incoming: pageResult(
                [provider(id: "second", name: "Second", distanceKm: 2)],
                total: 2,
                limit: 1,
                offset: 1,
                hasMore: false
            ),
            reset: false,
            requiresDistance: true,
            attemptedAt: now
        )
        guard case .found(let complete) = retried else {
            return XCTFail("expected the retried page to merge")
        }
        XCTAssertEqual(complete.providers.map(\.providerId), ["first", "second"])
        XCTAssertNil(complete.pageLoadFailure)

        for invalidating in [
            ProviderDirectoryUnavailableReason.malformedResponse,
            .inconsistentSource,
            .invalidSnapshot,
            .providerRegistryUnavailable,
        ] {
            let discarded = ProviderDirectoryPaging.merge(
                current: current,
                incoming: .unavailable(ProviderDirectoryUnavailable(
                    source: .central,
                    reason: invalidating,
                    detail: "changed underneath",
                    attemptedAt: now
                )),
                reset: false,
                requiresDistance: true,
                attemptedAt: now
            )
            guard case .unavailable(let failure) = discarded else {
                XCTFail("expected \(invalidating) to discard the accumulated pages")
                continue
            }
            XCTAssertEqual(failure.reason, invalidating)
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

    /// The only display floor is the fix's OWN accuracy.
    ///
    /// The former 100-metre floor existed because the service received a three-decimal coordinate, so
    /// no distance computed from it could be finer than that. The captain's exact-position ruling
    /// removed that coarsening, so keeping the floor would overstate uncertainty the request no longer
    /// introduces: a 5-metre fix may state 820 m. Mirrors Android
    /// `theOnlyDisplayFloorIsTheFixesOwnAccuracy`.
    func test_theOnlyDisplayFloorIsTheFixesOwnAccuracy() {
        let precise = NearbyDecision.presentation(
            directory: found([
                provider(id: "one", name: "One", distanceKm: 0.823),
            ]),
            location: .ready(NearbyOrigin(
                point: NearbyPoint(lat: 1.29349, lng: 103.85251),
                accuracyMetres: 5
            )),
            query: "",
            unitSystem: .metric
        )

        XCTAssertEqual(rows(precise)[0].distanceKm, 0.823)
        XCTAssertEqual(rows(precise)[0].distance, .measured(label: "820 m", approximate: true))
        XCTAssertEqual(rows(precise)[0].distance.display, "~820 m")

        // A coarse fix still floors the label at its own accuracy, and the bound rounds OUTWARD onto
        // the display ladder so it can never read tighter than the accuracy that produced it.
        let coarse = NearbyDecision.presentation(
            directory: found([
                provider(id: "one", name: "One", distanceKm: 0.823),
            ]),
            location: .ready(NearbyOrigin(
                point: NearbyPoint(lat: 1.29349, lng: 103.85251),
                accuracyMetres: 900
            )),
            query: "",
            unitSystem: .metric
        )
        XCTAssertEqual(rows(coarse)[0].distance, .measured(label: "< 900 m", approximate: true))
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

    // ---- The offline stored fallback (captain's cache ruling, 2026-07-30) ----
    //
    // These four close a gap AGENTS.md recorded openly: `storedFallback` / `storedProvidersOnly` /
    // `formatStoredAge` all shipped on this platform with no test referencing any of them, so the
    // iOS half of the offline decision rested on an it-mirrors-Android argument alone. Android's
    // `theStoredAgeIsCoarseAndNeverUnderstatesStaleness` even carries a comment claiming it mirrors
    // an iOS test of that name - which did not exist until now. Same shape as the pre-`VerdictDisplay`
    // gap: a property asserted on one platform only.

    /// The remembered set is UNRANKED and carries no distance, so it must not be routed through
    /// `presentation`: that drops every provider it has no server distance for and then reports
    /// `noNearbyProviders`, stating an absence about providers the phone is holding. This is the case
    /// that pins the separate presentation. Mirrors Android
    /// `theStoredFallbackPresentsRememberedProvidersWithoutDistanceOrRanking`.
    func test_theStoredFallbackPresentsRememberedProvidersWithoutDistanceOrRanking() {
        let records = StoredProviderRecords(
            providers: [
                provider(id: "a", name: "Alpha Vet"),
                provider(id: "b", name: "Beta Groomer", kind: "groomer"),
            ],
            storedAt: now
        )

        guard case .storedProvidersOnly(let shown, let storedAge) =
            NearbyDecision.storedFallback(records: records, query: "", now: now.addingTimeInterval(120))
        else { return XCTFail("remembered records must present as their own state") }
        XCTAssertEqual(shown.map(\.name), ["Alpha Vet", "Beta Groomer"])
        XCTAssertEqual(storedAge, "2 minutes ago")
        // That a stored row can claim no distance is pinned where it is actually enforced:
        // `DirectoryCacheTests.test_aDistanceIsNeverWrittenToDisk`, which writes non-nil distances and
        // asserts none reach the document. `storedFallback` only filters, so asserting it here would
        // either restate the helper's own default or claim a filter strips a field it never touches.

        // Routed through the live presentation instead, the same records would claim there are none.
        let throughNearby = NearbyDecision.presentation(
            directory: found(records.providers),
            location: .ready(NearbyOrigin(point: NearbyPoint(lat: 1.35, lng: 103.82), accuracyMetres: 50)),
            query: "",
            unitSystem: .metric
        )
        guard case .noNearbyProviders = throughNearby else {
            return XCTFail("the live path drops distance-less rows; that is why this state exists")
        }
    }

    /// Nothing remembered, or nothing matching, keeps the live could-not-check rather than answering.
    /// A fallback that quietly returned an empty list would turn "could not check" into an established
    /// absence. Mirrors Android `theStoredFallbackDeclinesRatherThanAnsweringAnEmptyList`.
    func test_theStoredFallbackDeclinesRatherThanAnsweringAnEmptyList() {
        XCTAssertNil(NearbyDecision.storedFallback(records: nil, query: "", now: now))
        let records = StoredProviderRecords(providers: [provider(id: "a", name: "Alpha Vet")], storedAt: now)
        XCTAssertNil(NearbyDecision.storedFallback(records: records, query: "no such provider", now: now))
    }

    /// A delisted provider stays hidden offline too, and an owner-foreign kind never appears; an
    /// unknown listing state remains eligible rather than being read as delisted. Mirrors Android
    /// `theStoredFallbackStillHidesDelistedProvidersAndOwnerForeignKinds`.
    func test_theStoredFallbackStillHidesDelistedProvidersAndOwnerForeignKinds() {
        let records = StoredProviderRecords(
            providers: [
                provider(id: "off", name: "Closed Vet", active: false),
                provider(id: "gov", name: "Ministry", kind: "government"),
                provider(id: "ok", name: "Open Vet", active: nil),
            ],
            storedAt: now
        )

        guard case .storedProvidersOnly(let shown, _) =
            NearbyDecision.storedFallback(records: records, query: "", now: now)
        else { return XCTFail("one eligible provider was remembered") }
        XCTAssertEqual(shown.map(\.name), ["Open Vet"])
    }

    /// The age rounds OUTWARD so a replay never reads fresher than it is, and a stored time in the
    /// future is a backwards clock jump rather than a fresh copy, so it says nothing at all. This is
    /// the test Android's own comment already claimed to mirror. Mirrors Android
    /// `theStoredAgeIsCoarseAndNeverUnderstatesStaleness`.
    func test_theStoredAgeIsCoarseAndNeverUnderstatesStaleness() {
        func age(_ elapsed: TimeInterval) -> String? {
            NearbyDecision.formatStoredAge(storedAt: now, now: now.addingTimeInterval(elapsed))
        }

        XCTAssertEqual(age(0), "less than a minute ago")
        XCTAssertEqual(age(59), "less than a minute ago")
        XCTAssertEqual(age(60), "1 minute ago")
        // 61 seconds is stated as two minutes, never as one.
        XCTAssertEqual(age(61), "2 minutes ago")
        XCTAssertEqual(age(3_599), "1 hour ago")
        XCTAssertEqual(age(3_600), "1 hour ago")
        XCTAssertEqual(age(3_601), "2 hours ago")
        XCTAssertEqual(age(86_399), "1 day ago")
        XCTAssertEqual(age(86_400), "1 day ago")
        XCTAssertEqual(age(6 * 86_400), "6 days ago")

        XCTAssertNil(age(-1))
    }

    // ---- The Directions handoff ----

    /// THE property of this affordance: the URL carries the provider's published destination and no
    /// trace of where the owner is. Apple Maps accepts a source address as `saddr` beside the
    /// destination `daddr`, so its absence is an active requirement rather than an omission - a URL
    /// handed to another application must never disclose the owner's own position, which is the same
    /// confinement the body-only nearest request exists to provide.
    ///
    /// Mirrors Android `theDirectionsHandoffCarriesTheDestinationAndNeverTheOrigin`.
    func test_theDirectionsHandoffCarriesTheDestinationAndNeverTheOrigin() {
        let url = NearbyDecision.directionsURL(
            for: provider(id: "a", name: "Alpha Vet", geo: NearbyPoint(lat: 1.35249, lng: 103.81951))
        )

        XCTAssertEqual(url?.absoluteString, "https://maps.apple.com/?daddr=1.352490,103.819510")
        let text = url?.absoluteString ?? ""
        XCTAssertFalse(text.contains("saddr"), "the owner's origin must never reach the maps handoff")
        // A nearby owner's own fix is close to, but not equal to, the destination.
        XCTAssertFalse(text.contains("1.3521"), "the owner's origin must never reach the maps handoff")
        XCTAssertFalse(text.contains("103.8198"), "the owner's origin must never reach the maps handoff")
    }

    /// A provider that published no location offers no Directions. Absence is `geo == nil` and only
    /// that: `(0, 0)` is a real coordinate off the coast of Ghana, so it routes like anywhere else.
    /// Reading it as absence is the bug this repo already fixed once in the admin directory.
    func test_onlyAnAbsentLocationWithholdsDirectionsAndZeroZeroIsARealDestination() {
        XCTAssertNil(NearbyDecision.directionsURL(for: provider(id: "c", name: "Contact Only", geo: nil)))
        XCTAssertEqual(
            NearbyDecision.directionsURL(
                for: provider(id: "g", name: "Gulf of Guinea", geo: NearbyPoint(lat: 0, lng: 0))
            )?.absoluteString,
            "https://maps.apple.com/?daddr=0.000000,0.000000"
        )
        // An unusable coordinate is not a destination either.
        XCTAssertNil(NearbyDecision.directionsURL(
            for: provider(id: "b", name: "Broken", geo: NearbyPoint(lat: 91, lng: 0))
        ))
        XCTAssertNil(NearbyDecision.directionsURL(
            for: provider(id: "n", name: "NaN", geo: NearbyPoint(lat: .nan, lng: 0))
        ))
    }

    /// Fixed-point, locale-independent, and signed. `"\(Double)"` would emit `1e-05` just off the
    /// meridian - which no maps app parses - and a locale-aware formatter would emit `1,35` in a
    /// comma-decimal locale, silently splitting the pair into two coordinates.
    func test_directionsCoordinatesAreFixedPointAndSurviveBothSigns() {
        XCTAssertEqual(
            NearbyDecision.directionsURL(
                for: provider(id: "s", name: "South", geo: NearbyPoint(lat: -33.86551, lng: -151.2099))
            )?.absoluteString,
            "https://maps.apple.com/?daddr=-33.865510,-151.209900"
        )
        XCTAssertEqual(
            NearbyDecision.directionsURL(
                for: provider(id: "m", name: "Meridian", geo: NearbyPoint(lat: 0.00001, lng: 0))
            )?.absoluteString,
            "https://maps.apple.com/?daddr=0.000010,0.000000"
        )
    }
}
