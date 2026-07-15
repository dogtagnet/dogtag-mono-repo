import Foundation

/// Device-side per-tag Merkle tree building + the recoverable owner-secret's device-local store
/// (Level-B M5).
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
/// # Recovery: the seed, plus the credential - never this file
///
/// `Documents/dogtag-owner-secrets.json` is a DEVICE-LOCAL store, not a cross-device backup: it is
/// excluded from device backups (`isExcludedFromBackup`) and written with `.completeFileProtection`,
/// deliberately at parity with the seed/entropy Keychain items' `…ThisDeviceOnly` class. It never
/// leaves the device, so it cannot carry a tag to a replacement one.
///
/// Cross-device recovery therefore rests on two inputs, both required:
///
/// 1. **The wallet seed (the 24-word phrase).** Regenerates the owner-control core - the
///    owner-secret, the consent key, and the reserved-leaf salts - bound to `dogTagId`. It is the
///    ONLY cross-device path to the owner-secret, so the phrase must be backed up.
/// 2. **The credential's attribute leaves.** Values AND salts are caller-supplied and are NOT
///    seed-derivable; they come back from the issued wrapped credential, which packs each leaf as
///    `"<saltHex>:<tag>:<value>"`. The seed alone does not rebuild the tree.
///
/// See `docs/MOBILE_OWNER_SECRET.md` for the file's documented contract.
enum ProfileTreeStore {
    /// The documented device-local store (NOT a backup - see above). Plain JSON in the app's
    /// Documents dir.
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
        case conflictingRoot(dogTagId: String, existing: String, proposed: String)
        case unreadableFile(underlying: Error)
        case seedBackupNotConfirmed

        var errorDescription: String? {
            switch self {
            case let .rootMismatch(expected, got):
                return "rebuilt R \(got) != recorded R \(expected)"
            case let .conflictingRoot(dogTagId, existing, proposed):
                return "dogTagId \(dogTagId) already has root \(existing); refusing replacement with \(proposed)"
            case let .unreadableFile(underlying):
                return "\(fileName) exists but could not be read; refusing to overwrite it "
                    + "(it holds recovery secrets): \(underlying)"
            case .seedBackupNotConfirmed:
                return "the wallet recovery phrase has not been confirmed as backed up; refusing to "
                    + "create an owner-secret that a lost phone would destroy permanently"
            }
        }
    }

    private static var fileURL: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(fileName)
    }

    private static let storeLock = NSLock()

    private static func withStoreLock<T>(_ body: () throws -> T) rethrows -> T {
        storeLock.lock()
        defer { storeLock.unlock() }
        return try body()
    }

    // ---- build ---------------------------------------------------------------------------------

    /// Build the tag's tree on-device from the wallet seed and persist the recovery record.
    ///
    /// Returns the full owner-private witness; hand the issuer ONLY `rootHex`.
    ///
    /// Throws `StoreError.seedBackupNotConfirmed` unless the user has confirmed they backed up their
    /// recovery phrase ([`SeedBackup`]). That gate is the point: this store is excluded from device
    /// backups, so the phrase is the ONLY thing that can regenerate this owner-secret on a
    /// replacement phone. Creating one without it would let a lost phone permanently destroy the tag
    /// with no warning and no on-chain remedy (`profileRoot` is write-once). Call
    /// `SeedBackup.confirm(seedHex:)` from the phrase-backup UX first.
    @discardableResult
    static func buildAndPersist(
        seedHex: String,
        dogTagIdDec: String,
        ownerAddress: String,
        attributes: [BackedUpAttribute]
    ) throws -> ProfileTreeFfi {
        guard SeedBackup.isConfirmed(forSeedHex: seedHex) else {
            throw StoreError.seedBackupNotConfirmed
        }
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
        try withStoreLock { try loadUnlocked() }
    }

    private static func loadUnlocked() throws -> [OwnerSecretRecord] {
        let url = fileURL
        guard FileManager.default.fileExists(atPath: url.path) else { return [] }
        do {
            let data = try Data(contentsOf: url)
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
        try withStoreLock {
            var records = try loadUnlocked()
            if let idx = records.firstIndex(where: {
                $0.dogTagIdHex.caseInsensitiveCompare(record.dogTagIdHex) == .orderedSame
            }) {
                let existing = records[idx]
                guard existing.rootHex.caseInsensitiveCompare(record.rootHex) == .orderedSame else {
                    throw StoreError.conflictingRoot(
                        dogTagId: record.dogTagIdDec,
                        existing: existing.rootHex,
                        proposed: record.rootHex
                    )
                }
                records[idx] = record
            } else {
                records.append(record)
            }
            try write(records)
        }
    }

    private static func write(_ records: [OwnerSecretRecord]) throws {
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        enc.dateEncodingStrategy = .iso8601
        let data = try enc.encode(records)
        let manager = FileManager.default
        let destinationURL = fileURL
        let directoryURL = destinationURL.deletingLastPathComponent()
        let token = UUID().uuidString
        let stagingURL = directoryURL.appendingPathComponent(".\(fileName).\(token).tmp")
        let backupName = ".\(fileName).\(token).bak"
        let backupURL = directoryURL.appendingPathComponent(backupName)
        var replacedExisting = false

        defer {
            if manager.fileExists(atPath: stagingURL.path) {
                try? manager.removeItem(at: stagingURL)
            }
        }

        try Data().write(to: stagingURL, options: [.completeFileProtection])
        try excludeFromBackup(stagingURL)
        let handle = try FileHandle(forWritingTo: stagingURL)
        do {
            try handle.write(contentsOf: data)
            try handle.synchronize()
            try handle.close()
        } catch {
            try? handle.close()
            throw error
        }
        try excludeFromBackup(stagingURL)

        if manager.fileExists(atPath: destinationURL.path) {
            _ = try manager.replaceItemAt(
                destinationURL,
                withItemAt: stagingURL,
                backupItemName: backupName,
                options: [.usingNewMetadataOnly, .withoutDeletingBackupItem]
            )
            replacedExisting = true
        } else {
            try manager.moveItem(at: stagingURL, to: destinationURL)
        }

        do {
            try excludeFromBackup(destinationURL)
            if replacedExisting {
                try manager.removeItem(at: backupURL)
            }
        } catch {
            let commitError = error
            if replacedExisting, manager.fileExists(atPath: backupURL.path) {
                if manager.fileExists(atPath: destinationURL.path) {
                    _ = try manager.replaceItemAt(
                        destinationURL,
                        withItemAt: backupURL,
                        options: [.usingNewMetadataOnly]
                    )
                } else {
                    try manager.moveItem(at: backupURL, to: destinationURL)
                }
                try excludeFromBackup(destinationURL)
            } else if manager.fileExists(atPath: destinationURL.path) {
                try manager.removeItem(at: destinationURL)
            }
            throw commitError
        }
    }

    private static func excludeFromBackup(_ fileURL: URL) throws {
        var url = fileURL
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try url.setResourceValues(values)
    }
}
