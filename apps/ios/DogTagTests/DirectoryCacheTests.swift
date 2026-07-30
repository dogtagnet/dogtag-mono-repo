import XCTest

/// The offline provider-record fallback, case by case.
///
/// REPLACES slice S-3's `CachedProviderDirectory` cases. Those pinned a decorator around a
/// no-argument `read()` of the whole provider set - live-read-first, replay-as-empty, nested stored
/// results, wrapper deadline renewal - and that seam no longer exists: a nearest read is personalized
/// and paged, so there is no single "the directory" response to substitute for. Everything that was
/// really about the CODEC or the store's failure tolerance carried over. What is new is the pair the
/// captain's ruling turns on: a distance is never persisted, and the stored order can never carry the
/// ranking.
///
/// Mirrors Android `DirectoryCacheTest` case for case; keep the two in step by hand.
final class DirectoryCacheTests: XCTestCase {
    private let namespace = "central:https://api.dogtag.io"
    private let storedAt = Date(timeIntervalSince1970: 1_785_312_000)

    private func provider(
        _ id: String,
        _ name: String,
        kind: String = "vet",
        geo: NearbyPoint? = NearbyPoint(lat: 1.3039, lng: 103.8318),
        contact: ProviderContact = ProviderContact(phone: "+65 6123 4567"),
        active: Bool? = nil,
        bindingState: IssuerBindingState = .unavailable,
        distanceKm: Double? = nil
    ) -> DirectoryProvider {
        DirectoryProvider(
            providerId: id,
            kind: kind,
            name: name,
            geo: geo,
            services: ["vaccination"],
            domain: nil,
            active: active,
            contact: contact,
            bindingState: bindingState,
            distanceKm: distanceKm
        )
    }

    private func cache(
        _ store: ProviderDirectoryCacheStore,
        namespace: String? = nil
    ) -> ProviderRecordCache {
        ProviderRecordCache(store: store, namespace: namespace ?? self.namespace)
    }

    // MARK: - The two properties the ruling turns on

    /// A distance is computed from the owner's position, so persisting one would put a position
    /// derivative on disk AND let a later offline read state a distance measured from somewhere the
    /// owner no longer is. Asserted against the raw document, because `DirectoryProvider` DOES carry
    /// `distanceKm` on this platform - dropping it is an active decision on every write.
    func test_aDistanceIsNeverWrittenToDisk() {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(
            [provider("a", "Alpha Vet", distanceKm: 1.4), provider("b", "Beta Vet", distanceKm: 9.2)],
            now: storedAt
        )

        let document = try? XCTUnwrap(store.read())
        let text = String(data: document ?? Data(), encoding: .utf8) ?? ""
        XCTAssertFalse(text.isEmpty)
        XCTAssertFalse(text.contains("distance"), "no distance may reach the document")
        XCTAssertFalse(text.contains("approximate"))
        // And it does not survive the round trip either.
        XCTAssertNil(cache(store).recall(now: storedAt.addingTimeInterval(1))?.providers.first?.distanceKm)
    }

    /// The array order of a nearest page IS the ranking, so replaying it verbatim would present a
    /// stale ordering as current - invisibly, because a sorted list simply looks sorted.
    func test_theStoredOrderIsNameOrderNotTheServerRanking() {
        let store = MemoryProviderDirectoryCacheStore()
        // As a nearest page arrives: closest first, which here is alphabetically backwards.
        cache(store).remember(
            [provider("z", "Zulu Vet"), provider("m", "Mike Vet"), provider("a", "Alpha Vet")],
            now: storedAt
        )

        XCTAssertEqual(
            ["Alpha Vet", "Mike Vet", "Zulu Vet"],
            cache(store).recall(now: storedAt.addingTimeInterval(1))?.providers.map(\.name)
        )
    }

    /// Re-sorting on READ is what makes that hold for a document this build did not write.
    func test_aReorderedDocumentIsStillReadBackInNameOrder() throws {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember([provider("a", "Alpha Vet"), provider("z", "Zulu Vet")], now: storedAt)

        var root = try XCTUnwrap(
            JSONSerialization.jsonObject(with: try XCTUnwrap(store.read())) as? [String: Any]
        )
        let rows = try XCTUnwrap(root["providers"] as? [[String: Any]])
        root["providers"] = Array(rows.reversed())
        store.write(try JSONSerialization.data(withJSONObject: root))

        XCTAssertEqual(
            ["Alpha Vet", "Zulu Vet"],
            cache(store).recall(now: storedAt.addingTimeInterval(1))?.providers.map(\.name)
        )
    }

