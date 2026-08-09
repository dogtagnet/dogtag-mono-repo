package io.liberalize.dogtag.profile

import org.json.JSONArray
import org.json.JSONObject

/** One backed-up attribute leaf. Mirrors `AttributeLeafFfi` / iOS `BackedUpAttribute`. */
data class BackedUpAttribute(
    val keyPath: String,
    val saltHex: String,
    val tag: UByte,
    val value: String,
)

/**
 * The deployment a record's tag lives on. Mirrors iOS `DeploymentScope` — change both together.
 *
 * A dog tag's on-chain identity is (chain, SBT contract, id): the SAME decimal id exists
 * independently on every deployment, because a redeploy stands up a fresh `DogTagSBTConsent` whose
 * id space starts empty. This pair is the SMALLEST honest key for "which deployment": the SBT
 * address alone names the contract the tag is literally a token on, and `chainId` closes the
 * cross-chain address-collision corner. Both come from the bundled `roax.json` (the ledger's own
 * key names), the one deployment fact the phone holds.
 *
 * Records written before this existed carry `null` — "recorded before deployments were tracked" —
 * which is a distinct fact, never assumed equal to any deployment (see [OwnerSecretRecords]).
 */
data class DeploymentScope(
    val chainId: Long,
    val sbtAddress: String,
) {
    fun matches(other: DeploymentScope): Boolean =
        chainId == other.chainId && sbtAddress.equals(other.sbtAddress, ignoreCase = true)

    companion object {
        /**
         * The CURRENT deployment, from the bundled config values. Returns null when the bundle is
         * stale/blank (every consumer already treats a blank address as could-not-check) — callers
         * must then fail closed rather than write a record whose deployment is a guess.
         */
        fun of(chainId: Long, sbtAddress: String): DeploymentScope? =
            if (sbtAddress.isBlank()) null else DeploymentScope(chainId, sbtAddress)
    }
}

/**
 * Everything needed to rebuild one tag's tree - and therefore to regenerate proofs after a device
 * loss. **Holds a recovery secret** ([ownerSecretHex]); see [ProfileTreeStore] and
 * `docs/MOBILE_OWNER_SECRET.md`.
 *
 * Carries every field iOS `ProfileTreeStore.OwnerSecretRecord` writes EXCEPT the three optional M6
 * re-issue fields (`abandonedAt`, `replacedByDogTagIdDec`, `replacesDogTagIdDec`), which Android does
 * not yet write because `reissue` (D3) is iOS-only - see the parity table in
 * `docs/MOBILE_OWNER_SECRET.md`. [OwnerSecretRecords.decode] ignores unknown keys, so an iOS record
 * re-encoded here would silently drop that abandoned↔fresh linkage.
 */
data class OwnerSecretRecord(
    /** Canonical dogTagId field, the value the tree is bound to. */
    val dogTagIdHex: String,
    /** The human-facing decimal id. */
    val dogTagIdDec: String,
    /** SECRET - the nullifier's secret leaf. Never transmit. */
    val ownerSecretHex: String,
    /** `R` - the only value the issuer ever sees. */
    val rootHex: String,
    val ownerAddress: String,
    val attributes: List<BackedUpAttribute>,
    val derivationVersion: String,
    /** ISO-8601 UTC. */
    val savedAt: String,
    /**
     * The deployment this tag was issued on, or null for a record written before deployments were
     * tracked (a legacy record — kept, never orphaned; see [OwnerSecretRecords.reuseDecision]).
     */
    val deployment: DeploymentScope? = null,
)

/**
 * The record list's pure logic: JSON codec + the conflict-checked upsert.
 *
 * Deliberately free of `Context`, Keystore and file I/O so the write-once-root invariant below is
 * covered by a plain JVM unit test. [ProfileTreeStore] supplies the encryption and persistence
 * around it.
 */
object OwnerSecretRecords {

