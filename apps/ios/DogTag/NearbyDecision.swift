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
