package io.liberalize.dogtag.net

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

    // keccak256("isValid(bytes32)")[:4] = 0x6d04f0 bc ... -> 0x6d04f0bc
    private const val IS_VALID_SELECTOR = "0x6d04f0bc"

    sealed class Result {
        object Valid : Result()
        object Invalid : Result()
        data class Unknown(val reason: String) : Result()
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

    // keccak256("isWhitelistedFor(bytes32,address)")[:4]
    private const val IS_WHITELISTED_FOR_SELECTOR = "0x779c3985"
    // keccak256("bindNonce(address)")[:4]
    private const val BIND_NONCE_SELECTOR = "0x15c95be6"
    // keccak256("profileRoot(uint256)")[:4]
    private const val PROFILE_ROOT_SELECTOR = "0x85105cb3"
    // keccak256("ownerOf(uint256)")[:4] (ERC-721)
    private const val OWNER_OF_SELECTOR = "0x6352211e"

    /**
     * `DogTagSBT.profileRoot(dogTagId)` → the on-chain DOG_PROFILE root (0x.. 32-byte), or null on
     * failure. `dogTagId` is the decimal tokenId. This is the SBT anchor used to verify an issued
     * DOG_PROFILE (NOT the DogTagIssuer-clone isValid).
     */
    suspend fun profileRoot(rpcUrl: String, dogTagSbt: String, dogTagId: String): String? {
        if (dogTagSbt.isBlank() || dogTagId.isBlank()) return null
        val data = PROFILE_ROOT_SELECTOR + padUint(dogTagId)
        return when (val r = ethCall(rpcUrl, dogTagSbt, data)) {
            is CallResult.Ok -> "0x" + r.hex.padStart(64, '0')
            is CallResult.Err -> null
        }
    }

    /**
     * `DogTagSBT.ownerOf(dogTagId)` → the owner address (0x.. 20-byte, lowercased), or null on
     * failure. `dogTagId` is the decimal tokenId.
     */
    suspend fun ownerOf(rpcUrl: String, dogTagSbt: String, dogTagId: String): String? {
        if (dogTagSbt.isBlank() || dogTagId.isBlank()) return null
        val data = OWNER_OF_SELECTOR + padUint(dogTagId)
        return when (val r = ethCall(rpcUrl, dogTagSbt, data)) {
            is CallResult.Ok -> {
                val padded = r.hex.padStart(64, '0')
                "0x" + padded.takeLast(40).lowercase()
            }
            is CallResult.Err -> null
        }
    }

    /**
     * `IssuerRegistry.isWhitelistedFor(key, signer)` — the PRE-PROOF groomer check. `key` is the
     * 0x.. 32-byte VERIFY key (`verifyWhitelistKeyHex(purpose)`); `signer` is the scanned relayer.
     * Returns Valid (whitelisted), Invalid (not), or Unknown (RPC unreachable). On Unknown the caller
     * MUST hard-stop — this gate is a user-safety requirement, so unknown is treated as not-authorized.
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
            is CallResult.Ok -> {
                val truthy = r.hex.trimStart('0').isNotEmpty()
                if (truthy) Result.Valid else Result.Invalid
            }
            is CallResult.Err -> Result.Unknown(r.reason)
        }
    }

    /** `ConsentKeyRegistry.bindNonce(subject)` → the current bind nonce (decimal) or null on failure. */
    suspend fun bindNonce(rpcUrl: String, consentKeyRegistry: String, subject: String): Long? {
        if (consentKeyRegistry.isBlank() || subject.isBlank()) return null
        val data = BIND_NONCE_SELECTOR + padAddr(subject)
        return when (val r = ethCall(rpcUrl, consentKeyRegistry, data)) {
            is CallResult.Ok -> runCatching {
                java.math.BigInteger(r.hex.ifBlank { "0" }, 16).toLong()
            }.getOrNull()
            is CallResult.Err -> null
        }
    }

    // keccak256("consumed(bytes32)")[:4]
    private const val CONSUMED_SELECTOR = "0x4648c943"

    /**
     * `VerificationRegistry.consumed(nullifier)` → true once the relayer's `recordVerificationZK`
     * (or the legacy path) has landed on-chain for this nullifier. This is the CANONICAL completion
     * signal for the async export/verify flow: the groomer host records in the background, so the
     * phone polls this until it flips true. `nullifier` is the proof's `pubSignals[4]` (a decimal
     * field element or 0x.. hex), encoded here as a 32-byte word. Returns false on any RPC failure so
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

    /** `VerificationRegistry.keyOf(subject)` → the bound consent keyHash (0x..), or null/0x00.. if unbound. */
    suspend fun keyOf(rpcUrl: String, verificationRegistry: String, subject: String): String? {
        if (verificationRegistry.isBlank() || subject.isBlank()) return null
        // keyOf(address) selector = keccak256("keyOf(address)")[:4]
        val data = "0xfa073d76" + padAddr(subject)
        return when (val r = ethCall(rpcUrl, verificationRegistry, data)) {
            is CallResult.Ok -> "0x" + r.hex.padStart(64, '0')
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
