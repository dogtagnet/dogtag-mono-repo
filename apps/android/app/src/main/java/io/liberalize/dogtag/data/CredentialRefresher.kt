package io.liberalize.dogtag.data

import android.content.Context
import io.liberalize.dogtag.net.RoaxRpc
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import uniffi.dogtag_standard.dogTagIdFieldHex
import uniffi.dogtag_standard.verifyIntegrity

/**
 * Re-reads the on-chain status of an ALREADY-STORED credential and returns an updated copy.
 *
 * Without this the verdict is frozen at import: a record REVOKED afterwards keeps showing whatever
 * it showed on the day it was scanned, forever. A refresh recomputes both pillars from what the
 * phone already holds - INTEGRITY offline over the stored wrapped doc, and ISSUANCE as a live read
 * on ROAX - and stamps `lastCheckedAt` so the answer's age is visible.
 *
 * It NEVER fails open. If the chain cannot be reached the result is UNVERIFIED with the reason:
 * "I could not check" is not "I checked and it is fine", and on a revocation check that distinction
 * is the whole point.
 *
 * Mirrors iOS `CredentialRefresher` in RecordImporter.swift.
 */
object CredentialRefresher {

    suspend fun refreshed(
        cred: Credential,
        roax: RoaxConfig,
        rpcUrl: String = RoaxRpc.DEFAULT_RPC,
        /**
         * Test seams ONLY, so the verdict rule can be driven without a live RPC or the native FFI
         * (neither is available in a JVM unit test). Production leaves all three null and every value
         * is read for real. Mirrors the injected-reader pattern the web path already uses
         * (`IssuerChainReader`) — without them the refresh path could not be tested at all, and it is
         * precisely the path that silently upgraded verdicts.
         */
        whitelistPillar: RoaxRpc.Result? = null,
        issuancePillar: RoaxRpc.Result? = null,
        integrityOverride: String? = null,
    ): Credential {
        val stamped = cred.copy(lastCheckedAt = Stamp.now())

        val doc = try {
            WrappedDoc(cred.wrappedDocJson)
        } catch (e: Exception) {
            return stamped.marked("UNVERIFIED", "the stored document could not be read")
        }

        // 1. INTEGRITY (offline) - the stored doc must still hash to its own signed root.
        val integrity = integrityOverride
            ?: try { verifyIntegrity(cred.wrappedDocJson) } catch (e: Exception) { "INVALID" }
        if (integrity != "VALID") {
            return stamped.marked("INVALID", "the record's contents do not match its signed Merkle root")
        }

        val root = doc.merkleRoot.ifBlank { cred.credentialRoot }
        if (root.isBlank()) {
            return stamped.marked("UNVERIFIED", "this record carries no Merkle root to look up")
        }

        // 2. ISSUANCE (on-chain). Two anchor shapes: an ordinary record is anchored in a DogTagIssuer
        // clone (isValid(root)); a DOG_PROFILE is anchored in the DogTagSBT itself (profileRoot).
        if (doc.documentStore.isNotBlank()) {
            val issuance = when (
                val r = issuancePillar ?: RoaxRpc.isValid(rpcUrl, doc.documentStore, root)
            ) {
                is RoaxRpc.Result.Valid -> "VALID" to "anchored on ROAX and not revoked"
                is RoaxRpc.Result.Invalid -> "INVALID" to "revoked or no longer anchored on ROAX"
                is RoaxRpc.Result.Unknown -> "UNVERIFIED" to "could not reach the chain (${r.reason})"
            }
            // 3. ISSUER WHITELIST — the SAME mandatory pillar import runs, through the SAME fold.
            //
            // Without it this path answered VALID from `isValid` alone, which meant the first refresh
            // silently UPGRADED a credential import had correctly marked UNVERIFIED for an unresolved
            // issuer. A refresh may lower a verdict, and may raise one only as far as the pillars
            // actually establish — never past them.
            //
            // Scoped to the clone path deliberately: the DOG_PROFILE branch below anchors in the SBT,
            // where `rootIssuer`/`issuedBy` do not exist, so applying the pillar there would resolve
            // indeterminate and turn every profile refresh into UNVERIFIED — the same over-claiming
            // mistake inverted.
            val (verdict, reason) = IssuerWhitelist.evaluate(
                issuance.first, issuance.second, rpcUrl, roax,
                doc.documentStore, root, doc.recordType, whitelistPillar,
            )
            return stamped.marked(verdict, reason)
        }

        if (roax.dogTagSbt.isBlank() || cred.dogTagId.isBlank()) {
            return stamped.marked("UNVERIFIED", "this record carries no on-chain anchor to re-check")
        }
        // The SBT is keyed by the canonical `field_of_value(dogTagId)`, not the raw handle. Without
        // that key there is no lookup to make, so the record is UNVERIFIED - never looked up under
        // the raw handle, which reads an unset slot and would read as a mismatch.
        val onchainId = try {
            java.math.BigInteger(dogTagIdFieldHex(cred.dogTagId).removePrefix("0x"), 16).toString()
        } catch (e: Exception) {
            return stamped.marked("UNVERIFIED", "this dog tag id could not be resolved on-chain")
        }
        val onchainRoot = RoaxRpc.profileRoot(rpcUrl, roax.dogTagSbt, onchainId)
            ?: return stamped.marked(
                "UNVERIFIED", "could not reach the chain (DogTagSBT profileRoot read failed)",
            )
        // An all-zero word is the UNSET slot: the tag is not anchored (yet), which is not the same
        // answer as an anchored root that disagrees with ours. Only the latter is INVALID.
        if (onchainRoot.drop(2).all { it == '0' }) {
            return stamped.marked(
                "UNVERIFIED", "no profile root is anchored on-chain for this dog tag",
            )
        }
        if (!onchainRoot.equals(root, ignoreCase = true)) {
            return stamped.marked(
                "INVALID", "the dog tag's on-chain profile root no longer matches this record",
            )
        }
        // The tag is custodial (owner-hidden), so there is no on-chain owner to compare against - the
        // profile-root match IS the anchor check, and the reason says exactly which check ran.
        return stamped.marked("VALID", "the dog tag's on-chain profile root matches this record")
    }

