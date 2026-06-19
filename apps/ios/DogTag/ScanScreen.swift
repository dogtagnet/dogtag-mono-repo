import SwiftUI

/// The single scan entry point for the user app. The owner ONLY scans — there is no QR display.
/// A scanned QR routes to one of two outcomes (architecture §7, impl §3.9 / §6.5):
///   - Import a record (issuer -> user): fetch the wrapped doc, verify, store under the pet.
///   - Export (user -> groomer): pick which stored record to present, DNS-verify the groomer, prove
///     on-device, POST the proof to the groomer host.
struct ScanScreen: View {
    @Environment(\.dogTagColors) var c
    @ObservedObject private var store = LocalStore.shared
    let onDone: () -> Void

    @State private var scanning = true
    @State private var payload: QrPayload? = nil
    @State private var status = ""
    @State private var working = false
    @State private var selected: Credential? = nil
    // Export-session metadata resolved (non-consuming) from the QR's one-time token.
    @State private var exportSession: CentralApi.ExportSession? = nil
    @State private var exportResolveErr: String? = nil

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
                case let .importRecordToken(host, token):
                    importPanel(host: host, recordId: token)
                case let .exportSession(host, token, groomerAddr):
                    exportPanel(host: host, token: token, groomerAddr: groomerAddr)
                case let .unknown(raw):
                    card {
                        Text("Unrecognised QR").font(.system(size: 15, weight: .bold)).foregroundColor(c.danger)
                        Text("This isn't a DogTag record link (/r/<token> or /r?t=) or export session (/x/<token>).").font(.system(size: 12)).foregroundColor(c.muted)
                        Text(String(raw.prefix(120))).font(.system(size: 11, design: .monospaced)).foregroundColor(c.muted)
                    }
                case .none:
                    EmptyView()
                }

                HStack(spacing: 10) {
                    Button { status = ""; payload = nil; selected = nil; exportSession = nil; exportResolveErr = nil; scanning = true } label: {
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

    // ---- export ----

    private func exportPanel(host: String, token: String, groomerAddr: String) -> some View {
        AnyView(exportPanelBody(host: host, token: token, groomerAddr: groomerAddr)
            .task(id: token) {
                // Resolve the export-session metadata from the one-time token (non-consuming GET /x/<token>).
                exportSession = nil; exportResolveErr = nil
                guard let s = await CentralApi.resolveExportSession(host: host, token: token) else {
                    exportResolveErr = "Could not resolve export session (expired or offline)."; return
                }
                // (b) The QR-claimed groomer address must match the session relayer — hard-stop on mismatch.
                if s.relayer.lowercased() != groomerAddr.lowercased() {
                    exportResolveErr = "Groomer address mismatch — refusing to present."; return
                }
                exportSession = s
            })
    }

    @ViewBuilder
    private func exportPanelBody(host: String, token: String, groomerAddr: String) -> some View {
        if let sess = exportSession {
            let wantGroup = CredentialGroup.from(recordType: sess.recordType)
            let matching = store.credentials.filter { $0.group == wantGroup }
            let candidates = matching.isEmpty ? store.credentials : matching
            VStack(alignment: .leading, spacing: 14) {
                card {
                    Text("Export request").font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
                    field("Groomer", sess.relayer.isEmpty ? "Unknown" : sess.relayer)
                    field("Purpose", sess.purpose.isEmpty ? "—" : sess.purpose)
                    field("Record type", sess.recordType.isEmpty ? "any" : sess.recordType)
                    field("Mode", (sess.mode.lowercased() == "normal" || sess.mode.lowercased() == "ecdsa") ? "ECDSA (EIP-712)" : "Zero-knowledge")
                }
                card {
                    Text("Select the record to export").font(.system(size: 15, weight: .bold)).foregroundColor(c.onBackground)
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
                Button { presentExport(host: host, token: token, groomerAddr: groomerAddr, sess: sess) } label: {
                    Text(working ? "Working…" : "Approve & export").frame(maxWidth: .infinity).padding(.vertical, 12)
                        .foregroundColor(.white).background(RoundedRectangle(cornerRadius: 12).fill(c.success))
                }
                .disabled(selected == nil || working)
                if !status.isEmpty { Text(status).font(.system(size: 12)).foregroundColor(c.muted) }
            }
        } else {
            card {
                Text("Export request").font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
                Text(exportResolveErr ?? "Resolving export session…")
                    .font(.system(size: 12)).foregroundColor(exportResolveErr != nil ? c.danger : c.muted)
            }
        }
    }

    private func presentExport(host: String, token: String, groomerAddr: String, sess: CentralApi.ExportSession) {
        guard let sel = selected else { status = "Select a record first."; return }
        let relayer = sess.relayer, purpose = sess.purpose, mode = sess.mode, sessionId = sess.sessionId
        let isZk = !(mode.lowercased() == "normal" || mode.lowercased() == "ecdsa")
        Biometric.authenticate(reason: "Present '\(sel.title)' to \(relayer.isEmpty ? "the groomer" : relayer)") { ok, e in
            guard ok else { status = e ?? "auth failed"; return }
            let wallet: WalletIdentity? = (try? Wallet.load()) ?? nil
            let subject = wallet?.ethAddress
            let req = VerificationRequest.from(
                exportToken: token, relayer: sess.relayer, purpose: sess.purpose,
                recordType: sess.recordType, challenge: sess.challenge, mode: sess.mode,
                dogTagIdDec: sel.dogTagId, credentialRoot: sel.credentialRoot,
                subjectWallet: subject, callbackUrl: "\(AppConfig.centralApi)/v1/verify/consent")

            if !isZk {
                // ECDSA (legacy) path — relay through central as before.
                Task {
                    do {
                        let signed = try ConsentSigner.sign(req, consentPrivHex: nil)
                        await MainActor.run { status = "Signed (\(signed.mode.rawValue)); submitting…" }
                        let r = await CentralApi.postConsent(sessionToken: AppConfig.sessionToken, payloadJson: signed.payloadJson)
                        await MainActor.run {
                            status = r.code < 0 ? "Signed locally; submit failed (no network / session)."
                                                : "POST /v1/verify/consent → \(r.code)"
                        }
                    } catch { await MainActor.run { status = "sign failed: \(error)" } }
                }
                return
            }

            // -------- ZERO-KNOWLEDGE on-device path --------
            guard let wallet = wallet else { status = "Create your wallet first (Profile)."; return }
            working = true
            let roax = RoaxConfig.load()
            Task {
                do {
                    // (a) PRE-PROOF GROOMER CHECK — hard-stop if the relayer is not whitelisted.
                    await MainActor.run { status = "Checking groomer authorization…" }
                    let verifyKey = verifyWhitelistKeyHex(purposeLabel: purpose)
                    let wl = await RoaxRpc.isWhitelistedFor(
                        rpcUrl: AppConfig.roaxRpc, issuerRegistry: roax.issuerRegistry,
                        key: verifyKey, signer: relayer)
                    guard case .valid = wl else {
                        await MainActor.run {
                            working = false
                            status = "This groomer is not authorized (not whitelisted)."
                        }
                        return
                    }

                    // (a2) DNS VERIFY (prod/remote only) — the groomer's domain must publish a TXT
                    // `dogtag-verify=<groomerAddr>`. Local hosts skip this entirely.
                    if !DnsVerify.isLocalHost(host) {
                        await MainActor.run { status = "Verifying groomer DNS…" }
                        let dnsOk = await DnsVerify.verifyGroomer(host: host, groomerAddr: groomerAddr)
                        if !dnsOk {
                            await MainActor.run {
                                working = false
                                status = "Groomer DNS not verified — refusing to present."
                            }
                            return
                        }
                    }

                    // (b) Sign EdDSA consent + generate the Groth16 proof on-device.
                    let eddsa = try signConsentEddsa(
                        prvHex: wallet.consent.prvHex,
                        dogTagIdHex: req.dogTagId, recordTypeHex: req.recordType, purposeHex: req.purpose,
                        credentialRootHex: req.credentialRoot, challengeHex: req.challenge,
                        relayerHex: req.relayer, subjectHex: req.subject, nonceHex: req.nonce, deadlineHex: req.deadline)
                    guard let zkeyUrl = Bundle.main.url(forResource: "verification_final", withExtension: "zkey") else {
                        await MainActor.run { working = false; status = "proving key missing from bundle." }
                        return
                    }
                    await MainActor.run { status = "Generating proof…" }
                    let eddsaInput = EddsaSigInput(
                        r8xDec: eddsa.r8xDec, r8yDec: eddsa.r8yDec, sDec: eddsa.sDec,
                        axHex: wallet.consent.axHex, ayHex: wallet.consent.ayHex)
                    let proof = try proveVerification(
                        wrappedDocJson: sel.wrappedDocJson, consentJson: eddsaConsentJson(req),
                        eddsaSig: eddsaInput, zkeyPath: zkeyUrl.path)

                    // (c) CONSENT-KEY BIND (gasless) — owner signs the EIP-712 digest; relayer submits.
                    var bind: ConsentKeyBind? = nil
                    let nonce = await RoaxRpc.bindNonce(
                        rpcUrl: AppConfig.roaxRpc, consentKeyRegistry: roax.consentKeyRegistry,
                        subject: wallet.ethAddress) ?? 0
                    if let digestHex = try? bindConsentKeyDigestHex(
                        consentKeyRegistryAddr: roax.consentKeyRegistry, keyHashHex: wallet.consent.keyHashHex,
                        walletAddr: wallet.ethAddress, nonce: nonce, chainId: UInt64(roax.chainId)) {
                        let ownerSig = wallet.signEthDigest(hexToData(digestHex))
                        bind = ConsentKeyBind(subject: wallet.ethAddress, keyHash: wallet.consent.keyHashHex, ownerSig: ownerSig)
                    }

                    // (d) SUBMIT to the QR host (groomer), NOT central. The one-time exportToken is
                    // consumed server-side on submit.
                    await MainActor.run { status = "Submitting proof to groomer…" }
                    let signed = try ConsentSigner.sign(req, consentPrivHex: wallet.consent.prvHex, proof: proof, bind: bind)
                    let r = await CentralApi.postVerifyConsentToHost(host: host, payloadJson: signed.payloadJson)
                    guard r.ok else {
                        await MainActor.run { working = false; status = "Submit failed (\(r.code))." }
                        return
                    }

                    // (e) POLL session status until recorded → show txHash.
                    await MainActor.run { status = "Proof submitted — waiting for on-chain record…" }
                    var recorded = false
                    for _ in 0..<30 {
                        if let st = await CentralApi.verifySessionStatus(host: host, sessionId: sessionId, token: token),
                           st.status == "recorded" {
                            recorded = true
                            let tx = st.txHash.map { String($0.prefix(14)) } ?? ""
                            await MainActor.run { status = "Verified on-chain — no data disclosed. tx \(tx)…" }
                            break
                        }
                        try? await Task.sleep(nanoseconds: 2_000_000_000)
                    }
                    await MainActor.run {
                        working = false
                        if !recorded { status = "Submitted; awaiting confirmation (poll timed out)." }
                    }
                } catch {
                    await MainActor.run { working = false; status = "ZK verify failed: \(error)" }
                }
            }
        }
    }

    /// The canonical §1.10 consent JSON the Rust prover consumes for `proveVerification`.
    private func eddsaConsentJson(_ req: VerificationRequest) -> String {
        let o: [String: Any] = [
            "dogTagId": req.dogTagId, "recordType": req.recordType, "purpose": req.purpose,
            "credentialRoot": req.credentialRoot, "challenge": req.challenge, "relayer": req.relayer,
            "subject": req.subject, "nonce": req.nonce, "deadline": req.deadline,
        ]
        return String(data: (try? JSONSerialization.data(withJSONObject: o)) ?? Data(), encoding: .utf8) ?? "{}"
    }

    private func hexToData(_ hex: String) -> Data {
        var h = hex.hasPrefix("0x") ? String(hex.dropFirst(2)) : hex
        if h.count % 2 != 0 { h = "0" + h }
        var out = Data()
        var i = h.startIndex
        while i < h.endIndex {
            let next = h.index(i, offsetBy: 2)
            if let b = UInt8(h[i..<next], radix: 16) { out.append(b) }
            i = next
        }
        return out
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
