import SwiftUI

/// Animated waiting view shown while the dog tag is minted on-chain (the bind returns instantly; the
/// SBT mint lands ~12-24s later as the phone polls the chain). A pulsing, glowing dog tag being forged.
private struct ForgeWaitView: View {
    @Environment(\.dogTagColors) var c
    let status: String
    var title: String = "Forging your dog tag"
    @State private var pulse = false
    var body: some View {
        VStack(spacing: 12) {
            Text("🏷️")
                .font(.system(size: 52))
                .scaleEffect(pulse ? 1.16 : 0.84)
                .opacity(pulse ? 1.0 : 0.45)
                .animation(.easeInOut(duration: 0.75).repeatForever(autoreverses: true), value: pulse)
            Text(title).font(.system(size: 15, weight: .bold)).foregroundColor(c.onBackground)
            Text(status.isEmpty ? "Minting on-chain…" : status).font(.system(size: 12)).foregroundColor(c.muted)
            ProgressView().progressViewStyle(.linear).tint(c.accent).frame(maxWidth: 240)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 18)
        .onAppear { pulse = true }
    }
}

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
    // Dog-tag issuance result + verdict (vet-issues-the-dog-tag flow).
    @State private var issued: CentralApi.DogTagIssue? = nil
    @State private var issueVerdict = ""
    @State private var issueErr = ""

    var body: some View {
        // SCAN GATE (B1): import + export both need a wallet (the device address is what the record is
        // minted to / the consent is signed with). No wallet → don't scan; point the user to Profile.
        if !Wallet.exists() {
            walletGate
        } else if scanning {
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

    private var walletGate: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("Scan").font(.system(size: 26, weight: .bold)).foregroundColor(c.onBackground)
                card {
                    Text("Create your wallet first").font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
                    Text("You need an embedded wallet before you can import or export records. Go to Profile → Create embedded wallet.")
                        .font(.system(size: 12)).foregroundColor(c.muted)
                }
                Button(action: onDone) {
                    Text("Back").foregroundColor(c.onAccent).padding(.horizontal, 16).padding(.vertical, 10)
                        .background(Capsule().fill(c.accent))
                }.buttonStyle(.plain)
                Spacer(minLength: 24)
            }
            .padding(20)
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
                case let .dogTagIssueSession(host, token):
                    issuePanel(host: host, token: token)
                case let .unknown(raw):
                    card {
                        Text("Unrecognised QR").font(.system(size: 15, weight: .bold)).foregroundColor(c.danger)
                        Text("This isn't a DogTag record link (/r/<token> or /r?t=), dog-tag issuance (/p/<token>) or export session (/x/<token>).").font(.system(size: 12)).foregroundColor(c.muted)
                        Text(String(raw.prefix(120))).font(.system(size: 11, design: .monospaced)).foregroundColor(c.muted)
                    }
                case .none:
                    EmptyView()
                }

                HStack(spacing: 10) {
                    Button { status = ""; payload = nil; selected = nil; exportSession = nil; exportResolveErr = nil; issued = nil; issueVerdict = ""; issueErr = ""; scanning = true } label: {
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

    // ---- issue (vet-issues-the-dog-tag) ----

    @ViewBuilder
    private func issuePanel(host: String, token: String) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            card {
                Text("Issue dog tag").font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
                Text("From \(host)").font(.system(size: 12)).foregroundColor(c.muted)
                Text("Token \(String(token.prefix(18)))…").font(.system(size: 11, design: .monospaced)).foregroundColor(c.muted)
                Text("Your vet will bind a new dog tag to this wallet. We'll sign the binding, then verify the issued profile against the DogTagSBT (profileRoot + ownerOf) before storing it.")
                    .font(.system(size: 12)).foregroundColor(c.muted)

                if issued == nil && !working {
                    Button { bindIssue(host: host, token: token) } label: {
                        Text("Issue & verify").frame(maxWidth: .infinity).padding(.vertical, 12)
                            .foregroundColor(c.onAccent).background(RoundedRectangle(cornerRadius: 12).fill(c.accent))
                    }
                }
                if working {
                    ForgeWaitView(status: status)
                }
                if !working && !status.isEmpty {
                    Text(status).font(.system(size: 12)).foregroundColor(issueVerdict == "VALID" ? c.success : c.muted)
                }
                if !issueErr.isEmpty { Text(issueErr).font(.system(size: 12)).foregroundColor(c.danger) }
            }

            if let res = issued {
                card {
                    Text("Dog tag issued").font(.system(size: 16, weight: .bold)).foregroundColor(c.success)
                    field("dogTagId", res.dogTagId.isEmpty ? "—" : res.dogTagId)
                    field("Verdict", issueVerdict.isEmpty ? "—" : issueVerdict)
                    field("Root", (res.root.isEmpty ? "—" : String(res.root.prefix(18))) + "…")
                    field("Tx", (res.txHash.isEmpty ? "—" : String(res.txHash.prefix(18))) + "…")
                    Text("Stored under your dog tags.").font(.system(size: 12)).foregroundColor(c.muted)
                }
            }
        }
    }

    private func bindIssue(host: String, token: String) {
        issueErr = ""
        Biometric.authenticate(reason: "Authenticate to bind this dog tag to your wallet") { ok, e in
            guard ok else { issueErr = e ?? "auth failed"; return }
            guard let wallet = (try? Wallet.load()) ?? nil else {
                issueErr = "Create your wallet first (Profile)."; return
            }
            working = true
            status = "Binding dog tag…"
            let sig = wallet.registerSignature()
            let addr = wallet.ethAddress
            let roax = RoaxConfig.load()
            Task {
                guard let res = await CentralApi.bindDogTagIssue(host: host, token: token, walletAddress: addr, signature: sig) else {
                    await MainActor.run { working = false; issueErr = "Bind failed (expired token / network)." }
                    return
                }
                // The bind responds immediately (status "minting") and the vet mints the SBT in the
                // background. Poll the chain (profileRoot + ownerOf) until the mint lands — retrying a
                // miss rather than failing on the first read.
                await MainActor.run { status = "Minting your dog tag on-chain…" }
                let poll = await RecordImporter.pollSbtMint(
                    dogTagId: res.dogTagId, expectedRoot: res.root, walletAddress: addr,
                    dogTagSbt: roax.dogTagSbt, rpcUrl: AppConfig.roaxRpc)
                if case .timeout = poll {
                    await MainActor.run { working = false; issueErr = "Mint not confirmed — check the vet portal." }
                    return
                }
                await MainActor.run { status = "Verifying against DogTagSBT…" }
                let r = await RecordImporter.verifyIssuedDogTag(
                    wrappedDocJson: res.wrappedDocJson, dogTagId: res.dogTagId, expectedRoot: res.root,
                    walletAddress: addr, dogTagSbt: roax.dogTagSbt, rpcUrl: AppConfig.roaxRpc)
                await MainActor.run {
                    working = false
                    if let cred = r.credential {
                        store.addCredential(cred)
                        issued = res
                        issueVerdict = r.verdict
                        status = "Issued (\(r.verdict)) — \(r.detail)"
                    } else {
                        issueErr = "Verify failed: \(r.detail)"
                    }
                }
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
                if working {
                    ForgeWaitView(status: status.isEmpty ? "Recording your verification on-chain…" : status,
                                  title: "Recording your verification on-chain")
                }
                Button { presentExport(host: host, token: token, groomerAddr: groomerAddr, sess: sess) } label: {
                    Text(working ? "Working…" : "Approve & export").frame(maxWidth: .infinity).padding(.vertical, 12)
                        .foregroundColor(.white).background(RoundedRectangle(cornerRadius: 12).fill(c.success))
                }
                .disabled(selected == nil || working)
                // While working the ForgeWaitView already surfaces the live status; show the plain
                // status text only when idle (the final success/timeout line).
                if !working && !status.isEmpty {
                    Text(status).font(.system(size: 12))
                        .foregroundColor(status.hasPrefix("Verified on-chain") ? c.success : c.muted)
                }
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
                    // The witness graph (`verification.graph`) is the pure-Rust circom-witnesscalc
                    // input — loaded by absolute path exactly like the zkey. Mirrors Android
                    // ZkeyAsset.ensureGraph().
                    guard let graphPath = ZkeyAsset.ensureGraph() else {
                        await MainActor.run { working = false; status = "witness graph missing from bundle." }
                        return
                    }
                    await MainActor.run { status = "Generating proof…" }
                    let eddsaInput = EddsaSigInput(
                        r8xDec: eddsa.r8xDec, r8yDec: eddsa.r8yDec, sDec: eddsa.sDec,
                        axHex: wallet.consent.axHex, ayHex: wallet.consent.ayHex)
                    let proof = try proveVerification(
                        wrappedDocJson: sel.wrappedDocJson, consentJson: eddsaConsentJson(req),
                        eddsaSig: eddsaInput, zkeyPath: zkeyUrl.path, graphPath: graphPath)

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

                    // The proof's nullifier = pubSignals[4] (a decimal field element). This is the
                    // on-chain `VerificationRegistry.consumed(bytes32)` key — the canonical completion
                    // signal we poll the CHAIN for below.
                    let nullifier = proof.pubSignals.count > 4 ? proof.pubSignals[4] : ""

                    // PRE-SUBMIT REPLAY GUARD: if this nullifier is already consumed on-chain, the
                    // verification was recorded before — submitting again is a doomed replay the relayer
                    // will reject. Stop early with a clear message (mirrors Android ScanScreen).
                    let alreadyRecorded = await RoaxRpc.consumed(
                        rpcUrl: AppConfig.roaxRpc, verificationRegistry: roax.verificationRegistry,
                        nullifier: nullifier)
                    if alreadyRecorded {
                        await MainActor.run { working = false; status = "This verification was already recorded." }
                        return
                    }

                    // (d) SUBMIT to the QR host (groomer), NOT central. The one-time exportToken is
                    // consumed server-side on record-success. The submit now returns 200
                    // {status:"recording"} FAST and records on-chain in the background, so we ALWAYS
                    // proceed to the chain poll — even on a non-2xx/network-failure submit response
                    // (the relayer may still be recording). Only a clearly-rejected 4xx carrying an
                    // {error} body is a hard failure.
                    await MainActor.run { status = "Submitting proof to groomer…" }
                    let signed = try ConsentSigner.sign(req, consentPrivHex: wallet.consent.prvHex, proof: proof, bind: bind)
                    let r = await CentralApi.postVerifyConsentToHost(host: host, payloadJson: signed.payloadJson)
                    if (400..<500).contains(r.code),
                       let d = r.body.data(using: .utf8),
                       let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any],
                       let rejectMsg = o["error"] as? String, !rejectMsg.isEmpty {
                        await MainActor.run { working = false; status = "Submit rejected (\(rejectMsg))." }
                        return
                    }
                    // Otherwise (2xx, network failure, or a 4xx without an {error} body) proceed to the
                    // chain poll — the canonical success signal is consumed(nullifier).

                    // (e) POLL THE CHAIN until VerificationRegistry.consumed(nullifier) == true. Do NOT
                    // rely SOLELY on the token-gated session poll: the export token is consumed at
                    // record-success, so that GET starts returning 401 exactly when it succeeds. Poll
                    // every ~3s up to ~120s.
                    // The export token is consumed server-side only on record-SUCCESS, so the
                    // token-gated session poll keeps returning until success/error. We poll it
                    // ALONGSIDE the chain: a bind/record failure flips the session to status="error"
                    // (the reason is carried in txHash/message) and would otherwise never set
                    // consumed(nullifier)=true — leaving the phone stuck until the ~120s timeout. On
                    // error we STOP and surface it immediately (mirrors Android ScanScreen).
                    await MainActor.run { status = "Recording your verification on-chain…" }
                    var done = false
                    var failedMsg: String? = nil
                    for _ in 0..<40 {
                        if await RoaxRpc.consumed(rpcUrl: AppConfig.roaxRpc, verificationRegistry: roax.verificationRegistry, nullifier: nullifier) {
                            done = true
                            break
                        }
                        if let st = await CentralApi.verifySessionStatus(host: host, sessionId: sessionId, token: token),
                           st.status == "error" {
                            let reason = st.txHash?.isEmpty == false ? st.txHash! : "recording failed"
                            failedMsg = reason
                            break
                        }
                        try? await Task.sleep(nanoseconds: 3_000_000_000)
                    }
                    if let failedMsg = failedMsg {
                        await MainActor.run { working = false; status = "Verification failed: \(failedMsg)" }
                        return
                    }
                    if done {
                        // Optional: one best-effort session read to surface a txHash for display.
                        let tx = await CentralApi.verifySessionStatus(host: host, sessionId: sessionId, token: token)?.txHash
                        await MainActor.run {
                            working = false
                            if let tx = tx, !tx.isEmpty {
                                status = "Verified on-chain — no data disclosed. tx \(String(tx.prefix(14)))…"
                            } else {
                                status = "Verified on-chain — no data disclosed."
                            }
                        }
                    } else {
                        await MainActor.run { working = false; status = "Submitted; awaiting confirmation." }
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
