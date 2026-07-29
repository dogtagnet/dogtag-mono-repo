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
struct ProviderContact: Equatable {
    var phone: String? = nil
    var whatsapp: String? = nil
    var telegram: String? = nil
    var email: String? = nil

    var hasAny: Bool {
        [phone, whatsapp, telegram, email].contains { value in
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
    let expiresAt: Date?
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
    func read() async -> ProviderDirectoryResult
}

enum NearbyLocationState: Equatable {
    case notRequested
    case locating
    case ready(NearbyPoint)
    case refused
    case unavailable
    case invalidChosenLocation
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
        let distanceKm: Double
        let distanceLabel: String
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

            let origin: NearbyPoint
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
            case .ready(let point):
                guard point.isValid else {
                    return .locationUnavailable
                }
                origin = point
            }

            let ranked = candidates.enumerated().compactMap { index, provider -> (Int, Row)? in
                guard let destination = provider.geo, destination.isValid,
                      let km = distanceKm(origin, destination),
                      km.isFinite, km >= 0,
                      let label = formatDistanceKm(km, unitSystem: unitSystem) else {
                    return nil
                }
                return (
                    index,
                    Row(
                        provider: provider,
                        distanceKm: km,
                        distanceLabel: label,
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
                    let comparison = $0.name.localizedCaseInsensitiveCompare($1.name)
                    return comparison == .orderedSame
                        ? $0.providerId < $1.providerId
                        : comparison == .orderedAscending
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

    static func formatDistanceKm(
        _ km: Double?,
        unitSystem: NearbyUnitSystem
    ) -> String? {
        guard let km, km.isFinite, km >= 0 else { return nil }
        if unitSystem == .imperial {
            let miles = km / 1.609344
            if miles < 0.1 {
                let feet = km * (1000 / 0.3048)
                if feet < 25 { return "< 25 ft" }
                return "\(Int((feet / 25).rounded() * 25)) ft"
            }
            let oneDecimal = (miles * 10).rounded() / 10
            if oneDecimal < 10 { return String(format: "%.1f mi", oneDecimal) }
            return "\(Int(miles.rounded())) mi"
        }

        if km < 1 {
            let metres = km * 1000
            if metres < 10 { return "< 10 m" }
            let rounded = (metres / 10).rounded() * 10
            if rounded < 1000 { return "\(Int(rounded)) m" }
        }
        let oneDecimal = (km * 10).rounded() / 10
        if oneDecimal < 10 { return String(format: "%.1f km", oneDecimal) }
        return "\(Int(km.rounded())) km"
    }
}
