package io.liberalize.dogtag.qr

import android.net.Uri
import android.util.Base64
import org.json.JSONObject

/**
 * The two scan outcomes the user app supports. The pet owner's app ONLY scans — it never shows a QR.
 * The vet/groomer is the party that DISPLAYS a one-time-JWT QR; we detect which kind it is by the URL
 * shape (architecture §7, impl §3.9 / §6.5).
 *
 *  - Import (issuer -> user): `https://<vet-host>/r?t=<jwt>&i=<recordId>`
 *  - Verify (verifier -> user): `https://<host>/v?t=<jwt>` (JWT carries relayer/purpose/challenge/recordType)
 */
sealed class QrPayload {
    /** A vet/groomer record link — fetch GET <host>/records/{recordId} with the Bearer JWT and import. */
    data class ImportRecord(val host: String, val recordId: String, val jwt: String) : QrPayload()

    /** A verify-session — show the request, let the user pick a stored record, sign + relay consent. */
    data class VerifySession(
        val host: String,
        val jwt: String,
        val relayer: String,
        val purpose: String,
        val recordType: String,
        val challenge: String,
        val mode: String,        // "zk" | "normal"
        val sessionId: String,
    ) : QrPayload()

    /** Anything we don't recognise. */
    data class Unknown(val raw: String) : QrPayload()

    companion object {
        fun parse(raw: String): QrPayload {
            val trimmed = raw.trim()
            return try {
                val uri = Uri.parse(trimmed)
                val origin = "${uri.scheme}://${uri.authority}"
                when (uri.path?.trimEnd('/')) {
                    "/r" -> {
                        val t = uri.getQueryParameter("t").orEmpty()
                        val i = uri.getQueryParameter("i").orEmpty()
                        if (t.isNotBlank() && i.isNotBlank()) ImportRecord(origin, i, t)
                        else Unknown(trimmed)
                    }
                    "/v" -> {
                        val t = uri.getQueryParameter("t").orEmpty()
                        if (t.isBlank()) return Unknown(trimmed)
                        val claims = decodeJwtClaims(t)
                        VerifySession(
                            host = origin,
                            jwt = t,
                            relayer = claims.optString("relayer", ""),
                            purpose = claims.optString("purpose", ""),
                            recordType = claims.optString("recordType", claims.optString("record_type", "")),
                            challenge = claims.optString("challenge", ""),
                            mode = claims.optString("mode", "zk"),
                            sessionId = claims.optString("sub", ""),
                        )
                    }
                    else -> Unknown(trimmed)
                }
            } catch (e: Exception) {
                Unknown(trimmed)
            }
        }

        /** Decode the (untrusted) JWT payload to read claims for display + consent fields. */
        fun decodeJwtClaims(jwt: String): JSONObject {
            return try {
                val parts = jwt.split(".")
                if (parts.size < 2) return JSONObject()
                val payload = Base64.decode(parts[1], Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
                JSONObject(String(payload))
            } catch (e: Exception) {
                JSONObject()
            }
        }
    }
}
