import Foundation

/// Device-side per-tag Merkle tree building + the recoverable owner-secret backup (Level-B M5).
///
/// The owner's app builds the tree LOCALLY and hands the issuer only the root `R`, which the issuer
/// seals into `DogTagSBTConsent.profileRoot(dogTagId)` (write-once). The owner's wallet never
/// reaches the chain. Everything else the builder returns - above all the owner-secret, which is the
/// nullifier's secret leaf - is owner-private and must never be transmitted: a server that learned
/// it could link nullifiers back to the owner, which is exactly what Level-B removes.
///
/// The tree math lives in Rust (`crates/dogtag-standard-rs/src/profile_tree.rs`) and is reached
/// through the `buildProfileTreeHex` FFI, so the Poseidon parameter set and the reserved-leaf
/// encoding stay pinned in one place across Rust/TS/circom.
///
/// # Two independent recovery paths (belt-and-suspenders)
///
/// 1. **Seed derivation (primary).** The owner-secret, the consent key and the reserved-leaf salts
///    are all derived from the BIP-39 wallet seed, bound to `dogTagId`. Restoring the seed
///    regenerates them, hence the same `R`. Nothing else is needed but the attribute values.
/// 2. **This local file (backup).** `Documents/dogtag-owner-secrets.json` additionally records the
///    secret, `R`, and the attribute leaves (values + salts) - the attribute salts are NOT
///    seed-derivable, so without them the tree cannot be rebuilt from the seed alone.
///
/// See `docs/MOBILE_OWNER_SECRET.md` for the file's documented contract.
enum ProfileTreeStore {
    /// The documented backup file. Plain JSON in the app's Documents dir.
    static let fileName = "dogtag-owner-secrets.json"

    /// Stamped into every record so a future KDF change is detectable rather than silently
    /// producing a different `R`. Must track `profile_tree::OWNER_SECRET_DOMAIN`.
    static let derivationVersion = "DogTag/owner-secret/v1"

    /// One backed-up attribute leaf. Mirrors `AttributeLeafFfi`.
    struct BackedUpAttribute: Codable, Equatable {
        let keyPath: String
        let saltHex: String
        let tag: UInt8
        let value: String
    }

    /// Everything needed to rebuild one tag's tree - and therefore to regenerate proofs after a
    /// device loss. **Holds a recovery secret** (`ownerSecretHex`); see the file's docs.
    struct OwnerSecretRecord: Codable, Equatable {
        /// Canonical dogTagId field (`dogTagIdFieldHex`), the value the tree is bound to.
        let dogTagIdHex: String
        /// The human-facing decimal id.
        let dogTagIdDec: String
        /// SECRET - the nullifier's secret leaf. Never transmit.
        let ownerSecretHex: String
        /// `R` - the only value the issuer ever sees.
        let rootHex: String
        let ownerAddress: String
        let attributes: [BackedUpAttribute]
        let derivationVersion: String
        let savedAt: Date
    }

    enum StoreError: Error, LocalizedError {
        case rootMismatch(expected: String, got: String)
        case unreadableFile(underlying: Error)

        var errorDescription: String? {
            switch self {
            case let .rootMismatch(expected, got):
                return "rebuilt R \(got) != recorded R \(expected)"
            case let .unreadableFile(underlying):
                return "\(fileName) exists but could not be read; refusing to overwrite it "
                    + "(it holds recovery secrets): \(underlying)"
            }
        }
    }

