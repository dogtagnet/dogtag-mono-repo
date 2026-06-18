package io.liberalize.dogtag.consent

import org.json.JSONObject

// UniFFI consent surface.
import uniffi.dogtag_standard.EddsaSignatureFfi
import uniffi.dogtag_standard.consentNullifierHex
import uniffi.dogtag_standard.eddsaConsentMessageHex
import uniffi.dogtag_standard.signConsentEddsa
import uniffi.dogtag_standard.verificationConsentTypehashHex

/** The proof mode a verifier requests. */
enum class ConsentMode { ECDSA, ZK }

/**
 * A verification request scanned from a verifier's QR. Mirrors the on-chain VerificationConsent
 * (impl §11.9). All 32-byte / address fields are 0x.. hex. Missing fields default to zero.
 */
data class VerificationRequest(
    val mode: ConsentMode,
    val dogTagId: String,
    val recordType: String,
    val purpose: String,
    val credentialRoot: String,
    val challenge: String,
    val relayer: String,
    val subject: String,
    val nonce: String,
    val deadline: String,
    val verifierName: String,
    val callbackUrl: String?, // central /v1/verify/consent endpoint
) {
    companion object {
        private const val ZERO32 = "0x0000000000000000000000000000000000000000000000000000000000000000"
        private const val ZERO20 = "0x0000000000000000000000000000000000000000"

        /** Parse the JSON payload a verifier encodes in its QR. */
        fun parse(qr: String): VerificationRequest {
            val o = JSONObject(qr)
            val mode = if (o.optString("mode", "ecdsa").lowercase() == "zk") ConsentMode.ZK else ConsentMode.ECDSA
            fun w(k: String) = o.optString(k, ZERO32).let { if (it.isBlank()) ZERO32 else it }
            fun a(k: String) = o.optString(k, ZERO20).let { if (it.isBlank()) ZERO20 else it }
            return VerificationRequest(
                mode = mode,
                dogTagId = w("dogTagId"),
                recordType = w("recordType"),
                purpose = w("purpose"),
                credentialRoot = w("credentialRoot"),
                challenge = w("challenge"),
                relayer = a("relayer"),
                subject = a("subject"),
                nonce = w("nonce"),
                deadline = w("deadline"),
                verifierName = o.optString("verifier", "Unknown verifier"),
                callbackUrl = o.optString("callback", "").ifBlank { null },
            )
        }
    }
}

/** The signed consent artifact ready to POST to the central /v1/verify/consent. */
data class SignedConsent(
    val mode: ConsentMode,
    val nullifier: String,
    val message: String,
    val typehash: String,
    val eddsa: EddsaSignatureFfi?, // ZK path
    val payloadJson: String,
)

/**
 * Build a signed consent for a request. For the ZK path we EdDSA-BabyJubjub-sign the §1.10 message
 * via the new FFI; for the ECDSA path the wallet would ECDSA-sign the EIP-712 digest (the digest /
 * nullifier / typehash come from the FFI; the actual secp256k1 signature is produced by the wallet —
 * here we surface the digest fields so the central can finish the ECDSA leg).
 */
object ConsentSigner {
    fun sign(req: VerificationRequest, consentPrivHex: String?): SignedConsent {
        val nullifier = consentNullifierHex(
            req.dogTagId, req.recordType, req.purpose, req.credentialRoot, req.challenge,
            req.relayer, req.subject, req.nonce, req.deadline,
        )
        val message = eddsaConsentMessageHex(
            req.dogTagId, req.recordType, req.purpose, req.credentialRoot, req.challenge,
            req.relayer, req.subject, req.nonce, req.deadline,
        )
        val typehash = verificationConsentTypehashHex()

        val eddsa = if (req.mode == ConsentMode.ZK && consentPrivHex != null) {
            signConsentEddsa(
                consentPrivHex,
                req.dogTagId, req.recordType, req.purpose, req.credentialRoot, req.challenge,
                req.relayer, req.subject, req.nonce, req.deadline,
            )
        } else null

        val payload = JSONObject().apply {
            put("mode", req.mode.name.lowercase())
            put("dogTagId", req.dogTagId)
            put("recordType", req.recordType)
            put("purpose", req.purpose)
            put("credentialRoot", req.credentialRoot)
            put("challenge", req.challenge)
            put("relayer", req.relayer)
            put("subject", req.subject)
            put("nonce", req.nonce)
            put("deadline", req.deadline)
            put("nullifier", nullifier)
            put("message", message)
            if (eddsa != null) {
                put("eddsaSig", JSONObject().apply {
                    put("R8x", eddsa.r8xDec)
                    put("R8y", eddsa.r8yDec)
                    put("S", eddsa.sDec)
                })
            }
        }
        return SignedConsent(req.mode, nullifier, message, typehash, eddsa, payload.toString())
    }
}
