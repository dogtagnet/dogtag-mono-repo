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

/// The live ROAX (chainId 135) deployment addresses, loaded from the bundled `roax.json`.
struct RoaxConfig {
    let chainId: Int
    let dogTagSbt: String
    let verificationRegistry: String
    let consentKeyRegistry: String
    let issuerRegistry: String
    let poseidon6: String

    static func load() -> RoaxConfig {
        guard let url = Bundle.main.url(forResource: "roax", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            return RoaxConfig(chainId: 135, dogTagSbt: "", verificationRegistry: "",
                              consentKeyRegistry: "", issuerRegistry: "", poseidon6: "")
        }
        return RoaxConfig(
            chainId: (o["chainId"] as? Int) ?? 135,
            dogTagSbt: (o["DogTagSBT"] as? String) ?? "",
            verificationRegistry: (o["VerificationRegistry"] as? String) ?? "",
            consentKeyRegistry: (o["ConsentKeyRegistry"] as? String) ?? "",
            issuerRegistry: (o["IssuerRegistry"] as? String) ?? "",
            poseidon6: (o["Poseidon6"] as? String) ?? ""
        )
    }
}

/// Endpoint configuration (mirrors Android AppConfig). Per-vet/-groomer hosts always come from scanned
/// QR origins — the device never calls a central admin base for registration or pet sync. The legacy
/// ECDSA export path still reads an optional central base URL + owner session token (read-only here);
/// `roaxRpc` is the on-chain read endpoint.
enum AppConfig {
    static let defaultCentralApi = "https://api.dogtag.io"
    static let roaxRpc = "https://devrpc.roax.net"

    private static let centralKey = "central_api"
    private static let sessionKey = "owner_session"

    /// The configured central API base URL (no trailing slash), or the compiled-in default.
    static var centralApi: String {
        let v = UserDefaults.standard.string(forKey: centralKey)?
            .trimmingCharacters(in: .whitespaces)
        if let v = v, !v.isEmpty {
            return v.hasSuffix("/") ? String(v.dropLast()) : v
        }
        return defaultCentralApi
    }

    static var sessionToken: String? {
        UserDefaults.standard.string(forKey: sessionKey)
    }
}
