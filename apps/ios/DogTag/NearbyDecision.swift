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

    /// The coordinate shape the owner app may send to provider discovery.
    ///
    /// Captain's ruling, 2026-07-30: the EXACT fix is sent and is NOT rounded. This previously rounded
    /// to three decimals; nothing may reintroduce that silently, and nothing may name a full-precision
    /// value "approximate", which would overstate the privacy the request provides. What guards the
    /// position now is confinement - body-only, never logged, never stored - not imprecision.
    ///
    /// The check that remains is validity, so an unusable fix cannot reach the wire.
    func validatedForProviderSearch() -> NearbyPoint? { isValid ? self : nil }
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
    /// Server-computed distance for a nearest request. Name/contact pages legitimately carry `nil`.
    let distanceKm: Double?

    var id: String { providerId }

    init(
        providerId: String,
        kind: String,
        name: String,
        geo: NearbyPoint?,
        services: [String],
        domain: String?,
        active: Bool?,
        contact: ProviderContact,
        bindingState: IssuerBindingState,
        distanceKm: Double? = nil
    ) {
        self.providerId = providerId
        self.kind = kind
        self.name = name
        self.geo = geo
        self.services = services
        self.domain = domain
        self.active = active
        self.contact = contact
        self.bindingState = bindingState
        self.distanceKm = distanceKm
    }
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
    let expiresAt: Date?
    let page: ProviderDirectoryPage?

    init(
        source: ProviderDirectorySource,
        providers: [DirectoryProvider],
        observation: ProviderDirectoryObservation,
        blockNumber: UInt64?,
        readAt: Date,
        expiresAt: Date?,
        page: ProviderDirectoryPage? = nil
    ) {
        self.source = source
        self.providers = providers
        self.observation = observation
        self.blockNumber = blockNumber
        self.readAt = readAt
        self.expiresAt = expiresAt
        self.page = page
    }
}

struct ProviderDirectoryPage: Equatable {
    let total: Int
    let limit: Int
    let offset: Int
    let hasMore: Bool
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

/// Pure append policy for independently fetched directory pages.
///
/// The next page must describe the same result set and begin exactly after the rows already loaded.
/// Without all three checks, a changing or malformed server response could silently skip, repeat, or
/// splice providers while the UI still labels the result as one coherent list.
enum ProviderDirectoryPaging {
    static func merge(
        current: ProviderDirectoryResult?,
        incoming: ProviderDirectoryResult,
        reset: Bool,
        requiresDistance: Bool,
        attemptedAt: Date
    ) -> ProviderDirectoryResult {
        guard !reset, let existing = successfulSnapshot(current) else { return incoming }
        guard let next = successfulSnapshot(incoming) else { return incoming }
        guard let existingPage = existing.page, let nextPage = next.page else {
            return malformed(
                source: next.source,
                detail: "The provider directory omitted paging metadata",
                attemptedAt: attemptedAt
            )
        }
        guard nextPage.offset == existing.providers.count else {
            return malformed(
                source: next.source,
                detail: "The provider directory returned a non-contiguous page offset",
                attemptedAt: attemptedAt
            )
        }
        guard nextPage.limit == existingPage.limit else {
            return malformed(
                source: next.source,
                detail: "The provider directory changed the page limit mid-list",
                attemptedAt: attemptedAt
            )
        }
        guard nextPage.total == existingPage.total else {
            return malformed(
                source: next.source,
                detail: "The provider directory changed the result total mid-list",
                attemptedAt: attemptedAt
            )
        }

        let oldIDs = Set(existing.providers.map(\.providerId))
        guard next.providers.allSatisfy({ !oldIDs.contains($0.providerId) }) else {
            return malformed(
                source: next.source,
                detail: "The provider directory repeated a row across pages",
                attemptedAt: attemptedAt
            )
        }
        if requiresDistance {
            let distanceIsValid = { (provider: DirectoryProvider) in
                provider.distanceKm.map { $0.isFinite && $0 >= 0 } == true
            }
            guard existing.providers.allSatisfy(distanceIsValid),
                  next.providers.allSatisfy(distanceIsValid) else {
                return malformed(
                    source: next.source,
                    detail: "A nearest directory page omitted a usable server distance",
                    attemptedAt: attemptedAt
                )
            }
            if let previous = existing.providers.last?.distanceKm,
               let first = next.providers.first?.distanceKm,
               first < previous {
                return malformed(
                    source: next.source,
                    detail: "The provider directory returned pages out of distance order",
                    attemptedAt: attemptedAt
                )
            }
        }

        let providers = existing.providers + next.providers
        let merged = ProviderDirectorySnapshot(
            source: next.source,
            providers: providers,
            observation: next.observation,
            blockNumber: next.blockNumber,
            readAt: next.readAt,
            expiresAt: nil,
            page: ProviderDirectoryPage(
                total: nextPage.total,
                limit: nextPage.limit,
                offset: 0,
                hasMore: nextPage.hasMore
            )
        )
        return providers.isEmpty ? .empty(merged) : .found(merged)
    }

