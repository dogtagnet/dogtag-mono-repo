package io.liberalize.dogtag.data

import org.json.JSONArray
import org.json.JSONObject

/**
 * A credential group as shown on Home (Health / Service / Travel). The group is derived from the
 * record's `recordType` (the on-chain label / issuer record type).
 */
enum class CredentialGroup(val title: String) {
    Health("Health Records"),
    Service("Service Dog"),
    Travel("Travel Docs");

    companion object {
        /** Map an issuer recordType label (e.g. "VACCINATION", "SERVICE_ATTESTATION") to a group. */
        fun fromRecordType(recordType: String?): CredentialGroup {
            val rt = (recordType ?: "").uppercase()
            return when {
                rt.contains("SERVICE") || rt.contains("DOT") -> Service
                rt.contains("TRAVEL") || rt.contains("CDC") || rt.contains("EU_HEALTH") ||
                    rt.contains("IMPORT") || rt.contains("USDA") -> Travel
                else -> Health
            }
        }
    }
}

/** A pet the user owns. Seeded from central GET /v1/pets and/or imported records. */
data class Pet(
    val dogTagId: String,        // on-chain DogTagSBT tokenId (decimal) — primary key
    val name: String,
    val breed: String,
    val ageLabel: String,
    val microchip: String? = null,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("dogTagId", dogTagId)
        put("name", name)
        put("breed", breed)
        put("ageLabel", ageLabel)
        if (microchip != null) put("microchip", microchip)
    }

    companion object {
        fun fromJson(o: JSONObject): Pet = Pet(
            dogTagId = o.optString("dogTagId"),
            name = o.optString("name", "Unnamed"),
            breed = o.optString("breed", ""),
            ageLabel = o.optString("ageLabel", ""),
            microchip = o.optString("microchip", "").ifBlank { null },
        )

        /** Parse a pet from the central GET /v1/pets `pets[]` entry. */
        fun fromCentral(o: JSONObject): Pet {
            val mc = o.optJSONObject("microchip")
            // central pets may not be minted yet (no dogTagId); fall back to the pet id.
            val tag = o.optString("dogTagId", "").ifBlank { o.optString("id", "") }
            val profile = o.optJSONObject("profile")
            return Pet(
                dogTagId = tag,
                name = o.optString("name", "Unnamed"),
                breed = profile?.optString("breed", "") ?: "",
                ageLabel = profile?.optString("dateOfBirth", "") ?: "",
                microchip = mc?.optString("code", "")?.ifBlank { null },
            )
        }
    }
}

/**
 * A single imported credential / record held for a pet. The full wrapped doc JSON is kept so the
 * 4-check verification (integrity + on-chain isValid + identity) can be re-run on demand, and so the
 * record can be re-presented (consent over `credentialRoot`) in a verify flow.
 */
data class Credential(
    val id: String,              // recordId from the vet record link, or a local uuid
    val dogTagId: String,        // owning pet
    val group: CredentialGroup,
    val recordType: String,      // raw issuer record type label
    val title: String,
    val subtitle: String,
    val issuer: String,
    val issuedOn: String,
    val credentialRoot: String,  // signature.merkleRoot (0x..) — what consent signs over
    val verdict: String,         // "VALID" / "INVALID" / "UNVERIFIED"
    val wrappedDocJson: String,  // the full wrapped doc (for re-verify + disclosure)
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("id", id)
        put("dogTagId", dogTagId)
        put("group", group.name)
        put("recordType", recordType)
        put("title", title)
        put("subtitle", subtitle)
        put("issuer", issuer)
        put("issuedOn", issuedOn)
        put("credentialRoot", credentialRoot)
        put("verdict", verdict)
        put("wrappedDocJson", wrappedDocJson)
    }

    companion object {
        fun fromJson(o: JSONObject): Credential = Credential(
            id = o.optString("id"),
            dogTagId = o.optString("dogTagId"),
            group = runCatching { CredentialGroup.valueOf(o.optString("group")) }
                .getOrDefault(CredentialGroup.Health),
            recordType = o.optString("recordType"),
            title = o.optString("title"),
            subtitle = o.optString("subtitle"),
            issuer = o.optString("issuer"),
            issuedOn = o.optString("issuedOn"),
            credentialRoot = o.optString("credentialRoot"),
            verdict = o.optString("verdict", "UNVERIFIED"),
            wrappedDocJson = o.optString("wrappedDocJson"),
        )
    }
}

/**
 * A thin, typed view over a wrapped-doc JSON (the §1.4 WrappedDoc). Used to extract the fields the
 * mobile app needs without re-implementing the canonicalization (the heavy lifting stays in Rust via
 * `verifyIntegrity` / `buildMerkleRootHex`).
 */
class WrappedDoc(val json: String) {
    private val root = JSONObject(json)
    private val sig: JSONObject = root.optJSONObject("signature") ?: JSONObject()
    private val issuerObj: JSONObject = root.optJSONObject("issuer") ?: JSONObject()
    private val data: JSONObject = root.optJSONObject("data") ?: JSONObject()

