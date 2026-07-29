import Foundation

/// A coordinate the directory or the user already supplied. This type performs no acquisition or I/O.
struct NearbyPoint: Equatable {
    let lat: Double
    let lng: Double

    var isValid: Bool {
        lat.isFinite && lng.isFinite &&
            (-90.0...90.0).contains(lat) &&
            (-180.0...180.0).contains(lng)
    }
}

/// Public contact channels a provider chose to publish.
///
/// Membership and order mirror `PROVIDER_CONTACT_CHANNELS` in
/// `packages/ui/src/directory/channels.ts`, which Swift cannot import: keep them aligned by hand.
/// A channel the server serves and this type omits is worse than one never added - the parser drops
/// it, `hasAny` then reads false, and a provider that published a website is told to the owner as
/// having published nothing.
struct ProviderContact: Equatable {
    var phone: String? = nil
    var whatsapp: String? = nil
    var telegram: String? = nil
    var email: String? = nil
    var website: String? = nil

    var hasAny: Bool {
        [phone, whatsapp, telegram, email, website].contains { value in
            value?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
        }
    }
}

/// The source-neutral row the native directory adapter gives the screen.
struct DirectoryProvider: Equatable, Identifiable {
    let providerId: String
    let kind: String
    let name: String
    /// `nil` is a first-class contact-only provider, never a `0,0` sentinel.
    let geo: NearbyPoint?
    let services: [String]
    let domain: String?
    /// `nil` means the source made no current-listing claim.
    let active: Bool?
    let contact: ProviderContact
    /// Reuses the credential provenance state machine; never a listing-specific verification enum.
    let bindingState: IssuerBindingState

    var id: String { providerId }
}

enum ProviderDirectorySource: Equatable {
    case central
    case onchain
}

enum ProviderDirectoryObservation: Equatable {
    case live
    case stored
}

struct ProviderDirectorySnapshot: Equatable {
    let source: ProviderDirectorySource
    let providers: [DirectoryProvider]
    var observation: ProviderDirectoryObservation
    let blockNumber: UInt64?
    let readAt: Date
    /// `nil` means no cache wrapper set a deadline. It does NOT mean the facts are permanent.
    var expiresAt: Date?
}

enum ProviderDirectoryUnavailableReason: Equatable {
    case sourceUnavailable
    case malformedResponse
    case providerRegistryUnavailable
    case inconsistentSource
    case invalidSnapshot
}

struct ProviderDirectoryUnavailable: Equatable {
    let source: ProviderDirectorySource
    let reason: ProviderDirectoryUnavailableReason
    let detail: String
    let attemptedAt: Date
}

/// Exactly one of a successful non-empty read, successful empty read, or a read that did not answer.
/// `unavailable` deliberately carries no provider array.
enum ProviderDirectoryResult: Equatable {
    case found(ProviderDirectorySnapshot)
    case empty(ProviderDirectorySnapshot)
    case unavailable(ProviderDirectoryUnavailable)
}

/// The read seam consumed by Nearby. It accepts no query, leaving nowhere for a user position.
protocol ProviderDirectoryReading {
    var source: ProviderDirectorySource { get }
    /// Stable identity of the configured source, used ONLY to scope a stored copy.
    ///
    /// It must distinguish two distinct configured endpoints and two future chain/registry
    /// configurations. It must never contain a user position or anything derived from one.
    var cacheNamespace: String { get }
    func read() async -> ProviderDirectoryResult
}

// MARK: - The on-device local copy

/// Persistence seam. All policy lives above it, so the whole decision is covered by the host-less
/// `DogTagTests` target while the file I/O stays a thin uncovered edge.
protocol ProviderDirectoryCacheStore {
    /// The stored document, or `nil` when nothing is stored or it could not be read.
    func read() -> Data?
    func write(_ document: Data)
    func clear()
}

/// In-memory store for tests and for a caller that deliberately wants no disk copy.
final class MemoryProviderDirectoryCacheStore: ProviderDirectoryCacheStore {
    private var document: Data?
    init() {}
    func read() -> Data? { document }
    func write(_ document: Data) { self.document = document }
    func clear() { document = nil }
}

/// A stored snapshot plus the identity needed to decide whether it may be replayed at all.
struct ProviderDirectoryCacheEntry: Equatable {
    let namespace: String
    let snapshot: ProviderDirectorySnapshot
    let readAt: Date
    let expiresAt: Date
}