    private static func successfulSnapshot(
        _ result: ProviderDirectoryResult?
    ) -> ProviderDirectorySnapshot? {
        switch result {
        case .found(let snapshot), .empty(let snapshot):
            return snapshot
        case .unavailable, .none:
            return nil
        }
    }

    private static func malformed(
        source: ProviderDirectorySource,
        detail: String,
        attemptedAt: Date
    ) -> ProviderDirectoryResult {
        .unavailable(ProviderDirectoryUnavailable(
            source: source,
            reason: .inconsistentSource,
            detail: detail,
            attemptedAt: attemptedAt
        ))
    }
}

/// The owner app's complete request surface. It deliberately has no chosen-place, radius, map,
/// geocoder, viewport, or autocomplete member.
struct OwnerProviderDirectoryRequest: Equatable {
    enum Mode: Equatable {
        case nearest(location: NearbyPoint, accuracyMetres: Double?)
        case contacts
    }

    static let pageSize = 25
    static let kinds = ["vet", "groomer"]

    let mode: Mode
    let name: String?
    let offset: Int

    var isNearest: Bool {
        if case .nearest = mode { return true }
        return false
    }

    static func nearest(
        location: NearbyPoint,
        accuracyMetres: Double?,
        name: String?,
        offset: Int
    ) -> OwnerProviderDirectoryRequest? {
        guard let validated = location.validatedForProviderSearch(), offset >= 0 else { return nil }
        return OwnerProviderDirectoryRequest(
            mode: .nearest(
                location: validated,
                accuracyMetres: accuracyMetres
            ),
            name: normalizedName(name),
            offset: offset
        )
    }

    static func contacts(name: String?, offset: Int) -> OwnerProviderDirectoryRequest? {
        guard offset >= 0 else { return nil }
        return OwnerProviderDirectoryRequest(
            mode: .contacts,
            name: normalizedName(name),
            offset: offset
        )
    }

