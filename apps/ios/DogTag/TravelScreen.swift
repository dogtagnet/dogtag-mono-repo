import SwiftUI

private struct DocType: Identifiable {
    let id = UUID()
    let title: String
    let subtitle: String
    let detail: String
}

struct TravelScreen: View {
    @Environment(\.dogTagColors) var c
    @State private var selected = 0

    private let types = [
        DocType(title: "CDC Dog Import Form",
                subtitle: "Required for U.S. entry (as of Aug 2024)",
                detail: "Required for all dogs entering the United States as of August 1, 2024. Dogs must be at least 6 months old and have a microchip."),
        DocType(title: "DOT Service Dog Form",
                subtitle: "DOT Service Animal Air Transportation Form",
                detail: "Required for flying with a service animal on U.S. airlines. Airlines must receive this form at least 48 hours before departure."),
        DocType(title: "Other Document",
                subtitle: "Other travel document",
                detail: "Add any other travel-related document for your dog."),
    ]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                HStack {
                    ZStack {
                        Circle().fill(c.travelTint).frame(width: 36, height: 36)
                        Image(systemName: "airplane").foregroundColor(c.accent).font(.system(size: 16))
                    }
                    VStack(alignment: .leading, spacing: 1) {
                        Text("Add Travel Document").font(.system(size: 20, weight: .bold)).foregroundColor(c.onBackground)
                        Text("for \(DemoData.pet.name)").font(.system(size: 12)).foregroundColor(c.muted)
                    }
                }
                Text("Document Type").font(.system(size: 22, weight: .bold)).foregroundColor(c.onBackground)
                Text("What type of travel document are you adding?").font(.system(size: 13)).foregroundColor(c.muted)

                ForEach(Array(types.enumerated()), id: \.offset) { i, t in
                    DocRow(title: t.title, subtitle: t.subtitle, detail: t.detail, selected: selected == i) {
                        selected = i
                    }
                }

                Button {
                } label: {
                    Text("Continue to \(types[selected].title.components(separatedBy: " ").first ?? "") Form")
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 12)
                        .foregroundColor(c.onAccent)
                        .background(RoundedRectangle(cornerRadius: 12).fill(c.accent))
                }
                .padding(.top, 4)

                Spacer(minLength: 24)
            }
            .padding(20)
        }
    }
}

private struct DocRow: View {
    @Environment(\.dogTagColors) var c
    let title: String
    let subtitle: String
    let detail: String
    let selected: Bool
    let onTap: () -> Void

    var body: some View {
        Button(action: onTap) {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Image(systemName: "doc.text").foregroundColor(c.accent).font(.system(size: 18))
                    VStack(alignment: .leading, spacing: 1) {
                        Text(title).font(.system(size: 15, weight: .semibold)).foregroundColor(c.onBackground)
                        Text(subtitle).font(.system(size: 11)).foregroundColor(c.muted)
                    }
                    Spacer()
                    if selected { Image(systemName: "checkmark.circle.fill").foregroundColor(c.accent) }
                }
                if selected { Text(detail).font(.system(size: 12)).foregroundColor(c.muted) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(14)
            .background(RoundedRectangle(cornerRadius: 14).fill(selected ? c.accent.opacity(0.14) : c.surface))
            .overlay(
                RoundedRectangle(cornerRadius: 14)
                    .stroke(selected ? c.accent : c.outline, lineWidth: selected ? 1.5 : 1)
            )
        }
        .buttonStyle(.plain)
    }
}
