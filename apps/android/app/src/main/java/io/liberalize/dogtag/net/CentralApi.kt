package io.liberalize.dogtag.net

import org.json.JSONArray
import org.json.JSONObject
import uniffi.dogtag_standard.ProofFfi

/**
 * Typed client for the per-host vet/groomer APIs: dog-tag issuance bind, the export-consent relay, and
 * the export-session resolve/poll. Every host comes from a scanned QR — the device never calls a
 * central admin base for registration or pet sync (the dog tag is issued by the vet via `/p/<token>`).
 */
object CentralApi {

    /** POST /v1/verify/consent — relay the signed consent. Owner-session gated server-side. */
    suspend fun postConsent(centralBase: String, sessionToken: String?, payloadJson: String): Http.Response =
        Http.postJson("$centralBase/v1/verify/consent", payloadJson, bearer = sessionToken)

    /** The result of binding a dog-tag at the vet host: the issued DOG_PROFILE + its on-chain anchors. */
    data class DogTagIssue(
        val wrappedDocJson: String,
        val dogTagId: String,
        val root: String,
        val txHash: String,
        val walletAddress: String,
    )

    /**
     * POST <host>/profiles/issue/bind { token, walletAddress, signature } — the vet-issues-the-dog-tag
     * flow. The host comes from the scanned `/p/<token>` QR (NOT a central base URL). `signature` is the
     * EIP-191 personal_sign from `Wallet.registerSignature()`. The server now responds IMMEDIATELY with
     * the off-chain-built credential `{ wrappedDoc, dogTagId, root, walletAddress, status: "minting" }`
     * and mints the SBT in the background; there is NO `txHash` in this response — the phone polls the
     * chain (profileRoot/ownerOf) until the mint lands. Null on failure.
     */
    suspend fun bindDogTagIssue(host: String, token: String, walletAddress: String, signature: String): DogTagIssue? {
        if (token.isBlank()) return null
        val body = JSONObject().apply {
            put("token", token)
            put("walletAddress", walletAddress)
            put("signature", signature)
        }.toString()
        return try {
            // the bind no longer waits for the on-chain mint (it returns the off-chain credential at once,
            // status "minting"), so a modest read timeout suffices — the slow chain wait moves to the poll.
            val resp = Http.postJson("$host/profiles/issue/bind", body, readTimeoutMs = 20000)
            if (!resp.ok) return null
            val o = JSONObject(resp.body)
            val wrapped = o.opt("wrappedDoc")
            val wrappedJson = when (wrapped) {
                is JSONObject -> wrapped.toString()
                is String -> wrapped
                else -> return null
            }
            DogTagIssue(
                wrappedDocJson = wrappedJson,
                dogTagId = o.optString("dogTagId", ""),
                root = o.optString("root", ""),
                txHash = o.optString("txHash", o.optString("tx_hash", "")),
                walletAddress = o.optString("walletAddress", walletAddress),
            )
        } catch (e: Exception) {
            null
        }
    }

    /**
     * The export-session metadata resolved from the QR's one-time token. The phone GETs this
     * (non-consuming) before proving so it can assert the groomer address, run the whitelist + DNS
     * checks, and build the consent. The token is consumed only on submit.
     */
    /**
     * The platform-OWNED, UNVERIFIED discovery claims from the resolve GET's `unverifiedClaims` block
     * (M7 §5.2). NONE of these is authority — under a `mode == "levelb"` session they are validated
     * against the dogtag `ProtocolRegistry` anchor via `validateDiscovery` before the app acts. Null
     * when the server omits the block (every pre-M-4 / non-levelb response); the Level-A path never
     * reads them. Deliberately NOT named `ConvenienceClaims` — that is the FFI record
     * `validateDiscovery` consumes; this is the raw parse the caller maps into it.
     */
    data class UnverifiedClaims(
        val protocolVersion: String,
        val chainId: Long,
        val verificationRegistry: String,
        val issuerClone: String,
        val purpose: String,
    )

    data class ExportSession(
        val sessionId: String,
        val relayer: String,
        val purpose: String,
        val recordType: String,
        val challenge: String,
        val mode: String,
        /** The `unverifiedClaims` block, present only when the server emits it (M-4 levelb sessions). */
        val claims: UnverifiedClaims? = null,
    )

    /** GET <host>/x/<token> → export-session metadata (non-consuming). Null on failure. */
    suspend fun resolveExportSession(host: String, token: String): ExportSession? {
        if (token.isBlank()) return null
        return try {
            val resp = Http.getJson("$host/x/$token")
            if (!resp.ok) return null
            val o = JSONObject(resp.body)
            // The convenience tier is additive: absent on every non-levelb / pre-M-4 response.
            val claims = o.optJSONObject("unverifiedClaims")?.let { uc ->
                UnverifiedClaims(
                    protocolVersion = uc.optString("protocolVersion", ""),
                    chainId = uc.optLong("chainId", 0L),
                    verificationRegistry = uc.optString("verificationRegistry", ""),
                    issuerClone = uc.optString("issuerClone", ""),
                    purpose = uc.optString("purpose", ""),
                )
            }
            ExportSession(
                sessionId = o.optString("sessionId", o.optString("session_id", "")),
                relayer = o.optString("relayer", ""),
                purpose = o.optString("purpose", ""),
                recordType = o.optString("recordType", o.optString("record_type", "")),
                challenge = o.optString("challenge", ""),
                mode = o.optString("mode", "zk"),
                claims = claims,
            )
        } catch (e: Exception) {
            null
        }
    }

