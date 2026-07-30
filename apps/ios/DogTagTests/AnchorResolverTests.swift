import XCTest

/// Pins the pure ProtocolRegistry ABI decoders. These need no FFI and no chain, so they run in the
/// host-less bundle. The actual fail-closed comparison is Rust-tested in `dogtag_standard`.
///
/// The Kotlin mirror is `apps/android/.../net/AnchorResolverTest.kt`.
final class AnchorResolverTests: XCTestCase {

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
    private let genOneGolden =
        "36a8d69d16a9f540fa11be5f0311ebd5efd8e971b66cd704a6e197ee15b01b3d" +
        "000000000000000000000000d3179abbfb0274d0a5f7017d76015a93c159511d" +
        "000000000000000000000000b9b313c17fd8725bb50a7f41121ac4cf5f4fec87" +
        "00000000000000000000000096cba4580d79bc9b8e51fc1b3a044a29592afffc" +
        "000000000000000000000000272be146c0aed6401000e9aa8241201f6f0fdf1a" +
        "a708f8e240d9734e5f054f55fa891a37c31f536a5de28874439572018c9aa54f" +
        "000000000000000000000000000000000000000000000000000000000002a301" +
        "0000000000000000000000000000000000000000000000000000000000000001"

    /// The EXACT bytes `ProtocolRegistryV2.getDiscoverySet(keccak("dogtag-levelb/2"))` returns, captured
    /// from `abi.encode(...)` in `contracts/test/ProtocolRegistryV2.t.sol`, which pins the SAME literal
    /// from the Solidity end. A change to the record's shape or member order fails there first, naming
    /// this file to regenerate — never hand-edit this hex, which would test an idea of the ABI rather than
    /// the ABI. The Kotlin mirror carries the identical bytes.
    private let genTwoGolden =
        "44a8d618d62ba7874b6df187bc27a0c147d0523c8cda11f7764d343d032ae0b5" + // 0 discoverySetId
        "0000000000000000000000001c9ac2eb3f1a2d4b5c6d7e8f90a1b2c3d4e5f607" + // 1 factory (skipped)
        "000000000000000000000000b9b313c17fd8725bb50a7f41121ac4cf5f4fec87" + // 2 verificationRegistry
        "00000000000000000000000096cba4580d79bc9b8e51fc1b3a044a29592afffc" + // 3 sbt (skipped)
        "000000000000000000000000272be146c0aed6401000e9aa8241201f6f0fdf1a" + // 4 verifier (skipped)
        "0000000000000000000000009309ab1c2d3e4f5061728394a5b6c7d8e9f00112" + // 5 providerRegistry
        "000000000000000000000000120127e4a5b6c7d8e9f001122334455667788990" + // 6 rootIndex (router)
        "a708f8e240d9734e5f054f55fa891a37c31f536a5de28874439572018c9aa54f" + // 7 circuitId
        "000000000000000000000000000000000000000000000000000000006b4c7500" + // 8 publishedAt (skipped)
        "0000000000000000000000000000000000000000000000000000000000000001"   // 9 active = true

    func testDecodeContractSetGolden() {
        let cs = AnchorResolver.decodeContractSet(genOneGolden)
        // The canonical owner-hidden trio (kept at its deployed compatibility key).
        XCTAssertEqual(cs?.verificationRegistry, "0xb9b313c17fd8725bb50a7f41121ac4cf5f4fec87")
        XCTAssertEqual(cs?.active, true)
    }

