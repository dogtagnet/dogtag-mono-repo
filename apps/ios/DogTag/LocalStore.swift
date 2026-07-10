import Foundation
import Combine
import UIKit

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

/// Local, on-device pet photos keyed by `dogTagId`. UI-ONLY: a photo is never uploaded, never written
/// on-chain, and never part of a credential — it is purely a local avatar for the Home screen. Images
/// are stored as JPEG under `Documents/pet-photos/<dogTagId>.jpg` and cached in memory.
///
/// Kept deliberately SEPARATE from `Pet` (rather than a field on the struct) because `Pet` records are
/// overwritten by `mergeCentralPets`/`upsertPet`; a photo keyed by the stable `dogTagId` survives those
/// syncs untouched.
///
/// `version` is published and bumped on every change so any view holding this as an `@ObservedObject`
/// re-renders when a photo is set or removed.
final class PetPhotoStore: ObservableObject {
    static let shared = PetPhotoStore()

    @Published private(set) var version = 0

    private let dir: URL
    /// Keyed by the sanitized identifier (see `cacheKey(for:)`) so the memory key and on-disk file name
    /// can never diverge. A present entry with a `nil` value is a cached miss: a dog-tag known to have no
    /// photo, so subsequent lookups skip the disk instead of re-reading an absent file every re-render.
    private var cache: [String: UIImage?] = [:]

    private init() {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        dir = docs.appendingPathComponent("pet-photos", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    /// dogTagIds are decimal token ids, but sanitize defensively so any id is a safe file name. This is
    /// also the in-memory cache key, keeping the memory key and on-disk file name in lockstep.
    private func cacheKey(for dogTagId: String) -> String {
        let safe = String(dogTagId.unicodeScalars.filter { CharacterSet.alphanumerics.contains($0) })
        return safe.isEmpty ? "unknown" : safe
    }

    private func fileURL(for dogTagId: String) -> URL {
        dir.appendingPathComponent("\(cacheKey(for: dogTagId)).jpg")
    }

    /// The stored photo for a dog tag, or `nil`. Reads from disk once — both hits and misses are cached,
    /// so a photoless dog-tag never re-reads the disk on subsequent calls.
    func image(for dogTagId: String) -> UIImage? {
        let key = cacheKey(for: dogTagId)
        if let cached = cache[key] { return cached }
        let img = (try? Data(contentsOf: fileURL(for: dogTagId))).flatMap { UIImage(data: $0) }
        cache.updateValue(img, forKey: key)
        return img
    }

    func hasImage(for dogTagId: String) -> Bool { image(for: dogTagId) != nil }

    /// Store (or replace) the photo for a dog tag. Downscales + JPEG-compresses so a full-resolution
    /// camera shot doesn't sit in Documents at several megabytes.
    func setImage(_ image: UIImage, for dogTagId: String) {
        let resized = image.downscaled(maxDimension: 1024)
        cache.updateValue(resized, forKey: cacheKey(for: dogTagId))
        if let data = resized.jpegData(compressionQuality: 0.85) {
            try? data.write(to: fileURL(for: dogTagId), options: .atomic)
        }
        version &+= 1
    }

    func removeImage(for dogTagId: String) {
        try? FileManager.default.removeItem(at: fileURL(for: dogTagId))
        cache.updateValue(nil, forKey: cacheKey(for: dogTagId))
        version &+= 1
    }
}

private extension UIImage {
    /// Aspect-preserving downscale so the longest edge is at most `maxDimension` points. Upscales are
    /// skipped (a small image is returned as-is).
    func downscaled(maxDimension: CGFloat) -> UIImage {
        let longest = max(size.width, size.height)
        guard longest > maxDimension, longest > 0 else { return self }
        let scale = maxDimension / longest
        let target = CGSize(width: size.width * scale, height: size.height * scale)
        let format = UIGraphicsImageRendererFormat.default()
        format.scale = 1
        return UIGraphicsImageRenderer(size: target, format: format).image { _ in
            draw(in: CGRect(origin: .zero, size: target))
        }
    }
}