    private static func normalizedName(_ name: String?) -> String? {
        let trimmed = name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }
}

/// The read seam consumed by Nearby. Every call is explicitly nearest or name/contact paging.
protocol ProviderDirectoryReading {
    var source: ProviderDirectorySource { get }
    /// Stable identity of the configured source, used ONLY to scope remembered records.
    ///
    /// It must distinguish two distinct configured endpoints, so one deployment's stored records can
    /// never be replayed as another's. It must never contain a position or anything derived from one.
    var cacheNamespace: String { get }
    func read(_ request: OwnerProviderDirectoryRequest) async -> ProviderDirectoryResult
}

// MARK: - The offline provider-record fallback

/// The on-device local copy of provider RECORDS - never of a ranking.
///
/// Captain's ruling, 2026-07-30: "its purpose is only for UX, so that some cache results can be shown
/// when the device is offline, keep the cache data to minimal, and minimal usage." So this is a
/// fallback that stops the screen being blank, not a feature, and it is deliberately narrower than the
/// full-set snapshot cache it replaces (slice S-3, PR #112).
///
/// What it stores: provider identity and published contacts for the providers the owner most recently
/// saw. What it must NEVER store: the nearest ordering, a distance, or anything else derived from the
/// owner's position - because that would both put the owner's whereabouts on disk and let a stale
/// ranking be replayed as though it were current, which a sorted list makes invisible.
///
/// Mirrors Android `DirectoryCache.kt` case for case; keep the two in step by hand.
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

/// Remembered records plus when they were remembered. There is deliberately no distance and no page
/// here: a caller cannot render a stored distance because it has none to render.
struct StoredProviderRecords: Equatable {
    let providers: [DirectoryProvider]
    let storedAt: Date
}

/// A stored record set with the identity needed to decide whether it may be replayed at all.
struct ProviderRecordCacheEntry: Equatable {
    let namespace: String
    let providers: [DirectoryProvider]
    let storedAt: Date
    let expiresAt: Date
}

/// The integrity a remembered record must have to be written OR replayed.
///
/// The replay half is load-bearing: a record arriving from disk was not produced by the live parser,
/// so a blank id or name is possible there in a way it is not on the live path - and a nameless row
/// renders as a list entry the owner cannot act on.
func providerRecordIsWellFormed(_ provider: DirectoryProvider) -> Bool {
    !provider.providerId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        && !provider.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
}

/// The position-free order. Applied on write AND on read, so the array order can never carry a
/// ranking. Ties break on `providerId` so the order is total.
func providerRecordsInStoredOrder(_ providers: [DirectoryProvider]) -> [DirectoryProvider] {
    providers.sorted { left, right in
        let l = left.name.lowercased(), r = right.name.lowercased()
        return l == r ? left.providerId < right.providerId : l < r
    }
}

/// Remember the records the owner just saw; replay them when a live read cannot answer at all.
///
/// Deliberately NOT a decorator around the directory seam, unlike the `CachedProviderDirectory` it
/// replaces. That shape wrapped a no-argument `read()` of the whole set, which no longer exists: a
/// nearest read is personalized and paged, so there is no single response that is "the directory" to
/// substitute for. The caller drives this explicitly instead, which also keeps recall off the success
/// path.
struct ProviderRecordCache {
    /// Bounds only the fallback - every read re-checks live first - so shortening it buys no freshness
    /// and only cuts an offline owner off sooner.
    static let defaultTtl: TimeInterval = 7 * 24 * 60 * 60

    /// The cap that makes "minimal" concrete: one page's worth. A cache that grew with the directory
    /// would reintroduce on disk exactly the bulk the server-side pivot removed from the wire.
    static let maxRecords = OwnerProviderDirectoryRequest.pageSize

    private let store: ProviderDirectoryCacheStore
    private let namespace: String
    private let ttl: TimeInterval

    init(
        store: ProviderDirectoryCacheStore,
        namespace: String,
        ttl: TimeInterval = ProviderRecordCache.defaultTtl
    ) {
        self.store = store
        self.namespace = namespace
        self.ttl = ttl
    }

    /// Replace the remembered set with what the owner is looking at now.
    ///
    /// Replace rather than accumulate, because "minimal" was the instruction twice over. The
    /// consequence is honest and worth stating: offline shows the last providers seen, not a history.
    ///
    /// An empty live page writes nothing and clears nothing - it means this query matched nothing,
    /// which is not evidence that the previously remembered providers ceased to exist.
    func remember(_ providers: [DirectoryProvider], now: Date) {
        var seen = Set<String>()
        let deduped = providers.filter { providerRecordIsWellFormed($0) && seen.insert($0.providerId).inserted }
        let records = Array(providerRecordsInStoredOrder(deduped).prefix(Self.maxRecords))
        guard !records.isEmpty else { return }
        let entry = ProviderRecordCacheEntry(
            namespace: namespace,
            providers: records,
            storedAt: now,
            expiresAt: now.addingTimeInterval(ttl)
        )
        guard let document = ProviderDirectoryCacheCodec.encode(entry) else { return }
        store.write(document)
    }

