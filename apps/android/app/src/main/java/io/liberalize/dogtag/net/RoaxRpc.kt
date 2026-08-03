package io.liberalize.dogtag.net

import io.liberalize.dogtag.wallet.Keccak256
import kotlinx.coroutines.CancellationException
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI

/**
 * Read-only JSON-RPC client for the ROAX chain (chainId 135 by default).
 *
 * Used to re-check the issuance pillar: `DogTagIssuer.isValid(bytes32 root)` over the wrapped doc's
 * `issuer.documentStore`. This is a pure `eth_call` (no signing, no gas). The RPC may be unreachable
 * (it returned 502 at design time) — callers treat an RPC failure as an UNKNOWN, never a hard fail.
 */
object RoaxRpc {
    const val DEFAULT_RPC = "https://devrpc.roax.net"

    /** Why an endpoint could not establish the chain whose contract addresses are bundled. */
    sealed class EndpointFailure {
        data object InvalidUrl : EndpointFailure()
        data object Unavailable : EndpointFailure()
        data object InvalidChainIdResponse : EndpointFailure()
        data class WrongChain(val actualChainId: Long) : EndpointFailure()
    }

    /**
     * Route chosen after probing `eth_chainId`. [Unavailable] means even the bundled endpoint failed
     * the guard, so no address-bound read may be sent or trusted.
     */
    sealed class EndpointRoute {
        data object Bundled : EndpointRoute()
        data class Custom(val url: String) : EndpointRoute()
        data class BundledFallback(val customFailure: EndpointFailure) : EndpointRoute()
        data class Unavailable(
            val customFailure: EndpointFailure?,
            val bundledFailure: EndpointFailure,
        ) : EndpointRoute()

        val rpcUrl: String?
            get() = when (this) {
                Bundled, is BundledFallback -> DEFAULT_RPC
                is Custom -> url
                is Unavailable -> null
            }
    }

    /**
     * Check the requested peer immediately before a blockchain read.
     *
     * An invalid, unreachable, malformed, or different-chain custom peer falls back to the bundled
     * peer. The bundled peer is guarded too; if it cannot establish [expectedChainId], callers return
     * their existing unknown/null/false result and no contract request is sent.
     *
     * This prevents accidental cross-chain address use, not dishonest replies: a malicious peer can
     * fabricate `eth_chainId` and every later chain result. The Profile settings copy says so.
     */
    suspend fun endpointRoute(requestedRpcUrl: String, expectedChainId: Long): EndpointRoute =
        endpointRoute(requestedRpcUrl, expectedChainId) { url, body -> Http.postJson(url, body) }

    /** Injectable seam for focused endpoint-routing tests. */
    internal suspend fun endpointRoute(
        requestedRpcUrl: String,
        expectedChainId: Long,
        probe: suspend (String, String) -> Http.Response,
    ): EndpointRoute {
        val candidate = requestedRpcUrl.trim()
        val custom = normalizeRpcUrl(candidate)
        if (candidate.isBlank() || candidate == DEFAULT_RPC) {
            val bundledFailure = chainFailure(DEFAULT_RPC, expectedChainId, probe)
            return if (bundledFailure == null) {
                EndpointRoute.Bundled
            } else {
                EndpointRoute.Unavailable(null, bundledFailure)
            }
        }

        val customFailure = if (custom == null) {
            EndpointFailure.InvalidUrl
        } else {
            chainFailure(custom, expectedChainId, probe)
                ?: return EndpointRoute.Custom(custom)
        }
        val bundledFailure = chainFailure(DEFAULT_RPC, expectedChainId, probe)
        return if (bundledFailure == null) {
            EndpointRoute.BundledFallback(customFailure)
        } else {
            EndpointRoute.Unavailable(customFailure, bundledFailure)
        }
    }

    /** Only HTTP(S) JSON-RPC URLs with an authority are usable by [Http]. */
    internal fun normalizeRpcUrl(value: String): String? = runCatching {
        val trimmed = value.trim()
        val uri = URI(trimmed)
        val scheme = uri.scheme?.lowercase()
        if ((scheme != "http" && scheme != "https") || uri.host.isNullOrBlank() || uri.fragment != null) {
            null
        } else {
            trimmed
        }
    }.getOrNull()

    /** Probe `eth_chainId`; only this method may contact a peer before the chain guard passes. */
    private suspend fun chainFailure(
        rpcUrl: String,
        expectedChainId: Long,
        probe: suspend (String, String) -> Http.Response,
    ): EndpointFailure? {
        val payload = JSONObject().apply {
            put("jsonrpc", "2.0")
            put("id", 1)
            put("method", "eth_chainId")
            put("params", JSONArray())
        }.toString()
        return try {
            val resp = probe(rpcUrl, payload)
            if (!resp.ok) return EndpointFailure.Unavailable
            val body = JSONObject(resp.body)
            if (body.has("error")) return EndpointFailure.InvalidChainIdResponse
            val actual = parseChainId(body.optString("result", ""))
                ?: return EndpointFailure.InvalidChainIdResponse
            if (actual == expectedChainId) null else EndpointFailure.WrongChain(actual)
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            EndpointFailure.Unavailable
        }
    }

    internal fun parseChainId(value: String): Long? = runCatching {
        if (!value.startsWith("0x", ignoreCase = true)) return@runCatching null
        val hex = value.removePrefix("0x").removePrefix("0X")
        if (hex.isBlank() || hex.any { it !in '0'..'9' && it.lowercaseChar() !in 'a'..'f' }) {
            return@runCatching null
        }
        java.math.BigInteger(hex, 16).takeIf { it.signum() >= 0 && it.bitLength() <= 63 }?.toLong()
    }.getOrNull()

    /**
     * The single raw transport seam for ALL blockchain JSON-RPC reads below.
     *
     * A custom peer that passes `eth_chainId` but disappears before the actual read gets one guarded
     * retry on the bundled endpoint. Central/provider/indexer and QR-discovered role APIs never enter
     * this function and remain deliberately unaffected by the user's chain setting.
     */
    private suspend fun guardedPostJson(
        requestedRpcUrl: String,
        expectedChainId: Long,
        body: String,
    ): Http.Response = guardedPostJson(requestedRpcUrl, expectedChainId, body) { url, payload ->
        Http.postJson(url, payload)
    }

