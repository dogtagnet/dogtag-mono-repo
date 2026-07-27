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
     * `DogTagIssuerFactory` — LINK 1 of the issuer↔domain chain (`isClone`). Blank means clone
     * provenance cannot be checked, and the binding reports "could not read" rather than claiming
     * anything.
     */
    val issuerFactory: String,
    /**
     * `IssuerDomainRegistry` — the clone's on-chain domain claim. Blank until deployed; the binding then
     * reports "could not read", never "no domain claimed".
     */
    val issuerDomainRegistry: String,
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
                issuerFactory = o.optString("DogTagIssuerFactory"),
                issuerDomainRegistry = o.optString("IssuerDomainRegistry"),
                protocolRegistry = o.optString("ProtocolRegistry"),
            )
        }
    }
}
