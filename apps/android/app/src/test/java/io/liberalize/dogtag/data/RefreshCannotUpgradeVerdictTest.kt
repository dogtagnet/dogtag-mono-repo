package io.liberalize.dogtag.data

import io.liberalize.dogtag.net.RoaxRpc
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/**
 * CROSS-PR SECURITY REGRESSION (#87 x fm/dogtag-failopen-f2).
 *
 * Refresh-from-chain shipped answering VALID from `isValid` alone, with no issuer-whitelist pillar.
 * That did not merely miss a check: it ACTIVELY UPGRADED a credential that import had correctly
 * recorded as UNVERIFIED. The very feature added so a revocation could reach the owner became the
 * thing that laundered an unverified credential into a trusted one, silently, on first refresh.
 *
 * It is worse than the original import fail-open, because that one at least never overwrote a correct
 * negative with a positive.
 *
 * These tests pin the rule a refresh must obey: it may LOWER a verdict freely, and may RAISE one only
 * as far as the pillars actually establish - never past them.
 */
@RunWith(RobolectricTestRunner::class)
class RefreshCannotUpgradeVerdictTest {

    /** A stored credential already marked UNVERIFIED because the whitelist pillar did not resolve. */
    private fun unverifiedCredential() = Credential(
        id = "rec-1",
        dogTagId = "42",
        group = CredentialGroup.Travel,
        recordType = "TRAVEL_CLEARANCE",
        title = "Travel Clearance",
        subtitle = "TRAVEL_CLEARANCE",
        issuer = "DogTag Government Authority",
        issuedOn = "",
        credentialRoot = "0x11",
        verdict = "UNVERIFIED",
        wrappedDocJson = """{"version":"dogtag/1.0","data":{},
            "signature":{"type":"DogTagMerkleProof","targetHash":"0x11","proof":[],"merkleRoot":"0x11"},
            "privacy":{"obfuscated":[]},
            "issuer":{"documentStore":"0xabc","name":"Gov","domain":"gov.example","recordType":"TRAVEL_CLEARANCE"}}""",
        verdictReason = "could not establish who issued this record (rpc 502)",
    )

    private val roax = RoaxConfig(
        chainId = 135,
        dogTagSbt = "0xsbt",
        issuerRegistry = "0xreg",
        protocolRegistry = "0xproto",
        dogTagIssuerFactory = "0xfactory",
    )

    /**
     * THE regression. The refresh learns exactly one new thing - `isValid` is true - and the whitelist
     * pillar is STILL unresolved. That must not become VALID.
     *
     * This test can genuinely fail: delete the `IssuerWhitelist.evaluate` call from
     * `CredentialRefresher` and the verdict reverts to VALID / "anchored on ROAX and not revoked",
     * which is precisely the shipped defect.
     */
    @Test
    fun anUnresolvedWhitelistPillarIsNotUpgradedToValidByARefresh() = runBlocking {
        val out = CredentialRefresher.refreshed(
            unverifiedCredential(),
            roax,
            whitelistPillar = RoaxRpc.Result.Unknown("no factory clone ever issued this root"),
            issuancePillar = RoaxRpc.Result.Valid,
            integrityOverride = "VALID",
        )

        assertEquals("UNVERIFIED", out.verdict)
        // …and the reason must not still claim the record is anchored and unrevoked.
        assertEquals(
            "could not establish who issued this record (no factory clone ever issued this root)",
            out.verdictReason,
        )
    }

    /** A refresh MAY raise the verdict when the pillars genuinely establish it - that is its job. */
    @Test
    fun aFullyResolvedRefreshDoesRaiseTheVerdict() = runBlocking {
        val out = CredentialRefresher.refreshed(
            unverifiedCredential(),
            roax,
            whitelistPillar = RoaxRpc.Result.Valid,
            issuancePillar = RoaxRpc.Result.Valid,
            integrityOverride = "VALID",
        )

        assertEquals("VALID", out.verdict)
        assertEquals("anchored on ROAX and not revoked", out.verdictReason)
    }

    /** A resolved-but-unauthorised issuer is a hard failure on refresh, exactly as on import. */
    @Test
    fun anUnauthorisedIssuerFailsClosedOnRefresh() = runBlocking {
        val out = CredentialRefresher.refreshed(
            unverifiedCredential(),
            roax,
            whitelistPillar = RoaxRpc.Result.Invalid,
            issuancePillar = RoaxRpc.Result.Valid,
            integrityOverride = "VALID",
        )

        assertEquals("INVALID", out.verdict)
        assertEquals(
            "the address that issued this record is not authorised to issue this record type",
            out.verdictReason,
        )
    }

    /** A revoked record still reports revoked; the pillar tightens, it never launders. */
    @Test
    fun aRevokedRecordStaysInvalidEvenWithAnAuthorisedIssuer() = runBlocking {
        val out = CredentialRefresher.refreshed(
            unverifiedCredential(),
            roax,
            whitelistPillar = RoaxRpc.Result.Valid,
            issuancePillar = RoaxRpc.Result.Invalid,
            integrityOverride = "VALID",
        )

        assertEquals("INVALID", out.verdict)
        assertEquals("revoked or no longer anchored on ROAX", out.verdictReason)
    }
}
