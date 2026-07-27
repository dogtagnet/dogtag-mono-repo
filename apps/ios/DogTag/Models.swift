import Foundation

/// A credential group as shown on Home (Health / Service / Travel). Derived from the on-chain
/// `recordType` label. Mirrors Models.kt.
enum CredentialGroup: String, CaseIterable, Identifiable, Codable {
    case health, service, travel
    var id: String { rawValue }
    var title: String {
        switch self {
        case .health: return "Health Records"
        case .service: return "Service Dog"
        case .travel: return "Travel Docs"
        }
    }

    /// Map an issuer recordType label (e.g. "VACCINATION", "SERVICE_ATTESTATION") to a group.
    static func from(recordType: String?) -> CredentialGroup {
        let rt = (recordType ?? "").uppercased()
        if rt.contains("SERVICE") || rt.contains("DOT") { return .service }
        if rt.contains("TRAVEL") || rt.contains("CDC") || rt.contains("EU_HEALTH") ||
            rt.contains("IMPORT") || rt.contains("USDA") { return .travel }
        return .health
    }
}

/// A pet the user owns. Seeded from central GET /v1/pets and/or imported records. Keyed by dogTagId.
struct Pet: Identifiable, Codable, Equatable {
    var dogTagId: String     // on-chain DogTagSBT tokenId (decimal) — primary key
    var name: String
    var breed: String
    var ageLabel: String
    var microchip: String?

    var id: String { dogTagId }

    /// Parse a pet from the central GET /v1/pets `pets[]` entry.
    static func fromCentral(_ o: [String: Any]) -> Pet {
        let mc = o["microchip"] as? [String: Any]
        let profile = o["profile"] as? [String: Any]
        let tagFromChain = (o["dogTagId"] as? String) ?? ""
        let tag = tagFromChain.isEmpty ? ((o["id"] as? String) ?? "") : tagFromChain
        return Pet(
            dogTagId: tag,
            name: (o["name"] as? String) ?? "Unnamed",
            breed: (profile?["breed"] as? String) ?? "",
            ageLabel: (profile?["dateOfBirth"] as? String) ?? "",
            microchip: (mc?["code"] as? String)
        )
    }
}

/// A single imported credential / record held for a pet. The full wrapped doc JSON is kept so the
/// verification can be re-run and the record re-presented (consent over `credentialRoot`).
struct Credential: Identifiable, Codable, Equatable {
    var id: String                // recordId from the vet record link
    var dogTagId: String          // owning pet
    var group: CredentialGroup
    var recordType: String
    var title: String
    var subtitle: String
    var issuer: String
    var issuedOn: String
    var credentialRoot: String    // signature.merkleRoot (0x..) — what consent signs over
    var verdict: String           // "VALID" / "INVALID" / "UNVERIFIED"
    var wrappedDocJson: String    // the full wrapped doc (for re-verify + disclosure)

    // ---- provenance / freshness (all optional: records stored before these shipped have none) ----

    /// When THIS device stored the record (`Stamp` ISO-8601 UTC). Two records of the same type are
    /// otherwise indistinguishable in a list, so this is what tells them apart for the owner.
    var importedAt: String?
    /// When the on-chain status behind `verdict` was last determined, successfully or not. Written
    /// together with `verdict` + `verdictReason` so all three always describe the SAME check.
    var lastCheckedAt: String?
    /// Short human explanation of `verdict` - above all, why it is not VALID.
    var verdictReason: String?
}

/// ISO-8601 (UTC, whole seconds) stamping and human display for the credential timestamps. Android
/// writes the identical shape ("2026-07-27T14:32:10Z"), so the two stores stay readable across ports.
enum Stamp {
    /// The current instant, in the stored form.
    static func now() -> String { writer.string(from: Date()) }

    /// Parse a stored stamp, tolerating a fractional-seconds variant. nil for absent/unparseable.
    static func parse(_ raw: String?) -> Date? {
        guard let raw = raw, !raw.isEmpty else { return nil }
        return writer.date(from: raw) ?? fractional.date(from: raw)
    }

    /// Absolute local date + time, e.g. "27 Jul 2026 at 14:32". Used where the exact moment is the
    /// point (which of two look-alike records is which).
    static func absolute(_ date: Date) -> String { absoluteFormatter.string(from: date) }

    /// Relative age, e.g. "just now" / "5 minutes ago" / "3 days ago". Used for freshness, where the
    /// distinction that matters is "checked seconds ago" vs "checked last week".
    static func relative(_ date: Date) -> String {
        Date().timeIntervalSince(date) < 60 ? "just now" : relativeFormatter.localizedString(for: date, relativeTo: Date())
    }

