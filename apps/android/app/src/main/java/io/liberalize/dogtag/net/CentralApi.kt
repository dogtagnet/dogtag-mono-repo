package io.liberalize.dogtag.net

import io.liberalize.dogtag.profile.BackedUpAttribute
import org.json.JSONArray
import org.json.JSONObject

/**
 * Typed client for the scanned vet/groomer host. Issuance sends only the device-built owner-hidden
 * profile root; verification sends only the owner-hidden consent proof.
 */
object CentralApi {

    data class WeightEntry(val unit: String, val value: String, val measuredOn: String)

    data class Microchip(
        val code: String,
        val standard: String,
        val implantDate: String,
        val bodyLocation: String,
    )

    data class PetProfile(
        val name: String,
        val species: String,
        val breedVbo: String,
        val breedLabel: String,
        val sex: String,
        val neuterStatus: String,
        val dateOfBirth: String,
        val weightHistory: List<WeightEntry>,
        val microchip: Microchip,
    ) {
        /**
         * The pet-profile leaves committed into R, each with a fresh device-random salt. The
         * owner-control triple is added by the Rust profile-tree builder; the D1 identity leaves
         * come from [ProfileIssueSession.identityLeaves] with VET-generated salts and are appended
         * AFTER these by the issuance flow.
         */
        fun attributes(salt: () -> String): List<BackedUpAttribute> {
            val out = mutableListOf<BackedUpAttribute>()
            fun add(path: String, value: String, tag: UByte = 2u) {
                if (value.isNotBlank()) out += BackedUpAttribute(path, salt(), tag, value)
            }
            add("credentialSubject.name", name)
            add("credentialSubject.species", species)
            add("credentialSubject.breedVbo", breedVbo)
            add("credentialSubject.breedLabel", breedLabel)
            add("credentialSubject.sex", sex)
            add("credentialSubject.neuterStatus", neuterStatus)
            add("credentialSubject.dateOfBirth", dateOfBirth)
            weightHistory.forEachIndexed { i, weight ->
                add("credentialSubject.weightHistory[$i].unit", weight.unit)
                add("credentialSubject.weightHistory[$i].value", weight.value, 4u)
                add("credentialSubject.weightHistory[$i].measuredOn", weight.measuredOn)
            }
            add("credentialSubject.microchip.code", microchip.code)
            add("credentialSubject.microchip.standard", microchip.standard)
            add("credentialSubject.microchip.implantDate", microchip.implantDate)
            add("credentialSubject.microchip.bodyLocation", microchip.bodyLocation)
            return out
        }

        /** Compare metadata on an issuance retry without comparing freshly generated salts. */
        fun matches(attributes: List<BackedUpAttribute>): Boolean {
            val expected = attributes { "ignored" }.map { Triple(it.keyPath, it.tag, it.value) }
            val stored = attributes.map { Triple(it.keyPath, it.tag, it.value) }
            return expected == stored
        }
    }

    /**
     * D1: one vet-salted identity attribute leaf the device MUST fold into R alongside the pet
     * attributes. The salt is the VET's - at bind it requires the posted identity openings to
     * EXACTLY match its own stored set while rebuilding R from the full leaf list, refusing the
     * mint otherwise - and the device persists the triple as a disclosure opening.
     */
    data class IdentityLeaf(
        val keyPath: String,
        val saltHex: String,
        val tag: UByte,
        val value: String,
    ) {
        fun asAttribute(): BackedUpAttribute = BackedUpAttribute(keyPath, saltHex, tag, value)
    }

    data class ProfileIssueSession(
        val sessionId: String,
        val dogTagId: String,
        val pet: PetProfile,
        /** The vet-collected identity block, shown to the owner before binding. */
        val ownerIdentity: Map<String, String>,
        /** D1: the salted identity leaves to fold into R (empty when the vet collected none). */
        val identityLeaves: List<IdentityLeaf> = emptyList(),
    ) {
        /**
         * Compare metadata on an issuance retry: pet attrs on (keyPath, tag, value) - their salts
         * are device-random - and identity leaves INCLUDING the vet salt, which the bind-time
         * integrity gate recomputes with. Order is pet-then-identity, matching the build path.
         */
        fun matchesStored(attributes: List<BackedUpAttribute>): Boolean {
            val petExpected = pet.attributes { "ignored" }.map { Triple(it.keyPath, it.tag, it.value) }
            if (attributes.size != petExpected.size + identityLeaves.size) return false
            val petStored = attributes.take(petExpected.size).map { Triple(it.keyPath, it.tag, it.value) }
            val identityStored = attributes.drop(petExpected.size)
            return petStored == petExpected && identityStored == identityLeaves.map { it.asAttribute() }
        }
    }

    data class CustodialBind(
        val dogTagId: String,
        val root: String,
        val status: String,
        val txHash: String?,
    )