    /** Test seam that proves wrong-chain peers receive no contract call. */
    internal suspend fun guardedPostJson(
        requestedRpcUrl: String,
        expectedChainId: Long,
        body: String,
        post: suspend (String, String) -> Http.Response,
    ): Http.Response {
        val route = endpointRoute(requestedRpcUrl, expectedChainId, post)
        val selected = route.rpcUrl
            ?: return Http.Response(-1, "chain guard could not establish the bundled chain")
        val response = postOrFailure(selected, body, post)
        if (route !is EndpointRoute.Custom || response.ok) return response

        // The custom peer passed its guard but failed the actual request. Probe the bundled peer
        // immediately before retrying; never turn transport fallback into an unguarded contract read.
        val fallback = endpointRoute(DEFAULT_RPC, expectedChainId, post).rpcUrl
            ?: return Http.Response(-1, "chain guard could not establish the bundled chain")
        return postOrFailure(fallback, body, post)
    }

    /** Normalize transport exceptions into the response failure path without swallowing cancellation. */
    private suspend fun postOrFailure(
        url: String,
        body: String,
        post: suspend (String, String) -> Http.Response,
    ): Http.Response = try {
        post(url, body)
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Exception) {
        Http.Response(-1, "rpc transport failed")
    }

    internal enum class ProfileRootObservation { Pending, Matched, Mismatch }

    /** 4-byte function selector = first 4 bytes of keccak256(canonical signature), `0x`-prefixed hex. */
    internal fun functionSelector(signature: String): String =
        "0x" + Keccak256.digest(signature.toByteArray(Charsets.US_ASCII)).take(4)
            .joinToString("") { "%02x".format(it) }

    /**
     * `topic0` = the FULL 32-byte keccak256 of an event's canonical signature, `0x`-prefixed hex.
     *
     * A separate helper from [functionSelector] on purpose: a call selector is the first FOUR bytes
     * and a log topic is all thirty-two, so reaching for the selector here yields a topic that matches
     * no log at all - which reads exactly like "this pair was never granted" and would turn a genuine
     * credential into a definite refusal. Canonical signature form: no spaces, no parameter names, no
     * `indexed`. `RoaxRpcSelectorTest` pins each derived value.
     */
    internal fun eventTopic(signature: String): String =
        "0x" + Keccak256.digest(signature.toByteArray(Charsets.US_ASCII))
            .joinToString("") { "%02x".format(it) }

    /**
     * Selectors for every `eth_call` below, DERIVED from the canonical signature rather than
     * hard-coded, so they cannot drift away from the deployed contracts.
     *
     * A hard-coded selector can silently disagree with the chain: `isValid` once shipped as the
     * literal `0x6d04f0bc`, whose comment *claimed* to be keccak256("isValid(bytes32)")[:4] but
     * wasn't. That selector REVERTS on the deployed ROAX clone, so every validity read fell through
     * to Unknown (accept-with-caveat) and a revoked credential never surfaced as revoked. The
     * canonical value is 0x6a938567 - what viem, the Rust/alloy ABI, the vet-api `verify_credential`
     * handler and the web direct-RPC path all bind. `RoaxRpcSelectorTest` pins each value here.
     */
    private val IS_VALID_SELECTOR = functionSelector("isValid(bytes32)")
    private val ISSUED_BY_SELECTOR = functionSelector("issuedBy(bytes32)")
    private val RECORD_TYPE_SELECTOR = functionSelector("recordType()")
    private val IS_WHITELISTED_FOR_SELECTOR = functionSelector("isWhitelistedFor(bytes32,address)")

    /**
     * The ONE registry whose `_wl` mapping the clone's `onlyWhitelisted` consults, and therefore the
     * only authority whose grant log can answer for that contract's issuances. Written once in
     * `initialize` from the factory's own `immutable registry`, with no setter.
     */
    internal val REGISTRY_SELECTOR = functionSelector("registry()")


    /**
     * Topics for the grant history the issuer-whitelist pillar folds. Both event arguments are
     * `indexed` on `IssuerRegistry`, so one filtered `eth_getLogs` per `(recordType, signer)` pair
     * reconstructs the whole history - and `whitelistFor`/`delistFor` emit unconditionally, so the
     * log is complete rather than edge-triggered.
     */
    internal val ROOT_ISSUED_TOPIC = eventTopic("RootIssued(bytes32,address,uint256)")
    /**
     * The authority's issuance-grant event. BOTH leading args are indexed, so one filtered
     * `eth_getLogs` on `(service, signer)` reconstructs the whole history; `allowed` is the one
     * NON-indexed argument, so grant and withdrawal arrive on this single topic and are told apart
     * by the log's DATA word rather than by which topic matched.
     *
     * A topic is the full 32-byte keccak, never a 4-byte selector: the shorter value matches no log
     * at all, which is indistinguishable from "never granted" and would refuse every credential.
     */
    internal val ISSUANCE_CAPABILITY_SET_TOPIC =
        eventTopic("IssuanceCapabilitySet(address,address,bool)")
    private val CONSUMED_SELECTOR = functionSelector("consumed(bytes32)")
    private val PROFILE_ROOT_SELECTOR = functionSelector("profileRoot(uint256)")

    // ---- the issuer↔domain binding chain ------------------------------------------------------
    private val IS_CLONE_SELECTOR = functionSelector("isClone(address)")
    private val DOMAIN_OF_SELECTOR = functionSelector("domainOf(address)")
    private val ISSUER_NAME_SELECTOR = functionSelector("name()")
    private val ROOT_ISSUER_SELECTOR = functionSelector("rootIssuer(bytes32)")

    sealed class Result {
        object Valid : Result()
        object Invalid : Result()
        data class Unknown(val reason: String) : Result()
    }

    /**
     * The outcome of a read whose on-chain answer is an address or a word.
     *
     * Three outcomes, never two: [Unset] is the chain ANSWERING with its zero value (nobody ever wrote
     * that slot), while [Unresolved] is the read not answering at all - a transport failure, a revert,
     * or a reply too short to hold the value. Collapsing the pair into one `null` is how a dropped
     * connection came to be reported to the holder as the definite chain fact "no factory clone ever
     * issued this root". Both remain indeterminate and neither can ever become a pass; only what the
     * owner is told differs. Mirrors iOS `RoaxRpc.HexRead`.
     */
    sealed class HexRead {
        data class Found(val hex: String) : HexRead()
        object Unset : HexRead()
        data class Unresolved(val reason: String) : HexRead()
    }

    /**
     * Where a mined log sits. Ordered by `(blockNumber, logIndex)`; `logIndex` is BLOCK-SCOPED and so
     * comparable ACROSS contracts within one block, which is the only reason a registry grant and a
     * clone's issuance landing in the same block can be sequenced against each other at all.
     *
     * Mirrors Swift `RoaxRpc.LogPoint` and Rust `dogtag_standard::verify::LogPoint`.
     */
    data class LogPoint(val blockNumber: Long, val logIndex: Long) : Comparable<LogPoint> {
        override fun compareTo(other: LogPoint): Int =
            if (blockNumber != other.blockNumber) blockNumber.compareTo(other.blockNumber)
            else logIndex.compareTo(other.logIndex)
    }

