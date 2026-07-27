import SwiftUI
import UniformTypeIdentifiers

struct DocumentsScreen: View {
    @Environment(\.dogTagColors) var c
    @ObservedObject private var store = LocalStore.shared
    @ObservedObject private var refresher = RefreshCenter.shared
    let onScan: () -> Void
    @State private var filterPetId: String? = nil   // nil == All pets
    @State private var detailCred: Credential? = nil
    @State private var pendingDelete: Credential? = nil

    private var shown: [Credential] {
        filterPetId == nil ? store.credentials : store.credentials.filter { $0.dogTagId == filterPetId }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Documents").font(.system(size: 26, weight: .bold)).foregroundColor(c.onBackground)

                if store.credentials.isEmpty {
                    EmptyStateCard(
                        title: "No documents yet",
                        message: "Scan a vet or groomer's QR to import a verified record. Imported records appear here, grouped by dog.",
                        onScan: onScan)
                } else {
                    PetFilterRow(pets: store.pets, selectedId: filterPetId) { filterPetId = $0 }
                    HStack {
                        SectionTitle(text: "Records", trailing: "\(shown.count)")
                        RefreshAllButton(credentials: shown)
                    }
                    if shown.isEmpty {
                        Text("No records for this dog yet.").font(.system(size: 13)).foregroundColor(c.muted)
                    } else {
                        // Export the held credentials the user is currently viewing (respects the pet
                        // filter) as the app's own WrappedDoc JSON, via the OS share sheet.
                        ShareLink(
                            item: DocumentExport.bundle(shown),
                            preview: SharePreview("DogTag documents (\(shown.count))")
                        ) {
                            Label("Export \(shown.count == 1 ? "document" : "documents")",
                                  systemImage: "square.and.arrow.up")
                                .font(.system(size: 13, weight: .semibold))
                                .foregroundColor(c.accent)
                        }
                    }
                    ForEach(shown) { cred in
                        HStack(alignment: .top, spacing: 10) {
                            Button { detailCred = cred } label: {
                                HStack(alignment: .top, spacing: 12) {
                                    ZStack {
                                        Circle().fill(c.surfaceVariant).frame(width: 38, height: 38)
                                        Image(systemName: "doc.text").foregroundColor(c.accent).font(.system(size: 16))
                                    }
                                    VStack(alignment: .leading, spacing: 2) {
                                        CredentialLabel(cred: cred, petName: store.petDisplayName(for: cred))
                                        if !cred.issuer.isEmpty {
                                            Text(cred.issuer).font(.system(size: 11)).foregroundColor(c.muted)
                                        }
                                        Text(cred.importedAtLabel).font(.system(size: 11)).foregroundColor(c.muted)
                                        CredentialStatusLine(cred: cred)
                                    }
                                    Spacer(minLength: 0)
                                }
                                .contentShape(Rectangle())
                            }.buttonStyle(.plain)

                            VStack(alignment: .trailing, spacing: 8) {
                                VerdictBadge(verdict: cred.verdict)
                                HStack(spacing: 2) {
                                    RefreshCredentialButton(cred: cred)
                                    DeleteCredentialButton(cred: cred) { pendingDelete = cred }
                                }
                            }
                        }
                        .padding(14)
                        .background(RoundedRectangle(cornerRadius: 14).fill(c.surface))
                    }
                }
                Spacer(minLength: 24)
            }
            .padding(20)
        }
        .sheet(item: $detailCred) { cred in
            CredentialDetailScreen(cred: cred).environment(\.dogTagColors, c)
        }
        .confirmDeleteCredential($pendingDelete) { cred in
            store.deleteCredential(id: cred.id)
        }
    }
}

/// A chip row with an "All pets" option plus one chip per dog. Shared by Travel + Documents.
struct PetFilterRow: View {
    @Environment(\.dogTagColors) var c
    let pets: [Pet]
    let selectedId: String?
    let onSelect: (String?) -> Void
    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                chip("All pets", selected: selectedId == nil) { onSelect(nil) }
                ForEach(pets) { p in chip(p.name, selected: selectedId == p.dogTagId) { onSelect(p.dogTagId) } }
            }
        }
    }

    private func chip(_ label: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(label).font(.system(size: 13, weight: .semibold))
                .foregroundColor(selected ? c.onAccent : c.onBackground)
                .padding(.horizontal, 14).padding(.vertical, 8)
                .background(Capsule().fill(selected ? c.accent : c.surfaceVariant))
        }.buttonStyle(.plain)
    }
}

