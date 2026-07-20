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

    /**
     * Conflict-checked insert/replace keyed by the canonical `dogTagIdHex`.
     *
     * Fail-closed for the write-once root: an identical root is an idempotent retry and refreshes
     * the record, a DIFFERENT root for the same id is rejected before the existing witness is
     * touched. `DogTagSBTConsent.profileRoot` is write-once on-chain, so once a tag is issued its
     * `R` can never be moved - a store that quietly overwrote the witness for that id would strand
     * the tag with a secret that no longer rebuilds the sealed root, and the attribute salts it
     * dropped exist nowhere else on the device.
     */
    fun upsert(records: List<OwnerSecretRecord>, record: OwnerSecretRecord): List<OwnerSecretRecord> {
        val idx = records.indexOfFirst { it.dogTagIdHex.equals(record.dogTagIdHex, ignoreCase = true) }
        if (idx < 0) return records + record

        val existing = records[idx]
        if (!existing.rootHex.equals(record.rootHex, ignoreCase = true)) {
            throw ConflictingRootException(record.dogTagIdDec, existing.rootHex, record.rootHex)
        }
        return records.toMutableList().also { it[idx] = record }
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
            arr.put(
                JSONObject()
                    .put("dogTagIdHex", r.dogTagIdHex)
                    .put("dogTagIdDec", r.dogTagIdDec)
                    .put("ownerSecretHex", r.ownerSecretHex)
                    .put("rootHex", r.rootHex)
                    .put("ownerAddress", r.ownerAddress)
                    .put("attributes", attrs)
                    .put("derivationVersion", r.derivationVersion)
                    .put("savedAt", r.savedAt),
            )
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
            )
        }
    }
}