    class CustodialBindRejectedException(code: Int, body: String) : IllegalStateException(
        "custodial bind rejected ($code): ${body.take(160)}",
    )

    /** Resolve the scanned `/p/<token>` before constructing the device-private profile tree. */
    suspend fun resolveProfileIssueSession(host: String, token: String): ProfileIssueSession? {
        if (token.isBlank()) return null
        return try {
            val response = Http.getJson("$host/p/$token")
            if (!response.ok) null else parseProfileIssueSession(response.body)
        } catch (_: Exception) {
            null
        }
    }

    /**
     * POST `{token, root, leaves, reservedLeafHashes}`. No owner address or signature crosses this
     * boundary. `leaves` opens EVERY attribute leaf of the tree (pet and identity alike) and
     * `reservedLeafHashes` names the owner-control triple's leaf hashes OPAQUELY - the vet rebuilds
     * `R` from them (the D1 full-leaf-list attestation-integrity gate) and refuses the mint unless
     * it commits exactly the vet-attested identity. The reserved leaves' PREIMAGES (owner address,
     * consent key, owner secret) never cross this boundary.
     */
    suspend fun bindCustodialIssue(
        host: String,
        token: String,
        root: String,
        leaves: List<BackedUpAttribute>,
        reservedLeafHashes: List<String>,
    ): CustodialBind? {
        if (token.isBlank() || root.isBlank()) return null
        val body = JSONObject()
            .put("token", token)
            .put("root", root)
            .put(
                "leaves",
                JSONArray().apply {
                    leaves.forEach { l ->
                        put(
                            JSONObject()
                                .put("keyPath", l.keyPath)
                                .put("saltHex", l.saltHex)
                                .put("tag", l.tag.toInt())
                                .put("value", l.value),
                        )
                    }
                },
            )
            .put("reservedLeafHashes", JSONArray(reservedLeafHashes))
            .toString()
        return try {
            val response = Http.postJson(
                "$host/profiles/issue/custodial-bind",
                body,
                readTimeoutMs = 20_000,
            )
            if (!response.ok) {
                if (response.code == 410) return null
                throw CustodialBindRejectedException(response.code, response.body)
            }
            val o = JSONObject(response.body)
            CustodialBind(
                dogTagId = o.string("dogTagId", "dog_tag_id"),
                root = o.string("root", "R"),
                status = o.optString("status", ""),
                txHash = o.string("txHash", "tx_hash").ifBlank { null },
            )
        } catch (rejected: CustodialBindRejectedException) {
            throw rejected
        } catch (_: Exception) {
            null
        }
    }

    /** Raw, platform-provided discovery hints. They are checked against ProtocolRegistry on-chain. */
    data class UnverifiedClaims(
        val protocolVersion: String,
        val chainId: Long,
        val verificationRegistry: String,
        val issuerClone: String,
        val purpose: String,
    )

    data class ExportSession(
        val sessionId: String,
        val relayer: String,
        val purpose: String,
        val recordType: String,
        val claims: UnverifiedClaims? = null,
    )

    /** GET `<host>/x/<token>` → owner-hidden verification session metadata. */
    suspend fun resolveExportSession(host: String, token: String): ExportSession? {
        if (token.isBlank()) return null
        return try {
            val response = Http.getJson("$host/x/$token")
            if (!response.ok) return null
            val o = JSONObject(response.body)
            val claims = o.optJSONObject("unverifiedClaims")?.let { uc ->
                UnverifiedClaims(
                    protocolVersion = uc.optString("protocolVersion", ""),
                    chainId = uc.optLong("chainId", 0L),
                    verificationRegistry = uc.optString("verificationRegistry", ""),
                    issuerClone = uc.optString("issuerClone", ""),
                    purpose = uc.optString("purpose", ""),
                )
            }
            ExportSession(
                sessionId = o.string("sessionId", "session_id"),
                relayer = o.optString("relayer", ""),
                purpose = o.optString("purpose", ""),
                recordType = o.string("recordType", "record_type"),
                claims = claims,
            )
        } catch (_: Exception) {
            null
        }
    }

    /**
     * Submit the one owner-hidden verification proof. `/v1/verify/consent` is the sole canonical
     * route: it accepts the `{exportToken, proof}` payload and resolves the session from the
     * owner's one-time export token (the migration-era `/v1/verify/consent/levelb` alias is gone
     * and 404s).
     */
    suspend fun postVerifyConsentToHost(host: String, payloadJson: String): Http.Response =
        Http.postJson("$host/v1/verify/consent", payloadJson, readTimeoutMs = 20_000)

    data class SessionStatus(val status: String, val txHash: String?)