    /// "Minimal" was the instruction twice over: the stored set cannot grow with the directory.
    func test_theStoredSetIsCappedAtOnePage() {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(
            (1...80).map { provider("p\($0)", String(format: "Vet %02d", $0)) },
            now: storedAt
        )

        XCTAssertEqual(ProviderRecordCache.maxRecords, OwnerProviderDirectoryRequest.pageSize)
        XCTAssertEqual(
            ProviderRecordCache.maxRecords,
            cache(store).recall(now: storedAt.addingTimeInterval(1))?.providers.count
        )
    }

    // MARK: - Could-not-answer never becomes an established absence

    func test_nothingStoredIsNilRatherThanAnEmptyRecordSet() {
        XCTAssertNil(cache(MemoryProviderDirectoryCacheStore()).recall(now: storedAt))
    }

    func test_anUndecodableDocumentIsNothingStored() {
        let store = MemoryProviderDirectoryCacheStore()
        store.write(Data("{ not json".utf8))

        XCTAssertNil(cache(store).recall(now: storedAt))
    }

    func test_aStoredSetIsNeverReplayedForADifferentlyConfiguredDeployment() {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember([provider("a", "Alpha Vet")], now: storedAt)

        XCTAssertNil(
            cache(store, namespace: "central:https://other.example")
                .recall(now: storedAt.addingTimeInterval(1))
        )
    }

    /// At the exact deadline it is expired, matching Android and the web.
    func test_anEntryExpiresAtItsExactDeadlineNotAfterIt() {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember([provider("a", "Alpha Vet")], now: storedAt)
        let deadline = storedAt.addingTimeInterval(ProviderRecordCache.defaultTtl)

        XCTAssertNotNil(cache(store).recall(now: deadline.addingTimeInterval(-1)))
        XCTAssertNil(cache(store).recall(now: deadline))
    }

    /// A stored time in the future is a backwards clock jump, not a fresh copy.
    func test_aSetStoredInTheFutureIsDroppedRatherThanTrusted() {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember([provider("a", "Alpha Vet")], now: storedAt)

        XCTAssertNil(cache(store).recall(now: storedAt.addingTimeInterval(-1)))
    }

    /// Version 1 was S-3's full-set snapshot, whose `providers` array was in server order - the
    /// ranking. Dropping rather than migrating is what stops it being reinterpreted as a record set.
    func test_aDocumentFromTheEarlierSnapshotShapeIsDroppedRatherThanMigrated() throws {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember([provider("a", "Alpha Vet")], now: storedAt)

        var root = try XCTUnwrap(
            JSONSerialization.jsonObject(with: try XCTUnwrap(store.read())) as? [String: Any]
        )
        root["version"] = 1
        store.write(try JSONSerialization.data(withJSONObject: root))

        XCTAssertNil(cache(store).recall(now: storedAt.addingTimeInterval(1)))
    }

    func test_aStoredSetCarryingNoProvidersIsMalformedRatherThanAnEmptyAnswer() throws {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember([provider("a", "Alpha Vet")], now: storedAt)

        var root = try XCTUnwrap(
            JSONSerialization.jsonObject(with: try XCTUnwrap(store.read())) as? [String: Any]
        )
        root["providers"] = [[String: Any]]()
        store.write(try JSONSerialization.data(withJSONObject: root))

        XCTAssertNil(cache(store).recall(now: storedAt.addingTimeInterval(1)))
    }

    /// A nameless row renders as a list entry the owner cannot act on.
    func test_aBlankNameOrIdIsRefusedOnWriteAndOnReplay() {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember([provider("", "No Id"), provider("b", "   ")], now: storedAt)

        XCTAssertNil(store.read(), "nothing well-formed was offered, so nothing is stored")
        XCTAssertFalse(providerRecordIsWellFormed(provider("", "No Id")))
        XCTAssertFalse(providerRecordIsWellFormed(provider("b", " ")))
        XCTAssertTrue(providerRecordIsWellFormed(provider("b", "Beta Vet")))
    }