    /** Write a verdict and its reason together, so the pair can never drift apart. */
    private fun Credential.marked(verdict: String, reason: String): Credential =
        copy(verdict = verdict, verdictReason = reason)
}

/**
 * Shared state for on-chain status refreshes, so every surface that shows a verdict (Home,
 * Documents, Travel, the credential detail) reports the SAME three distinguishable states for a
 * given record: checking now, checked (with the answer and its age), or could-not-check (with why).
 *
 * The outcome itself is persisted on the credential; this only holds what is in flight.
 * Mirrors iOS `RefreshCenter` in LocalStore.swift.
 */
class RefreshCenter(context: Context) {
    private val appContext = context.applicationContext
    private val store = LocalStore.get(appContext)

    private val _inFlight = MutableStateFlow<Set<String>>(emptySet())

    /** Credential ids with a check currently running. */
    val inFlight: StateFlow<Set<String>> = _inFlight

    private val _batchRemaining = MutableStateFlow(0)

    /** How many records are left in a running "refresh all" (0 when no batch is running). */
    val batchRemaining: StateFlow<Int> = _batchRemaining

    private var roax: RoaxConfig? = null

    /** Re-read one credential's on-chain status and persist the result. */
    suspend fun refresh(cred: Credential) {
        if (_inFlight.value.contains(cred.id)) return
        _inFlight.value = _inFlight.value + cred.id
        try {
            val config = resolvedRoax()
            val updated = withContext(Dispatchers.IO) {
                CredentialRefresher.refreshed(cred, config)
            }
            store.updateCredential(updated)
        } finally {
            _inFlight.value = _inFlight.value - cred.id
        }
    }

    /**
     * Re-read a whole list, one record at a time so the RPC is not hammered and the remaining count
     * stays meaningful. A second call while a batch is running is ignored.
     */
    suspend fun refreshAll(creds: List<Credential>) {
        if (_batchRemaining.value != 0 || creds.isEmpty()) return
        _batchRemaining.value = creds.size
        try {
            for (cred in creds) {
                refresh(cred)
                _batchRemaining.value = _batchRemaining.value - 1
            }
        } finally {
            _batchRemaining.value = 0
        }
    }

    private fun resolvedRoax(): RoaxConfig =
        roax ?: RoaxConfig.load(appContext).also { roax = it }

    companion object {
        @Volatile private var instance: RefreshCenter? = null

        fun get(context: Context): RefreshCenter =
            instance ?: synchronized(this) {
                instance ?: RefreshCenter(context.applicationContext).also { instance = it }
            }
    }
}