/// A consistent record label shared by the Home, Documents, Travel and export-picker lists. Always
/// states WHAT the record is (record type, and for a vaccination the product + date) and WHICH pet it
/// belongs to (name + dogTagId), so a record is never presented as a bare "Dog Profile".
struct CredentialLabel: View {
    @Environment(\.dogTagColors) var c
    let cred: Credential
    /// Resolved by the caller via `LocalStore.petDisplayName(for:)`; nil => show the dogTagId alone.
    let petName: String?

    /// "<name> · DogTag #<id>", or just "DogTag #<id>" when no real name is known.
    private var petLine: String {
        let line = PetLabel.line(name: petName, dogTagId: cred.dogTagId)
        return line.isEmpty ? "Unknown dog" : line
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(cred.displayTypeLabel)
                .font(.system(size: 14, weight: .semibold)).foregroundColor(c.onBackground)
            Text(petLine).font(.system(size: 12)).foregroundColor(c.muted)
            if let detail = cred.vaccinationDetail {
                Text(detail).font(.system(size: 11)).foregroundColor(c.muted)
            }
        }
    }
}

struct VerdictBadge: View {
    @Environment(\.dogTagColors) var c
    let verdict: String
    var body: some View {
        let (bg, fg): (Color, Color) = {
            switch verdict {
            case "VALID": return (c.success.opacity(0.18), c.success)
            case "INVALID": return (c.danger.opacity(0.18), c.danger)
            default: return (c.surfaceVariant, c.muted)
            }
        }()
        return Text(verdict).font(.system(size: 10, weight: .bold)).foregroundColor(fg)
            .padding(.horizontal, 10).padding(.vertical, 4)
            .background(Capsule().fill(bg))
    }
}

// MARK: - Document export

/// A portable export of the holder's own credential(s) as the app's existing WrappedDoc JSON - the
/// exact format the app imports and verifies. Shared as a `.json` file through the OS share sheet
/// (Save to Files / AirDrop / Mail …). A single credential exports its `wrappedDocJson` verbatim; a
/// bundle exports a JSON array of those same docs (no envelope, no invented fields).
struct ExportedDocument: Transferable {
    let filename: String
    let json: String

    // FileRepresentation (iOS 16+) carries the filename via the written file URL - unlike
    // `.suggestedFileName`, which is iOS 17+ (deployment target here is 16.0).
    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(exportedContentType: .json) { doc in
            let url = FileManager.default.temporaryDirectory.appendingPathComponent(doc.filename)
            try Data(doc.json.utf8).write(to: url, options: .atomic)
            return SentTransferredFile(url)
        }
    }
}

enum DocumentExport {
    /// One credential → its WrappedDoc JSON verbatim (byte-for-byte what was imported/verified).
    static func single(_ cred: Credential) -> ExportedDocument {
        ExportedDocument(filename: filename(for: cred), json: cred.wrappedDocJson)
    }

    /// Many credentials → a JSON array of their WrappedDoc objects (re-parsed + re-serialized so the
    /// array is always well-formed even if a stored doc had stray whitespace).
    static func bundle(_ creds: [Credential]) -> ExportedDocument {
        let objects: [Any] = creds.compactMap { cred in
            guard let d = cred.wrappedDocJson.data(using: .utf8) else { return nil }
            return try? JSONSerialization.jsonObject(with: d)
        }
        let json: String
        if let data = try? JSONSerialization.data(withJSONObject: objects, options: [.prettyPrinted]),
           let s = String(data: data, encoding: .utf8) {
            json = s
        } else {
            json = "[]"
        }
        return ExportedDocument(filename: "dogtag-documents-\(objects.count).json", json: json)
    }

    private static func filename(for cred: Credential) -> String {
        let rt = cred.recordType.isEmpty ? "record" : cred.recordType.lowercased()
        let tag = cred.dogTagId.isEmpty ? "" : "-\(cred.dogTagId)"
        return "dogtag-\(sanitize(rt))\(sanitize(tag)).json"
    }

    private static func sanitize(_ s: String) -> String {
        let allowed = Set("abcdefghijklmnopqrstuvwxyz0123456789-_")
        return String(s.lowercased().map { allowed.contains($0) ? $0 : "-" })
    }
}

// ---- shared status / refresh / delete affordances (Documents, Travel, Home, detail) --------------

