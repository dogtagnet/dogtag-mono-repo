import SwiftUI

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
