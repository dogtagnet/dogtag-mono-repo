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

/// Endpoint configuration for the live backends (mirrors Android AppConfig).
enum AppConfig {
    static let centralApi = "https://api.dogtag.io"
    static let roaxRpc = "https://devrpc.roax.net"

    private static let sessionKey = "owner_session"
    static var sessionToken: String? {
        get { UserDefaults.standard.string(forKey: sessionKey) }
        set {
            if let v = newValue { UserDefaults.standard.set(v, forKey: sessionKey) }
            else { UserDefaults.standard.removeObject(forKey: sessionKey) }
        }
    }
}
