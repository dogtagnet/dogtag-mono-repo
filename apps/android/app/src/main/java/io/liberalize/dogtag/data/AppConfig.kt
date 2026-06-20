package io.liberalize.dogtag.data

import android.content.Context

/**
 * Endpoint configuration. Per-vet/-groomer hosts always come from scanned QR origins — the device
 * never calls a central admin base for registration or pet sync. The legacy ECDSA export path still
 * reads an optional central base URL + owner session token (read-only here); the ROAX RPC is the
 * on-chain read endpoint.
 */
object AppConfig {
    const val DEFAULT_CENTRAL_API = "https://api.dogtag.io"
    const val ROAX_RPC = "https://devrpc.roax.net"

    /**
     * The TRUSTED PROVER SERVICE base URL (no trailing slash) — used ONLY by 32-bit-only devices that
     * cannot generate a Groth16 proof on-device. They POST `{wrappedDoc, consent, eddsaSig}` to
     * `<proverApiUrl>/prove-verification` and submit the returned proof to the groomer themselves, so
     * the groomer never sees the witness. 64-bit devices prove on-device and never call this.
     *
     * The compiled-in default points at the demo prover-service via its cloudflared tunnel
     * (PROVER_PUBLIC_URL — the phone's network has client isolation, so LAN IP won't reach the Mac).
     * Override at runtime via the `prover_api` pref. Empty string = "no remote prover configured"
     * (a 32-bit device then surfaces a clear error).
     */
    const val DEFAULT_PROVER_API = "https://vertical-emails-escape-speech.trycloudflare.com"

    private const val PREFS = "dogtag_config"
    private const val KEY_SESSION = "owner_session"
    private const val KEY_CENTRAL = "central_api"
    private const val KEY_PROVER = "prover_api"

    /** The configured central API base URL (no trailing slash), or the compiled-in default. */
    fun centralApi(context: Context): String {
        val v = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY_CENTRAL, null)
        return (v?.trim()?.trimEnd('/')?.ifBlank { null }) ?: DEFAULT_CENTRAL_API
    }

    /**
     * The configured prover-service base URL (no trailing slash), or the compiled-in default. Blank
     * means no remote prover is configured (only relevant on 32-bit-only devices).
     */
    fun proverApiUrl(context: Context): String {
        val v = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY_PROVER, null)
        return (v?.trim()?.trimEnd('/')?.ifBlank { null }) ?: DEFAULT_PROVER_API
    }

    fun sessionToken(context: Context): String? =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY_SESSION, null)
}
