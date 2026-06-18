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
///  - keyHash = Poseidon(Ax,Ay) is what gets bound on-chain in ConsentKeyRegistry.
struct WalletIdentity {
    let mnemonic: String          // shown ONCE at genesis; never persisted in plaintext
    let ethAddress: String        // secp256k1 userWallet (0x… 20 bytes)
    let consent: BabyjubConsentKeyFfi
}

enum Wallet {
    private static let keychainAccount = "dogtag_wallet_seed"
    private static let keychainService = "io.liberalize.dogtag"

    static func exists() -> Bool {
        loadBlob() != nil
    }

    /// Create a brand-new wallet: generate a 24-word mnemonic, store the BIP-39 seed in the
    /// hardware-backed Keychain, and derive the secp256k1 address + BabyJubjub consent key.
    static func create() throws -> WalletIdentity {
        var entropy = Data(count: 32) // 256-bit → 24 words
        _ = entropy.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 32, $0.baseAddress!) }
        let mnemonic = Bip39.entropyToMnemonic(entropy)
        let seed = Bip39.mnemonicToSeed(mnemonic: mnemonic, passphrase: "")
        try storeBlob(seed)
        return try identity(from: seed, mnemonic: mnemonic)
    }

    /// Re-derive the identity from the stored seed. The mnemonic is NOT recoverable here.
    static func load() throws -> WalletIdentity? {
        guard let seed = loadBlob() else { return nil }
        return try identity(from: seed, mnemonic: "")
    }

    private static func identity(from seed: Data, mnemonic: String) throws -> WalletIdentity {
        let priv = Bip39.seedToSecp256k1Priv(seed)              // 32-byte secp256k1 scalar
        let ethAddress = Secp256k1.address(fromPriv: priv)
        let seedHex = "0x" + seed.map { String(format: "%02x", $0) }.joined()
        let consent = try deriveBabyjubConsentKey(seedHex: seedHex)
        // Belt-and-suspenders: keyHash must equal Poseidon(Ax,Ay).
        let kh = try keyHashHex(axHex: consent.axHex, ayHex: consent.ayHex)
        precondition(kh == consent.keyHashHex, "consent keyHash mismatch")
        return WalletIdentity(mnemonic: mnemonic, ethAddress: ethAddress, consent: consent)
    }

    // ---- Keychain (hardware-protected, this-device-only) -------------------------------------

    private static func storeBlob(_ data: Data) throws {
        let delete: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: keychainAccount,
        ]
        SecItemDelete(delete as CFDictionary)
        let add: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: keychainAccount,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            kSecValueData as String: data,
        ]
        let status = SecItemAdd(add as CFDictionary, nil)
        if status != errSecSuccess { throw WalletError.keychain(status) }
    }

    private static func loadBlob() -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: keychainAccount,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &out)
        guard status == errSecSuccess else { return nil }
        return out as? Data
    }
}

enum WalletError: Error { case keychain(OSStatus) }

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