    val merkleRoot: String get() = sig.optString("merkleRoot")
    val targetHash: String get() = sig.optString("targetHash")
    val documentStore: String get() = issuerObj.optString("documentStore")
    val issuerName: String get() = issuerObj.optString("name", "Unknown issuer")
    val issuerDomain: String get() = issuerObj.optString("domain", "")
    val recordType: String get() = issuerObj.optString("recordType", "")

    /** Best-effort dogTagId extraction from the data tree (data.credentialSubject.dogTagId leaf). */
    val dogTagId: String
        get() {
            // leaves are packed "tag:salt:value"; pull the trailing value if present.
            val cs = data.optJSONObject("credentialSubject")
            val raw = cs?.optString("dogTagId", "") ?: ""
            return raw.substringAfterLast(":").ifBlank { raw }
        }

    /** A short human title for the record (issuer recordType + best subject hint). */
    fun displayTitle(): String {
        val rt = recordType.ifBlank { "Record" }
        return rt.replace('_', ' ').lowercase().replaceFirstChar { it.uppercase() }
    }

    /** The hashes the issuer redacted (selective disclosure). */
    val obfuscatedCount: Int
        get() = root.optJSONObject("privacy")?.optJSONArray("obfuscated")?.length() ?: 0

    /**
     * One decoded Merkle leaf: the dotted key path, the type tag, and the human-readable value.
     * Leaves in `data` are packed as `"<saltHex>:<tag>:<value>"` (split on the FIRST TWO colons —
     * the value may itself contain ':').
     */
    data class DecodedField(val keyPath: String, val tag: Int, val value: String) {
        /** A title-cased, human label derived from the keyPath (strips a leading `credentialSubject.`). */
        val label: String get() = humanizeKeyPath(keyPath)
    }

    /**
     * Flatten `data` into an ordered list of decoded leaves. Objects recurse with dotted key paths;
     * arrays index with `[i]`. Each scalar leaf is parsed `"<salt>:<tag>:<value>"` (first two ':').
     */
    fun decodedFields(): List<DecodedField> {
        val out = ArrayList<DecodedField>()
        flatten(data, "", out)
        return out
    }

    private fun flatten(node: Any?, prefix: String, out: MutableList<DecodedField>) {
        when (node) {
            is JSONObject -> {
                val keys = node.keys()
                while (keys.hasNext()) {
                    val k = keys.next()
                    val child = node.opt(k)
                    val path = if (prefix.isEmpty()) k else "$prefix.$k"
                    flatten(child, path, out)
                }
            }
            is JSONArray -> {
                for (i in 0 until node.length()) {
                    flatten(node.opt(i), "$prefix[$i]", out)
                }
            }
            is String -> out.add(parseLeaf(prefix, node))
            // Non-string scalars are not part of the packed-leaf format; show as-is.
            else -> if (node != null && node != JSONObject.NULL) {
                out.add(DecodedField(prefix, 2, node.toString()))
            }
        }
    }

    companion object {
        /** Parse a packed `"<salt>:<tag>:<value>"` leaf — split on the FIRST TWO colons only. */
        fun parseLeaf(keyPath: String, raw: String): DecodedField {
            val first = raw.indexOf(':')
            if (first < 0) return DecodedField(keyPath, 2, raw)
            val second = raw.indexOf(':', first + 1)
            if (second < 0) return DecodedField(keyPath, 2, raw)
            val tag = raw.substring(first + 1, second).toIntOrNull() ?: 2
            val value = raw.substring(second + 1)
            return DecodedField(keyPath, tag, value)
        }

        /**
         * Humanize a dotted keyPath into a Title Case label. Strips a leading `credentialSubject.`,
         * splits on dots, splits camelCase into words, drops array indices, and title-cases.
         * e.g. `credentialSubject.microchip.code` -> "Microchip code",
         *      `vaccineProductName` -> "Vaccine product name".
         */
        fun humanizeKeyPath(keyPath: String): String {
            var path = keyPath.removePrefix("credentialSubject.")
            // Drop array index brackets, e.g. "vaccinations[0].date" -> "vaccinations.date 1".
            val idxMatch = Regex("\\[(\\d+)]")
            val indices = idxMatch.findAll(path).map { it.groupValues[1].toInt() + 1 }.toList()
            path = idxMatch.replace(path, "")
            val words = path.split('.')
                .filter { it.isNotBlank() }
                .flatMap { splitCamel(it) }
            if (words.isEmpty()) return keyPath
            val titled = words.mapIndexed { i, w ->
                if (i == 0) w.replaceFirstChar { it.uppercase() } else w.lowercase()
            }.joinToString(" ")
            return if (indices.isNotEmpty()) "$titled ${indices.joinToString(" ")}" else titled
        }

        private fun splitCamel(s: String): List<String> =
            Regex("(?<=[a-z0-9])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])")
                .split(s)
                .filter { it.isNotBlank() }
    }
}
