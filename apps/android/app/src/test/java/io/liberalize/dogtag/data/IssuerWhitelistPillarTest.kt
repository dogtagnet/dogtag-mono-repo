package io.liberalize.dogtag.data

import io.liberalize.dogtag.net.RoaxRpc
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
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
     * `rootIssuer(bytes32)` on the FACTORY is what makes the pillar trustworthy at all: it is the
     * write-once, `isClone`-gated index naming the contract that really issued the root, so the
     * document never gets to choose which contract answers for it. A drifted selector reverts, the
     * clone never resolves, and every credential silently degrades to unresolved.
     * `cast sig "rootIssuer(bytes32)"`.
     */
    @Test
    fun rootIssuerSelectorMatchesTheCanonicalSignature() {
        assertEquals("0x41e41d17", RoaxRpc.functionSelector("rootIssuer(bytes32)"))
    }

    /**
     * `recordType()` on the resolved clone supplies the whitelist key, so the pillar asks about the
     * record type the CHAIN says the root has rather than the one the envelope claims.
     * `cast sig "recordType()"`.
     */
    @Test
    fun recordTypeSelectorMatchesTheCanonicalSignature() {
        assertEquals("0xe55e492c", RoaxRpc.functionSelector("recordType()"))
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

    /**
     * A read that never ANSWERED must not be reported to the owner as a chain FACT.
     *
     * "No factory clone ever issued this root" is a statement about the chain's contents; a dropped
     * connection, a revert or a truncated reply establishes nothing of the sort. Both stay
     * indeterminate - that tri-state is untouched - but only the genuine zero slot may claim what the
     * chain says, because this text is persisted in `verdictReason` and shown to the holder.
     *
     * This is the PR's own defect class relocated into a string: an unrun check reported as a
     * conclusion.
     */
    @Test
    fun anUnresolvedReadSaysTheReadFailedRatherThanAssertingAChainFact() = runBlocking {
        val pillar = RoaxRpc.issuerWhitelistPillar(
            rpcUrl = "http://127.0.0.1:1",
            issuerRegistry = "0xreg",
            issuerFactory = "0xfactory",
            documentStore = "0xabc",
            root = "",
            recordType = "TRAVEL_CLEARANCE",
        )
        val reason = (pillar as RoaxRpc.Result.Unknown).reason
        assertTrue(reason, reason.startsWith("could not read the factory's issuer index ("))
        assertTrue(reason, !reason.contains("no factory clone ever issued this root"))
    }

    /**
     * A document naming NO issuer contract is indeterminate, not a mismatch.
     *
     * The pillar used to compare the factory's answer against the empty string and return a definite
     * Invalid, so import stamped a hard INVALID while [CredentialRefresher] - which scopes the pillar
     * to a non-blank `documentStore` for exactly this reason, since a DOG_PROFILE anchors in the SBT
     * instead - answered indeterminate for the same record. Two verdicts for one credential.
     */
    @Test
    fun aDocumentNamingNoIssuerContractIsIndeterminateNotAMismatch() = runBlocking {
        val pillar = RoaxRpc.issuerWhitelistPillar(
            rpcUrl = "http://127.0.0.1:1",
            issuerRegistry = "0xreg",
            issuerFactory = "0xfactory",
            documentStore = "   ",
            root = "0x11",
            recordType = "DOG_PROFILE",
        )
        assertEquals(
            RoaxRpc.Result.Unknown("document names no issuer contract"),
            pillar,
        )
        // …and folding it can only degrade a pass, never manufacture a forgery accusation.
        assertEquals("UNVERIFIED", RecordImporter.foldIssuerWhitelist("VALID", pillar))
    }
}
