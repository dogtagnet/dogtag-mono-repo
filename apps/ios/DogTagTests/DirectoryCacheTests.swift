import XCTest

/// The on-device local copy, case by case.
///
/// Mirrors the Android `DirectoryCacheTest` and `packages/ui/test/providerDirectory.test.ts`'s cache
/// cases. The rule they all serve: `unavailable` and `empty` are different statements, and no cache
/// state may turn the first into the second.
final class DirectoryCacheTests: XCTestCase {
    private let namespace = "central:https://central.test/v1/businesses"

    private let located = DirectoryProvider(
        providerId: "located",
        kind: "vet",
        name: "Harbour Vet",
        geo: NearbyPoint(lat: 1.3039, lng: 103.8318),
        services: ["vaccination"],
        domain: "harbour.test",
        active: nil,
        contact: ProviderContact(phone: "+65 6123 4567"),
        bindingState: .unavailable
    )

    private let contactOnly = DirectoryProvider(
        providerId: "contact-only",
        kind: "groomer",
        name: "Call-only Grooming",
        geo: nil,
        services: [],
        domain: nil,
        active: nil,
        contact: ProviderContact(website: "https://call-only.test"),
        bindingState: .noDomainListed
    )

    private final class FakeDirectory: ProviderDirectoryReading {
        let source: ProviderDirectorySource = .central
        let cacheNamespace: String
        var next: () -> ProviderDirectoryResult
        private(set) var reads = 0

        init(
            cacheNamespace: String = "central:https://central.test/v1/businesses",
            next: @escaping () -> ProviderDirectoryResult
        ) {
            self.cacheNamespace = cacheNamespace
            self.next = next
        }

        func read() async -> ProviderDirectoryResult {
            reads += 1
            return next()
        }
    }

    private func snapshot(
        providers: [DirectoryProvider],
        readAt: TimeInterval,
        expiresAt: TimeInterval? = nil,
        observation: ProviderDirectoryObservation = .live,
        source: ProviderDirectorySource = .central
    ) -> ProviderDirectorySnapshot {
        ProviderDirectorySnapshot(
            source: source,
            providers: providers,
            observation: observation,
            blockNumber: nil,
            readAt: Date(timeIntervalSince1970: readAt),
            expiresAt: expiresAt.map { Date(timeIntervalSince1970: $0) }
        )
    }

    private func found(_ readAt: TimeInterval, expiresAt: TimeInterval? = nil) -> ProviderDirectoryResult {
        .found(snapshot(providers: [located, contactOnly], readAt: readAt, expiresAt: expiresAt))
    }

    private func unavailable(_ at: TimeInterval) -> ProviderDirectoryResult {
        .unavailable(ProviderDirectoryUnavailable(
            source: .central,
            reason: .sourceUnavailable,
            detail: "offline",
            attemptedAt: Date(timeIntervalSince1970: at)
        ))
    }

    private func date(_ seconds: TimeInterval) -> Date { Date(timeIntervalSince1970: seconds) }

    // MARK: - the statement each state is entitled to make

    /// The headline property. An unreachable directory must not read as an empty one: telling an
    /// owner there is no help nearby when the truth is "we could not ask" sends them away.
    func test_anUnreachableDirectoryWithNothingStoredStaysUnavailableAndNeverBecomesEmpty() async {
        let directory = CachedProviderDirectory(
            delegate: FakeDirectory { self.unavailable(5_000) },
            store: MemoryProviderDirectoryCacheStore(),
            now: { self.date(5_000) }
        )
        guard case .unavailable(let result) = await directory.read() else {
            return XCTFail("an unreachable directory must never resolve found or empty")
        }
        XCTAssertEqual(result.reason, .sourceUnavailable)
    }

