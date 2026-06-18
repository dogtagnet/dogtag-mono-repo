package io.liberalize.dogtag.data

import android.content.Context
import org.json.JSONObject

/**
 * The live ROAX (chainId 135) deployment addresses, loaded from the bundled `roax.json`
 * (contracts/deployments/roax.json). Used as the default config for on-chain reads and for the
 * consent-binding (ConsentKeyRegistry) + verification (VerificationRegistry) flows.
 */
data class RoaxConfig(
    val chainId: Long,
    val dogTagSbt: String,
    val verificationRegistry: String,
    val consentKeyRegistry: String,
    val issuerRegistry: String,
    val poseidon6: String,
) {
    companion object {
        fun load(context: Context): RoaxConfig {
            val json = context.assets.open("roax.json").bufferedReader().use { it.readText() }
            val o = JSONObject(json)
            return RoaxConfig(
                chainId = o.optLong("chainId", 135),
                dogTagSbt = o.optString("DogTagSBT"),
                verificationRegistry = o.optString("VerificationRegistry"),
                consentKeyRegistry = o.optString("ConsentKeyRegistry"),
                issuerRegistry = o.optString("IssuerRegistry"),
                poseidon6 = o.optString("Poseidon6"),
            )
        }
    }
}