    /** Raised when an upsert would move a tag's `R`. See [upsert]. */
    class ConflictingRootException(
        val dogTagIdDec: String,
        val existing: String,
        val proposed: String,
    ) : IllegalStateException(
        "dogTagId $dogTagIdDec already has root $existing; refusing replacement with $proposed",
    )

    /** Do two records live on the same deployment? Two legacy (untracked) records do; a legacy and
     * a scoped record NEVER do — "unknown" must not compare equal to any deployment, or the
     * write-once check below would refuse a genuinely free id on a fresh deployment (the exact
     * poisoning a redeploy used to inflict on every low id a handset had seen). */
    fun sameScope(a: DeploymentScope?, b: DeploymentScope?): Boolean = when {
        a == null && b == null -> true
        a != null && b != null -> a.matches(b)
        else -> false
    }

    /**
     * Conflict-checked insert/replace keyed by (deployment, canonical `dogTagIdHex`).
     *
     * Fail-closed for the write-once root WITHIN one deployment: an identical root is an
     * idempotent retry and refreshes the record, a DIFFERENT root for the same id ON THE SAME
     * deployment is rejected before the existing witness is touched. `DogTagSBTConsent.profileRoot`
     * is write-once on-chain, so once a tag is issued its `R` can never be moved - a store that
     * quietly overwrote the witness for that id would strand the tag with a secret that no longer
     * rebuilds the sealed root, and the attribute salts it dropped exist nowhere else on the
     * device. The same id on a DIFFERENT deployment is a different tag entirely and the two
     * records coexist.
     *
     * MIGRATION is the one cross-scope write: a scoped record whose root equals a legacy
     * (untracked) record's root REPLACES that record — stamping it with the deployment the chain
     * evidence just tied it to — rather than duplicating the tag.
     */
    fun upsert(records: List<OwnerSecretRecord>, record: OwnerSecretRecord): List<OwnerSecretRecord> {
        val idx = records.indexOfFirst {
            it.dogTagIdHex.equals(record.dogTagIdHex, ignoreCase = true) &&
                sameScope(it.deployment, record.deployment)
        }
        if (idx >= 0) {
            val existing = records[idx]
            if (!existing.rootHex.equals(record.rootHex, ignoreCase = true)) {
                throw ConflictingRootException(record.dogTagIdDec, existing.rootHex, record.rootHex)
            }
            return records.toMutableList().also { it[idx] = record }
        }
        if (record.deployment != null) {
            val legacyIdx = records.indexOfFirst {
                it.dogTagIdHex.equals(record.dogTagIdHex, ignoreCase = true) &&
                    it.deployment == null &&
                    it.rootHex.equals(record.rootHex, ignoreCase = true)
            }
            if (legacyIdx >= 0) {
                return records.toMutableList().also { it[legacyIdx] = record }
            }
        }
        return records + record
    }

    /**
     * Which stored record answers for `dogTagIdDec` on the CURRENT deployment: an exact scope
     * match first, else a legacy (untracked) record, NEVER a record scoped to a different
     * deployment — that one is another deployment's tag and answering with it is how tag 1 on a
     * new deployment collided with tag 1 on the old one. Mirrors iOS
     * `OwnerSecretScoping.preferredIndex`.
     */
    fun preferredFor(
        records: List<OwnerSecretRecord>,
        dogTagIdDec: String,
        current: DeploymentScope?,
    ): OwnerSecretRecord? {
        val forTag = records.filter { it.dogTagIdDec == dogTagIdDec }
        return forTag.firstOrNull { it.deployment != null && sameScope(it.deployment, current) }
            ?: forTag.firstOrNull { it.deployment == null }
    }

    /** The bind flow's verdict on a stored record for the id the vet just named. Mirrors iOS
     * `OwnerSecretScoping.ReuseDecision` — change both together. */
    enum class ReuseDecision {
        /** Same session (content byte-identical): rebuild from the stored witness. Safe across a
         * redeploy — `R` is a pure device-side commitment, and it is being anchored NOW on the
         * current deployment. A legacy record reused this way gets STAMPED with the current
         * deployment (the migration: the byte-identical vet-salted identity leaves are the
         * evidence tying it here). */
        REUSE,