    private static let writer: ISO8601DateFormatter = ISO8601DateFormatter()
    private static let fractional: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
    private static let absoluteFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .medium
        f.timeStyle = .short
        return f
    }()
    private static let relativeFormatter: RelativeDateTimeFormatter = {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .full
        return f
    }()
}

extension Credential {
    /// When this record landed on this phone, bare. Records stored before import stamping shipped
    /// carry no stamp: say so plainly rather than inventing a date.
    var importedAtValue: String {
        guard let d = Stamp.parse(importedAt) else { return "Unknown" }
        return Stamp.absolute(d)
    }

    /// The same, as a standalone list line.
    var importedAtLabel: String {
        Stamp.parse(importedAt) == nil ? "Import date unknown" : "Imported \(importedAtValue)"
    }

    /// How fresh `verdict` is. A verdict is frozen at the moment it was read off the chain, so a
    /// record revoked since then still reads VALID until it is refreshed - this is what makes that
    /// staleness visible instead of silent.
    var lastCheckedLabel: String {
        guard let d = Stamp.parse(lastCheckedAt) else { return "Not checked on-chain yet" }
        return "Checked \(Stamp.relative(d))"
    }

    /// The freshness line, plus the reason whenever the verdict is anything other than VALID.
    var statusLine: String {
        guard verdict != "VALID", let why = verdictReason, !why.isEmpty else { return lastCheckedLabel }
        return "\(lastCheckedLabel) · \(why)"
    }

    /// Body of the delete confirmation. It leads with the details that tell two same-type records
    /// apart, so the owner can see WHICH one is about to go, then states plainly what deleting does.
    /// Deleting is local: it must not read as a revocation, because it is not one.
    func deleteConfirmationMessage(petLabel: String) -> String {
        let which = petLabel.isEmpty ? importedAtLabel : "\(importedAtLabel) · \(petLabel)"
        return which + "\n\nThis removes the copy stored on this phone. The record is not revoked, "
            + "nothing changes on-chain, and the issuer still holds their copy."
    }
}

/// A thin, typed view over a wrapped-doc JSON (§1.4 WrappedDoc). Extracts the fields the app needs;
/// the canonicalization heavy-lifting stays in Rust (`verifyIntegrity` / `buildMerkleRootHex`).
struct WrappedDoc {
    let json: String
    private let root: [String: Any]
    private var sig: [String: Any] { (root["signature"] as? [String: Any]) ?? [:] }
    private var issuerObj: [String: Any] { (root["issuer"] as? [String: Any]) ?? [:] }
    private var data: [String: Any] { (root["data"] as? [String: Any]) ?? [:] }

