import Foundation

/// Minimal async JSON HTTP (URLSession) for the central + vet + ROAX endpoints (mirrors Android Http).
enum Http {
    struct Response { let code: Int; let body: String; var ok: Bool { (200..<300).contains(code) } }

    static func getJSON(_ url: String, bearer: String? = nil) async -> Response {
        await request(url, method: "GET", body: nil, bearer: bearer)
    }

    static func postJSON(_ url: String, body: String, bearer: String? = nil) async -> Response {
        await request(url, method: "POST", body: body, bearer: bearer)
    }

    private static func request(_ url: String, method: String, body: String?, bearer: String?) async -> Response {
        guard let u = URL(string: url) else { return Response(code: -1, body: "bad url") }
        var req = URLRequest(url: u, timeoutInterval: 8)
        req.httpMethod = method
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        if let bearer = bearer { req.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization") }
        if let body = body {
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            req.httpBody = body.data(using: .utf8)
        }
        do {
            let (data, resp) = try await URLSession.shared.data(for: req)
            let code = (resp as? HTTPURLResponse)?.statusCode ?? -1
            return Response(code: code, body: String(data: data, encoding: .utf8) ?? "")
        } catch {
            return Response(code: -1, body: error.localizedDescription)
        }
    }
}

/// Read-only JSON-RPC client for the ROAX chain (chainId 135). Re-checks `DogTagIssuer.isValid(root)`.
/// The RPC may be unreachable (502 at design time) — callers treat failure as UNKNOWN, never a hard fail.
enum RoaxRpc {
    enum Result { case valid, invalid, unknown(String) }

    private static let isValidSelector = "0x6d04f0bc"   // keccak256("isValid(bytes32)")[:4]

    static func isValid(rpcUrl: String, documentStore: String, root: String) async -> Result {
        guard !documentStore.isEmpty, !root.isEmpty else { return .unknown("missing addr/root") }
        let data = isValidSelector + pad32(root)
        let payload: [String: Any] = [
            "jsonrpc": "2.0", "id": 1, "method": "eth_call",
            "params": [["to": documentStore, "data": data], "latest"],
        ]
        guard let raw = try? JSONSerialization.data(withJSONObject: payload),
              let bodyStr = String(data: raw, encoding: .utf8) else { return .unknown("encode") }
        let resp = await Http.postJSON(rpcUrl, body: bodyStr)
        guard resp.ok else { return .unknown("rpc \(resp.code)") }
        guard let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any] else {
            return .unknown("bad rpc json")
        }
        if let err = o["error"] as? [String: Any] {
            return .unknown((err["message"] as? String) ?? "rpc error")
        }
        let result = (o["result"] as? String) ?? ""
        let hex = result.hasPrefix("0x") ? String(result.dropFirst(2)) : result
        if hex.isEmpty { return .unknown("empty result") }
        let truthy = hex.contains { $0 != "0" }
        return truthy ? .valid : .invalid
    }

    private static let isWhitelistedForSelector = "0x779c3985" // keccak256("isWhitelistedFor(bytes32,address)")[:4]
    private static let bindNonceSelector = "0x15c95be6"        // keccak256("bindNonce(address)")[:4]
    private static let keyOfSelector = "0xfa073d76"            // keccak256("keyOf(address)")[:4]

    /// `IssuerRegistry.isWhitelistedFor(key, signer)` — the PRE-PROOF groomer check. On Unknown the
    /// caller MUST hard-stop (this gate is a user-safety requirement → unknown == not authorized).
    static func isWhitelistedFor(rpcUrl: String, issuerRegistry: String, key: String, signer: String) async -> Result {
        guard !issuerRegistry.isEmpty, !key.isEmpty, !signer.isEmpty else { return .unknown("missing addr/key/signer") }
        let data = isWhitelistedForSelector + pad32(key) + padAddr(signer)
        switch await ethCall(rpcUrl: rpcUrl, to: issuerRegistry, data: data) {
        case let .success(hex):
            let truthy = hex.contains { $0 != "0" }
            return truthy ? .valid : .invalid
        case let .failure(reason):
            return .unknown(reason)
        }
    }

