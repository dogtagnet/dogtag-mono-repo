import SwiftUI

struct HomeScreen: View {
    @Environment(\.dogTagColors) var c
    @ObservedObject private var store = LocalStore.shared
    let onScan: () -> Void
    @State private var expanded: CredentialGroup? = nil
    @State private var selectedPetId: String? = nil
    @State private var detailCred: Credential? = nil

    private var currentPet: Pet? {
        store.pets.first { $0.dogTagId == selectedPetId } ?? store.pets.first
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Dog Tags").font(.system(size: 26, weight: .bold)).foregroundColor(c.onBackground)
                    Spacer()
                    Button(action: onScan) {
                        ZStack {
                            Circle().fill(c.accent).frame(width: 40, height: 40)
                            Image(systemName: "qrcode.viewfinder").foregroundColor(c.onAccent)
                        }
                    }
                }

                if store.pets.isEmpty {
                    EmptyStateCard(
                        title: "No pets yet",
                        message: "Scan a vet or groomer's QR to import your dog's first verified record — your pets appear here automatically.",
                        onScan: onScan)
                } else {
                    if store.pets.count > 1 {
                        PetChips(pets: store.pets, selectedId: currentPet?.dogTagId) { selectedPetId = $0 }
                    }
                    if let pet = currentPet {
                        petIdentity(pet)
                        petPhotoCard
                        let petCreds = store.credentials.filter { $0.dogTagId == pet.dogTagId }
                        SectionTitle(text: "Credentials", trailing: "\(petCreds.count) total")
                        if petCreds.isEmpty {
                            EmptyStateCard(title: "No credentials yet",
                                           message: "Scan a vet's QR to import a record for \(pet.name).",
                                           onScan: onScan)
                        } else {
                            groupCard(.health, icon: "heart.fill", tint: c.healthTint, iconTint: c.danger, creds: petCreds)
                            groupCard(.service, icon: "shield.fill", tint: c.serviceTint, iconTint: c.success, creds: petCreds)
                            groupCard(.travel, icon: "airplane", tint: c.travelTint, iconTint: Color(hex: 0x2F6BFF), creds: petCreds)
                        }
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

    private func petIdentity(_ pet: Pet) -> some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 2) {
                Text("NAME").font(.system(size: 11, weight: .semibold)).foregroundColor(c.muted)
                Text(pet.name).font(.system(size: 22, weight: .bold)).foregroundColor(c.onBackground)
                Text("DogTag #\(pet.dogTagId)").font(.system(size: 12)).foregroundColor(c.muted)
            }
            Spacer()
            if !pet.breed.isEmpty {
                VStack(alignment: .trailing, spacing: 2) {
                    Text("BREED").font(.system(size: 11, weight: .semibold)).foregroundColor(c.muted)
                    Text(pet.breed).font(.system(size: 15, weight: .semibold)).foregroundColor(c.onBackground)
                    if !pet.ageLabel.isEmpty { Text(pet.ageLabel).font(.system(size: 12)).foregroundColor(c.muted) }
                }
            }
        }
    }

    private var petPhotoCard: some View {
        ZStack(alignment: .bottom) {
            RoundedRectangle(cornerRadius: 180).fill(c.accent.opacity(0.18)).aspectRatio(1.15, contentMode: .fit)
            Circle().fill(c.surfaceVariant)
                .overlay(Image(systemName: "pawprint.fill").font(.system(size: 64)).foregroundColor(c.accent))
                .frame(maxWidth: 220, maxHeight: 220)
            Button(action: onScan) {
                ZStack {
                    Circle().fill(c.danger).frame(width: 44, height: 44)
                    Image(systemName: "plus").foregroundColor(.white)
                }
            }
            .padding(.bottom, 8)
        }
    }

    private func groupCard(_ group: CredentialGroup, icon: String, tint: Color, iconTint: Color, creds: [Credential]) -> some View {
        let items = creds.filter { $0.group == group }
        return VStack(alignment: .leading, spacing: 10) {
            Button { expanded = (expanded == group) ? nil : group } label: {
                HStack {
                    ZStack {
                        Circle().fill(c.surface).frame(width: 38, height: 38)
                        Image(systemName: icon).foregroundColor(iconTint).font(.system(size: 18))
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(group.title).font(.system(size: 15, weight: .semibold)).foregroundColor(c.onBackground)
                        Text("\(items.count) record\(items.count == 1 ? "" : "s")").font(.system(size: 12)).foregroundColor(c.muted)
                    }
                    Spacer()
                    Image(systemName: "chevron.right").foregroundColor(c.muted)
                }
            }
            .buttonStyle(.plain)
            if expanded == group {
                ForEach(items) { cred in
                    Button { detailCred = cred } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(cred.title).font(.system(size: 14, weight: .semibold)).foregroundColor(c.onBackground)
                                Text("\(cred.recordType) · \(cred.verdict)").font(.system(size: 12)).foregroundColor(c.muted)
                                if !cred.issuer.isEmpty { Text(cred.issuer).font(.system(size: 11)).foregroundColor(c.muted) }
                            }
                            Spacer()
                            Image(systemName: "chevron.right").foregroundColor(c.muted).font(.system(size: 12))
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                        .background(RoundedRectangle(cornerRadius: 12).fill(c.surface))
                    }.buttonStyle(.plain)
                }
            }
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 16).fill(tint))
    }
}

/// A horizontal chip row used to switch the active pet (Home).
struct PetChips: View {
    @Environment(\.dogTagColors) var c
    let pets: [Pet]
    let selectedId: String?
    let onSelect: (String?) -> Void
    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(pets) { p in
                    let sel = p.dogTagId == selectedId
                    Button { onSelect(p.dogTagId) } label: {
                        Text(p.name).font(.system(size: 13, weight: .semibold))
                            .foregroundColor(sel ? c.onAccent : c.onBackground)
                            .padding(.horizontal, 14).padding(.vertical, 8)
                            .background(Capsule().fill(sel ? c.accent : c.surfaceVariant))
                    }.buttonStyle(.plain)
                }
            }
        }
    }
}

/// A reusable empty-state card (mirrors Android EmptyState).
struct EmptyStateCard: View {
    @Environment(\.dogTagColors) var c
    let title: String
    let message: String
    let onScan: () -> Void
    var body: some View {
        VStack(spacing: 10) {
            ZStack {
                Circle().fill(c.surfaceVariant).frame(width: 56, height: 56)
                Image(systemName: "qrcode.viewfinder").foregroundColor(c.accent).font(.system(size: 26))
            }
            Text(title).font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
            Text(message).font(.system(size: 13)).foregroundColor(c.muted).multilineTextAlignment(.center)
            Button(action: onScan) {
                Text("Scan a QR").font(.system(size: 13, weight: .semibold)).foregroundColor(c.onAccent)
                    .padding(.horizontal, 18).padding(.vertical, 10)
                    .background(Capsule().fill(c.accent))
            }.buttonStyle(.plain)
        }
        .frame(maxWidth: .infinity)
        .padding(20)
        .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
    }
}