    init?(json: String) {
        guard let d = json.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any] else { return nil }
        self.json = json
        self.root = o
    }

    var merkleRoot: String { (sig["merkleRoot"] as? String) ?? "" }
    var targetHash: String { (sig["targetHash"] as? String) ?? "" }
    var documentStore: String { (issuerObj["documentStore"] as? String) ?? "" }
    var issuerName: String { (issuerObj["name"] as? String) ?? "Unknown issuer" }
    var issuerDomain: String { (issuerObj["domain"] as? String) ?? "" }
    var recordType: String { (issuerObj["recordType"] as? String) ?? "" }

    var dogTagId: String {
        let cs = data["credentialSubject"] as? [String: Any]
        let raw = (cs?["dogTagId"] as? String) ?? ""
        if let tail = raw.split(separator: ":").last { return String(tail) }
        return raw
    }

    func displayTitle() -> String {
        let rt = recordType.isEmpty ? "Record" : recordType
        return rt.replacingOccurrences(of: "_", with: " ").capitalized
    }

    /// The number of leaf hashes the issuer redacted (selective disclosure).
    var obfuscatedCount: Int {
        let privacy = root["privacy"] as? [String: Any]
        return (privacy?["obfuscated"] as? [Any])?.count ?? 0
    }

    /// One decoded Merkle leaf: dotted keyPath, type tag, and human-readable value.
    struct DecodedField: Identifiable {
        let keyPath: String
        let tag: Int
        let value: String
        var id: String { keyPath }
        /// A title-cased label derived from the keyPath (strips a leading `credentialSubject.`).
        var label: String { WrappedDoc.humanizeKeyPath(keyPath) }
    }

    /// Flatten `data` into an ordered list of decoded leaves. Objects recurse with dotted key paths;
    /// arrays index with `[i]`. Each scalar leaf is parsed `"<salt>:<tag>:<value>"` (first two ':').
    func decodedFields() -> [DecodedField] {
        var out: [DecodedField] = []
        WrappedDoc.flatten(data, prefix: "", into: &out)
        return out
    }

    private static func flatten(_ node: Any?, prefix: String, into out: inout [DecodedField]) {
        if let obj = node as? [String: Any] {
            // Preserve a stable order: sort keys so output is deterministic.
            for k in obj.keys.sorted() {
                let path = prefix.isEmpty ? k : "\(prefix).\(k)"
                flatten(obj[k], prefix: path, into: &out)
            }
        } else if let arr = node as? [Any] {
            for (i, child) in arr.enumerated() {
                flatten(child, prefix: "\(prefix)[\(i)]", into: &out)
            }
        } else if let s = node as? String {
            out.append(parseLeaf(keyPath: prefix, raw: s))
        } else if let n = node, !(n is NSNull) {
            out.append(DecodedField(keyPath: prefix, tag: 2, value: "\(n)"))
        }
    }

    /// Parse a packed `"<salt>:<tag>:<value>"` leaf — split on the FIRST TWO colons only
    /// (the value may itself contain ':').
    static func parseLeaf(keyPath: String, raw: String) -> DecodedField {
        guard let first = raw.firstIndex(of: ":") else {
            return DecodedField(keyPath: keyPath, tag: 2, value: raw)
        }
        let afterFirst = raw.index(after: first)
        guard let second = raw[afterFirst...].firstIndex(of: ":") else {
            return DecodedField(keyPath: keyPath, tag: 2, value: raw)
        }
        let tag = Int(raw[afterFirst..<second]) ?? 2
        let value = String(raw[raw.index(after: second)...])
        return DecodedField(keyPath: keyPath, tag: tag, value: value)
    }

    /// Humanize a dotted keyPath into a Title Case label. Strips a leading `credentialSubject.`,
    /// splits on dots, splits camelCase into words, drops array indices, title-cases.
    /// e.g. `credentialSubject.microchip.code` -> "Microchip code".
    static func humanizeKeyPath(_ keyPath: String) -> String {
        var path = keyPath
        if path.hasPrefix("credentialSubject.") {
            path = String(path.dropFirst("credentialSubject.".count))
        }
        // Capture array indices (1-based) and strip the brackets.
        var indices: [Int] = []
        do {
            let re = try NSRegularExpression(pattern: "\\[(\\d+)\\]")
            let ns = path as NSString
            for m in re.matches(in: path, range: NSRange(location: 0, length: ns.length)) {
                if let i = Int(ns.substring(with: m.range(at: 1))) { indices.append(i + 1) }
            }
            path = re.stringByReplacingMatches(in: path, range: NSRange(location: 0, length: ns.length), withTemplate: "")
        } catch {}

        let words = path.split(separator: ".").flatMap { splitCamel(String($0)) }
        guard !words.isEmpty else { return keyPath }
        let titled = words.enumerated().map { i, w in
            i == 0 ? w.prefix(1).uppercased() + w.dropFirst() : w.lowercased()
        }.joined(separator: " ")
        return indices.isEmpty ? titled : titled + " " + indices.map(String.init).joined(separator: " ")
    }

    private static func splitCamel(_ s: String) -> [String] {
        var out: [String] = []
        var current = ""
        let chars = Array(s)
        for (i, ch) in chars.enumerated() {
            if i > 0, ch.isUppercase {
                let prev = chars[i - 1]
                let next: Character? = i + 1 < chars.count ? chars[i + 1] : nil
                // Boundary: lower/digit -> Upper, or Upper -> Upper followed by lower (acronym end).
                if prev.isLowercase || prev.isNumber || (prev.isUppercase && (next?.isLowercase ?? false)) {
                    if !current.isEmpty { out.append(current); current = "" }
                }
            }
            current.append(ch)
        }
        if !current.isEmpty { out.append(current) }
        return out.filter { !$0.isEmpty }
    }
}

// --------------------------------------------------------------------------------------------
// Record-display helpers (impl §6 UX). Every credential list/detail must state WHAT a record is and
// WHICH pet it belongs to — never a bare "Dog Profile". These derive that from data the app already
// holds (the WrappedDoc leaves + the local Pet / DOG_PROFILE name), shared by Home, Documents, Travel,
// the credential detail and the export picker so the presentation stays consistent.
// --------------------------------------------------------------------------------------------