    /** One `IssuerRegistry.whitelistFor`/`delistFor` call, as observed in that registry's own log. */
    data class GrantEvent(val at: LogPoint, val granted: Boolean)

    /**
     * Did the issuing signer hold the capability AT THE MOMENT this root was anchored?
     *
     * [Authorized] and [NotAuthorized] are answers ABOUT the credential; [Undetermined] says the
     * question could not be put. Only the first may contribute to a pass, only the second may refuse,
     * and the third must never be rendered as either.
     */
    enum class GrantAtIssuance { Authorized, NotAuthorized, Undetermined }

    /**
     * Fold one `(recordType, signer)` grant history against the point a root was anchored.
     *
     * THE definition of the rule for this app, mirroring Rust `grant_in_force_at`, TS `grantInForceAt`
     * and Swift `RoaxRpc.grantInForceAt`: the state as of the anchoring point is the LAST event at or
     * before it.
     *
     * An EMPTY prior history is [GrantAtIssuance.NotAuthorized], not [GrantAtIssuance.Undetermined]:
     * the registry answered and its own log records no grant, which is evidence about the credential
     * rather than about our ability to check. A log read that FAILED never reaches this function.
     *
     * The tie is broken EXPLICITLY on `>=`, taking the LAST of any events sharing one `(blockNumber,
     * logIndex)`, because that is what Rust's `max_by_key` and the TS sort-then-last do. A conforming
     * chain cannot produce such a pair - `logIndex` is unique within a block - so this only ever
     * arises from a lying or buggy peer, where either answer is equally arbitrary; the point is that
     * one rule mirrored four ways must not quietly be four rules. `maxByOrNull` would return the
     * FIRST maximum, so the divergence would be invisible in every language's own tests.
     */
    internal fun grantInForceAt(history: List<GrantEvent>, anchoredAt: LogPoint): GrantAtIssuance {
        val asOf = history
            .filter { it.at <= anchoredAt }
            .fold(null as GrantEvent?) { best, e -> if (best == null || e.at >= best.at) e else best }
        return if (asOf?.granted == true) GrantAtIssuance.Authorized else GrantAtIssuance.NotAuthorized
    }

    /**
     * Call `isValid(root)` on the issuer clone. `documentStore` is the issuer contract address from the
     * wrapped doc; `root` is the 0x.. 32-byte merkleRoot.
     */
    suspend fun isValid(
        rpcUrl: String,
        expectedChainId: Long,
        documentStore: String,
        root: String,
    ): Result {
        if (documentStore.isBlank() || root.isBlank()) return Result.Unknown("missing addr/root")
        val data = IS_VALID_SELECTOR + pad32(root)
        val params = JSONArray().apply {
            put(JSONObject().apply {
                put("to", documentStore)
                put("data", data)
            })
            put("latest")
        }
        val payload = JSONObject().apply {
            put("jsonrpc", "2.0")
            put("id", 1)
            put("method", "eth_call")
            put("params", params)
        }.toString()

        return try {
            val resp = guardedPostJson(rpcUrl, expectedChainId, payload)
            if (!resp.ok) return Result.Unknown("rpc ${resp.code}")
            val o = JSONObject(resp.body)
            if (o.has("error")) return Result.Unknown(o.getJSONObject("error").optString("message", "rpc error"))
            val result = o.optString("result", "")
            // bool return: 32-byte word, last byte 1 == true.
            val hex = result.removePrefix("0x")
            if (hex.isBlank()) return Result.Unknown("empty result")
            val truthy = hex.trimStart('0').isNotEmpty()
            if (truthy) Result.Valid else Result.Invalid
        } catch (e: Exception) {
            Result.Unknown(e.message ?: "rpc unreachable")
        }
    }

    /**
     * `DogTagSBT.profileRoot(dogTagId)` → the on-chain DOG_PROFILE root (0x.. 32-byte), or null on
     * failure. `dogTagId` is the decimal tokenId. This is the SBT anchor used to verify an issued
     * DOG_PROFILE (NOT the DogTagIssuer-clone isValid).
     */
    suspend fun profileRoot(
        rpcUrl: String,
        expectedChainId: Long,
        dogTagSbt: String,
        dogTagId: String,
    ): String? {
        if (dogTagSbt.isBlank() || dogTagId.isBlank()) return null
        val data = PROFILE_ROOT_SELECTOR + padUint(dogTagId)
        return when (val r = ethCall(rpcUrl, expectedChainId, dogTagSbt, data)) {
            is CallResult.Ok -> normalizeBytes32(r.hex)
            is CallResult.Err -> null
        }
    }

    internal fun normalizeBytes32(value: String): String? {
        val raw = if (value.startsWith("0x", ignoreCase = true)) value.substring(2) else value
        if (raw.length != 64 || raw.any { it !in '0'..'9' && it.lowercaseChar() !in 'a'..'f' }) {
            return null
        }
        return "0x${raw.lowercase()}"
    }

    internal fun classifyProfileRoot(chainRoot: String?, expectedRoot: String): ProfileRootObservation {
        val observed = chainRoot?.let(::normalizeBytes32) ?: return ProfileRootObservation.Pending
        if (observed.drop(2).all { it == '0' }) return ProfileRootObservation.Pending
        val expected = normalizeBytes32(expectedRoot) ?: return ProfileRootObservation.Mismatch
        return if (observed == expected) ProfileRootObservation.Matched else ProfileRootObservation.Mismatch
    }

    /**
     * `IssuerRegistry.isWhitelistedFor(key, signer)` — the PRE-PROOF groomer check. `key` is the
     * 0x.. 32-byte VERIFY key (`verifyWhitelistKeyHex(purpose)`); `signer` is the scanned relayer.
     * Returns Valid (whitelisted), Invalid (not), or Unknown (RPC unreachable). On Unknown the caller
     * MUST hard-stop — this gate is a user-safety requirement, so unknown is treated as not-authorized.
     *
     * A reply too short to hold a bool word is the registry NOT ANSWERING (a codeless or wrong address
     * returns `0x` as a SUCCESSFUL call), so it is Unknown - never the definite Invalid that accuses a
     * genuine credential's issuer of being unauthorised. Only a full 32-byte zero word is a real "no".
     */
    suspend fun isWhitelistedFor(
        rpcUrl: String,
        expectedChainId: Long,
        issuerRegistry: String,
        key: String,
        signer: String,
    ): Result {
        if (issuerRegistry.isBlank() || key.isBlank() || signer.isBlank()) {
            return Result.Unknown("missing addr/key/signer")
        }
        val data = IS_WHITELISTED_FOR_SELECTOR + pad32(key) + padAddr(signer)
        return when (val r = ethCall(rpcUrl, expectedChainId, issuerRegistry, data)) {
            is CallResult.Ok ->
                if (r.hex.length < 64) Result.Unknown("the registry returned no answer")
                else if (r.hex.trimStart('0').isNotEmpty()) Result.Valid
                else Result.Invalid
            is CallResult.Err -> Result.Unknown(r.reason)
        }
    }

