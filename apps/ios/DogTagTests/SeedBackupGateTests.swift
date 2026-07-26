import XCTest

/// The seed-backup gate that the legacy-wallet dead end turns on.
///
/// `ProfileTreeStore.buildAndPersist` refuses to mint an owner-secret unless `SeedBackup.isConfirmed`,
/// and the only honest way to satisfy that is a phrase the app can actually show the user. These cover
/// the gate's own correctness: that a confirmation is bound to the seed it was made for, that
/// `clear()` forgets it, and that malformed input fails closed rather than confirming something.
///
/// # What is deliberately NOT here
///
/// `Wallet.create/deleteKeys/replace` touch the Keychain, and this bundle has no host application by
/// design (see `project.yml`: it must stay runnable on a plain checkout where the xcframework is
/// absent). A host-less bundle has no keychain-access-group entitlement, so those calls return
/// `errSecMissingEntitlement (-34018)` here regardless of whether the logic is right. Giving this
/// target a host app to work around that would trade a documented invariant for coverage, so the
/// Keychain lifecycle is verified in the running app instead - see `apps/ios/AGENTS.md`.
final class SeedBackupGateTests: XCTestCase {
    /// A 64-byte seed's worth of hex, distinct per test so one test's fingerprint cannot satisfy
    /// another's assertion.
    private func seedHex(byte: UInt8) -> String {
        "0x" + String(repeating: String(format: "%02x", byte), count: 64)
    }

    override func setUp() {
        super.setUp()
        SeedBackup.clear()
    }

    override func tearDown() {
        SeedBackup.clear()
        super.tearDown()
    }

    func testUnconfirmedByDefault() {
        XCTAssertFalse(SeedBackup.isConfirmed(forSeedHex: seedHex(byte: 0x11)))
    }

    func testConfirmThenIsConfirmed() {
        let seed = seedHex(byte: 0x22)
        XCTAssertTrue(SeedBackup.confirm(seedHex: seed))
        XCTAssertTrue(SeedBackup.isConfirmed(forSeedHex: seed))
    }

    /// The property that makes clearing the confirmation hygiene rather than the safety mechanism: a
    /// leftover confirmation cannot vouch for a different wallet's seed.
    func testConfirmationIsBoundToItsSeed() {
        XCTAssertTrue(SeedBackup.confirm(seedHex: seedHex(byte: 0x33)))
        XCTAssertFalse(SeedBackup.isConfirmed(forSeedHex: seedHex(byte: 0x44)),
                       "a confirmation for one seed must not satisfy the gate for another")
    }

    /// What `Wallet.replace()`/`AppReset.deleteWallet()` rely on: the gate re-prompts for whatever
    /// phrase comes next.
    func testClearForgetsTheConfirmation() {
        let seed = seedHex(byte: 0x55)
        XCTAssertTrue(SeedBackup.confirm(seedHex: seed))
        SeedBackup.clear()
        XCTAssertFalse(SeedBackup.isConfirmed(forSeedHex: seed))
    }

    func testClearIsIdempotent() {
        SeedBackup.clear()
        SeedBackup.clear()
        XCTAssertFalse(SeedBackup.isConfirmed(forSeedHex: seedHex(byte: 0x66)))
    }

    /// Fail closed: unparseable input must not record a confirmation, and must not report one either.
    func testMalformedSeedHexNeitherConfirmsNorPasses() {
        for bad in ["", "0x", "0xabc", "0xzz", "zz", "abc"] {
            XCTAssertFalse(SeedBackup.confirm(seedHex: bad), "confirm should reject \(bad.debugDescription)")
            XCTAssertFalse(SeedBackup.isConfirmed(forSeedHex: bad),
                           "isConfirmed should reject \(bad.debugDescription)")
        }
    }

    /// A malformed call must not clobber a good confirmation already on record.
    func testMalformedConfirmDoesNotDisturbAnExistingConfirmation() {
        let seed = seedHex(byte: 0x77)
        XCTAssertTrue(SeedBackup.confirm(seedHex: seed))
        XCTAssertFalse(SeedBackup.confirm(seedHex: "0xzz"))
        XCTAssertTrue(SeedBackup.isConfirmed(forSeedHex: seed))
    }

    /// The `0x` prefix is cosmetic - the same seed must fingerprint identically either way, so a
    /// caller's formatting cannot silently re-prompt a user who already confirmed.
    func testPrefixedAndBareHexAreTheSameSeed() {
        let bare = String(repeating: "88", count: 64)
        XCTAssertTrue(SeedBackup.confirm(seedHex: "0x" + bare))
        XCTAssertTrue(SeedBackup.isConfirmed(forSeedHex: bare))
        XCTAssertTrue(SeedBackup.isConfirmed(forSeedHex: "0X" + bare))
    }

