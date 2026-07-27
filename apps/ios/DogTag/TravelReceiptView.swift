import SwiftUI
import CoreImage.CIFilterBuiltins

/// The pet-owner's mobile TRAVEL_CLEARANCE receipt — the CDC-modeled "show this on your phone"
/// surface (govarch-r8 §3, mirrors the web receipt `stacks/government/web/src/pages/Receipt.tsx`).
///
/// It renders entirely from the LOCALLY stored `wrappedDocJson` (offline-capable except the one live
/// on-chain status read): letterhead, effectiveStatus banner, Receipt ID / issuance / validity block,
/// the legal preamble, Section A/B/C tables, and a Verification block whose QR points at the PII-free
/// public status page `/r/:receiptId` (the endpoint PR-1 built).
///
/// PRESENT is holder-controlled selective disclosure: Section-A person-PII leaves default to WITHHELD,
/// Section B/C default to shown, and `dogTagId` is locked visible. "Share redacted copy" runs the
/// merkle `obfuscate()` (exposed in the mobile FFI as `obfuscateDocumentJson`) LOCALLY so the shared
/// WrappedDoc hides the withheld leaves while the tree still rebuilds to the same on-chain root R —
/// a PII-free presentation proof produced on the phone with zero new ceremony.
struct TravelReceiptView: View {
    @Environment(\.dogTagColors) var c
    @Environment(\.dismiss) var dismiss
    let cred: Credential

    /// Parsed once at init (mirrors Android's `remember(doc)`) so recomposition never re-parses.
    private let doc: WrappedDoc?
    /// `credentialSubject` leaves flattened to dotted paths WITHOUT the `credentialSubject.` prefix
    /// (e.g. `importer.firstName`, `animal.name`), decoded once from the packed `salt:tag:value` leaves.
    private let subject: [String: String]

    @State private var live: RoaxRpc.Result = .unknown("checking…")
    @State private var withheld: Set<String>
    @State private var shareItem: SharePayload? = nil
    @State private var shareError: String? = nil
    /// QR image cached across recompositions and generated off the main thread (mirrors Android's
    /// `remember(publicUrl)` + `Dispatchers.Default`), so reveal-toggles never regenerate it inline.
    @State private var qr: UIImage? = nil

    /// The amber "expired / withheld" accent. Now the theme's own `warning` token rather than a third
    /// hardcoded #b45309 — the list badges need the same colour for the same state, and a literal here
    /// would guarantee they drifted (and would ignore dark mode, as this one did).
    private var amber: Color { c.warning }