    /**
     * `DogTagIssuer.issuedBy(root)` → the H-1 originator that actually called `issue(root)` on this
     * clone. [HexRead.Unset] when the clone never issued it (the on-chain zero address),
     * [HexRead.Unresolved] when the read did not answer.
     */
    suspend fun issuedBy(
        rpcUrl: String,
        expectedChainId: Long,
        documentStore: String,
        root: String,
    ): HexRead {
        if (documentStore.isBlank() || root.isBlank()) return HexRead.Unresolved("missing addr/root")
        val data = ISSUED_BY_SELECTOR + pad32(root)
        return when (val r = ethCall(rpcUrl, expectedChainId, documentStore, data)) {
            // address is right-aligned in a 32-byte word; all-zero == never issued here.
            is CallResult.Ok ->
                if (r.hex.length < 40) HexRead.Unresolved("the issuer clone returned no address")
                else if (r.hex.trimStart('0').isEmpty()) HexRead.Unset
                else HexRead.Found("0x" + r.hex.takeLast(40).lowercase())
            is CallResult.Err -> HexRead.Unresolved(r.reason)
        }
    }


    /**
     * `DogTagIssuer.recordType()` → the clone's own immutable record-type key. [HexRead.Unset] when
     * the contract reports the zero word (uninitialized / not a clone), [HexRead.Unresolved] when the
     * read did not answer.
     */
    suspend fun recordTypeOf(
        rpcUrl: String,
        expectedChainId: Long,
        issuerClone: String,
    ): HexRead {
        if (issuerClone.isBlank()) return HexRead.Unresolved("missing issuer clone")
        return when (val r = ethCall(rpcUrl, expectedChainId, issuerClone, RECORD_TYPE_SELECTOR)) {
            is CallResult.Ok -> {
                val word = normalizeBytes32(r.hex)
                when {
                    word == null ->
                        HexRead.Unresolved("the issuer clone returned no record-type word")
                    word.drop(2).all { it == '0' } -> HexRead.Unset
                    else -> HexRead.Found(word)
                }
            }
            is CallResult.Err -> HexRead.Unresolved(r.reason)
        }
    }

    /** `keccak256(recordType utf8)` — the `IssuerRegistry` whitelist key, and the same value the
     *  clone's own `recordType()` holds. Mirrors the backend `record_type_key` and the web
     *  `recordTypeKey`; verified against `cast keccak "TRAVEL_CLEARANCE"` on chain 135. */
    fun recordTypeKey(recordType: String): String =
        "0x" + Keccak256.digest(recordType.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }

    /**
     * The ISSUER-WHITELIST pillar for a held credential, resolved end-to-end on-chain.
     *
     * The document's `issuer` block is NOT covered by the Merkle root, so `name`, `domain`,
     * `recordType` and — the sharp one — `documentStore` are chosen by whoever built the document.
     * Asking `documentStore` whether the credential is valid, and who issued it, is asking the
     * suspect for their own references: deploy a contract that answers `isValid = true` and names a
     * genuinely whitelisted signer, and integrity plus the issuance read both still pass.
     *
     * So the clone is resolved from the FACTORY in the app's own bundled `roax.json`
     * ([rootIssuer]) — never from the document. Then the record type comes from that clone's own
     * `recordType()`, and the issuing signer from its `issuedBy`. An envelope naming a different
     * clone, or a different record type, than the chain does is a definite [Result.Invalid], not
     * merely unresolved.
     *
     * WHICH AUTHORITY answers for that signer comes off the clone's own `registry()` too, never off
     * the bundled `IssuerRegistry` — see [whitelistedAtIssuance]. [issuerRegistry] survives here only
     * as a "is this bundle configured at all" precondition: an app shipped without one is missing the
     * whole address set, so refusing to state a chain fact from it is the honest answer.
     *
     * [Result.Unknown] means the pillar did not resolve; a caller must treat that as indeterminate,
     * never as a pass. A read that FAILED and a slot the chain says is empty both land there, but they
     * say so differently: only the latter may state a chain fact.
     *
     * A document naming no issuer contract at all is indeterminate too, not a mismatch. There is
     * nothing to compare the factory's answer against, and the DOG_PROFILE records that legitimately
     * carry no `documentStore` anchor in the SBT instead - so a comparison against the empty string
     * would call every one of them a forgery.
     */
    suspend fun issuerWhitelistPillar(
        rpcUrl: String,
        expectedChainId: Long,
        issuerRegistry: String,
        issuerFactory: String,
        documentStore: String,
        root: String,
        recordType: String,
    ): Result {
        if (issuerRegistry.isBlank()) return Result.Unknown("no IssuerRegistry configured")
        if (issuerFactory.isBlank()) return Result.Unknown("no DogTagIssuerFactory configured")
        if (recordType.isBlank()) return Result.Unknown("document declares no recordType")
        val claimedStore = documentStore.trim()
        if (claimedStore.isBlank()) return Result.Unknown("document names no issuer contract")
        // Uses the SAME factory read the issuer-domain binding uses, so both features agree on which
        // contract issued a root. `NoRecord` (the factory answered, and has none) and `Failure` (we
        // could not ask) are kept distinct: neither is a pass, but only one is evidence.
        val clone = when (val r = rootIssuer(rpcUrl, expectedChainId, issuerFactory, root, null)) {
            is AddressRead.Value -> r.address
            is AddressRead.NoRecord -> return Result.Unknown("no factory clone ever issued this root")
            is AddressRead.Failure ->
                return Result.Unknown("could not read the factory's issuer index (${r.reason})")
        }
        // The envelope points somewhere other than the contract that actually issued the root: a
        // definite misrepresentation, refused before the registry is consulted.
        if (!clone.equals(claimedStore, ignoreCase = true)) return Result.Invalid
        val chainRecordType = when (val r = recordTypeOf(rpcUrl, expectedChainId, clone)) {
            is HexRead.Found -> r.hex
            is HexRead.Unset -> return Result.Unknown("issuer clone reports no recordType")
            is HexRead.Unresolved ->
                return Result.Unknown("could not read the issuer clone's record type (${r.reason})")
        }
        if (!chainRecordType.equals(recordTypeKey(recordType), ignoreCase = true)) return Result.Invalid
        val signer = when (val r = issuedBy(rpcUrl, expectedChainId, clone, root)) {
            is HexRead.Found -> r.hex
            is HexRead.Unset -> return Result.Unknown("issuer clone reports no issuer for this root")
            is HexRead.Unresolved ->
                return Result.Unknown("could not read who issued this root (${r.reason})")
        }
        // Ask whether that signer held the capability AT THE MOMENT it anchored this root, not
        // whether it holds it now. Delisting is forward-only (`DogTagIssuer.sol:82`; `adminRevoke` is
        // the retroactive lever), so `isWhitelistedFor` - a current-state getter - would refuse every
        // credential a rotated, retired or lapsed signer ever issued while the protocol says each one
        // is genuine. `issuerRegistry` is deliberately NOT the authority asked: see below.
        return when (whitelistedAtIssuance(rpcUrl, expectedChainId, clone, signer, root)) {
            GrantAtIssuance.Authorized -> Result.Valid
            GrantAtIssuance.NotAuthorized -> Result.Invalid
            GrantAtIssuance.Undetermined ->
                Result.Unknown("could not establish whether the issuer was authorised at issuance")
        }
    }

