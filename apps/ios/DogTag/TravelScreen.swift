import SwiftUI

/// Travel tab — the dog's travel documents (CDC import form, DOT service form, USDA health cert, etc).
/// REAL imported records filtered by a per-pet selector. No mock data; legitimately empty until a
/// travel record is scanned in.
struct TravelScreen: View {
    @Environment(\.dogTagColors) var c
    @ObservedObject private var store = LocalStore.shared
    let onScan: () -> Void
    @State private var filterPetId: String? = nil
    @State private var detailCred: Credential? = nil

    private var travel: [Credential] {
        store.credentials
            .filter { $0.group == .travel }
            .filter { filterPetId == nil || $0.dogTagId == filterPetId }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                HStack {
                    ZStack {
                        Circle().fill(c.travelTint).frame(width: 36, height: 36)
                        Image(systemName: "airplane").foregroundColor(c.accent).font(.system(size: 16))
                    }
                    Text("Travel").font(.system(size: 22, weight: .bold)).foregroundColor(c.onBackground)
                }

                if !store.credentials.contains(where: { $0.group == .travel }) {
                    EmptyStateCard(
                        title: "No travel documents yet",
                        message: "Travel records (CDC import form, DOT service form, USDA health certificate, rabies certificate) appear here once a vet or USDA endorser shares them. Scan their QR to import.",
                        onScan: onScan)
                } else {
                    PetFilterRow(pets: store.pets, selectedId: filterPetId) { filterPetId = $0 }
                    HStack {
                        SectionTitle(text: "Travel records", trailing: "\(travel.count)")
                        RefreshAllButton(credentials: travel)
                    }
                    if travel.isEmpty {
                        Text("No travel records for this dog yet.").font(.system(size: 13)).foregroundColor(c.muted)
                    }
                    ForEach(travel) { cred in
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
                                VerdictBadge(cred: cred)
                                RefreshCredentialButton(cred: cred)
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
    }
}
