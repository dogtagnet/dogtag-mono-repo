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

    // ---- provenance / freshness (all nullable: records stored before these shipped have none) ----

    /**
     * When THIS device stored the record ([Stamp] ISO-8601 UTC). Two records of the same type are
     * otherwise indistinguishable in a list, so this is what tells them apart for the owner.
     */
    val importedAt: String? = null,
    /**
     * When the on-chain status behind [verdict] was last determined, successfully or not. Written
     * together with [verdict] + [verdictReason] so all three always describe the SAME check.
     */
    val lastCheckedAt: String? = null,
    /** Short human explanation of [verdict] - above all, why it is not VALID. */
    val verdictReason: String? = null,
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
        if (importedAt != null) put("importedAt", importedAt)
        if (lastCheckedAt != null) put("lastCheckedAt", lastCheckedAt)
        if (verdictReason != null) put("verdictReason", verdictReason)
    }

    /**
     * When this record landed on this phone, bare. Records stored before import stamping shipped
     * carry no stamp: say so plainly rather than inventing a date.
     */
    val importedAtValue: String get() = Stamp.parse(importedAt)?.let { Stamp.absolute(it) } ?: "Unknown"

    /** The same, as a standalone list line. */
    val importedAtLabel: String
        get() = if (Stamp.parse(importedAt) == null) "Import date unknown" else "Imported $importedAtValue"

    /**
     * How fresh [verdict] is. A verdict is frozen at the moment it was read off the chain, so a
     * record revoked since then still reads VALID until it is refreshed - this is what makes that
     * staleness visible instead of silent.
     */
    val lastCheckedLabel: String
        get() = Stamp.parse(lastCheckedAt)?.let { "Checked ${Stamp.relative(it)}" } ?: "Not checked on-chain yet"

    /** The freshness line, plus the reason whenever the verdict is anything other than VALID. */
    val statusLine: String
        get() = if (verdict != "VALID" && !verdictReason.isNullOrBlank()) {
            "$lastCheckedLabel · $verdictReason"
        } else {
            lastCheckedLabel
        }

    /**
     * Body of the delete confirmation. It leads with the details that tell two same-type records
     * apart, so the owner can see WHICH one is about to go, then states plainly what deleting does.
     * Deleting is local: it must not read as a revocation, because it is not one.
     */
    fun deleteConfirmationMessage(petName: String): String {
        val which = if (petName.isBlank()) importedAtLabel else "$importedAtLabel · $petName"
        return which + "\n\nThis removes the copy stored on this phone. The record is not revoked, " +
            "nothing changes on-chain, and the issuer still holds their copy."
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
            importedAt = o.optString("importedAt", "").ifBlank { null },
            lastCheckedAt = o.optString("lastCheckedAt", "").ifBlank { null },
            verdictReason = o.optString("verdictReason", "").ifBlank { null },
        )
    }
}

/**
 * ISO-8601 (UTC, whole seconds) stamping and human display for the credential timestamps. iOS writes
 * the identical shape ("2026-07-27T14:32:10Z"), so the two stores stay readable across ports.
 */
object Stamp {
    /** The current instant, in the stored form. */
    fun now(): String = java.time.Instant.now().truncatedTo(java.time.temporal.ChronoUnit.SECONDS).toString()

    /** Parse a stored stamp. null for absent/unparseable. */
    fun parse(raw: String?): java.time.Instant? {
        if (raw.isNullOrBlank()) return null
        return runCatching { java.time.Instant.parse(raw) }.getOrNull()
    }

    /**
     * Absolute local date + time, e.g. "27 Jul 2026, 14:32". Used where the exact moment is the
     * point (which of two look-alike records is which).
     */
    fun absolute(instant: java.time.Instant): String = ABSOLUTE.format(instant)

    /**
     * Relative age, e.g. "just now" / "5 minutes ago" / "3 days ago". Used for freshness, where the
     * distinction that matters is "checked seconds ago" vs "checked last week".
     */
    fun relative(instant: java.time.Instant): String {
        val seconds = java.time.Duration.between(instant, java.time.Instant.now()).seconds
        if (seconds < 60) return "just now"
        val minutes = seconds / 60
        if (minutes < 60) return plural(minutes, "minute")
        val hours = minutes / 60
        if (hours < 24) return plural(hours, "hour")
        val days = hours / 24
        if (days < 7) return plural(days, "day")
        val weeks = days / 7
        if (weeks < 5) return plural(weeks, "week")
        val months = days / 30
        return if (months < 12) plural(months, "month") else plural(days / 365, "year")
    }

    private fun plural(n: Long, unit: String): String = "$n $unit${if (n == 1L) "" else "s"} ago"

    private val ABSOLUTE: java.time.format.DateTimeFormatter =
        java.time.format.DateTimeFormatter
            .ofLocalizedDateTime(java.time.format.FormatStyle.MEDIUM, java.time.format.FormatStyle.SHORT)
            .withZone(java.time.ZoneId.systemDefault())
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
    private val protocolObj: JSONObject = root.optJSONObject("protocol") ?: JSONObject()

    val merkleRoot: String get() = sig.optString("merkleRoot")
    val targetHash: String get() = sig.optString("targetHash")
    val documentStore: String get() = issuerObj.optString("documentStore")
    val issuerName: String get() = issuerObj.optString("name", "Unknown issuer")
    val issuerDomain: String get() = issuerObj.optString("domain", "")

    /**
     * The ROOT-COVERED issuer identity leaf (`data.issuer`, or `data.credentialSubject.issuer`).
     *
     * Unlike the top-level `issuer` block, this is inside the Merkle root, so it cannot be relabelled
     * without breaking integrity. It is the value `IssuerIdentity.assertDomain` checks the displayed
     * domain against.
     */
    val rootCoveredIssuerLeaf: String?
        get() {
            data.optString("issuer", "").takeIf { it.isNotEmpty() }?.let { return it }
            return data.optJSONObject("credentialSubject")
                ?.optString("issuer", "")
                ?.takeIf { it.isNotEmpty() }
        }
    val recordType: String get() = issuerObj.optString("recordType", "")

    /**
     * `protocol.statusBaseUrl` — the REACHABLE origin the issuer serves its public, PII-free receipt
     * status page from (`<base>/r/<receiptId>`), stamped at issuance from the issuer's
     * `DEPLOYMENT_URL`. Blank when the issuer publishes none, including every pre-stamping document.
     *
     * This is NOT [issuerDomain]: that is a `did:web` identity, a name that need not resolve (the
     * shipped `gov.example` default is RFC-2606 reserved, hence NXDOMAIN). Never substitute one for
     * the other — a QR built from the identity is a dead link wearing a verification affordance.
     */
    val statusBaseUrl: String get() = protocolObj.optString("statusBaseUrl", "").trim().trimEnd('/')

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
