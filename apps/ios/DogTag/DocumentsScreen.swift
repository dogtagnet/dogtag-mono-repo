import SwiftUI
import UniformTypeIdentifiers

struct DocumentsScreen: View {
    @Environment(\.dogTagColors) var c
    @ObservedObject private var store = LocalStore.shared
    let onScan: () -> Void
    @State private var filterPetId: String? = nil   // nil == All pets
    @State private var detailCred: Credential? = nil

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
                    SectionTitle(text: "Records", trailing: "\(shown.count)")
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
                        Button { detailCred = cred } label: {
                            HStack {
                                ZStack {
                                    Circle().fill(c.surfaceVariant).frame(width: 38, height: 38)
                                    Image(systemName: "doc.text").foregroundColor(c.accent).font(.system(size: 16))
                                }
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(cred.title).font(.system(size: 14, weight: .semibold)).foregroundColor(c.onBackground)
                                    Text("\(cred.group.title) · \(cred.recordType)").font(.system(size: 12)).foregroundColor(c.muted)
                                    let petName = store.pets.first { $0.dogTagId == cred.dogTagId }?.name ?? "DogTag #\(cred.dogTagId)"
                                    Text(petName).font(.system(size: 11)).foregroundColor(c.muted)
                                }
                                Spacer()
                                VerdictBadge(verdict: cred.verdict)
                            }
                            .padding(14)
                            .background(RoundedRectangle(cornerRadius: 14).fill(c.surface))
                        }.buttonStyle(.plain)
                    }
                }
                Spacer(minLength: 24)
            }
            .padding(20)
        }
        .sheet(item: $detailCred) { cred in
            CredentialDetailScreen(cred: cred).environment(\.dogTagColors, c)
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
        return ExportedDocument(filename: "dogtag-documents-\(creds.count).json", json: json)
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
