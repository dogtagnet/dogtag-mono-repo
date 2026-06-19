import Foundation

/// Minimal async JSON HTTP (URLSession) for the central + vet + ROAX endpoints (mirrors Android Http).
enum Http {
    struct Response { let code: Int; let body: String; var ok: Bool { (200..<300).contains(code) } }

    static func getJSON(_ url: String, bearer: String? = nil) async -> Response {
        await request(url, method: "GET", body: nil, bearer: bearer)
    }

    /// GET with an explicit Accept header (e.g. DoH `application/dns-json`).
    static func getJSON(_ url: String, accept: String) async -> Response {
        guard let u = URL(string: url) else { return Response(code: -1, body: "bad url") }
        var req = URLRequest(url: u, timeoutInterval: 8)
        req.httpMethod = "GET"
        req.setValue(accept, forHTTPHeaderField: "Accept")
        do {
            let (data, resp) = try await URLSession.shared.data(for: req)
            let code = (resp as? HTTPURLResponse)?.statusCode ?? -1
            return Response(code: code, body: String(data: data, encoding: .utf8) ?? "")
        } catch {
            return Response(code: -1, body: error.localizedDescription)
        }
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

    /// The export-session metadata resolved (non-consuming) from the QR's one-time token before proving.
    struct ExportSession {
        let sessionId: String
        let relayer: String
        let purpose: String
        let recordType: String
        let challenge: String
        let mode: String
    }

    /// GET <host>/x/<token> → export-session metadata (non-consuming). Nil on failure.
    static func resolveExportSession(host: String, token: String) async -> ExportSession? {
        guard !token.isEmpty else { return nil }
        let resp = await Http.getJSON("\(host)/x/\(token)")
        guard resp.ok, let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any] else { return nil }
        return ExportSession(
            sessionId: (o["sessionId"] as? String) ?? (o["session_id"] as? String) ?? "",
            relayer: (o["relayer"] as? String) ?? "",
            purpose: (o["purpose"] as? String) ?? "",
            recordType: (o["recordType"] as? String) ?? (o["record_type"] as? String) ?? "",
            challenge: (o["challenge"] as? String) ?? "",
            mode: (o["mode"] as? String) ?? "zk")
    }

    /// ZK path: POST the proof bundle directly to the GROOMER host (scanned QR origin), NOT central.
    /// The body carries the one-time `exportToken` (no bearer, consumed on submit). The groomer relays
    /// `recordVerificationZK`.
    static func postVerifyConsentToHost(host: String, payloadJson: String) async -> Http.Response {
        await Http.postJSON("\(host)/v1/verify/consent", body: payloadJson)
    }

    struct SessionStatus { let status: String; let txHash: String? }

    /// Poll GET <host>/verify/session/{id}?token=<token> → {status, txHash}.
    static func verifySessionStatus(host: String, sessionId: String, token: String) async -> SessionStatus? {
        guard !sessionId.isEmpty else { return nil }
        let resp = await Http.getJSON("\(host)/verify/session/\(sessionId)?token=\(token)")
        guard resp.ok, let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any] else { return nil }
        let status = (o["status"] as? String) ?? ""
        let tx = (o["txHash"] as? String) ?? (o["tx_hash"] as? String)
        return SessionStatus(status: status, txHash: (tx?.isEmpty == false) ? tx : nil)
    }
}

/// Phone-side DNS verification of the groomer (mirrors `stacks/admin/api/src/dns.rs`). The export QR
/// carries the groomer's wallet address; before disclosing a proof the phone requires the groomer's
/// DOMAIN to publish a TXT `dogtag-verify=<groomerAddr lowercased>` resolved via DoH (Cloudflare). This
/// is enforced ONLY for real public domains; LOCAL hosts (IP literal / localhost / *.local / LAN) skip it.
enum DnsVerify {
    /// The canonical TXT a groomer must publish to prove control of its domain.
    static func expectedTxt(_ groomerAddr: String) -> String { "dogtag-verify=\(groomerAddr.lowercased())" }

    /// Strip scheme/port/path from an origin or host string → bare host.
    static func hostOnly(_ host: String) -> String {
        var h = host.trimmingCharacters(in: .whitespaces)
        if let r = h.range(of: "://") { h = String(h[r.upperBound...]) }
        if let slash = h.firstIndex(of: "/") { h = String(h[..<slash]) }
        if h.hasPrefix("["), let close = h.firstIndex(of: "]") {  // [IPv6]
            return String(h[h.index(after: h.startIndex)..<close])
        }
        // strip :port for IPv4/hostname (single colon only)
        if h.filter({ $0 == ":" }).count == 1, let colon = h.firstIndex(of: ":") {
            h = String(h[..<colon])
        }
        return h
    }

    /// True when `host` is LOCAL (IP literal / localhost / *.local / private-LAN), so DNS is skipped.
    static func isLocalHost(_ host: String) -> Bool {
        let h = hostOnly(host).lowercased()
        if h.isEmpty { return true }
        if h == "localhost" || h.hasSuffix(".local") || h.hasSuffix(".localhost") { return true }
        if h == "::1" || h.hasPrefix("fe80:") || h.hasPrefix("fc") || h.hasPrefix("fd") { return true }
        let octets = h.split(separator: ".").map(String.init)
        if octets.count == 4, octets.allSatisfy({ Int($0).map { (0...255).contains($0) } ?? false }) {
            let a = Int(octets[0])!, b = Int(octets[1])!
            if a == 127 || a == 10 || a == 0 { return true }
            if a == 192 && b == 168 { return true }
            if a == 172 && (16...31).contains(b) { return true }
            if a == 169 && b == 254 { return true }
            return false   // any other IPv4 literal = public
        }
        return false
    }

    /// Resolve the groomer's domain via DoH and require a TXT answer CONTAINING the expected binding.
    /// Returns true for LOCAL hosts (skip — gate via `isLocalHost`).
    static func verifyGroomer(host: String, groomerAddr: String) async -> Bool {
        if isLocalHost(host) { return true }
        let domain = hostOnly(host)
        if domain.isEmpty || groomerAddr.isEmpty { return false }
        let expected = expectedTxt(groomerAddr)
        guard let name = domain.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) else { return false }
        let url = "https://cloudflare-dns.com/dns-query?name=\(name)&type=TXT"
        let resp = await Http.getJSON(url, accept: "application/dns-json")
        guard resp.ok, let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any],
              let answers = o["Answer"] as? [[String: Any]] else { return false }
        return answers.contains { ans in
            let data = ((ans["data"] as? String) ?? "").trimmingCharacters(in: CharacterSet(charactersIn: "\""))
            return data.contains(expected)
        }
    }
}