    /// A source that really did answer "none" keeps saying so, stored or live.
    func test_aStoredEmptyReplaysAsEmptyRatherThanUnavailable() async {
        var time: TimeInterval = 1_000
        let delegate = FakeDirectory { .empty(self.snapshot(providers: [], readAt: 1_000)) }
        let directory = CachedProviderDirectory(
            delegate: delegate,
            store: MemoryProviderDirectoryCacheStore(),
            ttl: 10_000,
            now: { self.date(time) }
        )
        guard case .empty = await directory.read() else { return XCTFail("expected a live empty") }

        time = 2_000
        delegate.next = { self.unavailable(2_000) }
        guard case .empty(let replayed) = await directory.read() else {
            return XCTFail("a stored empty must stay empty, not degrade to unavailable")
        }
        XCTAssertEqual(replayed.observation, .stored)
    }

    /// A `found` carrying zero providers renders as "nothing found" and is the exact false absence
    /// this module exists to prevent. TypeScript makes it unrepresentable; a Swift array cannot, so
    /// the guard must run on the way IN and on the way back OUT of storage.
    func test_aFoundCarryingNoProvidersIsRefusedOnWriteAndOnReplay() async {
        let empty = ProviderDirectoryResult.found(snapshot(providers: [], readAt: 1_000))
        XCTAssertFalse(providerDirectorySnapshotIsWellFormed(empty))

        let store = MemoryProviderDirectoryCacheStore()
        let directory = CachedProviderDirectory(
            delegate: FakeDirectory { empty },
            store: store,
            now: { self.date(1_000) }
        )
        guard case .unavailable(let result) = await directory.read() else {
            return XCTFail("a found with no providers must not reach the list")
        }
        XCTAssertEqual(result.reason, .invalidSnapshot)
        XCTAssertNil(store.read(), "a corrupt snapshot is never stored")

        // And the same document arriving from disk cannot be replayed either.
        var forged = try! JSONSerialization.jsonObject(
            with: ProviderDirectoryCacheCodec.encode(ProviderDirectoryCacheEntry(
                namespace: namespace,
                snapshot: snapshot(providers: [located], readAt: 1_000, expiresAt: 5_000),
                readAt: date(1_000),
                expiresAt: date(5_000)
            ))!
        ) as! [String: Any]
        forged["providers"] = []
        store.write(try! JSONSerialization.data(withJSONObject: forged))

        let replayed = await CachedProviderDirectory(
            delegate: FakeDirectory { self.unavailable(2_000) },
            store: store,
            ttl: 10_000,
            now: { self.date(2_000) }
        ).read()
        guard case .unavailable = replayed else {
            return XCTFail("a stored found-with-zero-providers must not reach the list")
        }
    }

    // MARK: - the two review findings

    /// Review finding 1, satisfied STRUCTURALLY rather than by a catch.
    ///
    /// `ProviderDirectoryReading.read()` is a non-throwing `async` function, so an unexpected error
    /// cannot escape a delegate at all - the compiler enforces here what the Kotlin twin has to
    /// enforce with a rethrowing catch. This pins that the seam stays non-throwing: if someone makes
    /// it `async throws`, this stops compiling and the catch-before-replay becomes mandatory.
    func test_theReadSeamIsNonThrowingSoAnUnexpectedErrorCannotSkipTheReplay() async {
        let seam: ProviderDirectoryReading = FakeDirectory { self.unavailable(1_000) }
        // No `try`. A throwing seam would fail to compile on this line.
        let result: ProviderDirectoryResult = await seam.read()
        guard case .unavailable = result else { return XCTFail("expected the fake's own result") }
    }

    /// The behavioural half: a delegate that fails for ANY reason still reaches the stored copy.
    func test_aFailedRefreshForAnyReasonStillConsultsTheStoredCopy() async {
        var time: TimeInterval = 1_000
        let delegate = FakeDirectory { self.found(1_000) }
        let directory = CachedProviderDirectory(
            delegate: delegate,
            store: MemoryProviderDirectoryCacheStore(),
            ttl: 10_000,
            now: { self.date(time) }
        )
        guard case .found = await directory.read() else { return XCTFail("expected a live found") }

        time = 2_000
        for reason: ProviderDirectoryUnavailableReason in [.sourceUnavailable, .malformedResponse] {
            delegate.next = {
                .unavailable(ProviderDirectoryUnavailable(
                    source: .central,
                    reason: reason,
                    detail: "failed",
                    attemptedAt: self.date(2_000)
                ))
            }
            guard case .found(let replayed) = await directory.read() else {
                return XCTFail("a \(reason) refresh must replay the stored copy")
            }
            XCTAssertEqual(replayed.observation, .stored)
        }
    }

