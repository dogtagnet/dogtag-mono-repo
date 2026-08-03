package io.liberalize.dogtag.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Pins the pure ProtocolRegistry ABI decoders. These need no FFI and no chain.
 *
 * The iOS mirror is `apps/ios/DogTagTests/AnchorResolverTests.swift`.
 */
class AnchorResolverTest {

    private fun word(hex: String): String {
        val h = hex.removePrefix("0x")
        return h.padStart(64, '0')
    }

    private fun repeatByte(bb: String): String = bb.repeat(32)

    /** A short/misaligned return fails closed (null), never a partial decode. */
    @Test
    fun decodeDiscoverySetFailsClosedOnShortReturn() {
        assertNull(AnchorResolver.decodeDiscoverySet(word("01") + word("02")))
        assertNull(AnchorResolver.decodeDiscoverySet("abc"))
    }

    /**
     * The arity guard is EXACT, and these are the two widths that make it load-bearing rather than
     * pedantic. Both are real record shapes this project has shipped a decoder for:
     *
     *  - EIGHT words is the superseded `getContractSet` tuple.
     *  - TEN words is the superseded discovery record, which carried `rootIndex` as its own member.
     *
     * Under a `>= 9` check either would decode at the wrong indices into a plausible-looking live
     * record - an `active` that is really a block timestamp (truthy), a `circuitId` that is really an
     * address. That is the identical-shape/different-semantics failure this repo has paid for before,
     * and the exact check is what makes it unreachable. This test is what keeps the check exact.
     */
    @Test
    fun aRecordOfAnyOtherWidthFailsClosedRatherThanDecodingAtTheWrongIndices() {
        val eightWords = DISCOVERY_SET_GOLDEN.substring(0, 64 * 8)
        val tenWords = DISCOVERY_SET_GOLDEN + word("01")
        assertNull("an 8-word return must not decode as the 9-word discovery set", AnchorResolver.decodeDiscoverySet(eightWords))
        assertNull("a 10-word return must not decode as the 9-word discovery set", AnchorResolver.decodeDiscoverySet(tenWords))
        // ...and the 9-word record itself does decode, so neither refusal is vacuous.
        assertNotNull(AnchorResolver.decodeDiscoverySet(DISCOVERY_SET_GOLDEN))
    }

    /**
     * The discovery axis and the artifact axis are keyed SEPARATELY and rotate independently, which is
     * the whole point of the two-axis registry: rotating a zkey moves the artifact key and touches no
     * contract address, and rotating the contracts moves the discovery key and touches no artifact.
     * The circuit id belongs to the frozen ceremony and moves with neither.
     */
    @Test
    fun theTwoAxesAreKeyedSeparately() {
        assertEquals("dogtag-levelb/1", AnchorResolver.PROTOCOL_VERSION)
        assertEquals("dogtag-levelb-artifacts/1", AnchorResolver.ARTIFACT_SET)
        assertEquals("consent.circom/DogTagConsent(6)", AnchorResolver.CIRCUIT_ID)
    }

    /**
     * `getActiveArtifactSet` is a DYNAMIC tuple: a leading offset, a 9-word head, then string tails.
     * Decode reads `artifactSetId`(head 0) and `active`(head 8), and FOLLOWS the `minAppVersion`
     * offset (head 6) to its `[length][bytes]` tail — here `"1.4.0"`. `artifactBaseUrl` is ignored.
     */
    @Test
    fun decodeArtifactSet() {
        val minAppData = "312e342e30" + "0".repeat(64 - 10) // "1.4.0" right-padded to a word
        val hex =
            word("20") +               // outer offset → tuple starts at word 1
            word(repeatByte("33")) +   // head 0 artifactSetId
            word("00") +               // head 1 zkeySha
            word("00") +               // head 2 witnessMobileSha
            word("00") +               // head 3 witnessR1csSha
            word("00") +               // head 4 witnessWasmSha
            word("160") +              // head 5 artifactBaseUrl offset (rel 352) — ignored
            word("120") +              // head 6 minAppVersion offset (rel 288 = word 10)
            word("00") +               // head 7 publishedAt
            word("01") +               // head 8 active = true
            word("05") +               // word 10: minAppVersion length = 5
            minAppData                 // word 11: "1.4.0"
        val a = AnchorResolver.decodeArtifactSet(hex)
        assertEquals("0x" + repeatByte("33"), a?.artifactSetId)
        assertEquals("1.4.0", a?.minAppVersion)
        assertEquals(true, a?.active)
    }

