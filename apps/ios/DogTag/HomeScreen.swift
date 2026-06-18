import SwiftUI

struct HomeScreen: View {
    @Environment(\.dogTagColors) var c
    let onScan: () -> Void
    @State private var expanded: CredentialGroup? = nil

    private let pet = DemoData.pet

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                // Header row.
                HStack {
                    Text("Dog Tags").font(.system(size: 26, weight: .bold)).foregroundColor(c.onBackground)
                    Spacer()
                    Button(action: onScan) {
                        ZStack {
                            Circle().fill(c.accent).frame(width: 40, height: 40)
                            Image(systemName: "plus").foregroundColor(c.onAccent)
                        }
                    }
                }

                // Pet identity row.
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("NAME").font(.system(size: 11, weight: .semibold)).foregroundColor(c.muted)
                        Text(pet.name).font(.system(size: 22, weight: .bold)).foregroundColor(c.onBackground)
                        Text("DogTag #\(pet.dogTagId)").font(.system(size: 12)).foregroundColor(c.muted)
                    }
                    Spacer()
                    VStack(alignment: .trailing, spacing: 2) {
                        Text("BREED").font(.system(size: 11, weight: .semibold)).foregroundColor(c.muted)
                        Text(pet.breed).font(.system(size: 15, weight: .semibold)).foregroundColor(c.onBackground)
                        Text(pet.ageLabel).font(.system(size: 12)).foregroundColor(c.muted)
                    }
                }

                // Pet photo card (placeholder ring with dog glyph, accent-tinted).
                ZStack(alignment: .bottom) {
                    RoundedRectangle(cornerRadius: 180)
                        .fill(c.accent.opacity(0.18))
                        .aspectRatio(1.15, contentMode: .fit)
                    Circle()
                        .fill(c.surfaceVariant)
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

                SectionTitle(text: "Credentials", trailing: "\(DemoData.credentials.count) total")

                CredentialGroupCard(group: .health, icon: "heart.fill", tint: c.healthTint, iconTint: c.danger,
                                    expanded: expanded == .health) { toggle(.health) }
                CredentialGroupCard(group: .service, icon: "shield.fill", tint: c.serviceTint, iconTint: c.success,
                                    expanded: expanded == .service) { toggle(.service) }
                CredentialGroupCard(group: .travel, icon: "airplane", tint: c.travelTint, iconTint: Color(hex: 0x2F6BFF),
                                    expanded: expanded == .travel) { toggle(.travel) }

                Spacer(minLength: 24)
            }
            .padding(20)
        }
    }

    private func toggle(_ g: CredentialGroup) { expanded = (expanded == g) ? nil : g }
}

private struct CredentialGroupCard: View {
    @Environment(\.dogTagColors) var c
    let group: CredentialGroup
    let icon: String
    let tint: Color
    let iconTint: Color
    let expanded: Bool
    let onToggle: () -> Void

    var body: some View {
        let count = DemoData.count(for: group)
        VStack(alignment: .leading, spacing: 10) {
            Button(action: onToggle) {
                HStack {
                    ZStack {
                        Circle().fill(c.surface).frame(width: 38, height: 38)
                        Image(systemName: icon).foregroundColor(iconTint).font(.system(size: 18))
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(group.title).font(.system(size: 15, weight: .semibold)).foregroundColor(c.onBackground)
                        Text("\(count) record\(count == 1 ? "" : "s")").font(.system(size: 12)).foregroundColor(c.muted)
                    }
                    Spacer()
                    Image(systemName: "chevron.right").foregroundColor(c.muted)
                }
            }
            .buttonStyle(.plain)

            if expanded {
                ForEach(DemoData.credentials.filter { $0.group == group }) { cred in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(cred.title).font(.system(size: 14, weight: .semibold)).foregroundColor(c.onBackground)
                        Text("\(cred.recordType) · \(cred.subtitle)").font(.system(size: 12)).foregroundColor(c.muted)
                        Text("\(cred.issuer) · \(cred.issuedOn)").font(.system(size: 11)).foregroundColor(c.muted)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                    .background(RoundedRectangle(cornerRadius: 12).fill(c.surface))
                }
            }
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 16).fill(tint))
    }
}
