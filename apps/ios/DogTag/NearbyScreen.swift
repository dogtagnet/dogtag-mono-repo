import SwiftUI
import CoreLocation
import MapKit

/// One-shot, user-driven location acquisition for Nearby.
///
/// Creating this object does not request permission or read a position. The manager is touched only
/// after the owner presses "Use my current location"; the chosen-coordinate path never touches it.
@MainActor
final class NearbyLocationController: NSObject, ObservableObject, @preconcurrency CLLocationManagerDelegate {
    @Published private(set) var state: NearbyLocationState = .notRequested

    private let manager = CLLocationManager()
    private var requestPending = false

    override init() {
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyHundredMeters

        switch manager.authorizationStatus {
        case .denied, .restricted:
            state = .refused
        default:
            // Even an existing grant does not cause acquisition until the explicit button tap.
            state = .notRequested
        }
    }

    func requestCurrentLocation() {
        requestPending = true
        state = .locating

        switch manager.authorizationStatus {
        case .notDetermined:
            manager.requestWhenInUseAuthorization()
        case .authorizedAlways, .authorizedWhenInUse:
            manager.requestLocation()
        case .denied, .restricted:
            requestPending = false
            state = .refused
        @unknown default:
            requestPending = false
            state = .unavailable
        }
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        switch manager.authorizationStatus {
        case .denied, .restricted:
            requestPending = false
            state = .refused
        case .authorizedAlways, .authorizedWhenInUse:
            guard requestPending else { return }
            manager.requestLocation()
        case .notDetermined:
            break
        @unknown default:
            requestPending = false
            state = .unavailable
        }
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        requestPending = false
        guard let location = locations.last else {
            state = .unavailable
            return
        }
        let point = NearbyPoint(
            lat: location.coordinate.latitude,
            lng: location.coordinate.longitude
        )
        // A negative `horizontalAccuracy` means Core Location considers the coordinate invalid, so
        // it is not a ready origin at all. Otherwise the reported uncertainty is carried into the
        // pure policy, which is what stops a hundred-metre-class fix rendering a metre-level number.
        guard point.isValid, location.horizontalAccuracy >= 0 else {
            state = .unavailable
            return
        }
        state = .ready(NearbyOrigin(
            point: point,
            source: .currentLocation,
            accuracyMetres: location.horizontalAccuracy
        ))
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        requestPending = false
        if let coreError = error as? CLError, coreError.code == .denied {
            state = .refused
        } else {
            state = .unavailable
        }
    }
}

/// Distance-ranked providers without an in-app map.
///
/// The screen reads one position-free provider snapshot, then applies name search, the 50 km browse
/// range and Core Location distance entirely in process. Directions is a destination-only handoff to
/// Apple Maps after an explicit tap.
struct NearbyScreen: View {
    private enum Scope: String, CaseIterable, Identifiable {
        case nearby = "Nearby"
        case contacts = "Provider contacts"

        var id: String { rawValue }
    }

    private enum OriginMode: String, CaseIterable, Identifiable {
        case current = "Current location"
        case chosen = "Chosen location"

        var id: String { rawValue }
    }

    /// A list entry, identified within the ONE scope entitled to render it.
    ///
    /// The two scopes present the same `providerId` under incompatible promises: Nearby states
    /// distance, bearing and Directions, while Provider contacts must claim none of them. They
    /// previously keyed both lists on the bare `providerId` inside one shared lazy container, and an
    /// explicit `id` overrides structural identity there, so switching scope re-presented an
    /// already-realised Nearby row under the contacts list - distance and Directions included,
    /// directly beneath the copy promising neither. `DirectoryProvider` is itself `Identifiable` on
    /// `providerId`, so passing it to `ForEach` straight is the collision; these wrappers exist to
    /// make it unrepresentable rather than merely documented, in both directions.
    private struct NearbyRowEntry: Identifiable {
        let row: NearbyDecision.Row

