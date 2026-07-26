import Foundation
import CryptoKit
import CommonCrypto
import LocalAuthentication
import Security

/// The embedded, self-custodial wallet (mirrors Android Wallet.kt).
///
/// Design:
///  - A BIP-39 mnemonic (24 words) → BIP-39 seed. Its secp256k1 public address is used only as one
///    private input to the per-tag owner-control tree; issuance never transmits it or mints to it.
///    The per-tag BabyJubjub consent key and owner secret are derived inside Rust by
///    `buildProfileTreeHex` / `proveConsent`.
///  - The 64-byte BIP-39 seed is stored in the iOS Keychain with
///    `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`; reveal is gated behind a fresh LAContext
///    biometric/passcode authentication. (A Secure Enclave key can wrap the blob on real hardware;
///    the Keychain item itself is hardware-protected and non-exportable — see notes below.)
///  - The 32-byte BIP-39 *entropy* is ALSO persisted (same hardware-protected, this-device-only
///    Keychain protection) so the user can re-view + export their 24-word recovery phrase later for
///    self-custody backup/migration - BIP-39 seed→mnemonic is one-way, so without the entropy the
///    phrase would be unrecoverable after genesis. Export (`revealMnemonic`) is biometric-gated and
///    the phrase is never logged or transmitted. Neither Keychain item is iCloud-synced.
struct WalletIdentity {
    let mnemonic: String          // fresh phrase at genesis; re-derivable later via Wallet.revealMnemonic()
    let ethAddress: String        // private profile input (0x… 20 bytes); never sent at issuance
}

enum Wallet {
    private static let seedAccount = "dogtag_wallet_seed"
    private static let entropyAccount = "dogtag_wallet_entropy"
    private static let keychainService = "io.liberalize.dogtag"

    static func exists() -> Bool {
        blobExists(account: seedAccount)
    }

    /// Whether the 24-word phrase can be re-derived on this device — i.e. whether this is a modern
    /// wallet or a LEGACY, phrase-less one (created before the entropy was persisted).
    ///
    /// Presence-only, so a view can ask on every render without pulling the entropy into memory. A
    /// `false` here is what makes issuance permanently unreachable for that wallet: no phrase means no
    /// honest way to satisfy `SeedBackup`, which `ProfileTreeStore.buildAndPersist` gates on. Offer
    /// `replace()` when this is false.
    static func hasExportablePhrase() -> Bool {
        blobExists(account: entropyAccount)
    }

