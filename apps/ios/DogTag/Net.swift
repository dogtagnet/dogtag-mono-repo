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

    static func postJSON(_ url: String, body: String, bearer: String? = nil, timeout: TimeInterval = 8) async -> Response {
        await request(url, method: "POST", body: body, bearer: bearer, timeout: timeout)
    }

    private static func request(_ url: String, method: String, body: String?, bearer: String?, timeout: TimeInterval = 8) async -> Response {
        guard let u = URL(string: url) else { return Response(code: -1, body: "bad url") }
        var req = URLRequest(url: u, timeoutInterval: timeout)
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

    /// `DogTagIssuer.isValid(bytes32)` selector, DERIVED from the canonical signature rather than
    /// hard-coded: keccak256("isValid(bytes32)")[:4] = 0x6a938567 - the exact selector viem, the
    /// Rust/alloy ABI, the vet-api `verify_credential` handler and the web direct-RPC path all bind.
    /// It was previously a stale constant `0x6d04f0bc` whose comment *claimed* to be this hash but
    /// wasn't; that selector REVERTS on the deployed ROAX clone, so every check fell through to
    /// `.unknown` (accept-with-caveat) and a revoked credential never showed as revoked. Deriving it
    /// from the signature makes it impossible to drift again.
    private static let isValidSelector = functionSelector("isValid(bytes32)")

    /// 4-byte function selector = first 4 bytes of keccak256(canonical signature), `0x`-prefixed hex.
    private static func functionSelector(_ signature: String) -> String {
        "0x" + Keccak256.digest(Data(signature.utf8)).prefix(4).map { String(format: "%02x", $0) }.joined()
    }

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
    private static let consumedSelector = "0x4648c943"         // keccak256("consumed(bytes32)")[:4]
    private static let profileRootSelector = "0x85105cb3"      // keccak256("profileRoot(uint256)")[:4]
    private static let ownerOfSelector = "0x6352211e"          // keccak256("ownerOf(uint256)")[:4] (ERC-721)

    /// `DogTagSBT.profileRoot(dogTagId)` → the on-chain DOG_PROFILE root (0x.. 32-byte), or nil. This
    /// is the SBT anchor used to verify an issued DOG_PROFILE (NOT the DogTagIssuer-clone isValid).
    static func profileRoot(rpcUrl: String, dogTagSbt: String, dogTagId: String) async -> String? {
        guard !dogTagSbt.isEmpty, !dogTagId.isEmpty else { return nil }
        let data = profileRootSelector + padUint(dogTagId)
        switch await ethCall(rpcUrl: rpcUrl, to: dogTagSbt, data: data) {
        case let .success(hex): return "0x" + String(repeating: "0", count: max(0, 64 - hex.count)) + hex
        case .failure: return nil
        }
    }

    /// `DogTagSBT.ownerOf(dogTagId)` → owner address (0x.. 20-byte, lowercased), or nil.
    static func ownerOf(rpcUrl: String, dogTagSbt: String, dogTagId: String) async -> String? {
        guard !dogTagSbt.isEmpty, !dogTagId.isEmpty else { return nil }
        let data = ownerOfSelector + padUint(dogTagId)
        switch await ethCall(rpcUrl: rpcUrl, to: dogTagSbt, data: data) {
        case let .success(hex):
            let padded = String(repeating: "0", count: max(0, 64 - hex.count)) + hex
            return "0x" + padded.suffix(40).lowercased()
        case .failure: return nil
        }
    }

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

    /// `VerificationRegistry.consumed(nullifier)` → true once the relayer's `recordVerificationZK`
    /// (or the legacy path) has landed on-chain for this nullifier. This is the CANONICAL completion
    /// signal for the async export/verify flow: the groomer host records in the background, so the
    /// phone polls this until it flips true. `nullifier` is the proof's nullifier signal —
    /// `PublicSignalIndex.levelA.nullifier` for the Level-A proofs this app produces today, which is
    /// index 4; note Level-B puts `R` in that slot and the nullifier at 3, so callers must resolve the
    /// index through `PublicSignalIndex` rather than hard-coding it (passing `R` here would poll a key
    /// that is never set and hang on a verification that actually succeeded). Accepts a decimal field
    /// element or 0x.. hex, encoded here as a 32-byte word. Returns false on any RPC failure so
    /// the caller simply keeps polling (and ultimately times out) rather than treating it as success.
    static func consumed(rpcUrl: String, verificationRegistry: String, nullifier: String) async -> Bool {
        guard !verificationRegistry.isEmpty, !nullifier.isEmpty else { return false }
        let data = consumedSelector + padUint(nullifier)
        switch await ethCall(rpcUrl: rpcUrl, to: verificationRegistry, data: data) {
        case let .success(hex): return hex.contains { $0 != "0" }
        case .failure: return false
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

    private static let getContractSetSelector = functionSelector("getContractSet(bytes32)")
    private static let getActiveArtifactSetSelector = functionSelector("getActiveArtifactSet(bytes32)")

    /// keccak256 of a version string as a 32-byte word — the `ProtocolRegistry` map key
    /// (`contractSetId`) for that version. `AnchorResolver.levelBVersion` → the Level-B query key.
    static func versionId(_ version: String) -> String {
        Keccak256.digest(Data(version.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    /// `ProtocolRegistry.getContractSet(versionId)` → the on-chain contract-set record (M-4 PR3).
    /// Returns nil when the registry is unconfigured/unreachable OR the version is unpublished (the
    /// getter reverts "unknown contract set"), so the Level-B branch FAILS CLOSED and never touches
    /// the Level-A path. `protocolRegistry` empty (not yet deployed) is a nil, by design.
    static func getContractSet(rpcUrl: String, protocolRegistry: String, version: String) async -> AnchorResolver.ContractSetRecord? {
        guard !protocolRegistry.isEmpty else { return nil }
        let data = getContractSetSelector + versionId(version)
        switch await ethCall(rpcUrl: rpcUrl, to: protocolRegistry, data: data) {
        case let .success(hex): return AnchorResolver.decodeContractSet(hex)
        case .failure: return nil
        }
    }

    /// `ProtocolRegistry.getActiveArtifactSet(versionId)` → the artifact-set record currently bound to
    /// the version (M-4 PR3). Follows `activeArtifactSetOf` on-chain and reverts if unbound, so a nil
    /// here (unconfigured registry, unpublished version, or no binding) fails the Level-B branch
    /// closed. Decodes only `artifactSetId`/`minAppVersion`/`active` — see `AnchorResolver`.
    static func getActiveArtifactSet(rpcUrl: String, protocolRegistry: String, version: String) async -> AnchorResolver.ArtifactSetRecord? {
        guard !protocolRegistry.isEmpty else { return nil }
        let data = getActiveArtifactSetSelector + versionId(version)
        switch await ethCall(rpcUrl: rpcUrl, to: protocolRegistry, data: data) {
        case let .success(hex): return AnchorResolver.decodeArtifactSet(hex)
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

    /// Encode a decimal (or 0x-hex) uint256 tokenId as a 64-char hex word. Handles values beyond
    /// UInt64 via schoolbook big-endian byte arithmetic (multiply-by-10, add-digit).
    private static func padUint(_ dec: String) -> String {
        if dec.hasPrefix("0x") { return pad32(dec) }
        var bytes = [UInt8](repeating: 0, count: 32) // big-endian accumulator
        for ch in dec {
            guard let d = ch.wholeNumberValue, d >= 0, d <= 9 else { continue }
            var carry = d
            var i = bytes.count - 1
            while i >= 0 {
                let v = Int(bytes[i]) * 10 + carry
                bytes[i] = UInt8(v & 0xFF)
                carry = v >> 8
                i -= 1
            }
        }
        return bytes.map { String(format: "%02x", $0) }.joined()
    }
}

/// Typed client for the per-host vet/groomer APIs: dog-tag issuance bind, the verify-consent relay,
/// and the export-session resolve/poll. Every host comes from a scanned QR — the device never calls a
/// central admin base for registration or pet sync (the dog tag is issued by the vet via `/p/<token>`).
enum CentralApi {
    static func postConsent(sessionToken: String?, payloadJson: String) async -> Http.Response {
        await Http.postJSON("\(AppConfig.centralApi)/v1/verify/consent", body: payloadJson, bearer: sessionToken)
    }

    /// The result of binding a dog-tag at the vet host: the issued DOG_PROFILE + its on-chain anchors.
    struct DogTagIssue {
        let wrappedDocJson: String
        let dogTagId: String
        let root: String
        let txHash: String
        let walletAddress: String
    }

    /// POST <host>/profiles/issue/bind { token, walletAddress, signature } — the vet-issues-the-dog-tag
    /// flow. The host comes from the scanned `/p/<token>` QR (NOT a central base URL). `signature` is the
    /// EIP-191 personal_sign from `WalletIdentity.registerSignature()`. The server now responds
    /// IMMEDIATELY with the off-chain-built credential `{ wrappedDoc, dogTagId, root, walletAddress,
    /// status: "minting" }` and mints the SBT in the background; there is NO `txHash` yet — the phone
    /// polls the chain until the mint lands. Nil on failure. A modest timeout suffices (no on-chain wait).
    static func bindDogTagIssue(host: String, token: String, walletAddress: String, signature: String) async -> DogTagIssue? {
        guard !token.isEmpty else { return nil }
        let body: [String: Any] = ["token": token, "walletAddress": walletAddress, "signature": signature]
        guard let raw = try? JSONSerialization.data(withJSONObject: body),
              let bodyStr = String(data: raw, encoding: .utf8) else { return nil }
        let resp = await Http.postJSON("\(host)/profiles/issue/bind", body: bodyStr, timeout: 20)
        guard resp.ok, let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any] else { return nil }
        // wrappedDoc may be a nested JSON object or a string — normalise to a JSON string.
        let wrappedJson: String
        if let obj = o["wrappedDoc"] as? [String: Any],
           let wd = try? JSONSerialization.data(withJSONObject: obj),
           let s = String(data: wd, encoding: .utf8) {
            wrappedJson = s
        } else if let s = o["wrappedDoc"] as? String {
            wrappedJson = s
        } else {
            return nil
        }
        // dogTagId may serialise as a JSON string or number.
        let dogTagId: String
        if let s = o["dogTagId"] as? String { dogTagId = s }
        else if let n = o["dogTagId"] as? NSNumber { dogTagId = n.stringValue }
        else { dogTagId = "" }
        return DogTagIssue(
            wrappedDocJson: wrappedJson,
            dogTagId: dogTagId,
            root: (o["root"] as? String) ?? "",
            txHash: (o["txHash"] as? String) ?? (o["tx_hash"] as? String) ?? "",
            walletAddress: (o["walletAddress"] as? String) ?? walletAddress)
    }

    /// The platform-OWNED, UNVERIFIED discovery claims from the resolve GET's `unverifiedClaims` block
    /// (M7 §5.2). NONE of these is authority — under a `mode == "levelb"` session they are validated
    /// against the dogtag `ProtocolRegistry` anchor via `validateDiscovery` before the app acts. Empty
    /// when the server omits the block (every pre-M-4 / non-levelb response), which is fine: the
    /// Level-A path never reads them. Deliberately NOT named `ConvenienceClaims` — that is the FFI
    /// record `validateDiscovery` consumes; this is the raw parse the caller maps into it.
    struct UnverifiedClaims {
        let protocolVersion: String
        let chainId: UInt64
        let verificationRegistry: String
        let issuerClone: String
        let purpose: String
    }

    /// The export-session metadata resolved (non-consuming) from the QR's one-time token before proving.
    struct ExportSession {
        let sessionId: String
        let relayer: String
        let purpose: String
        let recordType: String
        let challenge: String
        let mode: String
        /// The `unverifiedClaims` block, present only when the server emits it (M-4 levelb sessions).
        let claims: UnverifiedClaims?

        /// Whether this session presents via the zero-knowledge path. The ECDSA (EIP-712) modes
        /// ("normal"/"ecdsa") have no leaf-count limit; anything else is the ZK circuit path, which
        /// can only prove records within the circuit's leaf budget. Single source for that decision.
        var isZk: Bool {
            let m = mode.lowercased()
            return m != "normal" && m != "ecdsa"
        }
    }

    /// GET <host>/x/<token> → export-session metadata (non-consuming). Nil on failure.
    static func resolveExportSession(host: String, token: String) async -> ExportSession? {
        guard !token.isEmpty else { return nil }
        let resp = await Http.getJSON("\(host)/x/\(token)")
        guard resp.ok, let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any] else { return nil }
        // The convenience tier is additive: absent on every non-levelb / pre-M-4 response.
        var claims: UnverifiedClaims?
        if let uc = o["unverifiedClaims"] as? [String: Any] {
            let chain: UInt64
            if let n = uc["chainId"] as? NSNumber { chain = n.uint64Value }
            else if let s = uc["chainId"] as? String { chain = UInt64(s) ?? 0 }
            else { chain = 0 }
            claims = UnverifiedClaims(
                protocolVersion: (uc["protocolVersion"] as? String) ?? "",
                chainId: chain,
                verificationRegistry: (uc["verificationRegistry"] as? String) ?? "",
                issuerClone: (uc["issuerClone"] as? String) ?? "",
                purpose: (uc["purpose"] as? String) ?? "")
        }
        return ExportSession(
            sessionId: (o["sessionId"] as? String) ?? (o["session_id"] as? String) ?? "",
            relayer: (o["relayer"] as? String) ?? "",
            purpose: (o["purpose"] as? String) ?? "",
            recordType: (o["recordType"] as? String) ?? (o["record_type"] as? String) ?? "",
            challenge: (o["challenge"] as? String) ?? "",
            mode: (o["mode"] as? String) ?? "zk",
            claims: claims)
    }

    /// ZK path: POST the proof bundle directly to the GROOMER host (scanned QR origin), NOT central.
    /// The body carries the one-time `exportToken` (no bearer, consumed on submit). The groomer relays
    /// `recordVerificationZK`.
    static func postVerifyConsentToHost(host: String, payloadJson: String) async -> Http.Response {
        // The submit now returns 200 {status:"recording", sessionId} FAST (no on-chain wait — the
        // groomer binds the consent key + relays recordVerificationZK in the background, ~24-48s on
        // ROAX). A ~20s timeout (like bindDogTagIssue) is ample for the quick ack.
        await Http.postJSON("\(host)/v1/verify/consent", body: payloadJson, timeout: 20)
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
        // dev tunnels exposing the LOCAL demo to a phone (can't host a dogtag-verify TXT) → skip, like
        // any local host. DNS-verify stays enforced for real groomer domains in prod.
        if h.hasSuffix(".trycloudflare.com") || h.hasSuffix(".ngrok-free.app") || h.hasSuffix(".ngrok.io") || h.hasSuffix(".loca.lt") { return true }
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
