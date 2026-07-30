import SwiftUI
import CoreLocation

/// One-shot, user-driven location acquisition for Nearby.
///
/// Creating this object does not request permission or read a position. The manager is touched only
/// after the owner presses "Use my current location".
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
        let rawPoint = NearbyPoint(
            lat: location.coordinate.latitude,
            lng: location.coordinate.longitude
        )
        // A negative `horizontalAccuracy` means Core Location considers the coordinate invalid, so it
        // is not a ready origin at all. Otherwise the reported uncertainty is carried into the pure
        // policy, where it is now the ONLY bound on how finely a distance may be stated - the request
        // sends the exact fix, so it introduces no coarseness of its own to floor against.
        guard let validated = rawPoint.validatedForProviderSearch(),
              location.horizontalAccuracy >= 0 else {
            state = .unavailable
            return
        }
        state = .ready(NearbyOrigin(
            point: validated,
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

/// Paged, server-ranked providers in a list. There is no map, chosen-location search, autocomplete,
/// geocoder, local distance computation, or Directions handoff.
struct NearbyScreen: View {
    private enum Scope: String, CaseIterable, Identifiable {
        case nearby = "Nearby"
        case contacts = "Provider contacts"

        var id: String { rawValue }
    }

    /// A list entry, identified within the ONE scope entitled to render it.
    ///
    /// The two scopes present the same `providerId` under incompatible promises: Nearby states a
    /// server-computed distance, while Provider contacts must claim none. They
    /// previously keyed both lists on the bare `providerId` inside one shared lazy container, and an
    /// explicit `id` overrides structural identity there, so switching scope re-presented an
    /// already-realised Nearby row under the contacts list - distance included,
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
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var location = NearbyLocationController()

    let onDone: () -> Void
    /// The seam, not the concrete adapter. The offline copy is deliberately NOT wrapped around it: a
    /// nearest page is personalized, so there is no whole-directory response to substitute for, and
    /// the screen drives remember/recall explicitly instead.
    private let directory: ProviderDirectoryReading
    /// Records only: identity and contacts, never a distance or the ranking.
    private let recordCache: ProviderRecordCache

    @State private var storedRecords: StoredProviderRecords?
    @State private var nearbyResult: ProviderDirectoryResult?
    @State private var contactResult: ProviderDirectoryResult?
    @State private var isLoadingNearby = false
    @State private var isLoadingContacts = false
    @State private var scope: Scope = .nearby
    @State private var query = ""
    @State private var nearbyName: String?
    @State private var contactName: String?
    @State private var nearbyRequestID: UUID?
    @State private var contactRequestID: UUID?

    init(
        onDone: @escaping () -> Void,
        directory: ProviderDirectoryReading = ProviderDirectories.central(),
        recordCache: ProviderRecordCache? = nil
    ) {
        self.onDone = onDone
        self.directory = directory
        self.recordCache = recordCache ?? ProviderDirectories.recordCache(for: directory)
    }

    private var unitSystem: NearbyUnitSystem {
        NearbyUnitSystem.forRegion(Locale.current.region?.identifier)
    }

    private var nearbyPresentation: NearbyDecision.Presentation {
        let live = NearbyDecision.presentation(
            directory: nearbyResult,
            location: location.state,
            query: nearbyName ?? "",
            unitSystem: unitSystem
        )
        // The remembered set may only stand in when the live read could not answer. It never overrides
        // a real answer, and when nothing relevant is remembered the live "could not check" stands - a
        // fallback that answered an empty list would turn could-not-check into an established absence.
        if case .directoryUnavailable = live {
            // Read `scenePhase` for its DEPENDENCY only, carried over from the helper this replaced.
            // This property is inlined into `body`, so registering the dependency is what makes a
            // foreground return re-evaluate and re-sample `Date()`; without it the owner could come back
            // to an age computed from an older sample, understating staleness in exactly the direction
            // the outward rounding exists to prevent. The value is deliberately not branched on: only
            // the read matters, and an `.inactive` scene - app-switcher snapshot, pulled-down Control
            // Centre, a view pushed before `.active` lands - is still one the owner is looking at, so
            // suppressing the label there would blank it on exactly the fresh entry it serves.
            _ = scenePhase
            return NearbyDecision.storedFallback(
                records: storedRecords,
                query: nearbyName ?? "",
                now: Date()
            ) ?? live
        }
        return live
    }

    private var activeResult: ProviderDirectoryResult? {
        scope == .nearby ? nearbyResult : contactResult
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
                                originCard
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
        .onChange(of: scope) { selected in
            guard selected == .contacts, contactResult == nil else { return }
            Task { await loadContacts(reset: true) }
        }
        .onChange(of: location.state) { state in
            guard case .ready = state else { return }
            Task { await loadNearest(reset: true) }
        }
        .task {
            if scope == .contacts, contactResult == nil {
                await loadContacts(reset: true)
            }
        }
    }

    private var privacyCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("How your location is used", systemImage: "hand.raised.fill")
                .font(.system(size: 15, weight: .bold))
                .foregroundColor(c.onBackground)
            // The one pinned sentence, not a sibling literal. Commit 0f643ff put the copy in the pure
            // layer so a test could hold it byte-for-byte, but this card kept its own wording and went
            // on claiming the fix was rounded here long after that stopped being true. Reading the
            // constant is what makes a future softening impossible without failing that test.
            Text(NearbyDecision.locationDisclosure)
                .font(.system(size: 13))
                .foregroundColor(c.muted)
                .fixedSize(horizontal: false, vertical: true)
            Text("Provider-name searches are sent to the same service. There is no map, place search, autocomplete, or geocoder.")
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
                .submitLabel(.search)
                .onSubmit {
                    Task { await submitSearch() }
                }
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
            Button {
                Task { await submitSearch() }
            } label: {
                Image(systemName: "arrow.right.circle.fill")
                    .foregroundColor(c.accent)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Search providers")
        }
        .padding(.horizontal, 14)
        .frame(height: 46)
        .background(RoundedRectangle(cornerRadius: 13).fill(c.surface))
        .overlay(RoundedRectangle(cornerRadius: 13).stroke(c.outline))
    }

    /// The live badge only.
    ///
    /// A successful result is now always a live read: the offline copy no longer produces a
    /// `ProviderDirectoryResult` at all - it produces the `storedProvidersOnly` presentation, which
    /// carries its own age and its own plainly-stored card. So the former "Stored provider
    /// directory · remembered <age>" arm here became unreachable, and with it the `storedAgeClause`
    /// helper that computed an age from a snapshot's `readAt`. Rendering it from a live snapshot would
    /// have labelled a fresh answer as remembered.
    @ViewBuilder
    private var directoryObservation: some View {
        if let observation = successfulObservation, observation == .live {
            HStack(spacing: 6) {
                Image(systemName: "bolt.fill")
                Text("Live provider directory")
            }
            .font(.system(size: 11, weight: .semibold))
            .foregroundColor(c.success)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(Capsule().fill(c.success.opacity(0.12)))
        }
    }

    private var successfulObservation: ProviderDirectoryObservation? {
        switch activeResult {
        case .found(let snapshot), .empty(let snapshot):
            return snapshot.observation
        case .unavailable, .none:
            return nil
        }
    }

    private var originCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Find providers nearest to you")
                .font(.system(size: 15, weight: .bold))
                .foregroundColor(c.onBackground)

            Text(NearbyDecision.locationDisclosure)
                .font(.system(size: 13, weight: .semibold))
                .foregroundColor(c.onBackground)
                .fixedSize(horizontal: false, vertical: true)

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
            Text("When-in-use permission is requested only when you tap. Your location is used to rank nearby providers and for nothing else.")
                .foregroundColor(c.muted)
        case .locating:
            HStack(spacing: 8) {
                ProgressView()
                Text("Finding your current location…")
            }
            .foregroundColor(c.muted)
        case .ready(let fix):
            // State the fix's own uncertainty rather than implying a precise position was obtained.
            Label(
                NearbyDecision.accuracyNote(fix.accuracyMetres, unitSystem: unitSystem)
                    .map { "Using your current location; the phone reported accuracy of \($0)" }
                    ?? "Using your current location",
                systemImage: "checkmark.circle.fill"
            )
            .foregroundColor(c.success)
        case .refused:
            Text("Location permission was refused. Allow when-in-use access in Settings to find nearby providers.")
                .foregroundColor(c.danger)
        case .unavailable:
            Text("The phone could not provide a location. Try again.")
                .foregroundColor(c.warning)
        }
    }

    @ViewBuilder
    private var nearbyContent: some View {
        switch nearbyPresentation {
        case .loadingDirectory:
            NearbyMessageCard(
                icon: "arrow.down.circle",
                title: "Finding nearby providers…",
                message: "The service is ranking a page from the location sent by this phone.",
                tone: c.muted,
                showsProgress: true
            )
        case .providersFound(let rows, _):
            SectionTitle(
                text: nearbyName == nil ? "Nearest providers" : "Nearest name matches",
                trailing: resultCountLabel(nearbyResult, loaded: rows.count)
            )
            ForEach(rows.map { NearbyRowEntry(row: $0) }) { entry in
                nearbyProviderRow(entry.row)
            }
            loadMoreButton(for: .nearby)
        case .storedProvidersOnly(let providers, let storedAge):
            NearbyMessageCard(
                icon: "internaldrive",
                title: "Showing providers saved on this phone",
                message: {
                    var text = "We could not reach the provider directory, so these are providers this "
                    text += "phone saw before"
                    if let storedAge { text += " (last updated \(storedAge))" }
                    text += ". They are NOT sorted by distance and no distance is shown: that needs "
                    text += "the service. Contact details may be out of date."
                    return text
                }(),
                tone: c.warning
            )
            // The contact row deliberately: it renders identity and contacts and makes no proximity
            // claim, which is exactly what a remembered record can support.
            ForEach(providers.map { ContactRowEntry(provider: $0) }) { entry in
                providerContactRow(entry.provider)
            }
        case .directoryEmpty:
            NearbyMessageCard(
                icon: "building.2.crop.circle",
                title: "Provider directory is empty",
                message: "The directory answered successfully, but no providers were published.",
                tone: c.muted
            )
        case .noNearbyProviders:
            NearbyMessageCard(
                icon: "location.slash",
                title: "No nearby providers found",
                message: "The service found no location-published vets or groomers for this page. Providers without a location remain searchable under Provider contacts.",
                tone: c.muted
            )
        case .noNameMatch:
            NearbyMessageCard(
                icon: "magnifyingglass",
                title: "No located provider has that name",
                message: "The provider service found no located vet or groomer matching that name. Contact-only providers are searchable under Provider contacts.",
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
                message: "DogTag cannot request server-ranked nearby providers without your current location. Allow when-in-use access in Settings, or search Provider contacts by name.",
                tone: c.danger,
                actionTitle: "Open Settings",
                action: openSettings
            )
        case .locationUnavailable:
            NearbyMessageCard(
                icon: "location.slash",
                title: "Current location unavailable",
                message: "The phone could not provide a position. Try once more, or search Provider contacts by name.",
                tone: c.warning,
                actionTitle: "Try location again",
                action: { location.requestCurrentLocation() }
            )
        case .awaitingOrigin:
            NearbyMessageCard(
                icon: "location.circle",
                title: "Use your current location",
                message: "Tap above to let the provider service return the nearest vets and groomers, or search by provider name under Provider contacts.",
                tone: c.muted
            )
        case .locating:
            NearbyMessageCard(
                icon: "location.circle",
                title: "Finding your location…",
                message: "The provider service will use it only to rank nearby providers, and does not store it.",
                tone: c.muted,
                showsProgress: true
            )
        }
    }

    @ViewBuilder
    private var contactContent: some View {
        let presentation = NearbyDecision.contactPresentation(
            directory: contactResult,
            query: contactName ?? ""
        )

        switch presentation {
        case .loadingDirectory:
            NearbyMessageCard(
                icon: "arrow.down.circle",
                title: "Loading provider directory…",
                message: "Fetching a page of vets and groomers from the provider service.",
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
                message: "The provider service found no vet or groomer matching that name.",
                tone: c.muted
            )
        case .providersFound(let contacts, _):
            SectionTitle(
                text: contactName == nil ? "Provider contacts" : "Name matches",
                trailing: resultCountLabel(contactResult, loaded: contacts.count)
            )
            Text("Includes located and contact-only vets and groomers. This list sends no position and shows no map.")
                .font(.system(size: 12))
                .foregroundColor(c.muted)
                .fixedSize(horizontal: false, vertical: true)
            ForEach(contacts.map { ContactRowEntry(provider: $0) }) { entry in
                providerContactRow(entry.provider)
            }
            loadMoreButton(for: .contacts)
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
            }
            .font(.system(size: 13))
            .foregroundColor(c.onBackground)
        case .uncertain(let reason):
            // Never a confident number the origin cannot support, and never a silent blank either.
            HStack(spacing: 7) {
                Image(systemName: "location.slash")
                Text(reason)
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
            providerContactActions(row.provider)
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

            providerContactActions(provider)
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
    private func providerContactActions(_ provider: DirectoryProvider) -> some View {
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

    @ViewBuilder
    private func loadMoreButton(for requestedScope: Scope) -> some View {
        let result = requestedScope == .nearby ? nearbyResult : contactResult
        let loading = requestedScope == .nearby ? isLoadingNearby : isLoadingContacts
        let failure = loading ? nil : snapshot(result)?.pageLoadFailure
        if hasMore(result) {
            VStack(alignment: .leading, spacing: 6) {
                // The failed page is named beside its own retry. Keeping the loaded pages without
                // saying why the next one is missing would leave "Load more" looking as if it did
                // nothing.
                if let failure {
                    Text("The next page could not be loaded. \(failure). The providers already listed are unaffected.")
                        .font(.system(size: 12))
                        .foregroundColor(c.warning)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Button {
                    Task {
                        if requestedScope == .nearby {
                            await loadNearest(reset: false)
                        } else {
                            await loadContacts(reset: false)
                        }
                    }
                } label: {
                    HStack(spacing: 8) {
                        if loading {
                            ProgressView()
                        }
                        Text(failure == nil ? "Load more" : "Try loading more again")
                            .font(.system(size: 14, weight: .semibold))
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 11)
                    .foregroundColor(c.accent)
                    .background(RoundedRectangle(cornerRadius: 12).fill(c.surface))
                }
                .buttonStyle(.plain)
                .disabled(loading)
            }
        }
    }

    private func resultCountLabel(_ result: ProviderDirectoryResult?, loaded: Int) -> String {
        guard let total = snapshot(result)?.page?.total else { return "\(loaded)" }
        return loaded < total ? "\(loaded) of \(total)" : "\(total)"
    }

    private func hasMore(_ result: ProviderDirectoryResult?) -> Bool {
        snapshot(result)?.page?.hasMore == true
    }

    private func snapshot(_ result: ProviderDirectoryResult?) -> ProviderDirectorySnapshot? {
        switch result {
        case .found(let snapshot), .empty(let snapshot):
            return snapshot
        case .unavailable, .none:
            return nil
        }
    }

    @MainActor
    private func submitSearch() async {
        let searched = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = searched.isEmpty ? nil : searched
        if scope == .nearby {
            if case .ready = location.state {
                nearbyName = name
                await loadNearest(reset: true)
            } else {
                // Name search is a distinct position-free request. Move to the contact/name list
                // instead of acquiring or attaching a location the user did not ask this action for.
                scope = .contacts
                contactName = name
                await loadContacts(reset: true)
            }
        } else {
            contactName = name
            await loadContacts(reset: true)
        }
    }

    @MainActor
    private func loadNearest(reset: Bool) async {
        // A reset must never be dropped. The in-flight guard exists to stop a second load-MORE
        // appending the same page twice; applying it to a reset silently discarded the owner's newest
        // search while `nearbyName` had already moved, so the list rendered rows matched against a
        // different needle than the one just submitted, with nothing left to re-trigger. Supersession
        // is `requestID`'s job below, and the superseded call returns before touching the flag.
        guard reset || !isLoadingNearby,
              case .ready(let origin) = location.state else {
            return
        }
        if reset, nearbyName == nil {
            let searched = query.trimmingCharacters(in: .whitespacesAndNewlines)
            nearbyName = searched.isEmpty ? nil : searched
        }
        let offset = reset ? 0 : (snapshot(nearbyResult)?.providers.count ?? 0)
        guard let request = OwnerProviderDirectoryRequest.nearest(
            location: origin.point,
            accuracyMetres: origin.accuracyMetres,
            name: nearbyName,
            offset: offset
        ) else {
            nearbyResult = malformedPagingResult("The location request was invalid")
            return
        }

        let requestID = UUID()
        nearbyRequestID = requestID
        isLoadingNearby = true
        if reset { nearbyResult = nil }
        let incoming = await directory.read(request)
        guard nearbyRequestID == requestID else { return }
        nearbyResult = merge(
            current: nearbyResult,
            incoming: incoming,
            reset: reset,
            requiresDistance: true
        )
        // Remember what the owner just saw, and reach for the remembered set ONLY when the live read
        // could not answer at all. An `empty` IS an answer, so it is not a fallback case.
        switch incoming {
        case .found(let snapshot):
            recordCache.remember(snapshot.providers, now: Date())
        case .unavailable:
            storedRecords = recordCache.recall(now: Date())
        case .empty:
            break
        }
        isLoadingNearby = false
    }

    @MainActor
    private func loadContacts(reset: Bool) async {
        // Same rule as `loadNearest`: a reset supersedes, it is never dropped.
        guard reset || !isLoadingContacts else { return }
        if reset, contactName == nil {
            let searched = query.trimmingCharacters(in: .whitespacesAndNewlines)
            contactName = searched.isEmpty ? nil : searched
        }
        let offset = reset ? 0 : (snapshot(contactResult)?.providers.count ?? 0)
        guard let request = OwnerProviderDirectoryRequest.contacts(
            name: contactName,
            offset: offset
        ) else {
            contactResult = malformedPagingResult("The provider-name request was invalid")
            return
        }

        let requestID = UUID()
        contactRequestID = requestID
        isLoadingContacts = true
        if reset { contactResult = nil }
        let incoming = await directory.read(request)
        guard contactRequestID == requestID else { return }
        contactResult = merge(
            current: contactResult,
            incoming: incoming,
            reset: reset,
            requiresDistance: false
        )
        isLoadingContacts = false
    }

    private func merge(
        current: ProviderDirectoryResult?,
        incoming: ProviderDirectoryResult,
        reset: Bool,
        requiresDistance: Bool
    ) -> ProviderDirectoryResult {
        ProviderDirectoryPaging.merge(
            current: current,
            incoming: incoming,
            reset: reset,
            requiresDistance: requiresDistance,
            attemptedAt: Date()
        )
    }

    private func malformedPagingResult(_ detail: String) -> ProviderDirectoryResult {
        .unavailable(ProviderDirectoryUnavailable(
            source: .central,
            reason: .inconsistentSource,
            detail: detail,
            attemptedAt: Date()
        ))
    }

    private func openSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        openURL(url)
    }

    private func refreshDirectory() async {
        if scope == .nearby {
            await loadNearest(reset: true)
        } else {
            await loadContacts(reset: true)
        }
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
