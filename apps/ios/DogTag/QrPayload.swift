import Foundation

/// The two scan outcomes the user app supports. The pet owner's app ONLY scans — it never shows a QR.
/// Detect which kind by the URL shape (architecture §7, impl §3.9 / §6.5).
///   - Import (issuer -> user, SHORT token): `https://<vet-host>/r/<32hex>` — preferred, low-density QR.
///   - Import (issuer -> user, legacy JWT): `https://<vet-host>/r?t=<jwt>&i=<recordId>` (back-compat).
///   - Export (user -> groomer, SHORT token): `https://<host>/x/<token>?a=<relayerAddr>` — one-time
///     token + groomer wallet address. The phone resolves `/x/<token>` for the session metadata,
///     DNS-verifies the groomer (prod/remote), proves on-device, and POSTs the proof using the token.
enum QrPayload {
    case importRecord(host: String, recordId: String, jwt: String)
    /// A SHORT one-time share token — fetch GET <host>/r/<token> (no Bearer); the server consumes it.
    case importRecordToken(host: String, token: String)
    /// An export-session one-time token plus the groomer's wallet/relayer address (`/x/<token>?a=<addr>`).
    case exportSession(host: String, token: String, groomerAddr: String)
    /// A dog-tag ISSUANCE session — the vet displays `/p/<token>` (32 hex, one-time, 180s). The phone
    /// POSTs `<host>/profiles/issue/bind { token, walletAddress, signature }`, verifies the returned
    /// DOG_PROFILE against the DogTagSBT, and stores it as a credential.
    case dogTagIssueSession(host: String, token: String)
    case unknown(String)

    static func parse(_ raw: String) -> QrPayload {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let comps = URLComponents(string: trimmed),
              let scheme = comps.scheme, let host = comps.host else { return .unknown(trimmed) }
        var origin = "\(scheme)://\(host)"
        if let port = comps.port { origin += ":\(port)" }
        let path = comps.path.hasSuffix("/") ? String(comps.path.dropLast()) : comps.path
        let q = Dictionary(uniqueKeysWithValues: (comps.queryItems ?? []).map { ($0.name, $0.value ?? "") })
        let segs = path.split(separator: "/").map(String.init)

        // SHORT one-time share token: `/r/<token>` (no query string). Preferred.
        if segs.count == 2, segs[0] == "r", comps.queryItems?.isEmpty ?? true {
            let token = segs[1]
            return token.isEmpty ? .unknown(trimmed) : .importRecordToken(host: origin, token: token)
        }

        // Export session one-time token: `/x/<token>?a=<groomerAddr>`.
        if segs.count == 2, segs[0] == "x" {
            let token = segs[1]
            let addr = q["a"] ?? ""
            return (!token.isEmpty && !addr.isEmpty)
                ? .exportSession(host: origin, token: token, groomerAddr: addr)
                : .unknown(trimmed)
        }

        // Dog-tag issuance one-time token: `/p/<token>` (no query string).
        if segs.count == 2, segs[0] == "p", comps.queryItems?.isEmpty ?? true {
            let token = segs[1]
            return token.isEmpty ? .unknown(trimmed) : .dogTagIssueSession(host: origin, token: token)
        }

        switch path {
        case "/r":
            let t = q["t"] ?? "", i = q["i"] ?? ""
            return (!t.isEmpty && !i.isEmpty) ? .importRecord(host: origin, recordId: i, jwt: t) : .unknown(trimmed)
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