    /**
     * Was `signer` authorised to anchor on `clone` when `root` was anchored there?
     *
     * Takes no registry address on purpose. The authority comes off the clone's own `registry()`,
     * which for a factory-resolved clone is unforgeable - `registry` is written once in `initialize`
     * from the factory's own `immutable registry` - and is the only instance whose `_wl` mapping gated
     * that contract's `issue()`. Asking this app's own bundled registry would ask a different
     * contract's mapping and, on a mis-paired bundle, refuse a genuine credential over our own
     * configuration.
     */
    internal suspend fun whitelistedAtIssuance(
        rpcUrl: String,
        expectedChainId: Long,
        clone: String,
        signer: String,
        root: String,
    ): GrantAtIssuance {
        // (1) WHICH authority answers. Zero, or a read that did not answer, means there is no
        // authority to ask - an initialized clone never reports zero here.
        val governing = when (val r = ethCall(rpcUrl, expectedChainId, clone, REGISTRY_SELECTOR)) {
            is CallResult.Ok ->
                if (r.hex.length < 40 || r.hex.trimStart('0').isEmpty()) {
                    return GrantAtIssuance.Undetermined
                } else {
                    "0x" + r.hex.takeLast(40).lowercase()
                }
            is CallResult.Err -> return GrantAtIssuance.Undetermined
        }

        // (2) WHEN this root was anchored, as a log point. `issuedAt` is a unix TIMESTAMP and cannot
        // be compared against a log's height without a timestamp->block search.
        val anchoring = ethGetLogs(
            rpcUrl, expectedChainId, clone, listOf(ROOT_ISSUED_TOPIC), listOf(pad32Topic(root)),
        ) ?: return GrantAtIssuance.Undetermined
        // Write-once `issuedAt` makes a second `RootIssued` impossible on an honest clone; take the
        // FIRST regardless, so a clone that somehow emitted twice cannot move the anchoring later.
        val anchoredAt = anchoring.mapNotNull { logPoint(it) }.minOrNull()
            ?: return GrantAtIssuance.Undetermined

        // (3) That authority's grant history for this exact (service, signer) pair. Keyed on the
        // SERVICE, not a record-type key: a clone carries exactly one record type, so filtering by
        // service inherently scopes the history to it.
        val grants = ethGetLogs(
            rpcUrl,
            expectedChainId,
            governing,
            listOf(ISSUANCE_CAPABILITY_SET_TOPIC),
            listOf("0x" + padAddr(clone), "0x" + padAddr(signer)),
        ) ?: return GrantAtIssuance.Undetermined
        val history = ArrayList<GrantEvent>(grants.size)
        for (log in grants) {
            // A grant whose position is unknown cannot be sequenced, and dropping it could turn a
            // withdrawn-before into an authorised. Refuse to answer instead.
            val at = logPoint(log) ?: return GrantAtIssuance.Undetermined
            // `allowed` is the one NON-indexed argument, so it is the single DATA word. Anything
            // that is not a well-formed bool is a malformed log, not a fact about the credential.
            val granted = allowedFromLogData(log) ?: return GrantAtIssuance.Undetermined
            history.add(GrantEvent(at, granted))
        }

        // (4) An EMPTY history is a DEFINITE refusal, and that is evidence about the credential
        // rather than about us: `issue()` is `onlyIssuanceCapable`, so an honest clone cannot have
        // anchored this root without the authority having granted this signer the capability. A read
        // that FAILED returned above and never arrives here as an empty one.
        return grantInForceAt(history, anchoredAt)
    }

    /**
     * The `allowed` word of an `IssuanceCapabilitySet` log, or null if the body is not a bool.
     *
     * Strict on purpose: a word that is neither 0 nor 1 is a log this build does not understand, and
     * guessing either way would state a grant or a withdrawal that was never recorded.
     */
    internal fun allowedFromLogData(log: JSONObject): Boolean? {
        val data = log.optString("data", "").removePrefix("0x").trimStart('0')
        return when (data) {
            "" -> false
            "1" -> true
            else -> null
        }
    }

    /** A 32-byte topic value, `0x`-prefixed and left-padded, from a hex word of any width. */
    private fun pad32Topic(hex: String): String = "0x" + pad32(hex).lowercase()

    /**
     * `VerificationRegistry.consumed(nullifier)` → true once the relayer's `recordVerificationZK`
     * has landed on-chain for this nullifier. This is the canonical completion
     * signal for the async export/verify flow: the groomer host records in the background, so the
     * phone polls this until it flips true. `nullifier` is public signal index 3. Accepts a decimal field
     * element or 0x.. hex, encoded here as a 32-byte word. Returns false on any RPC failure so
     * the caller simply keeps polling (and ultimately times out) rather than treating it as success.
     */
    suspend fun consumed(
        rpcUrl: String,
        expectedChainId: Long,
        verificationRegistry: String,
        nullifier: String,
    ): Boolean {
        if (verificationRegistry.isBlank() || nullifier.isBlank()) return false
        val data = CONSUMED_SELECTOR + padUint(nullifier)
        return when (val r = ethCall(rpcUrl, expectedChainId, verificationRegistry, data)) {
            is CallResult.Ok -> r.hex.trimStart('0').isNotEmpty()
            is CallResult.Err -> false
        }
    }

