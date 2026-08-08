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
    /// The terminal result of a presentation. Nil while idle or in flight. Rendered as a
    /// prominent card - a refusal on this screen must never be 12pt muted text again.
    @State private var outcome: ConsentOutcome? = nil
    @State private var working = false
    @State private var selected: Credential? = nil
    // D1 disclosure picker: the `owner.identity.*` keyPaths the owner chose to REVEAL to this
    // verifier. Default empty - nothing is disclosed unless the owner opts in per leaf.
    @State private var revealKeyPaths: Set<String> = []
    // Export-session metadata resolved (non-consuming) from the QR's one-time token.
    @State private var exportSession: CentralApi.ExportSession? = nil
    @State private var exportResolveErr: String? = nil
    // Dog-tag issuance metadata and final owner-hidden anchoring result.
    @State private var issueSession: CentralApi.DogTagIssueSession? = nil
    @State private var issued: CentralApi.DogTagIssue? = nil
    @State private var issueErr = ""

    var body: some View {
        // Import and owner-hidden presentation both rely on the local recovery seed. No wallet means
        // there is no owner-control witness to build or prove.
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
                    Button { status = ""; payload = nil; selected = nil; revealKeyPaths = []; exportSession = nil; exportResolveErr = nil; issueSession = nil; issued = nil; issueErr = ""; scanning = true } label: {
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
                Text("Your phone will create the private owner proof locally. Only its Merkle root is sent to the vet for anchoring; your wallet and owner secret never leave this device.")
                    .font(.system(size: 12)).foregroundColor(c.muted)

                if let session = issueSession {
                    field("Pet", session.pet.name.isEmpty ? "DogTag #\(session.dogTagId)" : session.pet.name)
                    field("Dog tag", session.dogTagId)
                    if !session.pet.breedLabel.isEmpty { field("Breed", session.pet.breedLabel) }
                    if !session.pet.microchip.code.isEmpty { field("Microchip", session.pet.microchip.code) }
                    if !session.ownerIdentity.name.isEmpty { field("Owner", session.ownerIdentity.name) }
                } else if issueErr.isEmpty {
                    Text("Resolving issuance session…").font(.system(size: 12)).foregroundColor(c.muted)
                }

                if issued == nil && !working, let session = issueSession {
                    Button { bindIssue(host: host, token: token, session: session) } label: {
                        Text("Create private profile & issue").frame(maxWidth: .infinity).padding(.vertical, 12)
                            .foregroundColor(c.onAccent).background(RoundedRectangle(cornerRadius: 12).fill(c.accent))
                    }
                }
                if working {
                    ForgeWaitView(status: status)
                }
                if !working && !status.isEmpty {
                    Text(status).font(.system(size: 12)).foregroundColor(issued == nil ? c.muted : c.success)
                }
                if !issueErr.isEmpty { Text(issueErr).font(.system(size: 12)).foregroundColor(c.danger) }
            }

            if let res = issued {
                card {
                    Text("Dog tag issued").font(.system(size: 16, weight: .bold)).foregroundColor(c.success)
                    CopyableMonoRow(label: "dogTagId", value: res.dogTagId, truncate: false)
                    CopyableMonoRow(label: "Merkle root", value: res.root)
                    if !res.txHash.isEmpty {
                        CopyableMonoRow(label: "Transaction", value: res.txHash)
                    }
                    Text("The private recovery witness is stored on this device.").font(.system(size: 12)).foregroundColor(c.muted)
                }
            }
        }
        .task(id: token) {
            issueSession = nil
            issued = nil
            issueErr = ""
            guard let session = await CentralApi.resolveDogTagIssue(host: host, token: token) else {
                // Two different faults arrive here identically (this API cannot tell them apart):
                // the vet's machine was unreachable, or the QR's one-time token has expired. Name
                // both remedies rather than picking one.
                issueErr = "Could not reach the vet at \(host) — or the QR has expired. "
                    + "Check this phone is on the same Wi-Fi network as the vet, then ask for a fresh QR."
                return
            }
            issueSession = session
        }
    }

    private func bindIssue(host: String, token: String, session: CentralApi.DogTagIssueSession) {
        issueErr = ""
        Biometric.authenticate(reason: "Authenticate to create this dog tag's private owner proof") { ok, e in
            guard ok else { issueErr = e ?? "auth failed"; return }
            guard let wallet = (try? Wallet.load()) ?? nil else {
                issueErr = "Create your wallet first (Profile)."; return
            }
            guard let seedHex = Wallet.seedHex() else {
                issueErr = "Wallet seed unavailable — authenticate and try again."; return
            }
            working = true
            status = "Building private owner profile on this device…"
            Task {
                do {
                    let (root, leaves, reservedLeafHashes, reused) = try buildOrReuseIssueRoot(
                        session: session, seedHex: seedHex, ownerAddress: wallet.ethAddress)
                    await MainActor.run { status = "Sending the profile commitment to your vet…" }
                    let bindResult = await CentralApi.bindDogTagIssue(
                        host: host, token: token, root: root,
                        leaves: leaves, reservedLeafHashes: reservedLeafHashes)
                    var accepted: CentralApi.DogTagIssue? = nil
                    switch bindResult {
                    case let .accepted(result):
                        accepted = result
                        if result.dogTagId != session.dogTagId {
                            throw issueFailure("vet returned a different dogTagId")
                        }
                        if result.root.caseInsensitiveCompare(root) != .orderedSame {
                            throw issueFailure("vet returned a different profile root")
                        }
                    case .inconclusive:
                        accepted = nil
                    case .gone where reused:
                        // This phone has bound this tag before, so the consumed token is plausibly
                        // our own earlier bind whose response was lost — the chain poll below is
                        // what settles it either way.
                        accepted = nil
                    case .gone:
                        // A FRESH tag: this phone's bind definitively did not happen — a consumed
                        // or expired token refused it before any chain write. Say so instead of
                        // polling three minutes for an anchor that cannot arrive and then claiming
                        // "Submitted".
                        throw issueFailure(
                            "the vet's QR expired or was already used before this phone could send "
                                + "the profile. Nothing was issued — ask the vet to draw a new QR, "
                                + "then scan it again.")
                    case let .rejected(statusCode, body):
                        throw issueFailure(
                            "custodial bind rejected (\(statusCode)): \(String(body.prefix(160)))")
                    }

                    // The HTTP response can be lost after the one-time token was consumed, and the
                    // issuance-session status route is operator-gated (the device holds no operator
                    // session). Confirm completion the way the pre-change flow did: read the anchor
                    // straight from the public chain. `mintCustodial` writes
                    // `DogTagSBTConsent.profileRoot(id) = R` once both txs mine, keyed by the CANONICAL
                    // field-hashed id (never the raw handle). The persisted root is authoritative, so
                    // poll even when the POST result is inconclusive and never rebuild with new salts.
                    let roax = RoaxConfig.load()
                    let onchainId = try dogTagIdFieldHex(dogTagIdDec: session.dogTagId)
                    // The slow phase is the NETWORK's confirmation, not proof generation — say so,
                    // with how long it may reasonably take, or the wait reads as a hang.
                    await MainActor.run {
                        status = "Profile sent. Waiting for the network to confirm your dog tag — "
                            + "this can take a minute or two…"
                    }
                    var anchored = false
                    var delayNanos: UInt64 = 2_000_000_000
                    for _ in 0..<40 {
                        if let chainRoot = await RoaxRpc.profileRoot(
                            rpcUrl: RpcEndpointSettings.rpcUrl(),
                            dogTagSbt: roax.dogTagSbt, dogTagId: onchainId),
                           chainRoot.dropFirst(2).contains(where: { $0 != "0" }) {
                            guard chainRoot.caseInsensitiveCompare(root) == .orderedSame else {
                                throw issueFailure(
                                    "dog tag is already anchored to a different profile root")
                            }
                            anchored = true
                            break
                        }
                        // The server's own answer, through the consumed token's status peek: a
                        // session settled to "error" is DEFINITE and the chain poll could only
                        // ever wait past it — measured live 2026-08-07, where the mint had
                        // reverted minutes earlier and this screen kept forging.
                        if let peek = await CentralApi.dogTagIssueStatus(host: host, token: token),
                           peek.status == "error" {
                            throw issueFailure(IssuanceWait.failureText(reason: peek.reason))
                        }
                        try? await Task.sleep(nanoseconds: delayNanos)
                        delayNanos = min(delayNanos + 500_000_000, 5_000_000_000)
                    }
                    guard anchored else {
                        // The wait is OVER — say what was observed, never that the work is still
                        // coming (`IssuanceWait` owns the sentences, mirrored with Android).
                        let peek = await CentralApi.dogTagIssueStatus(host: host, token: token)
                        await MainActor.run {
                            working = false
                            // The server's own answer outranks local knowledge; when the peek
                            // itself is unreachable, what this phone knows locally — whether the
                            // vet ACCEPTED the bind — is still stated accurately. "Submitted" may
                            // only be claimed when the vet actually accepted; on the inconclusive
                            // path nothing is known to have arrived.
                            if let peek = peek {
                                issueErr = IssuanceWait.timeoutText(
                                    serverStatus: peek.status, reason: peek.reason)
                            } else if accepted != nil {
                                issueErr = "The vet accepted the profile, but the network has not "
                                    + "confirmed the anchor yet. The vet portal shows when it "
                                    + "completes; this dog tag appears here after that."
                            } else {
                                issueErr = "Could not confirm the vet received the profile. Check "
                                    + "the vet portal: if this issuance is not shown there, ask "
                                    + "the vet to draw a new QR and scan again."
                            }
                        }
                        return
                    }
                    await MainActor.run {
                        store.upsertPet(Pet(
                            dogTagId: session.dogTagId,
                            name: session.pet.name.isEmpty ? "DogTag #\(session.dogTagId)" : session.pet.name,
                            breed: session.pet.breedLabel.isEmpty ? session.pet.breedVbo : session.pet.breedLabel,
                            ageLabel: session.pet.dateOfBirth,
                            microchip: session.pet.microchip.code.isEmpty ? nil : session.pet.microchip.code))
                        issued = CentralApi.DogTagIssue(
                            dogTagId: session.dogTagId,
                            root: root,
                            txHash: accepted?.txHash ?? "",
                            status: "bound",
                            bound: true)
                        working = false
                        status = "Issued and anchored — owner hidden."
                    }
                } catch {
                    await MainActor.run {
                        working = false
                        issueErr = "Issue failed: \(error.localizedDescription)"
                    }
                }
            }
        }
    }

    /// A bind retry must reuse the exact persisted salts/root. Generating fresh salts for the same
    /// allocated dogTagId would create a second root that the write-once contract can never accept.
    ///
    /// D1: the vet-salted identity leaves are folded into `R` ALONGSIDE the pet attributes -
    /// identity keeps the VET's salts (the bind-time full-leaf-list gate requires the posted
    /// identity openings to EXACTLY match the vet's own `{keyPath, salt, value}` set), pet
    /// attributes keep device-random salts. Both are persisted as ordinary attribute openings in
    /// the owner-secret store, so the same list feeds issuance, the consent-proof rebuild, and
    /// later disclosures. Returns `R` plus the bind's full-leaf-list commitment: the opening of
    /// every attribute leaf and the three OPAQUE reserved leaf hashes (never their preimages).
    /// `reused` reports whether a persisted record for this dogTagId already existed — the local
    /// fact that disambiguates a 410 on the bind (see `CustodialBindResult.gone`).
    private func buildOrReuseIssueRoot(
        session: CentralApi.DogTagIssueSession,
        seedHex: String,
        ownerAddress: String
    ) throws -> (root: String, leaves: [[String: Any]], reservedLeafHashes: [String], reused: Bool) {
        let requested = session.pet.profileAttributeValues
        let identity = session.identityLeaves.map {
            ProfileTreeStore.BackedUpAttribute(
                keyPath: $0.keyPath, saltHex: $0.saltHex, tag: $0.tag, value: $0.value)
        }
        if let existing = try ProfileTreeStore.load().first(where: {
            $0.dogTagIdDec == session.dogTagId
        }) {
            guard existing.abandonedAt == nil else {
                throw issueFailure("this dogTagId was retired and cannot be reused")
            }
            guard existing.derivationVersion == ProfileTreeStore.derivationVersion else {
                throw issueFailure("existing owner secret uses an unsupported derivation version")
            }
            // Pet attrs compare on (keyPath, value, tag) - their salts are device-random. Identity
            // attrs ALSO compare on the salt: the vet verifies inclusion with ITS stored salt, so a
            // divergent salt could never bind.
            let petStored = existing.attributes.prefix(requested.count)
            let identityStored = existing.attributes.dropFirst(requested.count)
            let metadataMatches = existing.attributes.count == requested.count + identity.count
                && zip(petStored, requested).allSatisfy { stored, incoming in
                    stored.keyPath == incoming.keyPath
                        && stored.value == incoming.value
                        && stored.tag == incoming.tag
                }
                && zip(identityStored, identity).allSatisfy { stored, incoming in
                    stored == incoming
                }
            let matches = existing.ownerAddress.caseInsensitiveCompare(ownerAddress) == .orderedSame
                && metadataMatches
            guard matches else {
                throw issueFailure("this dogTagId already has different private profile metadata")
            }
            _ = try ProfileTreeStore.verifyRecoverable(seedHex: seedHex, record: existing)
            // Deterministic rebuild from the persisted record for the reserved leaf hashes;
            // nothing new is persisted. The record stores the CANONICAL dogTagId field.
            let tree = try buildProfileTreeHex(
                seedHex: seedHex,
                dogTagIdHex: existing.dogTagIdHex,
                ownerAddressHex: existing.ownerAddress,
                attributes: existing.attributes.map {
                    AttributeLeafFfi(keyPath: $0.keyPath, saltHex: $0.saltHex, tag: $0.tag, value: $0.value)
                })
            return (tree.rootHex, bindLeafOpenings(existing.attributes),
                    [tree.ownerAddressLeafHex, tree.consentKeyLeafHex, tree.ownerSecretLeafHex],
                    true)
        }
        var attributes = try requested.map { item -> ProfileTreeStore.BackedUpAttribute in
            let salted = try ProfileTreeStore.randomStringAttribute(
                keyPath: item.keyPath, value: item.value)
            return ProfileTreeStore.BackedUpAttribute(
                keyPath: salted.keyPath, saltHex: salted.saltHex, tag: item.tag, value: salted.value)
        }
        attributes.append(contentsOf: identity)
        let tree = try ProfileTreeStore.buildAndPersist(
            seedHex: seedHex,
            dogTagIdDec: session.dogTagId,
            ownerAddress: ownerAddress,
            attributes: attributes)
        return (tree.rootHex, bindLeafOpenings(attributes),
                [tree.ownerAddressLeafHex, tree.consentKeyLeafHex, tree.ownerSecretLeafHex],
                false)
    }

    /// The bind's `leaves`: the opening of every attribute leaf, `{keyPath, saltHex, tag, value}`.
    private func bindLeafOpenings(_ attributes: [ProfileTreeStore.BackedUpAttribute]) -> [[String: Any]] {
        attributes.map {
            ["keyPath": $0.keyPath, "saltHex": $0.saltHex, "tag": Int($0.tag), "value": $0.value]
        }
    }

    private func issueFailure(_ message: String) -> NSError {
        NSError(domain: "DogTag.OwnerHiddenIssuance", code: 1,
                userInfo: [NSLocalizedDescriptionKey: message])
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
            // One decision for both the control's enabled-ness and the sentence beside it, so the
            // two can never disagree — see `ExportAvailability` for why the reason cannot live
            // inside the action a disabled control never fires.
            let availability = ExportAvailability.of(
                candidateCount: candidates.count, hasSelection: selected != nil, working: working)
            VStack(alignment: .leading, spacing: 14) {
                card {
                    Text("Export request").font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
                    field("Groomer", sess.relayer.isEmpty ? "Unknown" : sess.relayer)
                    field("Purpose", sess.purpose.isEmpty ? "—" : sess.purpose)
                    field("Record type", sess.recordType.isEmpty ? "any" : sess.recordType)
                }
                card {
                    Text("Select the record to export").font(.system(size: 15, weight: .bold)).foregroundColor(c.onBackground)
                    if candidates.isEmpty {
                        Text("No matching records yet — scan a vet's QR to import one first.").font(.system(size: 12)).foregroundColor(c.muted)
                    }
                    ForEach(candidates) { cred in
                        let isSel = selected?.id == cred.id
                        Button { selected = cred; revealKeyPaths = [] } label: {
                            VStack(alignment: .leading, spacing: 6) {
                                HStack(alignment: .top) {
                                    CredentialLabel(cred: cred, petName: store.petDisplayName(for: cred))
                                    Spacer()
                                    VerdictBadge(cred: cred)
                                }
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(12)
                            .background(RoundedRectangle(cornerRadius: 12).fill(isSel ? c.accent.opacity(0.14) : c.surfaceVariant))
                            .overlay(RoundedRectangle(cornerRadius: 12).stroke(isSel ? c.accent : .clear, lineWidth: 1.5))
                        }
                        .buttonStyle(.plain)
                    }
                }
                // D1: the owner-facing disclosure picker. Shown only when the selected tag's
                // owner-secret record carries identity leaves; every leaf defaults to HIDDEN and
                // each reveal is an explicit per-leaf opt-in for THIS verifier only.
                if let cred = selected {
                    let identityLeaves = identityAttributes(forDogTagIdDec: cred.dogTagId)
                    if !identityLeaves.isEmpty {
                        card {
                            Text("Share your identity (optional)").font(.system(size: 15, weight: .bold)).foregroundColor(c.onBackground)
                            Text("Each detail you switch on is revealed to this verifier, proven against your dog tag's sealed profile. Everything stays hidden by default.")
                                .font(.system(size: 12)).foregroundColor(c.muted)
                            ForEach(identityLeaves, id: \.keyPath) { leaf in
                                Toggle(isOn: Binding(
                                    get: { revealKeyPaths.contains(leaf.keyPath) },
                                    set: { on in
                                        if on { revealKeyPaths.insert(leaf.keyPath) }
                                        else { revealKeyPaths.remove(leaf.keyPath) }
                                    }
                                )) {
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(identityLabel(for: leaf.keyPath)).font(.system(size: 13, weight: .semibold)).foregroundColor(c.onBackground)
                                        Text(leaf.value).font(.system(size: 12, design: .monospaced)).foregroundColor(c.muted)
                                    }
                                }
                                .tint(c.accent)
                            }
                        }
                    }
                }
                if working {
                    ForgeWaitView(status: status.isEmpty ? "Recording your verification on-chain…" : status,
                                  title: "Recording your verification on-chain")
                }
                Button { presentExport(host: host, token: token, groomerAddr: groomerAddr, sess: sess) } label: {
                    Text(working ? "Working…" : "Approve & export").frame(maxWidth: .infinity).padding(.vertical, 12)
                        .foregroundColor(.white)
                        // A disabled control must not keep the live colour: `.disabled` does not dim a
                        // custom background, so the button read as pressable while doing nothing.
                        .background(RoundedRectangle(cornerRadius: 12)
                            .fill(availability.canProceed ? c.success : c.muted.opacity(0.45)))
                }
                .disabled(!availability.canProceed)
                // WHY the control is inert, rendered from the state rather than from the action the
                // disabled control never fires. Absent this, an empty wallet met a dead button in
                // silence — the state a first-time holder is always in.
                if let reason = availability.blockedReason {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(reason).font(.system(size: 12, weight: .semibold)).foregroundColor(c.onBackground)
                        if let step = availability.nextStep {
                            Text(step).font(.system(size: 12)).foregroundColor(c.muted)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                // While working the ForgeWaitView already surfaces the live status; show the plain
                // status text only when idle (the final success/timeout line).
                // The terminal result. This is where a SECURITY REFUSAL is delivered, so it is a card
                // with an icon and a headline rather than a line of 12pt muted text - the shape that
                // let a correct refusal read as "nothing happened" on the consent screen.
                if !working, let o = outcome {
                    consentOutcomeCard(o)
                }
                if !working && outcome == nil && !status.isEmpty {
                    Text(status).font(.system(size: 12)).foregroundColor(c.muted)
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
        guard let credential = selected else {
            status = "Select a record first."
            return
        }
        Biometric.authenticate(
            reason: "Present '\(credential.displayTypeLabel)' to \(sess.relayer.isEmpty ? "the verifier" : sess.relayer)"
        ) { ok, error in
            guard ok else {
                status = ""
                outcome = .couldNotComplete(.authentication(error))
                return
            }
            working = true
            outcome = nil
            let roax = RoaxConfig.load()
            Task {
                await runLevelBFlow(
                    sess: sess,
                    roax: roax,
                    credential: credential,
                    host: host,
                    token: token,
                    groomerAddr: groomerAddr)
            }
        }
    }
    /// The one owner-hidden consent flow. It validates the on-chain discovery anchor, rebuilds the
    /// device-private profile witness, proves consent, submits it to the canonical verifier route,
    /// and polls the detached on-chain recording.
    @MainActor
    private func runLevelBFlow(
        sess: CentralApi.ExportSession,
        roax: RoaxConfig,
        credential: Credential,
        host: String,
        token: String,
        groomerAddr: String
    ) async {
        status = "Validating owner-hidden discovery anchor…"
        guard let claims = sess.claims else {
            working = false
            status = ""
            outcome = .refused(.sessionMissingClaims)
            return
        }
        // Resolve both on-chain axes. Missing configuration/publication fails closed.
        //
        // These TWO reads deliberately name the BUNDLED endpoint rather than the holder's transport
        // choice, and they are the only chain reads that bypass that choice. The contract set they return
        // IS the trust anchor `validateDiscovery` compares the platform's claimed
        // `verificationRegistry`/version against - the anti-redirect trip. A peer that answers
        // `eth_chainId` with 135 (trivial for a hostile peer) could otherwise supply BOTH sides of
        // that comparison, so a hostile portal plus a holder-chosen peer would satisfy a check whose
        // whole point is independence from the portal. Endpoint choice is transport liveness for
        // reads whose answers the anchor then constrains; it must never answer the anchor itself.
        // Still routed through `guardedPostJSON`, so the bundled peer is probed for `eth_chainId`
        // immediately before each read and an unavailable/wrong-chain bundled peer yields nil here
        // (fail closed) rather than falling back to the custom peer - `endpointRoute` takes its
        // `requested == bundled` branch, which has no custom candidate to fall back to.
        let version = AnchorResolver.protocolVersion
        async let csTask = RoaxRpc.getDiscoverySet(
            rpcUrl: AppConfig.roaxRpc,
            protocolRegistry: roax.protocolRegistry, version: version)
        async let asTask = RoaxRpc.getActiveArtifactSet(
            rpcUrl: AppConfig.roaxRpc,
            protocolRegistry: roax.protocolRegistry, version: version)
        guard let cs = await csTask, let arti = await asTask else {
            working = false
            status = ""
            outcome = .couldNotComplete(.anchorUnavailable)
            return
        }
        // Build the FFI `TrustedAnchor`. `contractSetActive`/`artifactSetActive` come from the two
        // records SEPARATELY (never AND-ed) — `validateDiscovery` requires both true independently.
        let anchor = TrustedAnchor(
            version: version,
            versionId: cs.discoverySetId,
            artifactSet: AnchorResolver.artifactSet,
            artifactSetId: arti.artifactSetId,
            chainId: UInt64(roax.chainId),
            verificationRegistry: cs.verificationRegistry,
            circuitId: AnchorResolver.circuitId,
            minAppVersion: arti.minAppVersion,
            contractSetActive: cs.active,
            artifactSetActive: arti.active,
            // Both come from the discovery record itself. `rootIndex` resolves to that record's
            // `factory`, and on this contract set that is not a shortcut: there is one launch set and
            // no earlier generation to bridge, `VerificationRegistryConsent` pins the factory in its
            // immutable `rootIndex` slot, and the publish script's preflight refuses to stage a record
            // whose `factory` is not `verificationRegistry.rootIndex()`. So the equality is asserted
            // on chain before anything is published, rather than assumed here.
            //
            // Never source either from the app's own bundle instead: the whole point of reading them
            // from the anchor is that a platform's claim gets CHECKED against a dogtag-governed record.
            providerRegistry: cs.providerRegistry,
            rootIndex: cs.rootIndex)
        let ffiClaims = ConvenienceClaims(
            protocolVersion: claims.protocolVersion,
            chainId: claims.chainId,
            verificationRegistry: claims.verificationRegistry,
            issuerClone: claims.issuerClone,
            purpose: claims.purpose)
        // `appVersion`: this build's marketing version (dotted semver). `expectedPurpose`: the
        // session's purpose. The app has no purpose independent of the scanned QR
        // today, so `validateDiscovery`'s purpose check (§5.3 step 4) is INTENTIONALLY WEAK here —
        // claim vs the same session it came from. The load-bearing anti-redirect weight sits in the
        // registry/chainId/version/versionId/both-active/minAppVersion checks, which all still fire.
        // An independent app-side purpose is queued as follow-up hardening.
        let appVersion = (Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String) ?? "0.0.0"
        // The anti-redirect trip gets its OWN catch: a verifier that disagrees with the on-chain
        // anchor is a deliberate REFUSAL, and folding it into the outer catch would render the one
        // protective decision on this screen as "something went wrong".
        do {
            _ = try validateDiscovery(
                claims: ffiClaims, anchor: anchor, appVersion: appVersion, expectedPurpose: sess.purpose)
        } catch {
            working = false
            status = ""
            outcome = .refused(.verifierFailedAnchorCheck(detail: ConsentOutcome.discoveryDetail("\(error)")))
            return
        }
        do {

            // The server and contract repeat this whitelist check before spending gas.
            status = "Checking groomer authorization…"
            let verifyKey = verifyWhitelistKeyHex(purposeLabel: sess.purpose)
            let wl = await RoaxRpc.isWhitelistedFor(
                rpcUrl: RpcEndpointSettings.rpcUrl(), issuerRegistry: roax.issuerRegistry,
                key: verifyKey, signer: sess.relayer)
            guard case .valid = wl else {
                working = false
                status = ""
                outcome = .refused(.verifierNotAuthorized)
                return
            }
            if !DnsVerify.isLocalHost(host) {
                status = "Verifying groomer DNS…"
                guard await DnsVerify.verifyGroomer(host: host, groomerAddr: groomerAddr) else {
                    working = false
                    status = ""
                    outcome = .refused(.verifierDomainUnverified)
                    return
                }
            }

            // M-2b accessors confirmed at kickoff: Wallet.seedHex() supplies the seed, while the
            // per-tag ProfileTreeStore record supplies the decimal handle, owner address and salted
            // attributes. The stored ownerSecretHex is deliberately NOT handed across this seam;
            // proveConsent derives and checks the complete witness internally from the seed.
            guard let seedHex = Wallet.seedHex() else {
                working = false
                status = ""
                outcome = .couldNotComplete(.walletLocked)
                return
            }
            // Use the throwing accessor here: an encrypted store that exists but cannot be read is
            // not equivalent to "no secret", and the prover must fail closed rather than hide it.
            guard let owner = try ProfileTreeStore.load().first(where: {
                $0.dogTagIdDec == credential.dogTagId
            }),
                  owner.abandonedAt == nil else {
                working = false
                status = ""
                outcome = .couldNotComplete(.noOwnerSecret)
                return
            }
            guard owner.derivationVersion == ProfileTreeStore.derivationVersion else {
                working = false
                status = ""
                outcome = .couldNotComplete(.unsupportedSecretVersion)
                return
            }
            let attributes = owner.attributes.map { a -> [String: Any] in
                ["keyPath": a.keyPath, "salt": a.saltHex, "tag": Int(a.tag), "value": a.value]
            }
            let attributesData = try JSONSerialization.data(withJSONObject: attributes)
            guard let attributesJson = String(data: attributesData, encoding: .utf8) else {
                throw NSError(domain: "DogTag.OwnerHidden", code: 1,
                              userInfo: [NSLocalizedDescriptionKey: "could not encode profile attributes"])
            }
            let consentNonce = "0x" + Keccak256.digest(Data(UUID().uuidString.utf8))
                .map { String(format: "%02x", $0) }.joined()
            // The route requires >120s at preflight because its on-chain broadcast is detached and
            // retried. Ten minutes leaves ample room for proving plus deferred settlement.
            let deadlineDec = String(UInt64(Date().timeIntervalSince1970) + 600)

            guard let descriptor = ZkeyAsset.resolve(version: AnchorResolver.protocolVersion),
                  let zkeyPath = ZkeyAsset.ensure(descriptor: descriptor),
                  let graphPath = ZkeyAsset.ensureGraph(descriptor: descriptor) else {
                working = false
                status = ""
                outcome = .couldNotComplete(.provingArtifactMissing)
                return
            }
            status = "Generating owner-hidden proof…"
            // Groth16 proving is synchronous and runs seconds-to-minutes on-device. This function is
            // @MainActor, so the FFI MUST run off the main actor or it freezes the UI (and the
            // ForgeWait animation) and risks a watchdog kill. Use Task.detached (a plain Task would
            // re-inherit MainActor here);
            // capture only Sendable String locals and transfer the ProofFfi back before resuming.
            let dogTagIdHandle = owner.dogTagIdDec
            let ownerAddressHex = owner.ownerAddress
            let purposeHex = ConsentField.keccakLabel(sess.purpose)
            let relayerHex = sess.relayer
            let recordTypeHex = ConsentField.keccakLabel(sess.recordType)
            let proof = try await Task.detached(priority: .userInitiated) {
                try proveConsent(
                    seedHex: seedHex,
                    dogTagIdHandle: dogTagIdHandle,
                    ownerAddressHex: ownerAddressHex,
                    attributesJson: attributesJson,
                    purposeHex: purposeHex,
                    relayerHex: relayerHex,
                    recordTypeHex: recordTypeHex,
                    consentNonceHex: consentNonce,
                    deadlineDec: deadlineDec,
                    zkeyPath: zkeyPath,
                    graphPath: graphPath)
            }.value

            // The consent nullifier is index 3. Index 4 is R and is never a consumed-key.
            let nfIdx = PublicSignalIndex.ownerHidden.nullifier
            guard proof.pubSignals.count > nfIdx else {
                throw NSError(domain: "DogTag.OwnerHidden", code: 2,
                              userInfo: [NSLocalizedDescriptionKey: "proof omitted the consent nullifier"])
            }
            let nullifier = proof.pubSignals[nfIdx]
            let verificationRegistry = cs.verificationRegistry
            if await RoaxRpc.consumed(
                rpcUrl: RpcEndpointSettings.rpcUrl(), verificationRegistry: verificationRegistry,
                nullifier: nullifier) {
                working = false
                status = ""
                outcome = .refused(.alreadyRecorded)
                return
            }

            let proofObject: [String: Any] = [
                "a": proof.a, "b": proof.b, "c": proof.c, "pubSignals": proof.pubSignals,
            ]
            var payload: [String: Any] = ["exportToken": token, "proof": proofObject]
            // D1: the owner-picked identity disclosure rides ALONGSIDE the consent proof - same
            // session, same `R` - so it inherits the proof's relayer/deadline anti-replay binding.
            // The envelope is built by the SAME Rust core the verifier checks with and embedded
            // verbatim; the consent proof itself stays leaf-blind.
            if !revealKeyPaths.isEmpty {
                let disclosureJson = try buildProfileDisclosureJson(
                    seedHex: seedHex,
                    dogTagIdHex: dogTagIdFieldHex(dogTagIdDec: owner.dogTagIdDec),
                    ownerAddressHex: owner.ownerAddress,
                    attributes: owner.attributes.map {
                        AttributeLeafFfi(keyPath: $0.keyPath, saltHex: $0.saltHex, tag: $0.tag, value: $0.value)
                    },
                    revealKeyPaths: Array(revealKeyPaths))
                guard let disclosureData = disclosureJson.data(using: .utf8),
                      let disclosureObject = try? JSONSerialization.jsonObject(with: disclosureData) else {
                    throw NSError(domain: "DogTag.OwnerHidden", code: 4,
                                  userInfo: [NSLocalizedDescriptionKey: "could not encode the identity disclosure"])
                }
                payload["profileDisclosure"] = disclosureObject
            }
            let payloadData = try JSONSerialization.data(withJSONObject: payload)
            guard let payloadJson = String(data: payloadData, encoding: .utf8) else {
                throw NSError(domain: "DogTag.OwnerHidden", code: 3,
                              userInfo: [NSLocalizedDescriptionKey: "could not encode consent proof"])
            }

            status = "Submitting owner-hidden proof to groomer…"
            let response = await CentralApi.postVerifyConsentToHost(
                host: host, payloadJson: payloadJson)
            if (400..<500).contains(response.code),
               let data = response.body.data(using: .utf8),
               let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
               let reject = object["error"] as? String, !reject.isEmpty {
                working = false
                status = ""
                outcome = .couldNotComplete(.proofRejected("\(reject)"))
                return
            }

            // Detached-broadcast read-back: the session reports terminal errors while the registry's
            // consumed(nullifier) bit is the canonical success signal.
            status = "Recording your owner-hidden verification on-chain…"
            var done = false
            var failedMsg: String? = nil
            for _ in 0..<40 {
                if await RoaxRpc.consumed(
                    rpcUrl: RpcEndpointSettings.rpcUrl(), verificationRegistry: verificationRegistry,
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
                status = ""
                outcome = .couldNotComplete(.recordingFailed(failedMsg))
                return
            }
            if done {
                let tx = await CentralApi.verifySessionStatus(
                    host: host, sessionId: sess.sessionId, token: token)?.txHash
                working = false
                if let tx, !tx.isEmpty {
                    status = ""
                    outcome = .succeeded(txHash: String(tx.prefix(14)) + "…")
                } else {
                    status = ""
                    outcome = .succeeded(txHash: nil)
                }
            } else {
                working = false
                status = ""
                outcome = .awaitingConfirmation
            }
        } catch {
            // Reached only by a throw AFTER the anchor check passed (proving/submission). A refusal
            // by `validateDiscovery` has its own catch above and must never arrive here as a failure.
            working = false
            status = ""
            outcome = .couldNotComplete(.unexpected("\(error)"))
        }
    }

    /// Colour for an outcome's tone. A refusal is deliberately the WARNING colour, not the danger
    /// colour: nothing is broken, DogTag decided. Three tones so the holder can tell "it worked",
    /// "we stopped this" and "it failed" apart at a glance, by colour AND by icon.
    private func outcomeTint(_ o: ConsentOutcome) -> Color {
        switch o.tone {
        case .success: return c.success
        case .blocked: return c.warning
        case .failure: return c.danger
        }
    }

    @ViewBuilder
    private func consentOutcomeCard(_ o: ConsentOutcome) -> some View {
        let tint = outcomeTint(o)
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .center, spacing: 10) {
                Image(systemName: o.iconName).font(.system(size: 22, weight: .semibold)).foregroundColor(tint)
                Text(o.title).font(.system(size: 17, weight: .bold)).foregroundColor(c.onBackground)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // THE ANSWER TO THE FIRST QUESTION A HOLDER ASKS. Its own line, above the prose, because
            // "did my record go out?" must not have to be inferred from a paragraph.
            if o.nothingWasShared {
                Text("Your record was not shared.")
                    .font(.system(size: 14, weight: .semibold)).foregroundColor(tint)
            }
            Text(o.explanation).font(.system(size: 13)).foregroundColor(c.onBackground)
                .fixedSize(horizontal: false, vertical: true)
            if let detail = o.technicalDetail, !detail.isEmpty {
                Text(detail).font(.system(size: 11, design: .monospaced)).foregroundColor(c.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if o.suggestsRetry {
                Text("You can try again.").font(.system(size: 12)).foregroundColor(c.muted)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 12).fill(tint.opacity(0.10)))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(tint, lineWidth: 1.5))
    }

    // ---- helpers ----

    /// The selected tag's persisted `owner.identity.*` attribute openings - the leaves the owner
    /// CAN disclose. Empty for tags issued before D1 (nothing to pick, the card hides).
    private func identityAttributes(forDogTagIdDec dec: String) -> [ProfileTreeStore.BackedUpAttribute] {
        guard let record = ProfileTreeStore.record(forDogTagIdDec: dec) else { return [] }
        return record.attributes.filter { $0.keyPath.hasPrefix("owner.identity.") }
    }

    /// Owner-readable label for an identity keyPath (falls back to the raw suffix).
    private func identityLabel(for keyPath: String) -> String {
        switch keyPath {
        case "owner.identity.fullName": return "Full name"
        case "owner.identity.country": return "Country"
        case "owner.identity.docNumber": return "ID number"
        default: return String(keyPath.dropFirst("owner.identity.".count))
        }
    }

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
