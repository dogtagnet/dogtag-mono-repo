import SwiftUI

/// The single scan entry point for the user app. The owner ONLY scans — there is no QR display.
/// A scanned QR routes to one of two outcomes (architecture §7, impl §3.9 / §6.5):
///   - Import a record (issuer -> user): fetch the wrapped doc, verify, store under the pet.
///   - Verify (verifier -> user): pick which stored record to present, sign consent, relay to central.
struct ScanScreen: View {
    @Environment(\.dogTagColors) var c
    @ObservedObject private var store = LocalStore.shared
    let onDone: () -> Void

    @State private var scanning = true
    @State private var payload: QrPayload? = nil
    @State private var status = ""
    @State private var working = false
    @State private var selected: Credential? = nil

    var body: some View {
        if scanning {
            ZStack(alignment: .bottom) {
                QRScannerView { raw in
                    scanning = false
                    payload = QrPayload.parse(raw)
                }
                .ignoresSafeArea()
                VStack(spacing: 8) {
                    Text("Point the camera at the vet/groomer's QR").foregroundColor(.white).font(.system(size: 13))
                    Button("Cancel") { scanning = false; onDone() }
                        .foregroundColor(.white).padding(.horizontal, 18).padding(.vertical, 8)
                        .background(Capsule().fill(c.accent))
                }
                .padding(.bottom, 40)
            }
        } else {
            content
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("Scan").font(.system(size: 26, weight: .bold)).foregroundColor(c.onBackground)

                switch payload {
                case let .importRecord(host, recordId, _):
                    importPanel(host: host, recordId: recordId)
                case .verifySession:
                    verifyPanel
                case let .unknown(raw):
                    card {
                        Text("Unrecognised QR").font(.system(size: 15, weight: .bold)).foregroundColor(c.danger)
                        Text("This isn't a DogTag record link (/r) or verify session (/v).").font(.system(size: 12)).foregroundColor(c.muted)
                        Text(String(raw.prefix(120))).font(.system(size: 11, design: .monospaced)).foregroundColor(c.muted)
                    }
                case .none:
                    EmptyView()
                }

                HStack(spacing: 10) {
                    Button { status = ""; payload = nil; selected = nil; scanning = true } label: {
                        Text("Scan again").foregroundColor(c.onBackground).padding(.horizontal, 16).padding(.vertical, 10)
                            .background(Capsule().fill(c.surfaceVariant))
                    }.buttonStyle(.plain)
                    Button(action: onDone) {
                        Text("Done").foregroundColor(c.onAccent).padding(.horizontal, 16).padding(.vertical, 10)
                            .background(Capsule().fill(c.accent))
                    }.buttonStyle(.plain)
                }
                Spacer(minLength: 24)
            }
            .padding(20)
        }
    }

    // ---- import ----

    private func importPanel(host: String, recordId: String) -> some View {
        card {
            Text("Import record").font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
            Text("From \(host)").font(.system(size: 12)).foregroundColor(c.muted)
            Text("Record \(String(recordId.prefix(18)))…").font(.system(size: 11, design: .monospaced)).foregroundColor(c.muted)
            Text("We'll fetch the wrapped document, recompute its Merkle root (offline) and re-check DogTagIssuer.isValid on ROAX before storing it under your pet.")
                .font(.system(size: 12)).foregroundColor(c.muted)
            Button {
                guard let p = payload else { return }
                working = true; status = "Fetching + verifying record…"
                Task {
                    let r = await RecordImporter.import(p)
                    await MainActor.run {
                        working = false
                        if let cred = r.credential {
                            store.addCredential(cred)
                            status = "Imported (\(r.verdict)) — \(r.detail)"
                        } else {
                            status = "Import failed: \(r.detail)"
                        }
                    }
                }
            } label: {
                Text(working ? "Working…" : "Verify & import").frame(maxWidth: .infinity).padding(.vertical, 12)
                    .foregroundColor(c.onAccent).background(RoundedRectangle(cornerRadius: 12).fill(c.accent))
            }
            .disabled(working)
            if !status.isEmpty {
                Text(status).font(.system(size: 12)).foregroundColor(status.hasPrefix("Imported (VALID") ? c.success : c.muted)
            }
        }
    }

    // ---- verify ----

    private var verifyPanel: some View {
        guard case let .verifySession(_, _, relayer, purpose, recordType, _, mode, _) = payload! else {
            return AnyView(EmptyView())
        }
        let wantGroup = CredentialGroup.from(recordType: recordType)
        let matching = store.credentials.filter { $0.group == wantGroup }
        let candidates = matching.isEmpty ? store.credentials : matching

        return AnyView(VStack(alignment: .leading, spacing: 14) {
            card {
                Text("Verification request").font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
                field("Verifier", relayer.isEmpty ? "Unknown" : relayer)
                field("Purpose", purpose.isEmpty ? "—" : purpose)
                field("Record type", recordType.isEmpty ? "any" : recordType)
                field("Mode", (mode.lowercased() == "normal" || mode.lowercased() == "ecdsa") ? "ECDSA (EIP-712)" : "Zero-knowledge")
            }
            card {
                Text("Select the record to present").font(.system(size: 15, weight: .bold)).foregroundColor(c.onBackground)
                if candidates.isEmpty {
                    Text("No matching records yet — scan a vet's QR to import one first.").font(.system(size: 12)).foregroundColor(c.muted)
                }
                ForEach(candidates) { cred in
                    let isSel = selected?.id == cred.id
                    Button { selected = cred } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 1) {
                                Text(cred.title).font(.system(size: 14, weight: .semibold)).foregroundColor(c.onBackground)
                                Text("\(cred.group.title) · \(cred.verdict)").font(.system(size: 11)).foregroundColor(c.muted)
                            }
                            Spacer()
                        }
                        .padding(12)
                        .background(RoundedRectangle(cornerRadius: 12).fill(isSel ? c.accent.opacity(0.14) : c.surfaceVariant))
                        .overlay(RoundedRectangle(cornerRadius: 12).stroke(isSel ? c.accent : .clear, lineWidth: 1.5))
                    }.buttonStyle(.plain)
                }
            }
            Button { presentSelected() } label: {
                Text("Approve & present").frame(maxWidth: .infinity).padding(.vertical, 12)
                    .foregroundColor(.white).background(RoundedRectangle(cornerRadius: 12).fill(c.success))
            }
            .disabled(selected == nil)
            if !status.isEmpty { Text(status).font(.system(size: 12)).foregroundColor(c.muted) }
        })
    }

    private func presentSelected() {
        guard let p = payload, let sel = selected else { status = "Select a record first."; return }
        guard case let .verifySession(_, _, relayer, _, _, _, mode, _) = p else { return }
        Biometric.authenticate(reason: "Present '\(sel.title)' to \(relayer.isEmpty ? "the verifier" : relayer)") { ok, e in
            guard ok else { status = e ?? "auth failed"; return }
            do {
                let wallet: WalletIdentity? = (try? Wallet.load()) ?? nil
                let subject = wallet?.ethAddress
                let isZk = !(mode.lowercased() == "normal" || mode.lowercased() == "ecdsa")
                let consentPriv: String? = isZk ? wallet?.consent.prvHex : nil
                guard let req = VerificationRequest.from(
                    session: p, dogTagIdDec: sel.dogTagId, credentialRoot: sel.credentialRoot,
                    subjectWallet: subject, callbackUrl: "\(AppConfig.centralApi)/v1/verify/consent")
                else { status = "could not build consent"; return }
                let signed = try ConsentSigner.sign(req, consentPrivHex: consentPriv)
                status = "Signed (\(signed.mode.rawValue)); submitting…"
                Task {
                    let r = await CentralApi.postConsent(sessionToken: AppConfig.sessionToken, payloadJson: signed.payloadJson)
                    await MainActor.run {
                        status = r.code < 0 ? "Signed locally; submit failed (no network / session)."
                                            : "POST /v1/verify/consent → \(r.code)"
                    }
                }
            } catch { status = "sign failed: \(error)" }
        }
    }

    // ---- helpers ----

    @ViewBuilder private func card<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 6, content: content)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(16)
            .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
    }

    private func field(_ label: String, _ value: String) -> some View {
        HStack(alignment: .top) {
            Text(label).font(.system(size: 12)).foregroundColor(c.muted).frame(width: 110, alignment: .leading)
            Text(value).font(.system(size: 12, design: .monospaced)).foregroundColor(c.onBackground)
        }
    }
}