    init(cred: Credential) {
        self.cred = cred
        let parsed = WrappedDoc(json: cred.wrappedDocJson)
        self.doc = parsed
        var subj: [String: String] = [:]
        for f in parsed?.decodedFields() ?? [] {
            guard f.keyPath.hasPrefix("credentialSubject.") else { continue }
            subj[String(f.keyPath.dropFirst("credentialSubject.".count))] = f.value
        }
        self.subject = subj
        // Seed WITHHELD eagerly to every present Section-A leaf so the first render never shows
        // person PII in cleartext (privacy-by-default; mirrors Android's eager `remember` init).
        let present = Self.sectionAPaths.filter { !(subj[$0.path] ?? "").isEmpty }
        _withheld = State(initialValue: Set(present.map { $0.path }))
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                topBar
                disclosureControls
                receiptSheet
                Spacer(minLength: 24)
            }
            .padding(20)
        }
        .background(c.background.ignoresSafeArea())
        .task { await refreshStatus() }
        .task(id: publicStatusUrl) { await generateQR() }
        .sheet(item: $shareItem) { ShareSheet(items: [$0.text]) }
    }

    // MARK: - decoded subject

    private func pick(_ path: String) -> String { subject[path] ?? "" }
    private func isTrue(_ path: String) -> Bool {
        let v = pick(path).lowercased()
        return v == "true" || v == "1" || v == "yes"
    }

    private var receiptId: String { pick("receiptId") }
    /// Issuance date: the `validity.issuedOn` leaf (PR-1 schema), falling back to the imported record's.
    private var issuanceDate: String {
        let leaf = pick("validity.issuedOn")
        return leaf.isEmpty ? cred.issuedOn : leaf
    }
    private var isTravel: Bool { (cred.recordType.isEmpty ? (doc?.recordType ?? "") : cred.recordType).uppercased().contains("TRAVEL") }

    /// Public PII-free status URL the QR encodes: `<protocol.statusBaseUrl>/r/<receiptId>`.
    ///
    /// The base comes from the document's `protocol` block, stamped at issuance from the issuer's
    /// `DEPLOYMENT_URL` — the only host in the document a phone can actually reach. It deliberately
    /// does NOT fall back to `issuer.domain`: that is a `did:web` identity, and the shipped default
    /// `gov.example` is RFC-2606 reserved (NXDOMAIN), so every QR built from it encoded a dead link
    /// that a border official would read as a working live-status check.
    ///
    /// "" when the issuer stamped no base — including every document issued before it did. The card
    /// then renders no QR and says so, rather than printing a URL that resolves to nothing.
    private var publicStatusUrl: String {
        let base = doc?.statusBaseUrl ?? ""
        guard !base.isEmpty, !receiptId.isEmpty else { return "" }
        return "\(base)/r/\(receiptId)"
    }

    // MARK: - effective status

    /// The receipt verdict, derived like the web `effectiveStatus`: a live revoke wins, then a lapsed
    /// validity window, else VALID. When the chain is unreachable we fall back to the stored verdict —
    /// but ONLY while that stored verdict is still fresh.
    ///
    /// That freshness gate is the fix. This pill previously read a green VALID off `cred.verdict`
    /// whenever the chain was unreachable, no matter how old that stored answer was, while the sub-line
    /// underneath said "On-chain status unconfirmed" — two contradictory claims on one screen, and the
    /// loud one was the one that could be wrong. An unreachable chain plus a stale stored answer is
    /// exactly "I could not check", and `.unknown` is what that looks like.
    private enum Eff { case valid, expired, revoked, unknown }
    private var effective: Eff {
        switch live {
        case .invalid: return .revoked
        case .valid:
            return validUntilLapsed ? .expired : .valid
        case .unknown:
            if validUntilLapsed { return .expired }
            let fresh = VerdictDisplay.isFresh(cred.lastCheckedAt, now: Date())
            return cred.verdict == "VALID" && fresh ? .valid : .unknown
        }
    }

    /// THE single implementation of "is this expired", shared with the list badges. It lived here as a
    /// private comparison, which is how this sheet came to be the only mobile surface that enforced
    /// expiry at all: one rule with one implementation cannot be half-adopted.
    private var validUntilLapsed: Bool {
        VerdictDisplay.lapsed(pick("validity.validUntil"), now: Date())
    }
    private var effLabel: String {
        switch effective { case .valid: return "VALID"; case .expired: return "EXPIRED"
        case .revoked: return "REVOKED"; case .unknown: return "UNCONFIRMED" }
    }
    private var effAccent: Color {
        switch effective { case .valid: return c.success; case .expired: return amber
        case .revoked: return c.danger; case .unknown: return c.muted }
    }

    private func refreshStatus() async {
        guard let d = doc else { return }
        let root = d.merkleRoot.isEmpty ? cred.credentialRoot : d.merkleRoot
        live = await RoaxRpc.isValid(rpcUrl: AppConfig.roaxRpc, documentStore: d.documentStore, root: root)
    }

    // MARK: - top bar

    private var topBar: some View {
        HStack {
            Button { dismiss() } label: {
                Image(systemName: "xmark").foregroundColor(c.onBackground)
                    .frame(width: 32, height: 32).background(Circle().fill(c.surfaceVariant))
            }.buttonStyle(.plain)
            Spacer()
            Button {
                shareRedacted()
            } label: {
                Label("Share redacted", systemImage: "square.and.arrow.up")
                    .font(.system(size: 13, weight: .semibold))
                    .padding(.horizontal, 14).padding(.vertical, 8)
                    .background(Capsule().fill(c.accent)).foregroundColor(c.onAccent)
            }.buttonStyle(.plain)
        }
    }

    // MARK: - selective disclosure

    /// Section-A person/importer leaves — the PII block that defaults to WITHHELD when presenting.
    private static let sectionAPaths: [(label: String, path: String)] = {
        [
            ("First Name", "importer.firstName"),
            ("Middle Name/Initial", "importer.middleName"),
            ("Last Name", "importer.lastName"),
            ("Role", "importer.role"),
            ("Identification Type", "importer.idType"),
            ("ID Jurisdiction", "importer.idJurisdiction"),
            ("Identification No.", "importer.idNumber"),
            ("Date of Birth", "importer.dateOfBirth"),
            ("Email", "importer.email"),
            ("Phone Number", "importer.phone"),
            ("Consignee/Additional Owner", "consignee.fullName"),
            ("Consignee Email", "consignee.email"),
            ("Consignee ID Type", "consignee.idType"),
        ]
    }()
    /// Present Section-A paths (a value exists in the credential).
    private var presentSectionA: [(label: String, path: String)] {
        Self.sectionAPaths.filter { !pick($0.path).isEmpty }
    }

    private var disclosureControls: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Image(systemName: "eye.slash").font(.system(size: 13)).foregroundColor(amber)
                Text("Selective disclosure").font(.system(size: 13, weight: .bold)).foregroundColor(c.onBackground)
            }
            Text("Person (Section A) details are hidden by default. Reveal only what an official needs; hidden fields are stripped from a shared copy but still verify against the on-chain root.")
                .font(.system(size: 11)).foregroundColor(c.muted)
            if presentSectionA.isEmpty {
                Text("This receipt carries no Section-A personal fields.").font(.system(size: 11)).foregroundColor(c.muted)
            } else {
                ForEach(presentSectionA, id: \.path) { item in
                    Toggle(isOn: Binding(
                        get: { !withheld.contains(item.path) },
                        set: { reveal in
                            if reveal { withheld.remove(item.path) } else { withheld.insert(item.path) }
                        }
                    )) {
                        Text("Reveal \(item.label)").font(.system(size: 12)).foregroundColor(c.onSurface)
                    }
                    .tint(c.accent)
                }
            }
            if let e = shareError {
                Text(e).font(.system(size: 11)).foregroundColor(c.danger)
            }
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 14).fill(c.surface))
    }

    /// Run the merkle `obfuscate()` LOCALLY over the withheld leaves and hand the redacted WrappedDoc
    /// to the OS share sheet. `credentialSubject.` prefix is restored for the FFI key paths.
    private func shareRedacted() {
        shareError = nil
        let paths = withheld.map { "credentialSubject.\($0)" }
        do {
            let redacted = paths.isEmpty
                ? cred.wrappedDocJson
                : try obfuscateDocumentJson(wrappedDocJson: cred.wrappedDocJson, keyPaths: paths)
            shareItem = SharePayload(text: redacted)
        } catch {
            shareError = "Could not redact: \(error.localizedDescription)"
        }
    }

    // MARK: - receipt sheet

    private var receiptSheet: some View {
        VStack(alignment: .leading, spacing: 0) {
            letterhead
            VStack(alignment: .leading, spacing: 14) {
                titleAndStatus
                identityBlock
                legalPreamble
                if let n = doc?.obfuscatedCount, n > 0 {
                    Text("Note: \(n) field(s) were withheld by the holder in this copy; withheld leaves still verify against the on-chain root.")
                        .font(.system(size: 11)).foregroundColor(amber)
                }
                if isTravel {
                    sectionTable("Section A - Person Importing the Animal", sectionARows)
                    sectionTable("Section B - Animal Information", sectionBRows)
                    sectionTable("Section C - Travel Information", sectionCRows)
                } else {
                    sectionTable("Health Certificate (Annex IV)", healthRows)
                }
                verificationBlock
            }
            .padding(16)
        }
        .background(RoundedRectangle(cornerRadius: 18).fill(c.surface))
        .overlay(RoundedRectangle(cornerRadius: 18).stroke(c.outline, lineWidth: 1))
    }

    private var letterhead: some View {
        HStack(spacing: 12) {
            Text("🏛").font(.system(size: 28))
            VStack(alignment: .leading, spacing: 2) {
                Text(doc?.issuerName ?? "Competent Authority")
                    .font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
                Text("Pet Travel Clearance · authority-endorsed credential")
                    .font(.system(size: 12)).foregroundColor(c.muted)
            }
            Spacer()
        }
        .padding(16)
        .background(c.surfaceVariant)
        .clipShape(RoundedCorner(radius: 18, corners: [.topLeft, .topRight]))
    }

    private var titleAndStatus: some View {
        HStack {
            Text(isTravel ? "Pet Travel Clearance Receipt" : "Pet Health Certificate")
                .font(.system(size: 15, weight: .semibold)).foregroundColor(c.onBackground)
            Spacer()
            Text("● \(effLabel)")
                .font(.system(size: 12, weight: .bold)).foregroundColor(.white)
                .padding(.horizontal, 10).padding(.vertical, 5)
                .background(Capsule().fill(effAccent))
        }
    }

    private var identityBlock: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Receipt ID: \(receiptId.isEmpty ? "—" : receiptId)")
                .font(.system(size: 18, weight: .bold)).foregroundColor(c.onBackground)
            row("Date of issuance", issuanceDate.isEmpty ? "—" : issuanceDate)
            row(isTrue("validity.multipleEntries") ? "Valid for multiple entries until" : "Valid until",
                pick("validity.validUntil").isEmpty ? "—" : pick("validity.validUntil"))
            row("Dog Tag ID (SBT)", "#\(pick("dogTagId").isEmpty ? cred.dogTagId : pick("dogTagId"))")
        }
    }

    private var legalPreamble: some View {
        let binding = pick("validity.countryOfDepartureBinding")
        return Text("This receipt is valid for the animal listed for the validity window shown above\(binding.isEmpty ? "" : ", for entry from the listed country of departure (\(binding))"). If the animal travels via a different or high-risk country, a new clearance may be required. ")
            .font(.system(size: 12)).foregroundColor(c.muted)
        + Text("You must show this receipt (printed or on your phone) to airline staff and port-of-entry officials.")
            .font(.system(size: 12, weight: .bold)).foregroundColor(c.onBackground)
        + Text(" The authority reserves the right to request additional supporting documentation on arrival.")
            .font(.system(size: 12)).foregroundColor(c.muted)
    }

    // MARK: - section rows

    private struct Row { let label: String; let value: String; let withheld: Bool }

    private func mk(_ label: String, _ path: String) -> Row {
        Row(label: label, value: pick(path), withheld: withheld.contains(path))
    }

    private var sectionARows: [Row] {
        // "The person listed above is the" (role) + humanized values, mirroring the web layout.
        var rows = presentSectionA.map { item -> Row in
            let v: String
            switch item.path {
            case "importer.role", "importer.idType", "consignee.idType": v = humanize(pick(item.path))
            default: v = pick(item.path)
            }
            return Row(label: item.label, value: v, withheld: withheld.contains(item.path))
        }
        rows = rows.filter { !$0.value.isEmpty }
        return rows
    }

    private var sectionBRows: [Row] {
        let sex = pick("animal.sex")
        let sexValue = sex.isEmpty ? "" : "\(sex.prefix(1).uppercased())\(sex.dropFirst())\(isTrue("animal.neutered") ? " Neutered" : "")"
        return [
            mk("Animal Name", "animal.name"),
            mk("Age - Year(s)", "animal.ageYears"),
            mk("Age - Month(s)", "animal.ageMonths"),
            Row(label: "Sex", value: sexValue, withheld: false),
            mk("Dog Breed", "animal.breed"),
            mk("Color/Markings", "animal.colorMarkings"),
            mk("Microchip Number", "animal.microchipNumber"),
            Row(label: "Importation Purpose", value: humanize(pick("animal.importationPurpose")), withheld: false),
        ].filter { !$0.value.isEmpty }
    }

    private var sectionCRows: [Row] {
        [
            Row(label: "Travel Type", value: humanize(pick("travel.travelType")), withheld: false),
            mk("Country or Area of Departure", "travel.countryOfDeparture"),
            mk("Date of Arrival", "travel.dateOfArrival"),
            mk("Port of Entry", "travel.portOfEntry"),
            mk("Carrier / Flight", "travel.carrierOrFlight"),
        ].filter { !$0.value.isEmpty }
    }

    private var healthRows: [Row] {
        [
            mk("Species", "species"),
            mk("Microchip Number", "microchipNumber"),
            mk("Rabies Vaccination Date", "rabiesVaccinationDate"),
            mk("Rabies Valid Until", "rabiesValidUntil"),
            mk("Examining Veterinarian", "examiningVeterinarian"),
            Row(label: "Clinical Health Status", value: humanize(pick("clinicalHealthStatus")), withheld: false),
            mk("Examination Date", "examinationDate"),
        ].filter { !$0.value.isEmpty }
    }

    @ViewBuilder
    private func sectionTable(_ title: String, _ rows: [Row]) -> some View {
        if !rows.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                Text(title)
                    .font(.system(size: 12, weight: .bold)).foregroundColor(.white)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12).padding(.vertical, 8)
                    .background(Color(hex: 0x334155))
                ForEach(Array(rows.enumerated()), id: \.offset) { i, r in
                    HStack(alignment: .top, spacing: 10) {
                        Text(r.label).font(.system(size: 12, weight: .semibold)).foregroundColor(c.muted)
                            .frame(width: 130, alignment: .leading)
                        if r.withheld {
                            Text("— withheld by holder —").font(.system(size: 13, weight: .medium)).foregroundColor(amber)
                        } else {
                            Text(r.value).font(.system(size: 13)).foregroundColor(c.onSurface)
                        }
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 12).padding(.vertical, 8)
                    .background(i % 2 == 0 ? c.surface : c.surfaceVariant.opacity(0.5))
                }
            }
            .overlay(Rectangle().stroke(c.outline, lineWidth: 1))
            .padding(.top, 4)
        }
    }

    // MARK: - verification block

    private var verificationBlock: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Verification")
                .font(.system(size: 12, weight: .bold)).foregroundColor(.white)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12).padding(.vertical, 8)
                .background(Color(hex: 0x334155))
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .top, spacing: 14) {
                    if let img = qr, !publicStatusUrl.isEmpty {
                        Image(uiImage: img).interpolation(.none).resizable()
                            .frame(width: 116, height: 116)
                            .background(Color.white).cornerRadius(6)
                    }
                    VStack(alignment: .leading, spacing: 4) {
                        Text(publicStatusUrl.isEmpty
                             ? "No public status page for this receipt"
                             : "Scan to confirm live status on-chain")
                            .font(.system(size: 13, weight: .semibold)).foregroundColor(c.onBackground)
                        Text(publicStatusUrl.isEmpty
                             ? "This credential's issuer published no reachable status URL. Confirm it against the on-chain record below."
                             : publicStatusUrl)
                            .font(.system(size: 11)).foregroundColor(c.muted).lineLimit(3)
                        Text("The public page shows NO personal data — only the live VALID / EXPIRED / REVOKED verdict and provenance.")
                            .font(.system(size: 10)).foregroundColor(c.muted)
                    }
                }
                Divider().overlay(c.outline)
                liveStatusRow
                let root = (doc?.merkleRoot).flatMap { $0.isEmpty ? nil : $0 } ?? cred.credentialRoot
                CopyableMonoRow(label: "Credential root", value: root)
                if let store = doc?.documentStore, !store.isEmpty {
                    Text("Anchored: ROAX chainId 135 · issuer \(store.prefix(12))…")
                        .font(.system(size: 11)).foregroundColor(c.muted)
                }
            }
            .padding(12)
        }
        .overlay(Rectangle().stroke(c.outline, lineWidth: 1))
        .padding(.top, 4)
    }

    private var liveStatusRow: some View {
        let (text, color): (String, Color) = {
            switch live {
            case .valid: return ("On-chain: anchored & valid", c.success)
            case .invalid: return ("On-chain: REVOKED / not anchored", c.danger)
            case let .unknown(r): return ("On-chain status unconfirmed (\(r))", c.muted)
            }
        }()
        return HStack(spacing: 8) {
            Circle().fill(color).frame(width: 9, height: 9)
            Text(text).font(.system(size: 12, weight: .medium)).foregroundColor(color)
        }
    }

    // MARK: - small helpers

    private func row(_ label: String, _ value: String) -> some View {
        HStack(alignment: .top) {
            Text(label).font(.system(size: 12, weight: .semibold)).foregroundColor(c.muted)
                .frame(width: 150, alignment: .leading)
            Text(value).font(.system(size: 13)).foregroundColor(c.onSurface)
            Spacer(minLength: 0)
        }
    }

    /// Humanize a snake_case enum value ("service_animal" → "Service Animal"); leave codes/blank as-is.
    private func humanize(_ v: String) -> String {
        if v.isEmpty { return "" }
        if v == "true" { return "Yes" }
        if v == "false" { return "No" }
        guard v.contains("_") else { return v }
        return v.split(separator: "_").map { $0.isEmpty ? "" : $0.prefix(1).uppercased() + $0.dropFirst() }.joined(separator: " ")
    }

    /// Build the QR off the main thread once per `publicStatusUrl`, then publish it on the main actor.
    private func generateQR() async {
        let url = publicStatusUrl
        guard !url.isEmpty else { qr = nil; return }
        let img = await Task.detached(priority: .userInitiated) { Self.qrImage(url) }.value
        qr = img
    }

    /// Generate a QR code image for `value` (PII-free public status URL). CoreImage, offline.
    /// `nonisolated`: pure CoreImage compute (no UI/actor state), so it runs off the main actor
    /// from `generateQR`'s `Task.detached` without hopping back - the whole point of detaching.
    private nonisolated static func qrImage(_ value: String) -> UIImage? {
        guard !value.isEmpty else { return nil }
        let ctx = CIContext()
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(value.utf8)
        filter.correctionLevel = "M"
        guard let out = filter.outputImage else { return nil }
        let scaled = out.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        guard let cg = ctx.createCGImage(scaled, from: scaled.extent) else { return nil }
        return UIImage(cgImage: cg)
    }
}

// MARK: - support types

/// Identifiable share payload for the `.sheet(item:)` presentation.
private struct SharePayload: Identifiable { let id = UUID(); let text: String }

/// A UIActivityViewController bridge for the redacted-doc share sheet.
private struct ShareSheet: UIViewControllerRepresentable {
    let items: [Any]
    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }
    func updateUIViewController(_ vc: UIActivityViewController, context: Context) {}
}

/// Round specific corners (used for the letterhead's top corners).
private struct RoundedCorner: Shape {
    var radius: CGFloat
    var corners: UIRectCorner
    func path(in rect: CGRect) -> Path {
        Path(UIBezierPath(roundedRect: rect, byRoundingCorners: corners,
                          cornerRadii: CGSize(width: radius, height: radius)).cgPath)
    }
}
