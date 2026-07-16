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
        assertEquals("0x15c95be6", RoaxRpc.functionSelector("bindNonce(address)"))
        assertEquals("0xfa073d76", RoaxRpc.functionSelector("keyOf(address)"))
        assertEquals("0x4648c943", RoaxRpc.functionSelector("consumed(bytes32)"))
        assertEquals("0x85105cb3", RoaxRpc.functionSelector("profileRoot(uint256)"))
        assertEquals("0x6352211e", RoaxRpc.functionSelector("ownerOf(uint256)"))
    }
}
