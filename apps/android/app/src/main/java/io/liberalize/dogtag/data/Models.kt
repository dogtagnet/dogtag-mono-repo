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
}
