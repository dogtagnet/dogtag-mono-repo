import Foundation

/// The proof mode a verifier requests.
enum ConsentMode: String {
    case ecdsa, zk
}

private let ZERO32 = "0x0000000000000000000000000000000000000000000000000000000000000000"
private let ZERO20 = "0x0000000000000000000000000000000000000000"

/// A verification request scanned from a verifier's QR. Mirrors the on-chain VerificationConsent
/// (impl §11.9). All 32-byte / address fields are 0x.. hex. Missing fields default to zero.
struct VerificationRequest {
    let mode: ConsentMode
    let dogTagId: String
    let recordType: String
    let purpose: String
    let credentialRoot: String
    let challenge: String
    let relayer: String
    let subject: String
    let nonce: String
    let deadline: String
    let verifierName: String
    let callbackUrl: String?

    /// Parse the JSON payload a verifier encodes in its QR.
    static func parse(_ qr: String) -> VerificationRequest? {
        guard let data = qr.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            return nil
        }
        func w(_ k: String) -> String {
            let v = (o[k] as? String) ?? ZERO32
            return v.isEmpty ? ZERO32 : v
        }
        func a(_ k: String) -> String {
            let v = (o[k] as? String) ?? ZERO20
            return v.isEmpty ? ZERO20 : v
        }
        let modeStr = ((o["mode"] as? String) ?? "ecdsa").lowercased()
        let mode: ConsentMode = (modeStr == "zk") ? .zk : .ecdsa
        let cb = (o["callback"] as? String) ?? ""
        return VerificationRequest(
            mode: mode,
            dogTagId: w("dogTagId"),
            recordType: w("recordType"),
            purpose: w("purpose"),
            credentialRoot: w("credentialRoot"),
            challenge: w("challenge"),
            relayer: a("relayer"),
            subject: a("subject"),
            nonce: w("nonce"),
            deadline: w("deadline"),
            verifierName: (o["verifier"] as? String) ?? "Unknown verifier",
            callbackUrl: cb.isEmpty ? nil : cb
        )
    }
}

/// The signed consent artifact ready to POST to the central /v1/verify/consent.
struct SignedConsent {
    let mode: ConsentMode
    let nullifier: String
    let message: String
    let typehash: String
    let eddsa: EddsaSignatureFfi?
    let payloadJson: String
}

/// Build a signed consent for a request. For the ZK path we EdDSA-BabyJubjub-sign the §1.10 message
/// via the FFI; for the ECDSA path we surface the digest/nullifier/typehash from the FFI so the
/// central can finish the secp256k1 leg (the wallet ECDSA-signing is handled elsewhere / device-side).
enum ConsentSigner {
    static func sign(_ req: VerificationRequest, consentPrivHex: String?) throws -> SignedConsent {
        let nullifier = try consentNullifierHex(
            dogTagIdHex: req.dogTagId, recordTypeHex: req.recordType, purposeHex: req.purpose,
            credentialRootHex: req.credentialRoot, challengeHex: req.challenge,
            relayerHex: req.relayer, subjectHex: req.subject, nonceHex: req.nonce, deadlineHex: req.deadline)
        let message = try eddsaConsentMessageHex(
            dogTagIdHex: req.dogTagId, recordTypeHex: req.recordType, purposeHex: req.purpose,
            credentialRootHex: req.credentialRoot, challengeHex: req.challenge,
            relayerHex: req.relayer, subjectHex: req.subject, nonceHex: req.nonce, deadlineHex: req.deadline)
        let typehash = verificationConsentTypehashHex()

        var eddsa: EddsaSignatureFfi? = nil
        if req.mode == .zk, let priv = consentPrivHex {
            eddsa = try signConsentEddsa(
                prvHex: priv,
                dogTagIdHex: req.dogTagId, recordTypeHex: req.recordType, purposeHex: req.purpose,
                credentialRootHex: req.credentialRoot, challengeHex: req.challenge,
                relayerHex: req.relayer, subjectHex: req.subject, nonceHex: req.nonce, deadlineHex: req.deadline)
        }

        var payload: [String: Any] = [
            "mode": req.mode.rawValue,
            "dogTagId": req.dogTagId,
            "recordType": req.recordType,
            "purpose": req.purpose,
            "credentialRoot": req.credentialRoot,
            "challenge": req.challenge,
            "relayer": req.relayer,
            "subject": req.subject,
            "nonce": req.nonce,
            "deadline": req.deadline,
            "nullifier": nullifier,
            "message": message,
        ]
        if let e = eddsa {
            payload["eddsaSig"] = ["R8x": e.r8xDec, "R8y": e.r8yDec, "S": e.sDec]
        }
        let json = String(data: try JSONSerialization.data(withJSONObject: payload), encoding: .utf8) ?? "{}"
        return SignedConsent(mode: req.mode, nullifier: nullifier, message: message,
                             typehash: typehash, eddsa: eddsa, payloadJson: json)
    }
}
