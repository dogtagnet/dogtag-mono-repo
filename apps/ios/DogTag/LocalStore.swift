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

    static let petsFileName = "pets.json"
    static let credentialsFileName = "credentials.json"

    private let dir: URL
    private let petsURL: URL
    private let credsURL: URL

    private init() {
        dir = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        petsURL = dir.appendingPathComponent(Self.petsFileName)
        credsURL = dir.appendingPathComponent(Self.credentialsFileName)
        pets = load([Pet].self, from: petsURL) ?? []
        credentials = load([Credential].self, from: credsURL) ?? []
    }

    // ---- destructive reset ---------------------------------------------------------------------

    /// Drop every locally-held pet/dog-tag row and its file.
    ///
    /// The published list is cleared ONLY on a complete sweep: if the file survived, it is still the
    /// truth, and showing an empty UI over it would misreport what was destroyed. Call on the main
    /// thread - this mutates published state the UI observes.
    func deleteAllPets() -> LocalDataSweep.Outcome {
        let outcome = LocalDataSweep.remove(from: dir, files: [Self.petsFileName])
        if outcome.isComplete { pets = [] }
        return outcome
    }

    /// Drop every imported credential/record and its file. Same main-thread and partial-failure
    /// contract as `deleteAllPets`.
    ///
    /// Records are re-obtainable in a way owner-secrets are not: the credential was issued by the
    /// vet and stays on-chain, so it can be imported again from a fresh vet QR.
    func deleteAllCredentials() -> LocalDataSweep.Outcome {
        let outcome = LocalDataSweep.remove(from: dir, files: [Self.credentialsFileName])
        if outcome.isComplete { credentials = [] }
        return outcome
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

    /// Replace a stored credential in place, keyed by id. Used by the on-chain refresh to write back
    /// a re-read verdict. A no-op if the record was deleted while the refresh was in flight.
    func updateCredential(_ cred: Credential) {
        guard let idx = credentials.firstIndex(where: { $0.id == cred.id }) else { return }
        credentials[idx] = cred
        save(credentials, to: credsURL)
    }

    /// Remove ONE credential from THIS device. Local only: it does not revoke anything on-chain and
    /// does not touch the issuer's copy of the record. Removes a single entry even in the impossible
    /// case of a store holding duplicate ids, so a delete can never take a sibling with it.
    func deleteCredential(id: String) {
        guard let idx = credentials.firstIndex(where: { $0.id == id }) else { return }
        credentials.remove(at: idx)
        save(credentials, to: credsURL)
    }

    // ---- pet display name ----------------------------------------------------------------------

    /// The best display name for the pet owning `dogTagId`:
    ///   1. the synced `Pet` name (skipping placeholder fallbacks),
    ///   2. else the name baked into the pet's DOG_PROFILE credential (`credentialSubject.name`),
    ///   3. else nil — callers then show the dogTagId alone (never a bare "Dog Profile").
    func petDisplayName(forDogTagId id: String) -> String? {
        if let n = pet(for: id)?.name, Self.isRealName(n) { return n }
        for c in credentials where c.dogTagId == id && c.recordType.uppercased().contains("DOG_PROFILE") {
            if let name = WrappedDoc(json: c.wrappedDocJson)?.petName, Self.isRealName(name) { return name }
        }
        return nil
    }

    /// Convenience: the display name for the pet owning `cred`.
    func petDisplayName(for cred: Credential) -> String? { petDisplayName(forDogTagId: cred.dogTagId) }

    /// A pet name is "real" only if it isn't empty or one of the auto placeholders ("Unnamed" from a
    /// central sync, "DogTag #…" from the on-import fallback).
    private static func isRealName(_ n: String) -> Bool {
        let t = n.trimmingCharacters(in: .whitespaces)
        return !t.isEmpty && t != "Unnamed" && !t.hasPrefix("DogTag #") && !t.hasPrefix("DogTag#")
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
    static let directoryName = "pet-photos"

    @Published private(set) var version = 0

    private let dir: URL
    /// Keyed by the sanitized identifier (see `cacheKey(for:)`) so the memory key and on-disk file name
    /// can never diverge. A present entry with a `nil` value is a cached miss: a dog-tag known to have no
    /// photo, so subsequent lookups skip the disk instead of re-reading an absent file every re-render.
    private var cache: [String: UIImage?] = [:]

    private init() {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        dir = docs.appendingPathComponent(Self.directoryName, isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    /// Drop every stored pet photo. UI-only material: nothing here was ever uploaded, written
    /// on-chain, or part of a credential, so this is the least consequential of the reset actions.
    ///
    /// The directory is recreated empty so `setImage` keeps working without an app relaunch, and the
    /// memory cache is dropped only on a complete sweep - a cache cleared over surviving files would
    /// re-populate itself from them on the next read and contradict the reported outcome.
    func removeAll() -> LocalDataSweep.Outcome {
        let outcome = LocalDataSweep.remove(
            from: dir.deletingLastPathComponent(), directories: [Self.directoryName])
        guard outcome.isComplete else { return outcome }
        cache.removeAll()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        version &+= 1
        return outcome
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

/// Shared state for on-chain status refreshes, so every surface that shows a verdict (Home,
/// Documents, Travel, the credential detail) reports the SAME three distinguishable states for a
/// given record: checking now, checked (with the answer and its age), or could-not-check (with why).
///
/// The outcome itself is persisted on the credential; this only holds what is in flight.
@MainActor
final class RefreshCenter: ObservableObject {
    static let shared = RefreshCenter()

    /// Credential ids with a check currently running.
    @Published private(set) var inFlight: Set<String> = []
    /// How many records are left in a running "refresh all" (0 when no batch is running).
    @Published private(set) var batchRemaining: Int = 0

    private var roax: RoaxConfig?

    private init() {}

    func isChecking(_ cred: Credential) -> Bool { inFlight.contains(cred.id) }
    var isBatchRunning: Bool { batchRemaining > 0 }

    /// Re-read one credential's on-chain status and persist the result.
    func refresh(_ cred: Credential) async {
        guard !inFlight.contains(cred.id) else { return }
        inFlight.insert(cred.id)
        defer { inFlight.remove(cred.id) }

        let config = resolvedRoax()
        // The integrity pillar is a synchronous FFI call, so the whole check runs off the main actor
        // to keep the list scrolling while it works.
        let updated = await Task.detached(priority: .userInitiated) {
            await CredentialRefresher.refreshed(cred, roax: config)
        }.value
        LocalStore.shared.updateCredential(updated)
    }

    /// Re-read a whole list, one record at a time so the RPC is not hammered and the remaining count
    /// stays meaningful. A second call while a batch is running is ignored.
    func refreshAll(_ creds: [Credential]) async {
        guard batchRemaining == 0, !creds.isEmpty else { return }
        batchRemaining = creds.count
        defer { batchRemaining = 0 }
        for cred in creds {
            await refresh(cred)
            batchRemaining -= 1
        }
    }

    private func resolvedRoax() -> RoaxConfig {
        if let r = roax { return r }
        let r = RoaxConfig.load()
        roax = r
        return r
    }
}