    private val GET_DISCOVERY_SET_SELECTOR = functionSelector("getDiscoverySet(bytes32)")
    private val GET_ACTIVE_ARTIFACT_SET_SELECTOR = functionSelector("getActiveArtifactSet(bytes32)")

    /** keccak256 of a version string as a 32-byte word — the `ProtocolRegistry` map key
     * (`contractSetId`) for that version. */
    fun versionId(version: String): String =
        Keccak256.digest(version.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }

    /**
     * `ProtocolRegistry.getDiscoverySet(versionId)` → the on-chain discovery record.
     *
     * The getter is `getDiscoverySet`, NOT `getContractSet` — that name belongs to an earlier record
     * shape and is absent from this contract. Calling the absent one reverts at the dispatcher with
     * EMPTY returndata, which arrives here as [CallResult.Err] and is indistinguishable from an
     * unpublished version; the app then fails closed for a reason that is not the real one. The
     * rename is deliberate: a record's SHAPE and its getter's NAME move together precisely so a stale
     * client reverts on dispatch instead of decoding every member one slot out.
     *
     * Null when the registry is unconfigured/unreachable OR the version is unpublished (the getter
     * reverts "unknown discovery set"), so verification fails closed. A blank address is null.
     */
    suspend fun getDiscoverySet(
        rpcUrl: String,
        expectedChainId: Long,
        protocolRegistry: String,
        version: String,
    ): AnchorResolver.DiscoverySetRecord? {
        if (protocolRegistry.isBlank()) return null
        val data = GET_DISCOVERY_SET_SELECTOR + versionId(version)
        return when (val r = ethCall(rpcUrl, expectedChainId, protocolRegistry, data)) {
            is CallResult.Ok -> AnchorResolver.decodeDiscoverySet(r.hex)
            is CallResult.Err -> null
        }
    }

    /**
     * `ProtocolRegistry.getActiveArtifactSet(versionId)` → the artifact-set record currently bound to
     * the version (M-4 PR3). Follows `activeArtifactSetOf` on-chain and reverts if unbound, so a null
     * here (unconfigured registry, unpublished version, or no binding) fails verification
     * closed. Decodes only `artifactSetId`/`minAppVersion`/`active` — see `AnchorResolver`.
     */
    suspend fun getActiveArtifactSet(
        rpcUrl: String,
        expectedChainId: Long,
        protocolRegistry: String,
        version: String,
    ): AnchorResolver.ArtifactSetRecord? {
        if (protocolRegistry.isBlank()) return null
        val data = GET_ACTIVE_ARTIFACT_SET_SELECTOR + versionId(version)
        return when (val r = ethCall(rpcUrl, expectedChainId, protocolRegistry, data)) {
            is CallResult.Ok -> AnchorResolver.decodeArtifactSet(r.hex)
            is CallResult.Err -> null
        }
    }

    /**
     * Reading a dynamic `string` return has THREE outcomes, and collapsing any two of them is how a
     * fail-open gets reintroduced:
     *  - [Value] the call returned ABI-encoded bytes (possibly an EMPTY string, which is a real answer
     *    meaning "no claim published");
     *  - [NoContract] an EMPTY `eth_call` result, which is what a call to an address with no code
     *    returns, i.e. the contract is not deployed for this config. Emphatically NOT an empty string,
     *    and it must surface as "we could not check", never as "no claim";
     *  - [Failure] the RPC did not answer.
     */
    sealed class StringRead {
        data class Value(val value: String) : StringRead()
        object NoContract : StringRead()
        data class Failure(val reason: String) : StringRead()
    }

    /**
     * Reading an `address` return has the same three outcomes, kept apart for the same reason:
     *  - [Value] a real, non-zero address;
     *  - [NoRecord] the mapping answered with the zero address, i.e. "no entry for this key" — a
     *    DEFINITE answer, not a failure;
     *  - [Failure] the RPC did not answer, or the body was not an address word (an empty result, which
     *    is what a call to an address with no code returns, lands here). Emphatically NOT "no record":
     *    a read we could not make is evidence of nothing.
     */
    sealed class AddressRead {
        data class Value(val address: String) : AddressRead()
        object NoRecord : AddressRead()
        data class Failure(val reason: String) : AddressRead()
    }

    /**
     * `DogTagIssuerFactory.rootIssuer(root)` — the clone that ISSUED this root, write-once on-chain.
     *
     * THE authoritative answer to "which contract issued this credential". The document's
     * `issuer.documentStore` is only a claim, and pointing it at ANOTHER authority's genuine clone is the
     * sharper form of the relabelling attack: link 1 (`isClone`) passes because the target really is a
     * factory clone, so without this read the phone renders that other authority's on-chain identity.
     */
    suspend fun rootIssuer(
        rpcUrl: String,
        expectedChainId: Long,
        factory: String,
        root: String,
        atBlock: Long?,
    ): AddressRead {
        if (factory.isBlank() || root.isBlank()) return AddressRead.Failure("missing factory/root")
        val data = ROOT_ISSUER_SELECTOR + pad32(root)
        return when (val r = ethCall(rpcUrl, expectedChainId, factory, data, atBlock)) {
            is CallResult.Err -> AddressRead.Failure(r.reason)
            is CallResult.Ok -> {
                val addr = decodeAbiAddress(r.hex) ?: return AddressRead.Failure("not an address word")
                if (isZeroAddress(addr)) AddressRead.NoRecord else AddressRead.Value(addr)
            }
        }
    }

    /**
     * Decode a right-aligned 32-byte `address` word to lowercase `0x..`. Returns null rather than
     * guessing for anything that is not one — including an EMPTY result (no contract at that address)
     * and a word with dirty high bytes.
     */
    fun decodeAbiAddress(hex: String): String? {
        val h = hex.removePrefix("0x").lowercase()
        if (h.length != 64) return null
        if (!h.all { it in '0'..'9' || it in 'a'..'f' }) return null
        if (!h.take(24).all { it == '0' }) return null
        return "0x" + h.takeLast(40)
    }

    /** The zero address, i.e. an unset mapping slot. */
    fun isZeroAddress(addr: String): Boolean {
        val h = addr.removePrefix("0x")
        return h.isNotEmpty() && h.all { it == '0' }
    }

    /**
     * The current chain head, so every read in one verification pins to ONE block. Against a world where
     * DNS changes and clones are superseded, an unanchored answer is not auditable.
     */
    suspend fun blockNumber(rpcUrl: String, expectedChainId: Long): Long? {
        val payload = JSONObject().apply {
            put("jsonrpc", "2.0"); put("id", 1); put("method", "eth_blockNumber"); put("params", JSONArray())
        }.toString()
        return try {
            val resp = guardedPostJson(rpcUrl, expectedChainId, payload)
            if (!resp.ok) return null
            val hex = JSONObject(resp.body).optString("result", "").removePrefix("0x")
            if (hex.isEmpty()) null else hex.toLong(16)
        } catch (e: Exception) {
            null
        }
    }

