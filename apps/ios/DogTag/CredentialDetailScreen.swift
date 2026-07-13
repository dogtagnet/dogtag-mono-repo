import SwiftUI

/// Credential detail sheet. Shows the verdict + dogTagId header, the on-chain bits (Merkle root,
/// issuer domain, recordType), and every decoded Merkle leaf (the underlying record fields).
struct CredentialDetailScreen: View {
    @Environment(\.dogTagColors) var c
    @Environment(\.dismiss) var dismiss
    @ObservedObject private var store = LocalStore.shared
    let cred: Credential

    @State private var showReceipt = false

    private var doc: WrappedDoc? { WrappedDoc(json: cred.wrappedDocJson) }
    private var fields: [WrappedDoc.DecodedField] { doc?.decodedFields() ?? [] }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                header

                // Travel credentials get the CDC-modeled receipt as their "present to an official"
                // surface (holder-controlled selective disclosure + PII-free public-status QR).
                if cred.group == .travel {
                    Button { showReceipt = true } label: {
                        HStack {
                            Image(systemName: "doc.richtext")
                            Text("Show travel receipt").font(.system(size: 14, weight: .semibold))
                            Spacer()
                            Image(systemName: "chevron.right").font(.system(size: 12))
                        }
                        .foregroundColor(c.onAccent)
                        .padding(14)
                        .background(RoundedRectangle(cornerRadius: 14).fill(c.accent))
                    }.buttonStyle(.plain)
                }

                // Export this credential as the app's WrappedDoc JSON (holder's own data), via the
                // OS share sheet - Save to Files, AirDrop, Mail, etc.
                ShareLink(
                    item: DocumentExport.single(cred),
                    preview: SharePreview(cred.title.isEmpty ? (doc?.displayTitle() ?? "Credential") : cred.title)
                ) {
                    HStack(spacing: 12) {
                        Image(systemName: "square.and.arrow.up")
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Export full record").font(.system(size: 14, weight: .semibold))
                            Text("Full record, all fields - for your backup")
                                .font(.system(size: 11)).foregroundColor(c.muted)
                        }
                        Spacer()
                    }
                    .foregroundColor(c.accent)
                    .padding(14)
                    .background(RoundedRectangle(cornerRadius: 14).stroke(c.accent, lineWidth: 1.5))
                }
                .buttonStyle(.plain)

                // On-chain card.
                VStack(alignment: .leading, spacing: 10) {
                    Text("ON-CHAIN").font(.system(size: 11, weight: .bold)).foregroundColor(c.muted)
                    let root = (doc?.merkleRoot).flatMap { $0.isEmpty ? nil : $0 } ?? cred.credentialRoot
                    CopyableMonoRow(label: "Merkle root", value: root)
                    if let store = doc?.documentStore, !store.isEmpty {
                        CopyableMonoRow(label: "Issuer address", value: store)
                    }
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
        .sheet(isPresented: $showReceipt) {
            TravelReceiptView(cred: cred).environment(\.dogTagColors, c)
        }
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
                Text(cred.displayTypeLabel)
                    .font(.system(size: 20, weight: .bold)).foregroundColor(c.onBackground)
                Spacer()
                VerdictBadge(verdict: cred.verdict)
            }
            // Which pet: name (synced Pet or the DOG_PROFILE credential) + dogTagId — never a bare type.
            // The dogTagId stays tap-to-copy (InlineCopyText), as the operator relies on copying it.
            let tag = cred.dogTagId.isEmpty ? (doc?.dogTagId ?? "") : cred.dogTagId
            let petName = store.petDisplayName(forDogTagId: tag)
            if tag.isEmpty {
                if let n = petName, !n.isEmpty { Text(n).font(.system(size: 13)).foregroundColor(c.muted) }
            } else {
                HStack(spacing: 6) {
                    if let n = petName, !n.isEmpty {
                        Text("\(n) ·").font(.system(size: 13)).foregroundColor(c.muted)
                    }
                    InlineCopyText(text: "DogTag #\(tag)", copyValue: tag, font: .system(size: 13))
                }
            }
            if let detail = cred.vaccinationDetail { Text(detail).font(.system(size: 12)).foregroundColor(c.muted) }
            let rt = cred.recordType.isEmpty ? (doc?.recordType ?? "") : cred.recordType
            if !rt.isEmpty { Text(rt).font(.system(size: 12)).foregroundColor(c.muted) }
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