    /// `ConsentKeyRegistry.bindNonce(subject)` → the current bind nonce (or nil on failure).
    static func bindNonce(rpcUrl: String, consentKeyRegistry: String, subject: String) async -> UInt64? {
        guard !consentKeyRegistry.isEmpty, !subject.isEmpty else { return nil }
        let data = bindNonceSelector + padAddr(subject)
        switch await ethCall(rpcUrl: rpcUrl, to: consentKeyRegistry, data: data) {
        case let .success(hex): return UInt64(hex.suffix(16), radix: 16) ?? UInt64(hex, radix: 16) ?? 0
        case .failure: return nil
        }
    }

    /// `VerificationRegistry.keyOf(subject)` → bound consent keyHash (0x..), or nil.
    static func keyOf(rpcUrl: String, verificationRegistry: String, subject: String) async -> String? {
        guard !verificationRegistry.isEmpty, !subject.isEmpty else { return nil }
        let data = keyOfSelector + padAddr(subject)
        switch await ethCall(rpcUrl: rpcUrl, to: verificationRegistry, data: data) {
        case let .success(hex): return "0x" + String(repeating: "0", count: max(0, 64 - hex.count)) + hex
        case .failure: return nil
        }
    }

    private enum CallResult { case success(String); case failure(String) }

    private static func ethCall(rpcUrl: String, to: String, data: String) async -> CallResult {
        let payload: [String: Any] = [
            "jsonrpc": "2.0", "id": 1, "method": "eth_call",
            "params": [["to": to, "data": data], "latest"],
        ]
        guard let raw = try? JSONSerialization.data(withJSONObject: payload),
              let bodyStr = String(data: raw, encoding: .utf8) else { return .failure("encode") }
        let resp = await Http.postJSON(rpcUrl, body: bodyStr)
        guard resp.ok else { return .failure("rpc \(resp.code)") }
        guard let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any] else {
            return .failure("bad rpc json")
        }
        if let err = o["error"] as? [String: Any] { return .failure((err["message"] as? String) ?? "rpc error") }
        let result = (o["result"] as? String) ?? ""
        return .success(result.hasPrefix("0x") ? String(result.dropFirst(2)) : result)
    }

    private static func padAddr(_ addr: String) -> String {
        let h = (addr.hasPrefix("0x") ? String(addr.dropFirst(2)) : addr).lowercased()
        return String(repeating: "0", count: max(0, 64 - h.count)) + h
    }

    private static func pad32(_ hex: String) -> String {
        let h = hex.hasPrefix("0x") ? String(hex.dropFirst(2)) : hex
        return String(repeating: "0", count: max(0, 64 - h.count)) + h
    }
}

/// Typed client for the central (admin) API: pet sync + the verify-consent relay (mirrors Android).
enum CentralApi {
    static func listPets(sessionToken: String?) async -> [Pet] {
        guard let token = sessionToken, !token.isEmpty else { return [] }
        let resp = await Http.getJSON("\(AppConfig.centralApi)/v1/pets", bearer: token)
        guard resp.ok, let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any],
              let arr = o["pets"] as? [[String: Any]] else { return [] }
        return arr.map { Pet.fromCentral($0) }.filter { !$0.dogTagId.isEmpty }
    }

    static func postConsent(sessionToken: String?, payloadJson: String) async -> Http.Response {
        await Http.postJSON("\(AppConfig.centralApi)/v1/verify/consent", body: payloadJson, bearer: sessionToken)
    }

    /// ZK path: POST the proof bundle directly to the GROOMER host (scanned QR origin), NOT central.
    /// The body's `sessionJwt` is the auth (no bearer). The groomer relays `recordVerificationZK`.
    static func postVerifyConsentToHost(host: String, payloadJson: String) async -> Http.Response {
        await Http.postJSON("\(host)/v1/verify/consent", body: payloadJson)
    }

    struct SessionStatus { let status: String; let txHash: String? }

    /// Poll GET <host>/verify/session/{id} (session-JWT auth) → {status, txHash}.
    static func verifySessionStatus(host: String, sessionId: String, sessionJwt: String) async -> SessionStatus? {
        guard !sessionId.isEmpty else { return nil }
        let resp = await Http.getJSON("\(host)/verify/session/\(sessionId)", bearer: sessionJwt)
        guard resp.ok, let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any] else { return nil }
        let status = (o["status"] as? String) ?? ""
        let tx = (o["txHash"] as? String) ?? (o["tx_hash"] as? String)
        return SessionStatus(status: status, txHash: (tx?.isEmpty == false) ? tx : nil)
    }
}