/// The integrity a snapshot must have to be written OR replayed.
///
/// Running it on the replay path is the load-bearing half. TypeScript makes a non-empty `found`
/// unrepresentable (`readonly [P, ...P[]]`); a Swift `[DirectoryProvider]` cannot, so a `found`
/// carrying zero providers becomes possible the moment a snapshot arrives from disk rather than from
/// the live path - and it renders as "no vets near you", the exact false absence this exists to
/// prevent. The guard is the only thing standing there.
func providerDirectorySnapshotIsWellFormed(_ result: ProviderDirectoryResult) -> Bool {
    switch result {
    case .found(let snapshot):
        return snapshot.readAt.timeIntervalSince1970.isFinite && !snapshot.providers.isEmpty
    case .empty(let snapshot):
        return snapshot.readAt.timeIntervalSince1970.isFinite && snapshot.providers.isEmpty
    // `unavailable` is not a snapshot. It is never stored and never replayed.
    case .unavailable:
        return false
    }
}

/// Re-check the directory, replaying the last successful snapshot only when the live read could not
/// answer and the hard TTL has not elapsed.
///
/// This is the Swift port of `packages/ui/src/directory/cache.ts`, and its semantics are that file's,
/// not new ones. A successful LIVE read replaces the stored copy. A stored replay never renews its
/// hard deadline. An entry AT its exact expiry is expired. Missing, wrong-namespace, and expired
/// entries all leave the live `unavailable` exactly as it was; none may turn it into `empty`.
///
/// What it buys over the in-process snapshot it replaces: it survives the process. A cache held in a
/// static is empty on every cold launch, which is exactly the state a phone is in when the owner
/// opens the app somewhere with no signal.
///
/// There is deliberately no `catch` around the delegate read, unlike the Kotlin twin. `read()` is a
/// non-throwing `async` function, so an unexpected error cannot escape it: the compiler enforces
/// here what Android has to enforce at runtime. A catch would be unreachable code masquerading as a
/// safeguard.
struct CachedProviderDirectory: ProviderDirectoryReading {
    static let defaultTtl: TimeInterval = 15 * 60

    private let delegate: ProviderDirectoryReading
    private let store: ProviderDirectoryCacheStore
    private let ttl: TimeInterval
    private let now: () -> Date

    var source: ProviderDirectorySource { delegate.source }
    var cacheNamespace: String { delegate.cacheNamespace }

    init(
        delegate: ProviderDirectoryReading,
        store: ProviderDirectoryCacheStore,
        ttl: TimeInterval = CachedProviderDirectory.defaultTtl,
        now: @escaping () -> Date = Date.init
    ) {
        self.delegate = delegate
        self.store = store
        self.ttl = ttl
        self.now = now
    }

    func read() async -> ProviderDirectoryResult {
        let live = await delegate.read()
        let currentTime = now()

        guard ttl.isFinite, ttl > 0 else {
            // A misconfigured lifetime disables the copy rather than storing an entry whose deadline
            // no later read could evaluate.
            store.clear()
            return live
        }

        if case .unavailable = live { return replay(live: live, currentTime: currentTime) }

        // A source that identifies itself differently from the one configured is a trust-boundary
        // change, not a cache hit.
        if snapshotSource(live) != delegate.source {
            store.clear()
            return .unavailable(ProviderDirectoryUnavailable(
                source: delegate.source,
                reason: .inconsistentSource,
                detail: "The directory identified itself as a different source than the one configured",
                attemptedAt: currentTime
            ))
        }

        guard providerDirectorySnapshotIsWellFormed(live), let snapshot = successfulSnapshot(live) else {
            store.clear()
            return .unavailable(ProviderDirectoryUnavailable(
                source: delegate.source,
                reason: .invalidSnapshot,
                detail: "The directory returned an invalid provider list or timestamp",
                attemptedAt: currentTime
            ))
        }

        if snapshot.observation == .stored {
            // A replay handed up by an inner wrapper is not a successful refresh. Passing it through
            // preserves the original hard deadline; storing it here would let stacked wrappers renew
            // stale data forever.
            guard let inherited = snapshot.expiresAt, currentTime < inherited else {
                return .unavailable(ProviderDirectoryUnavailable(
                    source: delegate.source,
                    reason: .invalidSnapshot,
                    detail: "The directory returned a stored snapshot without an unexpired hard TTL",
                    attemptedAt: currentTime
                ))
            }
            return live
        }

        // The TTL runs from the SOURCE's observation, not from insertion, so a slow read or an outer
        // wrapper can never silently lengthen the hard maximum age.
        let localExpiry = snapshot.readAt.addingTimeInterval(ttl)
        let expiresAt = min(localExpiry, snapshot.expiresAt ?? localExpiry)
        guard currentTime < expiresAt else {
            store.clear()
            return .unavailable(ProviderDirectoryUnavailable(
                source: delegate.source,
                reason: .invalidSnapshot,
                detail: "The live directory snapshot was already outside its hard TTL",
                attemptedAt: currentTime
            ))
        }

        var stamped = snapshot
        stamped.expiresAt = expiresAt
        let entry = ProviderDirectoryCacheEntry(
            namespace: delegate.cacheNamespace,
            snapshot: stamped,
            readAt: snapshot.readAt,
            expiresAt: expiresAt
        )
        if let document = ProviderDirectoryCacheCodec.encode(entry) { store.write(document) }
        return stamped.providers.isEmpty ? .empty(stamped) : .found(stamped)
    }