        var id: String { "nearby:\(row.provider.providerId)" }
    }

    private struct ContactRowEntry: Identifiable {
        let provider: DirectoryProvider

        var id: String { "contacts:\(provider.providerId)" }
    }

    @Environment(\.dogTagColors) private var c
    @Environment(\.openURL) private var openURL
    @StateObject private var location = NearbyLocationController()

    let onDone: () -> Void
    private let directory: CentralProviderDirectory

    @State private var directoryResult: ProviderDirectoryResult?
    @State private var isRefreshing = false
    @State private var scope: Scope = .nearby
    @State private var originMode: OriginMode = .current
    @State private var query = ""
    @State private var latitude = ""
    @State private var longitude = ""
    @State private var chosenLocation: NearbyLocationState = .notRequested

    init(
        onDone: @escaping () -> Void,
        directory: CentralProviderDirectory = CentralProviderDirectory()
    ) {
        self.onDone = onDone
        self.directory = directory
    }

    private var activeLocation: NearbyLocationState {
        originMode == .current ? location.state : chosenLocation
    }

    private var unitSystem: NearbyUnitSystem {
        NearbyUnitSystem.forRegion(Locale.current.region?.identifier)
    }

    private var nearbyPresentation: NearbyDecision.Presentation {
        NearbyDecision.presentation(
            directory: directoryResult,
            location: activeLocation,
            query: query,
            unitSystem: unitSystem,
            distanceKm: localDistance
        )
    }

    private var hasReadyOrigin: Bool {
        if case .ready = activeLocation { return true }
        return false
    }

    /// Do not invite a location permission prompt when the pure policy already proves no row can
    /// render. For a phone-local no-match/range result, keep an existing origin editable; a directory
    /// that has not answered still hides the controls because no distance claim can be made at all.
    private var shouldShowOriginCard: Bool {
        switch nearbyPresentation {
        case .loadingDirectory, .directoryUnavailable, .directoryEmpty:
            return false
        case .noneWithinRange, .noNameMatch:
            return hasReadyOrigin
        case .awaitingOrigin, .locating, .permissionRefused, .locationUnavailable,
             .invalidChosenLocation, .providersFound:
            return true
        }
    }