    /// Review finding 2. The namespace must actually distinguish deployments, or one backend's
    /// providers are replayed as another's. It is derived from the configured endpoint.
    func test_aStoredCopyIsNeverReplayedForADifferentlyConfiguredDeployment() async {
        let store = MemoryProviderDirectoryCacheStore()
        var time: TimeInterval = 1_000
        _ = await CachedProviderDirectory(
            delegate: FakeDirectory(cacheNamespace: "central:https://a.test/v1/businesses") {
                self.found(1_000)
            },
            store: store,
            ttl: 10_000,
            now: { self.date(time) }
        ).read()
        XCTAssertNotNil(store.read())

        time = 2_000
        let other = await CachedProviderDirectory(
            delegate: FakeDirectory(cacheNamespace: "central:https://b.test/v1/businesses") {
                self.unavailable(2_000)
            },
            store: store,
            ttl: 10_000,
            now: { self.date(time) }
        ).read()
        guard case .unavailable = other else {
            return XCTFail("a foreign deployment's copy is not a cache hit")
        }
        XCTAssertNil(store.read(), "and the mismatched entry is dropped")
    }

    /// Two real central endpoints really do produce two namespaces.
    func test_theCentralNamespaceFollowsTheConfiguredEndpoint() {
        XCTAssertEqual(
            CentralProviderDirectory(baseURL: "https://a.test").cacheNamespace,
            "central:https://a.test/v1/businesses"
        )
        XCTAssertNotEqual(
            CentralProviderDirectory(baseURL: "https://a.test").cacheNamespace,
            CentralProviderDirectory(baseURL: "https://b.test").cacheNamespace
        )
    }

    // MARK: - TTL semantics

    /// Re-check first: a reachable source is always asked, and its answer always wins.
    func test_aLiveReadIsAlwaysAttemptedAndAlwaysReplacesTheStoredCopy() async {
        var time: TimeInterval = 1_000
        let delegate = FakeDirectory { self.found(time) }
        let directory = CachedProviderDirectory(
            delegate: delegate,
            store: MemoryProviderDirectoryCacheStore(),
            ttl: 10_000,
            now: { self.date(time) }
        )
        _ = await directory.read()
        time = 2_000
        guard case .found(let second) = await directory.read() else { return XCTFail("expected found") }
        XCTAssertEqual(delegate.reads, 2)
        XCTAssertEqual(second.observation, .live)
        XCTAssertEqual(second.readAt, date(2_000))
        XCTAssertEqual(second.expiresAt, date(12_000))
    }

    /// Expiry is at the exact boundary: usable strictly before it, expired the instant it arrives.
    func test_anEntryExpiresAtItsExactDeadlineNotAfterIt() async {
        var time: TimeInterval = 1_000
        let delegate = FakeDirectory { self.found(1_000) }
        let store = MemoryProviderDirectoryCacheStore()
        let directory = CachedProviderDirectory(
            delegate: delegate, store: store, ttl: 1_000, now: { self.date(time) }
        )
        _ = await directory.read()
        delegate.next = { self.unavailable(time) }

        time = 1_999
        guard case .found = await directory.read() else {
            return XCTFail("strictly before the deadline the copy is usable")
        }

        time = 2_000
        guard case .unavailable = await directory.read() else {
            return XCTFail("at the deadline itself the entry is expired")
        }
        XCTAssertNil(store.read())
    }