    private func replay(live: ProviderDirectoryResult, currentTime: Date) -> ProviderDirectoryResult {
        guard let document = store.read() else { return live }
        guard let entry = ProviderDirectoryCacheCodec.decode(document),
              entry.namespace == delegate.cacheNamespace,
              entry.snapshot.source == delegate.source else {
            store.clear()
            return live
        }
        let replayed: ProviderDirectoryResult =
            entry.snapshot.providers.isEmpty ? .empty(entry.snapshot) : .found(entry.snapshot)
        guard providerDirectorySnapshotIsWellFormed(replayed),
              entry.snapshot.readAt == entry.readAt,
              entry.snapshot.expiresAt == entry.expiresAt,
              // A snapshot read in the future means the clock moved backwards. Trusting the stored
              // absolute deadline there would extend the hard window by however far it jumped.
              entry.readAt <= currentTime,
              entry.expiresAt <= entry.readAt.addingTimeInterval(ttl),
              currentTime < entry.expiresAt else {
            store.clear()
            return live
        }
        var stored = entry.snapshot
        stored.observation = .stored
        return stored.providers.isEmpty ? .empty(stored) : .found(stored)
    }

    private func snapshotSource(_ result: ProviderDirectoryResult) -> ProviderDirectorySource {
        switch result {
        case .found(let snapshot), .empty(let snapshot): return snapshot.source
        case .unavailable(let unavailable): return unavailable.source
        }
    }

    private func successfulSnapshot(_ result: ProviderDirectoryResult) -> ProviderDirectorySnapshot? {
        switch result {
        case .found(let snapshot), .empty(let snapshot): return snapshot
        case .unavailable: return nil
        }
    }
}

/// The stored document's wire form.
///
/// Everything decodes strictly and any failure is `nil`, which the caller reads as "nothing stored".
/// A cache that guessed at a half-understood document would be inventing directory content.
enum ProviderDirectoryCacheCodec {
    /// Bump on ANY change to the stored shape, including a change to what a field means.
    ///
    /// A stale-version document is dropped rather than migrated. The concrete reason this exists from
    /// day one: S-1 made `geo` optional, and before it a location-less provider was persisted as the
    /// real coordinate `0,0`. AGENTS.md is explicit that such a row cannot be safely reinterpreted by
    /// code, so a stored copy from an older shape must never be replayed as if it were this one.
    static let version = 1

    private struct StoredPoint: Codable, Equatable {
        let lat: Double
        let lng: Double
    }

    private struct StoredContact: Codable, Equatable {
        var phone: String?
        var whatsapp: String?
        var telegram: String?
        var email: String?
        var website: String?
    }

    private struct StoredProvider: Codable, Equatable {
        let providerId: String
        let kind: String
        let name: String
        /// Absence is stored as absence. It is never written as `0,0`, a real coordinate off the
        /// coast of Ghana that would place a contact-only provider on the map.
        let geo: StoredPoint?
        let services: [String]
        let domain: String?
        let active: Bool?
        let contact: StoredContact
        /// Only the two states a directory source may honestly produce. A stored `verified` would be
        /// a claim about a DNS/chain check nobody performed, so it can be neither written nor read.
        let bindingState: String
    }

    private struct StoredEntry: Codable, Equatable {
        let version: Int
        let namespace: String
        /// Seconds since the epoch. Absolute, so a replay cannot slide its own deadline forward.
        let readAt: Double
        let expiresAt: Double
        /// `"found"` or `"empty"`. `observation` is deliberately NOT stored: a document read back
        /// from disk is a replay by definition, and a persisted `"live"` could present a remembered
        /// answer as a fresh one.
        let state: String
        let providers: [StoredProvider]
    }