    /// The remembered records, or `nil` when there is nothing replayable.
    ///
    /// `nil` covers every could-not-answer: nothing stored, an undecodable document, a different
    /// configured source, an elapsed deadline, and a stored time in the future (a backwards clock
    /// jump, not a fresh copy). None of them may become an empty success.
    func recall(now: Date) -> StoredProviderRecords? {
        guard let document = store.read(),
              let entry = ProviderDirectoryCacheCodec.decode(document),
              entry.namespace == namespace,
              // At the exact deadline it is expired, matching Android and the web.
              now < entry.expiresAt,
              now >= entry.storedAt
        else { return nil }
        var seen = Set<String>()
        let deduped = entry.providers.filter {
            providerRecordIsWellFormed($0) && seen.insert($0.providerId).inserted
        }
        let records = Array(providerRecordsInStoredOrder(deduped).prefix(Self.maxRecords))
        guard !records.isEmpty else { return nil }
        return StoredProviderRecords(providers: records, storedAt: entry.storedAt)
    }

    func clear() { store.clear() }
}

enum ProviderDirectoryCacheCodec {
    /// Bump on ANY change to the stored shape, including a change to what a field means.
    ///
    /// Version 1 was slice S-3's full-set snapshot, whose `providers` array was in server order -
    /// which for a nearest response IS the ranking. Reading one under this shape would replay that
    /// ranking as a remembered record set, so the bump is what stops the previous build's document
    /// being reinterpreted as something it is not. A stale version is dropped, never migrated.
    static let version = 2

    static func encode(_ entry: ProviderRecordCacheEntry) -> Data? {
        let root: [String: Any] = [
            "version": version,
            "namespace": entry.namespace,
            "storedAt": entry.storedAt.timeIntervalSince1970,
            "expiresAt": entry.expiresAt.timeIntervalSince1970,
            "providers": entry.providers.map(encodeProvider),
        ]
        return try? JSONSerialization.data(withJSONObject: root)
    }

    static func decode(_ document: Data) -> ProviderRecordCacheEntry? {
        guard let root = (try? JSONSerialization.jsonObject(with: document)) as? [String: Any],
              (root["version"] as? Int) == version,
              let namespace = root["namespace"] as? String, !namespace.isEmpty,
              let storedAt = root["storedAt"] as? Double, storedAt.isFinite,
              let expiresAt = root["expiresAt"] as? Double, expiresAt.isFinite,
              let rows = root["providers"] as? [[String: Any]], !rows.isEmpty
        else { return nil }
        var providers: [DirectoryProvider] = []
        for row in rows {
            guard let provider = decodeProvider(row) else { return nil }
            providers.append(provider)
        }
        return ProviderRecordCacheEntry(
            namespace: namespace,
            providers: providers,
            storedAt: Date(timeIntervalSince1970: storedAt),
            expiresAt: Date(timeIntervalSince1970: expiresAt)
        )
    }

    /// Identity and published contacts only.
    ///
    /// There is deliberately no `distanceKm` key, and its absence is a requirement rather than an
    /// omission: `DirectoryProvider` DOES carry `distanceKm` on this platform, so dropping it here is
    /// an active decision on every write, not something the type happens to prevent.
    private static func encodeProvider(_ provider: DirectoryProvider) -> [String: Any] {
        var row: [String: Any] = [
            "providerId": provider.providerId,
            "kind": provider.kind,
            "name": provider.name,
            "services": provider.services,
            "bindingState": encodeBindingState(provider.bindingState),
        ]
        // Absence is stored as absence, never as the real coordinate `0,0`.
        if let geo = provider.geo { row["geo"] = ["lat": geo.lat, "lng": geo.lng] }
        if let domain = provider.domain { row["domain"] = domain }
        if let active = provider.active { row["active"] = active }
        var contact: [String: Any] = [:]
        if let value = provider.contact.phone { contact["phone"] = value }
        if let value = provider.contact.whatsapp { contact["whatsapp"] = value }
        if let value = provider.contact.telegram { contact["telegram"] = value }
        if let value = provider.contact.email { contact["email"] = value }
        if let value = provider.contact.website { contact["website"] = value }
        row["contact"] = contact
        return row
    }