    /** An empty `minAppVersion` decodes to "" (not null) — a length-0 string is well-formed. */
    @Test
    fun decodeArtifactSetEmptyMinAppVersion() {
        val hex =
            word("20") +
            word(repeatByte("44")) +   // artifactSetId
            word("00") + word("00") + word("00") + word("00") + // shas
            word("160") +              // artifactBaseUrl offset (ignored)
            word("120") +              // minAppVersion offset → word 10
            word("00") +               // publishedAt
            word("00") +               // active = false
            word("00")                 // word 10: minAppVersion length = 0
        val a = AnchorResolver.decodeArtifactSet(hex)
        assertEquals("", a?.minAppVersion)
        assertEquals(false, a?.active)
    }

    /** A truncated dynamic return (offset points past the data) fails closed. */
    @Test
    fun decodeArtifactSetFailsClosedOnTruncation() {
        val hex = word("20") + word(repeatByte("33")) // offset + one head word, nothing else
        assertNull(AnchorResolver.decodeArtifactSet(hex))
    }

    // ---- GOLDEN vectors from the real ABI encoder (independent of this decoder's author) ----

    /**
     * The EXACT nine-word encoding of a `getDiscoverySet` record, one word per member of the static
     * tuple: `[discoverySetId, factory, verificationRegistry, sbt, verifier, providerRegistry,
     * circuitId, publishedAt, active]`.
     *
     * Produced by the REAL Solidity ABI encoder, in
     * `contracts/test/ProtocolRegistry.t.sol::test_the_golden_encoding_the_mobile_decoders_are_pinned_against`,
     * which asserts these same bytes from the contract end. That shared literal is what makes the three
     * files one contract rather than three independent guesses - and they had already drifted apart
     * once, when the Solidity end was made synthetic and this pair was left holding a live chain
     * capture, so the comment claiming they were pinned together was false for a while.
     *
     * THE ADDRESSES ARE SYNTHETIC, DELIBERATELY. What this vector pins is the record's ARITY and
     * MEMBER ORDER, which is independent of which addresses the members hold; using the live deployed
     * ones bought nothing and put real addresses in the tree, which is what `make check-addresses`
     * exists to refuse. They are distinct per member so a member-order regression cannot pass by two
     * slots holding the same value.
     *
     * Never hand-edit this hex: a hand-built vector is written from the same understanding as the
     * decoder, so a shared mistake passes both. Regenerate by running that Solidity test and reading
     * the actual bytes out of its failure message, then paste them into all three files.
     */
    private val DISCOVERY_SET_GOLDEN =
        "36a8d69d16a9f540fa11be5f0311ebd5efd8e971b66cd704a6e197ee15b01b3d" + // 0 discoverySetId
        "0000000000000000000000001c9ac2eb3f1a2d4b5c6d7e8f90a1b2c3d4e5f607" + // 1 factory
        "0000000000000000000000002b4d6f8a0c1e3a5b7d9f0e2c4a6b8d0f1e3a5c70" + // 2 verificationRegistry
        "0000000000000000000000003c5e7a9b0d2f4a6c8e0b1d3f5a7c9e0b2d4f6a80" + // 3 sbt (skipped)
        "0000000000000000000000004d6f8b0c2e4a6b8d0f2c4e6a8b0d2f4c6e8a0b90" + // 4 verifier (skipped)
        "0000000000000000000000009309ab1c2d3e4f5061728394a5b6c7d8e9f00112" + // 5 providerRegistry
        "a708f8e240d9734e5f054f55fa891a37c31f536a5de28874439572018c9aa54f" + // 6 circuitId
        "000000000000000000000000000000000000000000000000000000006b4c7500" + // 7 publishedAt (skipped)
        "0000000000000000000000000000000000000000000000000000000000000001"   // 8 active = true