    /// Create a brand-new wallet: generate a 24-word mnemonic, store the BIP-39 seed (and the 32-byte
    /// entropy, so the phrase stays exportable) in the hardware-backed Keychain, and derive the
    /// secp256k1 address + BabyJubjub consent key.
    static func create() throws -> WalletIdentity {
        var entropy = Data(count: 32) // 256-bit → 24 words
        let rc = entropy.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 32, $0.baseAddress!) }
        guard rc == errSecSuccess else { throw WalletError.randomGenerationFailed }
        let mnemonic = Bip39.entropyToMnemonic(entropy)
        let seed = Bip39.mnemonicToSeed(mnemonic: mnemonic, passphrase: "")
        // Persist the entropy first, then the seed: it re-derives the exact recovery phrase for
        // self-custody export (seed→mnemonic is one-way). Same protection class as the seed; both
        // stay on this device. exists() keys off the seed, so if the seed write fails genesis stays
        // clean-retriable (no live-but-unexportable wallet) and the orphan entropy is overwritten.
        try storeBlob(entropy, account: entropyAccount)
        try storeBlob(seed, account: seedAccount)
        return identity(from: seed, mnemonic: mnemonic)
    }

    /// Replace this wallet with a fresh, phrase-backed one: destroy the existing Keychain material
    /// and the now-meaningless backup confirmation, then run genesis.
    ///
    /// This is the honest way out of a LEGACY wallet - one stored before the entropy was persisted,
    /// whose 24 words `revealMnemonic()` genuinely cannot reconstruct. Such a wallet can never satisfy
    /// `SeedBackup`, because the app must never invite the user to confirm they backed up a phrase it
    /// cannot show them, and `ProfileTreeStore.buildAndPersist` gates issuance on that confirmation.
    /// So the resolution is not to loosen the gate but to give the user a wallet whose phrase they can
    /// actually write down.
    ///
    /// IRREVERSIBLE, and the caller MUST say so first: every dog tag already bound to the outgoing
    /// seed loses its owner-secret. `profileRoot` is write-once on-chain and `proveConsent` re-derives
    /// the owner-secret from the seed on every proof, so those tags can never prove consent again -
    /// re-importing them will not bring them back (D3: the remedy is a fresh issuance under a NEW
    /// `dogTagId`). Offer the private-key export BEFORE calling this.
    static func replace() throws -> WalletIdentity {
        try deleteKeys()
        SeedBackup.clear()
        return try create()
    }

    /// Delete the wallet's Keychain material: the BIP-39 seed AND the entropy that re-derives the
    /// recovery phrase. Idempotent - a missing item is not an error.
    ///
    /// The seed goes FIRST on purpose. `exists()` keys off the seed, so if the entropy delete then
    /// fails the app is already in its first-run state (and the next `create()` overwrites the orphan
    /// entropy). The reverse order could drop the entropy while leaving a live seed - manufacturing
    /// exactly the legacy, phrase-less wallet this type exists to get users out of.
    static func deleteKeys() throws {
        try deleteBlob(account: seedAccount)
        try deleteBlob(account: entropyAccount)
    }

    /// Re-derive the identity from the stored seed. The mnemonic is not returned here; use
    /// `revealMnemonic()` (biometric-gated) to re-derive it for export.
    static func load() throws -> WalletIdentity? {
        guard let seed = loadBlob(account: seedAccount) else { return nil }
        return identity(from: seed, mnemonic: "")
    }

    /// Re-derive the 24-word BIP-39 recovery phrase from the stored entropy, for self-custody
    /// export/backup. Returns nil for wallets created before the entropy was persisted (seed-only),
    /// whose phrase is genuinely unrecoverable on this device. Callers MUST gate this behind a fresh
    /// biometric check and must never log or transmit the result.
    static func revealMnemonic() -> String? {
        guard let entropy = loadBlob(account: entropyAccount) else { return nil }
        return Bip39.entropyToMnemonic(entropy)
    }

    /// Re-derive the raw secp256k1 private key (0x-hex, 32 bytes) from the stored seed, for
    /// self-custody export. This IS the on-chain wallet key: importing it into a standard EVM wallet
    /// reproduces `ethAddress` exactly, whereas the mnemonic alone would derive a DIFFERENT address
    /// there (this wallet uses the raw BIP-32 master key, not `m/44'/60'/0'/0/0`). Available even for
    /// legacy seed-only wallets. Same rules as `revealMnemonic()`: biometric-gate the call, never log
    /// or transmit the result.
    static func revealPrivateKeyHex() -> String? {
        guard let seed = loadBlob(account: seedAccount) else { return nil }
        let priv = Bip39.seedToSecp256k1Priv(seed)
        return "0x" + priv.map { String(format: "%02x", $0) }.joined()
    }

    /// The stored 64-byte BIP-39 seed as `0x…` hex, for the Rust FFI's seed-derived material
    /// (`deriveOwnerSecretHex`, `buildProfileTreeHex`, `proveConsent`).
    ///
    /// This is the root secret of the wallet: keep it in memory only for the duration of a
    /// derivation call, and never log, persist or transmit it. The user's 24-word phrase restores
    /// the owner-control core; rebuilding the same tree and `R` also requires the original owner
    /// address plus the credential's attribute values and salts.
    static func seedHex() -> String? {
        guard let seed = loadBlob(account: seedAccount) else { return nil }
        return "0x" + seed.map { String(format: "%02x", $0) }.joined()
    }

    private static func identity(from seed: Data, mnemonic: String) -> WalletIdentity {
        let priv = Bip39.seedToSecp256k1Priv(seed)              // 32-byte secp256k1 scalar
        let ethAddress = Secp256k1.address(fromPriv: priv)
        return WalletIdentity(mnemonic: mnemonic, ethAddress: ethAddress)
    }

    // ---- Keychain (hardware-protected, this-device-only) -------------------------------------

    private static func storeBlob(_ data: Data, account: String) throws {
        let delete: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(delete as CFDictionary)
        let add: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            kSecValueData as String: data,
        ]
        let status = SecItemAdd(add as CFDictionary, nil)
        if status != errSecSuccess { throw WalletError.keychain(status) }
    }

    /// Presence check that does NOT return the secret: no `kSecReturnData`, so the Keychain answers
    /// from attributes and the seed/entropy never enters this process's memory.
    private static func blobExists(account: String) -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        return SecItemCopyMatching(query as CFDictionary, nil) == errSecSuccess
    }

    private static func deleteBlob(account: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account,
        ]
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw WalletError.keychain(status)
        }
    }

    private static func loadBlob(account: String) -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &out)
        guard status == errSecSuccess else { return nil }
        return out as? Data
    }
}