    private static func decodeProvider(_ row: [String: Any]) -> DirectoryProvider? {
        guard let providerId = row["providerId"] as? String,
              let kind = row["kind"] as? String,
              let name = row["name"] as? String,
              let services = row["services"] as? [String]
        else { return nil }
        var geo: NearbyPoint?
        if let stored = row["geo"] {
            guard let object = stored as? [String: Any],
                  let lat = object["lat"] as? Double,
                  let lng = object["lng"] as? Double
            else { return nil }
            let point = NearbyPoint(lat: lat, lng: lng)
            guard point.isValid else { return nil }
            geo = point
        }
        let contact = row["contact"] as? [String: Any] ?? [:]
        func text(_ key: String) -> String? {
            guard let value = contact[key] as? String else { return nil }
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? nil : trimmed
        }
        return DirectoryProvider(
            providerId: providerId,
            kind: kind,
            name: name,
            geo: geo,
            services: services,
            domain: row["domain"] as? String,
            active: row["active"] as? Bool,
            contact: ProviderContact(
                phone: text("phone"),
                whatsapp: text("whatsapp"),
                telegram: text("telegram"),
                email: text("email"),
                website: text("website")
            ),
            bindingState: decodeBindingState(row["bindingState"] as? String ?? "")
            // distanceKm is deliberately left at its `nil` default: a remembered record asserts no
            // distance, so there is nothing here for a stale ranking to be rebuilt from.
        )
    }

    /// A stored `verified` would claim a DNS check nobody performed, so it cannot be written or read;
    /// anything unrecognised degrades to `unavailable`, which asserts nothing.
    private static func encodeBindingState(_ state: IssuerBindingState) -> String {
        if case .noDomainListed = state { return "noDomainListed" }
        return "unavailable"
    }

    private static func decodeBindingState(_ raw: String) -> IssuerBindingState {
        raw == "noDomainListed" ? .noDomainListed : .unavailable
    }
}

/// Where distance is measured from, and how precise that origin actually is.
///
/// `accuracyMetres` is the horizontal uncertainty the DEVICE reported for a `currentLocation` fix,
/// carried so no row can render a distance finer than the fix supports.
struct NearbyOrigin: Equatable {
    let point: NearbyPoint
    var accuracyMetres: Double?
}

enum NearbyLocationState: Equatable {
    case notRequested
    case locating
    case ready(NearbyOrigin)
    case refused
    case unavailable
}

/// What may be SAID about one measured distance, given how precise the origin actually is.
///
/// A device fix carries real uncertainty, so the rendered number must not be finer than the fix
/// supports, and when nothing numeric is supportable the surface shows `uncertain` rather than an
/// arbitrary confident figure.
enum DistanceClaim: Equatable {
    /// `approximate` is true for a device fix, whose own accuracy makes the number inexact.
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
/// The indexer owns distance calculation and ordering. This policy only validates and formats the
/// returned distance; it never recomputes or re-sorts it on the device.
enum NearbyDecision {
    /// The sentence shown immediately beside the action that starts the location permission. It lives
    /// here, in the pure layer, rather than as a literal in `NearbyScreen`, for the same reason
    /// `VerdictDisplay` does: this bundle can pin it, so the copy cannot quietly drift from Android's
    /// `NearbyDecision.LOCATION_DISCLOSURE` or be softened back toward a "never leaves this phone"
    /// claim that the server-side nearest search made untrue.
    static let locationDisclosure =
        "Your location is sent to DogTag to find nearby vets and groomers. It is not stored."

