package io.liberalize.dogtag.consent

import io.liberalize.dogtag.qr.QrPayload
import io.liberalize.dogtag.wallet.Keccak256
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
 * A verification request scanned from a verifier's QR, combined with the SPECIFIC stored record the
 * user chose to present. Mirrors the on-chain VerificationConsent (impl §1.10). All 32-byte / address
 * fields are 0x.. hex. The verifier supplies relayer/purpose/challenge/recordType (from its /v?t= JWT);
 * the user supplies `credentialRoot` (the merkleRoot of the record they selected) + `subject` (their
 * wallet).
 */
data class VerificationRequest(
    val mode: ConsentMode,
    val sessionJwt: String,
    val callbackUrl: String?,   // central /v1/verify/consent endpoint
    val verifierName: String,
    val purposeLabel: String,
    val recordTypeLabel: String,
    // consent fields (all 0x.. hex)
    val dogTagId: String,
    val recordType: String,
    val purpose: String,
    val credentialRoot: String,
    val challenge: String,
    val relayer: String,
    val subject: String,
    val nonce: String,
    val deadline: String,
) {
    companion object {
        const val ZERO32 = "0x0000000000000000000000000000000000000000000000000000000000000000"
        const val ZERO20 = "0x0000000000000000000000000000000000000000"

        /** keccak256(label) -> 0x.. bytes32 (recordType / purpose namespacing — §3 keep-list). */
        fun keccakLabel(label: String): String {
            if (label.isBlank()) return ZERO32
            // accept an already-hashed 0x bytes32 verbatim.
            if (label.startsWith("0x") && label.length == 66) return label
            val h = Keccak256.digest(label.toByteArray(Charsets.UTF_8))
            return "0x" + h.joinToString("") { "%02x".format(it) }
        }

        private fun dogTagIdToHex(dec: String): String {
            return try {
                val n = java.math.BigInteger(dec.removePrefix("0x").ifBlank { "0" },
                    if (dec.startsWith("0x")) 16 else 10)
                "0x" + n.toString(16).padStart(64, '0')
            } catch (e: Exception) {
                ZERO32
            }
        }

        /**
         * Build a consent request from the scanned verify-session and the record the user selected to
         * present. `subjectWallet` is the user's secp256k1 address.
         */
        fun from(
            session: QrPayload.VerifySession,
            dogTagIdDec: String,
            credentialRoot: String,
            subjectWallet: String?,
            callbackUrl: String?,
        ): VerificationRequest {
            val mode = if (session.mode.lowercase() == "normal" || session.mode.lowercase() == "ecdsa")
                ConsentMode.ECDSA else ConsentMode.ZK
            val deadline = "0x" + java.math.BigInteger.valueOf(
                (System.currentTimeMillis() / 1000) + 300,
            ).toString(16).padStart(64, '0')
            val nonce = "0x" + java.math.BigInteger.valueOf(System.currentTimeMillis()).toString(16).padStart(64, '0')
            return VerificationRequest(
                mode = mode,
                sessionJwt = session.jwt,
                callbackUrl = callbackUrl,
                verifierName = session.relayer.ifBlank { "Verifier" },
                purposeLabel = session.purpose.ifBlank { "verification" },
                recordTypeLabel = session.recordType.ifBlank { "record" },
                dogTagId = dogTagIdToHex(dogTagIdDec),
                recordType = keccakLabel(session.recordType),
                purpose = keccakLabel(session.purpose),
                credentialRoot = if (credentialRoot.isBlank()) ZERO32 else credentialRoot,
                challenge = session.challenge.ifBlank { ZERO32 },
                relayer = session.relayer.ifBlank { ZERO20 },
                subject = subjectWallet?.ifBlank { ZERO20 } ?: ZERO20,
                nonce = nonce,
                deadline = deadline,
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
 * Build a signed consent for a request over the SELECTED record's root. For the ZK path we
 * EdDSA-BabyJubjub-sign the §1.10 message via the FFI; for the ECDSA path the central finishes the
 * ECDSA leg from the surfaced digest fields. The POST body matches the central `/v1/verify/consent`
 * contract: `{ sessionJwt, consent, sig, mode }`.
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

        val consent = JSONObject().apply {
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
        }
        val sig = if (eddsa != null) {
            JSONObject().apply {
                put("R8x", eddsa.r8xDec)
                put("R8y", eddsa.r8yDec)
                put("S", eddsa.sDec)
            }.toString()
        } else ""

        val payload = JSONObject().apply {
            put("sessionJwt", req.sessionJwt)
            put("consent", consent)
            put("sig", sig)
            put("mode", req.mode.name.lowercase())
        }
        return SignedConsent(req.mode, nullifier, message, typehash, eddsa, payload.toString())
    }
}
