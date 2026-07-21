import XCTest

/// Pins the pure Level-B discovery-anchor logic: the `mode == "levelb"` gate and the two
/// `ProtocolRegistry` ABI decoders. These need no FFI and no chain, so they run in the host-less
/// bundle. The actual fail-closed comparison is `validateDiscovery` (Rust-tested in `dogtag_standard`
/// / #55) and is not re-tested here; this covers the caller's two jobs — gate + decode.
///
/// The Kotlin mirror is `apps/android/.../net/AnchorResolverTest.kt`.
final class AnchorResolverTests: XCTestCase {

    // MARK: - The gate (the acceptance bar: non-levelb ⇒ Level-B logic never runs)

    /// Only `"levelb"` (any case) is Level-B. Every other mode the server can stamp — `normal`, `zk`,
    /// and anything unexpected — is NOT, so the caller makes ZERO ProtocolRegistry calls and stays on
    /// the Level-A path. This is the load-bearing "Level-A untouched" guarantee, at its source.
    func testIsLevelBGate() {
        XCTAssertTrue(AnchorResolver.isLevelB(mode: "levelb"))
        XCTAssertTrue(AnchorResolver.isLevelB(mode: "LEVELB"))
        XCTAssertTrue(AnchorResolver.isLevelB(mode: "LevelB"))
        for notLevelB in ["zk", "normal", "ecdsa", "level-b", "levelb ", "", "consent"] {
            XCTAssertFalse(AnchorResolver.isLevelB(mode: notLevelB), "\(notLevelB) must NOT be Level-B")
        }
    }

    // MARK: - ABI decoders

    private func word(_ hex: String) -> String {
        let h = hex.hasPrefix("0x") ? String(hex.dropFirst(2)) : hex
        return String(repeating: "0", count: 64 - h.count) + h
    }
    private func repeatByte(_ bb: String) -> String { String(repeating: bb, count: 32) }

    /// `getContractSet` is a STATIC 8-word tuple. Decode extracts `contractSetId`(0),
    /// `verificationRegistry`(2), `circuitId`(5), `active`(7) — and nothing from the skipped slots.
    func testDecodeContractSet() {
        let reg = "b9b313c17fd8725bb50a7f41121ac4cf5f4fec87"
        let hex =
            word(repeatByte("11")) +   // 0 contractSetId
            word("00") +               // 1 factory (skipped)
            word(reg) +                // 2 verificationRegistry
            word("00") +               // 3 sbt (skipped)
            word("00") +               // 4 verifier (skipped)
            word(repeatByte("22")) +   // 5 circuitId
            word("00") +               // 6 publishedAt (skipped)
            word("01")                 // 7 active = true
        let cs = AnchorResolver.decodeContractSet(hex)
        XCTAssertNotNil(cs)
        XCTAssertEqual(cs?.contractSetId, "0x" + repeatByte("11"))
        XCTAssertEqual(cs?.verificationRegistry, "0x" + reg)
        XCTAssertEqual(cs?.circuitId, "0x" + repeatByte("22"))
        XCTAssertEqual(cs?.active, true)
    }

    /// A short/misaligned return fails closed (nil), never a partial decode.
    func testDecodeContractSetFailsClosedOnShortReturn() {
        XCTAssertNil(AnchorResolver.decodeContractSet(word("01") + word("02"))) // only 2 words
        XCTAssertNil(AnchorResolver.decodeContractSet("abc")) // not a whole word
    }

    /// `getActiveArtifactSet` is a DYNAMIC tuple: a leading offset, a 9-word head, then string tails.
    /// Decode reads `artifactSetId`(head 0) and `active`(head 8), and FOLLOWS the `minAppVersion`
    /// offset (head 6) to its `[length][bytes]` tail — here `"1.4.0"`. `artifactBaseUrl` is ignored.
    func testDecodeArtifactSet() {
        // "1.4.0" = 0x312e342e30 (5 bytes), right-padded to a word.
        let minAppData = "312e342e30" + String(repeating: "0", count: 64 - 10)
        let hex =
            word("20") +               // outer offset → tuple starts at word 1
            word(repeatByte("33")) +   // head 0 artifactSetId
            word("00") +               // head 1 zkeySha
            word("00") +               // head 2 witnessMobileSha
            word("00") +               // head 3 witnessR1csSha
            word("00") +               // head 4 witnessWasmSha
            word("160") +              // head 5 artifactBaseUrl offset (rel 352 = word 12) — ignored
            word("120") +              // head 6 minAppVersion offset (rel 288 = word 10)
            word("00") +               // head 7 publishedAt
            word("01") +               // head 8 active = true
            word("05") +               // word 10: minAppVersion length = 5
            minAppData                 // word 11: "1.4.0"
        let a = AnchorResolver.decodeArtifactSet(hex)
        XCTAssertNotNil(a)
        XCTAssertEqual(a?.artifactSetId, "0x" + repeatByte("33"))
        XCTAssertEqual(a?.minAppVersion, "1.4.0")
        XCTAssertEqual(a?.active, true)
    }

