import Foundation

/// The proof mode a verifier requests.
enum ConsentMode: String { case ecdsa, zk }

private let ZERO32 = "0x0000000000000000000000000000000000000000000000000000000000000000"
private let ZERO20 = "0x0000000000000000000000000000000000000000"

/// A verification request scanned from a verifier's QR combined with the SPECIFIC stored record the
/// user chose to present (impl §1.10). The verifier supplies relayer/purpose/challenge/recordType
/// (from its /v?t= JWT); the user supplies `credentialRoot` (the selected record's merkleRoot) and
/// `subject` (their wallet).
struct VerificationRequest {
    let mode: ConsentMode
    let sessionJwt: String
    let callbackUrl: String?
    let verifierName: String
    let purposeLabel: String
    let recordTypeLabel: String
    // consent fields (all 0x.. hex)
    let dogTagId: String
    let recordType: String
    let purpose: String
    let credentialRoot: String
    let challenge: String
    let relayer: String
    let subject: String
    let nonce: String
    let deadline: String

    /// keccak256(label) -> 0x.. bytes32 (recordType / purpose namespacing — §3 keep-list).
    static func keccakLabel(_ label: String) -> String {
        if label.isEmpty { return ZERO32 }
        if label.hasPrefix("0x") && label.count == 66 { return label }
        let h = Keccak256.digest(Data(label.utf8))
        return "0x" + h.map { String(format: "%02x", $0) }.joined()
    }

    private static func dogTagIdToHex(_ dec: String) -> String {
        // Decimal/hex dogTagId -> 0x.. 32-byte. Fall back to zero on parse failure.
        let s = dec.hasPrefix("0x") ? String(dec.dropFirst(2)) : dec
        let radix = dec.hasPrefix("0x") ? 16 : 10
        guard let n = UInt64(s, radix: radix) else { return ZERO32 }
        let hex = String(n, radix: 16)
        return "0x" + String(repeating: "0", count: max(0, 64 - hex.count)) + hex
    }

    /// Build a consent request from the scanned verify-session + the record the user selected.
    static func from(session: QrPayload, dogTagIdDec: String, credentialRoot: String,
                     subjectWallet: String?, callbackUrl: String?) -> VerificationRequest? {
        guard case let .verifySession(_, jwt, relayer, purpose, recordType, challenge, mode, _) = session else {
            return nil
        }
        let m: ConsentMode = (mode.lowercased() == "normal" || mode.lowercased() == "ecdsa") ? .ecdsa : .zk
        let now = UInt64(Date().timeIntervalSince1970)
        let deadlineHex = String(now + 300, radix: 16)
        let nonceHex = String(UInt64(Date().timeIntervalSince1970 * 1000), radix: 16)
        return VerificationRequest(
            mode: m,
            sessionJwt: jwt,
            callbackUrl: callbackUrl,
            verifierName: relayer.isEmpty ? "Verifier" : relayer,
            purposeLabel: purpose.isEmpty ? "verification" : purpose,
            recordTypeLabel: recordType.isEmpty ? "record" : recordType,
            dogTagId: dogTagIdToHex(dogTagIdDec),
            recordType: keccakLabel(recordType),
            purpose: keccakLabel(purpose),
            credentialRoot: credentialRoot.isEmpty ? ZERO32 : credentialRoot,
            challenge: challenge.isEmpty ? ZERO32 : challenge,
            relayer: relayer.isEmpty ? ZERO20 : relayer,
            subject: (subjectWallet?.isEmpty == false) ? subjectWallet! : ZERO20,
            nonce: "0x" + String(repeating: "0", count: max(0, 64 - nonceHex.count)) + nonceHex,
            deadline: "0x" + String(repeating: "0", count: max(0, 64 - deadlineHex.count)) + deadlineHex
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

/// The gasless consent-key bind block. The RELAYER (groomer) submits
/// `ConsentKeyRegistry.bindConsentKeyFor(subject, keyHash, ownerSig)` so the owner NEVER pays gas.
struct ConsentKeyBind { let subject: String; let keyHash: String; let ownerSig: String }

/// Build a signed consent over the SELECTED record's root. The POST body matches the central
/// `/v1/verify/consent` contract: `{ sessionJwt, consent, sig, mode }` (+ `proof`/`bind` on the ZK path).
enum ConsentSigner {
    static func sign(_ req: VerificationRequest, consentPrivHex: String?,
                     proof: ProofFfi? = nil, bind: ConsentKeyBind? = nil) throws -> SignedConsent {
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

        let consent: [String: Any] = [
            "dogTagId": req.dogTagId, "recordType": req.recordType, "purpose": req.purpose,
            "credentialRoot": req.credentialRoot, "challenge": req.challenge, "relayer": req.relayer,
            "subject": req.subject, "nonce": req.nonce, "deadline": req.deadline,
            "nullifier": nullifier, "message": message,
        ]
        var sig = ""
        if let e = eddsa {
            let sigObj: [String: Any] = ["R8x": e.r8xDec, "R8y": e.r8yDec, "S": e.sDec]
            sig = String(data: (try? JSONSerialization.data(withJSONObject: sigObj)) ?? Data(), encoding: .utf8) ?? ""
        }
        var payload: [String: Any] = [
            "sessionJwt": req.sessionJwt, "consent": consent, "sig": sig, "mode": req.mode.rawValue,
        ]
        if let p = proof {
            payload["proof"] = [
                "a": p.a, "b": p.b, "c": p.c, "pubSignals": p.pubSignals,
            ]
        }
        if let b = bind {
            payload["bind"] = ["subject": b.subject, "keyHash": b.keyHash, "ownerSig": b.ownerSig]
        }
        let json = String(data: try JSONSerialization.data(withJSONObject: payload), encoding: .utf8) ?? "{}"
        return SignedConsent(mode: req.mode, nullifier: nullifier, message: message,
                             typehash: typehash, eddsa: eddsa, payloadJson: json)
    }
}