    /**
     * ZK path: POST the proof bundle directly to the GROOMER host (the scanned QR origin), NOT central.
     * The groomer relays `recordVerificationZK` on-chain as the gas-payer. The body carries the one-time
     * `exportToken` (consumed server-side on submit) plus `{consent, sig, mode, proof, bind}`.
     */
    suspend fun postVerifyConsentToHost(host: String, payloadJson: String): Http.Response =
        // The submit now returns 200 {status:"recording", sessionId} FAST (no on-chain wait — the
        // groomer binds the consent key + relays recordVerificationZK in the background, ~24-48s on
        // ROAX). A ~20s read timeout (like bindDogTagIssue) is ample for the quick ack.
        Http.postJson("$host/v1/verify/consent", payloadJson, readTimeoutMs = 20000)

    /**
     * Level-B twin of [postVerifyConsentToHost]. The owner-hidden route acknowledges the detached
     * broadcast immediately; the caller reuses [verifySessionStatus] plus the Level-B registry's
     * `consumed(nullifier)` read-back loop.
     */
    suspend fun postVerifyConsentLevelBToHost(host: String, payloadJson: String): Http.Response =
        Http.postJson("$host/v1/verify/consent/levelb", payloadJson, readTimeoutMs = 20000)

    /**
     * 32-bit-device fallback: ask the TRUSTED PROVER SERVICE to generate the Groth16 proof.
     *
     * A 32-bit-only Android phone cannot run the on-device circom-prover, so instead of
     * `proveVerification(...)` it POSTs the SAME inputs — `{wrappedDoc, consent, eddsaSig}` — to
     * `<proverBase>/prove-verification` and gets back the Solidity calldata `{a, b, c, pub}`. We adapt
     * that into a `ProofFfi` (the exact type the on-device path yields) so the downstream submit +
     * chain-poll flow is byte-for-byte identical. The prover service sees the witness; the GROOMER
     * (where the proof is submitted next) never does.
     *
     * `eddsaSig` carries `{ r8xDec, r8yDec, sDec, axHex, ayHex }`. Returns null on any failure
     * (no prover configured, network error, non-2xx, malformed body) — the caller surfaces an error.
     */
    suspend fun proveOnServer(
        proverBase: String,
        wrappedDocJson: String,
        consentJson: String,
        eddsaSig: ProverEddsaSig,
    ): ProofFfi? {
        if (proverBase.isBlank()) return null
        val body = JSONObject().apply {
            // wrappedDoc/consent are sent as embedded JSON objects (the server also accepts strings).
            put("wrappedDoc", JSONObject(wrappedDocJson))
            put("consent", JSONObject(consentJson))
            put("eddsaSig", JSONObject().apply {
                put("r8xDec", eddsaSig.r8xDec)
                put("r8yDec", eddsaSig.r8yDec)
                put("sDec", eddsaSig.sDec)
                put("axHex", eddsaSig.axHex)
                put("ayHex", eddsaSig.ayHex)
            })
        }.toString()
        return try {
            // Server-side Groth16 proving is CPU-heavy (~10-30s release); generous read timeout so a
            // cold prove + LAN latency can't trip it during a demo (it returns as soon as the prove is done).
            val resp = Http.postJson("$proverBase/prove-verification", body, readTimeoutMs = 120000)
            if (!resp.ok) return null
            val o = JSONObject(resp.body)
            // Server returns the 7-vector under "pub" (Groth16Output shape); the on-device ProofFfi
            // names it "pubSignals" — map across here.
            val toList = { arr: JSONArray -> (0 until arr.length()).map { arr.getString(it) } }
            val a = toList(o.getJSONArray("a"))
            val c = toList(o.getJSONArray("c"))
            val bOuter = o.getJSONArray("b")
            val b = (0 until bOuter.length()).map { toList(bOuter.getJSONArray(it)) }
            val pub = toList(o.getJSONArray("pub"))
            ProofFfi(a, b, c, pub)
        } catch (e: Exception) {
            null
        }
    }

    /** Pass-through EdDSA signature fields for [proveOnServer] (mirrors the UniFFI `EddsaSigInput`). */
    data class ProverEddsaSig(
        val r8xDec: String,
        val r8yDec: String,
        val sDec: String,
        val axHex: String,
        val ayHex: String,
    )

    /**
     * Poll the export-session status on the GROOMER host: GET /verify/session/{id}?token=<token>.
     * Returns the parsed `{status, txHash}` (or null on failure). Status flips to `recorded` once the
     * relayer's `recordVerificationZK` tx confirms.
     */
    data class SessionStatus(val status: String, val txHash: String?)

    suspend fun verifySessionStatus(host: String, sessionId: String, token: String): SessionStatus? {
        if (sessionId.isBlank()) return null
        return try {
            val resp = Http.getJson("$host/verify/session/$sessionId?token=$token")
            if (!resp.ok) return null
            val o = JSONObject(resp.body)
            SessionStatus(
                status = o.optString("status", ""),
                txHash = o.optString("txHash", o.optString("tx_hash", "")).ifBlank { null },
            )
        } catch (e: Exception) {
            null
        }
    }
}