    var body: some View {
        NavigationStack {
            ZStack {
                c.background.ignoresSafeArea()
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 16) {
                        privacyCard
                        Picker("Directory view", selection: $scope) {
                            ForEach(Scope.allCases) { item in
                                Text(item.rawValue).tag(item)
                            }
                        }
                        .pickerStyle(.segmented)

                        searchField
                        directoryObservation

                        // Each scope owns its OWN lazy container, so no provider row is ever a direct
                        // child of a container the other scope also fills. A lazy stack caches its
                        // cells by their explicit `id`, which is why sharing one container let a
                        // realised Nearby row reappear under Provider contacts.
                        if scope == .nearby {
                            LazyVStack(alignment: .leading, spacing: 16) {
                                if shouldShowOriginCard {
                                    originCard
                                }
                                nearbyContent
                            }
                        } else {
                            LazyVStack(alignment: .leading, spacing: 16) {
                                contactContent
                            }
                        }
                        Spacer(minLength: 20)
                    }
                    .padding(20)
                }
                .refreshable { await refreshDirectory() }
            }
            .navigationTitle("Nearby")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                // `.navigationBarTrailing`, not `.topBarTrailing`: the latter is iOS 17+ and this
                // app targets iOS 16.
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button("Done", action: onDone)
                }
            }
        }
        .task {
            if directoryResult == nil {
                await refreshDirectory()
            }
        }
    }

    private var privacyCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Private by design", systemImage: "hand.raised.fill")
                .font(.system(size: 15, weight: .bold))
                .foregroundColor(c.onBackground)
            Text("Your position and name search stay on this phone. DogTag downloads the same full provider list for everyone.")
                .font(.system(size: 13))
                .foregroundColor(c.muted)
                .fixedSize(horizontal: false, vertical: true)
            Text("Directions opens Maps only after you tap and hands off the provider's public destination.")
                .font(.system(size: 12))
                .foregroundColor(c.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
    }

    private var searchField: some View {
        HStack(spacing: 10) {
            Image(systemName: "magnifyingglass")
                .foregroundColor(c.muted)
            TextField("Search by provider name", text: $query)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .foregroundColor(c.onBackground)
            if !query.isEmpty {
                Button {
                    query = ""
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundColor(c.muted)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear provider search")
            }
        }
        .padding(.horizontal, 14)
        .frame(height: 46)
        .background(RoundedRectangle(cornerRadius: 13).fill(c.surface))
        .overlay(RoundedRectangle(cornerRadius: 13).stroke(c.outline))
    }

    @ViewBuilder
    private var directoryObservation: some View {
        if let observation = successfulObservation {
            HStack(spacing: 6) {
                Image(systemName: observation == .live ? "bolt.fill" : "clock.arrow.circlepath")
                Text(observation == .live
                     ? "Live provider directory"
                     : "Stored provider directory · live refresh unavailable")
            }
            .font(.system(size: 11, weight: .semibold))
            .foregroundColor(observation == .live ? c.success : c.warning)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(
                Capsule().fill((observation == .live ? c.success : c.warning).opacity(0.12))
            )
        }
    }

    private var successfulObservation: ProviderDirectoryObservation? {
        switch directoryResult {
        case .found(let snapshot), .empty(let snapshot):
            return snapshot.observation
        case .unavailable, .none:
            return nil
        }
    }

    private var originCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Distance from")
                .font(.system(size: 15, weight: .bold))
                .foregroundColor(c.onBackground)

            Picker("Starting location", selection: $originMode) {
                ForEach(OriginMode.allCases) { item in
                    Text(item.rawValue).tag(item)
                }
            }
            .pickerStyle(.segmented)

            if originMode == .current {
                Button {
                    location.requestCurrentLocation()
                } label: {
                    Label(
                        locationButtonLabel,
                        systemImage: "location.fill"
                    )
                    .font(.system(size: 14, weight: .semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 11)
                    .foregroundColor(c.onAccent)
                    .background(RoundedRectangle(cornerRadius: 12).fill(c.accent))
                }
                .buttonStyle(.plain)
                .disabled(location.state == .locating)

                currentLocationCaption
            } else {
                Text("Enter coordinates from a place you choose. They are parsed on this phone; DogTag does not geocode or send them anywhere.")
                    .font(.system(size: 12))
                    .foregroundColor(c.muted)
                    .fixedSize(horizontal: false, vertical: true)

                HStack(spacing: 10) {
                    coordinateField("Latitude", text: $latitude)
                    coordinateField("Longitude", text: $longitude)
                }

                Button {
                    applyChosenLocation()
                } label: {
                    Text("Use these coordinates")
                        .font(.system(size: 14, weight: .semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 11)
                        .foregroundColor(c.onAccent)
                        .background(RoundedRectangle(cornerRadius: 12).fill(c.accent))
                }
                .buttonStyle(.plain)

                chosenLocationCaption
            }
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
    }

    private var locationButtonLabel: String {
        switch location.state {
        case .ready: return "Update current location"
        case .locating: return "Finding location…"
        default: return "Use my current location"
        }
    }

    @ViewBuilder
    private var currentLocationCaption: some View {
        switch location.state {
        case .notRequested:
            Text("When-in-use permission is requested only when you tap. Your position stays on this phone.")
                .foregroundColor(c.muted)
        case .locating:
            HStack(spacing: 8) {
                ProgressView()
                Text("Finding your location on this device…")
            }
            .foregroundColor(c.muted)
        case .ready(let fix):
            // State the fix's own uncertainty rather than implying a precise position was obtained.
            Label(
                NearbyDecision.accuracyNote(fix.accuracyMetres, unitSystem: unitSystem)
                    .map { "Using the current location held on this phone, accurate to \($0)" }
                    ?? "Using the current location held on this phone",
                systemImage: "checkmark.circle.fill"
            )
            .foregroundColor(c.success)
        case .refused:
            Text("Location permission was refused. Choose a location above or allow access in Settings.")
                .foregroundColor(c.danger)
        case .unavailable:
            Text("The phone could not provide a location. Try again or use a chosen location.")
                .foregroundColor(c.warning)
        case .invalidChosenLocation:
            EmptyView()
        }
    }

    @ViewBuilder
    private var chosenLocationCaption: some View {
        switch chosenLocation {
        case .ready(let chosen):
            Label(
                String(
                    format: "Using %.5f, %.5f on this phone",
                    chosen.point.lat,
                    chosen.point.lng
                ),
                systemImage: "checkmark.circle.fill"
            )
            .foregroundColor(c.success)
        case .invalidChosenLocation:
            Text("Enter latitude from −90 to 90 and longitude from −180 to 180.")
                .foregroundColor(c.danger)
        default:
            EmptyView()
        }
    }

    private func coordinateField(_ title: String, text: Binding<String>) -> some View {
        TextField(title, text: text)
            .keyboardType(.numbersAndPunctuation)
            .textContentType(.none)
            .padding(.horizontal, 12)
            .frame(height: 42)
            .foregroundColor(c.onBackground)
            .background(RoundedRectangle(cornerRadius: 10).fill(c.surfaceVariant))
            .overlay(RoundedRectangle(cornerRadius: 10).stroke(c.outline))
    }

    @ViewBuilder
    private var nearbyContent: some View {
        switch nearbyPresentation {
        case .loadingDirectory:
            NearbyMessageCard(
                icon: "arrow.down.circle",
                title: "Loading provider directory…",
                message: "Fetching the same position-free provider list used for every owner.",
                tone: c.muted,
                showsProgress: true
            )
        case .providersFound(let rows, _):
            SectionTitle(
                text: query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? "Within \(rangeLabel(NearbyDecision.defaultRadiusKm))"
                    : "Name matches",
                trailing: "\(rows.count)"
            )
            ForEach(rows.map { NearbyRowEntry(row: $0) }) { entry in
                nearbyProviderRow(entry.row)
            }
        case .directoryEmpty:
            NearbyMessageCard(
                icon: "building.2.crop.circle",
                title: "Provider directory is empty",
                message: "The directory answered successfully, but no providers were published.",
                tone: c.muted
            )
        case .noneWithinRange(let radiusKm, _):
            NearbyMessageCard(
                icon: "location.slash",
                title: "No providers within \(rangeLabel(radiusKm))",
                message: "The directory answered, but no location-published vets or groomers are within range. Providers without a location remain reachable under Provider contacts.",
                tone: c.muted
            )
        case .noNameMatch:
            NearbyMessageCard(
                icon: "magnifyingglass",
                title: "No located provider has that name",
                message: "Name search ran on this phone across every located provider. Contact-only providers are searchable under Provider contacts.",
                tone: c.muted
            )
        case .directoryUnavailable(let detail):
            NearbyMessageCard(
                icon: "wifi.exclamationmark",
                title: "Provider directory unavailable",
                message: "\(detail). This does not mean there are no providers nearby.",
                tone: c.danger,
                actionTitle: "Try again",
                action: { Task { await refreshDirectory() } }
            )
        case .permissionRefused:
            NearbyMessageCard(
                icon: "location.slash.fill",
                title: "Location permission refused",
                message: "DogTag cannot calculate distance from your current location. Use Chosen location above, or allow when-in-use access in Settings.",
                tone: c.danger,
                actionTitle: "Open Settings",
                action: openSettings
            )
        case .locationUnavailable:
            NearbyMessageCard(
                icon: "location.slash",
                title: "Current location unavailable",
                message: "The phone could not provide a position. Try once more, or use Chosen location above.",
                tone: c.warning,
                actionTitle: "Try location again",
                action: { location.requestCurrentLocation() }
            )
        case .invalidChosenLocation:
            NearbyMessageCard(
                icon: "exclamationmark.circle",
                title: "Chosen location is invalid",
                message: "Correct the latitude and longitude above. Nothing is sent while you edit or apply them.",
                tone: c.danger
            )
        case .awaitingOrigin:
            NearbyMessageCard(
                icon: "location.circle",
                title: "Choose a starting location",
                message: "Use your current location or enter coordinates above to calculate distance on this phone.",
                tone: c.muted
            )
        case .locating:
            NearbyMessageCard(
                icon: "location.circle",
                title: "Finding your location…",
                message: "Distance and ordering will appear once this phone supplies a position.",
                tone: c.muted,
                showsProgress: true
            )
        }
    }

    @ViewBuilder
    private var contactContent: some View {
        let presentation = NearbyDecision.contactPresentation(
            directory: directoryResult,
            query: query
        )

        switch presentation {
        case .loadingDirectory:
            NearbyMessageCard(
                icon: "arrow.down.circle",
                title: "Loading provider directory…",
                message: "Fetching the same position-free provider list used for every owner.",
                tone: c.muted,
                showsProgress: true
            )
        case .directoryUnavailable(let detail):
            NearbyMessageCard(
                icon: "wifi.exclamationmark",
                title: "Provider directory unavailable",
                message: "\(detail). This does not mean no providers have published contacts.",
                tone: c.danger,
                actionTitle: "Try again",
                action: { Task { await refreshDirectory() } }
            )
        case .directoryEmpty:
            NearbyMessageCard(
                icon: "building.2.crop.circle",
                title: "No vet or groomer contacts",
                message: "The directory answered, but it contained no eligible vet or groomer contact listings.",
                tone: c.muted
            )
        case .noNameMatch:
            NearbyMessageCard(
                icon: "magnifyingglass",
                title: "No provider has that name",
                message: "This unranked contact list includes located and contact-only providers. Name search happens on this phone.",
                tone: c.muted
            )
        case .providersFound(let contacts, _):
            SectionTitle(text: "Provider contacts", trailing: "\(contacts.count)")
            Text("Includes providers that publish contact details without publishing a location. This list never calculates distance or offers Directions.")
                .font(.system(size: 12))
                .foregroundColor(c.muted)
                .fixedSize(horizontal: false, vertical: true)
            ForEach(contacts.map { ContactRowEntry(provider: $0) }) { entry in
                providerContactRow(entry.provider)
            }
        }
    }

    @ViewBuilder
    private func distanceLine(_ row: NearbyDecision.Row) -> some View {
        switch row.distance {
        case .measured:
            HStack(spacing: 7) {
                Image(systemName: "location.fill")
                Text(row.distance.display ?? "")
                    .fontWeight(.bold)
                if let bearing = row.bearingLabel {
                    Text("· \(bearing)")
                }
            }
            .font(.system(size: 13))
            .foregroundColor(c.onBackground)
        case .uncertain(let reason):
            // Never a confident number the origin cannot support, and never a silent blank either.
            HStack(spacing: 7) {
                Image(systemName: "location.slash")
                Text(row.bearingLabel.map { "\(reason) Bearing \($0)." } ?? reason)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .font(.system(size: 12))
            .foregroundColor(c.muted)
        }
    }

    private func nearbyProviderRow(_ row: NearbyDecision.Row) -> some View {
        VStack(alignment: .leading, spacing: 11) {
            providerHeader(row.provider)
            distanceLine(row)

            providerServices(row.provider)
            DomainBindingLine(binding: IssuerBinding(
                state: row.provider.bindingState,
                domain: row.provider.domain ?? ""
            ))

            Button {
                openDirections(to: row.provider)
            } label: {
                Label("Directions", systemImage: "arrow.triangle.turn.up.right.diamond.fill")
                    .font(.system(size: 14, weight: .semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                    .foregroundColor(c.onAccent)
                    .background(RoundedRectangle(cornerRadius: 11).fill(c.accent))
            }
            .buttonStyle(.plain)
            .accessibilityHint("Opens the provider destination in Maps")
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
    }

    private func providerContactRow(_ provider: DirectoryProvider) -> some View {
        VStack(alignment: .leading, spacing: 11) {
            providerHeader(provider)
            providerServices(provider)
            DomainBindingLine(binding: IssuerBinding(
                state: provider.bindingState,
                domain: provider.domain ?? ""
            ))

            if provider.contact.hasAny {
                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 105), spacing: 8)],
                    alignment: .leading,
                    spacing: 8
                ) {
                    contactAction("Phone", value: provider.contact.phone, icon: "phone.fill", kind: .phone)
                    contactAction("WhatsApp", value: provider.contact.whatsapp, icon: "message.fill", kind: .whatsapp)
                    contactAction("Telegram", value: provider.contact.telegram, icon: "paperplane.fill", kind: .telegram)
                    contactAction("Email", value: provider.contact.email, icon: "envelope.fill", kind: .email)
                    contactAction("Website", value: provider.contact.website, icon: "globe", kind: .website)
                }
            } else {
                Text("No contact details published.")
                    .font(.system(size: 12))
                    .foregroundColor(c.muted)
            }
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
    }

    private func providerHeader(_ provider: DirectoryProvider) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(provider.name.isEmpty ? "Unnamed provider" : provider.name)
                .font(.system(size: 17, weight: .bold))
                .foregroundColor(c.onBackground)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 8)
            Text(provider.kind.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() == "groomer"
                 ? "Groomer"
                 : "Vet")
                .font(.system(size: 10, weight: .bold))
                .foregroundColor(c.accent)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(Capsule().fill(c.accent.opacity(0.12)))
        }
    }

    @ViewBuilder
    private func providerServices(_ provider: DirectoryProvider) -> some View {
        let services = provider.services
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        if !services.isEmpty {
            Text(services.joined(separator: " · "))
                .font(.system(size: 12))
                .foregroundColor(c.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private enum ContactKind {
        case phone
        case whatsapp
        case telegram
        case email
        case website
    }

    @ViewBuilder
    private func contactAction(
        _ label: String,
        value: String?,
        icon: String,
        kind: ContactKind
    ) -> some View {
        if let value, !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            if let url = contactURL(value, kind: kind) {
                Button {
                    openURL(url)
                } label: {
                    Label(label, systemImage: icon)
                        .font(.system(size: 12, weight: .semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 9)
                        .foregroundColor(c.accent)
                        .background(RoundedRectangle(cornerRadius: 10).fill(c.surfaceVariant))
                }
                .buttonStyle(.plain)
                .accessibilityValue(value)
            } else {
                VStack(alignment: .leading, spacing: 2) {
                    Label(label, systemImage: icon)
                        .font(.system(size: 11, weight: .semibold))
                    Text(value)
                        .font(.system(size: 11))
                        .lineLimit(2)
                }
                .foregroundColor(c.muted)
            }
        }
    }

    private func contactURL(_ raw: String, kind: ContactKind) -> URL? {
        let value = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        switch kind {
        case .phone:
            let number = value.filter { $0.isASCII && ($0.isNumber || "+*#".contains($0)) }
            return number.contains(where: \.isNumber) ? URL(string: "tel:\(number)") : nil
        case .whatsapp:
            let number = value.filter { $0.isASCII && $0.isNumber }
            return number.isEmpty ? nil : URL(string: "https://wa.me/\(number)")
        case .telegram:
            let handle = value
                .replacingOccurrences(of: "https://t.me/", with: "", options: [.caseInsensitive])
                .trimmingCharacters(in: CharacterSet(charactersIn: "@/ "))
            let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "_"))
            guard !handle.isEmpty,
                  handle.unicodeScalars.allSatisfy({ allowed.contains($0) }) else { return nil }
            return URL(string: "https://t.me/\(handle)")
        case .email:
            var components = URLComponents()
            components.scheme = "mailto"
            components.path = value
            return value.contains("@") ? components.url : nil
        case .website:
            // The first channel whose SCHEME comes from the directory string rather than from us:
            // the four above all construct theirs. So only an explicit http(s) value is opened;
            // anything else renders as published text rather than being handed to `openURL` as typed.
            let lowered = value.lowercased()
            guard lowered.hasPrefix("http://") || lowered.hasPrefix("https://") else { return nil }
            return URL(string: value)
        }
    }

    private func localDistance(_ origin: NearbyPoint, _ destination: NearbyPoint) -> Double? {
        guard origin.isValid, destination.isValid else { return nil }
        let from = CLLocation(latitude: origin.lat, longitude: origin.lng)
        let to = CLLocation(latitude: destination.lat, longitude: destination.lng)
        let kilometres = from.distance(from: to) / 1_000
        return kilometres.isFinite && kilometres >= 0 ? kilometres : nil
    }

    private func rangeLabel(_ radiusKm: Double) -> String {
        NearbyDecision.formatDistanceKm(radiusKm, unitSystem: unitSystem)
            ?? String(format: "%.0f km", radiusKm)
    }

    private func applyChosenLocation() {
        if let point = NearbyDecision.parseChosenOrigin(lat: latitude, lng: longitude) {
            // Typed coordinates carry no measurement uncertainty, so they keep ordinary precision.
            chosenLocation = .ready(.chosen(point))
        } else {
            chosenLocation = .invalidChosenLocation
        }
    }

    private func openDirections(to provider: DirectoryProvider) {
        guard let point = provider.geo, point.isValid else { return }
        let destination = MKMapItem(placemark: MKPlacemark(
            coordinate: CLLocationCoordinate2D(latitude: point.lat, longitude: point.lng)
        ))
        destination.name = provider.name
        // Destination only. The owner deliberately crosses into Maps here; DogTag supplies neither
        // the current/chosen origin nor any viewport, route, tile request or background update.
        destination.openInMaps(launchOptions: [
            MKLaunchOptionsDirectionsModeKey: MKLaunchOptionsDirectionsModeDriving,
        ])
    }

    private func openSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        openURL(url)
    }

    private func refreshDirectory() async {
        guard !isRefreshing else { return }
        isRefreshing = true
        let result = await directory.read()
        directoryResult = result
        isRefreshing = false
    }
}

private struct NearbyMessageCard: View {
    @Environment(\.dogTagColors) private var c

    let icon: String
    let title: String
    let message: String
    let tone: Color
    var actionTitle: String? = nil
    var action: (() -> Void)? = nil
    var showsProgress: Bool = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 9) {
                if showsProgress {
                    ProgressView().tint(tone)
                } else {
                    Image(systemName: icon)
                        .font(.system(size: 17, weight: .semibold))
                        .foregroundColor(tone)
                }
                Text(title)
                    .font(.system(size: 16, weight: .bold))
                    .foregroundColor(c.onBackground)
            }
            Text(message)
                .font(.system(size: 13))
                .foregroundColor(c.muted)
                .fixedSize(horizontal: false, vertical: true)

            if let actionTitle, let action {
                Button(action: action) {
                    Text(actionTitle)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundColor(c.accent)
                        .padding(.vertical, 4)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
        .overlay(RoundedRectangle(cornerRadius: 16).stroke(tone.opacity(0.18)))
    }
}