/// Formats the "which pet" line shared across every record display.
enum PetLabel {
    /// "<name> · DogTag #<id>", or just "DogTag #<id>", or "" when neither is known.
    static func line(name: String?, dogTagId: String) -> String {
        let tag = dogTagId.isEmpty ? "" : "DogTag #\(dogTagId)"
        if let n = name, !n.isEmpty { return tag.isEmpty ? n : "\(n) · \(tag)" }
        return tag
    }
}

extension WrappedDoc {
    /// The parsed (unsalted) value of the leaf whose keyPath exactly equals `keyPath`, or nil if absent.
    func leafValue(keyPath: String) -> String? {
        decodedFields().first { $0.keyPath == keyPath }?.value
    }

    /// The parsed value of the first leaf whose keyPath equals `suffix` or ends with ".<suffix>".
    /// Robust to whether a field sits at the `data` top level (vaccination fields) or nested under
    /// `credentialSubject` (profile fields).
    func leafValue(endingWith suffix: String) -> String? {
        decodedFields().first { $0.keyPath == suffix || $0.keyPath.hasSuffix("." + suffix) }?.value
    }

    /// The pet's name baked into a DOG_PROFILE credential (`credentialSubject.name`), or nil. Matched
    /// exactly so it is not confused with `ownerIdentity.name`.
    var petName: String? {
        guard let v = leafValue(keyPath: "credentialSubject.name"), !v.isEmpty else { return nil }
        return v
    }
}

extension Credential {
    /// The wrapped-doc view over this credential's stored JSON (nil if unparseable).
    var wrapped: WrappedDoc? { WrappedDoc(json: wrappedDocJson) }

    var isVaccination: Bool { recordType.uppercased().contains("VACCINATION") }

    /// A clear record-identity headline. Vaccinations read as "Rabies Vaccination" — this system issues
    /// only USDA-coded rabies certs (packages/ui `RABIES_VACCINATION`); the specific product + date come
    /// from `vaccinationDetail`. Every other type humanizes the recordType (DOG_PROFILE -> "Dog Profile").
    var displayTypeLabel: String {
        if isVaccination { return "Rabies Vaccination" }
        if !title.isEmpty && title != "Record" { return title }
        if let t = wrapped?.displayTitle(), !t.isEmpty { return t }
        return recordType.isEmpty ? "Record" : recordType
    }

    /// For a vaccination: "<product> · <date>" (either part may be missing). nil for other record types,
    /// so callers can conditionally show a vaccine detail line.
    var vaccinationDetail: String? {
        guard isVaccination, let d = wrapped else { return nil }
        let product = d.leafValue(endingWith: "vaccineProductName") ?? ""
        let date = d.leafValue(endingWith: "vaccinationDate") ?? ""
        let parts = [product, date].filter { !$0.isEmpty }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

}

/// The live ROAX (chainId 135) deployment addresses, loaded from the bundled `roax.json`.
struct RoaxConfig {
    let chainId: Int
    let dogTagSbt: String
    let issuerRegistry: String
    /// `DogTagIssuerFactory` — LINK 1 of the issuer↔domain chain (`isClone`). Empty means provenance
    /// cannot be checked, and the binding reports "could not read" rather than claiming anything.
    let issuerFactory: String
    /// `IssuerDomainRegistry` — the clone's on-chain domain claim. Empty until deployed; the binding then
    /// reports "could not read", never "no domain claimed".
    let issuerDomainRegistry: String
    let poseidon6: String
    /// `ProtocolRegistry` is the discovery trust anchor. Empty until redeployment; verification then
    /// fails closed instead of inventing an address.
    let protocolRegistry: String

    static func load() -> RoaxConfig {
        guard let url = Bundle.main.url(forResource: "roax", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            return RoaxConfig(chainId: 135, dogTagSbt: "", issuerRegistry: "",
                              issuerFactory: "", issuerDomainRegistry: "",
                              poseidon6: "", protocolRegistry: "")
        }
        return RoaxConfig(
            chainId: (o["chainId"] as? Int) ?? 135,
            dogTagSbt: (o["DogTagSBT"] as? String) ?? "",
            issuerRegistry: (o["IssuerRegistry"] as? String) ?? "",
            issuerFactory: (o["DogTagIssuerFactory"] as? String) ?? "",
            issuerDomainRegistry: (o["IssuerDomainRegistry"] as? String) ?? "",
            poseidon6: (o["Poseidon6"] as? String) ?? "",
            protocolRegistry: (o["ProtocolRegistry"] as? String) ?? ""
        )
    }
}

/// Endpoint configuration. Per-vet/-verifier hosts always come from scanned QR origins; `roaxRpc`
/// is the gasless on-chain read endpoint.
enum AppConfig {
    static let roaxRpc = "https://devrpc.roax.net"
}