    /**
     * The live record, decoded. `discoverySetId` is `keccak256("dogtag-levelb/1")` and `circuitId` is
     * `keccak256("consent.circom/DogTagConsent(6)")`, so both are checkable against the constants above
     * rather than only against themselves.
     */
    @Test
    fun decodeDiscoverySetGolden() {
        val d = AnchorResolver.decodeDiscoverySet(DISCOVERY_SET_GOLDEN)
        assertNotNull(d)
        assertEquals("0x36a8d69d16a9f540fa11be5f0311ebd5efd8e971b66cd704a6e197ee15b01b3d", d?.discoverySetId)
        assertEquals("0x1c9ac2eb3f1a2d4b5c6d7e8f90a1b2c3d4e5f607", d?.factory)
        assertEquals("0x2b4d6f8a0c1e3a5b7d9f0e2c4a6b8d0f1e3a5c70", d?.verificationRegistry)
        assertEquals("0x9309ab1c2d3e4f5061728394a5b6c7d8e9f00112", d?.providerRegistry)
        assertEquals("0xa708f8e240d9734e5f054f55fa891a37c31f536a5de28874439572018c9aa54f", d?.circuitId)
        assertEquals(true, d?.active)
    }

    /**
     * `rootIndex` IS `factory`, and it is a derived accessor rather than a tenth word.
     *
     * There is one launch set and no earlier generation to bridge, so there is nothing for a separate
     * provenance index to resolve through: `VerificationRegistryConsent` pins the factory in its
     * immutable `rootIndex` slot, and the publish preflight refuses to stage a record whose `factory`
     * is not that address - so the equality is asserted on chain rather than assumed on the device.
     * A superseded design carried `rootIndex` as its own member at word 6, which is why the record was
     * ten words wide and why a decoder built for it reads `circuitId` out of an address slot.
     */
    @Test
    fun theRootIndexIsTheFactory() {
        val d = AnchorResolver.decodeDiscoverySet(DISCOVERY_SET_GOLDEN)
        assertNotNull(d)
        assertEquals(d?.factory, d?.rootIndex)
        assertEquals("0x1c9ac2eb3f1a2d4b5c6d7e8f90a1b2c3d4e5f607", d?.rootIndex)
    }

    /**
     * The EXACT bytes `getActiveArtifactSet` returns for the levelb set — where `artifactBaseUrl`
     * precedes `minAppVersion` in the string tail (offsets `0x120` then `0x180`), the OPPOSITE order
     * from the hand vector above. The decoder must still pull `minAppVersion == "1.4.0"` by FOLLOWING
     * head-word 6's offset, not by assuming a tail position. Load-bearing: `minAppVersion` feeds an
     * uncross-checked `appVersion >= minAppVersion` gate, so a decode error here would fail OPEN.
     */
    @Test
    fun decodeArtifactSetGolden() {
        val hex =
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
        val a = AnchorResolver.decodeArtifactSet(hex)
        assertEquals("1.4.0", a?.minAppVersion)
        assertEquals(true, a?.active)
        // The graph pin is PINNED on chain as of 2026-07-28, so head-word 2 carries the real
        // `witnessMobileSha256` here rather than 0. Nothing asserts it: `AnchorResolver` decodes only
        // `artifactSetId`, `minAppVersion` and `active`, which is exactly why pinning it changed no app
        // behaviour. It is carried at full width so the head-word OFFSETS this test exists to pin stay
        // faithful to what `getActiveArtifactSet` now returns.
        // The artifactSetId is keccak(dogtag-levelb-artifacts/1) - UNCHANGED by the in-place re-publish,
        // which is what kept already-shipped apps resolvable.
        assertEquals("0xe28963a343070ded2096ce5abf4596f17a74bc8a813da8266cd8032a57fe6938", a?.artifactSetId)
    }
}