    static func encode(_ entry: ProviderDirectoryCacheEntry) -> Data? {
        // Only the central source has a stored shape today. A future on-chain directory must extend
        // this explicitly rather than being silently written under central's identity.
        guard entry.snapshot.source == .central else { return nil }
        let stored = StoredEntry(
            version: version,
            namespace: entry.namespace,
            readAt: entry.readAt.timeIntervalSince1970,
            expiresAt: entry.expiresAt.timeIntervalSince1970,
            state: entry.snapshot.providers.isEmpty ? "empty" : "found",
            providers: entry.snapshot.providers.map { provider in
                StoredProvider(
                    providerId: provider.providerId,
                    kind: provider.kind,
                    name: provider.name,
                    geo: provider.geo.map { StoredPoint(lat: $0.lat, lng: $0.lng) },
                    services: provider.services,
                    domain: provider.domain,
                    active: provider.active,
                    contact: StoredContact(
                        phone: provider.contact.phone,
                        whatsapp: provider.contact.whatsapp,
                        telegram: provider.contact.telegram,
                        email: provider.contact.email,
                        website: provider.contact.website
                    ),
                    bindingState: provider.bindingState == .noDomainListed
                        ? "noDomainListed"
                        : "unavailable"
                )
            }
        )
        return try? JSONEncoder().encode(stored)
    }

    static func decode(_ document: Data) -> ProviderDirectoryCacheEntry? {
        guard let stored = try? JSONDecoder().decode(StoredEntry.self, from: document),
              stored.version == version,
              !stored.namespace.isEmpty,
              stored.readAt.isFinite, stored.expiresAt.isFinite,
              stored.state == "found" || stored.state == "empty" else { return nil }
        if stored.state == "empty" && !stored.providers.isEmpty { return nil }
        if stored.state == "found" && stored.providers.isEmpty { return nil }

        var providers: [DirectoryProvider] = []
        providers.reserveCapacity(stored.providers.count)
        for row in stored.providers {
            var geo: NearbyPoint?
            if let point = row.geo {
                let candidate = NearbyPoint(lat: point.lat, lng: point.lng)
                guard candidate.isValid else { return nil }
                geo = candidate
            }
            providers.append(DirectoryProvider(
                providerId: row.providerId,
                kind: row.kind,
                name: row.name,
                geo: geo,
                services: row.services,
                domain: row.domain,
                active: row.active,
                contact: ProviderContact(
                    phone: row.contact.phone,
                    whatsapp: row.contact.whatsapp,
                    telegram: row.contact.telegram,
                    email: row.contact.email,
                    website: row.contact.website
                ),
                bindingState: row.bindingState == "noDomainListed" ? .noDomainListed : .unavailable
            ))
        }

        let readAt = Date(timeIntervalSince1970: stored.readAt)
        let expiresAt = Date(timeIntervalSince1970: stored.expiresAt)
        return ProviderDirectoryCacheEntry(
            namespace: stored.namespace,
            snapshot: ProviderDirectorySnapshot(
                source: .central,
                providers: providers,
                // Relabelled by the wrapper; a document off disk is a replay whatever it claims.
                observation: .stored,
                blockNumber: nil,
                readAt: readAt,
                expiresAt: expiresAt
            ),
            readAt: readAt,
            expiresAt: expiresAt
        )
    }
}

enum NearbyOriginSource: Equatable {
    case currentLocation
    case chosenCoordinates
}

/// Where distance is measured from, and how precise that origin actually is.
///
/// `accuracyMetres` is the horizontal uncertainty the DEVICE reported for a `currentLocation` fix,
/// carried so no row can render a distance finer than the fix supports. It is meaningless for
/// `chosenCoordinates`, which the owner typed exactly, so that source keeps ordinary precision.
struct NearbyOrigin: Equatable {
    let point: NearbyPoint
    let source: NearbyOriginSource
    var accuracyMetres: Double?

    static func chosen(_ point: NearbyPoint) -> NearbyOrigin {
        NearbyOrigin(point: point, source: .chosenCoordinates, accuracyMetres: nil)
    }
}

enum NearbyLocationState: Equatable {
    case notRequested
    case locating
    case ready(NearbyOrigin)
    case refused
    case unavailable
    case invalidChosenLocation
}

/// What may be SAID about one measured distance, given how precise the origin actually is.
///
/// A device fix carries real uncertainty, so the rendered number must not be finer than the fix
/// supports, and when nothing numeric is supportable the surface shows `uncertain` rather than an
/// arbitrary confident figure.
enum DistanceClaim: Equatable {
    /// `approximate` is true for every device fix, so the row can mark the number as such.
    case measured(label: String, approximate: Bool)
    /// No distance may be stated at all; `reason` is shown in place of a number.
    case uncertain(reason: String)