    /**
     * `DogTagIssuerFactory.isClone(candidate)` — LINK 1 of the issuer↔domain chain. Proves the contract
     * came from the DogTag factory, i.e. passed through KYC-gated `createIssuer`. Without it a domain
     * binding shows only that whoever deployed some contract also controls a domain.
     */
    suspend fun isClone(
        rpcUrl: String,
        expectedChainId: Long,
        factory: String,
        candidate: String,
        atBlock: Long?,
    ): Result {
        if (factory.isBlank() || candidate.isBlank()) return Result.Unknown("missing factory/candidate")
        val data = IS_CLONE_SELECTOR + padAddr(candidate)
        return when (val r = ethCall(rpcUrl, expectedChainId, factory, data, atBlock)) {
            is CallResult.Err -> Result.Unknown(r.reason)
            is CallResult.Ok ->
                // A call to a non-contract returns empty. That is not `false` — it is "no answer".
                if (r.hex.isEmpty()) Result.Unknown("empty result")
                else if (r.hex.any { it != '0' }) Result.Valid
                else Result.Invalid
        }
    }

    /**
     * `IssuerDomainRegistry.domainOf(clone)` — LINK 2. The domain the CONTRACT claims. Never the
     * document's `issuer.domain`, which is outside the Merkle root and can be relabelled at will.
     */
    suspend fun issuerClaimedDomain(
        rpcUrl: String,
        expectedChainId: Long,
        domainRegistry: String,
        clone: String,
        atBlock: Long?,
    ): StringRead {
        if (domainRegistry.isBlank() || clone.isBlank()) return StringRead.Failure("missing registry/clone")
        return ethCallString(
            rpcUrl, expectedChainId, domainRegistry, DOMAIN_OF_SELECTOR + padAddr(clone), atBlock,
        )
    }

    /**
     * `DogTagIssuer.name()` — the clone's own name, written by the factory's `onlyOwner` `createIssuer`
     * at KYC time. The only issuer name a surface may present: the document's `issuer.name` is outside
     * the Merkle root, so relabelling it alone passes integrity AND the `data.issuer` DID check (the DID
     * carries a domain, not a name).
     */
    suspend fun issuerOnchainName(
        rpcUrl: String,
        expectedChainId: Long,
        clone: String,
        atBlock: Long?,
    ): StringRead {
        if (clone.isBlank()) return StringRead.Failure("missing clone")
        return ethCallString(rpcUrl, expectedChainId, clone, ISSUER_NAME_SELECTOR, atBlock)
    }

    private suspend fun ethCallString(
        rpcUrl: String,
        expectedChainId: Long,
        to: String,
        data: String,
        atBlock: Long?,
    ): StringRead = when (val r = ethCall(rpcUrl, expectedChainId, to, data, atBlock)) {
        is CallResult.Err -> StringRead.Failure(r.reason)
        is CallResult.Ok ->
            if (r.hex.isEmpty()) StringRead.NoContract
            else StringRead.Value(decodeAbiString(r.hex) ?: "")
    }

    /**
     * Decode a single ABI-encoded dynamic `string` return: `[32-byte offset][32-byte length][bytes]`.
     * Returns null on a malformed body rather than guessing.
     */
    fun decodeAbiString(hex: String): String? {
        val bytes = hexToBytes(hex)
        if (bytes.size < 64) return null
        // Read the offset rather than assuming 0x20, so a tuple-wrapped return still decodes.
        val offset = beInt(bytes, 0) ?: return null
        // Compare by SUBTRACTION, never `offset + 32`: an Int is 32 bits here, so the addition wraps
        // negative for an offset near Int.MAX_VALUE, the guard passes, and the decoder throws instead of
        // returning null as documented. Swift's Int is 64-bit and cannot wrap, which is exactly how the
        // two ports drifted.
        if (offset < 0 || offset > bytes.size - 32) return null
        val len = beInt(bytes, offset) ?: return null
        val start = offset + 32
        if (len < 0 || len > bytes.size - start) return null
        return String(bytes, start, len, Charsets.UTF_8)
    }

    private fun hexToBytes(hex: String): ByteArray {
        val h = hex.removePrefix("0x")
        val n = h.length / 2
        val out = ByteArray(n)
        for (i in 0 until n) {
            out[i] = ((h[2 * i].digitToIntOrNull(16) ?: return out.copyOf(i)) shl 4 or
                (h[2 * i + 1].digitToIntOrNull(16) ?: return out.copyOf(i))).toByte()
        }
        return out
    }

    /**
     * Big-endian read of a 32-byte word as an Int. Returns null when anything is set above the low 4
     * bytes — a length or offset that large is not a value we will honour, and treating it as an Int
     * would wrap.
     */
    private fun beInt(bytes: ByteArray, at: Int): Int? {
        if (at < 0 || at > bytes.size - 32) return null
        for (i in at until at + 28) if (bytes[i] != 0.toByte()) return null
        var v = 0L
        for (i in at + 28 until at + 32) v = (v shl 8) or (bytes[i].toLong() and 0xff)
        return if (v > Int.MAX_VALUE) null else v.toInt()
    }

    /**
     * The outcome of one `eth_call`.
     *
     * [Err.answered] splits the failure in two, and the split is load-bearing rather than
     * bookkeeping: a node returning a JSON-RPC error for a request it PROCESSED is how a revert
     * arrives, and a revert is evidence about the contract, while a timeout, a dropped connection or
     * an HTTP status is no answer at all and is evidence about nothing. Every caller that draws a
     * conclusion ABOUT A CONTRACT from a failed call has to know which it got - see
     * [grantAtIssuance].
     */
    internal sealed class CallResult {
        data class Ok(val hex: String) : CallResult()
        /**
         * @param answered true only when the CONTRACT executed the call and reverted - see
         *   [isExecutionRevert]. False - the default, since it is the safe reading - for everything
         *   else, including a JSON-RPC error the node raised about ITSELF: a non-2xx HTTP status, a
         *   transport exception, unparseable JSON, a rate limit, an internal error.
         */
        data class Err(val reason: String, val answered: Boolean = false) : CallResult()
    }

