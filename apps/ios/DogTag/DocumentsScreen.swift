import SwiftUI

struct DocumentsScreen: View {
    @Environment(\.dogTagColors) var c
    @State private var shareOf: Credential? = nil

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Documents").font(.system(size: 26, weight: .bold)).foregroundColor(c.onBackground)
                SectionTitle(text: "All records", trailing: "\(DemoData.credentials.count)")

                ForEach(DemoData.credentials) { cred in
                    Button { shareOf = cred } label: {
                        HStack {
                            ZStack {
                                Circle().fill(c.surfaceVariant).frame(width: 38, height: 38)
                                Image(systemName: "doc.text").foregroundColor(c.accent).font(.system(size: 16))
                            }
                            VStack(alignment: .leading, spacing: 1) {
                                Text(cred.title).font(.system(size: 14, weight: .semibold)).foregroundColor(c.onBackground)
                                Text("\(cred.group.title) · \(cred.recordType)").font(.system(size: 12)).foregroundColor(c.muted)
                            }
                            Spacer()
                            Image(systemName: "qrcode").foregroundColor(c.muted)
                        }
                        .padding(14)
                        .background(RoundedRectangle(cornerRadius: 14).fill(c.surface))
                    }
                    .buttonStyle(.plain)
                }
                Spacer(minLength: 24)
            }
            .padding(20)
        }
        .sheet(item: $shareOf) { cred in
            ShareSheet(cred: cred)
                .environment(\.dogTagColors, c)
        }
    }
}

private struct ShareSheet: View {
    @Environment(\.dogTagColors) var c
    @Environment(\.dismiss) var dismiss
    let cred: Credential

    private var payload: String {
        let obj: [String: Any] = [
            "type": "dogtag.credential.share",
            "dogTagId": DemoData.pet.dogTagId,
            "credentialId": cred.id,
            "title": cred.title,
            "recordType": cred.recordType,
            "issuer": cred.issuer,
            "issuedOn": cred.issuedOn,
        ]
        let data = (try? JSONSerialization.data(withJSONObject: obj)) ?? Data()
        return String(data: data, encoding: .utf8) ?? "{}"
    }

    var body: some View {
        VStack(spacing: 14) {
            HStack {
                Text("Share credential").font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
                Spacer()
                Button { dismiss() } label: { Image(systemName: "xmark").foregroundColor(c.muted) }
            }
            Text(cred.title).font(.system(size: 14)).foregroundColor(c.muted)
            QRImageView(content: payload).frame(width: 240, height: 240)
            Text("Scan to share this credential reference").font(.system(size: 12)).foregroundColor(c.muted)
            Spacer()
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(c.background.ignoresSafeArea())
    }
}