enum WalletError: Error { case keychain(OSStatus); case randomGenerationFailed }

/// Records that the user has confirmed they wrote down their 24-word recovery phrase.
///
/// Load-bearing for owner-hidden recovery, not a nag: the wallet seed is the ONLY cross-device path to a
/// tag's owner-secret. `Documents/dogtag-owner-secrets.json` is excluded from device backups
/// (`ProfileTreeStore`), and the seed/entropy Keychain items are `…ThisDeviceOnly`, so a replacement
/// phone can regenerate the owner-secret ONLY by restoring the phrase. Minting an owner-secret for a
/// user who has not written the phrase down would mean a lost phone silently and permanently
/// destroys that tag - `profileRoot` is write-once, so there is no on-chain remedy (D3: re-issue a
/// fresh tag). `ProfileTreeStore.buildAndPersist` therefore GATES creation on this flag.
///
/// It records a user ASSERTION, not proof - the app cannot verify a phrase was really written down.
/// The assertion is bound to a one-way fingerprint of the seed so restored `UserDefaults` cannot
/// confirm a newly-created wallet whose `…ThisDeviceOnly` Keychain seed did not migrate. The
/// fingerprint is not a secret, so plain `UserDefaults` is the right home. A missing or mismatched
/// fingerprint re-prompts, which fails safe.
enum SeedBackup {
    private static let fingerprintKey = "seed_backup_fingerprint_v1"
    private static let fingerprintDomain = "DogTag/seed-backup-fingerprint/v1"

    static func isConfirmed(forSeedHex seedHex: String) -> Bool {
        guard let expected = fingerprint(seedHex: seedHex) else { return false }
        return UserDefaults.standard.string(forKey: fingerprintKey) == expected
    }

    /// Call when the user affirms they have stored the phrase offline (the "I've saved it" action).
    @discardableResult
    static func confirm(seedHex: String) -> Bool {
        guard let value = fingerprint(seedHex: seedHex) else { return false }
        UserDefaults.standard.set(value, forKey: fingerprintKey)
        return true
    }

    /// Forget the confirmation, so the gate re-prompts for whatever phrase comes next. Call whenever
    /// the wallet the assertion was bound to is destroyed or replaced.
    ///
    /// Not load-bearing for safety - the fingerprint is seed-bound, so a stale value could never pass
    /// for a different seed - but leaving a confirmation behind for a seed that no longer exists is
    /// stored state that lies about what the user asserted.
    static func clear() {
        UserDefaults.standard.removeObject(forKey: fingerprintKey)
    }