    suspend fun verifySessionStatus(host: String, sessionId: String, token: String): SessionStatus? {
        if (sessionId.isBlank()) return null
        return try {
            val response = Http.getJson("$host/verify/session/$sessionId?token=$token")
            if (!response.ok) return null
            val o = JSONObject(response.body)
            SessionStatus(
                status = o.optString("status", ""),
                txHash = o.string("txHash", "tx_hash").ifBlank { null },
            )
        } catch (_: Exception) {
            null
        }
    }

    internal fun parseProfileIssueSession(json: String): ProfileIssueSession? = try {
        val root = JSONObject(json)
        val petContainer = root.optJSONObject("pet")
        val profileContainer = petContainer?.optJSONObject("profile") ?: root.optJSONObject("profile")
        // An old backend returned only ids/claims. Issuing against that shape would silently build an
        // owner-triple-only R, so metadata containers are a hard compatibility boundary.
        if (petContainer == null && profileContainer == null) return null
        val owner = root.optJSONObject("ownerIdentity")
            ?: root.optJSONObject("owner_identity")
            ?: return null
        val pet = petContainer ?: JSONObject()
        val profile = profileContainer ?: pet
        fun metadata(vararg keys: String): String =
            sequenceOf(profile, pet, root).map { it.string(*keys) }.firstOrNull { it.isNotBlank() }.orEmpty()
        val microchip = profile.optJSONObject("microchip")
            ?: pet.optJSONObject("microchip")
            ?: root.optJSONObject("microchip")
            ?: JSONObject()
        val weights = profile.optJSONArray("weightHistory")
            ?: pet.optJSONArray("weightHistory")
            ?: root.optJSONArray("weightHistory")
            ?: JSONArray()
        val history = (0 until weights.length()).mapNotNull { i ->
            weights.optJSONObject(i)?.let { w ->
                WeightEntry(
                    unit = w.string("unit"),
                    value = w.valueString("value"),
                    measuredOn = w.string("measuredOn", "measured_on"),
                )
            }
        }
        val ownerFields = buildMap {
            val keys = owner.keys()
            while (keys.hasNext()) {
                val key = keys.next()
                val value = owner.opt(key)
                if (value != null && value != JSONObject.NULL) put(key, value.toString())
            }
        }
        // D1: the vet-salted identity leaves the device must fold into R. A leaf without its
        // keyPath or salt cannot be folded OR proven, so those are dropped at the edge.
        val identityLeavesJson = root.optJSONArray("identityLeaves") ?: JSONArray()
        val identityLeaves = (0 until identityLeavesJson.length()).mapNotNull { i ->
            identityLeavesJson.optJSONObject(i)?.let { leaf ->
                val keyPath = leaf.string("keyPath", "key_path")
                val saltHex = leaf.string("saltHex", "salt_hex")
                if (keyPath.isBlank() || saltHex.isBlank()) return@let null
                IdentityLeaf(
                    keyPath = keyPath,
                    saltHex = saltHex,
                    tag = leaf.optInt("tag", 2).toUByte(),
                    value = leaf.valueString("value"),
                )
            }
        }
        ProfileIssueSession(
            sessionId = root.string("sessionId", "session_id"),
            dogTagId = root.string("dogTagId", "dog_tag_id").ifBlank {
                pet.string("dogTagId", "dog_tag_id")
            },
            pet = PetProfile(
                name = metadata("name", "petName", "pet_name"),
                species = metadata("species"),
                breedVbo = metadata("breedVbo", "breed_vbo"),
                breedLabel = metadata("breedLabel", "breed_label", "breed"),
                sex = metadata("sex"),
                neuterStatus = metadata("neuterStatus", "neuter_status"),
                dateOfBirth = metadata("dateOfBirth", "date_of_birth"),
                weightHistory = history,
                microchip = Microchip(
                    code = microchip.string("code").ifBlank {
                        root.valueString("microchip").takeUnless { it.startsWith("{") }.orEmpty()
                    },
                    standard = microchip.string("standard"),
                    implantDate = microchip.string("implantDate", "implant_date"),
                    bodyLocation = microchip.string("bodyLocation", "body_location"),
                ),
            ),
            ownerIdentity = ownerFields,
            identityLeaves = identityLeaves,
        ).takeIf {
            it.sessionId.isNotBlank() && it.dogTagId.isNotBlank() && it.pet.name.isNotBlank()
        }
    } catch (_: Exception) {
        null
    }

    private fun JSONObject.string(vararg keys: String): String {
        keys.forEach { key ->
            val value = opt(key)
            if (value != null && value != JSONObject.NULL && value !is JSONObject && value !is JSONArray) {
                return value.toString()
            }
        }
        return ""
    }

    private fun JSONObject.valueString(key: String): String {
        val value = opt(key)
        return if (value == null || value == JSONObject.NULL) "" else value.toString()
    }
}