    struct Row: Equatable {
        let provider: DirectoryProvider
        /// The server measurement. The device preserves its order and only formats this value.
        let distanceKm: Double
        let distance: DistanceClaim
    }

    enum Presentation: Equatable {
        case loadingDirectory
        case directoryUnavailable(String)
        case directoryEmpty(ProviderDirectoryObservation)
        case awaitingOrigin
        case locating
        case permissionRefused
        case locationUnavailable
        case noNearbyProviders(ProviderDirectoryObservation)
        case noNameMatch(query: String, observation: ProviderDirectoryObservation)
        case providersFound(rows: [Row], observation: ProviderDirectoryObservation)

        /// The offline fallback: providers the owner has seen before, with NO distance and NO ranking.
        ///
        /// Deliberately its own case rather than a `providersFound` carrying no distances.
        /// `presentation` drops any provider it has no server distance for and then reports
        /// `noNearbyProviders`, so routing remembered records through it would render "no vets near
        /// you" about providers the phone is holding in its hand - the false absence this layer exists
        /// to prevent. `storedAge` may be `nil`: an age that could not be derived says nothing rather
        /// than inventing a number.
        case storedProvidersOnly(providers: [DirectoryProvider], storedAge: String?)
    }

    /// Complete unranked contact presentation. Located and contact-only providers share the found
    /// case; this state intentionally carries no distance claim.
    enum ContactDirectoryPresentation: Equatable {
        case loadingDirectory
        case directoryUnavailable(String)
        case directoryEmpty(ProviderDirectoryObservation)
        case noNameMatch(query: String, observation: ProviderDirectoryObservation)
        case providersFound(providers: [DirectoryProvider], observation: ProviderDirectoryObservation)
    }

    static func presentation(
        directory: ProviderDirectoryResult?,
        location: NearbyLocationState,
        query: String,
        unitSystem: NearbyUnitSystem
    ) -> Presentation {
        let origin: NearbyOrigin
        switch location {
        case .notRequested:
            return .awaitingOrigin
        case .locating:
            return .locating
        case .refused:
            return .permissionRefused
        case .unavailable:
            return .locationUnavailable
        case .ready(let candidate):
            guard candidate.point.isValid else { return .locationUnavailable }
            origin = candidate
        }

        guard let directory else { return .loadingDirectory }
        switch directory {
        case .unavailable(let failure):
            return .directoryUnavailable(failure.detail)
        case .empty(let snapshot):
            let searchedName = query.trimmingCharacters(in: .whitespacesAndNewlines)
            return searchedName.isEmpty
                ? .noNearbyProviders(snapshot.observation)
                : .noNameMatch(query: searchedName, observation: snapshot.observation)
        case .found(let snapshot):
            let effectiveAccuracy: Double?
            // Only the fix's OWN uncertainty bounds the label now. The former 100-metre floor existed
            // because the service received a three-decimal coordinate, so no distance computed from it
            // could be finer than that; the exact-position ruling removed that coarsening, and keeping
            // the floor would overstate uncertainty the request no longer introduces.
            if let reported = origin.accuracyMetres, reported.isFinite, reported >= 0 {
                effectiveAccuracy = reported
            } else {
                effectiveAccuracy = nil
            }
            let rows = snapshot.providers
                .filter(isEligibleProvider)
                .compactMap { provider -> Row? in
                    guard let km = provider.distanceKm, km.isFinite, km >= 0 else { return nil }
                    return Row(
                        provider: provider,
                        distanceKm: km,
                        distance: distanceClaim(
                            km,
                            accuracyMetres: effectiveAccuracy,
                            fromDeviceFix: true,
                            unitSystem: unitSystem
                        )
                    )
                }
            if !rows.isEmpty {
                return .providersFound(rows: rows, observation: snapshot.observation)
            }
            let searchedName = query.trimmingCharacters(in: .whitespacesAndNewlines)
            return searchedName.isEmpty
                ? .noNearbyProviders(snapshot.observation)
                : .noNameMatch(query: searchedName, observation: snapshot.observation)
        }
    }

