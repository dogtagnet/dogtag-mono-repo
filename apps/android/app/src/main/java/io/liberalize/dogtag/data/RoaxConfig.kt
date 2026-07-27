package io.liberalize.dogtag.data

import android.content.Context
import org.json.JSONObject

/**
 * The live ROAX (chainId 135) deployment addresses, loaded from the bundled `roax.json`
 * (contracts/deployments/roax.json). Used as the default config for on-chain reads and for the
 * owner-hidden issuance and verification reads.
 */
data class RoaxConfig(
    val chainId: Long,
    val dogTagSbt: String,
    val issuerRegistry: String,
    /**
     * `DogTagIssuerFactory` — the app's OWN anchor for "which contract issued this root?". Credential
     * verification resolves the issuing clone from its write-once `rootIssuer[R]` index instead of
     * trusting a document's `issuer.documentStore`, which sits outside the Merkle root.
     */
    val dogTagIssuerFactory: String,
    /** Protocol discovery trust anchor. Blank fails closed until the registry is deployed. */
    val protocolRegistry: String,
) {
    companion object {
        fun load(context: Context): RoaxConfig {
            val json = context.assets.open("roax.json").bufferedReader().use { it.readText() }
            val o = JSONObject(json)
            return RoaxConfig(
                chainId = o.optLong("chainId", 135),
                dogTagSbt = o.optString("DogTagSBT"),
                issuerRegistry = o.optString("IssuerRegistry"),
                dogTagIssuerFactory = o.optString("DogTagIssuerFactory"),
                protocolRegistry = o.optString("ProtocolRegistry"),
            )
        }
    }
}