    /// An empty live page means this query matched nothing, which is not evidence that the previously
    /// remembered providers ceased to exist - so it neither writes nor clears.
    func test_anEmptyLivePageLeavesThePreviouslyRememberedSetAlone() {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember([provider("a", "Alpha Vet")], now: storedAt)
        cache(store).remember([], now: storedAt.addingTimeInterval(1))

        XCTAssertEqual(
            ["Alpha Vet"],
            cache(store).recall(now: storedAt.addingTimeInterval(2))?.providers.map(\.name)
        )
    }

    /// Replace, not accumulate: offline shows the last providers seen, deliberately not a history.
    func test_rememberReplacesRatherThanAccumulating() {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember([provider("a", "Alpha Vet")], now: storedAt)
        cache(store).remember([provider("b", "Beta Vet")], now: storedAt.addingTimeInterval(1))

        XCTAssertEqual(
            ["Beta Vet"],
            cache(store).recall(now: storedAt.addingTimeInterval(2))?.providers.map(\.name)
        )
    }

    // MARK: - Codec fidelity (carried over from S-3)

    func test_theStoredDocumentRoundTripsEveryFieldIncludingAbsentLocation() {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(
            [
                provider(
                    "contact-only",
                    "Contact Only Vet",
                    geo: nil,
                    contact: ProviderContact(
                        phone: "+65 1",
                        whatsapp: "+65 2",
                        telegram: "@three",
                        email: "four@example.test",
                        website: "https://five.example.test"
                    ),
                    active: true,
                    bindingState: .noDomainListed
                ),
                provider("placed", "Placed Groomer", kind: "groomer", geo: NearbyPoint(lat: 0, lng: 0)),
            ],
            now: storedAt
        )

        let recalled = cache(store).recall(now: storedAt.addingTimeInterval(1))?.providers
        XCTAssertEqual(recalled?.count, 2)
        let contactOnly = recalled?.first { $0.providerId == "contact-only" }
        XCTAssertNil(contactOnly?.geo)
        XCTAssertEqual(contactOnly?.bindingState, .noDomainListed)
        XCTAssertEqual(contactOnly?.contact.phone, "+65 1")
        XCTAssertEqual(contactOnly?.contact.website, "https://five.example.test")
        XCTAssertEqual(contactOnly?.active, true)
        // `0,0` is a real coordinate and survives as one.
        let placed = recalled?.first { $0.providerId == "placed" }
        XCTAssertEqual(placed?.geo, NearbyPoint(lat: 0, lng: 0))
        XCTAssertEqual(placed?.kind, "groomer")
        XCTAssertNil(placed?.active)
    }

    func test_anAbsentLocationIsStoredAsAbsentNeverAsTheRealCoordinateZeroZero() throws {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember([provider("c", "Contact Only", geo: nil)], now: storedAt)

        let root = try XCTUnwrap(
            JSONSerialization.jsonObject(with: try XCTUnwrap(store.read())) as? [String: Any]
        )
        let row = try XCTUnwrap((root["providers"] as? [[String: Any]])?.first)
        XCTAssertNil(row["geo"], "absence must be absence, not a pin off the coast of Ghana")
    }

    /// A stored `verified` would claim a DNS check nobody performed.
    func test_aStoredBindingStateCanNeverClaimVerified() {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember(
            [provider("v", "Verified Vet", bindingState: .verified(description: "listed"))],
            now: storedAt
        )

        XCTAssertEqual(
            cache(store).recall(now: storedAt.addingTimeInterval(1))?.providers.first?.bindingState,
            .unavailable
        )
    }

    /// The document records WHEN it was stored, never that it was live: a replay is a replay.
    func test_aStoredDocumentCannotDeclareItselfLive() throws {
        let store = MemoryProviderDirectoryCacheStore()
        cache(store).remember([provider("a", "Alpha Vet")], now: storedAt)

        let text = String(data: try XCTUnwrap(store.read()), encoding: .utf8) ?? ""
        XCTAssertFalse(text.contains("observation"))
        XCTAssertFalse(text.contains("\"live\""))
        XCTAssertEqual(cache(store).recall(now: storedAt.addingTimeInterval(1))?.storedAt, storedAt)
    }
}
