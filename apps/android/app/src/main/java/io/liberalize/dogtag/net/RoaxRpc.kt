package io.liberalize.dogtag.net

import io.liberalize.dogtag.wallet.Keccak256
import org.json.JSONArray
import org.json.JSONObject

/**
 * Read-only JSON-RPC client for the ROAX chain (chainId 135, RPC https://devrpc.roax.net).
 *
 * Used to re-check the issuance pillar: `DogTagIssuer.isValid(bytes32 root)` over the wrapped doc's
 * `issuer.documentStore`. This is a pure `eth_call` (no signing, no gas). The RPC may be unreachable
 * (it returned 502 at design time) — callers treat an RPC failure as an UNKNOWN, never a hard fail.
 */
object RoaxRpc {
    const val DEFAULT_RPC = "https://devrpc.roax.net"

    internal enum class ProfileRootObservation { Pending, Matched, Mismatch }

    /** 4-byte function selector = first 4 bytes of keccak256(canonical signature), `0x`-prefixed hex. */
    internal fun functionSelector(signature: String): String =
        "0x" + Keccak256.digest(signature.toByteArray(Charsets.US_ASCII)).take(4)
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
    private val ROOT_ISSUER_SELECTOR = functionSelector("rootIssuer(bytes32)")
    private val RECORD_TYPE_SELECTOR = functionSelector("recordType()")
    private val IS_WHITELISTED_FOR_SELECTOR = functionSelector("isWhitelistedFor(bytes32,address)")
    private val CONSUMED_SELECTOR = functionSelector("consumed(bytes32)")
    private val PROFILE_ROOT_SELECTOR = functionSelector("profileRoot(uint256)")

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
     * Call `isValid(root)` on the issuer clone. `documentStore` is the issuer contract address from the
     * wrapped doc; `root` is the 0x.. 32-byte merkleRoot.
     */
    suspend fun isValid(rpcUrl: String, documentStore: String, root: String): Result {
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
            val resp = Http.postJson(rpcUrl, payload)
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
    suspend fun profileRoot(rpcUrl: String, dogTagSbt: String, dogTagId: String): String? {
        if (dogTagSbt.isBlank() || dogTagId.isBlank()) return null
        val data = PROFILE_ROOT_SELECTOR + padUint(dogTagId)
        return when (val r = ethCall(rpcUrl, dogTagSbt, data)) {
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
        issuerRegistry: String,
        key: String,
        signer: String,
    ): Result {
        if (issuerRegistry.isBlank() || key.isBlank() || signer.isBlank()) {
            return Result.Unknown("missing addr/key/signer")
        }
        val data = IS_WHITELISTED_FOR_SELECTOR + pad32(key) + padAddr(signer)
        return when (val r = ethCall(rpcUrl, issuerRegistry, data)) {
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
    suspend fun issuedBy(rpcUrl: String, documentStore: String, root: String): HexRead {
        if (documentStore.isBlank() || root.isBlank()) return HexRead.Unresolved("missing addr/root")
        val data = ISSUED_BY_SELECTOR + pad32(root)
        return when (val r = ethCall(rpcUrl, documentStore, data)) {
            // address is right-aligned in a 32-byte word; all-zero == never issued here.
            is CallResult.Ok ->
                if (r.hex.length < 40) HexRead.Unresolved("the issuer clone returned no address")
                else if (r.hex.trimStart('0').isEmpty()) HexRead.Unset
                else HexRead.Found("0x" + r.hex.takeLast(40).lowercase())
            is CallResult.Err -> HexRead.Unresolved(r.reason)
        }
    }

    /**
     * `DogTagIssuerFactory.rootIssuer(root)` → the clone that actually issued this root.
     * [HexRead.Unset] when no clone of this factory ever did (the on-chain zero address),
     * [HexRead.Unresolved] when the read did not answer.
     *
     * This is the anchor the issuer pillar hangs from. `registerRoot` is called only from inside a
     * clone's `issue()` and is `require(isClone[msg.sender])` + strictly write-once, so a contract the
     * factory never deployed can never appear here and a genuine root's issuer can never be
     * overwritten.
     */
    suspend fun rootIssuer(rpcUrl: String, factory: String, root: String): HexRead {
        if (factory.isBlank() || root.isBlank()) return HexRead.Unresolved("missing factory/root")
        val data = ROOT_ISSUER_SELECTOR + pad32(root)
        return when (val r = ethCall(rpcUrl, factory, data)) {
            is CallResult.Ok ->
                if (r.hex.length < 40) HexRead.Unresolved("the factory returned no address")
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
    suspend fun recordTypeOf(rpcUrl: String, issuerClone: String): HexRead {
        if (issuerClone.isBlank()) return HexRead.Unresolved("missing issuer clone")
        return when (val r = ethCall(rpcUrl, issuerClone, RECORD_TYPE_SELECTOR)) {
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
     * `recordType()`, and the issuing signer from its `issuedBy`, checked against the app's own
     * `IssuerRegistry`. An envelope naming a different clone, or a different record type, than the
     * chain does is a definite [Result.Invalid], not merely unresolved.
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
        val clone = when (val r = rootIssuer(rpcUrl, issuerFactory, root)) {
            is HexRead.Found -> r.hex
            is HexRead.Unset -> return Result.Unknown("no factory clone ever issued this root")
            is HexRead.Unresolved ->
                return Result.Unknown("could not read the factory's issuer index (${r.reason})")
        }
        // The envelope points somewhere other than the contract that actually issued the root: a
        // definite misrepresentation, refused before the registry is consulted.
        if (!clone.equals(claimedStore, ignoreCase = true)) return Result.Invalid
        val chainRecordType = when (val r = recordTypeOf(rpcUrl, clone)) {
            is HexRead.Found -> r.hex
            is HexRead.Unset -> return Result.Unknown("issuer clone reports no recordType")
            is HexRead.Unresolved ->
                return Result.Unknown("could not read the issuer clone's record type (${r.reason})")
        }
        if (!chainRecordType.equals(recordTypeKey(recordType), ignoreCase = true)) return Result.Invalid
        val signer = when (val r = issuedBy(rpcUrl, clone, root)) {
            is HexRead.Found -> r.hex
            is HexRead.Unset -> return Result.Unknown("issuer clone reports no issuer for this root")
            is HexRead.Unresolved ->
                return Result.Unknown("could not read who issued this root (${r.reason})")
        }
        return isWhitelistedFor(rpcUrl, issuerRegistry, chainRecordType, signer)
    }

    /**
     * `VerificationRegistry.consumed(nullifier)` → true once the relayer's `recordVerificationZK`
     * has landed on-chain for this nullifier. This is the canonical completion
     * signal for the async export/verify flow: the groomer host records in the background, so the
     * phone polls this until it flips true. `nullifier` is public signal index 3. Accepts a decimal field
     * element or 0x.. hex, encoded here as a 32-byte word. Returns false on any RPC failure so
     * the caller simply keeps polling (and ultimately times out) rather than treating it as success.
     */
    suspend fun consumed(rpcUrl: String, verificationRegistry: String, nullifier: String): Boolean {
        if (verificationRegistry.isBlank() || nullifier.isBlank()) return false
        val data = CONSUMED_SELECTOR + padUint(nullifier)
        return when (val r = ethCall(rpcUrl, verificationRegistry, data)) {
            is CallResult.Ok -> r.hex.trimStart('0').isNotEmpty()
            is CallResult.Err -> false
        }
    }

    private val GET_CONTRACT_SET_SELECTOR = functionSelector("getContractSet(bytes32)")
    private val GET_ACTIVE_ARTIFACT_SET_SELECTOR = functionSelector("getActiveArtifactSet(bytes32)")

    /** keccak256 of a version string as a 32-byte word — the `ProtocolRegistry` map key
     * (`contractSetId`) for that version. */
    fun versionId(version: String): String =
        Keccak256.digest(version.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }

    /**
     * `ProtocolRegistry.getContractSet(versionId)` → the on-chain contract-set record (M-4 PR3).
     * Null when the registry is unconfigured/unreachable OR the version is unpublished (the getter
     * reverts "unknown contract set"), so verification fails closed. A blank address is null.
     */
    suspend fun getContractSet(rpcUrl: String, protocolRegistry: String, version: String): AnchorResolver.ContractSetRecord? {
        if (protocolRegistry.isBlank()) return null
        val data = GET_CONTRACT_SET_SELECTOR + versionId(version)
        return when (val r = ethCall(rpcUrl, protocolRegistry, data)) {
            is CallResult.Ok -> AnchorResolver.decodeContractSet(r.hex)
            is CallResult.Err -> null
        }
    }

    /**
     * `ProtocolRegistry.getActiveArtifactSet(versionId)` → the artifact-set record currently bound to
     * the version (M-4 PR3). Follows `activeArtifactSetOf` on-chain and reverts if unbound, so a null
     * here (unconfigured registry, unpublished version, or no binding) fails verification
     * closed. Decodes only `artifactSetId`/`minAppVersion`/`active` — see `AnchorResolver`.
     */
    suspend fun getActiveArtifactSet(rpcUrl: String, protocolRegistry: String, version: String): AnchorResolver.ArtifactSetRecord? {
        if (protocolRegistry.isBlank()) return null
        val data = GET_ACTIVE_ARTIFACT_SET_SELECTOR + versionId(version)
        return when (val r = ethCall(rpcUrl, protocolRegistry, data)) {
            is CallResult.Ok -> AnchorResolver.decodeArtifactSet(r.hex)
            is CallResult.Err -> null
        }
    }

    private sealed class CallResult {
        data class Ok(val hex: String) : CallResult()
        data class Err(val reason: String) : CallResult()
    }

    private suspend fun ethCall(rpcUrl: String, to: String, data: String): CallResult {
        val params = JSONArray().apply {
            put(JSONObject().apply { put("to", to); put("data", data) })
            put("latest")
        }
        val payload = JSONObject().apply {
            put("jsonrpc", "2.0"); put("id", 1); put("method", "eth_call"); put("params", params)
        }.toString()
        return try {
            val resp = Http.postJson(rpcUrl, payload)
            if (!resp.ok) return CallResult.Err("rpc ${resp.code}")
            val o = JSONObject(resp.body)
            if (o.has("error")) return CallResult.Err(o.getJSONObject("error").optString("message", "rpc error"))
            CallResult.Ok(o.optString("result", "").removePrefix("0x"))
        } catch (e: Exception) {
            CallResult.Err(e.message ?: "rpc unreachable")
        }
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