    /// A cache that renewed its deadline on every replay would never expire, so an offline phone
    /// would keep showing a stale directory forever while calling it merely "stored".
    func test_aStoredReplayNeverRenewsTheHardDeadline() async {
        var time: TimeInterval = 1_000
        let delegate = FakeDirectory { self.found(1_000) }
        let directory = CachedProviderDirectory(
            delegate: delegate,
            store: MemoryProviderDirectoryCacheStore(),
            ttl: 1_000,
            now: { self.date(time) }
        )
        _ = await directory.read()
        delegate.next = { self.unavailable(time) }

        for instant: TimeInterval in [1_200, 1_500, 1_900] {
            time = instant
            guard case .found(let replay) = await directory.read() else {
                return XCTFail("expected a replay at \(instant)")
            }
            XCTAssertEqual(replay.expiresAt, date(2_000), "the deadline is fixed at the original read")
            XCTAssertEqual(replay.readAt, date(1_000))
        }

        time = 2_000
        guard case .unavailable = await directory.read() else {
            return XCTFail("a replayed copy must still reach its original deadline")
        }
    }

    /// A backwards clock jump must not extend the window. The stored deadline is absolute, so a
    /// snapshot "read" in the future would otherwise stay usable for however far the clock moved.
    func test_aSnapshotReadInTheFutureIsDroppedRatherThanTrusted() async {
        let store = MemoryProviderDirectoryCacheStore()
        store.write(ProviderDirectoryCacheCodec.encode(ProviderDirectoryCacheEntry(
            namespace: namespace,
            snapshot: snapshot(providers: [located], readAt: 9_000, expiresAt: 19_000),
            readAt: date(9_000),
            expiresAt: date(19_000)
        ))!)
        let result = await CachedProviderDirectory(
            delegate: FakeDirectory { self.unavailable(1_000) },
            store: store,
            ttl: 10_000,
            now: { self.date(1_000) }
        ).read()
        guard case .unavailable = result else {
            return XCTFail("a future-dated snapshot must not be replayed")
        }
        XCTAssertNil(store.read())
    }

    /// Shortening the TTL in a later build discards previously-stored entries rather than clamping
    /// them. That is fail-closed, and it is what keeps "a replay never renews" true across upgrades.
    func test_aStoredDeadlineLongerThanTheCurrentTtlIsRefused() async {
        let store = MemoryProviderDirectoryCacheStore()
        store.write(ProviderDirectoryCacheCodec.encode(ProviderDirectoryCacheEntry(
            namespace: namespace,
            snapshot: snapshot(providers: [located], readAt: 1_000, expiresAt: 100_000),
            readAt: date(1_000),
            expiresAt: date(100_000)
        ))!)
        let result = await CachedProviderDirectory(
            delegate: FakeDirectory { self.unavailable(2_000) },
            store: store,
            ttl: 1_000,
            now: { self.date(2_000) }
        ).read()
        guard case .unavailable = result else {
            return XCTFail("an over-long stored deadline must not be honoured")
        }
    }

    /// An inner wrapper's replay is not a successful refresh. Storing it would let stacked wrappers
    /// renew stale data indefinitely, which is the same never-expires defect by another route.
    func test_aNestedStoredResultIsPassedThroughAndNotRestored() async {
        let store = MemoryProviderDirectoryCacheStore()
        let nested = ProviderDirectoryResult.found(snapshot(
            providers: [located], readAt: 1_000, expiresAt: 2_000, observation: .stored
        ))
        let passed = await CachedProviderDirectory(
            delegate: FakeDirectory { nested },
            store: store,
            ttl: 10_000,
            now: { self.date(1_500) }
        ).read()
        XCTAssertEqual(passed, nested)
        XCTAssertNil(store.read(), "a replay must not be written back as a fresh copy")
    }

    /// A stored result whose own deadline has already passed is not a usable answer.
    func test_aNestedStoredResultPastItsDeadlineIsRefused() async {
        let nested = ProviderDirectoryResult.found(snapshot(
            providers: [located], readAt: 1_000, expiresAt: 2_000, observation: .stored
        ))
        let result = await CachedProviderDirectory(
            delegate: FakeDirectory { nested },
            store: MemoryProviderDirectoryCacheStore(),
            ttl: 10_000,
            now: { self.date(2_000) }
        ).read()
        guard case .unavailable(let unavailable) = result else {
            return XCTFail("an expired nested replay is not an answer")
        }
        XCTAssertEqual(unavailable.reason, .invalidSnapshot)
    }