/// The freshness of a record's verdict, and why it reads the way it does. Three distinguishable
/// states: a check in flight, a completed check with its age, and a check that could not reach the
/// chain (which shows UNVERIFIED plus the reason, never VALID).
struct CredentialStatusLine: View {
    @Environment(\.dogTagColors) var c
    let cred: Credential
    @ObservedObject private var refresher = RefreshCenter.shared
    var fontSize: CGFloat = 11

    init(cred: Credential, fontSize: CGFloat = 11) {
        self.cred = cred
        self.fontSize = fontSize
    }

    var body: some View {
        if refresher.isChecking(cred) {
            HStack(spacing: 5) {
                ProgressView().scaleEffect(0.6).frame(width: 10, height: 10)
                Text("Checking on-chain…").font(.system(size: fontSize)).foregroundColor(c.muted)
            }
        } else {
            Text(cred.statusLine)
                .font(.system(size: fontSize))
                .foregroundColor(cred.verdict == "INVALID" ? c.danger : c.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// Re-read ONE record's status from the chain.
struct RefreshCredentialButton: View {
    @Environment(\.dogTagColors) var c
    let cred: Credential
    @ObservedObject private var refresher = RefreshCenter.shared

    var body: some View {
        let checking = refresher.isChecking(cred)
        Button {
            Task { await refresher.refresh(cred) }
        } label: {
            ZStack {
                if checking {
                    ProgressView().scaleEffect(0.7)
                } else {
                    Image(systemName: "arrow.clockwise").font(.system(size: 14, weight: .semibold))
                        .foregroundColor(c.accent)
                }
            }
            .frame(width: 32, height: 32)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(checking)
        .accessibilityLabel("Refresh \(cred.title) from the chain")
    }
}

/// Re-read every record currently listed.
struct RefreshAllButton: View {
    @Environment(\.dogTagColors) var c
    let credentials: [Credential]
    @ObservedObject private var refresher = RefreshCenter.shared

    var body: some View {
        let running = refresher.isBatchRunning
        Button {
            Task { await refresher.refreshAll(credentials) }
        } label: {
            HStack(spacing: 5) {
                if running {
                    ProgressView().scaleEffect(0.6).frame(width: 10, height: 10)
                    Text("Checking \(refresher.batchRemaining) left")
                } else {
                    Image(systemName: "arrow.clockwise").font(.system(size: 11, weight: .semibold))
                    Text("Refresh all")
                }
            }
            .font(.system(size: 12, weight: .semibold))
            .foregroundColor(c.accent)
            .padding(.horizontal, 12).padding(.vertical, 7)
            .background(Capsule().fill(c.surfaceVariant))
        }
        .buttonStyle(.plain)
        .disabled(running || credentials.isEmpty)
        .opacity(credentials.isEmpty ? 0.5 : 1)
    }
}

/// Delete affordance. The destructive act itself is always behind `confirmDeleteCredential`.
struct DeleteCredentialButton: View {
    @Environment(\.dogTagColors) var c
    let cred: Credential
    let onTap: () -> Void

    var body: some View {
        Button(action: onTap) {
            Image(systemName: "trash").font(.system(size: 14))
                .foregroundColor(c.muted)
                .frame(width: 32, height: 32)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Delete \(cred.title)")
    }
}

extension View {
    /// The single home of the delete confirmation, so the wording cannot drift between the surfaces
    /// that offer it. The copy is deliberately local-only: deleting removes this phone's copy and
    /// nothing else.
    /// The label is resolved HERE, off the shared store, rather than passed in: a caller-supplied
    /// name is what let the two surfaces name the same record differently.
    func confirmDeleteCredential(
        _ pending: Binding<Credential?>,
        onDelete: @escaping (Credential) -> Void
    ) -> some View {
        confirmationDialog(
            pending.wrappedValue.map { "Delete \($0.title)?" } ?? "Delete this document?",
            isPresented: Binding(
                get: { pending.wrappedValue != nil },
                set: { if !$0 { pending.wrappedValue = nil } }),
            titleVisibility: .visible,
            presenting: pending.wrappedValue
        ) { cred in
            Button("Delete from this phone", role: .destructive) { onDelete(cred) }
            Button("Cancel", role: .cancel) {}
        } message: { cred in
            Text(cred.deleteConfirmationMessage(petLabel: PetLabel.line(
                name: LocalStore.shared.petDisplayName(for: cred), dogTagId: cred.dogTagId)))
        }
    }
}