    /// The generation-2 golden vector, decoded. The two members that only exist here are the point:
    /// `rootIndex` is what `rootIssuer`/`isClone` must be read from, and it is a DIFFERENT address from
    /// `factory` (word 1) — reading the factory instead resolves only generation-2 roots and silently
    /// misses every earlier one, which is exactly what the provenance router exists to prevent.
    ///
    /// `circuitId` is byte-identical to the generation-1 golden's, because generation 2 rotates no proving
    /// artifact.
    func testDecodeDiscoverySetGolden() {
        let d = AnchorResolver.decodeDiscoverySet(genTwoGolden)
        XCTAssertNotNil(d)
        XCTAssertEqual(d?.discoverySetId, "0x44a8d618d62ba7874b6df187bc27a0c147d0523c8cda11f7764d343d032ae0b5")
        XCTAssertEqual(d?.verificationRegistry, "0xb9b313c17fd8725bb50a7f41121ac4cf5f4fec87")
        XCTAssertEqual(d?.providerRegistry, "0x9309ab1c2d3e4f5061728394a5b6c7d8e9f00112")
        XCTAssertEqual(d?.rootIndex, "0x120127e4a5b6c7d8e9f001122334455667788990")
        XCTAssertEqual(d?.circuitId, "0xa708f8e240d9734e5f054f55fa891a37c31f536a5de28874439572018c9aa54f")
        XCTAssertEqual(d?.active, true)
        // The root index is NOT the factory, and the decoder must not have read word 1 for it.
        XCTAssertNotEqual(d?.rootIndex, "0x1c9ac2eb3f1a2d4b5c6d7e8f90a1b2c3d4e5f607")
        // The frozen circuit is unchanged across the generation boundary.
        XCTAssertEqual(d?.circuitId, AnchorResolver.decodeContractSet(genOneGolden)?.circuitId)
    }

    /// The generation-2 record (10 words) must not decode through the generation-1 decoder, and vice
    /// versa. Both directions matter and both used to be reachable: the generation-1 decoder accepted
    /// `>= 8` words, so a `getDiscoverySet` return would have decoded at generation-1 indices — reading
    /// `providerRegistry` as `circuitId` and `publishedAt` as `active`, i.e. an `active` that is a block
    /// timestamp (truthy) and a circuit id that is an address.
    ///
    /// That is a plausible-looking live record for a shape this build cannot read, which is the
    /// identical-shape/different-semantics failure this repo has paid for before. The exact-arity check is
    /// what makes it impossible; this test is what keeps the check.
    func testTheTwoGenerationsRecordsCannotDecodeAsEachOther() {
        XCTAssertNil(
            AnchorResolver.decodeContractSet(genTwoGolden),
            "a 10-word generation-2 return must not decode as a generation-1 contract set")
        XCTAssertNil(
            AnchorResolver.decodeDiscoverySet(genOneGolden),
            "an 8-word generation-1 return must not decode as a generation-2 discovery set")
        // ...and each decodes its own, so neither refusal is vacuous.
        XCTAssertEqual(AnchorResolver.decodeContractSet(genOneGolden)?.active, true)
        XCTAssertEqual(AnchorResolver.decodeDiscoverySet(genTwoGolden)?.active, true)
    }

    /// A short/misaligned generation-2 return fails closed too.
    func testDecodeDiscoverySetFailsClosedOnShortReturn() {
        XCTAssertNil(AnchorResolver.decodeDiscoverySet(word("01") + word("02")))
        XCTAssertNil(AnchorResolver.decodeDiscoverySet("abc"))
    }

    /// The generation-2 discovery key and the artifact key move INDEPENDENTLY, which is the point of the
    /// two axes: generation 2 rotates addresses and rotates no proving artifact, so its artifact identity
    /// and circuit id are generation 1's unchanged.
    func testTheGenerationTwoKeyBumpsWhileTheArtifactKeyDoesNot() {
        XCTAssertEqual(AnchorResolver.protocolVersionV2, "dogtag-levelb/2")
        XCTAssertEqual(AnchorResolver.artifactSet, "dogtag-levelb-artifacts/1")
        XCTAssertEqual(AnchorResolver.circuitId, "consent.circom/DogTagConsent(6)")
        XCTAssertEqual(AnchorResolver.protocolVersion, "dogtag-levelb/1")
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
            "2f74d26b800230400639e92211d80ff453bf82c2057b788fa1350e009748f793" + // witnessMobileSha (pinned)
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
        // The graph pin is PINNED on chain as of 2026-07-28, so head-word 2 carries the real
        // `witnessMobileSha256` here rather than 0. Nothing asserts it: `AnchorResolver` decodes only
        // `artifactSetId`, `minAppVersion` and `active`, which is exactly why pinning it changed no app
        // behaviour. It is carried at full width so the head-word OFFSETS this test exists to pin stay
        // faithful to what `getActiveArtifactSet` now returns.
        // The artifactSetId is keccak(dogtag-levelb-artifacts/1) - UNCHANGED by the in-place re-publish,
        // which is what kept already-shipped apps resolvable.
        XCTAssertEqual(a?.artifactSetId, "0xe28963a343070ded2096ce5abf4596f17a74bc8a813da8266cd8032a57fe6938")
    }
}