    /// What the row actually prints, `nil` when there is no number to print. Composed here, not at
    /// each call site, so the two apps cannot disagree — and so a label that is already a bound is
    /// not double-marked as `~< 150 m`.
    var display: String? {
        guard case .measured(let label, let approximate) = self else { return nil }
        return approximate && !label.hasPrefix("<") ? "~\(label)" : label
    }
}

enum NearbyUnitSystem: Equatable {
    case metric
    case imperial

    static func forRegion(_ raw: String?) -> NearbyUnitSystem {
        guard let raw else { return .metric }
        let parts = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            .split(whereSeparator: { $0 == "-" || $0 == "_" })
            .map(String.init)
        let imperialRegions = Set(["US", "GB", "LR", "MM"])
        if parts.count == 1, let only = parts.first,
           imperialRegions.contains(only.uppercased()) {
            return .imperial
        }
        for part in parts.dropFirst()
        where part.count == 2 && part.allSatisfy(\.isLetter) {
            if imperialRegions.contains(part.uppercased()) {
                return .imperial
            }
        }
        return .metric
    }
}

/// The shared, pure policy for what Nearby may show and claim.
///
/// Distance measurement is injected so the app can use Core Location while this file remains
/// Foundation-only and runnable in the host-less DogTagTests target.
enum NearbyDecision {
    static let defaultRadiusKm = 50.0

    /// Offline chosen-origin parser shared by the UI and tests. There is deliberately no geocoder:
    /// turning an address into coordinates would transmit the chosen location to a search service.
    static func parseChosenOrigin(lat: String, lng: String) -> NearbyPoint? {
        guard let latitude = Double(lat.trimmingCharacters(in: .whitespacesAndNewlines)),
              let longitude = Double(lng.trimmingCharacters(in: .whitespacesAndNewlines)) else {
            return nil
        }
        let point = NearbyPoint(lat: latitude, lng: longitude)
        return point.isValid ? point : nil
    }

    struct Row: Equatable {
        let provider: DirectoryProvider
        /// The RAW platform measurement. Ordering uses it; only `distance` is coarsened for display.
        let distanceKm: Double
        let distance: DistanceClaim
        let bearingLabel: String?

        /// A row reaches this type only after a real, valid coordinate and distance were established.
        var allowsDirections: Bool { true }
    }

    /// Complete Nearby presentation. Keeping rows inside only the `providersFound` case makes an
    /// unavailable directory impossible to accidentally render as an empty provider list.
    enum Presentation: Equatable {
        case loadingDirectory
        case directoryUnavailable(String)
        case directoryEmpty(ProviderDirectoryObservation)
        case awaitingOrigin
        case locating
        case permissionRefused
        case locationUnavailable
        case invalidChosenLocation
        case noneWithinRange(radiusKm: Double, observation: ProviderDirectoryObservation)
        case noNameMatch(query: String, observation: ProviderDirectoryObservation)
        case providersFound(rows: [Row], observation: ProviderDirectoryObservation)
    }

    /// Complete unranked contact presentation. Located and contact-only providers share the found
    /// case; this state intentionally carries no distance, bearing or Directions claim.
    enum ContactDirectoryPresentation: Equatable {
        case loadingDirectory
        case directoryUnavailable(String)
        case directoryEmpty(ProviderDirectoryObservation)
        case noNameMatch(query: String, observation: ProviderDirectoryObservation)
        case providersFound(providers: [DirectoryProvider], observation: ProviderDirectoryObservation)
    }

    typealias DistanceKm = (_ origin: NearbyPoint, _ destination: NearbyPoint) -> Double?

