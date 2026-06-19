package io.liberalize.dogtag.net

import io.liberalize.dogtag.data.AppConfig
import io.liberalize.dogtag.data.Pet
import org.json.JSONObject

/** Typed client for the central (admin) API: pet sync + the export-consent relay. */
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

    /**
     * The export-session metadata resolved from the QR's one-time token. The phone GETs this
     * (non-consuming) before proving so it can assert the groomer address, run the whitelist + DNS
     * checks, and build the consent. The token is consumed only on submit.
     */
    data class ExportSession(
        val sessionId: String,
        val relayer: String,
        val purpose: String,
        val recordType: String,
        val challenge: String,
        val mode: String,
    )

    /** GET <host>/x/<token> → export-session metadata (non-consuming). Null on failure. */
    suspend fun resolveExportSession(host: String, token: String): ExportSession? {
        if (token.isBlank()) return null
        return try {
            val resp = Http.getJson("$host/x/$token")
            if (!resp.ok) return null
            val o = JSONObject(resp.body)
            ExportSession(
                sessionId = o.optString("sessionId", o.optString("session_id", "")),
                relayer = o.optString("relayer", ""),
                purpose = o.optString("purpose", ""),
                recordType = o.optString("recordType", o.optString("record_type", "")),
                challenge = o.optString("challenge", ""),
                mode = o.optString("mode", "zk"),
            )
        } catch (e: Exception) {
            null
        }
    }

    /**
     * ZK path: POST the proof bundle directly to the GROOMER host (the scanned QR origin), NOT central.
     * The groomer relays `recordVerificationZK` on-chain as the gas-payer. The body carries the one-time
     * `exportToken` (consumed server-side on submit) plus `{consent, sig, mode, proof, bind}`.
     */
    suspend fun postVerifyConsentToHost(host: String, payloadJson: String): Http.Response =
        Http.postJson("$host/v1/verify/consent", payloadJson)

    /**
     * Poll the export-session status on the GROOMER host: GET /verify/session/{id}?token=<token>.
     * Returns the parsed `{status, txHash}` (or null on failure). Status flips to `recorded` once the
     * relayer's `recordVerificationZK` tx confirms.
     */
    data class SessionStatus(val status: String, val txHash: String?)

    suspend fun verifySessionStatus(host: String, sessionId: String, token: String): SessionStatus? {
        if (sessionId.isBlank()) return null
        return try {
            val resp = Http.getJson("$host/verify/session/$sessionId?token=$token")
            if (!resp.ok) return null
            val o = JSONObject(resp.body)
            SessionStatus(
                status = o.optString("status", ""),
                txHash = o.optString("txHash", o.optString("tx_hash", "")).ifBlank { null },
            )
        } catch (e: Exception) {
            null
        }
    }
}