    /**
     * The JSON-RPC error code geth returns for a call the EVM EXECUTED and reverted. Confirmed
     * against ROAX on 2026-07-31 with the exact production case: `isRecognizedIssuer` put to the
     * deployed generation-1 `IssuerRegistry` answers
     * `{"code":3,"message":"execution reverted","data":"0x"}`.
     */
    internal const val EXECUTION_REVERTED_CODE = 3

    /**
     * Did the CONTRACT execute this call and revert, or did the NODE fail on its own account?
     *
     * A JSON-RPC error member is not by itself evidence the contract did anything. `-32005` rate
     * limit, `-32603` internal error, `-32601` method not found and `-32002` resource unavailable are
     * the node speaking about ITSELF; reading one of those as generation 1 leaves an empty grant
     * history standing as a definite refusal, i.e. a forgery verdict against a genuine credential
     * produced by a call that never ran.
     *
     * Only an execution revert licenses that conclusion, and it is exactly the signal wanted: a
     * generation-1 `IssuerRegistry` has no `isRecognizedIssuer` and no fallback, so its dispatcher
     * reverts. The code is the typed discriminator; the canonical message is accepted alongside it for
     * clients that report the same revert under a different code (several spell it `-32000`), without
     * which the pillar would stop refusing every never-granted signer against such a peer.
     *
     * Mirrors Rust `answered_with_execution_revert`, Swift `RoaxRpc.isExecutionRevert` and viem's
     * `ExecutionRevertedError` (`code === 3 || /execution reverted/`, the same pair).
     */
    internal fun isExecutionRevert(code: Int, message: String): Boolean =
        code == EXECUTION_REVERTED_CODE || message.contains("execution reverted", ignoreCase = true)


    private suspend fun ethCall(
        rpcUrl: String,
        expectedChainId: Long,
        to: String,
        data: String,
        atBlock: Long? = null,
    ): CallResult {
        // Pin to a block when we have one so a whole verification is a consistent, reproducible
        // snapshot; `latest` otherwise, and the caller reports the answer as unanchored.
        val blockTag = atBlock?.let { "0x" + it.toString(16) } ?: "latest"
        val params = JSONArray().apply {
            put(JSONObject().apply { put("to", to); put("data", data) })
            put(blockTag)
        }
        val payload = JSONObject().apply {
            put("jsonrpc", "2.0"); put("id", 1); put("method", "eth_call"); put("params", params)
        }.toString()
        return try {
            val resp = guardedPostJson(rpcUrl, expectedChainId, payload)
            // A non-2xx is the transport failing, not the node answering: a revert comes back as a
            // 200 carrying a JSON-RPC `error` member.
            if (!resp.ok) return CallResult.Err("rpc ${resp.code}")
            val o = JSONObject(resp.body)
            if (o.has("error")) {
                val err = o.getJSONObject("error")
                val message = err.optString("message", "rpc error")
                // Carrying an `error` member is NOT enough to call this a contract answer - most of
                // what arrives that way is the node speaking about itself. See [isExecutionRevert].
                return CallResult.Err(
                    message,
                    answered = isExecutionRevert(err.optInt("code", 0), message),
                )
            }
            CallResult.Ok(o.optString("result", "").removePrefix("0x"))
        } catch (e: Exception) {
            CallResult.Err(e.message ?: "rpc unreachable")
        }
    }

    /**
     * `eth_getLogs` over the SAME chain-guarded transport every `eth_call` uses, so a wrong-chain or
     * unavailable peer receives no address-bound log query either.
     *
     * `topic0` is a SET, which the JSON-RPC spec allows: `Whitelisted` and `Delisted` come back
     * interleaved in log order from one call rather than two. A `null` entry matches any value in
     * that position. Returns `null` for a read that did not answer, which callers must keep apart
     * from an empty list - the empty list is the registry saying "nothing was ever recorded here".
     */
    private suspend fun ethGetLogs(
        rpcUrl: String,
        expectedChainId: Long,
        address: String,
        topic0: List<String>,
        topics: List<String?>,
    ): List<JSONObject>? {
        val topicArray = JSONArray().apply {
            put(JSONArray().apply { topic0.forEach { put(it) } })
            topics.forEach { put(it ?: JSONObject.NULL) }
        }
        val filter = JSONObject().apply {
            put("address", address)
            put("topics", topicArray)
            put("fromBlock", "0x0")
            put("toBlock", "latest")
        }
        val payload = JSONObject().apply {
            put("jsonrpc", "2.0"); put("id", 1); put("method", "eth_getLogs")
            put("params", JSONArray().apply { put(filter) })
        }.toString()
        return try {
            val resp = guardedPostJson(rpcUrl, expectedChainId, payload)
            if (!resp.ok) return null
            val o = JSONObject(resp.body)
            if (o.has("error")) return null
            val arr = o.optJSONArray("result") ?: return null
            (0 until arr.length()).mapNotNull { arr.optJSONObject(it) }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            null
        }
    }

    /**
     * A log's position, or `null` when it carries none.
     *
     * A pending log has no established place in the sequence, so it must not be silently dropped and
     * must not be ordered by guess - the caller treats an unpositioned log as UNDETERMINED.
     */
    private fun logPoint(log: JSONObject): LogPoint? {
        val block = hexQuantity(log.optString("blockNumber", "")) ?: return null
        val index = hexQuantity(log.optString("logIndex", "")) ?: return null
        return LogPoint(block, index)
    }

    /**
     * Parse an `0x`-prefixed JSON-RPC quantity. `null` for anything unparseable, never 0.
     *
     * The digit test is an explicit ASCII RANGE, never `Char.isDigit()`, which is Unicode-aware and
     * admits Nd digits such as `٣` (U+0663) - `toLongOrNull(16)` then accepts them too via
     * `Character.digit`. The Swift twin's `UInt64(_:radix:)` is ASCII-only and rejects them, so the
     * loose test made the two platforms parse a hostile peer's log positions differently.
     */
    private fun hexQuantity(hex: String): Long? {
        val h = hex.removePrefix("0x")
        val ascii = h.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }
        if (h.isEmpty() || h.length > 15 || !ascii) return null
        return h.toLongOrNull(16)
    }

    private fun padAddr(addr: String): String {
        val h = addr.removePrefix("0x").lowercase()
        return h.padStart(64, '0')
    }

    private fun pad32(hex: String): String {
        val h = hex.removePrefix("0x")
        return h.padStart(64, '0')
    }

    /** Encode a decimal (or 0x-hex) uint256 tokenId as a 64-char hex word. */
    private fun padUint(dec: String): String {
        val v = if (dec.startsWith("0x")) {
            java.math.BigInteger(dec.removePrefix("0x"), 16)
        } else {
            java.math.BigInteger(dec)
        }
        return v.toString(16).padStart(64, '0')
    }
}