    static func presentation(
        directory: ProviderDirectoryResult?,
        location: NearbyLocationState,
        query: String,
        unitSystem: NearbyUnitSystem,
        radiusKm: Double = defaultRadiusKm,
        distanceKm: DistanceKm
    ) -> Presentation {
        guard let directory else { return .loadingDirectory }

        switch directory {
        case .unavailable(let failure):
            return .directoryUnavailable(failure.detail)
        case .empty(let snapshot):
            return .directoryEmpty(snapshot.observation)
        case .found(let snapshot):
            let usableRadius = radiusKm.isFinite && radiusKm >= 0
                ? radiusKm
                : defaultRadiusKm
            let eligible = nearbyEligible(snapshot.providers)
            let term = normalized(query)
            let searchedName = query.trimmingCharacters(in: .whitespacesAndNewlines)
            let candidates = term.isEmpty
                ? eligible
                : eligible.filter { normalized($0.name).contains(term) }

            if !term.isEmpty && candidates.isEmpty {
                return .noNameMatch(query: searchedName, observation: snapshot.observation)
            }
            if candidates.isEmpty {
                return .noneWithinRange(
                    radiusKm: usableRadius,
                    observation: snapshot.observation
                )
            }

            let resolved: NearbyOrigin
            switch location {
            case .notRequested:
                return .awaitingOrigin
            case .locating:
                return .locating
            case .refused:
                return .permissionRefused
            case .unavailable:
                return .locationUnavailable
            case .invalidChosenLocation:
                return .invalidChosenLocation
            case .ready(let candidate):
                guard candidate.point.isValid else {
                    return .locationUnavailable
                }
                resolved = candidate
            }

            let origin = resolved.point
            let fromDeviceFix = resolved.source == .currentLocation
            let ranked = candidates.enumerated().compactMap { index, provider -> (Int, Row)? in
                guard let destination = provider.geo, destination.isValid,
                      let km = distanceKm(origin, destination),
                      km.isFinite, km >= 0 else {
                    return nil
                }
                return (
                    index,
                    Row(
                        provider: provider,
                        distanceKm: km,
                        distance: distanceClaim(
                            km,
                            accuracyMetres: resolved.accuracyMetres,
                            fromDeviceFix: fromDeviceFix,
                            unitSystem: unitSystem
                        ),
                        bearingLabel: initialBearing(origin, destination).flatMap(compassPoint8)
                    )
                )
            }
            .filter { !term.isEmpty || $0.1.distanceKm <= usableRadius }
            .sorted {
                if $0.1.distanceKm == $1.1.distanceKm { return $0.0 < $1.0 }
                return $0.1.distanceKm < $1.1.distanceKm
            }
            .map(\.1)

            if !ranked.isEmpty {
                return .providersFound(rows: ranked, observation: snapshot.observation)
            }
            return term.isEmpty
                ? .noneWithinRange(radiusKm: usableRadius, observation: snapshot.observation)
                : .noNameMatch(query: searchedName, observation: snapshot.observation)
        }
    }

    /// The separate, unranked provider-contact directory. Located and contact-only providers both
    /// remain reachable here; this scope never acquires an origin and never offers Directions.
    static func contactPresentation(
        directory: ProviderDirectoryResult?,
        query: String
    ) -> ContactDirectoryPresentation {
        guard let directory else { return .loadingDirectory }

        switch directory {
        case .unavailable(let failure):
            return .directoryUnavailable(failure.detail)
        case .empty(let snapshot):
            return .directoryEmpty(snapshot.observation)
        case .found(let snapshot):
            let term = normalized(query)
            let searchedName = query.trimmingCharacters(in: .whitespacesAndNewlines)
            let providers = snapshot.providers
                .filter(isEligibleProvider)
                .filter { term.isEmpty || normalized($0.name).contains(term) }
                .sorted {
                    let first = sortKey($0.name)
                    let second = sortKey($1.name)
                    return first == second
                        ? $0.providerId < $1.providerId
                        : first < second
                }
            if !providers.isEmpty {
                return .providersFound(
                    providers: providers,
                    observation: snapshot.observation
                )
            }
            return term.isEmpty
                ? .directoryEmpty(snapshot.observation)
                : .noNameMatch(query: searchedName, observation: snapshot.observation)
        }
    }

    private static func nearbyEligible(_ providers: [DirectoryProvider]) -> [DirectoryProvider] {
        providers.filter(isEligibleProvider).filter { $0.geo?.isValid == true }
    }

    private static func isEligibleProvider(_ provider: DirectoryProvider) -> Bool {
        let kind = provider.kind.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return (kind == "vet" || kind == "groomer") && provider.active != false
    }

