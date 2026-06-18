package io.liberalize.dogtag.data

import android.content.Context

/**
 * Endpoint configuration for the live backends. The central API is the fixed admin domain the mobile
 * app is configured against (architecture §7); per-vet hosts come from scanned QR origins. The session
 * token (set after the user logs in / connects a wallet) is persisted so GET /v1/pets + the consent
 * relay can authenticate as the owner.
 */
object AppConfig {
    const val CENTRAL_API = "https://api.dogtag.io"
    const val ROAX_RPC = "https://devrpc.roax.net"

    private const val PREFS = "dogtag_config"
    private const val KEY_SESSION = "owner_session"

    fun sessionToken(context: Context): String? =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY_SESSION, null)

    fun setSessionToken(context: Context, token: String?) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .apply { if (token == null) remove(KEY_SESSION) else putString(KEY_SESSION, token) }
            .apply()
    }
}