    private static func fingerprint(seedHex: String) -> String? {
        let hex = seedHex.hasPrefix("0x") || seedHex.hasPrefix("0X")
            ? String(seedHex.dropFirst(2))
            : seedHex
        guard !hex.isEmpty, hex.count.isMultiple(of: 2) else { return nil }

        var seed = Data(capacity: hex.count / 2)
        var index = hex.startIndex
        while index < hex.endIndex {
            let next = hex.index(index, offsetBy: 2)
            guard let byte = UInt8(hex[index..<next], radix: 16) else { return nil }
            seed.append(byte)
            index = next
        }

        var material = Data(fingerprintDomain.utf8)
        material.append(seed)
        return SHA256.hash(data: material).map { String(format: "%02x", $0) }.joined()
    }
}

/// AndroidX BiometricPrompt analogue: LAContext-gated authentication.
enum Biometric {
    static func authenticate(reason: String, completion: @escaping (Bool, String?) -> Void) {
        let ctx = LAContext()
        var err: NSError?
        let policy: LAPolicy = .deviceOwnerAuthentication // biometrics OR passcode
        if ctx.canEvaluatePolicy(policy, error: &err) {
            ctx.evaluatePolicy(policy, localizedReason: reason) { ok, e in
                DispatchQueue.main.async { completion(ok, e?.localizedDescription) }
            }
        } else {
            // No biometric/passcode available (e.g. headless sim): proceed so the flow still works.
            DispatchQueue.main.async { completion(true, nil) }
        }
    }
}

/// Minimal BIP-39 (entropy↔mnemonic, mnemonic→seed) over CommonCrypto PBKDF2-HMAC-SHA512.
enum Bip39 {
    static func entropyToMnemonic(_ entropy: Data) -> String {
        let cs = entropy.count * 8 / 32
        let hash = Data(SHA256.hash(data: entropy))
        var bits = ""
        for b in entropy { bits += String(repeating: "0", count: 8 - String(b, radix: 2).count) + String(b, radix: 2) }
        for i in 0..<cs {
            let bit = (hash[i / 8] >> (7 - UInt8(i % 8))) & 1
            bits += bit == 1 ? "1" : "0"
        }
        var words: [String] = []
        var i = 0
        while i < bits.count {
            let start = bits.index(bits.startIndex, offsetBy: i)
            let end = bits.index(start, offsetBy: 11)
            let idx = Int(bits[start..<end], radix: 2)!
            words.append(bip39Wordlist[idx])
            i += 11
        }
        return words.joined(separator: " ")
    }

    static func mnemonicToSeed(mnemonic: String, passphrase: String) -> Data {
        let pw = mnemonic.decomposedStringWithCompatibilityMapping.data(using: .utf8)!
        let salt = ("mnemonic" + passphrase).decomposedStringWithCompatibilityMapping.data(using: .utf8)!
        var derived = Data(count: 64)
        let result = derived.withUnsafeMutableBytes { dOut in
            salt.withUnsafeBytes { sIn in
                pw.withUnsafeBytes { pIn in
                    CCKeyDerivationPBKDF(
                        CCPBKDFAlgorithm(kCCPBKDF2),
                        pIn.bindMemory(to: Int8.self).baseAddress, pw.count,
                        sIn.bindMemory(to: UInt8.self).baseAddress, salt.count,
                        CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA512),
                        2048,
                        dOut.bindMemory(to: UInt8.self).baseAddress, 64
                    )
                }
            }
        }
        precondition(result == kCCSuccess)
        return derived
    }

    /// BIP-32 master: take HMAC-SHA512("Bitcoin seed", seed) left half as the secp256k1 scalar.
    static func seedToSecp256k1Priv(_ seed: Data) -> Data {
        let key = SymmetricKey(data: "Bitcoin seed".data(using: .utf8)!)
        let mac = HMAC<SHA512>.authenticationCode(for: seed, using: key)
        return Data(mac).prefix(32)
    }
}