    private static func normalized(_ value: String) -> String {
        value.trimmingCharacters(in: .whitespacesAndNewlines)
            .folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "en_US_POSIX"))
            .lowercased()
    }

    /// The contact list's ordering key, mirrored by Android.
    ///
    /// It is the SAME locale-independent fold the on-device name search uses. A collating comparator
    /// would order the two apps differently for the same directory, because each platform's collation
    /// is its own; folding first leaves one order that both can reproduce.
    private static func sortKey(_ value: String) -> String { normalized(value) }

    private static func initialBearing(_ origin: NearbyPoint, _ destination: NearbyPoint) -> Double? {
        guard origin.isValid, destination.isValid else { return nil }
        if origin == destination || abs(origin.lat) == 90 { return nil }

        let phi1 = origin.lat * .pi / 180
        let phi2 = destination.lat * .pi / 180
        let deltaLambda = (destination.lng - origin.lng) * .pi / 180
        let y = sin(deltaLambda) * cos(phi2)
        let x = cos(phi1) * sin(phi2) - sin(phi1) * cos(phi2) * cos(deltaLambda)
        if y == 0 && x == 0 { return nil }
        let degrees = atan2(y, x) * 180 / .pi
        return (degrees + 360).truncatingRemainder(dividingBy: 360)
    }

    private static func compassPoint8(_ degrees: Double) -> String? {
        guard degrees.isFinite else { return nil }
        let points = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"]
        let normalized = (degrees.truncatingRemainder(dividingBy: 360) + 360)
            .truncatingRemainder(dividingBy: 360)
        return points[Int((normalized / 45).rounded()) % points.count]
    }

    private static let kmPerMile = 1.609344
    private static let feetPerKm = 1000 / 0.3048
    private static let distanceUnavailable = "This provider's distance could not be measured."
    private static let ceilTolerance = 1e-9

    // Each rung below is the metre width of one display band, derived from the same constant that
    // band's own formatting divides by. A value rounded to a rung therefore always lands on a
    // numeral its band can express, which is what keeps a positive distance from printing as zero.
    private static let footStepMetres = 25_000.0 / feetPerKm
    private static let tenthMileMetres = kmPerMile * 100
    private static let mileMetres = kmPerMile * 1_000
    private static let tenMetreStepMetres = 10.0
    private static let tenthKilometreMetres = 100.0
    private static let kilometreStepMetres = 1_000.0

    /// The coarsest fix that can still place a provider at all. Beyond it, no number is claimed.
    private static let coarsestMetricMetres = 10 * kilometreStepMetres
    private static let coarsestImperialMetres = 10 * mileMetres

    /// Rounding steps a device fix may be shown at. The chosen step is never finer than the fix.
    private static let metricLadderMetres = [
        tenMetreStepMetres, tenthKilometreMetres, kilometreStepMetres, coarsestMetricMetres,
    ]
    private static let imperialLadderMetres = [
        footStepMetres, tenthMileMetres, mileMetres, coarsestImperialMetres,
    ]

    /// [minGranularityMetres] raises the display floor so a band finer than the origin's own
    /// uncertainty is never reached. It can only make a label COARSER; coarser is always honest,
    /// finer never is. The default 0 is the exact coordinate-arithmetic path, unchanged.
    static func formatDistanceKm(
        _ km: Double?,
        unitSystem: NearbyUnitSystem,
        minGranularityMetres: Double = 0
    ) -> String? {
        guard let km, km.isFinite, km >= 0 else { return nil }
        if unitSystem == .imperial {
            let miles = km / kmPerMile
            if minGranularityMetres <= footStepMetres, miles < 0.1 {
                let feet = km * feetPerKm
                if feet < 25 { return "< 25 ft" }
                return "\(Int((feet / 25).rounded() * 25)) ft"
            }
            let oneDecimal = (miles * 10).rounded() / 10
            if minGranularityMetres <= tenthMileMetres, oneDecimal < 10 {
                return String(format: "%.1f mi", oneDecimal)
            }
            if minGranularityMetres <= coarsestImperialMetres {
                return "\(Int(miles.rounded())) mi"
            }
            return nil
        }

        if minGranularityMetres <= tenMetreStepMetres, km < 1 {
            let metres = km * 1000
            if metres < 10 { return "< 10 m" }
            let rounded = (metres / 10).rounded() * 10
            if rounded < 1000 { return "\(Int(rounded)) m" }
        }
        let oneDecimal = (km * 10).rounded() / 10
        if minGranularityMetres <= tenthKilometreMetres, oneDecimal < 10 {
            return String(format: "%.1f km", oneDecimal)
        }
        if minGranularityMetres <= coarsestMetricMetres { return "\(Int(km.rounded())) km" }
        return nil
    }

    /// What this origin's precision permits the row to say about one measured distance.
    ///
    /// Chosen coordinates are exact arithmetic on numbers the owner typed, so they keep the ordinary
    /// bands. A device fix does not: its label is rounded to a step no finer than the reported
    /// horizontal accuracy, and a fix whose accuracy is missing, nonsensical, or coarser than any
    /// usable step yields no number at all.
    ///
    /// Anything nearer than the coarser of the accuracy and half that rounding step is stated as a
    /// BOUND. Half the step is the load-bearing half of that pair: below it the rounding collapses to
    /// zero, which the bands would then print as a confident `0 km`. The gate and the bound label are
    /// computed from the same value so they can never describe different distances.
    static func distanceClaim(
        _ km: Double?,
        accuracyMetres: Double?,
        fromDeviceFix: Bool,
        unitSystem: NearbyUnitSystem
    ) -> DistanceClaim {
        guard let km, km.isFinite, km >= 0 else {
            return .uncertain(reason: distanceUnavailable)
        }
        guard fromDeviceFix else {
            guard let label = formatDistanceKm(km, unitSystem: unitSystem) else {
                return .uncertain(reason: distanceUnavailable)
            }
            return .measured(label: label, approximate: false)
        }
        guard let accuracyMetres, accuracyMetres.isFinite, accuracyMetres >= 0 else {
            return .uncertain(
                reason: "This location fix reported no usable accuracy, so no distance is claimed."
            )
        }
        let ladder = unitSystem == .imperial ? imperialLadderMetres : metricLadderMetres
        guard let step = ladder.first(where: { $0 >= accuracyMetres }) else {
            return .uncertain(
                reason: "This location fix is accurate only to "
                    + "\(uncertaintyLabel(accuracyMetres, unitSystem: unitSystem)), "
                    + "which is too coarse to state a distance."
            )
        }
        let metres = km * 1000
        let bound = max(accuracyMetres, step / 2)
        if metres <= bound {
            // Nearer than the evidence can resolve is still something the fix DOES establish. State
            // that bound rather than a point value, and rather than withholding an answer we have.
            return .measured(
                label: "< \(uncertaintyLabel(bound, unitSystem: unitSystem))",
                approximate: true
            )
        }
        let rounded = (metres / step).rounded() * step / 1000
        guard let label = formatDistanceKm(
            rounded,
            unitSystem: unitSystem,
            minGranularityMetres: accuracyMetres
        ) else {
            return .uncertain(reason: distanceUnavailable)
        }
        return .measured(label: label, approximate: true)
    }

    /// How the fix's own uncertainty is written, e.g. `±40 m`. Never finer than the value itself.
    static func accuracyNote(
        _ accuracyMetres: Double?,
        unitSystem: NearbyUnitSystem
    ) -> String? {
        guard let accuracyMetres, accuracyMetres.isFinite, accuracyMetres >= 0 else { return nil }
        return "±\(uncertaintyLabel(accuracyMetres, unitSystem: unitSystem))"
    }

    /// A distance that IS the uncertainty. Rendered from the value itself so its own granularity can
    /// never over-claim, unlike running it through the measured-distance bands.
    ///
    /// It rounds OUTWARD, and every caller is why: a `< bound`, a `±error` and an "accurate only to"
    /// all state a ceiling, so a label one display step below its own value understates exactly what
    /// it exists to disclose. Rounding to nearest put a provider measured at 92 m behind `< 90 m`.
    ///
    /// The branch is chosen from the ROUNDED value, not the raw one, so a bound that rounds up to a
    /// whole kilometre reads `1.0 km` rather than `1000 m`.
    private static func uncertaintyLabel(
        _ metres: Double,
        unitSystem: NearbyUnitSystem
    ) -> String {
        if unitSystem == .imperial {
            let feet = max(ceilTo(metres * feetPerKm / 1000, 25), 25)
            if feet < 1000 {
                return "\(Int(feet)) ft"
            }
            return String(
                format: "%.1f mi",
                ceilTo(metres, tenthMileMetres) / 1000 / kmPerMile
            )
        }
        let rounded = max(ceilTo(metres, tenMetreStepMetres), tenMetreStepMetres)
        if rounded < 1000 {
            return "\(Int(rounded)) m"
        }
        return String(format: "%.1f km", ceilTo(metres, tenthKilometreMetres) / 1000)
    }

    /// Rounds up to a whole number of `step`s, tolerating a value that already IS a multiple of a
    /// step no double can hold exactly. `mileMetres` is `kmPerMile * 1_000`, which is not exactly
    /// 1609.344, so without that tolerance a whole mile would bump itself to `1.1 mi`. The tolerance
    /// is expressed in steps, so it can understate by at most a billionth of one - sub-micron here.
    private static func ceilTo(_ value: Double, _ step: Double) -> Double {
        (value / step - ceilTolerance).rounded(.up) * step
    }
}
