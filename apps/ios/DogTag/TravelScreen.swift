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
                    SectionTitle(text: "Travel records", trailing: "\(travel.count)")
                    if travel.isEmpty {
                        Text("No travel records for this dog yet.").font(.system(size: 13)).foregroundColor(c.muted)
                    }
                    ForEach(travel) { cred in
                        Button { detailCred = cred } label: {
                            HStack {
                                ZStack {
                                    Circle().fill(c.surfaceVariant).frame(width: 38, height: 38)
                                    Image(systemName: "doc.text").foregroundColor(c.accent).font(.system(size: 16))
                                }
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(cred.title).font(.system(size: 14, weight: .semibold)).foregroundColor(c.onBackground)
                                    let petName = store.pets.first { $0.dogTagId == cred.dogTagId }?.name ?? "DogTag #\(cred.dogTagId)"
                                    Text("\(petName) · \(cred.recordType)").font(.system(size: 12)).foregroundColor(c.muted)
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
