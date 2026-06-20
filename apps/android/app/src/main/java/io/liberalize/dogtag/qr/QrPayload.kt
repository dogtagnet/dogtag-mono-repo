package io.liberalize.dogtag.qr

import android.net.Uri
import android.util.Base64
import org.json.JSONObject

/**
 * The two scan outcomes the user app supports. The pet owner's app ONLY scans — it never shows a QR.
 * The vet/groomer is the party that DISPLAYS a one-time-token QR; we detect which kind it is by the URL
 * shape (architecture §7, impl §3.9 / §6.5).
 *
 *  - Import (issuer -> user, SHORT token): `https://<vet-host>/r/<32hex>` — preferred, low-density QR.
 *  - Import (issuer -> user, legacy JWT): `https://<vet-host>/r?t=<jwt>&i=<recordId>` (back-compat).
 *  - Export (user -> groomer, SHORT token): `https://<host>/x/<token>?a=<relayerAddr>` — one-time token +
 *    groomer wallet address. The phone resolves `GET <host>/x/<token>` for the session metadata, DNS-
 *    verifies the groomer (prod/remote), proves on-device, and POSTs the proof using the token.
 */
sealed class QrPayload {
    /** A vet/groomer record link — fetch GET <host>/records/{recordId} with the Bearer JWT and import. */
    data class ImportRecord(val host: String, val recordId: String, val jwt: String) : QrPayload()

    /**
     * A vet/groomer SHORT one-time share token — fetch GET <host>/r/<token> (no Bearer) and import.
     * The server resolves the token to the wrapped doc and deletes it (one-time, low-density QR).
     */
    data class ImportRecordToken(val host: String, val token: String) : QrPayload()

    /**
     * An export-session — the groomer requests the owner present a record. The QR is a SHORT one-time
     * token plus the groomer's wallet/relayer address: `/x/<token>?a=<addr>`. The session metadata
     * (relayer/purpose/recordType/challenge/mode/sessionId) is fetched non-consuming from `/x/<token>`.
     */
    data class ExportSession(
        val host: String,
        val token: String,
        val groomerAddr: String,
    ) : QrPayload()

    /**
     * A dog-tag ISSUANCE session — the vet displays `/p/<token>` (token = 32 hex, one-time, 180s).
     * The phone POSTs `<host>/profiles/issue/bind { token, walletAddress, signature }` to bind the
     * dog tag to this wallet, then verifies the returned DOG_PROFILE against the DogTagSBT and stores
     * it as a credential. (Optional non-consuming pre-step: GET `<host>/p/<token>`.)
     */
    data class DogTagIssueSession(val host: String, val token: String) : QrPayload()

    /** Anything we don't recognise. */
    data class Unknown(val raw: String) : QrPayload()

    companion object {
        fun parse(raw: String): QrPayload {
            val trimmed = raw.trim()
            return try {
                val uri = Uri.parse(trimmed)
                val origin = "${uri.scheme}://${uri.authority}"
                val path = uri.path?.trimEnd('/')
                val segs = uri.pathSegments
                when {
                    // SHORT one-time share token: `/r/<token>` (no query string). Preferred.
                    segs.size == 2 && segs[0] == "r" && uri.query.isNullOrBlank() -> {
                        val token = segs[1]
                        if (token.isNotBlank()) ImportRecordToken(origin, token)
                        else Unknown(trimmed)
                    }
                    // Export session one-time token: `/x/<token>?a=<groomerAddr>`.
                    segs.size == 2 && segs[0] == "x" -> {
                        val token = segs[1]
                        val addr = uri.getQueryParameter("a").orEmpty()
                        if (token.isNotBlank() && addr.isNotBlank()) ExportSession(origin, token, addr)
                        else Unknown(trimmed)
                    }
                    // Dog-tag issuance one-time token: `/p/<token>` (no query string).
                    segs.size == 2 && segs[0] == "p" && uri.query.isNullOrBlank() -> {
                        val token = segs[1]
                        if (token.isNotBlank()) DogTagIssueSession(origin, token)
                        else Unknown(trimmed)
                    }
                    // Legacy embedded record-JWT: `/r?t=<jwt>&i=<recordId>` (back-compat).
                    path == "/r" -> {
                        val t = uri.getQueryParameter("t").orEmpty()
                        val i = uri.getQueryParameter("i").orEmpty()
                        if (t.isNotBlank() && i.isNotBlank()) ImportRecord(origin, i, t)
                        else Unknown(trimmed)
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
