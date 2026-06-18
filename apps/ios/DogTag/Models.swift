import Foundation

/// A credential group as shown on Home (Health / Service / Travel). Mirrors Models.kt.
enum CredentialGroup: String, CaseIterable, Identifiable {
    case health, service, travel
    var id: String { rawValue }
    var title: String {
        switch self {
        case .health: return "Health Records"
        case .service: return "Service Dog"
        case .travel: return "Travel Docs"
        }
    }
}

/// A single credential / record held for a pet.
struct Credential: Identifiable {
    let id: String
    let group: CredentialGroup
    let recordType: String
    let title: String
    let subtitle: String
    let issuer: String
    let issuedOn: String
}

/// The pet card (reference: "Blaze", Goldendoodle).
struct Pet {
    let name: String
    let breed: String
    let ageLabel: String
    let dogTagId: String   // on-chain DogTagSBT tokenId (decimal)
}

/// In-memory seed data mirroring the reference app (identical to Android DemoData).
enum DemoData {
    static let pet = Pet(name: "Blaze", breed: "Goldendoodle", ageLabel: "2 yrs 7 mo", dogTagId: "42")

    static let credentials: [Credential] = [
        Credential(id: "h1", group: .health, recordType: "Vaccine", title: "Rabies (3-yr)", subtitle: "Lot #RB-2291 · valid to 2027", issuer: "Liberalize Vet Clinic", issuedOn: "2024-08-14"),
        Credential(id: "h2", group: .health, recordType: "Vaccine", title: "DHPP Booster", subtitle: "Annual core vaccine", issuer: "Liberalize Vet Clinic", issuedOn: "2024-08-14"),
        Credential(id: "h3", group: .health, recordType: "Checkup / Wellness", title: "Annual Wellness Exam", subtitle: "Healthy · 28.4 kg", issuer: "Liberalize Vet Clinic", issuedOn: "2025-03-02"),
        Credential(id: "h4", group: .health, recordType: "Lab Work", title: "Heartworm Antigen", subtitle: "Negative", issuer: "IDEXX Reference Labs", issuedOn: "2025-03-02"),
        Credential(id: "s1", group: .service, recordType: "DOT Service Dog Form", title: "Service Animal Attestation", subtitle: "DOT Air Transportation Form", issuer: "Owner-attested", issuedOn: "2025-01-10"),
        Credential(id: "t1", group: .travel, recordType: "CDC Dog Import Form", title: "U.S. Entry Receipt", subtitle: "Valid 6 months", issuer: "CDC", issuedOn: "2025-05-20"),
        Credential(id: "t2", group: .travel, recordType: "Microchip", title: "ISO 11784/11785", subtitle: "985112… (15 digit)", issuer: "Liberalize Vet Clinic", issuedOn: "2022-11-03"),
        Credential(id: "t3", group: .travel, recordType: "Rabies Certificate", title: "International Travel", subtitle: "EU/UK accepted", issuer: "Liberalize Vet Clinic", issuedOn: "2024-08-14"),
        Credential(id: "t4", group: .travel, recordType: "Health Certificate", title: "USDA-endorsed", subtitle: "10-day validity", issuer: "USDA APHIS", issuedOn: "2025-05-18"),
    ]

    static func count(for group: CredentialGroup) -> Int {
        credentials.filter { $0.group == group }.count
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