    /// The separate name/contact directory. The service already applied name and paging; the app
    /// preserves response order and only repeats its vet/groomer safety gate.
    static func contactPresentation(
        directory: ProviderDirectoryResult?,
        query: String
    ) -> ContactDirectoryPresentation {
        guard let directory else { return .loadingDirectory }

        switch directory {
        case .unavailable(let failure):
            return .directoryUnavailable(failure.detail)
        case .empty(let snapshot):
            let searchedName = query.trimmingCharacters(in: .whitespacesAndNewlines)
            return searchedName.isEmpty
                ? .directoryEmpty(snapshot.observation)
                : .noNameMatch(query: searchedName, observation: snapshot.observation)
        case .found(let snapshot):
            let providers = snapshot.providers.filter(isEligibleProvider)
            if !providers.isEmpty {
                return .providersFound(
                    providers: providers,
                    observation: snapshot.observation
                )
            }
            let searchedName = query.trimmingCharacters(in: .whitespacesAndNewlines)
            return searchedName.isEmpty
                ? .directoryEmpty(snapshot.observation)
                : .noNameMatch(query: searchedName, observation: snapshot.observation)
        }
    }

    private static func isEligibleProvider(_ provider: DirectoryProvider) -> Bool {
        let kind = provider.kind.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return (kind == "vet" || kind == "groomer") && provider.active != false
    }

    /// What may be shown when a live read could not answer at all and records were remembered.
    ///
    /// Returns `nil` when there is nothing honest to show - nothing remembered, or nothing remembered
    /// matching this search - so the caller keeps the live "could not check" rather than replacing it
    /// with a reassuring list. A fallback that quietly answered an empty result would turn
    /// could-not-check into an established absence.
    ///
    /// No distance is computed or carried, and the order is whatever position-free order the cache
    /// stored. Mirrors Android `NearbyDecision.storedFallback`.
    static func storedFallback(
        records: StoredProviderRecords?,
        query: String,
        now: Date
    ) -> Presentation? {
        guard let records else { return nil }
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let providers = records.providers
            .filter(isEligibleProvider)
            .filter { needle.isEmpty || $0.name.lowercased().contains(needle) }
        guard !providers.isEmpty else { return nil }
        return .storedProvidersOnly(
            providers: providers,
            storedAge: formatStoredAge(storedAt: records.storedAt, now: now)
        )
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

    /// How old a remembered record set is, in words, for the offline label.
    ///
    /// Rounds OUTWARD so a replay never reads fresher than it is, and promotes at the ceiling rather
    /// than printing "60 minutes ago". An age that cannot be derived - a stored time in the future,
    /// i.e. a backwards clock jump - says nothing rather than inventing a number.
    /// Mirrors Android; keep the two in step by hand.
    static func formatStoredAge(storedAt: Date, now: Date) -> String? {
        let elapsed = now.timeIntervalSince(storedAt)
        guard elapsed.isFinite, elapsed >= 0 else { return nil }
        if elapsed < 60 { return "less than a minute ago" }
        let minutes = (elapsed / 60).rounded(.up)
        if minutes < 60 { return agePhrase(minutes, "minute") }
        let hours = (elapsed / 3_600).rounded(.up)
        if hours < 24 { return agePhrase(hours, "hour") }
        return agePhrase((elapsed / 86_400).rounded(.up), "day")
    }

    private static func agePhrase(_ count: Double, _ unit: String) -> String {
        count == 1 ? "1 \(unit) ago" : String(format: "%.0f", count) + " \(unit)s ago"
    }

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
    /// A device fix is rounded to a step no finer than its effective horizontal accuracy. Nearby
    /// supplies the coarser of the device-reported accuracy and the network coordinate's roughly
    /// hundred-metre granularity. A fix whose accuracy is missing, nonsensical, or coarser than any
    /// usable step yields no number at all. The non-device branch remains for generic formatting
    /// tests and non-location callers; Nearby always uses the device branch.
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