    private static var fileURL: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(fileName)
    }

    // ---- build ---------------------------------------------------------------------------------

    /// Build the tag's tree on-device from the wallet seed and persist the recovery record.
    ///
    /// Returns the full owner-private witness; hand the issuer ONLY `rootHex`.
    @discardableResult
    static func buildAndPersist(
        seedHex: String,
        dogTagIdDec: String,
        ownerAddress: String,
        attributes: [BackedUpAttribute]
    ) throws -> ProfileTreeFfi {
        let dogTagIdHex = try dogTagIdFieldHex(dogTagIdDec: dogTagIdDec)
        let tree = try buildProfileTreeHex(
            seedHex: seedHex,
            dogTagIdHex: dogTagIdHex,
            ownerAddressHex: ownerAddress,
            attributes: attributes.map {
                AttributeLeafFfi(keyPath: $0.keyPath, saltHex: $0.saltHex, tag: $0.tag, value: $0.value)
            }
        )
        let record = OwnerSecretRecord(
            dogTagIdHex: dogTagIdHex,
            dogTagIdDec: dogTagIdDec,
            ownerSecretHex: tree.ownerSecretHex,
            rootHex: tree.rootHex,
            ownerAddress: ownerAddress,
            attributes: attributes,
            derivationVersion: derivationVersion,
            savedAt: Date()
        )
        try upsert(record)
        return tree
    }

    // ---- recover -------------------------------------------------------------------------------

    /// Rebuild a tag's `R` from the wallet seed + the record's attributes and assert it still equals
    /// the recorded `R` - the recovery round-trip, on-device.
    ///
    /// Throws `StoreError.rootMismatch` if the tree no longer reproduces, which would mean the tag's
    /// proofs can never verify again (`profileRoot` is write-once on-chain, so `R` cannot be moved).
    @discardableResult
    static func verifyRecoverable(seedHex: String, record: OwnerSecretRecord) throws -> String {
        let tree = try buildProfileTreeHex(
            seedHex: seedHex,
            dogTagIdHex: record.dogTagIdHex,
            ownerAddressHex: record.ownerAddress,
            attributes: record.attributes.map {
                AttributeLeafFfi(keyPath: $0.keyPath, saltHex: $0.saltHex, tag: $0.tag, value: $0.value)
            }
        )
        guard tree.rootHex == record.rootHex else {
            throw StoreError.rootMismatch(expected: record.rootHex, got: tree.rootHex)
        }
        return tree.rootHex
    }

    // ---- persistence ---------------------------------------------------------------------------

    /// The only legitimately empty state is "no file yet". An existing-but-unreadable file (corrupt,
    /// written by a newer schema, or simply locked - the file is `.completeFileProtection`) throws
    /// `StoreError.unreadableFile` rather than reporting zero records, so `upsert` cannot rebuild an
    /// empty array over it and wipe every other tag's attribute salts, which are not seed-derivable
    /// and therefore exist nowhere else.
    static func load() throws -> [OwnerSecretRecord] {
        guard FileManager.default.fileExists(atPath: fileURL.path) else { return [] }
        do {
            let data = try Data(contentsOf: fileURL)
            let dec = JSONDecoder()
            dec.dateDecodingStrategy = .iso8601
            return try dec.decode([OwnerSecretRecord].self, from: data)
        } catch {
            throw StoreError.unreadableFile(underlying: error)
        }
    }

    static func all() -> [OwnerSecretRecord] {
        (try? load()) ?? []
    }

    static func record(forDogTagIdDec dec: String) -> OwnerSecretRecord? {
        all().first { $0.dogTagIdDec == dec }
    }

    static func upsert(_ record: OwnerSecretRecord) throws {
        var records = try load()
        if let idx = records.firstIndex(where: { $0.dogTagIdDec == record.dogTagIdDec }) {
            records[idx] = record
        } else {
            records.append(record)
        }
        try write(records)
    }

    private static func write(_ records: [OwnerSecretRecord]) throws {
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        enc.dateEncodingStrategy = .iso8601
        let data = try enc.encode(records)
        // The file holds recovery secrets, so unlike pets.json it is written ATOMICALLY and with
        // complete file protection (encrypted at rest whenever the device is locked). A torn write
        // here would cost the owner a tag's recoverability.
        try data.write(to: fileURL, options: [.atomic, .completeFileProtection])
    }
}
