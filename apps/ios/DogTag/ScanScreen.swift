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
                    CopyableMonoRow(label: "dogTagId", value: res.dogTagId, truncate: false)
                    field("Verdict", issueVerdict.isEmpty ? "—" : issueVerdict)
                    CopyableMonoRow(label: "Merkle root", value: res.root)
                    CopyableMonoRow(label: "Transaction", value: res.txHash)
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
            // Zero-knowledge export can only prove records within the circuit's leaf budget; the ECDSA
            // (EIP-712) path has no such limit. Gate the "too many fields" state on the ZK mode only.
            let isZk = sess.isZk
            VStack(alignment: .leading, spacing: 14) {
                card {
                    Text("Export request").font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
                    field("Groomer", sess.relayer.isEmpty ? "Unknown" : sess.relayer)
                    field("Purpose", sess.purpose.isEmpty ? "—" : sess.purpose)
                    field("Record type", sess.recordType.isEmpty ? "any" : sess.recordType)
                    field("Mode", sess.isZk ? "Zero-knowledge" : "ECDSA (EIP-712)")
                }
                card {
                    Text("Select the record to export").font(.system(size: 15, weight: .bold)).foregroundColor(c.onBackground)
                    if candidates.isEmpty {
                        Text("No matching records yet — scan a vet's QR to import one first.").font(.system(size: 12)).foregroundColor(c.muted)
                    }
                    ForEach(candidates) { cred in
                        let tooManyFields = isZk && cred.exceedsZkLeafLimit
                        let isSel = selected?.id == cred.id
                        Button { selected = cred } label: {
                            VStack(alignment: .leading, spacing: 6) {
                                HStack(alignment: .top) {
                                    CredentialLabel(cred: cred, petName: store.petDisplayName(for: cred))
                                    Spacer()
                                    VerdictBadge(verdict: cred.verdict)
                                }
                                if tooManyFields {
                                    HStack(spacing: 5) {
                                        Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 10))
                                        Text("Can't be privately verified — too many fields (\(cred.leafCount) of max \(ZkCircuit.maxLeaves)).")
                                            .font(.system(size: 11))
                                    }
                                    .foregroundColor(c.danger)
                                }
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .opacity(tooManyFields ? 0.6 : 1)
                            .padding(12)
                            .background(RoundedRectangle(cornerRadius: 12).fill(isSel ? c.accent.opacity(0.14) : c.surfaceVariant))
                            .overlay(RoundedRectangle(cornerRadius: 12).stroke(isSel ? c.accent : .clear, lineWidth: 1.5))
                        }
                        .buttonStyle(.plain)
                        .disabled(tooManyFields)
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
        let relayer = sess.relayer, purpose = sess.purpose, sessionId = sess.sessionId
        let isZk = sess.isZk
        // Belt-and-suspenders behind the disabled picker row: never hand a record that exceeds the ZK
        // circuit's leaf budget to the prover — it would abort with `too many leaves`. Fail clearly first.
        if isZk && sel.exceedsZkLeafLimit {
            status = "This record can't be privately verified — too many fields (\(sel.leafCount) of max \(ZkCircuit.maxLeaves)). Pick a record with fewer fields."
            return
        }
        Biometric.authenticate(reason: "Present '\(sel.displayTypeLabel)' to \(relayer.isEmpty ? "the groomer" : relayer)") { ok, e in
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

            // -------- LEVEL-B owner-hidden path: validate, prove, submit, and read back --------
            // Gated on the STORED `mode`, EXPLICITLY — NOT `isZk`, which is true for any non-normal/ecdsa
            // mode and would drop a `levelb` session into the Level-A prover below. Every ProtocolRegistry
            // eth_call and the `validateDiscovery` call live inside this branch, so a non-levelb session
            // makes ZERO registry calls and the Level-A path is byte-for-byte untouched (the acceptance bar).
            if AnchorResolver.isLevelB(mode: sess.mode) {
                working = true
                let roax = RoaxConfig.load()
                Task {
                    await runLevelBFlow(
                        sess: sess, roax: roax, credential: sel, req: req,
                        host: host, token: token, groomerAddr: groomerAddr)
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

                    // The proof's nullifier (a decimal field element). This is the on-chain
                    // `VerificationRegistry.consumed(bytes32)` key — the canonical completion signal we
                    // poll the CHAIN for below. LEVEL-A index: this screen proves with the bundled
                    // Level-A zkey. Under Level-B that same slot is `R`, so a bare `pubSignals[4]`
                    // here would poll a key that is never set and hang on a SUCCESSFUL verification.
                    let nfIdx = PublicSignalIndex.levelA.nullifier
                    let nullifier = proof.pubSignals.count > nfIdx ? proof.pubSignals[nfIdx] : ""

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

    /// M-4 PR4 — the owner-hidden (`mode == "levelb"`) consent flow.
    ///
    /// The explicit branch preserves the Level-A path below verbatim. It resolves and validates the
    /// on-chain anchor, rebuilds the profile witness from the M-2b store, calls `proveConsent` with the
    /// bundled Level-B artifact, submits to the Level-B phone route, then polls the detached broadcast
    /// through the same session-status + `consumed(nullifier)` seams as Level-A.
    @MainActor
    private func runLevelBFlow(
        sess: CentralApi.ExportSession,
        roax: RoaxConfig,
        credential: Credential,
        req: VerificationRequest,
        host: String,
        token: String,
        groomerAddr: String
    ) async {
        status = "Validating owner-hidden discovery anchor…"
        guard let claims = sess.claims else {
            working = false
            status = "Owner-hidden session is missing its discovery claims — refusing."
            return
        }
        // Resolve BOTH on-chain axes. Either nil — registry unconfigured/undeployed, version
        // unpublished, or no artifact binding — fails closed without entering Level-A.
        let version = AnchorResolver.levelBVersion
        async let csTask = RoaxRpc.getContractSet(
            rpcUrl: AppConfig.roaxRpc, protocolRegistry: roax.protocolRegistry, version: version)
        async let asTask = RoaxRpc.getActiveArtifactSet(
            rpcUrl: AppConfig.roaxRpc, protocolRegistry: roax.protocolRegistry, version: version)
        guard let cs = await csTask, let arti = await asTask else {
            working = false
            status = "Owner-hidden verification is not available yet (discovery anchor unpublished)."
            return
        }
        // Build the FFI `TrustedAnchor`. `contractSetActive`/`artifactSetActive` come from the two
        // records SEPARATELY (never AND-ed) — `validateDiscovery` requires both true independently.
        let anchor = TrustedAnchor(
            version: version,
            versionId: cs.contractSetId,
            artifactSet: AnchorResolver.levelBArtifactSet,
            artifactSetId: arti.artifactSetId,
            chainId: UInt64(roax.chainId),
            verificationRegistry: cs.verificationRegistry,
            circuitId: AnchorResolver.levelBCircuitId,
            minAppVersion: arti.minAppVersion,
            contractSetActive: cs.active,
            artifactSetActive: arti.active)
        let ffiClaims = ConvenienceClaims(
            protocolVersion: claims.protocolVersion,
            chainId: claims.chainId,
            verificationRegistry: claims.verificationRegistry,
            issuerClone: claims.issuerClone,
            purpose: claims.purpose)
        // `appVersion`: this build's marketing version (dotted semver). `expectedPurpose`: the
        // session's purpose (D1 ruling (i)). The app has no purpose independent of the scanned QR
        // today, so `validateDiscovery`'s purpose check (§5.3 step 4) is INTENTIONALLY WEAK here —
        // claim vs the same session it came from. That is not a regression: it matches the Level-A
        // path's existing posture, and the load-bearing anti-redirect weight sits in the
        // registry/chainId/version/versionId/both-active/minAppVersion checks, which all still fire.
        // An independent app-side purpose is queued as follow-up hardening.
        let appVersion = (Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String) ?? "0.0.0"
        do {
            _ = try validateDiscovery(
                claims: ffiClaims, anchor: anchor, appVersion: appVersion, expectedPurpose: sess.purpose)

            // Match the Level-A pre-proof authorization posture, but keep every read inside the
            // Level-B branch. The server and contract repeat the whitelist check before spending gas.
            status = "Checking groomer authorization…"
            let verifyKey = verifyWhitelistKeyHex(purposeLabel: sess.purpose)
            let wl = await RoaxRpc.isWhitelistedFor(
                rpcUrl: AppConfig.roaxRpc, issuerRegistry: roax.issuerRegistry,
                key: verifyKey, signer: sess.relayer)
            guard case .valid = wl else {
                working = false
                status = "This groomer is not authorized (not whitelisted)."
                return
            }
            if !DnsVerify.isLocalHost(host) {
                status = "Verifying groomer DNS…"
                guard await DnsVerify.verifyGroomer(host: host, groomerAddr: groomerAddr) else {
                    working = false
                    status = "Groomer DNS not verified — refusing to present."
                    return
                }
            }

            // M-2b accessors confirmed at kickoff: Wallet.seedHex() supplies the seed, while the
            // per-tag ProfileTreeStore record supplies the decimal handle, owner address and salted
            // attributes. The stored ownerSecretHex is deliberately NOT handed across this seam;
            // proveConsent derives and checks the complete witness internally from the seed.
            guard let seedHex = Wallet.seedHex() else {
                working = false
                status = "Wallet seed unavailable — authenticate and try again."
                return
            }
            // Use the throwing accessor here: an encrypted store that exists but cannot be read is
            // not equivalent to "no secret", and the prover must fail closed rather than hide it.
            guard let owner = try ProfileTreeStore.load().first(where: {
                $0.dogTagIdDec == credential.dogTagId
            }),
                  owner.abandonedAt == nil else {
                working = false
                status = "No active owner-hidden secret exists for this dog tag."
                return
            }
            guard owner.derivationVersion == ProfileTreeStore.derivationVersion else {
                working = false
                status = "This owner-hidden secret uses an unsupported derivation version."
                return
            }
            let attributes = owner.attributes.map { a -> [String: Any] in
                ["keyPath": a.keyPath, "salt": a.saltHex, "tag": Int(a.tag), "value": a.value]
            }
            let attributesData = try JSONSerialization.data(withJSONObject: attributes)
            guard let attributesJson = String(data: attributesData, encoding: .utf8) else {
                throw NSError(domain: "DogTag.LevelB", code: 1,
                              userInfo: [NSLocalizedDescriptionKey: "could not encode profile attributes"])
            }
            let consentNonce = "0x" + Keccak256.digest(Data(UUID().uuidString.utf8))
                .map { String(format: "%02x", $0) }.joined()
            // The route requires >120s at preflight because its on-chain broadcast is detached and
            // retried. Ten minutes leaves ample room for proving plus deferred settlement.
            let deadlineDec = String(UInt64(Date().timeIntervalSince1970) + 600)

            guard let descriptor = ZkeyAsset.resolve(version: AnchorResolver.levelBVersion),
                  let zkeyPath = ZkeyAsset.ensure(descriptor: descriptor),
                  let graphPath = ZkeyAsset.ensureGraph(descriptor: descriptor) else {
                working = false
                status = "Owner-hidden proving artifact missing from bundle."
                return
            }
            status = "Generating owner-hidden proof…"
            let proof = try proveConsent(
                seedHex: seedHex,
                dogTagIdHandle: owner.dogTagIdDec,
                ownerAddressHex: owner.ownerAddress,
                attributesJson: attributesJson,
                purposeHex: req.purpose,
                relayerHex: req.relayer,
                recordTypeHex: req.recordType,
                consentNonceHex: consentNonce,
                deadlineDec: deadlineDec,
                zkeyPath: zkeyPath,
                graphPath: graphPath)

            // Level-B's nullifier is index 3. Index 4 is R; polling it is the classic successful-
            // submit hang because VerificationRegistryConsent never marks R as consumed.
            let nfIdx = PublicSignalIndex.levelB.nullifier
            guard proof.pubSignals.count > nfIdx else {
                throw NSError(domain: "DogTag.LevelB", code: 2,
                              userInfo: [NSLocalizedDescriptionKey: "proof omitted the Level-B nullifier"])
            }
            let nullifier = proof.pubSignals[nfIdx]
            let verificationRegistry = cs.verificationRegistry
            if await RoaxRpc.consumed(
                rpcUrl: AppConfig.roaxRpc, verificationRegistry: verificationRegistry,
                nullifier: nullifier) {
                working = false
                status = "This verification was already recorded."
                return
            }

            let proofObject: [String: Any] = [
                "a": proof.a, "b": proof.b, "c": proof.c, "pubSignals": proof.pubSignals,
            ]
            let payload: [String: Any] = ["exportToken": token, "proof": proofObject]
            let payloadData = try JSONSerialization.data(withJSONObject: payload)
            guard let payloadJson = String(data: payloadData, encoding: .utf8) else {
                throw NSError(domain: "DogTag.LevelB", code: 3,
                              userInfo: [NSLocalizedDescriptionKey: "could not encode Level-B proof"])
            }

            status = "Submitting owner-hidden proof to groomer…"
            let response = await CentralApi.postVerifyConsentLevelBToHost(
                host: host, payloadJson: payloadJson)
            if (400..<500).contains(response.code),
               let data = response.body.data(using: .utf8),
               let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
               let reject = object["error"] as? String, !reject.isEmpty {
                working = false
                status = "Submit rejected (\(reject))."
                return
            }

            // Detached-broadcast read-back: the session reports terminal errors while the Level-B
            // registry's consumed(nullifier) bit is the canonical success signal. Same cadence and
            // timeout as the existing Level-A primitive; there is no synchronous 120s HTTP request.
            status = "Recording your owner-hidden verification on-chain…"
            var done = false
            var failedMsg: String? = nil
            for _ in 0..<40 {
                if await RoaxRpc.consumed(
                    rpcUrl: AppConfig.roaxRpc, verificationRegistry: verificationRegistry,
                    nullifier: nullifier) {
                    done = true
                    break
                }
                if let session = await CentralApi.verifySessionStatus(
                    host: host, sessionId: sess.sessionId, token: token),
                   session.status == "error" {
                    failedMsg = session.txHash?.isEmpty == false ? session.txHash! : "recording failed"
                    break
                }
                try? await Task.sleep(nanoseconds: 3_000_000_000)
            }
            if let failedMsg {
                working = false
                status = "Verification failed: \(failedMsg)"
                return
            }
            if done {
                let tx = await CentralApi.verifySessionStatus(
                    host: host, sessionId: sess.sessionId, token: token)?.txHash
                working = false
                if let tx, !tx.isEmpty {
                    status = "Verified on-chain — owner hidden. tx \(String(tx.prefix(14)))…"
                } else {
                    status = "Verified on-chain — owner hidden."
                }
            } else {
                working = false
                status = "Submitted; awaiting confirmation."
            }
        } catch {
            working = false
            status = "Owner-hidden verification refused: \(error)"
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