    /// A source claiming to be something other than the configured one is a trust-boundary change.
    func test_aSourceThatIdentifiesItselfDifferentlyIsRefusedAndClearsTheCopy() async {
        let store = MemoryProviderDirectoryCacheStore()
        var time: TimeInterval = 1_000
        let delegate = FakeDirectory { self.found(1_000) }
        let directory = CachedProviderDirectory(
            delegate: delegate, store: store, ttl: 10_000, now: { self.date(time) }
        )
        _ = await directory.read()
        XCTAssertNotNil(store.read())

        time = 2_000
        delegate.next = {
            .found(self.snapshot(providers: [self.located], readAt: 2_000, source: .onchain))
        }
        guard case .unavailable(let result) = await directory.read() else {
            return XCTFail("a source swap is not a successful read")
        }
        XCTAssertEqual(result.reason, .inconsistentSource)
        XCTAssertNil(store.read())
    }

    // MARK: - the stored document

    /// Everything a row carries survives a round trip, absence included.
    func test_theStoredDocumentRoundTripsEveryFieldIncludingAbsentLocation() {
        let entry = ProviderDirectoryCacheEntry(
            namespace: namespace,
            snapshot: snapshot(providers: [located, contactOnly], readAt: 1_000, expiresAt: 5_000),
            readAt: date(1_000),
            expiresAt: date(5_000)
        )
        guard let document = ProviderDirectoryCacheCodec.encode(entry),
              let decoded = ProviderDirectoryCacheCodec.decode(document) else {
            return XCTFail("a well-formed entry must round trip")
        }
        XCTAssertEqual(decoded.namespace, namespace)
        XCTAssertEqual(decoded.readAt, date(1_000))
        XCTAssertEqual(decoded.expiresAt, date(5_000))
        XCTAssertEqual(decoded.snapshot.providers.count, 2)

        let first = decoded.snapshot.providers[0]
        XCTAssertEqual(first.geo, NearbyPoint(lat: 1.3039, lng: 103.8318))
        XCTAssertEqual(first.contact.phone, "+65 6123 4567")
        XCTAssertNil(first.active)

        let second = decoded.snapshot.providers[1]
        XCTAssertNil(second.geo, "a contact-only provider stays location-less")
        XCTAssertEqual(second.contact.website, "https://call-only.test")
        XCTAssertTrue(second.contact.hasAny)
        XCTAssertEqual(second.bindingState, .noDomainListed)
    }

    /// `0,0` is a legal coordinate off the coast of Ghana and was, before S-1, what a blank location
    /// became. Writing absence as `0,0` here would resurrect that pin from the phone's own storage.
    func test_anAbsentLocationIsStoredAsAbsentNeverAsTheRealCoordinateZeroZero() {
        let document = ProviderDirectoryCacheCodec.encode(ProviderDirectoryCacheEntry(
            namespace: namespace,
            snapshot: snapshot(providers: [contactOnly], readAt: 1_000, expiresAt: 5_000),
            readAt: date(1_000),
            expiresAt: date(5_000)
        ))!
        let object = try! JSONSerialization.jsonObject(with: document) as! [String: Any]
        let row = (object["providers"] as! [[String: Any]])[0]
        XCTAssertNil(row["geo"], "absence is written as absence")
    }

    /// A replay is a replay however the document is edited: observation is never persisted.
    func test_aStoredDocumentCannotDeclareItselfLive() {
        let document = ProviderDirectoryCacheCodec.encode(ProviderDirectoryCacheEntry(
            namespace: namespace,
            snapshot: snapshot(providers: [located], readAt: 1_000, expiresAt: 5_000),
            readAt: date(1_000),
            expiresAt: date(5_000)
        ))!
        var object = try! JSONSerialization.jsonObject(with: document) as! [String: Any]
        XCTAssertNil(object["observation"], "observation is not part of the stored shape")
        object["observation"] = "live"
        let edited = try! JSONSerialization.data(withJSONObject: object)
        XCTAssertEqual(ProviderDirectoryCacheCodec.decode(edited)?.snapshot.observation, .stored)
    }

