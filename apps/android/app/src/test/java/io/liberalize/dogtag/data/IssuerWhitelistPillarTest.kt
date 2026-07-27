package io.liberalize.dogtag.data

import io.liberalize.dogtag.net.RoaxRpc
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The import-time ISSUER-WHITELIST pillar.
 *
 * AUDIT FINDING (2026-07-27): a credential relabelled to a different issuing authority still verified.
 * The `issuer` block sits OUTSIDE the Merkle root, so `name`, `domain` and — the sharp one —
 * `documentStore` are attacker-controlled: integrity recomputes fine and `isValid` still returns true
 * when `documentStore` points at a contract the attacker controls. The whitelist pillar is the only
 * check that catches it, and on this path it was never run at all.
 *
 * The two halves pinned here are the two ways it could silently stop catching anything: the on-chain
 * reads addressing the wrong function, and an unresolved pillar being folded in as a pass.
 */
class IssuerWhitelistPillarTest {

    /**
     * `issuedBy(bytes32)` is what makes the pillar self-resolving — it answers "who actually called
     * `issue(root)` here?" without an operator typing an address. A drifted selector would revert and
     * turn every pillar into an unresolved one, so pin it: `cast sig "issuedBy(bytes32)"`.
     */
    @Test
    fun issuedBySelectorMatchesTheCanonicalSignature() {
        assertEquals("0xe0d272c0", RoaxRpc.functionSelector("issuedBy(bytes32)"))
    }

    /**
     * The whitelist key is `keccak256(recordType)` — the same value the clone's own `recordType()`
     * holds, the backend's `record_type_key` computes, and the web's `recordTypeKey` derives.
     * Confirmed on chain 135: `DogTagIssuer(0xB5D6654d…5F9F).recordType()` equals this exactly.
     */
    @Test
    fun recordTypeKeyMatchesTheOnChainRecordType() {
        assertEquals(
            "0x0ea3b61f198af15d1c1f1cd1bd926f52cb69cde62893f72fbb94e628c820321d",
            RoaxRpc.recordTypeKey("TRAVEL_CLEARANCE"),
        )
    }

    /** A resolved, authorized issuer leaves the issuance verdict exactly as it found it. */
    @Test
    fun anAuthorizedIssuerLeavesTheVerdictUnchanged() {
        assertEquals("VALID", RecordImporter.foldIssuerWhitelist("VALID", RoaxRpc.Result.Valid))
        assertEquals("INVALID", RecordImporter.foldIssuerWhitelist("INVALID", RoaxRpc.Result.Valid))
    }

    /** Resolved and NOT authorized: a real authenticity failure, not a caveat. */
    @Test
    fun anUnauthorizedIssuerFailsTheCredential() {
        assertEquals("INVALID", RecordImporter.foldIssuerWhitelist("VALID", RoaxRpc.Result.Invalid))
    }

    /**
     * THE regression. An unanswered check is not a passed check: an unresolved pillar must never leave
     * a VALID standing. This is the same shape as the `.unknown -> VALID` issuance fail-open — a check
     * that did not run being indistinguishable from one that passed.
     */
    @Test
    fun anUnresolvedPillarIsNeverAPass() {
        assertEquals(
            "UNVERIFIED",
            RecordImporter.foldIssuerWhitelist("VALID", RoaxRpc.Result.Unknown("rpc 502")),
        )
    }

    /** …but it never LAUNDERS a worse verdict into a better one either: the fold is monotone. */
    @Test
    fun anUnresolvedPillarNeverUpgradesAnInvalidVerdict() {
        assertEquals(
            "INVALID",
            RecordImporter.foldIssuerWhitelist("INVALID", RoaxRpc.Result.Unknown("rpc 502")),
        )
    }
}
