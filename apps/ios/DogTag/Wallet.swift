import Foundation
import CryptoKit
import CommonCrypto
import LocalAuthentication
import Security

/// The embedded, self-custodial wallet (mirrors Android Wallet.kt).
///
/// Design:
///  - A BIP-39 mnemonic (24 words) → BIP-39 seed → secp256k1 private key (the user's on-chain
///    `userWallet`). The BabyJubjub *consent* key is derived in Rust from the same seed under a
///    DISTINCT domain (so the two keys never collide), via the new EdDSA FFI `deriveBabyjubConsentKey`.
///  - The 64-byte BIP-39 seed is stored in the iOS Keychain with
///    `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`; reveal is gated behind a fresh LAContext
///    biometric/passcode authentication. (A Secure Enclave key can wrap the blob on real hardware;
///    the Keychain item itself is hardware-protected and non-exportable — see notes below.)
///  - The 32-byte BIP-39 *entropy* is ALSO persisted (same hardware-protected, this-device-only
///    Keychain protection) so the user can re-view + export their 24-word recovery phrase later for
///    self-custody backup/migration - BIP-39 seed→mnemonic is one-way, so without the entropy the
///    phrase would be unrecoverable after genesis. Export (`revealMnemonic`) is biometric-gated and
///    the phrase is never logged or transmitted. Neither Keychain item is iCloud-synced.
///  - keyHash = Poseidon(Ax,Ay) is what gets bound on-chain in ConsentKeyRegistry.
struct WalletIdentity {
    let mnemonic: String          // fresh phrase at genesis; re-derivable later via Wallet.revealMnemonic()
    let ethAddress: String        // secp256k1 userWallet (0x… 20 bytes)
    let consent: BabyjubConsentKeyFfi
    let secpPriv: Data            // 32-byte secp256k1 scalar (in-memory only, post-unlock)

    /// Sign a 32-byte EIP-712 digest with the secp256k1 wallet key → 65-byte 0x.. r||s||v.
    func signEthDigest(_ digest: Data) -> String { Secp256k1.signDigest(priv: secpPriv, digest: digest) }

    /// The EIP-191 `personal_sign` signature over the central registration message
    /// "DogTag wallet registration: <ethAddress lowercased>". The digest is
    /// keccak256(0x19 + "Ethereum Signed Message:\n" + len + message), signed with the secp256k1
    /// wallet key → 65-byte 0x.. r||s||v (recovers to ethAddress server-side).
    func registerSignature() -> String {
        let message = "DogTag wallet registration: \(ethAddress.lowercased())"
        let msgBytes = Data(message.utf8)
        var prefixed = Data([0x19])
        prefixed.append(Data("Ethereum Signed Message:\n\(msgBytes.count)".utf8))
        prefixed.append(msgBytes)
        let digest = Keccak256.digest(prefixed)
        return signEthDigest(digest)
    }
}

enum Wallet {
    private static let seedAccount = "dogtag_wallet_seed"
    private static let entropyAccount = "dogtag_wallet_entropy"
    private static let keychainService = "io.liberalize.dogtag"

    static func exists() -> Bool {
        loadBlob(account: seedAccount) != nil
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
        return try identity(from: seed, mnemonic: mnemonic)
    }

    /// Re-derive the identity from the stored seed. The mnemonic is not returned here; use
    /// `revealMnemonic()` (biometric-gated) to re-derive it for export.
    static func load() throws -> WalletIdentity? {
        guard let seed = loadBlob(account: seedAccount) else { return nil }
        return try identity(from: seed, mnemonic: "")
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
    /// (`deriveBabyjubConsentKey`, `deriveOwnerSecretHex`, `buildProfileTreeHex`).
    ///
    /// This is the root secret of the wallet: keep it in memory only for the duration of a
    /// derivation call, and never log, persist or transmit it. It stays derivable from the user's
    /// 24-word phrase, which is what makes a restored wallet rebuild the same tree and the same `R`.
    static func seedHex() -> String? {
        guard let seed = loadBlob(account: seedAccount) else { return nil }
        return "0x" + seed.map { String(format: "%02x", $0) }.joined()
    }

    private static func identity(from seed: Data, mnemonic: String) throws -> WalletIdentity {
        let priv = Bip39.seedToSecp256k1Priv(seed)              // 32-byte secp256k1 scalar
        let ethAddress = Secp256k1.address(fromPriv: priv)
        let seedHex = "0x" + seed.map { String(format: "%02x", $0) }.joined()
        let consent = try deriveBabyjubConsentKey(seedHex: seedHex)
        // Belt-and-suspenders: keyHash must equal Poseidon(Ax,Ay).
        let kh = try keyHashHex(axHex: consent.axHex, ayHex: consent.ayHex)
        precondition(kh == consent.keyHashHex, "consent keyHash mismatch")
        return WalletIdentity(mnemonic: mnemonic, ethAddress: ethAddress, consent: consent, secpPriv: priv)
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
/// Load-bearing for Level-B recovery, not a nag: the wallet seed is the ONLY cross-device path to a
/// tag's owner-secret. `Documents/dogtag-owner-secrets.json` is excluded from device backups
/// (`ProfileTreeStore`), and the seed/entropy Keychain items are `…ThisDeviceOnly`, so a replacement
/// phone can regenerate the owner-secret ONLY by restoring the phrase. Minting an owner-secret for a
/// user who has not written the phrase down would mean a lost phone silently and permanently
/// destroys that tag - `profileRoot` is write-once, so there is no on-chain remedy (D3: re-issue a
/// fresh tag). `ProfileTreeStore.buildAndPersist` therefore GATES creation on this flag.
///
/// It records a user ASSERTION, not proof - the app cannot verify a phrase was really written down.
/// It is not a secret, so plain `UserDefaults` (where the app keeps its other non-secret prefs) is
/// the right home rather than the Keychain. If it is ever lost the gate simply re-prompts, which
/// fails safe in the direction that matters.
enum SeedBackup {
    private static let confirmedKey = "seed_backup_confirmed_v1"

    static var isConfirmed: Bool { UserDefaults.standard.bool(forKey: confirmedKey) }

    /// Call when the user affirms they have stored the phrase offline (the "I've saved it" action).
    static func confirm() { UserDefaults.standard.set(true, forKey: confirmedKey) }
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