    /// An empty `minAppVersion` decodes to "" (not nil) — a length-0 string is well-formed.
    func testDecodeArtifactSetEmptyMinAppVersion() {
        let hex =
            word("20") +
            word(repeatByte("44")) +   // artifactSetId
            word("00") + word("00") + word("00") + word("00") + // shas
            word("160") +              // artifactBaseUrl offset (ignored)
            word("120") +              // minAppVersion offset → word 10
            word("00") +               // publishedAt
            word("00") +               // active = false
            word("00")                 // word 10: minAppVersion length = 0
        let a = AnchorResolver.decodeArtifactSet(hex)
        XCTAssertEqual(a?.minAppVersion, "")
        XCTAssertEqual(a?.active, false)
    }

    /// A truncated dynamic return (offset points past the data) fails closed.
    func testDecodeArtifactSetFailsClosedOnTruncation() {
        let hex = word("20") + word(repeatByte("33")) // offset + one head word, nothing else
        XCTAssertNil(AnchorResolver.decodeArtifactSet(hex))
    }

    // MARK: - GOLDEN vectors from Solidity's real ABI encoder (independent of this decoder's author)

    /// The EXACT bytes `ProtocolRegistry.getContractSet(keccak("dogtag-levelb/1"))` returns, captured
    /// from `abi.encode(reg.getContractSet(bId))` in a Foundry run against the real `ProtocolVersions`
    /// levelb set. This catches an ABI-model error the hand-built vectors above cannot: they were
    /// written from the same understanding as the decoder, so a shared mistake would pass both.
    func testDecodeContractSetGolden() {
        let hex =
            "36a8d69d16a9f540fa11be5f0311ebd5efd8e971b66cd704a6e197ee15b01b3d" +
            "000000000000000000000000d3179abbfb0274d0a5f7017d76015a93c159511d" +
            "000000000000000000000000b9b313c17fd8725bb50a7f41121ac4cf5f4fec87" +
            "00000000000000000000000096cba4580d79bc9b8e51fc1b3a044a29592afffc" +
            "000000000000000000000000272be146c0aed6401000e9aa8241201f6f0fdf1a" +
            "a708f8e240d9734e5f054f55fa891a37c31f536a5de28874439572018c9aa54f" +
            "000000000000000000000000000000000000000000000000000000000002a301" +
            "0000000000000000000000000000000000000000000000000000000000000001"
        let cs = AnchorResolver.decodeContractSet(hex)
        // The canonical M5 Level-B trio (ProtocolVersions.sol levelBContracts).
        XCTAssertEqual(cs?.verificationRegistry, "0xb9b313c17fd8725bb50a7f41121ac4cf5f4fec87")
        XCTAssertEqual(cs?.active, true)
    }

    /// The EXACT bytes `getActiveArtifactSet` returns for the levelb set — where `artifactBaseUrl`
    /// precedes `minAppVersion` in the string tail (offsets `0x120` then `0x180`), the OPPOSITE order
    /// from the hand vector above. The decoder must still pull `minAppVersion == "1.4.0"` by FOLLOWING
    /// head-word 6's offset, not by assuming a tail position. This is the load-bearing golden check:
    /// `minAppVersion` feeds an uncross-checked `appVersion >= minAppVersion` gate, so a decode error
    /// here would fail OPEN (a too-old build slips the anti-downgrade check).
    func testDecodeArtifactSetGolden() {
        let hex =
            "0000000000000000000000000000000000000000000000000000000000000020" + // outer offset
            "e28963a343070ded2096ce5abf4596f17a74bc8a813da8266cd8032a57fe6938" + // artifactSetId
            "f83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868" + // zkeySha256
            "0000000000000000000000000000000000000000000000000000000000000000" + // witnessMobileSha (0)
            "828e2923a159b04f2de421d4b447f8c85356677f4f83a5af55b42eb2b4f9b6b7" + // witnessR1csSha
            "482debcff5a4325c008dd00e4476bba011d0a706da955e3129d114f996a913e6" + // witnessWasmSha
            "0000000000000000000000000000000000000000000000000000000000000120" + // artifactBaseUrl off (288)
            "0000000000000000000000000000000000000000000000000000000000000180" + // minAppVersion off (384)
            "0000000000000000000000000000000000000000000000000000000000054601" + // publishedAt
            "0000000000000000000000000000000000000000000000000000000000000001" + // active = true
            "0000000000000000000000000000000000000000000000000000000000000023" + // artifactBaseUrl len (35)
            "68747470733a2f2f6172746966616374732e646f677461672e696f2f6c657665" + // "https://artifacts.dogtag.io/leve
            "6c62310000000000000000000000000000000000000000000000000000000000" + // lb1" + padding
            "0000000000000000000000000000000000000000000000000000000000000005" + // minAppVersion len (5)
            "312e342e30000000000000000000000000000000000000000000000000000000"   // "1.4.0" + padding
        let a = AnchorResolver.decodeArtifactSet(hex)
        XCTAssertEqual(a?.minAppVersion, "1.4.0", "must follow the head-6 offset to the SECOND string tail")
        XCTAssertEqual(a?.active, true)
        // witnessMobileSha256 is published 0/unpinned; the artifactSetId is keccak(dogtag-levelb-artifacts/1).
        XCTAssertEqual(a?.artifactSetId, "0xe28963a343070ded2096ce5abf4596f17a74bc8a813da8266cd8032a57fe6938")
    }
}
