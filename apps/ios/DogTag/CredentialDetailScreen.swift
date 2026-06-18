import SwiftUI

/// Credential detail sheet. Shows the verdict + dogTagId header, the on-chain bits (Merkle root,
/// issuer domain, recordType), and every decoded Merkle leaf (the underlying record fields).
struct CredentialDetailScreen: View {
    @Environment(\.dogTagColors) var c
    @Environment(\.dismiss) var dismiss
    let cred: Credential

    private var doc: WrappedDoc? { WrappedDoc(json: cred.wrappedDocJson) }
    private var fields: [WrappedDoc.DecodedField] { doc?.decodedFields() ?? [] }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                header

                // On-chain card.
                VStack(alignment: .leading, spacing: 10) {
                    Text("ON-CHAIN").font(.system(size: 11, weight: .bold)).foregroundColor(c.muted)
                    let root = (doc?.merkleRoot).flatMap { $0.isEmpty ? nil : $0 } ?? cred.credentialRoot
                    MonoCopyRow(label: "Merkle root", value: root)
                    if let domain = doc?.issuerDomain, !domain.isEmpty {
                        KeyValueRow(label: "Issuer domain", value: domain)
                    }
                    let rt = doc?.recordType.isEmpty == false ? doc!.recordType : cred.recordType
                    if !rt.isEmpty { KeyValueRow(label: "Record type", value: rt) }
                    Text("Anchored on the verification registry. Look the Merkle root up on the chain explorer to confirm validity.")
                        .font(.system(size: 11)).foregroundColor(c.muted)
                }
                .padding(16)
                .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))

                Text("Credential fields").font(.system(size: 18, weight: .bold)).foregroundColor(c.onBackground)
                if fields.isEmpty {
                    Text("No readable fields could be decoded from this credential.")
                        .font(.system(size: 13)).foregroundColor(c.muted)
                } else {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(fields) { f in
                            KeyValueRow(label: f.label, value: f.value)
                                .padding(.vertical, 8).padding(.horizontal, 12)
                        }
                    }
                    .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
                }

                if let n = doc?.obfuscatedCount, n > 0 {
                    Text("\(n) field(s) redacted (selective disclosure)")
                        .font(.system(size: 12)).foregroundColor(c.muted)
                }
                Spacer(minLength: 24)
            }
            .padding(20)
        }
        .background(c.background.ignoresSafeArea())
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Button { dismiss() } label: {
                    Image(systemName: "xmark").foregroundColor(c.onBackground)
                        .frame(width: 32, height: 32)
                        .background(Circle().fill(c.surfaceVariant))
                }.buttonStyle(.plain)
                Spacer()
            }
            HStack(alignment: .top) {
                Text(cred.title.isEmpty ? (doc?.displayTitle() ?? "Record") : cred.title)
                    .font(.system(size: 20, weight: .bold)).foregroundColor(c.onBackground)
                Spacer()
                VerdictBadge(verdict: cred.verdict)
            }
            let rt = cred.recordType.isEmpty ? (doc?.recordType ?? "") : cred.recordType
            if !rt.isEmpty { Text(rt).font(.system(size: 13)).foregroundColor(c.muted) }
            let tag = cred.dogTagId.isEmpty ? (doc?.dogTagId ?? "") : cred.dogTagId
            if !tag.isEmpty { Text("DogTag #\(tag)").font(.system(size: 13)).foregroundColor(c.muted) }
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
    }
}

private struct KeyValueRow: View {
    @Environment(\.dogTagColors) var c
    let label: String
    let value: String
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.system(size: 12, weight: .semibold)).foregroundColor(c.muted)
            Text(value).font(.system(size: 14)).foregroundColor(c.onBackground)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct MonoCopyRow: View {
    @Environment(\.dogTagColors) var c
    let label: String
    let value: String
    private var shown: String {
        value.count > 18 ? "\(value.prefix(10))…\(value.suffix(6))" : value
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.system(size: 12, weight: .semibold)).foregroundColor(c.muted)
            Button {
                UIPasteboard.general.string = value
            } label: {
                HStack {
                    Text(shown.isEmpty ? "—" : shown)
                        .font(.system(size: 13, design: .monospaced)).foregroundColor(c.onBackground)
                    Spacer()
                    if !value.isEmpty {
                        Image(systemName: "doc.on.doc").font(.system(size: 12)).foregroundColor(c.muted)
                    }
                }
            }.buttonStyle(.plain)
        }
    }
}
