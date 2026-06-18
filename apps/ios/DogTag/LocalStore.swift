import Foundation
import Combine

/// A tiny local persistence for pets + imported credentials (mirrors Android LocalStore). Backed by
/// two JSON files in the app's Documents dir. NO sample/mock data — both lists are legitimately empty
/// until the user imports a record (scans a vet QR) or a central pet sync runs.
///
/// Pets are keyed by `dogTagId`; credentials reference their pet by `dogTagId`, so the Travel/Documents
/// filters select by pet.
final class LocalStore: ObservableObject {
    static let shared = LocalStore()

    @Published private(set) var pets: [Pet] = []
    @Published private(set) var credentials: [Credential] = []

    private let petsURL: URL
    private let credsURL: URL

    private init() {
        let dir = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        petsURL = dir.appendingPathComponent("pets.json")
        credsURL = dir.appendingPathComponent("credentials.json")
        pets = load([Pet].self, from: petsURL) ?? []
        credentials = load([Credential].self, from: credsURL) ?? []
    }

    // ---- pets ----------------------------------------------------------------------------------

    func upsertPet(_ pet: Pet) {
        if let idx = pets.firstIndex(where: { $0.dogTagId == pet.dogTagId }) { pets[idx] = pet }
        else { pets.append(pet) }
        save(pets, to: petsURL)
    }

    /// Merge a batch of pets from the central backend without clobbering local edits.
    func mergeCentralPets(_ incoming: [Pet]) {
        for p in incoming where !p.dogTagId.isEmpty {
            if let idx = pets.firstIndex(where: { $0.dogTagId == p.dogTagId }) { pets[idx] = p }
            else { pets.append(p) }
        }
        save(pets, to: petsURL)
    }

    func pet(for dogTagId: String) -> Pet? { pets.first { $0.dogTagId == dogTagId } }

    // ---- credentials ---------------------------------------------------------------------------

    func addCredential(_ cred: Credential) {
        if pet(for: cred.dogTagId) == nil && !cred.dogTagId.isEmpty {
            upsertPet(Pet(dogTagId: cred.dogTagId, name: "DogTag #\(cred.dogTagId)", breed: "", ageLabel: "", microchip: nil))
        }
        if let idx = credentials.firstIndex(where: { $0.id == cred.id }) { credentials[idx] = cred }
        else { credentials.append(cred) }
        save(credentials, to: credsURL)
    }

    func credentials(for dogTagId: String?) -> [Credential] {
        guard let id = dogTagId else { return credentials }
        return credentials.filter { $0.dogTagId == id }
    }

    // ---- IO ------------------------------------------------------------------------------------

    private func load<T: Decodable>(_ type: T.Type, from url: URL) -> T? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(T.self, from: data)
    }

    private func save<T: Encodable>(_ value: T, to url: URL) {
        if let data = try? JSONEncoder().encode(value) { try? data.write(to: url) }
    }
}
