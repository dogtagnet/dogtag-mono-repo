package io.liberalize.dogtag.data

import android.content.Context
import org.json.JSONObject

/**
 * The live ROAX deployment addresses, loaded from the bundled `roax.json`, which is GENERATED from
 * `contracts/deployments/roax.json` by `scripts/gen-mobile-roax-config.sh` and gitignored. Used as
 * the default config for on-chain reads and for the owner-hidden issuance and verification reads.
 *
 * COMPILE-TIME. `roax.json` ships inside the APK assets, so editing that file is NOT a repoint — it
 * is the prerequisite for a build that is one, and nothing between those two states says which you
 * are in. An installed old build keeps reading the addresses baked into it, so a full redeploy ends
 * in a mobile rebuild AND reinstall; that is standing process, not a caveat.
 *
 * EVERY FIELD FAILS CLOSED BLANK, and that is what makes an absent or stale bundle safe. The keys
 * below are the LEDGER's key names. A bundle generated before that rename carries the old ones, so
 * every address here reads "" and each consumer reports could-not-check — the whitelist pillar
 * answers `Unknown`, the anchor read fails closed — rather than silently dialling the retired
 * contracts such a bundle names. `make check-addresses` is the gate that keeps the file out of the
 * tree. (The JSON itself can carry no comment saying this; its `_` key does.)
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
     * The clone's on-chain domain claim. ALWAYS BLANK today, deliberately: `IssuerDomainRegistry`
     * has no source in this repo and no launch-set deployment, so the generator writes no key for it
     * and the binding reports "could not read" — never "no domain claimed", which would assert a
     * read nobody made. `ServiceDomainResolver` is the launch set's domain surface; wiring this to it
     * is a client repoint, not a configuration change.
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
                dogTagSbt = o.optString("DogTagSBTConsent"),
                // What this field NAMES on the launch set is `ProviderRegistry`, which is also what
                // `DogTagIssuerFactory.registry()` answers. The Kotlin name is kept because the
                // pillar's own vocabulary is "issuer registry"; only its VALUE is dialled, and only
                // for the VERIFY-axis `isWhitelistedFor(VERIFY:<purpose>, relayer)` read — an
                // `eth_call` carries no `from`, so `ProviderRegistry` answers that off
                // `_verifierCapabilities`, which is the axis and the mapping the question means.
                // The issuance-axis authority is NOT this value: it comes off the clone's own
                // `registry()` (see `RoaxRpc.whitelistedAtIssuance`).
                issuerRegistry = o.optString("ProviderRegistry"),
                issuerFactory = o.optString("DogTagIssuerFactory"),
                issuerDomainRegistry = o.optString("IssuerDomainRegistry"),
                protocolRegistry = o.optString("ProtocolRegistry"),
            )
        }
    }
}