        /** A legacy record whose content differs, met while the current deployment IS known: the
         * redeploy reading. The record is another deployment's tag (the fixed vet allocator never
         * re-hands a minted id within one deployment), so this id is genuinely free here — build a
         * fresh witness and keep the old record untouched beside it. */
        BUILD_FRESH,

        /** Same deployment (or no way to tell one), same id, different content: a real conflict.
         * Refuse before the stored witness — unrecoverable attribute salts — can be disturbed. */
        REFUSE_CONFLICT,
    }

    fun reuseDecision(
        recordScope: DeploymentScope?,
        current: DeploymentScope?,
        contentMatches: Boolean,
    ): ReuseDecision = when {
        contentMatches -> ReuseDecision.REUSE
        recordScope == null && current != null -> ReuseDecision.BUILD_FRESH
        else -> ReuseDecision.REFUSE_CONFLICT
    }

    fun encode(records: List<OwnerSecretRecord>): String {
        val arr = JSONArray()
        records.forEach { r ->
            val attrs = JSONArray()
            r.attributes.forEach { a ->
                attrs.put(
                    JSONObject()
                        .put("keyPath", a.keyPath)
                        .put("saltHex", a.saltHex)
                        .put("tag", a.tag.toInt())
                        .put("value", a.value),
                )
            }
            val obj = JSONObject()
                .put("dogTagIdHex", r.dogTagIdHex)
                .put("dogTagIdDec", r.dogTagIdDec)
                .put("ownerSecretHex", r.ownerSecretHex)
                .put("rootHex", r.rootHex)
                .put("ownerAddress", r.ownerAddress)
                .put("attributes", attrs)
                .put("derivationVersion", r.derivationVersion)
                .put("savedAt", r.savedAt)
            // Written only when present, so a legacy record round-trips as legacy rather than
            // acquiring a deployment nobody established.
            r.deployment?.let {
                obj.put(
                    "deployment",
                    JSONObject().put("chainId", it.chainId).put("sbtAddress", it.sbtAddress),
                )
            }
            arr.put(obj)
        }
        return arr.toString(2)
    }

    /** Throws if the payload is not a well-formed record array; callers must NOT treat that as empty. */
    fun decode(json: String): List<OwnerSecretRecord> {
        val arr = JSONArray(json)
        return (0 until arr.length()).map { i ->
            val o = arr.getJSONObject(i)
            val attrs = o.getJSONArray("attributes")
            OwnerSecretRecord(
                dogTagIdHex = o.getString("dogTagIdHex"),
                dogTagIdDec = o.getString("dogTagIdDec"),
                ownerSecretHex = o.getString("ownerSecretHex"),
                rootHex = o.getString("rootHex"),
                ownerAddress = o.getString("ownerAddress"),
                attributes = (0 until attrs.length()).map { j ->
                    val a = attrs.getJSONObject(j)
                    BackedUpAttribute(
                        keyPath = a.getString("keyPath"),
                        saltHex = a.getString("saltHex"),
                        tag = a.getInt("tag").toUByte(),
                        value = a.getString("value"),
                    )
                },
                derivationVersion = o.getString("derivationVersion"),
                savedAt = o.getString("savedAt"),
                // `optJSONObject` because the key is ADDITIVE: stores written before deployment
                // scoping existed must keep decoding (this codec's required-key reads make any new
                // REQUIRED key an unreadable-store event for every pre-existing record). A present
                // but malformed object still throws — fail closed, never a silently-dropped scope.
                deployment = o.optJSONObject("deployment")?.let { d ->
                    DeploymentScope(
                        chainId = d.getLong("chainId"),
                        sbtAddress = d.getString("sbtAddress"),
                    )
                },
            )
        }
    }
}