    /// A directory source performs no DNS or chain check, so a stored `verified` would be a claim
    /// about work nobody did. It cannot be written and cannot be read back.
    func test_aStoredBindingStateCanNeverClaimVerified() {
        let document = ProviderDirectoryCacheCodec.encode(ProviderDirectoryCacheEntry(
            namespace: namespace,
            snapshot: snapshot(providers: [located], readAt: 1_000, expiresAt: 5_000),
            readAt: date(1_000),
            expiresAt: date(5_000)
        ))!
        var object = try! JSONSerialization.jsonObject(with: document) as! [String: Any]
        var rows = object["providers"] as! [[String: Any]]
        rows[0]["bindingState"] = "verified"
        object["providers"] = rows
        let edited = try! JSONSerialization.data(withJSONObject: object)
        let provider = ProviderDirectoryCacheCodec.decode(edited)!.snapshot.providers[0]
        XCTAssertEqual(provider.bindingState, .unavailable)
        XCTAssertNotEqual(provider.bindingState, .verified(description: "anything"))
    }

    /// A document from an older stored shape is dropped, not migrated. Pre-S-1 rows encode a blank
    /// location as `0,0`, which AGENTS.md records as unreinterpretable by code.
    func test_aDocumentFromAnEarlierStoredShapeIsDroppedRatherThanMigrated() async {
        let document = ProviderDirectoryCacheCodec.encode(ProviderDirectoryCacheEntry(
            namespace: namespace,
            snapshot: snapshot(providers: [located], readAt: 1_000, expiresAt: 5_000),
            readAt: date(1_000),
            expiresAt: date(5_000)
        ))!
        var object = try! JSONSerialization.jsonObject(with: document) as! [String: Any]
        object["version"] = ProviderDirectoryCacheCodec.version - 1
        let stale = try! JSONSerialization.data(withJSONObject: object)
        XCTAssertNil(ProviderDirectoryCacheCodec.decode(stale))

        let store = MemoryProviderDirectoryCacheStore()
        store.write(stale)
        let result = await CachedProviderDirectory(
            delegate: FakeDirectory { self.unavailable(2_000) },
            store: store,
            ttl: 10_000,
            now: { self.date(2_000) }
        ).read()
        guard case .unavailable = result else {
            return XCTFail("a stale-shape document must not be replayed")
        }
    }

    /// Garbage on disk is "nothing stored", never a guess at what it might have meant.
    func test_anUnreadableDocumentIsNothingStored() {
        XCTAssertNil(ProviderDirectoryCacheCodec.decode(Data("not json at all".utf8)))
        XCTAssertNil(ProviderDirectoryCacheCodec.decode(Data("{}".utf8)))
    }

    /// A stored `empty` may not smuggle providers in beside it, nor a `found` arrive with none.
    func test_aStoredStateMustAgreeWithItsProviderList() {
        let document = ProviderDirectoryCacheCodec.encode(ProviderDirectoryCacheEntry(
            namespace: namespace,
            snapshot: snapshot(providers: [located], readAt: 1_000, expiresAt: 5_000),
            readAt: date(1_000),
            expiresAt: date(5_000)
        ))!
        var object = try! JSONSerialization.jsonObject(with: document) as! [String: Any]
        object["state"] = "empty"
        XCTAssertNil(
            ProviderDirectoryCacheCodec.decode(try! JSONSerialization.data(withJSONObject: object))
        )
    }

    /// `unavailable` is not a snapshot: it is never stored, so it can never be replayed as one.
    func test_unavailableIsNeverAWellFormedSnapshot() {
        XCTAssertFalse(providerDirectorySnapshotIsWellFormed(unavailable(1_000)))
    }

    /// A misconfigured lifetime disables the copy rather than storing an unevaluable deadline.
    func test_aNonPositiveTtlDisablesTheCopyRatherThanStoringAnUnevaluableDeadline() async {
        let store = MemoryProviderDirectoryCacheStore()
        for ttl: TimeInterval in [0, -1] {
            let result = await CachedProviderDirectory(
                delegate: FakeDirectory { self.found(1_000) },
                store: store,
                ttl: ttl,
                now: { self.date(1_000) }
            ).read()
            guard case .found = result else { return XCTFail("the live answer still stands") }
            XCTAssertNil(store.read())
        }
    }
}