    func testHexIsCaseInsensitiveForTheSameSeed() {
        let lower = String(repeating: "ab", count: 64)
        XCTAssertTrue(SeedBackup.confirm(seedHex: "0x" + lower))
        XCTAssertTrue(SeedBackup.isConfirmed(forSeedHex: "0x" + lower.uppercased()))
    }
}

/// BIP-39 derivation the wallet's phrase export depends on: `revealMnemonic()` reconstructs the 24
/// words from the persisted entropy, so entropy->mnemonic->seed has to be exactly reproducible or a
/// user's written-down phrase would restore a different wallet.
final class Bip39DerivationTests: XCTestCase {
    /// BIP-39 test vector: all-zero 256-bit entropy.
    func testAllZeroEntropyMatchesTheKnownVector() {
        let mnemonic = Bip39.entropyToMnemonic(Data(repeating: 0, count: 32))
        XCTAssertEqual(
            mnemonic,
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon "
                + "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon "
                + "abandon abandon abandon art")
    }

    /// The published BIP-39 vector for that phrase, which uses the passphrase "TREZOR". Asserting the
    /// spec's own vector is what pins the PBKDF2 salt ("mnemonic" + passphrase), the 2048 iterations
    /// and the NFKD normalization - a drift in any of them would silently derive a different wallet
    /// from a correctly written-down phrase.
    func testKnownVectorSeedWithTheSpecPassphrase() {
        let mnemonic = Bip39.entropyToMnemonic(Data(repeating: 0, count: 32))
        let seed = Bip39.mnemonicToSeed(mnemonic: mnemonic, passphrase: "TREZOR")
        XCTAssertEqual(
            seed.map { String(format: "%02x", $0) }.joined(),
            "bda85446c68413707090a52022edd26a1c9462295029f2e60cd7c4f2bbd3097170af7a4d73245cafa9c3cca8d561a7c3de6f5d4a10be8ed2a5e608d68f92fcc8")
        XCTAssertEqual(seed.count, 64)
    }

    /// The empty passphrase is what `Wallet` actually uses, so pin that seed too.
    func testEmptyPassphraseSeedForTheKnownVectorPhrase() {
        let mnemonic = Bip39.entropyToMnemonic(Data(repeating: 0, count: 32))
        let seed = Bip39.mnemonicToSeed(mnemonic: mnemonic, passphrase: "")
        XCTAssertEqual(
            seed.map { String(format: "%02x", $0) }.joined(),
            "408b285c123836004f4b8842c89324c1f01382450c0d439af345ba7fc49acf705489c6fc77dbd4e3dc1dd8cc6bc9f043db8ada1e243c4a0eafb290d399480840")
    }

    /// A passphrase must change the seed - proof the salt really includes it.
    func testPassphraseChangesTheSeed() {
        let mnemonic = Bip39.entropyToMnemonic(Data(repeating: 0, count: 32))
        XCTAssertNotEqual(
            Bip39.mnemonicToSeed(mnemonic: mnemonic, passphrase: ""),
            Bip39.mnemonicToSeed(mnemonic: mnemonic, passphrase: "TREZOR"))
    }

    func testEntropyToMnemonicIsDeterministicAnd24Words() {
        let entropy = Data((0..<32).map { UInt8($0) })
        let first = Bip39.entropyToMnemonic(entropy)
        XCTAssertEqual(first, Bip39.entropyToMnemonic(entropy))
        XCTAssertEqual(first.split(separator: " ").count, 24)
    }

    /// A one-bit change must produce a different phrase - otherwise distinct wallets could share one.
    func testDifferentEntropyGivesADifferentPhrase() {
        var other = Data(repeating: 0, count: 32)
        other[31] = 1
        XCTAssertNotEqual(
            Bip39.entropyToMnemonic(Data(repeating: 0, count: 32)),
            Bip39.entropyToMnemonic(other))
    }

    func testEveryWordIsFromTheWordlist() {
        let entropy = Data((0..<32).map { UInt8(($0 * 7) % 256) })
        let words = Set(bip39Wordlist)
        for word in Bip39.entropyToMnemonic(entropy).split(separator: " ") {
            XCTAssertTrue(words.contains(String(word)), "\(word) is not a BIP-39 word")
        }
    }

    func testWordlistIsTheStandard2048() {
        XCTAssertEqual(bip39Wordlist.count, 2048)
    }
}
