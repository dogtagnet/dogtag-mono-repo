package io.liberalize.dogtag.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

/**
 * Guards the on-chain credential-validity read.
 *
 * The mobile `isValid` selector MUST equal the deployed `DogTagIssuer`'s, i.e.
 * `keccak256("isValid(bytes32)")[:4] = 0x6a938567` - the exact selector viem, the Rust/alloy ABI, the
 * vet-api `verify_credential` handler and the web direct-RPC path all bind (verified on-chain against
 * the ROAX VACCINATION clone `0x5c703910111f942EE0f47E02214291b5274cDb53`: a real issued root returns
 * `0x…01`). It was once hard-coded to `0x6d04f0bc`, which REVERTS on the deployed clone - so every
 * read fell through to `Unknown` (accept-with-caveat) and a revoked credential never showed as
 * revoked. `RoaxRpc.functionSelector` now derives it from the signature; these tests pin that.
 */
class RoaxRpcSelectorTest {
    @Test
    fun isValidSelectorMatchesCanonicalSignature() {
        assertEquals("0x6a938567", RoaxRpc.functionSelector("isValid(bytes32)"))
    }

    @Test
    fun isValidSelectorIsNotTheStaleRevertingConstant() {
        assertNotEquals("0x6d04f0bc", RoaxRpc.functionSelector("isValid(bytes32)"))
    }

    @Test
    fun derivationReproducesKnownSiblingSelectors() {
        // Sanity: the same derivation reproduces other selectors RoaxRpc already relies on (all
        // independently confirmed via keccak256 and on-chain), so the keccak → slice → hex pipeline
        // is correct, not just coincidentally right for `isValid`.
        assertEquals("0x4294857f", RoaxRpc.functionSelector("isRevoked(bytes32)"))
        assertEquals("0x6240dded", RoaxRpc.functionSelector("issuedAt(bytes32)"))
    }

    /**
     * Pins every selector `RoaxRpc` derives. These were the last hard-coded literals in the client;
     * each expected value here is the one that shipped as a constant, independently reconfirmed with
     * `cast sig` before the constants were removed, so this test proves the switch to derivation was
     * value-preserving and keeps the signatures honest if anyone edits them.
     */
    @Test
    fun everyDerivedSelectorMatchesItsCanonicalSignature() {
        assertEquals("0x6a938567", RoaxRpc.functionSelector("isValid(bytes32)"))
        assertEquals("0x779c3985", RoaxRpc.functionSelector("isWhitelistedFor(bytes32,address)"))
        assertEquals("0x4648c943", RoaxRpc.functionSelector("consumed(bytes32)"))
        assertEquals("0x85105cb3", RoaxRpc.functionSelector("profileRoot(uint256)"))
    }

    /**
     * The issuer↔domain chain's selectors, including `rootIssuer` — the read that decides WHICH contract
     * a credential's identity is resolved from. A wrong selector there reverts, which the client reports
     * as "could not read"; that fails closed, but it silently disables the defence against a swapped
     * `issuer.documentStore`. Confirmed with `cast sig`.
     */
    @Test
    fun theIssuerBindingSelectorsMatchTheirCanonicalSignatures() {
        assertEquals("0x41e41d17", RoaxRpc.functionSelector("rootIssuer(bytes32)"))
        assertEquals("0x00ae3676", RoaxRpc.functionSelector("isClone(address)"))
        assertEquals("0xe0095e0e", RoaxRpc.functionSelector("domainOf(address)"))
        assertEquals("0x06fdde03", RoaxRpc.functionSelector("name()"))
    }

    /**
     * The grant-history reads the issuer-whitelist pillar now folds.
     *
     * A LOG TOPIC is the FULL 32-byte keccak of the event signature, not a 4-byte call selector, and
     * that difference fails in the worst possible direction: a topic derived with the wrong width
     * matches no log at all, which is indistinguishable from "this pair was never granted" and would
     * turn every genuine credential into a definite refusal. So the two derivations are separate
     * functions and both are pinned. Values independently confirmed with `cast keccak`.
     */
    @Test
    fun theGrantHistoryTopicsMatchTheirCanonicalEventSignatures() {
        assertEquals(
            "0xf8cd30a628b432a1200caf81085096c82a5f570da14360572b72d4e0ba57e6d7",
            RoaxRpc.eventTopic("RootIssued(bytes32,address,uint256)"),
        )
        // The authority's issuance-grant topic. Pinned because the failure mode is SILENT: a value
        // derived at the wrong width, or from a drifted signature, matches no log at all - which
        // reads exactly like "this signer was never granted" and refuses every genuine credential.
        // Confirmed with `cast keccak "IssuanceCapabilitySet(address,address,bool)"`.
        assertEquals(
            "0x831abb96b1c02fe346a944062a9367343ef9d09be41d65818b796cd1a8676941",
            RoaxRpc.ISSUANCE_CAPABILITY_SET_TOPIC,
        )
        // The registry the CLONE names - the only authority whose grant log answers for it.
        assertEquals("0x7b103999", RoaxRpc.functionSelector("registry()"))

    }

    /**
     * A topic is 32 bytes and a selector is 4. Stated as its own assertion because the failure mode of
     * confusing them is silent: the shorter value simply matches nothing.
     */
    @Test
    fun anEventTopicIsTheWholeHashNotTheFourByteSelector() {
        val sig = "Whitelisted(bytes32,address)"
        assertEquals(66, RoaxRpc.eventTopic(sig).length)
        assertEquals(10, RoaxRpc.functionSelector(sig).length)
        assertNotEquals(RoaxRpc.eventTopic(sig), RoaxRpc.functionSelector(sig))
        assertEquals(RoaxRpc.functionSelector(sig), RoaxRpc.eventTopic(sig).take(10))
    }
}
