import Foundation

/// The two scan outcomes the user app supports. The pet owner's app ONLY scans — it never shows a QR.
/// Detect which kind by the URL shape (architecture §7, impl §3.9 / §6.5).
///   - Import (issuer -> user): `https://<vet-host>/r?t=<jwt>&i=<recordId>`
///   - Verify (verifier -> user): `https://<host>/v?t=<jwt>` (JWT carries relayer/purpose/challenge/recordType)
enum QrPayload {
    case importRecord(host: String, recordId: String, jwt: String)
    case verifySession(host: String, jwt: String, relayer: String, purpose: String,
                       recordType: String, challenge: String, mode: String, sessionId: String)
    case unknown(String)

    static func parse(_ raw: String) -> QrPayload {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let comps = URLComponents(string: trimmed),
              let scheme = comps.scheme, let host = comps.host else { return .unknown(trimmed) }
        var origin = "\(scheme)://\(host)"
        if let port = comps.port { origin += ":\(port)" }
        let path = comps.path.hasSuffix("/") ? String(comps.path.dropLast()) : comps.path
        let q = Dictionary(uniqueKeysWithValues: (comps.queryItems ?? []).map { ($0.name, $0.value ?? "") })

        switch path {
        case "/r":
            let t = q["t"] ?? "", i = q["i"] ?? ""
            return (!t.isEmpty && !i.isEmpty) ? .importRecord(host: origin, recordId: i, jwt: t) : .unknown(trimmed)
        case "/v":
            let t = q["t"] ?? ""
            guard !t.isEmpty else { return .unknown(trimmed) }
            let c = decodeJwtClaims(t)
            return .verifySession(
                host: origin, jwt: t,
                relayer: (c["relayer"] as? String) ?? "",
                purpose: (c["purpose"] as? String) ?? "",
                recordType: (c["recordType"] as? String) ?? (c["record_type"] as? String) ?? "",
                challenge: (c["challenge"] as? String) ?? "",
                mode: (c["mode"] as? String) ?? "zk",
                sessionId: (c["sub"] as? String) ?? ""
            )
        default:
            return .unknown(trimmed)
        }
    }

    /// Decode the (untrusted) JWT payload to read claims for display + consent fields.
    static func decodeJwtClaims(_ jwt: String) -> [String: Any] {
        let parts = jwt.split(separator: ".")
        guard parts.count >= 2 else { return [:] }
        var b64 = String(parts[1]).replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while b64.count % 4 != 0 { b64 += "=" }
        guard let data = Data(base64Encoded: b64),
              let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return [:] }
        return o
    }
}
