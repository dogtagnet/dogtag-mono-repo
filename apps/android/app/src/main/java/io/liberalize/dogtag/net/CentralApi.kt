package io.liberalize.dogtag.net

import io.liberalize.dogtag.data.AppConfig
import io.liberalize.dogtag.data.Pet
import org.json.JSONObject

/** Typed client for the central (admin) API: pet sync + the verify-consent relay. */
object CentralApi {

    /**
     * GET /v1/pets — the owner's pets. Requires an owner session token; returns an empty list (not an
     * error) when there is no session yet, so screens just show the empty state.
     */
    suspend fun listPets(sessionToken: String?): List<Pet> {
        if (sessionToken.isNullOrBlank()) return emptyList()
        return try {
            val resp = Http.getJson("${AppConfig.CENTRAL_API}/v1/pets", bearer = sessionToken)
            if (!resp.ok) return emptyList()
            val arr = JSONObject(resp.body).optJSONArray("pets") ?: return emptyList()
            (0 until arr.length()).mapNotNull { i ->
                runCatching { Pet.fromCentral(arr.getJSONObject(i)) }.getOrNull()
            }.filter { it.dogTagId.isNotBlank() }
        } catch (e: Exception) {
            emptyList()
        }
    }

    /** POST /v1/verify/consent — relay the signed consent. Owner-session gated server-side. */
    suspend fun postConsent(sessionToken: String?, payloadJson: String): Http.Response =
        Http.postJson("${AppConfig.CENTRAL_API}/v1/verify/consent", payloadJson, bearer = sessionToken)
}
